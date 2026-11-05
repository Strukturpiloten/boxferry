"""One safe complete-suite entrypoint; existing runner owns the finite collector/worker group."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import selectors
import signal
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NoReturn, cast

from boxferry_ci.runner import RunCancelled, run
from boxferry_ci.test_reporting_data import (
    CHUNK,
    PRIMARY_BYTES,
    RECORDS,
    RESULT_BYTES,
    SUMMARY_BYTES,
    TRACE_BYTES,
    Bounds,
    Logs,
    Shield,
    TestData,
    encoded,
    read_json,
)


def result_value(value: object) -> TestData:
    if not isinstance(value, dict):
        raise ValueError("result handoff must be an object")
    required = {
        "schema",
        "test_returncode",
        "tests_run",
        "tests_discovered",
        "counts",
        "records",
        "omitted_records",
        "omitted_trace_bytes",
        "report_errors",
        "duration_seconds",
    }
    if set(value) != required or type(value["schema"]) is not int or value["schema"] != 1:
        raise ValueError("unsupported result handoff")
    for key in ("tests_run", "tests_discovered", "omitted_records", "omitted_trace_bytes"):
        if type(value[key]) is not int or value[key] < 0:
            raise ValueError("invalid result counters")
    counts = value["counts"]
    if (
        not isinstance(counts, dict)
        or set(counts) != {"success", "failure", "error", "skipped", "expected", "unexpected"}
        or any(type(count) is not int or count < 0 for count in counts.values())
    ):
        raise ValueError("invalid outcome counters")
    duration = value["duration_seconds"]
    if type(duration) not in (int, float) or not math.isfinite(duration) or duration < 0:
        raise ValueError("invalid result duration")
    code = value["test_returncode"]
    expected = int(any(counts[name] for name in ("failure", "error", "unexpected")))
    if type(code) is not int or code != expected:
        raise ValueError("test status contradicts outcome counters")
    records = value["records"]
    if not isinstance(records, list) or len(records) > RECORDS:
        raise ValueError("failure record bound exceeded")
    for record in records:
        if not isinstance(record, dict) or set(record) != {
            "test_id",
            "exception",
            "cause",
            "location",
            "traceback",
            "omitted_trace_bytes",
        }:
            raise ValueError("invalid failure record")
        for key, bound in (
            ("test_id", 4096),
            ("exception", 512),
            ("cause", 4096),
            ("location", 1024),
            ("traceback", TRACE_BYTES + 256),
        ):
            if not isinstance(record[key], str) or len(record[key].encode()) > bound:
                raise ValueError("invalid failure text bound")
        omitted = record["omitted_trace_bytes"]
        if type(omitted) is not int or omitted < 0:
            raise ValueError("invalid trace omission counter")
    if len(records) + value["omitted_records"] != sum(
        counts[name] for name in ("failure", "error", "unexpected")
    ):
        raise ValueError("failure record accounting mismatch")
    errors = value["report_errors"]
    if (
        not isinstance(errors, list)
        or len(errors) > 32
        or any(not isinstance(error, str) or len(error) > 256 for error in errors)
    ):
        raise ValueError("invalid reporting outcomes")
    return cast(TestData, value)


def recover(result: Path, primary: Path) -> tuple[TestData | None, list[str]]:
    failures: list[str] = []
    try:
        return result_value(read_json(result, RESULT_BYTES - PRIMARY_BYTES)), failures
    except (OSError, ValueError, TypeError):
        failures.append("result-handoff-missing-malformed-or-oversized")
    try:
        return result_value(read_json(primary, PRIMARY_BYTES)), failures
    except (OSError, ValueError, TypeError):
        return None, failures


def collect(args: argparse.Namespace) -> int:
    saved = os.dup(1)
    os.set_inheritable(saved, False)
    # No test ever executes in this collector. Even collector diagnostics cannot
    # accidentally reach the runner: only a sanitized bounded envelope uses saved.
    sink = os.open(os.devnull, os.O_WRONLY)
    os.dup2(sink, 1)
    os.dup2(sink, 2)
    os.close(sink)
    errors: list[str] = []
    process: subprocess.Popen[bytes] | None = None
    payload: dict[str, object]
    try:
        shield = Shield(args.repository_root, os.environ)
        bounds = Bounds(args.original_limit, args.head_bytes, args.tail_bytes)
        bounds.validate()
        logs = Logs(shield, bounds)
        command = [
            sys.executable,
            "-m",
            "boxferry_ci.test_reporting_worker",
            "--root",
            str(args.repository_root),
            "--start-directory",
            str(args.start_directory),
            "--result",
            str(args.result),
            "--primary",
            str(args.primary),
        ]
        # No session/group escape: both processes stay under runner.run's group.
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert process.stdout is not None and process.stderr is not None
        with selectors.DefaultSelector() as selector:
            for stream, name in ((process.stdout, "stdout"), (process.stderr, "stderr")):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ, name)
            while selector.get_map():
                for key, _events in selector.select(0.025):
                    try:
                        chunk = os.read(key.fd, CHUNK)
                    except BlockingIOError:
                        continue
                    if chunk:
                        logs.add(str(key.data), chunk)
                    else:
                        selector.unregister(key.fileobj)
            # EOF is the success condition, not just worker exit or closed local
            # handles. Late inherited writers remain under the outer deadline.
        returncode = process.wait()
        logs.finish()
        if logs.exceeded:
            errors.append("hard-original-output-budget-exceeded")
        value, handoff_errors = recover(args.result, args.primary)
        errors.extend(handoff_errors)
        if value is not None:
            errors.extend(value["report_errors"])
            if returncode != value["test_returncode"]:
                errors.append("worker-exit-does-not-match-primary-test-status")
            if value["tests_discovered"] == 0:
                errors.append("empty-or-incomplete-discovery")
        payload = {
            "schema": 1,
            "result": value,
            "worker_returncode": returncode,
            "report_errors": errors,
            **logs.report(),
        }
        # Transform every final boundary again, including JSON escaping. The
        # emitted transport is finite and never contains raw capture buffers.
        wire = shield.text(
            encoded(payload, 1_900_000, ascii_only=False).decode(), 1_950_000
        ).encode()
        if len(wire) > 1_950_000:
            raise ValueError("collector transport budget exceeded")
        view = memoryview(wire)
        while view:
            written = os.write(saved, view)
            view = view[written:]
        if returncode in (130, -signal.SIGINT):
            return 130
        if returncode in (143, -signal.SIGTERM):
            return 143
        return 2 if errors else (returncode if returncode in (0, 1) else 2)
    except (Exception, KeyboardInterrupt):
        # Fixed-only failure output. Parent can still recover the separately
        # retained primary failure; never replay an untrusted exception/buffer.
        try:
            os.write(saved, b'{"schema":1,"report_errors":["collector-capture-or-render-failure"]}')
        except OSError:
            pass
        return 2
    finally:
        if process is not None:
            if process.poll() is None:
                process.terminate()
                process.wait()
            if process.stdout is not None:
                process.stdout.close()
            if process.stderr is not None:
                process.stderr.close()
        os.close(saved)


def evidence(environment: dict[str, str]) -> tuple[str, str]:
    candidate = environment.get("GITHUB_SHA", "unavailable")
    if re.fullmatch(r"[a-f0-9]{40}", candidate) is None:
        candidate = "unavailable"
    server = environment.get("GITHUB_SERVER_URL", "https://github.com")
    repository = environment.get("GITHUB_REPOSITORY", "")
    run_id = environment.get("GITHUB_RUN_ID", "")
    if (
        not re.fullmatch(r"https://[A-Za-z0-9.-]+", server)
        or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository)
        or not run_id.isdigit()
    ):
        return candidate, "unavailable"
    return candidate, f"{server}/{repository}/actions/runs/{run_id}"


def render(
    shield: Shield,
    value: TestData | None,
    errors: list[str],
    lifecycle: str,
    environment: dict[str, str],
    transcript: str,
    omissions: int | None,
    original: int | None,
) -> str:
    candidate, url = evidence(environment)
    lines = [
        "Shared automation unittest reporting",
        f"stage: {lifecycle}",
        f"candidate: {candidate}",
        f"evidence: {url}",
        f"reporting_failures: {', '.join(errors) if errors else 'none'}",
        f"omitted_transcript_bytes: {omissions if omissions is not None else 'unavailable'}",
        f"original_output_bytes: {original if original is not None else 'unavailable'}",
    ]
    if value is None:
        lines.append("primary_test_status: unavailable (reporting failed)")
    else:
        lines.extend(
            (
                f"primary_test_returncode: {value['test_returncode']}",
                f"tests_run: {value['tests_run']}; counts: {value['counts']}",
                f"tests_discovered: {value['tests_discovered']}",
                f"omitted_failure_records: {value['omitted_records']}",
                f"omitted_trace_bytes: {value['omitted_trace_bytes']}",
            )
        )
        for record in value["records"]:
            lines.extend(
                (
                    f"test: {shield.text(str(record['test_id']), 256)}",
                    f"cause: {shield.text(str(record['exception']), 128)}: "
                    f"{shield.text(str(record['cause']), 256)}",
                    f"location: {shield.text(str(record['location']), 256)}",
                    shield.text(str(record["traceback"]), 1024),
                )
            )
    summary = "\n".join(lines)
    # The summary cap is a hard failure, distinct from harmless log retention.
    if len(summary.encode()) > SUMMARY_BYTES:
        raise ValueError("hard-summary-budget-exceeded")
    merged = shield.text(transcript + "\n[summary]\n" + summary, 1_900_000)
    if len(merged.encode()) > 1_900_000:
        raise ValueError("final-report byte budget exceeded")
    return merged + "\n"


def reserve_summary(shield: Shield, value: TestData | None, environment: dict[str, str]) -> str:
    """Pre-render bounded safe primary metadata independently of the full report."""
    if value is None:
        return "primary_test_status: unavailable\n"
    fixed = f"primary_test_returncode: {value['test_returncode']}\n"
    try:
        records = value["records"]
        selected = records[:1] + (records[-1:] if len(records) > 1 else [])
        candidate, url = evidence(environment)
        lines = [fixed, "stage: report finalization", f"candidate: {candidate}", f"evidence: {url}"]
        for record in selected:
            lines.extend(
                (
                    "test: " + shield.text(record["test_id"], 256),
                    "cause: " + shield.text(record["exception"] + ": " + record["cause"], 512),
                    "location: " + shield.text(record["location"], 256),
                    shield.text(record["traceback"], 2048),
                )
            )
        return shield.text("\n".join(lines), 8192) + "\n"
    except (RunCancelled, KeyboardInterrupt):
        raise
    except Exception:
        # Integers and fixed text only when sanitization itself is unavailable.
        return fixed + "primary_details: unavailable because sanitization failed\n"


def execute(
    root: Path,
    start: Path,
    *,
    timeout: float = 900,
    bounds: Bounds | None = None,
    environment: dict[str, str] | None = None,
) -> tuple[int, str]:
    env = dict(os.environ if environment is None else environment)
    bounds = bounds or Bounds()
    try:
        bounds.validate()
        shield = Shield(root, env)
        if not 0 < timeout <= 900:
            raise ValueError("test deadline exceeds existing runner default")
    except (OSError, ValueError, UnicodeError):
        return 2, "Shared automation reporting failed: unsafe preflight bounds or patterns\n"
    signal_seen = 0
    lifecycle_code: int | None = None

    def cancel(signum: int, _frame: object) -> None:
        nonlocal signal_seen
        signal_seen = signum
        raise RunCancelled("shared unittest reporting cancelled")

    value: TestData | None = None
    reserved = "primary_test_status: unavailable\n"
    old = signal.signal(signal.SIGTERM, cancel)
    try:
        with tempfile.TemporaryDirectory(
            prefix="boxferry-tests-", dir=env.get("RUNNER_TEMP")
        ) as temporary:
            handoff = Path(temporary)
            result_path, primary_path = handoff / "result.json", handoff / "primary.json"
            command = (
                sys.executable,
                "-m",
                "boxferry_ci.test_reporting",
                "--collect",
                "--repository-root",
                str(root),
                "--start-directory",
                str(start),
                "--result",
                str(result_path),
                "--primary",
                str(primary_path),
                "--original-limit",
                str(bounds.original),
                "--head-bytes",
                str(bounds.head),
                "--tail-bytes",
                str(bounds.tail),
            )
            outcome = run(
                command,
                cwd=root,
                stage="shared unittest capture",
                timeout_seconds=timeout,
                environment=env,
            )
            if outcome.status == "cancelled":
                lifecycle_code = 143 if signal_seen == signal.SIGTERM else 130
            elif outcome.diagnostic.cause.startswith("command timed out after "):
                lifecycle_code = 124
            value, errors = recover(result_path, primary_path)
            transcript = ""
            omissions: int | None = None
            original: int | None = None
            lifecycle = outcome.status
            worker_cancel = 0
            try:
                payload = json.loads(outcome.stdout)
                if not isinstance(payload, dict) or payload.get("schema") != 1:
                    raise ValueError("invalid collector envelope")
                reported = payload.get("report_errors")
                if (
                    not isinstance(reported, list)
                    or len(reported) > 32
                    or any(not isinstance(x, str) or len(x) > 256 for x in reported)
                ):
                    raise ValueError("invalid capture outcomes")
                errors.extend(reported)
                if set(payload) != {
                    "schema",
                    "result",
                    "report_errors",
                    "worker_returncode",
                    "original_bytes",
                    "sanitized_bytes",
                    "omitted_bytes",
                    "transcript",
                }:
                    raise ValueError("incomplete collector envelope")
                for key in ("original_bytes", "sanitized_bytes", "omitted_bytes"):
                    if type(payload[key]) is not int or payload[key] < 0:
                        raise ValueError("invalid collector byte counters")
                worker_code = payload["worker_returncode"]
                if type(worker_code) is not int:
                    raise ValueError("invalid worker exit status")
                if worker_code in (130, -signal.SIGINT):
                    worker_cancel = 130
                    lifecycle_code = lifecycle_code or 130
                    lifecycle = "worker-cancelled"
                elif worker_code in (143, -signal.SIGTERM):
                    worker_cancel = 143
                    lifecycle_code = lifecycle_code or 143
                    lifecycle = "worker-cancelled"
                if payload["original_bytes"] > bounds.original:
                    errors.append("hard-original-output-budget-exceeded")
                if "result" in payload and payload["result"] is not None:
                    value = result_value(payload["result"])
                original = payload["original_bytes"]
                transcript = payload.get("transcript", "")
                omissions = payload.get("omitted_bytes", 0)
                if not isinstance(transcript, str) or type(omissions) is not int:
                    raise ValueError("invalid transcript counters")
            except (ValueError, TypeError):
                errors.append("collector-envelope-unavailable-or-invalid")
            if outcome.status not in ("success", "failed"):
                errors.append("capture-" + outcome.status)
            if outcome.returncode not in (0, 1):
                errors.append("capture-exit-failure")
            if value is None or value["tests_discovered"] == 0:
                errors.append("complete-test-result-unavailable")
            if value is not None:
                reserved = f"primary_test_returncode: {value['test_returncode']}\n"
            reserved = reserve_summary(shield, value, env)
            try:
                output = render(
                    shield, value, errors, lifecycle, env, transcript, omissions, original
                )
            except (RunCancelled, KeyboardInterrupt):
                raise
            except Exception as error:
                cause = "parent-report-render-failure"
                if isinstance(error, ValueError) and error.args == (
                    "hard-summary-budget-exceeded",
                ):
                    cause = "hard-summary-budget-exceeded"
                errors.append(cause)
                output = reserved + f"reporting_failure: {cause}\n"
            if outcome.status == "cancelled":
                return (143 if signal_seen == signal.SIGTERM else 130), output
            if outcome.diagnostic.cause.startswith("command timed out after "):
                return 124, output
            if worker_cancel:
                return worker_cancel, output
            if errors:
                return 2, output
            assert value is not None
            return value["test_returncode"], output
    except (RunCancelled, KeyboardInterrupt):
        return (143 if signal_seen == signal.SIGTERM else 130), (
            reserved + "reporting_failure: interrupted report finalization\n"
        )
    except Exception:
        return (
            lifecycle_code or 2,
            reserved + "reporting_failure: setup/capture/handoff/render failure\n",
        )
    finally:
        signal.signal(signal.SIGTERM, old)


class Arguments(argparse.ArgumentParser):
    def error(self, message: str) -> NoReturn:
        # argparse normally echoes untrusted arguments. Only fixed setup data
        # may be emitted before the capture/sanitization boundary exists.
        raise ValueError("invalid reporting arguments")


def main() -> int:
    parser = Arguments(description=__doc__)
    parser.add_argument("--repository-root", type=Path, default=Path.cwd())
    parser.add_argument("--start-directory", type=Path, required=True)
    parser.add_argument("--collect", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--result", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--primary", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--original-limit", type=int, default=2_000_000, help=argparse.SUPPRESS)
    parser.add_argument("--head-bytes", type=int, default=64 * 1024, help=argparse.SUPPRESS)
    parser.add_argument("--tail-bytes", type=int, default=384 * 1024, help=argparse.SUPPRESS)
    try:
        args = parser.parse_args()
        args.repository_root = args.repository_root.resolve()
        args.start_directory = args.start_directory.resolve()
        if args.collect:
            return collect(args)
        code, output = execute(args.repository_root, args.start_directory)
    except (OSError, ValueError):
        code, output = 2, "Shared automation reporting failed: invalid setup arguments\n"
    try:
        data = memoryview(output.encode("utf-8"))
        while data:
            count = os.write(1, data[:CHUNK])
            data = data[count:]
    except (OSError, UnicodeError):
        return 2
    return code


if __name__ == "__main__":
    raise SystemExit(main())
