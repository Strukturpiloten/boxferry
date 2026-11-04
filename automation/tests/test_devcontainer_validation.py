"""No image execution: independent producer contracts and failed-engine simulations."""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import textwrap
import time
import unittest
from collections.abc import Callable
from contextlib import ExitStack
from pathlib import Path
from typing import Any
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from boxferry_ci import devcontainer_validation as validation
from boxferry_ci.runner import run

SHA = "a" * 40
POLICY = "b" * 40
AUTOMATION = "c" * 40
IMAGE = "sha256:" + "d" * 64
CONTAINER = "e" * 64
INPUT = "f" * 64
ROOT = Path(__file__).resolve().parents[2]


class FakeCommands(validation.Commands):
    def __init__(self, private: Path) -> None:
        self.private = private
        self.budget: validation.Budget | None = None
        self.outcomes: list[dict[str, object]] = []
        self.calls: list[tuple[tuple[str, ...], str, int]] = []
        self.token = ""
        self.tag = ""
        self.failure = ""
        self.wrong_image = False
        self.substituted = False
        self.mutate_source: Path | None = None
        self.mutate_ignore = False
        self.docker_root = private / "docker-store"
        self.docker_root.mkdir(exist_ok=True)
        self.checkouts: list[Path] = []
        self.engine = "docker"
        self.mode = "hosted"

    def call(
        self, argv: tuple[str, ...], stage: str, timeout: int = 30, *, cwd: Path | None = None
    ) -> str:
        self.calls.append((argv, stage, timeout))
        if self.failure == stage:
            raise validation.ValidationError(f"{stage} failed")
        if stage == "podman-package-owner":
            return "podman: /usr/bin/podman"
        if stage == "podman-package-version":
            return "4.9.3+ds1-1ubuntu0.1"
        if stage == "podman-rootful-prerequisite":
            return "0"
        if stage == "engine-info":
            if self.engine == "podman":
                return json.dumps(
                    {
                        "host": {
                            "serviceIsRemote": False,
                            "arch": "amd64",
                            "security": {"rootless": self.mode == "local" and os.geteuid() != 0},
                        },
                        "store": {
                            "graphRoot": str(self.private / "store"),
                            "runRoot": str(self.private / "runroot"),
                        },
                        "version": {"Version": "1.2.3"},
                    }
                )
            return json.dumps(
                {
                    "OSType": "linux",
                    "Architecture": "x86_64",
                    "DockerRootDir": str(self.docker_root),
                }
            )
        if stage == "engine-version":
            if self.engine == "podman":
                return json.dumps({"Client": {"Version": "1.2.3", "APIVersion": "1.2.3"}})
            return json.dumps(
                {
                    "Server": {
                        "Version": "1.2.3",
                        "Platform": {"Name": "Docker Engine - Community"},
                        "Components": [{"Name": "Engine", "Version": "1.2.3"}],
                    }
                }
            )
        if stage == "node-version":
            return "v24.22.1\n"
        if stage == "cli-version":
            return "0.89.0\n"
        if stage == "build":
            if self.mutate_source:
                self.mutate_source.write_text("changed while image was building")
            if self.mutate_ignore:
                snapshot = Path(argv[argv.index("--workspace-folder") + 1])
                (snapshot / ".dockerignore").write_text("!.git/config\n")
            self.tag = argv[argv.index("--image-name") + 1]
            return json.dumps({"outcome": "success", "imageName": [self.tag]})
        if stage in {"image-inspect", "cleanup-image-identity"}:
            return json.dumps(
                [
                    {
                        "Id": IMAGE,
                        "Architecture": "arm64" if self.wrong_image else "amd64",
                        "Os": "linux",
                        "Config": {"Env": []},
                    }
                ]
            )
        if stage.startswith("create-"):
            self.token = argv[argv.index("--label") + 1].split("=", 1)[1]
            return CONTAINER
        if stage == "read-image-user":
            Path(argv[-1]).write_text("vscode:x:1000:1000:Development:/home/vscode:/bin/bash\n")
        if stage == "cleanup-identity":
            return json.dumps(
                [
                    {
                        "Id": CONTAINER,
                        "Config": {
                            "Labels": {
                                "io.boxferry.devcontainer.run": "unrelated"
                                if self.substituted
                                else self.token
                            }
                        },
                    }
                ]
            )
        if stage == "probe-state":
            return json.dumps([{"State": {"ExitCode": 0}, "Image": IMAGE}])
        return ""

    def git(self, root: Path, *argv: str) -> str:
        return "payload\0"


def docker_stat_observer(
    docker: Path, *, distinct: bool = False, inaccessible: bool = False
) -> Callable[..., os.stat_result]:
    original = Path.stat

    def observe(path: Path, *, follow_symlinks: bool = True) -> os.stat_result:
        if path == docker and inaccessible:
            raise PermissionError("Docker store observation denied")
        result = original(path, follow_symlinks=follow_symlinks)
        if path == docker and distinct:
            values = list(result)
            values[2] = result.st_dev + 1
            return os.stat_result(values)
        return result

    return observe


class ProducerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.config = json.loads((ROOT / ".devcontainer/devcontainer.json").read_text())

    def write_config(self, config: dict[str, Any]) -> Path:
        root = self.root / "config"
        (root / ".devcontainer").mkdir(parents=True, exist_ok=True)
        (root / validation.CONFIG).write_text(json.dumps(config))
        (root / validation.LOCK).write_text('{"features":{}}')
        (root / ".devcontainer/verify-tools.sh").write_text("exit 0\n")
        return root

    def test_source_resolution_reads_only_six_scoped_public_defaults_once(self) -> None:
        calls: list[str] = []

        def request(url: str) -> bytes:
            calls.append(url)
            return json.dumps(
                {"sha": SHA} if "/commits/" in url else {"default_branch": "main"}
            ).encode()

        manifest = validation.resolve_sources("boxferry", SHA, request)
        self.assertEqual(len(calls), 12)
        self.assertIsInstance(manifest["repositories"], dict)
        self.assertEqual(
            set(validation.parse_sources(manifest, "boxferry", SHA)),
            set(validation.ALL_REPOSITORIES),
        )
        self.assertEqual(validation.parse_sources(manifest, "boxferry", SHA)["boxferry"], SHA)
        self.assertEqual(len(set(calls)), len(calls))
        self.assertTrue(
            all(url.startswith("https://api.github.com/repos/Strukturpiloten/") for url in calls)
        )
        self.assertEqual(
            validation.resolve_sources("compose-lens", SHA, request)["repositories"],
            {"compose-lens": SHA},
        )
        self.assertEqual(len(calls), 12)

    def test_source_manifest_rejects_missing_extra_floating_or_wrong_candidate(self) -> None:
        good = {name: SHA for name in validation.ALL_REPOSITORIES}
        for sources in (
            {"boxferry": SHA},
            {**good, "unrelated": SHA},
            {**good, "podman-lens": "main"},
            {**good, "boxferry": POLICY},
        ):
            with self.subTest(sources=sources), self.assertRaises(ValueError):
                validation.parse_sources({"schema": 1, "repositories": sources}, "boxferry", SHA)
        for name in ("docker-lens", "kubernetes-lens", "boxferry-website"):
            with self.assertRaises(ValueError):
                validation.resolve_sources(name, SHA)

    def test_hosted_pr_queue_and_main_policy_bases_are_independent(self) -> None:
        validation.validate_binding(
            SHA, POLICY, "pull_request", {"pull_request": {"base": {"sha": POLICY}}}
        )
        validation.validate_binding(
            SHA, POLICY, "merge_group", {"merge_group": {"base_sha": POLICY}}
        )
        validation.validate_binding(SHA, SHA, "push", {})
        validation.validate_binding(SHA, SHA, "workflow_dispatch", {})
        for event, payload in (
            ("pull_request", {"pull_request": {"base": {"sha": POLICY}}}),
            ("merge_group", {"merge_group": {"base_sha": POLICY}}),
            ("workflow_run", {}),
        ):
            with self.assertRaises(ValueError):
                validation.validate_binding(SHA, SHA, event, payload)

    def test_supported_config_and_hooks_mount_or_host_environment_refusal(self) -> None:
        root = self.write_config(self.config)
        self.assertEqual(validation.validate_configuration(root, "boxferry"), self.config)
        for key, value in (
            ("initializeCommand", "touch host"),
            ("runArgs", ["--privileged"]),
            ("mounts", ["source=/var/run/docker.sock,target=/sock,type=bind"]),
            ("features", {"ghcr.io/example/tool:1.0.0": {"version": "${localEnv:TOKEN}"}}),
            ("remoteUser", "root"),
            ("postCreateCommand", "echo changed"),
        ):
            with self.subTest(key=key):
                self.write_config({**self.config, key: value})
                with self.assertRaises(ValueError):
                    validation.validate_configuration(root, "boxferry")

    def test_all_reviewed_consumer_configuration_shapes(self) -> None:
        for repository in ("compose-lens", "podman-lens", "quadlet-lens"):
            config = dict(self.config)
            config["workspaceFolder"] = f"/workspaces/{repository}"
            config["workspaceMount"] = (
                f"source=${{localWorkspaceFolder}},target=/workspaces/{repository},type=bind"
            )
            config["postCreateCommand"] += " && cargo fetch --locked"
            config["containerEnv"] = {
                "CARGO_HOME": f"/workspaces/.{repository}-cargo",
                "CARGO_TARGET_DIR": f"/workspaces/.{repository}-target",
            }
            if repository == "podman-lens":
                config["containerEnv"]["GH_CONFIG_DIR"] = f"/workspaces/.{repository}-gh"
            else:
                config["containerEnv"]["PATH"] = self.config["containerEnv"]["PATH"]
            names = {"CARGO_HOME": "cargo", "CARGO_TARGET_DIR": "target", "GH_CONFIG_DIR": "gh"}
            config["mounts"] = [
                f"source={repository}-{names[key]}-${{devcontainerId}},target={path},type=volume"
                for key, path in config["containerEnv"].items()
                if key != "PATH"
            ]
            root = self.write_config(config)
            self.assertEqual(validation.validate_configuration(root, repository), config)

    def test_local_always_requires_podman_and_hosted_backends_are_exact(self) -> None:
        for repository in validation.OWNERS:
            self.assertEqual(validation.backend(repository, "local", "podman"), "podman")
            with self.assertRaises(ValueError):
                validation.backend(repository, "local", "docker")
            expected = "docker" if repository in {"boxferry", "compose-lens"} else "podman"
            self.assertEqual(validation.backend(repository, "hosted", expected), expected)

    def test_image_identity_normalizes_podman_and_rejects_emulated_or_malformed_image(self) -> None:
        record = {"Id": IMAGE, "Architecture": "amd64", "Os": "linux", "Config": {"Env": []}}
        self.assertEqual(validation.inspect_image([record], "amd64"), IMAGE)
        self.assertEqual(
            validation.inspect_image([{**record, "Id": IMAGE.removeprefix("sha256:")}], "amd64"),
            IMAGE,
        )
        for change in (
            {"Architecture": "arm64"},
            {"Os": "windows"},
            {"Id": "tag"},
            {"Config": {"Volumes": {"/unowned": {}}, "Env": []}},
        ):
            with self.assertRaises(ValueError):
                validation.inspect_image([{**record, **change}], "amd64")

    def test_private_podman_backend_must_match_store_and_native_host(self) -> None:
        facts = {
            "host": {"serviceIsRemote": False, "arch": "arm64", "security": {"rootless": True}},
            "store": {"graphRoot": str(self.root / "store"), "runRoot": str(self.root / "runroot")},
        }
        validation.validate_engine(facts, "podman", "arm64", self.root, rootless=True)
        for changed in (
            {**facts, "host": {"serviceIsRemote": True, "arch": "arm64"}},
            {**facts, "store": {"graphRoot": "/personal/store", "runRoot": "/run/user/1000"}},
        ):
            with self.assertRaises(ValueError):
                validation.validate_engine(changed, "podman", "arm64", self.root, rootless=True)

    def test_hosted_podman_uses_root_owned_timeout_private_store_and_local_has_no_sudo(
        self,
    ) -> None:
        hosted = self.root / "hosted"
        local = self.root / "local"
        hosted.mkdir()
        local.mkdir()
        hosted_script = Path(validation._engine(hosted, "podman", "hosted")).read_text()
        local_script = Path(validation._engine(local, "podman", "local")).read_text()
        self.assertIn("/usr/bin/sudo -n /usr/bin/timeout", hosted_script)
        self.assertIn("--kill-after=2s", hosted_script)
        self.assertIn("seconds=3600", hosted_script)
        self.assertIn("seconds=300", hosted_script)
        self.assertIn("--remote=false", hosted_script)
        self.assertIn("--root " + str(hosted / "store"), hosted_script)
        self.assertIn("CONTAINERS_CONF_OVERRIDE=", hosted_script)
        self.assertNotIn("sudo", local_script)
        self.assertNotIn("docker", local_script)
        self.assertEqual((hosted / "containers.conf").read_text(), '[engine]\nlock_type = "file"\n')

    def test_hosted_podman_signed_apt_exception_is_explicit_and_rootful_is_mandatory(self) -> None:
        environment = {
            "GITHUB_ACTIONS": "true",
            "RUNNER_OS": "Linux",
            "GITHUB_REPOSITORY": "Strukturpiloten/podman-lens",
        }
        release = 'ID=ubuntu\nVERSION_ID="24.04"\n'
        fake = FakeCommands(self.root)
        with (
            patch.dict(os.environ, environment),
            patch.object(Path, "read_text", return_value=release),
            patch.object(Path, "is_file", return_value=False),
        ):
            profile = validation.prepare_hosted_podman(fake)
        self.assertEqual(profile["installation"], "ubuntu-signed-apt-prerequisite")
        self.assertEqual(profile["package"], "4.9.3+ds1-1ubuntu0.1")
        self.assertFalse(profile["rootless"])
        apt = next(
            argv
            for argv, stage, _timeout in fake.calls
            if stage == "podman-signed-apt-prerequisite"
        )
        self.assertEqual(
            apt[-1], "apt-get update && apt-get install --yes --no-install-recommends podman"
        )
        with self.assertRaises(ValueError):
            validation.validate_hosted_podman_host(
                {**environment, "GITHUB_ACTIONS": "false"}, release
            )
        with self.assertRaises(ValueError):
            validation.validate_hosted_podman_host(environment, "ID=debian\nVERSION_ID=12\n")
        version = {"Client": {"Version": "4.9.3", "APIVersion": "4.9.3"}}
        validation.validate_engine_version(version, "podman")
        with self.assertRaises(ValueError):
            validation.validate_engine_version(
                {"Client": {"Version": "unqualified", "APIVersion": "4.9.3"}}, "podman"
            )
        facts = {
            "host": {"serviceIsRemote": False, "arch": "amd64", "security": {"rootless": True}},
            "store": {"graphRoot": str(self.root / "store"), "runRoot": str(self.root / "runroot")},
        }
        with self.assertRaises(ValueError):
            validation.validate_engine(facts, "podman", "amd64", self.root, rootless=False)

    def test_build_context_excludes_real_git_and_binds_ignored_override_files(self) -> None:
        snapshot = self.root / "snapshot"
        (snapshot / ".devcontainer").mkdir(parents=True)
        (snapshot / ".git").mkdir()
        (snapshot / ".git/HEAD").write_text(SHA)
        (snapshot / ".git/config").write_text("first metadata")
        (snapshot / ".dockerignore").write_text("!.git/config\n")
        (snapshot / ".containerignore").write_text("!.git/config\n")
        (snapshot / ".devcontainer/Dockerfile.dockerignore").write_text("!.git\n")
        commands = validation.Commands(self.root / "command-private")
        # Candidate Git ignore may hide generated ignore files from ls-files.
        with patch.object(commands, "git", return_value=""):
            ignores = validation.exclude_git_build_metadata(snapshot)
            held = validation.build_context_fingerprint(snapshot, commands, ignores)
            self.assertIn(".containerignore", ignores)
            self.assertEqual(ignores[".containerignore"]["mode"], 0o644)
            for name in ignores:
                self.assertTrue((snapshot / name).read_text().endswith(".git\n**/.git\n"))
            (snapshot / ".git/config").write_text("different runtime-only metadata")
            self.assertEqual(
                validation.build_context_fingerprint(snapshot, commands, ignores), held
            )
            self.assertEqual((snapshot / ".git/HEAD").read_text(), SHA)
            # Even Git-ignored generated overrides have a held executable mode.
            for name in ignores:
                path = snapshot / name
                original_mode = path.stat().st_mode & 0o7777
                path.chmod(0o755)
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, "ignore changed"):
                    validation.build_context_fingerprint(snapshot, commands, ignores)
                path.chmod(original_mode)
            (snapshot / ".dockerignore").write_text("!.git/config\n")
            with self.assertRaisesRegex(ValueError, "ignore changed"):
                validation.build_context_fingerprint(snapshot, commands, ignores)

    def test_command_failures_and_budget_monitor_retain_redacted_stage_command_cause(self) -> None:
        private = self.root / "diagnostics"
        commands = validation.Commands(private)
        secret = "synthetic-private-command-token"
        with patch.dict(os.environ, {"TEST_TOKEN": secret}):
            with self.assertRaises(validation.ValidationError):
                commands.call(
                    (
                        sys.executable,
                        "-c",
                        "import sys; print('broken Dockerfile', file=sys.stderr); sys.exit(7)",
                        "--api-token",
                        secret,
                    ),
                    "candidate-build",
                )
            failure = commands.outcomes[-1]
            self.assertEqual(failure["stage"], "candidate-build")
            self.assertIn("command exited 7", str(failure["cause"]))
            self.assertIn("broken Dockerfile", str(failure["cause"]))
            self.assertIn("[REDACTED]", str(failure["command"]))
            self.assertNotIn(secret, json.dumps(commands.outcomes))
            commands.budget = validation.Budget(private, 0, time.monotonic() - 1, check_disk=False)
            with (
                self.assertRaisesRegex(ValueError, "disk-precheck:.*deadline"),
                patch.object(validation, "run") as execute,
            ):
                commands.call(("/bin/false", "--api-token", secret), "disk-precheck")
            execute.assert_not_called()
            self.assertIn("false", str(commands.outcomes[-1]["command"]))
            pid = private / "child.pid"
            commands.budget = validation.Budget(private, 0, time.monotonic() + 5, check_disk=False)

            def monitor() -> None:
                if pid.exists():
                    raise validation.ValidationError("disk reserve exhausted")

            program = (
                "import os,time; from pathlib import Path; "
                "Path('child.pid').write_text(str(os.getpid())); time.sleep(30)"
            )
            with (
                patch.object(commands.budget, "check", side_effect=monitor),
                self.assertRaisesRegex(ValueError, "image-probe:.*disk reserve"),
            ):
                commands.call((sys.executable, "-c", program), "image-probe", 5)
            self.assertEqual(commands.outcomes[-1]["stage"], "image-probe")
            self.assertIn("python", str(commands.outcomes[-1]["command"]))
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pid.read_text()), 0)

    def test_explicit_local_sibling_paths_have_exact_keys_and_no_fallback(self) -> None:
        sources = {"schema": 1, "repositories": {name: SHA for name in validation.ALL_REPOSITORIES}}
        good = {name: self.root / name for name in validation.SIBLINGS}
        for paths in (
            {},
            {**good, "unrelated": self.root},
            {name: path for name, path in good.items() if name != "compose-lens"},
        ):
            with (
                self.subTest(paths=paths),
                self.assertRaisesRegex(ValueError, "exactly the six scoped"),
            ):
                validation.qualify(
                    repository="boxferry",
                    candidate=SHA,
                    policy_sha=SHA,
                    automation_sha=AUTOMATION,
                    source=self.root,
                    policy=self.root,
                    automation=self.root,
                    manifest=sources,
                    engine="podman",
                    mode="local",
                    work=self.root,
                    local_sources=paths,
                )
        held = self.root / "actual-sibling"
        held.mkdir()
        commands = validation.Commands(self.root / "local-path-commands")
        paths = {**good, "compose-lens": held}
        for fault in ("revision", "origin", "dirty"):

            def git(root: Path, *arguments: str, fault: str = fault) -> str:
                self.assertEqual(root, held)
                if arguments == ("rev-parse", "--show-toplevel"):
                    return str(held)
                if arguments == ("rev-parse", "HEAD"):
                    return POLICY if fault == "revision" else SHA
                if arguments[0] == "status":
                    return " M file" if fault == "dirty" else ""
                return "https://github.com/Strukturpiloten/unrelated.git"

            with (
                self.subTest(fault=fault),
                patch.object(commands, "git", side_effect=git),
                self.assertRaises(ValueError),
            ):
                validation._local_sibling_source("compose-lens", self.root, paths, SHA, commands)
        with (
            patch.object(validation, "_held_revision") as held_check,
            patch.object(validation, "_verify_origin") as origin_check,
        ):
            self.assertEqual(
                validation._local_sibling_source("compose-lens", self.root, paths, SHA, commands),
                held,
            )
        held_check.assert_called_once_with(held, SHA, commands)
        origin_check.assert_called_once_with(held, "compose-lens", commands)

    def test_successful_cli_exports_bound_generated_context_digest(self) -> None:
        report = {
            "schema": 1,
            "status": "success",
            "candidate_sha": SHA,
            "source_manifest_sha256": INPUT,
            "input_sha256": INPUT,
            "build_context_sha256": "b" * 64,
            "architecture": "amd64",
        }
        output = self.root / "report.json"
        github_output = self.root / "github-output"
        manifest = {
            "schema": 1,
            "repositories": {name: SHA for name in validation.ALL_REPOSITORIES},
        }
        with patch.object(validation, "qualify", return_value=report):
            code = validation.main(
                [
                    "qualify",
                    "--repository",
                    "boxferry",
                    "--candidate-sha",
                    SHA,
                    "--policy-sha",
                    SHA,
                    "--automation-sha",
                    AUTOMATION,
                    "--source",
                    str(self.root),
                    "--policy",
                    str(self.root),
                    "--automation",
                    str(self.root),
                    "--sources-json",
                    json.dumps(manifest),
                    "--engine",
                    "docker",
                    "--output",
                    str(output),
                    "--github-output",
                    str(github_output),
                ]
            )
        self.assertEqual(code, 0)
        fields = dict(line.split("=", 1) for line in github_output.read_text().splitlines())
        self.assertEqual(fields["build_context_sha256"], "b" * 64)
        self.assertEqual(fields["candidate_sha"], SHA)

    def test_passwd_lookup_never_executes_candidate_user_discovery(self) -> None:
        passwd = self.root / "passwd"
        passwd.write_text("vscode:x:1000:1000::/home/vscode:/bin/bash\n")
        self.assertEqual(validation.passwd_identity(passwd, "vscode"), ("1000", "1000"))
        for text in (
            "vscode:x:0:0::/root:/bin/bash\n",
            "vscode:x:1000:1000::/home/vscode:/bin/bash\n" * 2,
            "vscode:bad",
        ):
            passwd.write_text(text)
            with self.assertRaises(ValueError):
                validation.passwd_identity(passwd, "vscode")
        passwd.unlink()
        passwd.symlink_to("/etc/passwd")
        with self.assertRaises(ValueError):
            validation.passwd_identity(passwd, "vscode")

    def test_probe_script_executes_versions_and_real_workspace_heads(self) -> None:
        script = validation.probe_script(
            {"uv": "1.2.3", "node": "4.5.6", "ruff": "7.8.9", "ty": "0.1.2"},
            {name: SHA for name in validation.ALL_REPOSITORIES},
            "boxferry",
        )
        self.assertIn("kubectl version --client=true", script)
        self.assertIn("bash .devcontainer/verify-tools.sh", script)
        self.assertIn("uv --version", script)
        self.assertIn("ruff 7.8.9", script)
        self.assertEqual(script.count("rev-parse HEAD"), 7)
        self.assertNotIn("command -v", script)
        self.assertNotIn("mkdir .git", script)
        self.assertNotIn("cargo fetch", script)

    def test_version_probe_requires_successful_execution_and_exact_selected_version(self) -> None:
        tools = self.root / "bin"
        tools.mkdir()
        for name, output in (("id", "1000"), ("uname", "x86_64")):
            executable = tools / name
            executable.write_text("#!/bin/sh\nprintf '%s\\n' " + output + "\n")
            executable.chmod(0o700)
        (self.root / ".devcontainer").mkdir()
        (self.root / ".devcontainer/verify-tools.sh").write_text("exit 0\n")
        with patch.object(validation, "PROBES", (("uv", "--version"),)):
            script = validation.probe_script({"uv": "0.13.6"}, {}, "boxferry")
        script = script.replace("/tmp/qualification-home", str(self.root / "owned-home"))
        for version, exitcode, expected in (
            ("0.13.6", 0, "success"),
            ("0.13.5", 0, "failed"),
            ("0.13.6", 32, "failed"),
            ("0.13.5" + "x" * 2000, 0, "failed"),
        ):
            executable = tools / "uv"
            executable.write_text(
                f"#!/bin/sh\nprintf 'uv {version} (platform)\\n'\nexit {exitcode}\n"
            )
            executable.chmod(0o700)
            result = run(
                ("/bin/bash", "-c", script, "qualification", "x86_64"),
                cwd=self.root,
                stage="probe-shell-contract",
                timeout_seconds=5,
                environment={"PATH": str(tools) + ":/usr/bin:/bin"},
            )
            self.assertEqual(result.status, expected, result)
            self.assertIn("probe: uv", result.stdout)
            if exitcode:
                self.assertEqual(result.returncode, exitcode)
            elif expected == "failed":
                self.assertIn("Version mismatch for uv", result.stderr)
                self.assertIn("expected uv 0.13.6", result.stderr)
                self.assertIn("observed uv 0.13.5", result.stderr)
                self.assertLess(len(result.stderr), 360)
                self.assertEqual(result.returncode, 1)

    def test_local_input_fingerprint_tracks_content_mode_and_deleted_files(self) -> None:
        source = self.root / "source"
        source.mkdir()
        target = source / "Dockerfile"
        target.write_text("FROM held-image\n")
        commands = validation.Commands(self.root / "private")
        with patch.object(commands, "git", return_value="Dockerfile\0"):
            original = validation.fingerprint(source, commands)
            target.write_text("FROM changed-image\n")
            self.assertNotEqual(validation.fingerprint(source, commands), original)
            target.write_text("FROM held-image\n")
            target.chmod(0o700)
            self.assertNotEqual(validation.fingerprint(source, commands), original)
            target.unlink()
            self.assertNotEqual(validation.fingerprint(source, commands), original)
            target.symlink_to("/etc/passwd")
            with self.assertRaises(ValueError):
                validation.fingerprint(source, commands)

    def test_empty_git_placeholder_cannot_inherit_parent_checkout_revision(self) -> None:
        source = self.root / "placeholder"
        (source / ".git").mkdir(parents=True)
        commands = validation.Commands(self.root / "private")
        with (
            patch.object(commands, "git", return_value=str(self.root)),
            self.assertRaisesRegex(ValueError, "actual independent Git checkout"),
        ):
            validation._held_revision(source, SHA, commands)

    def test_actual_workflow_aggregate_refuses_failed_missing_or_stale_lane_results(self) -> None:
        source = (ROOT / ".github/workflows/shared-devcontainer-validation.yml").read_text()
        script = textwrap.dedent(source.rsplit("        run: |\n", 1)[1])
        environment = {
            "PATH": "/usr/bin:/bin",
            "SOURCES_RESULT": "success",
            "X64_RESULT": "success",
            "ARM64_RESULT": "success",
            "CANDIDATE_SHA": SHA,
            "X64_CANDIDATE": SHA,
            "ARM64_CANDIDATE": SHA,
            "SOURCE_DIGEST": INPUT,
            "X64_SOURCE_DIGEST": INPUT,
            "ARM64_SOURCE_DIGEST": INPUT,
            "X64_INPUT": INPUT,
            "ARM64_INPUT": INPUT,
            "X64_CONTEXT": INPUT,
            "ARM64_CONTEXT": INPUT,
            "X64_ARCHITECTURE": "amd64",
            "ARM64_ARCHITECTURE": "arm64",
            "GITHUB_OUTPUT": str(self.root / "outputs"),
        }
        changes = [
            None,
            *(
                (key, status)
                for key in ("X64_RESULT", "ARM64_RESULT", "SOURCES_RESULT")
                for status in ("failure", "cancelled", "skipped", "")
            ),
            ("X64_CANDIDATE", POLICY),
            ("ARM64_CANDIDATE", POLICY),
            ("ARM64_SOURCE_DIGEST", "e" * 64),
            ("ARM64_INPUT", "d" * 64),
            ("ARM64_CONTEXT", "c" * 64),
            ("ARM64_ARCHITECTURE", "amd64"),
        ]
        for change in changes:
            env = {**environment, **({change[0]: change[1]} if change else {})}
            result = run(
                ("/bin/bash", "-c", script),
                cwd=self.root,
                stage="actual-workflow-aggregate",
                environment=env,
                timeout_seconds=5,
            )
            self.assertEqual(result.status, "success" if change is None else "failed", change)

    def simulate(
        self,
        failure: str = "",
        *,
        wrong_image: bool = False,
        substituted: bool = False,
        mutate_source: bool = False,
        mutate_ignore: bool = False,
        docker_root: Path | None = None,
        engine: str = "docker",
        mode: str = "hosted",
    ) -> tuple[dict[str, object], FakeCommands]:
        fake = FakeCommands(self.root)
        fake.engine, fake.mode = engine, mode
        repository = "podman-lens" if engine == "podman" else "boxferry"
        if docker_root is not None:
            fake.docker_root = docker_root
        fake.failure, fake.wrong_image, fake.substituted = failure, wrong_image, substituted
        (self.root / "payload").write_text("held source bytes")
        fake.mutate_source = self.root / "payload" if mutate_source else None
        fake.mutate_ignore = mutate_ignore
        actual_fingerprint = validation.fingerprint
        event = self.root / "event.json"
        event.write_text("{}")
        versions = {"node": "24.22.1", "uv": "0.13.6", "ruff": "0.17.3", "ty": "0.0.95"}

        def checkout(
            destination: Path, _revision: str, _commands: validation.Commands, _source: str
        ) -> None:
            fake.checkouts.append(destination)
            destination.mkdir()
            shutil.copyfile(self.root / "payload", destination / "payload")

        def commands(private: Path) -> FakeCommands:
            fake.private = private
            return fake

        with ExitStack() as stack:
            stack.enter_context(
                patch.dict(
                    "os.environ",
                    {
                        "GITHUB_SHA": SHA,
                        "GITHUB_REPOSITORY": f"Strukturpiloten/{repository}",
                        "GITHUB_EVENT_PATH": str(event),
                        "GITHUB_EVENT_NAME": "push",
                    },
                )
            )
            for name, replacement in (
                ("Commands", commands),
                ("_held_revision", lambda *_args, **_kwargs: None),
                ("_verify_origin", lambda *_args: None),
                ("fingerprint", actual_fingerprint if mutate_source else lambda *_args: INPUT),
                ("_checkout", checkout),
                ("validate_configuration", lambda *_args: self.config),
                ("canonical_versions", lambda *_args: versions),
                ("_cli", lambda *_args: (Path("/trusted/cli.js"), "0.89.0")),
            ):
                stack.enter_context(patch.object(validation, name, replacement))
            stack.enter_context(
                patch.object(
                    validation.Budget,
                    "start",
                    return_value=validation.Budget(
                        self.root, 13 * validation.GIB, 10**20, check_disk=False
                    ),
                )
            )
            stack.enter_context(patch.object(validation.platform, "machine", return_value="x86_64"))
            stack.enter_context(patch.object(validation.platform, "system", return_value="Linux"))
            stack.enter_context(
                patch.object(validation.shutil, "which", return_value=sys.executable)
            )
            result = validation.qualify(
                repository=repository,
                candidate=SHA,
                policy_sha=SHA,
                automation_sha=AUTOMATION,
                source=self.root,
                policy=self.root,
                automation=self.root,
                manifest={
                    "schema": 1,
                    "repositories": {
                        name: SHA
                        for name in (
                            validation.ALL_REPOSITORIES
                            if repository == "boxferry"
                            else (repository,)
                        )
                    },
                },
                engine=engine,
                mode=mode,
                work=self.root,
                expected_architecture="amd64",
            )
        return result, fake

    def test_successful_producer_builds_frozen_and_uses_exact_id_readonly_sources(self) -> None:
        result, fake = self.simulate()
        self.assertEqual(result["status"], "success", result)
        self.assertEqual(result["cleanup"], "success")
        self.assertEqual(
            result["workspace_sources"], {name: SHA for name in validation.ALL_REPOSITORIES}
        )
        build = next(argv for argv, stage, _timeout in fake.calls if stage == "build")
        self.assertIn("--experimental-frozen-lockfile", build)
        self.assertIn("linux/amd64", build)
        create = next(argv for argv, stage, _timeout in fake.calls if stage == "create-probe")
        self.assertIn(IMAGE, create)
        self.assertNotIn(fake.tag, create)
        self.assertIn("1000:1000", create)
        self.assertEqual(
            sum("type=bind" in value and value.endswith(",readonly") for value in create), 7
        )
        self.assertIn("none", create)
        self.assertNotIn("--privileged", create)
        self.assertNotIn("docker.sock", " ".join(create))
        self.assertTrue(
            all(
                "prune" not in argv and "reset" not in argv for argv, _stage, _timeout in fake.calls
            )
        )
        self.assertFalse(any("run" in argv for argv, _stage, _timeout in fake.calls))

    def test_build_probe_receipt_and_cleanup_failures_never_report_success(self) -> None:
        for stage in (
            "build",
            "tooling-workspace-probe",
            "probe-state",
            "cleanup-container",
            "cleanup-image-tag",
            "cleanup-container-absence",
        ):
            with self.subTest(stage=stage):
                result, _fake = self.simulate(stage)
                self.assertEqual(result["status"], "failure")
        result, _fake = self.simulate(wrong_image=True)
        self.assertEqual(result["status"], "failure")
        self.assertIn("native host architecture", str(result.get("cause")))

    def test_source_and_generated_context_changes_during_build_are_rejected(self) -> None:
        for change in ("source", "ignore"):
            with self.subTest(change=change):
                result, _fake = self.simulate(
                    mutate_source=change == "source", mutate_ignore=change == "ignore"
                )
                self.assertEqual(result["status"], "failure")
                self.assertEqual(result["cleanup"], "success")
                self.assertTrue(
                    "input closure changed" in str(result.get("cause"))
                    or "ignore changed" in str(result.get("cause")),
                    result,
                )

    def test_cleanup_refuses_substitution_and_preserves_unrelated_sentinel(self) -> None:
        sentinel = self.root / "unrelated-resource"
        sentinel.write_text("owned by another task")
        result, fake = self.simulate(substituted=True)
        self.assertEqual(result["status"], "failure")
        self.assertEqual(result["cleanup"], "failure")
        self.assertEqual(sentinel.read_text(), "owned by another task")
        self.assertFalse(any(stage == "cleanup-container" for _argv, stage, _timeout in fake.calls))
        self.assertTrue(Path(str(result["retained_private_directory"])).is_dir())

    def test_docker_budget_same_filesystem_records_facts_without_rebaselining(self) -> None:
        private, docker = self.root / "private", self.root / "daemon"
        private.mkdir()
        docker.mkdir()
        report: dict[str, object] = {}
        with (
            patch.object(validation.Budget, "start") as start,
            patch.object(validation.shutil, "disk_usage") as usage,
        ):
            validation.validate_docker_budget_filesystem(private, str(docker), report)
        start.assert_not_called()
        usage.assert_not_called()
        self.assertEqual(
            report["docker_storage"],
            {
                "reported_docker_root_dir": str(docker),
                "docker_root_dir": str(docker.resolve(strict=True)),
                "budget_path": str(private.resolve(strict=True)),
                "budget_device": private.stat().st_dev,
                "docker_device": docker.stat().st_dev,
                "same_filesystem": True,
            },
        )

    def test_successful_docker_qualification_reports_observed_storage(self) -> None:
        report, fake = self.simulate()
        self.assertEqual(report["status"], "success", report)
        facts = report["docker_storage"]
        self.assertIsInstance(facts, dict)
        assert isinstance(facts, dict)
        self.assertEqual(facts["docker_root_dir"], str(fake.docker_root.resolve(strict=True)))
        self.assertEqual(facts["budget_device"], self.root.stat().st_dev)
        self.assertEqual(facts["docker_device"], self.root.stat().st_dev)
        self.assertIs(facts["same_filesystem"], True)
        self.assertTrue(fake.checkouts)
        self.assertFalse(Path(str(facts["budget_path"])).exists())
        self.assertTrue(fake.docker_root.is_dir())

    def assert_docker_refused_before_preparation(
        self, report: dict[str, object], fake: FakeCommands
    ) -> None:
        self.assertEqual(report["status"], "failure")
        self.assertEqual(report["cleanup"], "success")
        self.assertEqual(fake.checkouts, [])
        self.assertEqual(
            [stage for _argv, stage, _timeout in fake.calls], ["engine-version", "engine-info"]
        )
        self.assertNotIn("retained_private_directory", report)

    def test_distinct_docker_filesystem_fails_before_snapshots_build_and_resource_cleanup(
        self,
    ) -> None:
        docker = self.root / "other-store"
        docker.mkdir()
        sentinel = docker / "unrelated-data"
        sentinel.write_text("preserve daemon data")
        with patch.object(Path, "stat", docker_stat_observer(docker, distinct=True)):
            report, fake = self.simulate(docker_root=docker)
        self.assert_docker_refused_before_preparation(report, fake)
        self.assertIn("different filesystems", str(report.get("cause")))
        facts = report["docker_storage"]
        assert isinstance(facts, dict)
        self.assertEqual(facts["docker_root_dir"], str(docker.resolve(strict=True)))
        self.assertNotEqual(facts["budget_device"], facts["docker_device"])
        self.assertIs(facts["same_filesystem"], False)
        self.assertEqual(sentinel.read_text(), "preserve daemon data")

    def test_missing_or_non_directory_docker_root_fails_before_preparation(self) -> None:
        missing, regular = self.root / "missing-store", self.root / "regular-store"
        regular.write_text("not a directory")
        for docker, cause in ((missing, "observable"), (regular, "must be existing directories")):
            with self.subTest(root=docker):
                report, fake = self.simulate(docker_root=docker)
                self.assert_docker_refused_before_preparation(report, fake)
                self.assertIn(cause, str(report.get("cause")))
                facts = report["docker_storage"]
                assert isinstance(facts, dict)
                self.assertIsNone(facts["same_filesystem"])
                self.assertEqual(facts["reported_docker_root_dir"], str(docker))
        self.assertFalse(missing.exists())
        self.assertEqual(regular.read_text(), "not a directory")

    def test_inaccessible_docker_stat_fails_actionably_and_retains_partial_report(self) -> None:
        docker = self.root / "inaccessible-store"
        docker.mkdir()
        with patch.object(Path, "stat", docker_stat_observer(docker, inaccessible=True)):
            report, fake = self.simulate(docker_root=docker)
        self.assert_docker_refused_before_preparation(report, fake)
        self.assertIn("observable", str(report.get("cause")))
        self.assertIn("observation denied", str(report.get("cause")))
        facts = report["docker_storage"]
        assert isinstance(facts, dict)
        self.assertEqual(facts["docker_root_dir"], str(docker.resolve(strict=True)))
        self.assertIsInstance(facts["budget_device"], int)
        self.assertIsNone(facts["docker_device"])
        self.assertIsNone(facts["same_filesystem"])

    def test_docker_storage_symlink_uses_resolved_target_device_and_report_path(self) -> None:
        private, docker = self.root / "private", self.root / "real-store"
        private.mkdir()
        docker.mkdir()
        alias = private / "apparent-store"
        alias.symlink_to(docker, target_is_directory=True)
        report: dict[str, object] = {}
        validation.validate_docker_budget_filesystem(private, str(alias), report)
        facts = report["docker_storage"]
        assert isinstance(facts, dict)
        self.assertEqual(facts["docker_root_dir"], str(docker))
        self.assertIs(facts["same_filesystem"], True)
        with (
            patch.object(Path, "stat", docker_stat_observer(docker, distinct=True)),
            self.assertRaisesRegex(validation.ValidationError, "different filesystems"),
        ):
            validation.validate_docker_budget_filesystem(private, str(alias), report)
        facts = report["docker_storage"]
        assert isinstance(facts, dict)
        self.assertEqual(facts["docker_root_dir"], str(docker))
        self.assertIs(facts["same_filesystem"], False)

    def test_private_budget_path_must_also_be_an_existing_observable_directory(self) -> None:
        docker = self.root / "daemon"
        docker.mkdir()
        missing, regular = self.root / "missing-budget", self.root / "regular-budget"
        regular.write_text("not a directory")
        for private in (missing, regular):
            with self.subTest(private=private), self.assertRaises(validation.ValidationError):
                validation.validate_docker_budget_filesystem(private, str(docker), {})
        with (
            patch.object(Path, "stat", docker_stat_observer(docker, inaccessible=True)),
            self.assertRaisesRegex(validation.ValidationError, "observable"),
        ):
            validation.validate_docker_budget_filesystem(docker, str(docker), {})

    def test_hosted_and_local_podman_do_not_apply_docker_storage_guard(self) -> None:
        for mode in ("hosted", "local"):
            with (
                self.subTest(mode=mode),
                patch.object(validation, "prepare_hosted_podman", return_value={"fixture": True}),
                patch.object(validation, "validate_docker_budget_filesystem") as guard,
            ):
                report, _fake = self.simulate(engine="podman", mode=mode)
                self.assertEqual(report["status"], "success", report)
                self.assertEqual(report["engine"], "podman")
                self.assertNotIn("docker_storage", report)
                guard.assert_not_called()

    def test_disk_initial_growth_and_reserve_boundaries(self) -> None:
        for free in (12 * validation.GIB - 1, 0):
            with (
                patch.object(
                    validation.shutil,
                    "disk_usage",
                    return_value=shutil._ntuple_diskusage(20 * validation.GIB, 0, free),
                ),
                self.assertRaises(ValueError),
            ):
                validation.Budget.start(self.root, 3600)
        budget = validation.Budget(self.root, 13 * validation.GIB, 10**20)
        with (
            patch.object(
                validation.shutil,
                "disk_usage",
                return_value=shutil._ntuple_diskusage(20 * validation.GIB, 0, 2 * validation.GIB),
            ),
            self.assertRaises(ValueError),
        ):
            budget.check()
        # Cleanup has its own deadline and must still operate when disk is full.
        cleanup = validation.Budget(self.root, 0, 10**20, check_disk=False)
        cleanup.check()

    def test_workflow_has_two_literal_native_lanes_and_fail_closed_aggregate(self) -> None:
        source = (ROOT / ".github/workflows/shared-devcontainer-validation.yml").read_text()
        self.assertIn("runs-on: ubuntu-24.04\n", source)
        self.assertIn("runs-on: ubuntu-24.04-arm\n", source)
        self.assertIn("needs: [sources, x64, arm64]", source)
        self.assertIn('test "${X64_RESULT}" = success', source)
        self.assertIn('test "${ARM64_RESULT}" = success', source)
        self.assertNotIn("secrets:", source)
        self.assertNotIn("continue-on-error", source)
        self.assertNotIn("docker/login", source)


if __name__ == "__main__":
    unittest.main()
