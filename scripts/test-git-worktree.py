#!/usr/bin/env python3
"""Preserved explicit complete-gate entry point for shared source/index tests."""

import importlib.util
import sys
import unittest
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(root / "automation/src"))
    specification = importlib.util.spec_from_file_location(
        "git_worktree_contracts", root / "automation/tests/test_git_worktree.py"
    )
    assert specification is not None and specification.loader is not None
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    unittest.main(module=module)


if __name__ == "__main__":
    main()
