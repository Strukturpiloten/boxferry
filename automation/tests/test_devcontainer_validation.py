"""No image execution: independent producer contracts and failed-engine simulations."""

from __future__ import annotations

import contextlib
import io
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
from boxferry_ci.runner import Diagnostic, RunResult, run

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
        self.abort_reports: list[dict[str, object]] = []
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
        self.outer_mode = "rootful"
        self.cancel_stage = ""
        self.podman_root = private / "runner-podman-store"
        self.podman_root.mkdir(exist_ok=True)
        (self.podman_root / "unrelated-cache").write_text("preserve")
        self.profile: dict[str, object] = {
            "cgroupManager": "cgroupfs",
            "eventLogger": "file",
            "ociRuntime": {"name": "runc", "path": "/usr/bin/runc"},
        }
        self.responses: dict[str, str] = {}

    def call(
        self,
        argv: tuple[str, ...],
        stage: str,
        timeout: int = 30,
        *,
        cwd: Path | None = None,
        while_running: Callable[[], None] | None = None,
        max_output_bytes: int = 2_000_000,
    ) -> str:
        self.calls.append((argv, stage, timeout))
        if self.cancel_stage == stage:
            raise validation.RunCancelled("workflow cancellation")
        if self.failure == stage:
            raise validation.ValidationError(f"{stage} failed")
        if stage in self.responses:
            return self.responses[stage]
        if stage == "podman-package-owner":
            return "podman: /usr/bin/podman"
        if stage == "podman-package-version":
            return "4.9.3+ds1-1ubuntu0.1"
        if stage == "podman-rootful-prerequisite":
            return "0"
        if stage == "engine-process-uid":
            return "0" if self.outer_mode == "rootful" else str(os.getuid())
        if stage == "engine-info":
            if self.engine == "podman":
                return json.dumps(
                    {
                        "host": {
                            "serviceIsRemote": False,
                            "arch": "amd64",
                            "security": {"rootless": self.outer_mode == "rootless"},
                            **self.profile,
                        },
                        "store": {
                            "graphDriverName": "overlay",
                            "graphRoot": str(self.podman_root),
                            "runRoot": "/run/containers/storage",
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
        if stage == "cleanup-image-list":
            return IMAGE if self.tag else ""
        if stage == "podman-storage-stat":
            return json.dumps(
                {"path": str(self.podman_root.resolve()), "device": self.podman_root.stat().st_dev}
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

    def test_single_clone_defaults_preserve_independent_seven_repository_ci_probe(self) -> None:
        self.config.pop("workspaceFolder", None)
        self.config.pop("workspaceMount", None)
        self.config["mounts"] = [
            item for item in self.config["mounts"] if item.endswith("type=volume")
        ]
        root = self.write_config(self.config)
        self.config = validation.validate_configuration(root, "boxferry")
        self.assertEqual(self.config["workspaceFolder"], "/workspaces/boxferry")
        self.assertEqual(len(self.config["mounts"]), 2)
        report, fake = self.simulate()
        self.assertEqual(report["status"], "success", report)
        sources = report["workspace_sources"]
        assert isinstance(sources, dict)
        self.assertEqual(set(sources), set(validation.ALL_REPOSITORIES))
        create = next(argv for argv, stage, _ in fake.calls if stage == "create-probe")
        binds = [value for value in create if value.startswith("type=bind,")]
        self.assertEqual(len(binds), 7)
        for repository in validation.ALL_REPOSITORIES:
            target = (
                "/workspaces/boxferry"
                if repository == "boxferry"
                else f"/workspaces/boxferry/.boxferry-workspace/{repository}"
            )
            self.assertTrue(any(f"target={target},readonly" in mount for mount in binds))
            self.assertIn(repository, create[-3])

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

    def test_compact_report_retains_candidate_and_both_failure_causes(self) -> None:
        report = {
            "schema": 1,
            "status": "failure",
            "mode": "hosted",
            "engine": "podman",
            "outer_mode": "rootful",
            "caller_uid": 1000,
            "run_nonce": "a" * 32,
            "candidate_sha": SHA,
            "policy_sha": POLICY,
            "automation_sha": AUTOMATION,
            "input_sha256": INPUT,
            "build_context_sha256": INPUT,
            "cause": "budget exceeded",
            "cleanup_cause": "owned cleanup failed",
            "retained_private_directory": "/owned/scratch",
            "large": "x" * validation.MAX_REPORT,
        }
        compact = json.loads(validation.serialize_report(report))
        for key, value in report.items():
            if key not in {"large", "cause"}:
                self.assertEqual(compact[key], value)
        self.assertEqual(compact["primary_cause"], report["cause"])
        self.assertLess(len(validation.serialize_report(report).encode()), validation.MAX_REPORT)

    def test_hosted_podman_signed_apt_exception_is_explicit_and_rootful_is_mandatory(self) -> None:
        environment = {
            "GITHUB_ACTIONS": "true",
            "RUNNER_ENVIRONMENT": "github-hosted",
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
            validation.validate_engine(facts, "podman", "amd64")

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

    def test_failure_evidence_retains_early_causes_from_both_streams_and_redacts_before_cut(
        self,
    ) -> None:
        private = self.root / "causal-diagnostics"
        commands = validation.Commands(private)
        automation = self.root / "outside-private-automation"
        commands.evidence_paths = (automation,)
        secret = "synthetic-private-boundary-secret"
        program = (
            "import sys; "
            "print('progress' * 800); "
            "print('fatal: missing build dependency ' + sys.argv[1]); "
            "print('stdout-tail' * 800); "
            "print('error: permission denied at ' + sys.argv[2], file=sys.stderr); "
            "print('error: Command failed at ' + sys.argv[3], file=sys.stderr); "
            "print('stack-frame' * 3000, file=sys.stderr); sys.exit(7)"
        )
        with patch.dict(os.environ, {"TEST_TOKEN": secret}):
            with self.assertRaisesRegex(ValueError, "missing build dependency"):
                commands.call(
                    (sys.executable, "-c", program, secret, str(private), str(automation)), "build"
                )
        evidence = commands.outcomes[-1]["failure_evidence"]
        self.assertIsInstance(evidence, dict)
        assert isinstance(evidence, dict)
        self.assertIn("missing build dependency", str(evidence["stdout"]))
        self.assertIn("permission denied", str(evidence["stderr"]))
        encoded = json.dumps(commands.outcomes)
        self.assertNotIn(secret, encoded)
        self.assertNotIn(str(private), encoded)
        self.assertNotIn(str(automation), encoded)
        self.assertLess(len(encoded.encode()), 10000)
        for value in evidence.values():
            self.assertLessEqual(len(value["excerpt"].encode()), validation.MAX_FAILURE_STREAM)
            self.assertTrue(value["truncated"])
        multibyte = "ä" * 253 + secret + "fatal: unavailable dependency\n" + "界" * 4000
        safe = validation.failure_evidence(validation.redact(multibyte, (secret,)))
        self.assertLessEqual(len(str(safe["excerpt"]).encode()), validation.MAX_FAILURE_STREAM)
        self.assertNotIn(secret[:8], json.dumps(safe))
        self.assertIn("unavailable dependency", str(safe))

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

    def test_hosted_backends_are_explicit_and_local_qualifier_is_retired(self) -> None:
        for owner in validation.OWNERS:
            engine = "podman" if owner in {"podman-lens", "quadlet-lens"} else "docker"
            self.assertEqual(validation.backend(owner, "hosted", engine), engine)
            with self.assertRaisesRegex(ValueError, "no fallback"):
                validation.backend(owner, "hosted", "docker" if engine == "podman" else "podman")
            with self.assertRaisesRegex(ValueError, "VS Code"):
                validation.backend(owner, "local", "podman")
        from boxferry_ci.__main__ import main

        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            main(["devcontainer-local"])

    def test_hosted_podman_launcher_uses_runner_defaults_and_root_owned_deadline(self) -> None:
        directory = self.root / "launcher"
        directory.mkdir()
        script = Path(validation._engine(directory, "podman")).read_text()
        self.assertIn(
            '/usr/bin/sudo -n /usr/bin/timeout --signal=TERM --kill-after=2s "${seconds}s"', script
        )
        self.assertIn(
            "/usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin HOME=/root LC_ALL=C.UTF-8", script
        )
        self.assertIn('/usr/bin/podman --remote=false "$@"', script)
        for forbidden in (
            "--root",
            "--runroot",
            "--tmpdir",
            "--storage-driver",
            "XDG_",
            "CONTAINERS_",
            "cgroupfs",
            "runc",
            "unshare",
        ):
            self.assertNotIn(forbidden, script)
        self.assertFalse((directory / "containers.conf").exists())

    def test_podman_accepts_observed_ordinary_storage_and_requires_native_rootful_backend(
        self,
    ) -> None:
        facts = {
            "host": {"serviceIsRemote": False, "arch": "amd64", "security": {"rootless": False}},
            "store": {
                "graphRoot": "/runner/storage",
                "runRoot": "/run/containers/storage",
                "graphDriverName": "overlay",
            },
        }
        validation.validate_engine(facts, "podman", "amd64")
        for section, key, value in (
            ("host", "serviceIsRemote", True),
            ("host", "arch", "arm64"),
            ("host", "security", {"rootless": True}),
            ("store", "graphRoot", "relative"),
            ("store", "graphRoot", None),
            ("store", "runRoot", ""),
            ("store", "graphDriverName", ""),
        ):
            modified = json.loads(json.dumps(facts))
            modified[section][key] = value
            with self.subTest(section=section, key=key, value=value), self.assertRaises(ValueError):
                validation.validate_engine(modified, "podman", "amd64")

    def test_qualification_requires_explicit_github_hosted_linux_runner(self) -> None:
        admitted = {
            "GITHUB_ACTIONS": "true",
            "RUNNER_ENVIRONMENT": "github-hosted",
            "RUNNER_OS": "Linux",
        }
        validation.validate_hosted_runner(admitted)
        for key, value in (
            ("GITHUB_ACTIONS", "false"),
            ("RUNNER_ENVIRONMENT", "self-hosted"),
            ("RUNNER_OS", "macOS"),
        ):
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "GitHub-hosted Linux"):
                validation.validate_hosted_runner({**admitted, key: value})

    def test_podman_scratch_observation_errors_fail_before_privileged_stat(self) -> None:
        regular = self.root / "budget-file"
        regular.write_text("preserve")
        for directory in (regular, self.root / "missing"):
            fake = FakeCommands(self.root)
            with (
                self.subTest(directory=directory),
                self.assertRaisesRegex(ValueError, "budget coverage"),
            ):
                validation.validate_podman_budget_filesystem(fake, directory, "/runner/storage", {})
            self.assertFalse(fake.calls)
        fake = FakeCommands(self.root)
        with patch.object(Path, "stat", side_effect=PermissionError("metadata denied")):
            with self.assertRaisesRegex(ValueError, "metadata denied"):
                validation.validate_podman_budget_filesystem(fake, self.root, "/runner/storage", {})
        self.assertFalse(fake.calls)

    def test_podman_standard_store_success_records_observed_runtime_without_selecting_it(
        self,
    ) -> None:
        report, fake = self.simulate(engine="podman")
        self.assertEqual(report["status"], "success", report)
        self.assertEqual(report["engine_graph_root"], str(fake.podman_root))
        self.assertEqual(report["engine_runtime"], {"name": "runc", "path": "/usr/bin/runc"})
        self.assertEqual((fake.podman_root / "unrelated-cache").read_text(), "preserve")
        for argv, stage, _ in fake.calls:
            if stage == "cleanup-container":
                self.assertEqual(argv[1:], ("rm", "--force", CONTAINER))
            if stage == "cleanup-image-tag":
                self.assertEqual(argv[1:], ("image", "rm", fake.tag))
        self.assertNotIn("docker_storage", report)

    def test_podman_storage_budget_records_actual_device_without_rebaselining(self) -> None:
        budget = validation.Budget(self.root, 13 * validation.GIB, 10**20)
        fake = FakeCommands(self.root)
        fake.budget = budget
        report: dict[str, object] = {}
        validation.validate_podman_budget_filesystem(fake, self.root, "/runner/storage", report)
        facts = report["podman_storage"]
        assert isinstance(facts, dict)
        self.assertTrue(facts["same_filesystem"])
        self.assertEqual(facts["reported_graph_root"], "/runner/storage")
        self.assertEqual(facts["podman_device"], self.root.stat().st_dev)
        self.assertEqual(budget.initial_free, 13 * validation.GIB)
        self.assertIs(fake.budget, budget)
        argv, stage, timeout = fake.calls[0]
        self.assertEqual((stage, timeout), ("podman-storage-stat", 8))
        self.assertEqual(argv[-1], "/runner/storage")
        self.assertEqual(argv[-2], validation.PODMAN_STORAGE_STAT)
        self.assertEqual(argv[:6], validation._hosted_root_command(5)[:6])

    def test_podman_storage_mismatch_malformed_or_observation_failure_precedes_build(self) -> None:
        for response in (
            "{}",
            "[]",
            "not json",
            json.dumps({"path": "/runner/storage", "device": self.root.stat().st_dev + 1}),
            json.dumps({"path": "relative", "device": self.root.stat().st_dev}),
            json.dumps({"path": "/runner/storage", "device": True}),
        ):
            with self.subTest(response=response):
                report, fake = self.simulate(
                    engine="podman", responses={"podman-storage-stat": response}
                )
                self.assertEqual(report["status"], "failure")
                self.assertIn("Hosted Podman budget coverage", str(report["cause"]))
                self.assertEqual(fake.checkouts, [])
                self.assertFalse(any(stage == "build" for _, stage, _ in fake.calls))
                self.assertTrue((fake.podman_root / "unrelated-cache").is_file())
        report, fake = self.simulate("podman-storage-stat", engine="podman")
        self.assertEqual(report["status"], "failure")
        self.assertEqual(fake.checkouts, [])
        self.assertIn("podman-storage-stat", str(report["cause"]))

    def test_podman_graphroot_permission_failure_retains_actionable_diagnostics(self) -> None:
        commands = validation.Commands(self.root / "commands")
        result = RunResult(
            "failed",
            1,
            "",
            "PermissionError: graphRoot metadata denied",
            0,
            Diagnostic(
                "podman-storage-stat", "command exited 1", "graphRoot", "hosted", "verify storage"
            ),
            ("trusted-metadata-observer",),
        )
        report: dict[str, object] = {}
        with patch.object(validation, "run", return_value=result):
            with self.assertRaisesRegex(ValueError, "graphRoot metadata denied"):
                validation.validate_podman_budget_filesystem(
                    commands, self.root, "/runner/storage", report
                )
        self.assertEqual(commands.outcomes[0]["stage"], "podman-storage-stat")
        self.assertEqual(commands.outcomes[0]["status"], "failed")
        facts = report["podman_storage"]
        assert isinstance(facts, dict)
        self.assertIsNone(facts["same_filesystem"])

    def test_podman_metadata_program_rejects_missing_and_non_directory_paths(self) -> None:
        regular = self.root / "regular-file"
        regular.write_text("preserve")
        for path in (self.root / "missing", regular):
            result = run(
                (sys.executable, "-I", "-S", "-c", validation.PODMAN_STORAGE_STAT, str(path)),
                cwd=self.root,
                stage="stat-fixture",
                timeout_seconds=5,
            )
            self.assertEqual(result.status, "failed")
        result = run(
            (sys.executable, "-I", "-S", "-c", validation.PODMAN_STORAGE_STAT, str(self.root)),
            cwd=self.root,
            stage="stat-fixture",
            timeout_seconds=5,
        )
        self.assertEqual(result.status, "success")
        self.assertEqual(
            json.loads(result.stdout),
            {"path": str(self.root.resolve()), "device": self.root.stat().st_dev},
        )

    def test_cancelled_build_and_probe_preserve_failure_and_only_clean_owned_resources(
        self,
    ) -> None:
        for stage in ("build", "tooling-workspace-probe"):
            report, fake = self.simulate(engine="podman", cancel_stage=stage)
            self.assertEqual(report["status"], "failure")
            self.assertIn("workflow cancellation", str(report["cause"]))
            self.assertEqual(report["cleanup"], "success")
            self.assertTrue((fake.podman_root / "unrelated-cache").is_file())
            self.assertFalse(any("cleanup-store" in name for _, name, _ in fake.calls))

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
        responses: dict[str, str] | None = None,
        cancel_stage: str = "",
    ) -> tuple[dict[str, object], FakeCommands]:
        fake = FakeCommands(self.root)
        fake.engine, fake.mode = engine, mode
        fake.responses = responses or {}
        fake.cancel_stage = cancel_stage
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
            self.addCleanup(shutil.rmtree, private, ignore_errors=True)
            return fake

        with ExitStack() as stack:
            stack.enter_context(
                patch.object(
                    validation,
                    "prepare_hosted_podman",
                    return_value={"installation": "test-prerequisite"},
                )
            )
            stack.enter_context(
                patch.dict(
                    os.environ,
                    {
                        "GITHUB_ACTIONS": "true",
                        "RUNNER_ENVIRONMENT": "github-hosted",
                        "RUNNER_OS": "Linux",
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

    def test_backend_cache_mounts_preserve_nonroot_ownership_and_isolation(self) -> None:
        # Independent expected argv from Podman v4.9.3/v6.1.4 mount references;
        # no native parser or runtime execution is claimed by this simulation.
        targets = {
            "CARGO_HOME": "/workspaces/.boxferry-cargo",
            "GH_CONFIG_DIR": "/workspaces/.boxferry-gh",
            "CARGO_TARGET_DIR": "/workspaces/.boxferry-target",
        }
        self.config["containerEnv"].update(targets)
        for engine in ("docker", "podman"):
            mode = "hosted"
            with (
                self.subTest(engine=engine),
                patch.object(validation, "passwd_identity", return_value=("1201", "1302")),
            ):
                result, fake = self.simulate(engine=engine, mode=mode)
                self.assertEqual(result["status"], "success", result)
                create = next(argv for argv, stage, _ in fake.calls if stage == "create-probe")
                options = {
                    flag: create[create.index(flag) + 1]
                    for flag in (
                        "--user",
                        "--network",
                        "--cap-drop",
                        "--security-opt",
                        "--pids-limit",
                        "--memory",
                        "--cpus",
                    )
                }
                self.assertEqual(options["--user"], "1201:1302")
                self.assertEqual(options["--network"], "none")
                self.assertEqual(options["--cap-drop"], "ALL")
                self.assertEqual(options["--security-opt"], "no-new-privileges")
                self.assertEqual(options["--pids-limit"], "512")
                self.assertEqual(options["--memory"], "4g")
                self.assertEqual(options["--cpus"], "2")
                self.assertIn("--read-only", create)
                self.assertIn("/tmp:rw,nosuid,nodev,size=256m", create)
                for target in targets.values():
                    expected = (
                        f"type=tmpfs,target={target},rw,nosuid,nodev,"
                        "tmpfs-size=268435456,tmpfs-mode=0700,chown=true"
                        if engine == "podman"
                        else f"{target}:rw,nosuid,nodev,uid=1201,gid=1302,mode=0700,size=256m"
                    )
                    index = create.index(expected)
                    self.assertEqual(
                        create[index - 1], "--mount" if engine == "podman" else "--tmpfs"
                    )
                if engine == "podman":
                    self.assertFalse(any("uid=" in value or "gid=" in value for value in create))
                binds = [value for value in create if value.startswith("type=bind,")]
                self.assertEqual(len(binds), 7 if engine == "docker" else 1)
                self.assertTrue(all(value.endswith(",readonly") for value in binds))
                self.assertNotIn("--privileged", create)

    def test_absent_cache_targets_are_not_mounted_and_create_failure_stays_failed(self) -> None:
        self.config["containerEnv"] = {"CARGO_HOME": "/workspaces/.only-cargo"}
        result, fake = self.simulate(engine="podman")
        self.assertEqual(result["status"], "success", result)
        create = next(argv for argv, stage, _ in fake.calls if stage == "create-probe")
        self.assertEqual(sum(value.startswith("type=tmpfs,") for value in create), 1)
        self.assertIn(
            "type=tmpfs,target=/workspaces/.only-cargo,rw,nosuid,nodev,"
            "tmpfs-size=268435456,tmpfs-mode=0700,chown=true",
            create,
        )
        result, fake = self.simulate("create-probe", engine="podman")
        self.assertEqual(result["status"], "failure")
        self.assertIn("create-probe failed", str(result["cause"]))
        self.assertEqual(sum(stage == "create-probe" for _, stage, _ in fake.calls), 1)
        self.assertFalse(any(stage == "tooling-workspace-probe" for _, stage, _ in fake.calls))

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

    def test_failed_build_without_final_tag_preserves_standard_store_and_cache(self) -> None:
        report, fake = self.simulate("build", engine="podman")
        self.assertEqual(report["status"], "failure")
        self.assertIn("build failed", str(report["cause"]))
        self.assertEqual(report["cleanup"], "success")
        self.assertEqual(report["cleanup_image_tag"], "absent-after-failed-build")
        self.assertEqual((fake.podman_root / "unrelated-cache").read_text(), "preserve")
        self.assertFalse(fake.private.exists())
        for argv, stage, _ in fake.calls:
            self.assertNotIn("prune", argv)
            self.assertNotIn("unshare", argv)
            self.assertNotIn("cleanup-store", stage)
            self.assertNotIn("rm", argv if stage == "podman-storage-stat" else ())

    def test_missing_successful_build_tag_and_unexpected_inspection_errors_fail_cleanup(
        self,
    ) -> None:
        for failure, responses in (
            ("", {"cleanup-image-list": ""}),
            ("cleanup-image-list", {}),
            ("cleanup-image-identity", {}),
            ("", {"cleanup-image-identity": "not-json"}),
            ("", {"cleanup-image-identity": json.dumps([{"Id": "sha256:" + "f" * 64}])}),
            ("", {"cleanup-image-list": "unrelated:tag"}),
        ):
            with self.subTest(failure=failure, responses=responses):
                report, fake = self.simulate(failure, engine="podman", responses=responses)
                self.assertEqual(report["status"], "failure")
                self.assertEqual(report["cleanup"], "failure")
                self.assertTrue(fake.private.exists())
                self.assertFalse(
                    any(stage == "cleanup-private-store" for _, stage, _ in fake.calls)
                )

    def test_exact_qualified_tag_listing_binds_full_image_identity(self) -> None:
        report, fake = self.simulate(
            responses={"cleanup-image-list": IMAGE.removeprefix("sha256:")}
        )
        self.assertEqual(report["cleanup"], "success")
        self.assertTrue(fake.tag.startswith("localhost/boxferry-devcontainer:"))
        argv = next(argv for argv, stage, _ in fake.calls if stage == "cleanup-image-list")
        self.assertIn("reference=" + fake.tag, argv)
        self.assertIn("--no-trunc", argv)
        self.assertEqual(argv[-1], "{{.ID}}")

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
