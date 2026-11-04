#!/usr/bin/env python3
"""Run the typed Kubernetes checksum refresher from the automation package."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "automation/src"))

from boxferry_ci.kubernetes_tool_checksum_refresh import RefreshError, main  # noqa: E402

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RefreshError as error:
        print(f"Kubernetes checksum refresh failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
