"""Standalone immutable bootstrap rejects mutable configuration and unsafe archives."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from collections.abc import Callable
from http.client import HTTPMessage
from pathlib import Path
from types import ModuleType
from unittest.mock import patch

from boxferry_ci import bootstrap


class BootstrapTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / ".github").mkdir()
        self.config = {
            "schema": 1,
            "repository": bootstrap.REPOSITORY,
            "revision": "a" * 40,
            "policy": "scripts/validation-policy.json",
            "interface": 1,
        }
        self.manifest()

    def manifest(self) -> None:
        (self.root / ".github/automation.json").write_text(json.dumps(self.config))

    def git(self, root: Path, *arguments: str) -> str:
        return subprocess.run(
            ["git", "-C", str(root), *arguments],
            check=True,
            capture_output=True,
            text=True,
            env=bootstrap.git_environment(),
        ).stdout.strip()

    def fixture_cache(self) -> tuple[str, Path]:
        fixture = self.root / "fixture"
        fixture.mkdir()
        self.git(fixture, "init", "--initial-branch=main", "--template=")
        self.git(fixture, "config", "user.email", "fixture@example.invalid")
        self.git(fixture, "config", "user.name", "Fixture")
        (fixture / "automation").mkdir()
        (fixture / "automation/pyproject.toml").write_text("[project]\nname='fixture'\n")
        (fixture / "automation/uv.lock").write_text("version = 1\n")
        (fixture / "renovate-presets").mkdir()
        (fixture / "renovate-presets/common.json").write_text("{}\n")
        (fixture / ".devcontainer").mkdir()
        source_root = Path(__file__).resolve().parents[2]
        (fixture / ".devcontainer/Dockerfile").write_bytes(
            (source_root / ".devcontainer/Dockerfile").read_bytes()
        )
        (fixture / ".devcontainer/not-exported.json").write_text("not part of interface 1")
        (fixture / "templates").mkdir()
        (fixture / "templates/not-exported.py").write_text("not part of interface 1")
        self.git(fixture, "add", ".")
        self.git(fixture, "commit", "-m", "fixture")
        head = self.git(fixture, "rev-parse", "HEAD")
        objects = self.root / f"target/boxferry-automation/{head}.git"
        objects.parent.mkdir(parents=True)
        self.git(self.root, "clone", "--bare", str(fixture), str(objects))
        (objects / "config").write_text(bootstrap.BARE_CONFIG)
        return head, objects

    def identity_archive(self) -> tuple[str, Path]:
        self.fixture_cache()
        fixture = self.root / "fixture"
        (fixture / "automation/src/boxferry_ci").mkdir(parents=True)
        (fixture / "automation/src/boxferry_ci/bootstrap.py").write_text("trusted = True\n")
        executable = fixture / "automation/executable.py"
        executable.write_text("print('tracked executable fixture')\n")
        executable.chmod(0o755)
        self.git(fixture, "add", ".")
        self.git(fixture, "commit", "-m", "identity fixture source")
        head = self.git(fixture, "rev-parse", "HEAD")
        objects = self.root / f"target/boxferry-automation/{head}.git"
        self.git(self.root, "clone", "--bare", str(fixture), str(objects))
        (objects / "config").write_text(bootstrap.BARE_CONFIG)
        self.config["revision"] = head
        self.manifest()
        return head, bootstrap.prepare_implementation(self.root, head)

    def test_archive_identity_is_read_only_and_checks_actual_bytes_and_locks(self) -> None:
        head, project = self.identity_archive()
        with patch.object(bootstrap, "__file__", str(project / "src/boxferry_ci/bootstrap.py")):
            self.assertEqual(
                bootstrap.automation_identity(self.root),
                {
                    "revision": head,
                    "kind": "interface-1-archive",
                },
            )
            for relative in ("src/boxferry_ci/bootstrap.py", "uv.lock", "pyproject.toml"):
                path = project / relative
                original = path.read_bytes()
                path.write_bytes(original + b"tampered\n")
                with self.subTest(relative=relative), self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.automation_identity(self.root, head)
                self.assertTrue(
                    path.read_bytes().endswith(b"tampered\n"), "verification must not repair"
                )
                path.write_bytes(original)

    def test_archive_never_shortcuts_to_matching_consumer_git_even_when_A_equals_C(self) -> None:
        head, project = self.identity_archive()
        self.git(self.root, "init", "--template=")
        self.git(self.root, "fetch", str(self.root / "fixture"), head)
        self.git(self.root, "reset", "--mixed", head)
        self.assertEqual(self.git(self.root, "rev-parse", "HEAD"), head)
        with patch.object(bootstrap, "__file__", str(project / "src/boxferry_ci/bootstrap.py")):
            self.assertEqual(bootstrap.automation_identity(self.root, head)["revision"], head)
            (project / "uv.lock").write_text("same HEAD cannot prove these bytes\n")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, head)

    def test_archive_identity_rejects_wrong_manifest_missing_cache_and_extra_source(self) -> None:
        head, project = self.identity_archive()
        with patch.object(bootstrap, "__file__", str(project / "src/boxferry_ci/bootstrap.py")):
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, "f" * 40)
            self.config["revision"] = "e" * 40
            self.manifest()
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, head)
            self.config["revision"] = head
            self.manifest()
            extra = project / "src/boxferry_ci/unexpected.py"
            extra.write_text("unexpected importable source\n")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, head)
            extra.unlink()
            (project / "uv.lock").unlink()
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, head)
            shutil.rmtree(project.parent.parent / f"{head}.git")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, head)

    def test_archive_runtime_exclusions_do_not_accept_importable_or_linked_source(self) -> None:
        head, project = self.identity_archive()
        bytecode = project / "src/boxferry_ci/__pycache__"
        bytecode.mkdir()
        (bytecode / "bootstrap.cpython-313.pyc").write_bytes(b"fixture runtime cache")
        (project / ".venv").mkdir()
        (project / ".venv/python").symlink_to(sys.executable)
        with patch.object(bootstrap, "__file__", str(project / "src/boxferry_ci/bootstrap.py")):
            self.assertEqual(bootstrap.automation_identity(self.root)["revision"], head)
            extra = bytecode / "unexpected.py"
            extra.write_text("not a bytecode cache\n")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root)

            extra.unlink()
            source = project / "src/boxferry_ci/bootstrap.py"
            source.unlink()
            source.symlink_to(self.root / "fixture/automation/src/boxferry_ci/bootstrap.py")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root)

    def test_archive_identity_uses_real_extraction_modes_and_rejects_mode_changes(self) -> None:
        head, project = self.identity_archive()
        raw = bootstrap.run_git(
            self.root / f"target/boxferry-automation/{head}.git",
            "archive",
            "--format=tar",
            head,
            "automation",
            binary=True,
        )
        assert isinstance(raw, bytes)
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            self.assertFalse(archive.getmember("automation/uv.lock").mode & 0o111)
            self.assertTrue(archive.getmember("automation/executable.py").mode & 0o111)
        self.assertEqual((project / "uv.lock").stat().st_mode & 0o777, 0o644)
        self.assertEqual((project / "executable.py").stat().st_mode & 0o777, 0o755)
        with patch.object(bootstrap, "__file__", str(project / "src/boxferry_ci/bootstrap.py")):
            self.assertEqual(bootstrap.automation_identity(self.root)["revision"], head)
            for path, invalid in (
                (project / "uv.lock", 0o664),
                (project / "executable.py", 0o644),
                (project.parent / bootstrap.CARGO_METADATA_PATH, 0o755),
            ):
                previous = path.stat().st_mode & 0o777
                path.chmod(invalid)
                with self.subTest(path=path), self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.automation_identity(self.root)
                path.chmod(previous)

    def test_identity_rejects_object_archive_and_configuration_overrides(self) -> None:
        head, project = self.identity_archive()
        cache = project.parent.parent / f"{head}.git"
        with patch.object(bootstrap, "__file__", str(project / "src/boxferry_ci/bootstrap.py")):
            for relative in ("objects/info/alternates", "info/grafts", "info/attributes"):
                override = cache / relative
                override.parent.mkdir(parents=True, exist_ok=True)
                override.write_text("automation/** export-ignore\n")
                with self.subTest(relative=relative), self.assertRaises(bootstrap.BootstrapError):
                    bootstrap.automation_identity(self.root)
                override.unlink()
            config = cache / "config"
            config.write_text(bootstrap.BARE_CONFIG + "\n[alias]\nunsafe = !false\n")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root)
            config.write_text(bootstrap.BARE_CONFIG)
            with patch.dict(os.environ, {"GIT_DIR": "/untrusted", "GIT_OBJECT_DIRECTORY": "/bad"}):
                self.assertEqual(bootstrap.automation_identity(self.root)["revision"], head)

    def test_direct_checkout_requires_own_git_root_and_dirty_self_is_development_only(self) -> None:
        head, _project = self.identity_archive()
        source = self.root / "fixture"
        module = source / "automation/src/boxferry_ci/bootstrap.py"
        with patch.object(bootstrap, "__file__", str(module)):
            self.assertEqual(bootstrap.automation_identity(self.root, head)["kind"], "git-checkout")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, "f" * 40)
            module.write_text("dirty producer work\n")
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(source, head)
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.automation_identity(self.root, head, allow_dirty_self=True)
            self.assertEqual(
                bootstrap.automation_identity(source, head, allow_dirty_self=True)["kind"],
                "producer-working-tree",
            )
        nested = source / "nested/automation/src/boxferry_ci/bootstrap.py"
        nested.parent.mkdir(parents=True)
        nested.write_text("ancestor Git is not implementation identity\n")
        with (
            patch.object(bootstrap, "__file__", str(nested)),
            self.assertRaisesRegex(bootstrap.BootstrapError, "ancestor Git"),
        ):
            bootstrap.automation_identity(source, head)

    def test_foreign_direct_checkout_default_binds_manifest_but_explicit_sha_is_trusted(
        self,
    ) -> None:
        head, _project = self.identity_archive()
        module = self.root / "fixture/automation/src/boxferry_ci/bootstrap.py"
        manifest = self.root / ".github/automation.json"
        with patch.object(bootstrap, "__file__", str(module)):
            self.assertEqual(bootstrap.automation_identity(self.root)["revision"], head)
            self.config["revision"] = "f" * 40
            self.manifest()
            with self.assertRaisesRegex(bootstrap.BootstrapError, "requested immutable SHA"):
                bootstrap.automation_identity(self.root)
            self.assertEqual(bootstrap.automation_identity(self.root, head)["revision"], head)
            manifest.unlink()
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "cannot read automation manifest"
            ):
                bootstrap.automation_identity(self.root)
            self.assertEqual(bootstrap.automation_identity(self.root, head)["revision"], head)
            manifest.write_text("not JSON")
            with self.assertRaisesRegex(
                bootstrap.BootstrapError, "cannot read automation manifest"
            ):
                bootstrap.automation_identity(self.root)

    def test_same_root_default_uses_own_head_and_dirty_development_identity(self) -> None:
        self.identity_archive()
        source = self.root / "fixture"
        module = source / "automation/src/boxferry_ci/bootstrap.py"
        (source / ".github").mkdir()
        (source / ".github/automation.json").write_text(
            json.dumps(self.config | {"revision": "f" * 40})
        )
        self.git(source, "add", ".")
        self.git(source, "commit", "-m", "stale self consumer pin")
        head = self.git(source, "rev-parse", "HEAD")
        with patch.object(bootstrap, "__file__", str(module)):
            self.assertEqual(
                bootstrap.automation_identity(source), {"revision": head, "kind": "git-checkout"}
            )
            module.write_text("dirty producer development\n")
            with self.assertRaisesRegex(bootstrap.BootstrapError, "mutable automation"):
                bootstrap.automation_identity(source)
            self.assertEqual(
                bootstrap.automation_identity(source, allow_dirty_self=True),
                {"revision": head, "kind": "producer-working-tree"},
            )

    def test_manifest_exact_interface_and_fixed_remote(self) -> None:
        self.assertEqual(bootstrap.load_config(self.root), self.config)
        for key, value in (
            ("repository", "attacker/arbitrary"),
            ("revision", "main"),
            ("interface", True),
            ("schema", 2),
            ("policy", "../outside.json"),
            ("policy", "/etc/config.json"),
            ("policy", "scripts\\policy.json"),
        ):
            original = self.config[key]
            self.config[key] = value
            self.manifest()
            with self.subTest(key=key, value=value), self.assertRaises(bootstrap.BootstrapError):
                bootstrap.load_config(self.root)
            self.config[key] = original
        self.config["remote"] = "https://attacker.invalid/repo"
        self.manifest()
        with self.assertRaises(bootstrap.BootstrapError):
            bootstrap.load_config(self.root)

    def test_policy_symlink_cannot_escape_consumer(self) -> None:
        outside = self.root / "outside"
        outside.mkdir()
        (self.root / "scripts").symlink_to(Path("/tmp"), target_is_directory=True)
        with self.assertRaises(bootstrap.BootstrapError):
            bootstrap.load_config(self.root)

    def test_verified_cache_reconstructs_tampered_checkout_and_keeps_presets(self) -> None:
        head, _ = self.fixture_cache()
        project = bootstrap.prepare_implementation(self.root, head)
        self.assertEqual((project.parent / "renovate-presets/common.json").read_text(), "{}\n")
        (project / "pyproject.toml").write_text("candidate tampering")
        (project / "untracked.py").write_text("not trusted")
        project = bootstrap.prepare_implementation(self.root, head)
        self.assertIn("name='fixture'", (project / "pyproject.toml").read_text())
        self.assertFalse((project / "untracked.py").exists())

    def archive_implementations(self) -> tuple[ModuleType, ModuleType]:
        root = Path(__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "archive_closure_loader", root / "templates/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        return bootstrap, loader

    def test_prepared_cargo_arg_data_is_exact_and_reconstructed_without_extra_roots(self) -> None:
        head, _ = self.fixture_cache()
        expected = (self.root / "fixture/.devcontainer/Dockerfile").read_bytes()
        self.assertRegex(expected, rb"(?m)^ARG CARGO_DENY_VERSION=[0-9]+\.[0-9]+\.[0-9]+$")
        for implementation in self.archive_implementations():
            with self.subTest(implementation=implementation.__name__):
                project = implementation.prepare_implementation(self.root, head)
                metadata = project.parent / ".devcontainer/Dockerfile"
                self.assertEqual(metadata.read_bytes(), expected)
                self.assertEqual({path.name for path in metadata.parent.iterdir()}, {"Dockerfile"})
                self.assertFalse((project.parent / "templates").exists())
                metadata.write_text("ARG CARGO_DENY_VERSION=0.0.0\n")
                (metadata.parent / "not-exported.json").write_text("candidate tampering")
                implementation.prepare_implementation(self.root, head)
                self.assertEqual(metadata.read_bytes(), expected)
                self.assertFalse((metadata.parent / "not-exported.json").exists())
                self.assertEqual(implementation.MAX_ARCHIVE_BYTES, 32 * 1024 * 1024)
                self.assertEqual(implementation.MAX_CACHE_BYTES, 128 * 1024 * 1024)
                self.assertEqual(
                    implementation.REMOTE, "https://github.com/Strukturpiloten/boxferry.git"
                )

    def test_cargo_data_archive_rejects_links_ancillary_files_and_path_aliases(self) -> None:
        cases = [
            (".devcontainer/not-exported.json", tarfile.REGTYPE),
            (".devcontainer/nested/Dockerfile", tarfile.REGTYPE),
            (".devcontainer/Dockerfile/child", tarfile.REGTYPE),
            (".devcontainer", tarfile.REGTYPE),
            (".devcontainer/Dockerfile", tarfile.DIRTYPE),
            (".devcontainer/Dockerfile", tarfile.SYMTYPE),
            (".devcontainer/Dockerfile", tarfile.LNKTYPE),
            (".devcontainer/../outside", tarfile.REGTYPE),
            ("/.devcontainer/Dockerfile", tarfile.REGTYPE),
            (".devcontainer//Dockerfile", tarfile.REGTYPE),
            (".devcontainer/./Dockerfile", tarfile.REGTYPE),
            ("./.devcontainer/Dockerfile", tarfile.REGTYPE),
        ]
        for index, (name, kind) in enumerate(cases):
            archive = self.root / f"unsafe-{index}.tar"
            with tarfile.open(archive, "w") as output:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "/etc/passwd"
                output.addfile(member, io.BytesIO())
            for implementation in self.archive_implementations():
                with (
                    self.subTest(implementation=implementation.__name__, name=name, kind=kind),
                    self.assertRaises(implementation.BootstrapError),
                ):
                    implementation.extract_archive(archive, self.root / f"out-{index}")

    def test_cargo_data_extraction_is_nonexecutable_and_keeps_archive_budget(self) -> None:
        archive = self.root / "data.tar"
        with tarfile.open(archive, "w") as output:
            member = tarfile.TarInfo(".devcontainer/Dockerfile")
            member.mode = 0o755
            member.size = 3
            output.addfile(member, io.BytesIO(b"ARG"))
        for index, implementation in enumerate(self.archive_implementations()):
            destination = self.root / f"data-{index}"
            implementation.extract_archive(archive, destination)
            data = destination / ".devcontainer/Dockerfile"
            self.assertEqual(data.read_bytes(), b"ARG")
            self.assertEqual(data.stat().st_mode & 0o777, 0o644)
            with (
                patch.object(implementation, "MAX_ARCHIVE_BYTES", 2),
                self.assertRaisesRegex(implementation.BootstrapError, "budget"),
            ):
                implementation.extract_archive(archive, self.root / f"over-{index}")

    def test_prepare_rejects_missing_cargo_data_even_after_successful_archive_call(self) -> None:
        head, _ = self.fixture_cache()
        archive = self.root / "incomplete.tar"
        with tarfile.open(archive, "w") as output:
            for name in (
                "automation/pyproject.toml",
                "automation/uv.lock",
                "renovate-presets/common.json",
            ):
                member = tarfile.TarInfo(name)
                member.size = 2
                output.addfile(member, io.BytesIO(b"{}"))
        for implementation in self.archive_implementations():

            def truncated_archive(
                directory: Path,
                *arguments: str,
                binary: bool = False,
                held_git: Callable[..., bytes | str] = implementation.run_git,
            ) -> bytes | str:
                if arguments[0] == "archive":
                    return archive.read_bytes()
                return held_git(directory, *arguments, binary=binary)

            with (
                patch.object(implementation, "run_git", side_effect=truncated_archive),
                self.assertRaisesRegex(implementation.BootstrapError, "Cargo tool metadata"),
            ):
                implementation.prepare_implementation(self.root, head)

    def test_git_archive_rejects_revision_missing_the_required_data_path(self) -> None:
        self.fixture_cache()
        fixture = self.root / "fixture"
        self.git(fixture, "rm", ".devcontainer/Dockerfile")
        self.git(fixture, "commit", "-m", "missing Cargo metadata")
        head = self.git(fixture, "rev-parse", "HEAD")
        objects = self.root / f"target/boxferry-automation/{head}.git"
        self.git(self.root, "clone", "--bare", str(fixture), str(objects))
        (objects / "config").write_text(bootstrap.BARE_CONFIG)
        for implementation in self.archive_implementations():
            with (
                self.subTest(implementation=implementation.__name__),
                self.assertRaisesRegex(implementation.BootstrapError, r"\.devcontainer/Dockerfile"),
            ):
                implementation.prepare_implementation(self.root, head)
            self.assertFalse((objects.parent / head).exists())

    def test_a_loader_passes_held_cargo_metadata_to_b_install_interface(self) -> None:
        _, _ = self.fixture_cache()
        fixture = self.root / "fixture"
        core = fixture / "automation/src/boxferry_ci/bootstrap.py"
        core.parent.mkdir(parents=True)
        core.write_text(
            "import json\n"
            "def run_project(root, project, arguments):\n"
            "    text = (project.parent / '.devcontainer/Dockerfile').read_text()\n"
            "    version = next(row.split('=', 1)[1] for row in text.splitlines()\n"
            "                   if row.startswith('ARG CARGO_DENY_VERSION='))\n"
            "    command = ['cargo', 'install', '--locked', '--version', version, 'cargo-deny']\n"
            "    (root / 'install-plan.json').write_text(json.dumps([arguments, command]))\n"
            "    return 23\n"
        )
        self.git(fixture, "add", ".")
        self.git(fixture, "commit", "-m", "selected B installer")
        head = self.git(fixture, "rev-parse", "HEAD")
        objects = self.root / f"target/boxferry-automation/{head}.git"
        self.git(self.root, "clone", "--bare", str(fixture), str(objects))
        (objects / "config").write_text(bootstrap.BARE_CONFIG)
        (self.root / ".devcontainer").mkdir()
        (self.root / ".devcontainer/Dockerfile").write_text("ARG CARGO_DENY_VERSION=0.0.0\n")
        self.config["revision"] = head
        self.manifest()
        _, loader = self.archive_implementations()
        arguments = ["tools", "--cargo-tools", "cargo-deny"]
        with patch.object(loader, "__file__", str(self.root / "scripts/shared-ci.py")):
            self.assertEqual(loader.main(arguments), 23)
        observed = json.loads((self.root / "install-plan.json").read_text())
        expected = re.search(
            r"(?m)^ARG CARGO_DENY_VERSION=(.+)$", (fixture / ".devcontainer/Dockerfile").read_text()
        )
        assert expected is not None
        self.assertEqual(
            observed,
            [arguments, ["cargo", "install", "--locked", "--version", expected[1], "cargo-deny"]],
        )

    def test_git_configuration_is_rejected_before_loading_hooks_or_filters(self) -> None:
        head, objects = self.fixture_cache()
        (objects / "config").write_text(bootstrap.BARE_CONFIG + "[include]\npath=/etc/gitconfig\n")
        with self.assertRaisesRegex(bootstrap.BootstrapError, "untrusted Git configuration"):
            bootstrap.prepare_implementation(self.root, head)

    def test_object_override_and_symlink_rejected(self) -> None:
        _, objects = self.fixture_cache()
        alternates = objects / "objects/info/alternates"
        alternates.parent.mkdir(exist_ok=True)
        alternates.write_text("/tmp/another-object-store\n")
        with self.assertRaisesRegex(bootstrap.BootstrapError, "object overrides"):
            bootstrap.validate_cache(objects)
        alternates.unlink()
        (objects / "objects/link").symlink_to("/tmp")
        with self.assertRaisesRegex(bootstrap.BootstrapError, "symlink"):
            bootstrap.validate_cache(objects)

    def test_inherited_git_credentials_and_config_are_not_used(self) -> None:
        with patch.dict(
            os.environ,
            {
                "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "alias.fsck",
                "GIT_CONFIG_VALUE_0": "!bad",
                "GIT_ASKPASS": "secret-reader",
            },
        ):
            environment = bootstrap.git_environment()
        self.assertNotIn("GIT_CONFIG_VALUE_0", environment)
        self.assertNotIn("GIT_ASKPASS", environment)
        self.assertEqual(environment["GIT_CONFIG_GLOBAL"], os.devnull)
        self.assertEqual(environment["GIT_TERMINAL_PROMPT"], "0")

    def test_archive_links_traversal_and_extra_roots_are_rejected(self) -> None:
        for name, kind in (
            ("automation/../../outside", tarfile.REGTYPE),
            ("automation/link", tarfile.SYMTYPE),
            ("other/file", tarfile.REGTYPE),
        ):
            archive = self.root / "fixture.tar"
            with tarfile.open(archive, "w") as output:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.size = 0
                member.linkname = "/etc/passwd"
                output.addfile(member, io.BytesIO())
            with self.subTest(name=name), self.assertRaises(bootstrap.BootstrapError):
                bootstrap.extract_archive(archive, self.root / "output")

    def test_explicit_trusted_policy_arguments_survive_defaults(self) -> None:
        explicit = ["plan", "--repository-root=/trusted/consumer", "--config", "/trusted/policy"]
        self.assertEqual(bootstrap.forwarded_arguments(explicit, self.root, "other"), explicit)
        defaulted = bootstrap.forwarded_arguments(
            ["run-local"], self.root, str(self.config["policy"])
        )
        self.assertEqual(defaulted[-2:], ["--config", str(self.root / str(self.config["policy"]))])

    def test_development_override_is_honest_and_uses_locked_project(self) -> None:
        development = self.root / "development"
        (development / "automation").mkdir(parents=True)
        for name in ("pyproject.toml", "uv.lock"):
            (development / "automation" / name).write_text("fixture")
        with (
            patch.object(bootstrap, "__file__", str(self.root / "scripts/shared-ci.py")),
            patch.object(bootstrap.subprocess, "call", return_value=0) as call,
            patch.object(bootstrap, "ensure_uv", return_value=Path("/checked/uv")),
            patch("sys.stderr", new_callable=io.StringIO) as diagnostic,
        ):
            code = bootstrap.main(["--implementation-checkout", str(development), "plan"])
        self.assertEqual(code, 0)
        self.assertIn("mutable development implementation", diagnostic.getvalue())
        command = call.call_args.args[0]
        self.assertEqual(command[:3], ["/checked/uv", "run", "--locked"])
        self.assertIn(str(development / "automation"), command)
        self.assertIn("boxferry_ci", command)

    def test_stable_loader_executes_selected_implementation_after_verification(self) -> None:
        root = Path(__file__).resolve().parents[2]
        spec = importlib.util.spec_from_file_location(
            "consumer_loader", root / "templates/shared-ci.py"
        )
        assert spec is not None and spec.loader is not None
        loader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(loader)
        marker = self.root / "selected"
        head, objects = self.fixture_cache()
        # Reconstruct a fresh pinned fixture whose B bootstrap has observable behavior.
        fixture = self.root / "fixture"
        implementation = fixture / "automation/src/boxferry_ci/bootstrap.py"
        implementation.parent.mkdir(parents=True)
        implementation.write_text(
            "from __future__ import annotations\n"
            "from dataclasses import dataclass\n"
            "from pathlib import Path\n"
            "@dataclass\n"
            "class Selected:\n"
            "    revision: str\n"
            "def run_project(repository_root, project, arguments):\n"
            "    text = Selected(str(project)).revision + '\\n' + repr(arguments)\n"
            "    (repository_root / 'selected').write_text(text)\n"
            "    return 23\n"
        )
        self.git(fixture, "add", ".")
        self.git(fixture, "commit", "-m", "implementation B")
        head = self.git(fixture, "rev-parse", "HEAD")
        objects = self.root / f"target/boxferry-automation/{head}.git"
        self.git(self.root, "clone", "--bare", str(fixture), str(objects))
        (objects / "config").write_text(loader.BARE_CONFIG)
        self.config["revision"] = head
        self.manifest()
        with patch.object(loader, "__file__", str(self.root / "scripts/shared-ci.py")):
            self.assertEqual(loader.main(["plan"]), 23)
        self.assertIn(
            str(self.root / f"target/boxferry-automation/{head}/automation"), marker.read_text()
        )
        self.assertTrue(marker.read_text().endswith(repr(["plan"])))
        with patch.object(loader, "__file__", str(self.root / "scripts/shared-ci.py")):
            self.assertEqual(loader.main(["future-command", "--option", "value"]), 23)
        self.assertTrue(marker.read_text().endswith(repr(["future-command", "--option", "value"])))
        marker.unlink()
        (objects / "config").write_text(loader.BARE_CONFIG + "[include]\npath=/etc/gitconfig\n")
        with patch.object(loader, "__file__", str(self.root / "scripts/shared-ci.py")):
            self.assertEqual(loader.main(["plan"]), 2)
        self.assertFalse(marker.exists())
        self.config["interface"] = 2
        self.manifest()
        with patch.object(loader, "__file__", str(self.root / "scripts/shared-ci.py")):
            self.assertEqual(loader.main(["plan"]), 2)
        self.assertFalse(marker.exists())

    def test_run_project_uses_selected_project_uv_and_locked_cli(self) -> None:
        project = self.root / "implementation-b/automation"
        with (
            patch.dict(os.environ, {"PYTHONPATH": str(self.root / "foreign/automation/src")}),
            patch.object(bootstrap, "ensure_uv", return_value=Path("/selected/uv")) as ensure,
            patch.object(bootstrap.subprocess, "call", return_value=19) as call,
        ):
            self.assertEqual(bootstrap.run_project(self.root, project, ["plan"]), 19)
        ensure.assert_called_once_with(project, self.root / "target/boxferry-automation/tools")
        self.assertNotIn("PYTHONPATH", call.call_args.kwargs["env"])
        self.assertEqual(call.call_args.kwargs["env"]["PATH"].split(os.pathsep)[0], "/selected")
        self.assertEqual(
            call.call_args.args[0][:5],
            ["/selected/uv", "run", "--locked", "--project", str(project)],
        )
        self.assertEqual(
            call.call_args.args[0][-5:],
            [
                "plan",
                "--repository-root",
                str(self.root),
                "--config",
                str(self.root / "scripts/validation-policy.json"),
            ],
        )

    def test_non_policy_commands_preserve_explicit_arguments(self) -> None:
        for arguments in (
            [],
            ["dependencies", "--base-directory", "before", "--head-directory", "after"],
            ["renovate-runtime", "--engine", "podman"],
            ["future-command", "--option", "value"],
        ):
            with self.subTest(arguments=arguments):
                self.assertEqual(
                    bootstrap.forwarded_arguments(
                        arguments, self.root, "scripts/validation-policy.json"
                    ),
                    arguments,
                )

    def test_tool_command_does_not_receive_planner_arguments(self) -> None:
        arguments = ["tools", "--profile", "core", "--destination", "target/tools"]
        with (
            patch.object(bootstrap, "ensure_uv", return_value=Path("/selected/uv")),
            patch.object(bootstrap.subprocess, "call", return_value=0) as call,
        ):
            self.assertEqual(bootstrap.run_project(self.root, self.root / "project", arguments), 0)
        self.assertEqual(call.call_args.args[0][-len(arguments) :], arguments)
        self.assertNotIn("--config", call.call_args.args[0])

    def test_nested_tasks_resolve_selected_uv_before_wrong_ambient_binary(self) -> None:
        selected = self.root / "selected/uv"
        ambient = self.root / "ambient/uv"
        selected.parent.mkdir()
        ambient.parent.mkdir()
        selected.write_text(
            f"#!{sys.executable}\n"
            "import subprocess, sys\n"
            "if sys.argv[1:] == ['--version']:\n"
            "    print('selected uv')\n"
            "else:\n"
            "    result = subprocess.run(['uv', '--version'], capture_output=True, text=True)\n"
            "    sys.exit(0 if result.returncode == 0 "
            "and result.stdout == 'selected uv\\n' else 17)\n"
        )
        ambient.write_text(f"#!{sys.executable}\nprint('wrong ambient uv')\n")
        selected.chmod(0o755)
        ambient.chmod(0o755)
        with (
            patch.object(bootstrap, "ensure_uv", return_value=selected),
            patch.dict(os.environ, {"PATH": str(ambient.parent)}),
        ):
            self.assertEqual(bootstrap.run_project(self.root, self.root / "project", ["plan"]), 0)


class UvBootstrapTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        (self.project / "pyproject.toml").write_text('[tool.uv]\nrequired-version = "==0.13.6"\n')
        self.binary = b"authentic test uv bytes"
        self.archives: dict[str, Path] = {}
        self.assets: list[dict[str, str]] = []
        for filename in bootstrap.UV_ASSETS.values():
            archive = self.root / filename
            self.write_archive(archive, filename)
            self.archives[filename] = archive
            self.assets.append(
                {
                    "filename": filename,
                    "currentVersion": "0.13.6",
                    "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                }
            )
        self.metadata()
        self.cache = self.root / "cache"
        self.system = patch.object(bootstrap.platform, "system", return_value="Linux")
        self.machine = patch.object(bootstrap.platform, "machine", return_value="x86_64")
        self.system.start()
        self.machine.start()
        self.addCleanup(self.system.stop)
        self.addCleanup(self.machine.stop)

    def metadata(self) -> None:
        (self.project / "uv-artifacts.json").write_text(
            json.dumps({"schema": 1, "assets": self.assets})
        )

    def write_archive(
        self,
        archive: Path,
        filename: str,
        *,
        bad_name: str | None = None,
        bad_kind: bytes = tarfile.REGTYPE,
        duplicate: bool = False,
        missing_uv: bool = False,
    ) -> None:
        root = filename.removesuffix(".tar.gz")
        with tarfile.open(archive, "w:gz") as output:
            directory = tarfile.TarInfo(root)
            directory.type = tarfile.DIRTYPE
            output.addfile(directory)
            records = [(root + "/uv", self.binary), (root + "/uvx", b"uvx test bytes")]
            for name, raw in records:
                if name.endswith("/uv") and missing_uv:
                    continue
                member = tarfile.TarInfo(bad_name or name)
                member.type = bad_kind
                member.linkname = "/etc/passwd"
                member.size = len(raw) if member.isfile() else 0
                output.addfile(member, io.BytesIO(raw))
                if duplicate:
                    output.addfile(member, io.BytesIO(raw))

    def download(self, version: str, filename: str, destination: Path) -> None:
        self.assertEqual(version, "0.13.6")
        shutil.copyfile(self.archives[filename], destination)

    def install(self) -> Path:
        with (
            patch.object(bootstrap, "download_uv_archive", side_effect=self.download),
            patch.object(bootstrap, "uv_version_matches", return_value=True),
        ):
            return bootstrap.ensure_uv(self.project, self.cache, prefer_path=False)

    def test_runtime_version_is_canonical_and_metadata_must_match_both_arches(self) -> None:
        version, checksums = bootstrap.uv_manifest(self.project)
        self.assertEqual(version, "0.13.6")
        self.assertEqual(set(checksums), set(bootstrap.UV_ASSETS.values()))
        self.assets[1]["currentVersion"] = "0.13.7"
        self.metadata()
        with self.assertRaisesRegex(bootstrap.BootstrapError, "disagrees"):
            bootstrap.uv_manifest(self.project)
        (self.project / "pyproject.toml").write_text('[tool.uv]\nrequired-version = ">=0.13.6"\n')
        with self.assertRaisesRegex(bootstrap.BootstrapError, "exact release"):
            bootstrap.uv_manifest(self.project)

    def test_metadata_rejects_missing_duplicate_malformed_and_unreviewed_assets(self) -> None:
        for mutation in ("missing", "duplicate", "malformed", "unreviewed"):
            value = json.loads((self.project / "uv-artifacts.json").read_text())
            if mutation == "missing":
                value["assets"].pop()
            elif mutation == "duplicate":
                value["assets"][1] = value["assets"][0]
            elif mutation == "malformed":
                value["assets"][0]["sha256"] = "not a digest"
            else:
                value["assets"][0]["filename"] = "uv-windows-unreviewed.zip"
            (self.project / "uv-artifacts.json").write_text(json.dumps(value))
            with self.subTest(mutation=mutation), self.assertRaises(bootstrap.BootstrapError):
                bootstrap.uv_manifest(self.project)
            self.metadata()
        (self.project / "pyproject.toml").write_text('tool = "malformed table"\n')
        with self.assertRaises(bootstrap.BootstrapError):
            bootstrap.uv_manifest(self.project)

    def test_unsupported_host_fails_before_using_path_uv_or_network(self) -> None:
        for system, machine in (("Darwin", "aarch64"), ("Linux", "riscv64"), ("Linux", "arm64")):
            with (
                patch.object(bootstrap.platform, "system", return_value=system),
                patch.object(bootstrap.platform, "machine", return_value=machine),
                patch.object(bootstrap, "download_uv_archive") as download,
                self.assertRaisesRegex(
                    bootstrap.BootstrapError, "supports Linux x86_64 and aarch64"
                ),
            ):
                bootstrap.ensure_uv(self.project, self.cache)
            download.assert_not_called()

    def test_matching_path_uv_is_checked_and_used_without_installing(self) -> None:
        with (
            patch.object(bootstrap.shutil, "which", return_value="/ambient/uv"),
            patch.object(bootstrap, "uv_version_matches", return_value=True) as version,
            patch.object(bootstrap, "download_uv_archive") as download,
        ):
            executable = bootstrap.ensure_uv(self.project, self.cache)
        self.assertEqual(executable, Path("/ambient/uv"))
        version.assert_called_once_with(Path("/ambient/uv"), "0.13.6")
        download.assert_not_called()

    def test_wrong_path_version_uses_official_cache_and_aarch64_has_own_asset(self) -> None:
        with (
            patch.object(bootstrap.platform, "machine", return_value="aarch64"),
            patch.object(bootstrap.shutil, "which", return_value="/wrong/uv"),
            patch.object(bootstrap, "uv_version_matches", side_effect=[False, True]),
            patch.object(bootstrap, "download_uv_archive", side_effect=self.download) as download,
        ):
            executable = bootstrap.ensure_uv(self.project, self.cache)
        self.assertIn("uv-aarch64-unknown-linux-gnu", str(executable))
        self.assertEqual(download.call_args.args[1], "uv-aarch64-unknown-linux-gnu.tar.gz")
        self.assertEqual(executable.read_bytes(), self.binary)
        self.assertEqual(executable.stat().st_mode & 0o777, 0o755)

    def test_cached_archive_and_binary_are_authenticated_without_trusting_sidecars(self) -> None:
        executable = self.install()
        executable.write_bytes(b"attacker uv executable")
        (executable.parent / "uv.sha256").write_text(
            hashlib.sha256(executable.read_bytes()).hexdigest()
        )
        with (
            patch.object(bootstrap, "download_uv_archive") as download,
            patch.object(bootstrap, "uv_version_matches", return_value=True),
        ):
            self.assertEqual(
                bootstrap.ensure_uv(self.project, self.cache, prefer_path=False), executable
            )
        download.assert_not_called()
        self.assertEqual(executable.read_bytes(), self.binary)
        archive = executable.parent / bootstrap.UV_ASSETS["x86_64"]
        archive.write_bytes(b"attacker archive")
        (archive.parent / "archive.sha256").write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest()
        )
        with (
            patch.object(bootstrap, "uv_version_matches") as version,
            self.assertRaisesRegex(bootstrap.BootstrapError, "cached uv archive SHA256"),
        ):
            bootstrap.ensure_uv(self.project, self.cache, prefer_path=False)
        version.assert_not_called()

    def test_download_checksum_failure_never_extracts_or_executes(self) -> None:
        def corrupt(version: str, filename: str, destination: Path) -> None:
            destination.write_bytes(b"wrong downloaded bytes")

        with (
            patch.object(bootstrap, "download_uv_archive", side_effect=corrupt),
            patch.object(bootstrap, "extract_uv_binary") as extract,
            patch.object(bootstrap, "uv_version_matches") as version,
            self.assertRaisesRegex(bootstrap.BootstrapError, "downloaded uv archive SHA256"),
        ):
            bootstrap.ensure_uv(self.project, self.cache, prefer_path=False)
        extract.assert_not_called()
        version.assert_not_called()

    def test_verified_binary_reporting_wrong_version_fails(self) -> None:
        with (
            patch.object(bootstrap, "download_uv_archive", side_effect=self.download),
            patch.object(bootstrap, "uv_version_matches", return_value=False),
            self.assertRaisesRegex(bootstrap.BootstrapError, "does not report"),
        ):
            bootstrap.ensure_uv(self.project, self.cache, prefer_path=False)

    def test_tar_traversal_links_duplicate_paths_and_missing_uv_fail(self) -> None:
        filename = bootstrap.UV_ASSETS["x86_64"]
        archive = self.root / "invalid.tar.gz"
        for name, kind, duplicate, missing in (
            ("../../outside", tarfile.REGTYPE, False, False),
            (None, tarfile.SYMTYPE, False, False),
            (None, tarfile.LNKTYPE, False, False),
            (None, tarfile.REGTYPE, True, False),
            (None, tarfile.REGTYPE, False, True),
        ):
            self.write_archive(
                archive,
                filename,
                bad_name=name,
                bad_kind=kind,
                duplicate=duplicate,
                missing_uv=missing,
            )
            output = self.root / "extracted"
            output.unlink(missing_ok=True)
            with self.assertRaises(bootstrap.BootstrapError):
                bootstrap.extract_uv_binary(archive, filename, output)
        self.assertFalse((self.root.parent / "outside").exists())

    def test_extraction_and_file_hash_size_bounds_are_enforced(self) -> None:
        archive = self.archives[bootstrap.UV_ASSETS["x86_64"]]
        with patch.object(bootstrap, "MAX_UV_EXTRACTED_BYTES", 1):
            with self.assertRaisesRegex(bootstrap.BootstrapError, "extracted size"):
                bootstrap.extract_uv_binary(archive, archive.name, self.root / "too-large")
        with self.assertRaisesRegex(bootstrap.BootstrapError, "oversized"):
            bootstrap.bounded_sha256(archive, 1)

    def test_cache_links_are_rejected(self) -> None:
        linked = self.root / "linked"
        linked.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(bootstrap.BootstrapError, "directory symlinks"):
            bootstrap.ensure_uv(self.project, linked / "cache", prefer_path=False)
        executable = self.install()
        executable.unlink()
        executable.symlink_to(self.root / "other")
        with self.assertRaisesRegex(bootstrap.BootstrapError, "executable must not be a symlink"):
            bootstrap.ensure_uv(self.project, self.cache, prefer_path=False)

    def test_version_check_is_exact_and_bounded(self) -> None:
        for raw, accepted in (
            (b"uv 0.13.6\n", True),
            (b"uv 0.13.60\n", False),
            (b"uv 0.13.5\n", False),
            (b"uv 0.13.6\nextra", False),
        ):
            result = subprocess.CompletedProcess([], 0, stdout=raw)
            with patch.object(bootstrap.subprocess, "run", return_value=result) as run:
                self.assertEqual(bootstrap.uv_version_matches(Path("/uv"), "0.13.6"), accepted)
            self.assertEqual(run.call_args.kwargs["timeout"], 10)
        with patch.object(
            bootstrap.subprocess, "run", side_effect=subprocess.TimeoutExpired("uv", 10)
        ):
            self.assertFalse(bootstrap.uv_version_matches(Path("/uv"), "0.13.6"))

    def test_shared_cli_outputs_verified_path_and_github_path_without_shell(self) -> None:
        executable = self.root / "verified/uv"
        github_path = self.root / "github-path"
        with (
            patch.object(bootstrap, "ensure_uv", return_value=executable) as ensure,
            patch("sys.stdout", new_callable=io.StringIO) as output,
        ):
            code = bootstrap.main(
                [
                    "install-uv",
                    "--project",
                    str(self.project),
                    "--cache-root",
                    str(self.cache),
                    "--github-path",
                    str(github_path),
                    "--no-path-uv",
                ]
            )
        self.assertEqual(code, 0)
        self.assertEqual(output.getvalue(), str(executable) + "\n")
        self.assertEqual(github_path.read_text(), str(executable.parent) + "\n")
        ensure.assert_called_once_with(self.project, self.cache, prefer_path=False)

    def test_shared_cli_rejects_environment_file_control_characters(self) -> None:
        with (
            patch.object(bootstrap, "ensure_uv", return_value=self.root / "bad\npath/uv"),
            patch("sys.stderr", new_callable=io.StringIO) as output,
        ):
            code = bootstrap.main(
                [
                    "install-uv",
                    "--project",
                    str(self.project),
                    "--cache-root",
                    str(self.cache),
                    "--github-path",
                    str(self.root / "github-path"),
                ]
            )
        self.assertEqual(code, 2)
        self.assertIn("control character", output.getvalue())

    def test_downloader_uses_fixed_official_url_without_credentials(self) -> None:
        class Response(io.BytesIO):
            def __init__(self, raw: bytes) -> None:
                super().__init__(raw)
                self.headers = {"Content-Length": "3"}

        with (
            patch.object(bootstrap, "build_opener") as opener,
            patch.dict(
                os.environ, {"GH_TOKEN": "must-not-be-forwarded", "GITHUB_TOKEN": "also-private"}
            ),
        ):
            opener.return_value.open.return_value = Response(b"uv!")
            output = self.root / "downloaded"
            bootstrap.download_uv_archive("0.13.6", bootstrap.UV_ASSETS["x86_64"], output)
        request = opener.return_value.open.call_args.args[0]
        self.assertEqual(
            request.full_url,
            "https://github.com/astral-sh/uv/releases/download/0.13.6/uv-x86_64-unknown-linux-gnu.tar.gz",
        )
        self.assertIsNone(request.get_header("Authorization"))
        self.assertEqual(output.read_bytes(), b"uv!")
        self.assertEqual(opener.return_value.open.call_args.kwargs["timeout"], 30)

    def test_downloader_enforces_declared_actual_and_time_bounds(self) -> None:
        class Response(io.BytesIO):
            def __init__(self, raw: bytes) -> None:
                super().__init__(raw)
                self.headers: dict[str, str] = {}

        filename = bootstrap.UV_ASSETS["x86_64"]
        for headers, raw, times in (
            ({"Content-Length": "999"}, b"uv", [0]),
            ({}, b"oversized", [0, 1]),
            ({}, b"uv", [0, 121]),
        ):
            output = self.root / "bounded-download"
            output.unlink(missing_ok=True)
            response = Response(raw)
            response.headers = headers
            with (
                patch.object(bootstrap, "build_opener") as opener,
                patch.object(bootstrap, "MAX_UV_ARCHIVE_BYTES", 3),
                patch.object(bootstrap.time, "monotonic", side_effect=times),
                self.assertRaises(bootstrap.BootstrapError),
            ):
                opener.return_value.open.return_value = response
                bootstrap.download_uv_archive("0.13.6", filename, output)

    def test_release_redirect_rejects_non_https_and_unreviewed_hosts(self) -> None:
        request = bootstrap.Request(
            "https://github.com/astral-sh/uv/releases/download/0.13.6/asset"
        )

        for location in (
            "http://github.com/asset",
            "https://attacker.invalid/asset",
            "https://token@release-assets.githubusercontent.com/asset",
        ):
            with self.assertRaisesRegex(bootstrap.BootstrapError, "reviewed HTTPS hosts"):
                bootstrap.UvArchiveRedirect().redirect_request(
                    request, io.BytesIO(), 302, "found", HTTPMessage(), location
                )


if __name__ == "__main__":
    unittest.main()
