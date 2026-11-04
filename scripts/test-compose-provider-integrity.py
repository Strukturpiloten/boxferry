#!/usr/bin/env python3
"""Exercise the canonical provider's interface and checksum failure without network."""

import hashlib
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROVIDER = ROOT / "scripts/lib/compose-provider.sh"


class ProviderIntegrity(unittest.TestCase):
    def test_existing_pin_and_repeat_source(self) -> None:
        result = subprocess.run(
            [
                "bash",
                "-c",
                'source "$1" && source "$1" && '
                'printf "%s\\n%s\\n" "$BOXFERRY_COMPOSE_PROVIDER_VERSION" '
                '"$BOXFERRY_COMPOSE_PROVIDER_URL"',
                "test",
                str(PROVIDER),
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        version, url = result.stdout.splitlines()
        self.assertRegex(version, r"^[0-9]+\.[0-9]+\.[0-9]+$")
        self.assertEqual(
            url,
            f"https://github.com/docker/compose/releases/download/v{version}/docker-compose-linux-x86_64",
        )

    def test_bad_download_refuses_before_chmod(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            content = PROVIDER.read_text()
            digest = hashlib.sha256(b"good").hexdigest()
            content, changed = re.subn(
                r'(?<=expected_sha256=")[a-f0-9]{64}', digest, content
            )
            self.assertEqual(changed, 1)
            provider = root / "provider.sh"
            provider.write_text(content)
            curl = root / "curl"
            curl.write_text(
                "#!/bin/bash\nwhile (($#)); do\n"
                'if [[ "$1" == --output ]]; then printf bad > "$2"; exit; fi\n'
                "shift; done\nexit 1\n"
            )
            curl.chmod(0o755)
            target = root / "binary"
            result = subprocess.run(
                [
                    "bash",
                    "-e",
                    "-c",
                    'source "$1"; boxferry_install_compose_provider "$2"',
                    "test",
                    str(provider),
                    str(target),
                ],
                env={**os.environ, "PATH": f"{root}{os.pathsep}{os.environ['PATH']}"},
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(target.read_bytes(), b"bad")
            self.assertFalse(target.stat().st_mode & 0o111)


if __name__ == "__main__":
    unittest.main()
