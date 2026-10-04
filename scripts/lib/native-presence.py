#!/usr/bin/env python3
"""Closed read-only local/Unix-socket Podman queries for disposable harnesses."""

from __future__ import annotations

import argparse
import importlib.util
import pathlib
import re
import signal
import time

SPEC = importlib.util.spec_from_file_location(
    "bounded_native_read", pathlib.Path(__file__).with_name("bounded-native-read.py"))
assert SPEC is not None and SPEC.loader is not None
native_read = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native_read)


def presence(engine: str, kind: str, name: str, socket: str | None = None) -> str:
    """Presence is observation, never ownership or authority to mutate/reuse."""
    if kind not in ("container", "volume", "network", "image") or re.fullmatch(r"[A-Za-z0-9_./-]+", engine) is None:
        return "unknown"
    pattern = r"[A-Za-z0-9][A-Za-z0-9_.:/@+-]{0,511}" if kind == "image" else r"[A-Za-z0-9][A-Za-z0-9_.-]{0,255}"
    if re.fullmatch(pattern, name) is None:
        return "unknown"
    if socket is not None and (not socket.startswith("/") or "\x00" in socket or "\n" in socket):
        return "unknown"
    arguments = [engine]
    if socket is not None:
        arguments.extend(["--url", "unix://" + socket])
    arguments.extend([kind, "exists", name])
    previous = {}
    def cancelled(_signum, _frame):
        raise KeyboardInterrupt
    try:
        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            previous[signum] = signal.signal(signum, cancelled)
        outcome, _ = native_read.readiness_read(arguments, time.monotonic() + 4, presence=True)
        return outcome if outcome in ("present", "absent") else "unknown"
    except (OSError, KeyboardInterrupt):
        return "unknown"
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", required=True)
    parser.add_argument("--kind", choices=("container", "volume", "network", "image"), required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--socket")
    args = parser.parse_args()
    outcome = presence(args.engine, args.kind, args.name, args.socket)
    print(outcome)
    return {"present": 0, "absent": 1, "unknown": 2}[outcome]


if __name__ == "__main__":
    raise SystemExit(main())
