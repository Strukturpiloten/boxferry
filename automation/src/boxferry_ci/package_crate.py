r"""Package the current Cargo checkout and write its release checksum.

From the candidate directory, set CRATE to the release metadata path and run
with the pinned BoxFerry automation project:

  uv run --locked --project "${GITHUB_WORKSPACE}/_automation/automation" \
    python -m boxferry_ci.package_crate "${CRATE}"

The sidecar uses the candidate-relative archive path for sha256sum --check.
This helper needs no release credentials.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import subprocess
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("crate", help="expected target/package/<name>-<version>.crate path")
    args = parser.parse_args()
    if not re.fullmatch(r"target/package/[A-Za-z0-9_.+-]+\.crate", args.crate):
        parser.error("crate must be a relative target/package/<name>-<version>.crate path")

    try:
        subprocess.run(["cargo", "ci-package"], check=True)
        with Path(args.crate).open("rb") as archive:
            digest = hashlib.file_digest(archive, "sha256").hexdigest()
        Path(f"{args.crate}.sha256").write_bytes(f"{digest}  {args.crate}\n".encode("ascii"))
    except subprocess.CalledProcessError as error:
        return error.returncode
    except OSError as error:
        print(f"Cannot package {args.crate}: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
