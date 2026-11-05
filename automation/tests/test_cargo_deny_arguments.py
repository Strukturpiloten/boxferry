"""Actual selected CLI controls for nested, explicit Cargo Deny policy ownership."""

from __future__ import annotations

import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "automation/tests/fixtures/cargo-deny-action/source"


class CargoDenyArgumentsTests(unittest.TestCase):
    def setUp(self) -> None:
        executable = shutil.which("cargo-deny")
        if executable is None:
            self.skipTest(
                "runner has no Cargo Deny; required dependency job exercises its pinned Action"
            )
        self.executable = executable
        expected = re.findall(
            r"(?m)^ARG CARGO_DENY_VERSION=([0-9]+\.[0-9]+\.[0-9]+)$",
            (ROOT / ".devcontainer/Dockerfile").read_text(),
        )
        self.assertEqual(len(expected), 1)
        version = subprocess.run(
            [self.executable, "--version"], capture_output=True, text=True, check=True, timeout=10
        )
        self.assertEqual(version.stdout.strip(), f"cargo-deny {expected[0]}")
        directory = tempfile.TemporaryDirectory(prefix="boxferry-cargo-deny-arguments-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        shutil.copytree(FIXTURE, self.root / "source")
        # A different valid root crate detects loss of the Action's manifest-path input.
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="wrong-root-must-not-be-audited"\nversion="0.0.0"\n'
            'edition="2024"\nlicense="MPL-2.0"\n'
        )
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text("// Metadata-only decoy.\n")

    def run_cli(
        self, *arguments: str, manifest: str = "source/Cargo.toml"
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [self.executable, "--offline", "--manifest-path", manifest, *arguments],
            cwd=self.root,
            capture_output=True,
            text=True,
            check=False,
            timeout=45,
        )

    def test_explicit_nondefault_nested_policy_and_manifest_succeed_without_compilation(
        self,
    ) -> None:
        result = self.run_cli(
            "--all-features",
            "--config",
            "source/explicit-policy.toml",
            "check",
            "licenses",
            "bans",
            "sources",
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for audit in ("licenses", "bans", "sources"):
            self.assertIn(f"{audit} ok", result.stdout + result.stderr)
        self.assertFalse((self.root / "source/target").exists(), "metadata checks must not compile")

    def test_old_command_specific_config_placement_is_rejected_before_auditing(self) -> None:
        result = self.run_cli(
            "--all-features", "check", "--config", "source/explicit-policy.toml", "licenses"
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("unexpected argument '--config'", result.stderr)
        self.assertNotIn("licenses ok", result.stdout + result.stderr)

    def test_implicit_or_explicit_decoy_policy_cannot_pass(self) -> None:
        for configuration in ((), ("--config", "source/deny.toml")):
            with self.subTest(configuration=configuration):
                result = self.run_cli("--all-features", *configuration, "check", "licenses")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("license is not explicitly allowed", result.stderr)
                self.assertIn("BSD-3-Clause", result.stderr)

    def test_wrong_root_manifest_or_missing_explicit_policy_cannot_pass(self) -> None:
        wrong = self.run_cli(
            "--all-features",
            "--config",
            "source/explicit-policy.toml",
            "check",
            "licenses",
            manifest="Cargo.toml",
        )
        self.assertNotEqual(wrong.returncode, 0)
        self.assertIn("wrong-root-must-not-be-audited", wrong.stderr)
        missing = self.run_cli(
            "--all-features", "--config", "source/missing-policy.toml", "check", "licenses"
        )
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn("missing-policy.toml", missing.stderr)


if __name__ == "__main__":
    unittest.main()
