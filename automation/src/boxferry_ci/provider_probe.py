"""Canonical manual cancellation-probe commands; admission never authorizes execution.

Run only from an already-held admitted checkout and already-installed locked Python
with network acquisition disabled. Initialization and descriptor creation are local;
all experiment HTTP is reserved in fresh zero-start custody before transport.
"""

from __future__ import annotations

import argparse
import json
import sys

from . import provider_probe_controller as controller
from . import provider_probe_observer as observer
from . import provider_probe_source as source


def main(argv: list[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    try:
        if arguments and arguments[0] == "control":
            return controller.main(arguments[1:])
        if arguments and arguments[0] == "observe":
            return observer.main(arguments[1:])
        parser = argparse.ArgumentParser(description=__doc__)
        commands = parser.add_subparsers(dest="command", required=True)
        descriptor = commands.add_parser(
            "describe", help="local descriptor; independently review before use"
        )
        descriptor.add_argument("--source-sha", required=True)
        descriptor.add_argument("--main-ci-run", required=True, type=int)
        commands.add_parser("init", add_help=False)
        args, rest = parser.parse_known_args(arguments)
        if args.command == "init":
            return observer.main(["init", *rest])
        parser.parse_args(arguments)
        sys.stdout.write(
            source.encoded(source.describe(args.source_sha, args.main_ci_run)).decode()
        )
        return 0
    except Exception as error:
        # This is also the bounded fallback for failures before custody can be loaded.
        print(
            json.dumps(
                {
                    "outcome": "INCONCLUSIVE",
                    "diagnostic_version": 2,
                    "phase_code": "INITIALIZATION_OR_ABORT_RECORD",
                    "refusal_code": controller.refusal_code(error),
                    "requests": None,
                }
            )
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
