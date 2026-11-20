"""Exercise the release artifact contract with a dependency-free Cargo package."""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HELPER = Path(__file__).resolve().parents[1] / "src/boxferry_ci/package_crate.py"
CRATE = "target/package/release-fixture-1.2.3.crate"


class PackageCrateTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="boxferry-package-")
        self.addCleanup(temporary.cleanup)
        self.workspace = Path(temporary.name)
        self.helper = self.workspace / "_automation/automation/src/boxferry_ci/package_crate.py"
        self.helper.parent.mkdir(parents=True)
        (self.helper.parent / "__init__.py").touch()
        shutil.copyfile(HELPER, self.helper)
        self.env = {
            **os.environ,
            "CARGO_HOME": str(self.workspace / "cargo-home"),
            "CARGO_NET_OFFLINE": "true",
            "CARGO_TARGET_DIR": "target",
            "PYTHONPATH": str(self.helper.parent.parent),
        }

    def create_package(self, candidate: Path, library: str = "pub fn example() {}\n") -> None:
        (candidate / "src").mkdir(parents=True)
        (candidate / ".cargo").mkdir()
        (candidate / "src/lib.rs").write_text(library, encoding="utf-8")
        (candidate / ".cargo/config.toml").write_text(
            '[alias]\nci-package = "package --locked"\n', encoding="utf-8"
        )
        (candidate / "Cargo.toml").write_text(
            '[package]\nname = "release-fixture"\nversion = "1.2.3"\nedition = "2021"\n'
            'license = "MPL-2.0"\ndescription = "Local release packaging test"\n'
            'include = ["src/lib.rs"]\n',
            encoding="utf-8",
        )
        subprocess.run(
            ["cargo", "generate-lockfile"],
            cwd=candidate,
            env=self.env,
            check=True,
            capture_output=True,
            timeout=30,
        )

    def package(self, candidate: Path, crate: str = CRATE) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-m", "boxferry_ci.package_crate", crate],
            cwd=candidate,
            env=self.env,
            capture_output=True,
            text=True,
            timeout=90,
        )

    def test_archive_and_sidecar_in_both_checkout_layouts(self) -> None:
        for layout in ("source", "."):
            with self.subTest(layout=layout):
                candidate = self.workspace / layout
                self.create_package(candidate)
                result = self.package(candidate)
                self.assertEqual(result.returncode, 0, result.stderr)

                archive = candidate / CRATE
                self.assertEqual(list(archive.parent.glob("*.crate")), [archive])
                sidecar = archive.with_suffix(".crate.sha256")
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                self.assertEqual(sidecar.read_bytes(), f"{digest}  {CRATE}\n".encode("ascii"))
                subprocess.run(
                    ["sha256sum", "--check", "--status", f"{CRATE}.sha256"],
                    cwd=candidate,
                    check=True,
                    timeout=30,
                )

    def test_cargo_error_leaves_no_sidecar(self) -> None:
        self.create_package(self.workspace, 'compile_error!("intentional packaging failure");\n')
        result = self.package(self.workspace)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("intentional packaging failure", result.stderr)
        self.assertFalse((self.workspace / f"{CRATE}.sha256").exists())

    def test_missing_expected_archive_leaves_no_sidecar(self) -> None:
        self.create_package(self.workspace)
        missing = "target/package/release-fixture-1.2.4.crate"
        result = self.package(self.workspace, missing)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"Cannot package {missing}", result.stderr)
        self.assertTrue((self.workspace / CRATE).is_file())
        self.assertFalse((self.workspace / f"{missing}.sha256").exists())

    def test_absolute_archive_path_fails_before_cargo(self) -> None:
        self.create_package(self.workspace)
        result = self.package(self.workspace, str(self.workspace / CRATE))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("crate must be a relative target/package/", result.stderr)
        self.assertFalse((self.workspace / CRATE).exists())
