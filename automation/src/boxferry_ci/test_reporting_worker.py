"""Exactly one normal unittest discovery/run. Descriptors stay captured through process exit."""

from __future__ import annotations

import argparse
import os
import sys
import time
import unittest
from collections.abc import Iterator
from itertools import chain
from pathlib import Path
from types import TracebackType

from boxferry_ci.test_reporting_data import (
    PRIMARY_BYTES,
    RESULT_BYTES,
    TRACE_BYTES,
    Record,
    Shield,
    TestData,
    write_json,
)

ExceptionInfo = (
    tuple[type[BaseException], BaseException, TracebackType | None] | tuple[None, None, None]
)


class Result(unittest.TestResult):
    def __init__(self, root: Path, shield: Shield, primary: Path) -> None:
        super().__init__()
        self.root = root
        self.discovered = 0
        self.shield = shield
        self.primary = primary
        self.counts = dict(success=0, failure=0, error=0, skipped=0, expected=0, unexpected=0)
        self.first: list[Record] = []
        self.last: list[Record] = []
        self.report_errors: list[str] = []
        self.trace_omitted = 0

    def startTest(self, test: unittest.TestCase) -> None:
        super().startTest(test)
        sys.stderr.write(self.shield.line(test.id()) + " ... ")

    def addDuration(self, test: unittest.TestCase, elapsed: float) -> None:
        # Outcome counters suffice; never retain one duration entry per test.
        pass

    def addSuccess(self, test: unittest.TestCase) -> None:
        self.counts["success"] += 1
        sys.stderr.write("ok\n")

    def addSkip(self, test: unittest.TestCase, reason: str) -> None:
        self.counts["skipped"] += 1
        sys.stderr.write("skipped: " + self.shield.line(reason) + "\n")

    def addExpectedFailure(self, test: unittest.TestCase, err: ExceptionInfo) -> None:
        self.counts["expected"] += 1
        sys.stderr.write("expected failure\n")

    def addUnexpectedSuccess(self, test: unittest.TestCase) -> None:
        self.counts["unexpected"] += 1
        self._record(test, "UnexpectedSuccess", "expected failure succeeded", None)

    def addFailure(self, test: unittest.TestCase, err: ExceptionInfo) -> None:
        self.counts["failure"] += 1
        if err[0] is None or err[1] is None:
            raise ValueError("missing unittest exception information")
        self._record(test, err[0].__name__, str(err[1]), err[2])

    def addError(self, test: unittest.TestCase, err: ExceptionInfo) -> None:
        self.counts["error"] += 1
        if err[0] is None or err[1] is None:
            raise ValueError("missing unittest exception information")
        self._record(test, err[0].__name__, str(err[1]), err[2])

    def addSubTest(
        self, test: unittest.TestCase, subtest: unittest.TestCase, err: ExceptionInfo | None
    ) -> None:
        if err is not None:
            if err[0] is None or err[1] is None:
                raise ValueError("missing subtest exception information")
            name = "failure" if issubclass(err[0], test.failureException) else "error"
            self.counts[name] += 1
            self._record(subtest, err[0].__name__, str(err[1]), err[2])

    def wasSuccessful(self) -> bool:
        return not any(self.counts[name] for name in ("failure", "error", "unexpected"))

    def _record(
        self, test: unittest.TestCase, kind: str, cause: str, traceback: TracebackType | None
    ) -> None:
        location = "unavailable"

        def frames() -> Iterator[str]:
            # Tracebacks contain no locals or source-file reads. At most one bounded
            # frame is transformed at a time; retained trace head/tail is separate.
            nonlocal location, traceback
            while traceback is not None:
                frame = traceback.tb_frame
                filename = frame.f_code.co_filename
                try:
                    display = Path(filename).relative_to(self.root).as_posix()
                except ValueError:
                    display = Path(filename).name
                if location == "unavailable" and "unittest" not in filename:
                    location = self.shield.line(f"{display}:{traceback.tb_lineno}", 512)
                yield self.shield.text(
                    f'  File "{display}", line {traceback.tb_lineno}, in {frame.f_code.co_name}\n',
                    1024,
                )
                traceback = traceback.tb_next

        trace, omitted = self.shield.parts(chain(frames(), (f"{kind}: {cause}\n",)), TRACE_BYTES)
        self.trace_omitted += omitted
        record: Record = {
            "test_id": self.shield.line(test.id()),
            "exception": self.shield.line(kind, 256),
            "cause": self.shield.line(cause),
            "location": location,
            "traceback": trace,
            "omitted_trace_bytes": omitted,
        }
        if len(self.first) < 16:
            self.first.append(record)
        else:
            self.last.append(record)
            del self.last[: max(0, len(self.last) - 8)]
        if len(self.first) == 1 and not self.last:
            self.write_primary(self.value())
        sys.stderr.write("FAIL\n" + trace)

    def write_primary(self, value: TestData) -> None:
        """Reserve counters and first/last causes even when final JSON cannot fit."""
        selected = value["records"][:1]
        if len(value["records"]) > 1:
            selected += value["records"][-1:]
        compact: list[Record] = []
        omitted = value["omitted_trace_bytes"]
        for original in selected:
            record = original.copy()
            record["test_id"] = self.shield.line(original["test_id"], 512)
            record["cause"] = self.shield.line(original["cause"], 512)
            record["location"] = self.shield.line(original["location"], 256)
            record["traceback"], clipped = self.shield.parts((original["traceback"],), 2048)
            record["omitted_trace_bytes"] += clipped
            omitted += clipped
            compact.append(record)
        reserve = value.copy()
        reserve["records"] = compact
        reserve["omitted_trace_bytes"] = omitted
        failures = sum(value["counts"][name] for name in ("failure", "error", "unexpected"))
        reserve["omitted_records"] = failures - len(compact)
        temporary = self.primary.with_suffix(".next")
        try:
            write_json(temporary, reserve, PRIMARY_BYTES)
            os.replace(temporary, self.primary)
        except (OSError, ValueError):
            self.report_errors.append("primary-result-write")
        finally:
            temporary.unlink(missing_ok=True)

    def value(self) -> TestData:
        records = self.first + self.last
        failures = sum(self.counts[name] for name in ("failure", "error", "unexpected"))
        return {
            "schema": 1,
            "duration_seconds": 0.0,
            "test_returncode": 0 if self.wasSuccessful() else 1,
            "tests_run": self.testsRun,
            "tests_discovered": self.discovered,
            "counts": self.counts,
            "records": records,
            "omitted_records": failures - len(records),
            "omitted_trace_bytes": self.trace_omitted,
            "report_errors": self.report_errors,
        }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--start-directory", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--primary", type=Path, required=True)
    args = parser.parse_args()
    # Never duplicate or restore raw output handles in this process: tests,
    # callbacks, atexit handlers and inherited writers only see captured pipes.
    shield = Shield(args.root, os.environ)
    result = Result(args.root, shield, args.primary)
    started = time.monotonic()
    interrupted = False
    try:
        suite = unittest.TestLoader().discover(str(args.start_directory), pattern="test*.py")
        result.discovered = suite.countTestCases()
        suite.run(result)
    except KeyboardInterrupt:
        interrupted = True
        result.report_errors.append("worker-interrupted")
    except Exception as error:
        result.report_errors.append(
            "discovery-or-interruption:" + shield.line(type(error).__name__, 128)
        )
    value = result.value()
    value["duration_seconds"] = time.monotonic() - started
    result.write_primary(value)
    try:
        write_json(args.result, value, RESULT_BYTES - PRIMARY_BYTES)
    except (OSError, ValueError):
        sys.stderr.write("test reporting: final result write failed\n")
        return 2
    sys.stderr.write(f"Ran {result.testsRun} tests\n")
    if interrupted:
        return 130
    return 2 if result.report_errors else int(value["test_returncode"])


if __name__ == "__main__":
    raise SystemExit(main())
