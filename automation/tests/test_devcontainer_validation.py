"""No image execution: independent producer contracts and failed-engine simulations."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import sys
import tempfile
import textwrap
import time
import unittest
from collections.abc import Callable
from contextlib import ExitStack
from pathlib import Path
from typing import Any
from unittest.mock import Mock, call, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from boxferry_ci import devcontainer_lifetime as lifetime
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
        if stage in {"engine-info", "cleanup-store-identity"}:
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
        if stage == "cleanup-image-list":
            return IMAGE if self.tag else ""
        if stage in {
            "cleanup-store-containers",
            "cleanup-store-image-mounts",
            "cleanup-store-container-mounts",
        }:
            return "[]"
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

    def test_rootful_operator_mode_is_local_only_and_never_inferred_from_euid(self) -> None:
        self.assertEqual(validation.effective_outer_mode("local", "podman"), "rootless")
        self.assertEqual(validation.effective_outer_mode("local", "podman", "rootful"), "rootful")
        for mode, engine, choice in (
            ("hosted", "podman", "rootful"),
            ("hosted", "docker", "rootless"),
            ("local", "docker", "rootful"),
            ("local", "podman", "auto"),
        ):
            with (
                self.subTest(mode=mode, engine=engine, choice=choice),
                self.assertRaises(ValueError),
            ):
                validation.effective_outer_mode(mode, engine, choice)
        with (
            patch.object(os, "getuid", return_value=0),
            patch.object(os, "geteuid", return_value=0),
            self.assertRaises(ValueError),
        ):
            validation.effective_outer_mode("local", "podman", "rootful")

    def test_local_rootful_bridge_is_byte_equivalent_to_hosted_at_same_private_path(self) -> None:
        private = self.root / "bridge"
        private.mkdir()
        hosted = Path(validation._engine(private, "podman", "hosted")).read_bytes()
        local = Path(validation._engine(private, "podman", "local", rootful=True)).read_bytes()
        self.assertEqual(local, hosted)
        text = local.decode()
        self.assertIn("/usr/bin/env -i", text)
        self.assertIn("--storage-driver overlay", text)
        self.assertNotIn("cgroup", text)
        self.assertNotIn("CONTAINER_HOST", text)

    def test_installed_podman_guard_runs_before_outer_root_namespace_launch(self) -> None:
        events: list[str] = []

        def guard() -> dict[str, object]:
            events.append("guard")
            raise validation.ValidationError("untrusted installed Podman")

        with (
            patch.object(validation, "installed_podman", side_effect=guard),
            patch.object(lifetime, "execute") as launch,
            self.assertRaisesRegex(ValueError, "untrusted installed Podman"),
        ):
            validation.qualify(
                repository="compose-lens",
                candidate=SHA,
                policy_sha=POLICY,
                automation_sha=AUTOMATION,
                source=self.root,
                policy=self.root,
                automation=self.root,
                manifest={"schema": 1, "repositories": {"compose-lens": SHA}},
                engine="podman",
                mode="local",
                work=self.root,
                outer_mode="rootful",
            )
        self.assertEqual(events, ["guard"])
        launch.assert_not_called()

    def test_installed_podman_observation_precedes_any_rootful_engine_call(self) -> None:
        report, fake = self.simulate(
            engine="podman", mode="local", outer_mode="rootful", installed_guard_failure=True
        )
        self.assertEqual(report["status"], "failure")
        self.assertIn("installed Podman custody failed", str(report["cause"]))
        self.assertFalse(any(stage.startswith("engine-") for _, stage, _ in fake.calls))
        self.assertFalse(fake.checkouts)

    def test_installed_podman_fixed_path_parent_custody_and_observed_integrity(self) -> None:
        executable = self.root / "podman"
        executable.write_bytes(b"inert installed binary fixture")
        executable.chmod(0o755)
        original_stat = Path.stat
        version = RunResult(
            "success",
            0,
            "podman version 5.0.0\n",
            "",
            0,
            Diagnostic("installed-podman-version", "", "", "", ""),
            (),
        )
        for fault in (
            "none",
            "owner",
            "parent-owner",
            "parent-write",
            "file-write",
            "no-exec",
            "privileged-mode",
            "not-regular",
            "version",
            "failed",
            "drift",
        ):
            calls = [0]

            def facts(
                path: Path,
                *,
                follow_symlinks: bool = True,
                fault: str = fault,
                calls: list[int] = calls,
            ) -> os.stat_result:
                actual = original_stat(path, follow_symlinks=follow_symlinks)
                values: list[Any] = list(actual)
                if path != executable:
                    values[0] &= ~0o022
                values[4] = 0
                if fault == "owner" and path == executable:
                    values[4] = os.getuid()
                if fault == "parent-owner" and path == executable.parent:
                    values[4] = os.getuid()
                if fault == "parent-write" and path == executable.parent:
                    values[0] |= 0o020
                if fault == "file-write" and path == executable:
                    values[0] |= 0o002
                if fault == "no-exec" and path == executable:
                    values[0] &= ~0o111
                if fault == "privileged-mode" and path == executable:
                    values[0] |= stat.S_ISUID
                if fault == "not-regular" and path == executable:
                    values[0] = stat.S_IFDIR | 0o755
                if fault == "drift" and path == executable:
                    calls[0] += 1
                    if calls[0] > 1:
                        values[6] += 1
                return os.stat_result(values)

            observed = version
            if fault in ("version", "failed"):
                observed = RunResult(
                    "success" if fault == "version" else "failed",
                    1,
                    "unknown version",
                    "",
                    0,
                    version.diagnostic,
                    (),
                )
            with (
                self.subTest(fault=fault),
                patch.object(
                    validation,
                    "_podman_path_custody",
                    side_effect=lambda: validation._root_owned_executable(executable),
                ),
                patch.object(Path, "stat", side_effect=facts, autospec=True),
                patch.object(validation, "run", return_value=observed) as invoke,
            ):
                if fault != "none":
                    with self.assertRaises((OSError, ValueError)):
                        validation.installed_podman()
                else:
                    receipt = validation.installed_podman()
                    self.assertEqual(receipt["path"], "/usr/bin/podman")
                    self.assertEqual(
                        receipt["sha256"], hashlib.sha256(executable.read_bytes()).hexdigest()
                    )
                    self.assertEqual(receipt["version"], "podman version 5.0.0")
                    self.assertTrue(receipt["path_custody"])
                    self.assertEqual(invoke.call_args.args[0], (str(executable), "--version"))
                    self.assertNotIn("sudo", str(invoke.call_args))

    def test_installed_podman_intermediate_symlink_parent_custody_is_checked(self) -> None:
        trusted = self.root / "trusted"
        untrusted = self.root / "untrusted"
        trusted.mkdir()
        untrusted.mkdir()
        binary = trusted / "binary"
        binary.write_text("inert fixture")
        binary.chmod(0o755)
        (untrusted / "redirect").symlink_to(binary)
        selected = trusted / "podman"
        selected.symlink_to(untrusted / "redirect")
        original_stat = Path.stat

        def facts(path: Path, *, follow_symlinks: bool = True) -> os.stat_result:
            actual = original_stat(path, follow_symlinks=follow_symlinks)
            values: list[Any] = list(actual)
            values[4] = os.getuid() if path == untrusted else 0
            if not stat.S_ISLNK(actual.st_mode):
                values[0] &= ~0o022
            return os.stat_result(values)

        with (
            patch.object(Path, "stat", side_effect=facts, autospec=True),
            self.assertRaisesRegex(ValueError, "parent custody"),
        ):
            validation._root_owned_executable(selected)
        with patch.object(
            validation, "_root_owned_executable", side_effect=ValueError("stop")
        ) as observe:
            with self.assertRaisesRegex(ValueError, "stop"):
                validation._podman_path_custody()
        observe.assert_called_once_with(Path("/usr/bin/podman"))

    def test_local_rootful_observes_uid_and_uses_only_finite_rootful_cleanup(self) -> None:
        report, fake = self.simulate(engine="podman", mode="local", outer_mode="rootful")
        self.assertEqual(report["status"], "success", report)
        self.assertEqual(report["outer_mode"], "rootful")
        self.assertEqual(report["caller_uid"], os.getuid())
        self.assertEqual(report["engine_uid"], 0)
        self.assertEqual(report["engine_storage_driver"], "overlay")
        self.assertEqual(report["engine_graph_root"], str(fake.private / "store"))
        self.assertEqual(report["engine_run_root"], str(fake.private / "runroot"))
        self.assertIs(report["engine_remote"], False)
        self.assertEqual(report["engine_cgroup_manager"], "cgroupfs")
        self.assertEqual(report["engine_runtime"], "runc")
        observer = next(argv for argv, stage, _ in fake.calls if stage == "engine-process-uid")
        self.assertEqual(
            observer[:6],
            ("/usr/bin/sudo", "-n", "/usr/bin/timeout", "--signal=TERM", "--kill-after=2s", "5s"),
        )
        self.assertEqual(observer[6:8], ("/usr/bin/env", "-i"))
        self.assertEqual(observer[-2:], ("/usr/bin/id", "-u"))
        self.assertFalse(
            any(stage == "cleanup-private-rootful-store" for _, stage, _ in fake.calls)
        )
        self.assertEqual(report["directory_cleanup"], "deferred-to-outer")
        self.assertEqual(report["engine_resource_cleanup"], "success")
        self.assertTrue(fake.private.exists())
        self.assertFalse(
            any(
                "unshare" in argv or "apt-get" in str(argv) or "dpkg" in str(argv)
                for argv, _, _ in fake.calls
            )
        )
        self.assertFalse(any(stage == "cleanup-private-store" for _, stage, _ in fake.calls))

    def test_local_rootful_wrong_uid_and_store_or_cleanup_failures_retain_failure(self) -> None:
        for uid in ("", "1000", "unknown"):
            with self.subTest(uid=uid):
                report, fake = self.simulate(
                    engine="podman",
                    mode="local",
                    outer_mode="rootful",
                    responses={"engine-process-uid": uid},
                )
                self.assertEqual(report["status"], "failure", report)
                self.assertFalse(fake.checkouts)
        for stage in ("build", "cleanup-store-containers", "cleanup-store-identity"):
            with self.subTest(stage=stage):
                report, fake = self.simulate(
                    stage, engine="podman", mode="local", outer_mode="rootful"
                )
                self.assertEqual(report["status"], "failure", report)
                if stage != "build":
                    self.assertEqual(report["cleanup"], "failure")
                    self.assertTrue(fake.private.exists())

    def test_local_missing_podman_has_no_installer_or_privilege_fallback(self) -> None:
        report, fake = self.simulate(
            engine="podman", mode="local", outer_mode="rootful", installed_podman=False
        )
        self.assertEqual(report["status"], "failure", report)
        self.assertIn("installed /usr/bin/podman", str(report["cause"]))
        self.assertFalse(fake.calls)
        self.assertFalse(fake.checkouts)

    def test_rootful_failed_build_aborts_without_another_engine_cleanup_budget(self) -> None:
        report, fake = self.simulate("build", engine="podman", mode="local", outer_mode="rootful")
        self.assertEqual(report["status"], "failure", report)
        self.assertIn("build failed", str(report["cause"]))
        self.assertEqual(report["cleanup"], "pending-outer-closure")
        self.assertEqual(report["directory_cleanup"], "deferred-to-outer")
        self.assertTrue(fake.private.exists())
        build = next(index for index, (_, stage, _) in enumerate(fake.calls) if stage == "build")
        self.assertEqual(len(fake.calls), build + 1, "abort cannot start late engine cleanup")

    def test_local_runtime_accepts_only_exact_same_binary_name_and_path_spellings(self) -> None:
        for name, name_kind in (
            ("runc", "alias-runc"),
            ("/usr/bin/runc", "selected-path"),
            ("/usr/sbin/runc", "resolved-path"),
        ):
            for path, path_kind in (
                ("/usr/bin/runc", "selected-path"),
                ("/usr/sbin/runc", "resolved-path"),
            ):
                with self.subTest(name=name, path=path):
                    report, fake = self.simulate(
                        engine="podman",
                        mode="local",
                        outer_mode="rootful",
                        profile={"ociRuntime": {"name": name, "path": path}},
                    )
                    self.assertEqual(report["status"], "success", report)
                    self.assertEqual(report["engine_runtime"], "runc")
                    self.assertEqual(report["engine_runtime_name_representation"], name_kind)
                    self.assertEqual(report["engine_runtime_path_representation"], path_kind)
                    self.assertTrue(fake.checkouts)
        receipt = {"path": "/usr/bin/runc", "resolved_path": "/usr/bin/runc"}
        for field in ("name", "path"):
            self.assertEqual(
                validation._runtime_representation({field: "/usr/bin/runc"}, field, receipt),
                "selected-path",
            )
        self.assertIsNone(validation.LIVE_PID_SCHEMA)

    def test_local_runtime_rejects_unknown_evidence_before_build_without_retaining_it(self) -> None:
        invalid: tuple[tuple[object, str], ...] = (
            (None, "invalid-type"),
            (True, "invalid-type"),
            (False, "invalid-type"),
            (1, "invalid-type"),
            (1.5, "invalid-type"),
            ([], "invalid-type"),
            ({}, "invalid-type"),
            ("", "other"),
            ("RUNC", "other"),
            ("runc ", "other"),
            (" runc", "other"),
            ("usr/bin/runc", "other"),
            ("./runc", "other"),
            ("/foreign/runc", "other"),
            ("/usr/bin/crun", "other"),
            ("crun", "other"),
            ("/usr/bin/../bin/runc", "other"),
            ("/usr//bin/runc", "other"),
            ("/usr/bin/runc/", "other"),
            ("/usr/bin/runc-canary-secret", "other"),
            ("/usr/bin/runc ", "other"),
            (" /usr/bin/runc", "other"),
            ("/usr/sbin/../sbin/runc", "other"),
            ("/usr//sbin/runc", "other"),
        )
        for field in ("name", "path"):
            for missing, value, expected in [
                (True, None, "missing"),
                *((False, value, expected) for value, expected in invalid),
            ]:
                runtime = {"name": "runc", "path": "/usr/bin/runc"}
                if missing:
                    del runtime[field]
                else:
                    runtime[field] = value
                with self.subTest(field=field, value=value, missing=missing):
                    report, fake = self.simulate(
                        engine="podman",
                        mode="local",
                        outer_mode="rootful",
                        profile={"ociRuntime": runtime},
                    )
                    self.assertEqual(report["status"], "failure", report)
                    self.assertEqual(report[f"engine_runtime_{field}_representation"], expected)
                    self.assertFalse(fake.checkouts)
                    self.assertFalse(any(stage == "build" for _, stage, _ in fake.calls))
                    self.assertNotIn("engine_runtime", report)
                    self.assertEqual(report["cleanup"], "pending-outer-closure")
                    self.assertTrue(fake.abort_reports)
                    compact = json.loads(
                        validation.serialize_report(
                            {**report, "large": "x" * validation.MAX_REPORT}
                        )
                    )
                    for key in (
                        "engine_runtime_name_representation",
                        "engine_runtime_path_representation",
                    ):
                        self.assertEqual(compact[key], report[key])
                        self.assertEqual(fake.abort_reports[0][key], report[key])
                    self.assertEqual(compact["primary_cause"], report["cause"])
                    sinks = json.dumps([report, compact, fake.abort_reports, fake.outcomes])
                    self.assertNotIn("runc-canary-secret", sinks)
        for runtime in (None, [], True, "runc-canary-secret"):
            report, fake = self.simulate(
                engine="podman", mode="local", outer_mode="rootful", profile={"ociRuntime": runtime}
            )
            self.assertEqual(report["status"], "failure", report)
            self.assertEqual(report["engine_runtime_name_representation"], "invalid-type")
            self.assertEqual(report["engine_runtime_path_representation"], "invalid-type")
            self.assertNotIn("runc-canary-secret", json.dumps([report, fake.abort_reports]))
            self.assertFalse(fake.checkouts)

    def test_installed_runc_reuses_complete_executable_custody_and_checks_drift(self) -> None:
        executable = self.root / "runc"
        executable.write_bytes(b"inert installed runtime")
        executable.chmod(0o755)
        original_stat = Path.stat
        original_custody = validation._root_owned_executable
        for fault in (
            "none",
            "owner",
            "parent-owner",
            "parent-write",
            "file-write",
            "no-exec",
            "privileged-mode",
            "not-regular",
            "ctime",
            "content",
            "version",
            "version-failed",
            "read-limit",
            "short-read",
            "inode",
            "custody-drift",
            "resolved-drift",
        ):
            visits = [0]

            def facts(
                path: Path,
                *,
                follow_symlinks: bool = True,
                fault: str = fault,
                visits: list[int] = visits,
            ) -> os.stat_result:
                actual = original_stat(path, follow_symlinks=follow_symlinks)
                result = Mock(spec=os.stat_result)
                for attr in (
                    "st_dev",
                    "st_ino",
                    "st_size",
                    "st_mtime_ns",
                    "st_ctime_ns",
                    "st_uid",
                    "st_mode",
                ):
                    setattr(result, attr, getattr(actual, attr))
                result.st_uid = 0
                result.st_mode &= ~0o022
                if fault == "owner" and path == executable:
                    result.st_uid = os.getuid()
                if fault == "parent-owner" and path == executable.parent:
                    result.st_uid = os.getuid()
                if fault == "parent-write" and path == executable.parent:
                    result.st_mode |= 0o020
                if fault == "file-write" and path == executable:
                    result.st_mode |= 0o002
                if fault == "no-exec" and path == executable:
                    result.st_mode &= ~0o111
                if fault == "privileged-mode" and path == executable:
                    result.st_mode |= stat.S_ISUID
                if fault == "not-regular" and path == executable:
                    result.st_mode = stat.S_IFDIR | 0o755
                if fault == "read-limit" and path == executable:
                    result.st_size = 64 * 1024**2 + 1
                if fault == "short-read" and path == executable:
                    result.st_size += 1
                if visits[0] > 1 and fault == "ctime" and path == executable:
                    result.st_ctime_ns += 1
                return result

            def observe(
                selected: Path,
                *,
                fault: str = fault,
                visits: list[int] = visits,
            ) -> tuple[Path, os.stat_result, list[dict[str, object]]]:
                self.assertEqual(selected, Path("/usr/bin/runc"))
                visits[0] += 1
                resolved, observed, custody = original_custody(executable)
                if visits[0] > 1 and fault == "custody-drift":
                    custody[0]["ctime_ns"] = -1
                if visits[0] > 1 and fault == "resolved-drift":
                    resolved = self.root / "other"
                return resolved, observed, custody

            commands = Mock(spec=validation.Commands)

            def version(*_args: object, fault: str = fault) -> str:
                if fault == "version-failed":
                    raise validation.ValidationError("selected-runc-version failed")
                if fault == "content":
                    before = original_stat(executable)
                    executable.write_bytes(b"changed binary content!")
                    os.utime(executable, ns=(before.st_atime_ns, before.st_mtime_ns))
                return "crun version 1.0.0" if fault == "version" else "runc version 1.5.1"

            commands.call.side_effect = version
            real_fstat = os.fstat

            def opened(
                fd: int,
                *,
                fault: str = fault,
                real_fstat: Callable[[int], os.stat_result] = real_fstat,
            ) -> os.stat_result:
                result = real_fstat(fd)
                if fault == "inode":
                    changed = list(result)
                    changed[1] += 1
                    return os.stat_result(changed)
                return result

            with (
                self.subTest(fault=fault),
                patch.object(Path, "stat", side_effect=facts, autospec=True),
                patch.object(validation, "_root_owned_executable", side_effect=observe) as guard,
                patch.object(validation.os, "fstat", side_effect=opened),
            ):
                if fault == "none":
                    receipt = validation.installed_runc(commands)
                    self.assertEqual(receipt["path"], "/usr/bin/runc")
                    self.assertEqual(receipt["resolved_path"], str(executable))
                    self.assertEqual(
                        receipt["sha256"], hashlib.sha256(executable.read_bytes()).hexdigest()
                    )
                    self.assertTrue(receipt["path_custody"])
                    self.assertEqual(
                        guard.call_args_list,
                        [call(Path("/usr/bin/runc")), call(Path("/usr/bin/runc"))],
                    )
                else:
                    with self.assertRaises((OSError, ValueError)):
                        validation.installed_runc(commands)
            executable.write_bytes(b"inert installed runtime")

        report, fake = self.simulate(
            engine="podman", mode="local", outer_mode="rootful", installed_runtime_failure=True
        )
        self.assertEqual(report["status"], "failure", report)
        self.assertIn("installed runtime custody failed", str(report["cause"]))
        self.assertFalse(fake.checkouts)
        self.assertFalse(any(stage == "build" for _, stage, _ in fake.calls))

    def test_installed_runc_checks_symlink_parent_target_and_link_swap(self) -> None:
        trusted, untrusted = self.root / "trusted", self.root / "untrusted"
        trusted.mkdir()
        untrusted.mkdir()
        binary = trusted / "binary"
        binary.write_bytes(b"inert runtime")
        binary.chmod(0o755)
        other = trusted / "other"
        other.write_bytes(binary.read_bytes())
        other.chmod(0o755)
        redirect = untrusted / "redirect"
        redirect.symlink_to(binary)
        selected = trusted / "runc"
        selected.symlink_to(redirect)
        original_stat = Path.stat
        original_custody = validation._root_owned_executable
        for fault in ("none", "parent", "target", "link", "swap"):

            def facts(
                path: Path,
                *,
                follow_symlinks: bool = True,
                fault: str = fault,
            ) -> os.stat_result:
                actual = original_stat(path, follow_symlinks=follow_symlinks)
                values: list[Any] = list(actual)
                values[4] = (
                    os.getuid()
                    if (
                        (fault == "parent" and path == untrusted)
                        or (fault == "target" and path == binary)
                        or (fault == "link" and path == redirect)
                    )
                    else 0
                )
                if not stat.S_ISLNK(actual.st_mode):
                    values[0] &= ~0o022
                return os.stat_result(values)

            def observe(path: Path) -> tuple[Path, os.stat_result, list[dict[str, object]]]:
                self.assertEqual(path, Path("/usr/bin/runc"))
                return original_custody(selected)

            commands = Mock(spec=validation.Commands)

            def version(*_args: object, fault: str = fault) -> str:
                if fault == "swap":
                    redirect.unlink()
                    redirect.symlink_to(other)
                return "runc version 1.5.1"

            commands.call.side_effect = version
            with (
                self.subTest(fault=fault),
                patch.object(Path, "stat", side_effect=facts, autospec=True),
                patch.object(validation, "_root_owned_executable", side_effect=observe),
            ):
                if fault == "none":
                    self.assertTrue(validation.installed_runc(commands)["path_custody"])
                else:
                    with self.assertRaises((OSError, ValueError)):
                        validation.installed_runc(commands)
            redirect.unlink()
            redirect.symlink_to(binary)

    def test_local_profile_has_no_runtime_manager_or_event_fallback(self) -> None:
        profiles: tuple[dict[str, object], ...] = (
            {"cgroupManager": "systemd"},
            {"eventLogger": "journald"},
            {"ociRuntime": {"name": "crun", "path": "/usr/bin/crun"}},
            {"ociRuntime": {"name": "runc", "path": "/unreviewed/runc"}},
        )
        for profile in profiles:
            with self.subTest(profile=profile):
                report, fake = self.simulate(
                    engine="podman", mode="local", outer_mode="rootful", profile=profile
                )
                self.assertEqual(report["status"], "failure", report)
                self.assertFalse(fake.checkouts)
                self.assertFalse(any(stage == "build" for _, stage, _ in fake.calls))
        private = self.root / "profile-command"
        private.mkdir()
        script = Path(
            validation._engine(private, "podman", "local", rootful=True, local_profile=True)
        ).read_text()
        self.assertIn(
            "--cgroup-manager cgroupfs --events-backend file --runtime /usr/bin/runc", script
        )
        self.assertNotIn("systemd", script)
        self.assertNotIn("crun", script)
        with self.assertRaises(ValueError):
            validation._engine(private, "podman", "hosted", rootful=True, local_profile=True)

    def test_rootful_overlay_and_native_facts_are_independently_required(self) -> None:
        document = {
            "host": {"serviceIsRemote": False, "arch": "amd64", "security": {"rootless": False}},
            "store": {
                "graphRoot": str(self.root / "store"),
                "runRoot": str(self.root / "runroot"),
                "graphDriverName": "overlay",
            },
        }
        validation.validate_engine(
            document, "podman", "amd64", self.root, rootless=False, storage_driver="overlay"
        )
        for section, key, value in (
            ("host", "serviceIsRemote", True),
            ("host", "arch", "arm64"),
            ("host", "security", {"rootless": True}),
            ("store", "graphRoot", "/personal"),
            ("store", "runRoot", "/run/personal"),
            ("store", "graphDriverName", "vfs"),
        ):
            with self.subTest(key=key), self.assertRaises(ValueError):
                changed = {**document, section: {**document[section], key: value}}
                validation.validate_engine(
                    changed, "podman", "amd64", self.root, rootless=False, storage_driver="overlay"
                )

    def test_compact_report_retains_modes_uids_nonce_and_both_failure_causes(self) -> None:
        report = {
            "schema": 1,
            "status": "failure",
            "requested_outer_mode": "rootful",
            "outer_mode": "rootful",
            "caller_uid": 1000,
            "engine_uid": 0,
            "run_nonce": "a" * 32,
            "private_directory_identity": "1:2:1000:700",
            "cause": "budget exceeded",
            "outer_abort_stage": "local-rootful-lifetime",
            "outer_abort_status": "cancelled",
            "outer_abort_cause": "actual SIGTERM (15)",
            "outer_abort_signal_number": 15,
            "outer_abort_signal_name": "SIGTERM",
            "outer_abort_persisted": False,
            "outer_abort_receipt": "/owned/outer-abort.json",
            "outer_abort_persistence_failure": "disk failure",
            "outer_abort_receipt_disposition": "persistence/custody unproved",
            "cleanup_cause": "owned cleanup failed",
            "retained_private_directory": "/owned/private",
            "large": "x" * validation.MAX_REPORT,
        }
        compact = json.loads(validation.serialize_report(report))
        for key in (
            "requested_outer_mode",
            "outer_mode",
            "caller_uid",
            "engine_uid",
            "run_nonce",
            "private_directory_identity",
            "cleanup_cause",
            "outer_abort_stage",
            "outer_abort_status",
            "outer_abort_cause",
            "outer_abort_signal_number",
            "outer_abort_signal_name",
            "outer_abort_persisted",
            "outer_abort_receipt",
            "outer_abort_persistence_failure",
            "outer_abort_receipt_disposition",
            "retained_private_directory",
        ):
            self.assertEqual(compact[key], report[key])
        self.assertEqual(compact["primary_cause"], report["cause"])
        self.assertLess(len(validation.serialize_report(report).encode()), validation.MAX_REPORT)

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
        outer_mode: str | None = None,
        installed_podman: bool = True,
        installed_guard_failure: bool = False,
        installed_runtime_failure: bool = False,
        profile: dict[str, object] | None = None,
        responses: dict[str, str] | None = None,
    ) -> tuple[dict[str, object], FakeCommands]:
        fake = FakeCommands(self.root)
        fake.engine, fake.mode = engine, mode
        fake.outer_mode = outer_mode or ("rootful" if mode == "hosted" else "rootless")
        fake.responses = responses or {}
        fake.profile.update(profile or {})
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
            if engine == "podman":
                (private / "store").mkdir()
                if failure == "build":
                    (private / "store" / "partial-image-layers").write_text(
                        "incomplete build layers"
                    )
            return fake

        original_is_file = Path.is_file
        context: dict[str, Any] | None = None
        if mode == "local" and fake.outer_mode == "rootful":
            scope = Path(tempfile.mkdtemp(prefix="fixture-lifetime-", dir=self.root))
            scope.chmod(0o700)
            context = {"private": str(scope), "nonce": "a" * 32}

        def notify(control: dict[str, Any], failure: object, report: object) -> None:
            control["abort_notified"] = True
            assert isinstance(report, dict)
            fake.abort_reports.append(json.loads(validation.serialize_report(report)))

        def podman_guard() -> dict[str, object]:
            self.assertFalse(any(stage.startswith("engine-") for _, stage, _ in fake.calls))
            if installed_guard_failure:
                raise validation.ValidationError("installed Podman custody failed")
            return {
                "path": "/usr/bin/podman",
                "uid": 0,
                "sha256": "a" * 64,
                "version": "podman version 5.0.0",
            }

        with ExitStack() as stack:
            stack.enter_context(
                patch.object(validation, "LiveProcessObservation", return_value=Mock())
            )
            stack.enter_context(
                patch.object(validation, "installed_podman", side_effect=podman_guard)
            )
            stack.enter_context(
                patch.object(
                    lifetime,
                    "validate_worker_context",
                    side_effect=lambda control: Path(control["private"]),
                )
            )
            stack.enter_context(patch.object(lifetime, "notify_abort", side_effect=notify))
            stack.enter_context(
                patch.object(
                    validation,
                    "installed_runc",
                    side_effect=(
                        validation.ValidationError("installed runtime custody failed")
                        if installed_runtime_failure
                        else None
                    ),
                    return_value={
                        "path": "/usr/bin/runc",
                        "resolved_path": "/usr/sbin/runc",
                        "uid": 0,
                        "mode": 0o755,
                        "sha256": "f" * 64,
                        "version": "runc version 1.5.1",
                    },
                )
            )
            stack.enter_context(
                patch.object(
                    Path,
                    "is_file",
                    lambda path: (
                        installed_podman
                        if path == Path("/usr/bin/podman")
                        else original_is_file(path)
                    ),
                )
            )
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
                outer_mode=outer_mode,
                lifetime_context=context,
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
        for engine, mode, outer_mode in (
            ("docker", "hosted", None),
            ("podman", "hosted", None),
            ("podman", "local", "rootless"),
            ("podman", "local", "rootful"),
        ):
            with (
                self.subTest(engine=engine, mode=mode, outer_mode=outer_mode),
                patch.object(validation, "passwd_identity", return_value=("1201", "1302")),
                patch.object(
                    validation, "prepare_hosted_podman", return_value={"fixture": True}
                ) as prerequisite,
            ):
                result, fake = self.simulate(engine=engine, mode=mode, outer_mode=outer_mode)
                self.assertEqual(result["status"], "success", result)
                if engine == "podman" and mode == "hosted":
                    prerequisite.assert_called_once()
                else:
                    prerequisite.assert_not_called()
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
        result, fake = self.simulate(engine="podman", mode="local")
        self.assertEqual(result["status"], "success", result)
        create = next(argv for argv, stage, _ in fake.calls if stage == "create-probe")
        self.assertEqual(sum(value.startswith("type=tmpfs,") for value in create), 1)
        self.assertIn(
            "type=tmpfs,target=/workspaces/.only-cargo,rw,nosuid,nodev,"
            "tmpfs-size=268435456,tmpfs-mode=0700,chown=true",
            create,
        )
        result, fake = self.simulate("create-probe", engine="podman", mode="local")
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

    def test_failed_build_without_final_tag_cleans_only_its_idle_private_store(self) -> None:
        sentinel = self.root / "maintainer-store"
        sentinel.mkdir()
        (sentinel / "image").write_text("unrelated")
        report, fake = self.simulate("build", engine="podman", mode="local")
        self.assertEqual(report["status"], "failure")
        self.assertEqual(report["cleanup"], "success")
        self.assertIn("build failed", str(report["cause"]))
        self.assertEqual(report["cleanup_image_tag"], "absent-after-failed-build")
        self.assertFalse(fake.private.exists())
        self.assertEqual((sentinel / "image").read_text(), "unrelated")
        stages = [stage for _argv, stage, _timeout in fake.calls]
        self.assertNotIn("cleanup-image-identity", stages)
        self.assertNotIn("cleanup-image-tag", stages)
        self.assertIn("cleanup-private-store", stages)
        self.assertFalse((fake.private / "store" / "partial-image-layers").exists())
        containers = next(
            argv for argv, stage, _ in fake.calls if stage == "cleanup-store-containers"
        )
        self.assertIn("--external", containers)
        for argv, _stage, _timeout in fake.calls:
            self.assertNotIn("prune", argv)
            self.assertNotIn(str(sentinel), argv)

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
                report, fake = self.simulate(
                    failure, engine="podman", mode="local", responses=responses
                )
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

    def test_private_store_cleanup_rejects_containers_mounts_malformed_and_errors(self) -> None:
        for stage in (
            "cleanup-store-containers",
            "cleanup-store-image-mounts",
            "cleanup-store-container-mounts",
        ):
            for value in ('[{"Id":"external-build-container"}]', "null", "{}", "broken-json"):
                with self.subTest(stage=stage, value=value):
                    report, fake = self.simulate(
                        "build", engine="podman", mode="local", responses={stage: value}
                    )
                    self.assertEqual(report["status"], "failure")
                    self.assertEqual(report["cleanup"], "failure")
                    self.assertIn("build failed", str(report["cause"]))
                    self.assertTrue(fake.private.exists())
                    self.assertFalse(any(s == "cleanup-private-store" for _, s, _ in fake.calls))
            with self.subTest(stage=stage, error=True):
                report, fake = self.simulate(stage, engine="podman", mode="local")
                self.assertEqual(report["cleanup"], "failure")
                self.assertFalse(any(s == "cleanup-private-store" for _, s, _ in fake.calls))

    def test_private_store_identity_and_delete_errors_remain_failures(self) -> None:
        for stage, responses in (
            ("cleanup-store-identity", {}),
            ("", {"cleanup-store-identity": json.dumps({"host": {}, "store": {}})}),
            ("cleanup-private-store", {}),
        ):
            with self.subTest(stage=stage):
                report, fake = self.simulate(
                    stage, engine="podman", mode="local", responses=responses
                )
                self.assertEqual(report["status"], "failure")
                self.assertEqual(report["cleanup"], "failure")
                self.assertTrue(fake.private.exists())

    def test_store_path_substitution_is_rejected_before_engine_or_delete(self) -> None:
        private = self.root / "private-idle"
        private.mkdir()
        commands = FakeCommands(private)
        for identity in ((-1, -1),):
            with self.assertRaisesRegex(ValueError, "substituted"):
                validation.verify_private_store_idle(
                    commands, ("podman",), private, "amd64", True, identity
                )
        self.assertEqual(commands.calls, [])
        facts = private.stat()
        (private / "store").symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "substituted"):
            validation.verify_private_store_idle(
                commands, ("podman",), private, "amd64", True, (facts.st_dev, facts.st_ino)
            )
        self.assertEqual(commands.calls, [])

    def test_rootless_store_mount_queries_enter_namespace_without_fallback(self) -> None:
        with patch.object(validation.os, "geteuid", return_value=os.getuid()):
            report, fake = self.simulate("build", engine="podman", mode="local")
        self.assertEqual(report["cleanup"], "success")
        for argv, stage, _timeout in fake.calls:
            if stage in {"cleanup-store-image-mounts", "cleanup-store-container-mounts"}:
                self.assertEqual(
                    argv[:3],
                    (str(fake.private / "podman"), "unshare", str(fake.private / "podman")),
                )
                self.assertNotIn("--all", argv)

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


class LiveObservationContracts(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.private = self.root / "private"
        self.private.mkdir(mode=0o700)
        self.commands = validation.Commands(self.private)
        self.report: dict[str, object] = {"schema": 1, "status": "failure"}
        self.commands.lifetime_report = self.report
        self.result = self.root / "result"
        self.result.touch(mode=0o600)
        reader, writer = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
        self.addCleanup(os.close, reader)
        self.addCleanup(os.close, writer)
        self.reader, self.writer = reader, writer
        self.commands.lifetime_context = {
            "private": str(self.private),
            "nonce": "c" * 32,
            "result": str(self.result),
            "result_identity": [self.result.stat().st_dev, self.result.stat().st_ino],
            "abort_fd": writer,
            "pid_namespace": "pid:[100]",
            "mount_namespace": "mnt:[200]",
            "host_pid_namespace": "pid:[90]",
            "host_mount_namespace": "mnt:[190]",
        }
        self.commands.budget = Mock(deadline=time.monotonic() + 300, check=Mock())
        self.held = validation.LiveHeldIdentity(
            CONTAINER, IMAGE, "c" * 32, 1000, "d" * 64, "podman version 5.0.0"
        )

    def native(self, *, running: bool = True) -> list[dict[str, Any]]:
        return [
            {
                "Id": CONTAINER,
                "Image": IMAGE,
                "Config": {
                    "Labels": {"io.boxferry.devcontainer.run": "c" * 32, "unrelated": "discarded"},
                    "Env": ["not-retained"],
                },
                "State": {
                    "Running": running,
                    "Status": "running" if running else "created",
                    "Pid": 20 if running else 0,
                    "ConmonPid": 30 if running else 0,
                    "StartedAt": "2026-11-05T18:00:00.123456789Z",
                    "Error": "not-retained",
                },
            }
        ]

    def request(self) -> validation.LiveProcessRequest:
        return validation.LiveProcessRequest(
            20, 30, 1000, "pid:[100]", "mnt:[200]", "pid:[90]", "mnt:[190]"
        )

    def observed(self) -> dict[str, Any]:
        return {
            "schema": 1,
            "status": "observed",
            "observer": {
                "pid": 40,
                "uids": [0] * 4,
                "pid_namespace": "pid:[100]",
                "mount_namespace": "mnt:[200]",
                "proc_pid_view": "lifetime",
            },
            "processes": [
                {
                    "role": "container-init",
                    "pid": 20,
                    "uids": [1000] * 4,
                    "nspid": [20, 1],
                    "start_ticks": "77",
                    "pid_namespace": "pid:[101]",
                    "mount_namespace": "mnt:[201]",
                    "pidfd_alive_before": True,
                    "pidfd_alive_after": True,
                },
                {
                    "role": "conmon",
                    "pid": 30,
                    "uids": [0] * 4,
                    "nspid": [30],
                    "start_ticks": "78",
                    "pid_namespace": "pid:[100]",
                    "mount_namespace": "mnt:[333]",
                    "pidfd_alive_before": True,
                    "pidfd_alive_after": True,
                },
            ],
        }

    def inspect_run(self, argv: tuple[str, ...], **options: object) -> RunResult:
        mode = options["opaque_observation"]
        assert isinstance(mode, validation.OpaqueObservation)
        consumer = options["on_private_stdout"]
        assert callable(consumer)
        content = json.dumps(self.native()).encode()
        consumer(content)
        metrics = validation.OpaqueMetrics(
            mode,
            validation.OpaqueReason.COMPLETED,
            "success",
            hashlib.sha256(json.dumps(argv, separators=(",", ":")).encode()).hexdigest(),
            None,
            len(content),
            0,
            hashlib.sha256(content).hexdigest(),
            hashlib.sha256(b"").hexdigest(),
        )
        return validation.OpaqueRunResult(
            "success",
            0,
            "",
            "",
            0,
            Diagnostic(mode.stage, "completed", "held", "metrics", "review"),
            ("opaque-observation", mode.value, metrics.argv_sha256),
            metrics=metrics,
        )

    def test_private_inspect_drops_unknown_fields_and_rejects_untrusted_timestamp(self) -> None:
        with (
            patch.object(validation, "run", side_effect=self.inspect_run),
            patch.object(
                validation, "_redacted_argv", side_effect=AssertionError("legacy renderer")
            ),
            patch.object(
                validation, "failure_evidence", side_effect=AssertionError("legacy excerpts")
            ),
        ):
            dto = self.commands._live_inspect(
                self.held, validation.OpaqueObservation.INSPECT_BEFORE
            )
        self.assertEqual((dto.container_pid, dto.conmon_pid), (20, 30))
        self.assertNotIn("not-retained", repr(dto))
        for timestamp in (
            "unknown text",
            "2026-99-01T00:00:00Z",
            "0001-01-01T00:00:00Z",
            "2026-11-05 18:00:00Z",
        ):
            data = self.native()
            data[0]["State"]["StartedAt"] = timestamp
            with self.subTest(timestamp=timestamp), self.assertRaises(validation.OpaqueParseError):
                validation._parse_live_inspect(json.dumps(data).encode(), self.held)

    def test_known_two_pid_schema_rejects_bad_types_topology_uid_and_liveness(self) -> None:
        good = validation._parse_live_processes(
            json.dumps(self.observed()).encode(), self.request()
        )
        self.assertEqual(
            good.processes[1].mount_namespace, "mnt:[333]"
        )  # No invented mount equality.
        changes = (
            ("pid", True),
            ("nspid", [20]),
            ("uids", [0] * 4),
            ("pid_namespace", "pid:[100]"),
            ("pidfd_alive_after", False),
            ("start_ticks", "invalid"),
        )
        for key, value in changes:
            data = self.observed()
            data["processes"][0][key] = value
            with self.subTest(key=key), self.assertRaises(validation.OpaqueParseError):
                validation._parse_live_processes(json.dumps(data).encode(), self.request())
        data = self.observed()
        data["observer"]["uids"] = [False] * 4
        with self.assertRaises(validation.OpaqueParseError):
            validation._parse_live_processes(json.dumps(data).encode(), self.request())

    def test_only_closed_private_producer_opts_into_runner_opacity(self) -> None:
        import ast

        tree = ast.parse(Path(validation.__file__).read_text())
        owners: list[str] = []
        for owner in ast.walk(tree):
            if isinstance(owner, ast.FunctionDef):
                for node in ast.walk(owner):
                    if (
                        isinstance(node, ast.Call)
                        and isinstance(node.func, ast.Name)
                        and node.func.id == "run"
                        and any(key.arg == "opaque_observation" for key in node.keywords)
                    ):
                        owners.append(owner.name)
        self.assertEqual(owners, ["_invoke_live_observation"])
        self.assertIsNone(validation.LIVE_PID_SCHEMA)

    def test_unapproved_selector_never_invokes_root_observer(self) -> None:
        with (
            patch.object(validation, "LIVE_PID_SCHEMA", None),
            patch.object(validation, "run") as execute,
            patch.object(validation, "_rootful_bridge") as bridge,
            self.assertRaises(validation.OpaqueRunError),
        ):
            self.commands._live_processes(self.request())
        execute.assert_not_called()
        bridge.assert_not_called()

    def test_schema_capture_survives_normal_compact_handoff_and_outer_merge_without_observer(
        self,
    ) -> None:
        def case(compact: bool) -> None:
            automation = self.root / str(compact)
            selected = automation / "automation/src/boxferry_ci/devcontainer_lifetime.py"
            selected.parent.mkdir(parents=True)
            selected.write_bytes(Path(lifetime.__file__).read_bytes())
            captures: list[dict[str, object]] = []
            inner_causes: list[str] = []
            runtime_facts = {
                "engine_runtime": "runc",
                "engine_runtime_name_representation": "selected-path",
                "engine_runtime_path_representation": "resolved-path",
            }

            def invoke(argv: tuple[str, ...], **options: object) -> RunResult:
                phase, plan = argv[-2], json.loads(argv[-1])
                scope, control = Path(plan["scope"]), Path(plan["preparation"])
                if phase == "qualify":
                    self.addCleanup(shutil.rmtree, scope, ignore_errors=True)
                    self.addCleanup(shutil.rmtree, control, ignore_errors=True)
                    commands = validation.Commands(scope)
                    commands.budget = Mock(deadline=time.monotonic() + 300, check=Mock())
                    commands.lifetime_report = {
                        "schema": 1,
                        "status": "failure",
                        "cleanup_cause": "separate",
                        **runtime_facts,
                    }
                    if compact:
                        commands.lifetime_report["large"] = "x" * validation.MAX_REPORT
                    commands.lifetime_context = {
                        **(self.commands.lifetime_context or {}),
                        "private": str(scope),
                        "result": plan["result"],
                        "result_identity": plan["result_identity"],
                    }
                    live = validation.LiveProcessObservation(commands, self.held)
                    with (
                        patch.object(validation, "run", side_effect=self.inspect_run),
                        patch.object(commands, "_live_processes") as observer,
                        self.assertRaises(validation.OpaqueRunError),
                    ):
                        live.tick()
                    observer.assert_not_called()
                    self.assertEqual(os.read(self.reader, 1), b"A")
                    inner = json.loads(Path(plan["result"]).read_text())
                    capture = inner["live_process_schema_capture"]
                    self.assertEqual(capture["state_pid_candidate"], 20)
                    self.assertEqual(capture["candidate_state_conmon_pid_candidate"], 30)
                    self.assertIs(capture["selector_approved"], False)
                    self.assertIs(capture["observer_executed"], False)
                    self.assertEqual(inner["cleanup_cause"], "separate")
                    captures.append(capture)
                    inner_causes.append(inner["cause"])
                    for key, expected in runtime_facts.items():
                        self.assertEqual(inner[key], expected)
                    hook = options["on_abort"]
                    assert callable(hook)
                    hook(
                        validation.AbortContext(
                            "local-rootful-lifetime", "failed", "command exited125"
                        )
                    )
                    return RunResult(
                        "failed",
                        125,
                        "",
                        "",
                        0,
                        Diagnostic("outer", "command exited125", "", "", ""),
                        argv,
                    )
                for directory in (scope, control):
                    for path in directory.iterdir():
                        if path.is_dir():
                            shutil.rmtree(path)
                        else:
                            path.unlink()
                return RunResult(
                    "success", 0, "", "", 0, Diagnostic("finalize", "done", "", "", ""), argv
                )

            with (
                patch.object(validation, "LIVE_PID_SCHEMA", None),
                patch.object(lifetime, "run", side_effect=invoke),
                patch.object(
                    lifetime,
                    "_witness",
                    return_value=[
                        {"event": "closed", "namespace_closed": True, "supervisor_closed": True}
                    ],
                ),
            ):
                report = lifetime.execute(
                    {"work": self.root, "automation": automation, "local_sources": None}
                )
            self.assertEqual(report["live_process_schema_capture"], captures[0])
            self.assertEqual(report["status"], "failure")
            self.assertTrue(report["namespace_closed"])
            self.assertEqual(report["cause"], inner_causes[0])
            self.assertEqual(report["cleanup_cause"], "separate")
            for key, expected in runtime_facts.items():
                self.assertEqual(report[key], expected)

        for compact in (False, True):
            with self.subTest(compact=compact):
                case(compact)

    def test_live_sandwich_is_once_and_requires_exact_before_after_identity(self) -> None:
        with patch.object(
            validation, "LIVE_PID_SCHEMA", validation.LivePidSchema.PODMAN_STATE_FIELDS
        ):
            before = validation._parse_live_inspect(json.dumps(self.native()).encode(), self.held)
            sample = validation._parse_live_processes(
                json.dumps(self.observed()).encode(), self.request()
            )
            live = validation.LiveProcessObservation(self.commands, self.held)
            with (
                patch.object(
                    self.commands, "_live_inspect", side_effect=[before, before]
                ) as inspected,
                patch.object(self.commands, "_live_processes", return_value=sample) as observed,
            ):
                live.tick()
                live.tick()
                live.require_complete()
            self.assertEqual(inspected.call_count, 2)
            observed.assert_called_once()
            observed_report = self.report["live_process_observation"]
            assert isinstance(observed_report, dict)
            self.assertEqual(observed_report["scope"], "known-two-pid-snapshot")
            changed = self.native()
            changed[0]["State"]["StartedAt"] = "2026-11-05T18:00:01Z"
            after = validation._parse_live_inspect(json.dumps(changed).encode(), self.held)
            live = validation.LiveProcessObservation(self.commands, self.held)
            with (
                patch.object(self.commands, "_live_inspect", side_effect=[before, after]),
                patch.object(self.commands, "_live_processes", return_value=sample),
                self.assertRaises(validation.OpaqueRunError),
            ):
                live.tick()
            self.assertIsNone(live.completed)

    def test_readiness_is_finite_scheduled_without_sleep_or_new_stage_budget(self) -> None:
        created = validation._parse_live_inspect(
            json.dumps(self.native(running=False)).encode(), self.held
        )
        live = validation.LiveProcessObservation(self.commands, self.held)
        assert self.commands.budget is not None
        deadline = self.commands.budget.deadline
        with (
            patch.object(validation.time, "monotonic", return_value=100.0),
            patch.object(self.commands, "_live_inspect", return_value=created) as inspect,
            patch.object(validation.time, "sleep", side_effect=AssertionError("sleep forbidden")),
        ):
            live.tick()
            live.tick()
        inspect.assert_called_once()
        for when in (100.11, 100.22, 100.33):
            with (
                patch.object(validation.time, "monotonic", return_value=when),
                patch.object(self.commands, "_live_inspect", return_value=created),
            ):
                live.tick()
        with (
            patch.object(validation.time, "monotonic", return_value=100.44),
            self.assertRaises(validation.OpaqueRunError),
        ):
            live.tick()
        self.assertEqual(live.attempts, 4)
        self.assertEqual(self.commands.budget.deadline, deadline)

    def test_root_observer_literal_uses_only_bounded_known_proc_reads_and_closes_local_fds(
        self,
    ) -> None:
        import ast

        tree = ast.parse(validation.LIVE_PROCESS_OBSERVER_PROGRAM)
        namespace: dict[str, Any] = {}
        definitions = ast.Module(
            body=[node for node in tree.body if not isinstance(node, ast.If)], type_ignores=[]
        )
        exec(compile(definitions, "fixed-root-observer-unit-only", "exec"), namespace)
        opened: dict[int, tuple[int, str]] = {}
        closed: list[int] = []
        pidfds: list[int] = []
        count = [500]

        def open_fd(name: str, _flags: int, *, dir_fd: int | None = None) -> int:
            if name == "/proc":
                return 10
            if dir_fd == 10:
                return int(name) + 1000
            assert dir_fd is not None
            count[0] += 1
            opened[count[0]] = (dir_fd - 1000, name)
            return count[0]

        def read(fd: int, _bound: int) -> bytes:
            pid, name = opened[fd]
            if name == "stat":
                return (
                    str(pid) + " (name with ) and spaces) R " + " ".join(["0"] * 18 + ["77"])
                ).encode()
            uid = 1000 if pid == 20 else 0
            nspid = [20, 1] if pid == 20 else [pid]
            return (
                "Uid: " + " ".join([str(uid)] * 4) + "\nNSpid: " + " ".join(map(str, nspid)) + "\n"
            ).encode()

        def link(name: str, *, dir_fd: int) -> str:
            pid = dir_fd - 1000
            return (
                ("pid:[101]" if pid == 20 else "pid:[100]")
                if name == "ns/pid"
                else ("mnt:[200]" if pid == 40 else "mnt:[333]")
            )

        def pidfd(pid: int, _flags: int) -> int:
            pidfds.append(2000 + pid)
            return 2000 + pid

        with (
            patch.object(os, "getpid", return_value=40),
            patch.object(os, "getuid", return_value=0),
            patch.object(os, "geteuid", return_value=0),
            patch.object(os, "getresuid", return_value=(0, 0, 0)),
            patch.object(os, "open", side_effect=open_fd),
            patch.object(os, "read", side_effect=read),
            patch.object(os, "readlink", side_effect=link),
            patch.object(os, "pidfd_open", side_effect=pidfd),
            patch.object(os, "set_inheritable") as inheritable,
            patch.object(os, "close", side_effect=closed.append),
            patch.object(namespace["select"], "select", return_value=([], [], [])),
        ):
            result = namespace["observe"](validation.asdict(self.request()))
        self.assertEqual(result["status"], "observed")
        self.assertEqual(pidfds, [2020, 2030])
        self.assertTrue(set(pidfds).issubset(closed))
        self.assertEqual(
            inheritable.call_args_list,
            [call(2020, False), call(2030, False)],
        )
        self.assertTrue(set(opened).issubset(closed))
        self.assertIn(10, closed)
        self.assertNotIn("listdir", validation.LIVE_PROCESS_OBSERVER_PROGRAM)
        self.assertNotIn("setns", validation.LIVE_PROCESS_OBSERVER_PROGRAM)


if __name__ == "__main__":
    unittest.main()
