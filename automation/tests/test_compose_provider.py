"""Run provider integrity and native Renovate contracts in complete automation gates."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class ComposeProviderContracts(unittest.TestCase):
    def test_installer_integrity(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/test-compose-provider-integrity.py")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_native_renovate(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        self.assertTrue(runtime.is_dir(), "provision the pinned native Renovate runtime first")
        marker = runtime / "source-image.txt"
        node = runtime / "node"
        self.assertTrue(marker.is_file(), "native Renovate runtime identity marker is missing")
        self.assertTrue(node.is_file(), "native Renovate runtime Node executable is missing")
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual(marker.read_text().strip(), pin, "native Renovate runtime pin differs")
        result = subprocess.run(
            [
                str(node),
                str(ROOT / "scripts/test-compose-provider-renovate.cjs"),
                str(runtime),
                str(ROOT),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
