"""Query the canonical Node version from the selected immutable shared checkout."""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

from . import bootstrap, devcontainer_validation


class NodeSetupError(ValueError):
    """The selected shared Node authority could not be verified."""


def _module_root(filename: str) -> Path:
    path = Path(filename).absolute()
    if len(path.parents) < 4:
        raise NodeSetupError("shared Node authority must own its complete source layout")
    root = path.parents[3]
    for entry in (path, *path.parents):
        if entry.is_symlink():
            raise NodeSetupError("shared Node module path must not follow symlinks")
        if entry == root:
            break
    return root.resolve(strict=True)


def node_version(automation_root: Path, automation_sha: str) -> str:
    """Bind all loaded authority modules to one clean checkout, then reuse its reader."""
    try:
        root = automation_root.resolve(strict=True)
        if any(
            _module_root(filename) != root
            for filename in (__file__, bootstrap.__file__, devcontainer_validation.__file__)
        ):
            raise NodeSetupError("loaded shared Node authority differs from selected checkout")
        identity = bootstrap.automation_identity(root, automation_sha, allow_dirty_self=False)
        if identity != {"revision": automation_sha, "kind": "git-checkout"}:
            raise NodeSetupError("shared Node authority must be the exact immutable checkout")
        version = devcontainer_validation.canonical_versions(root)["node"]
        if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None:
            raise NodeSetupError("canonical Node version must be an exact numeric release")
        return version
    except (OSError, ValueError, KeyError, TypeError) as error:
        if isinstance(error, NodeSetupError):
            raise
        raise NodeSetupError(
            "selected immutable Node authority is unavailable or invalid"
        ) from None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--automation-root", required=True, type=Path)
    parser.add_argument("--automation-sha", required=True)
    parser.add_argument("--github-output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if not sys.flags.isolated:
            raise NodeSetupError("shared Node query requires isolated Python")
        root = args.automation_root.resolve(strict=True)
        interpreter = root / "automation/.venv/bin/python"
        if (
            Path(sys.executable).absolute() != interpreter
            or not interpreter.is_file()
            or not os.access(interpreter, os.X_OK)
        ):
            raise NodeSetupError("selected shared project interpreter is required")
        version = node_version(root, args.automation_sha)
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"node_version={version}\n")
        return 0
    except (OSError, ValueError):
        print(
            "Shared Node setup: selected immutable authority could not be verified", file=sys.stderr
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
