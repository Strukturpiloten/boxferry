#!/usr/bin/env python3
"""Explicit caller for the repository's canonical source/index helper."""

import json
import os
import re
import sys
from pathlib import Path


def main() -> int:
    system_python = sys.version_info
    if system_python < (3, 13):
        # The pinned image retains the locked shared Python. Do not bootstrap,
        # download, or select an unrelated system interpreter during startup.
        tooling = Path("/opt/boxferry-tooling")
        try:
            manifest = tooling / ".github/automation.json"
            if manifest.stat().st_size > 4096:
                raise ValueError("retained automation manifest exceeds its bound")
            document = json.loads(manifest.read_text())
            if not isinstance(document, dict):
                raise ValueError("retained automation manifest must be an object")
            revision = document["revision"]
            if not isinstance(revision, str) or re.fullmatch(r"[a-f0-9]{40}", revision) is None:
                raise ValueError("retained automation revision must be an exact commit")
            interpreter = (
                tooling / "target/boxferry-automation" / revision / "automation/.venv/bin/python"
            )
            if not interpreter.is_file() or not os.access(interpreter, os.X_OK):
                raise ValueError("retained locked Python is unavailable")
            program = (
                "import runpy,sys\n"
                "if sys.version_info < (3,13):\n"
                " raise SystemExit('Locked Python must be 3.13 or newer; rebuild the image')\n"
                "sys.argv=sys.argv[1:]\n"
                "runpy.run_path(sys.argv[0],run_name='__main__')\n"
            )
            os.execv(interpreter, [str(interpreter), "-I", "-c", program, *sys.argv])
        except (OSError, ValueError, KeyError) as error:
            print(
                f"BoxFerry Git source check needs locked Python 3.13: {error}; "
                "rebuild the canonical image, or on the host run uv run --locked "
                "--project automation python scripts/git-worktree.py with the same arguments",
                file=sys.stderr,
            )
            return 1
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "automation/src"))
    from boxferry_ci.git_worktree import main as inspect_main

    return inspect_main()


if __name__ == "__main__":
    raise SystemExit(main())
