"""Independent unknown-canary checks at opaque observation publication boundaries.

Only harmless nonroot fixture children run. The PROCESSES invocation keeps its real
source-owned program/digest at the runner boundary; a Popen test double substitutes
our harmless child instead of observing any host processes or invoking a runtime.
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import traceback
import unittest
from collections.abc import Callable, Iterator, Mapping
from dataclasses import asdict, is_dataclass
from pathlib import Path
from typing import Any, cast
from unittest.mock import Mock, patch

from boxferry_ci import devcontainer_lifetime as lifetime
from boxferry_ci import devcontainer_local as local
from boxferry_ci import devcontainer_validation as validation
from boxferry_ci import runner

# Deliberately not environment secrets, sensitive options, IDs, or held metadata.
CANARIES = {
    name: "opaque_unregistered_" + name + "_7e63a92f"
    for name in ("stdout", "stderr", "program", "request", "parser", "hook", "cleanup", "spawn")
}
MODES = tuple(runner.OpaqueObservation)


class UnprintablePrivateError(Exception):
    private_payload: str

    def __str__(self) -> str:
        raise AssertionError("private exception must not be formatted")

    def __repr__(self) -> str:
        raise AssertionError("private exception must not be represented")


class CancellingEnvironment(Mapping[str, str]):
    def __init__(self, failure: BaseException) -> None:
        self.failure = failure

    def __iter__(self) -> Iterator[str]:
        raise self.failure

    def __len__(self) -> int:
        return 1

    def __getitem__(self, key: str) -> str:
        raise KeyError(key)


@unittest.skipIf(os.getuid() == 0, "fixture processes must remain nonroot")
class OpaqueObservationPrivacy(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="bf-opaque-adversarial-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        hidden = runner.protected_values(os.environ)
        for marker in CANARIES.values():
            self.assertFalse(any(marker in secret for secret in hidden))
            self.assertFalse(any(marker in value for value in os.environ.values()))

    def assert_private_absent(self, *values: object) -> None:
        """Inspect fields and chains too; suppressing exception display is insufficient."""
        seen: set[int] = set()

        def scan(value: object) -> None:
            if id(value) in seen:
                return
            seen.add(id(value))
            if isinstance(value, bytes):
                display = repr(value)
            else:
                display = str(value)
            for name, marker in CANARIES.items():
                if marker in display or marker in repr(value):
                    self.fail("Unregistered " + name + " canary reached a publication surface")
            if isinstance(value, BaseException):
                scan(value.args)
                scan(vars(value))
                scan(value.__cause__)
                scan(value.__context__)
                scan("".join(traceback.format_exception(value)))
            elif isinstance(value, dict):
                for key, item in value.items():
                    scan(key)
                    scan(item)
            elif isinstance(value, (tuple, list)):
                for item in value:
                    scan(item)
            elif is_dataclass(value) and not isinstance(value, type):
                scan(asdict(value))
                scan(vars(value))
                if isinstance(value, runner.RunResult):
                    scan(value.to_dict())
                    self.assertEqual(
                        (value.stdout, value.stderr, value.stdout_bytes), ("", "", b"")
                    )

        for value in values:
            scan(value)

    def fixture_program(self, ending: str) -> str:
        return (
            "import os,sys,time; "
            f"# {CANARIES['program']}\n"
            f"os.write(1, {('{"unknown":"' + CANARIES['stdout']).encode()!r}); "
            f"os.write(2, {CANARIES['stderr'].encode()!r}); " + ending
        )

    def invoke(
        self,
        mode: runner.OpaqueObservation,
        program: str,
        *,
        consumer: Callable[[bytes], None] | None = None,
        abort: Callable[[runner.AbortContext], None] | None = None,
        monitor: Callable[[], None] | None = None,
        transform: Callable[[subprocess.Popen[bytes]], None] | None = None,
    ) -> runner.OpaqueRunResult:
        actual_popen = subprocess.Popen
        argv = (sys.executable, "-I", "-S", "-c", program, CANARIES["request"])
        if mode is runner.OpaqueObservation.PROCESSES:
            argv = (
                "/usr/bin/python3",
                "-I",
                "-S",
                "-c",
                validation.LIVE_PROCESS_OBSERVER_PROGRAM,
                CANARIES["request"],
            )

        def harmless_child(_argv: tuple[str, ...], **kwargs: object) -> subprocess.Popen[bytes]:
            self.assertEqual(_argv, argv)
            self.assertTrue(kwargs["start_new_session"])
            process = cast(Any, actual_popen)((sys.executable, "-I", "-S", "-c", program), **kwargs)
            if transform is not None:
                transform(process)
            return process

        with (
            patch.object(runner.subprocess, "Popen", side_effect=harmless_child),
            patch.object(runner, "_redacted_argv", side_effect=AssertionError("raw argv renderer")),
        ):
            result = runner.run(
                argv,
                cwd=self.root,
                stage=mode.stage,
                timeout_seconds=mode.timeout,
                max_output_bytes=mode.output_cap,
                opaque_observation=mode,
                on_private_stdout=consumer or Mock(),
                on_abort=abort,
                monitor=monitor,
                grace_seconds=0.05,
            )
            assert isinstance(result, runner.OpaqueRunResult)
            return result

    def handoff(self) -> tuple[dict[str, Any], int]:
        result = self.root / ("handoff-" + str(len(list(self.root.iterdir()))))
        result.touch(mode=0o600)
        facts = result.stat()
        read_fd, write_fd = os.pipe()
        self.addCleanup(os.close, read_fd)
        self.addCleanup(os.close, write_fd)
        return {
            "result": str(result),
            "result_identity": [facts.st_dev, facts.st_ino],
            "abort_fd": write_fd,
        }, read_fd

    def persisted_failure(
        self,
        mode: runner.OpaqueObservation,
        ending: str,
        *,
        monitor: Callable[[], None] | None = None,
    ) -> tuple[runner.OpaqueRunResult, runner.OpaqueAbortContext, dict[str, object]]:
        context, read_fd = self.handoff()
        failures: list[runner.AbortContext] = []
        consumer = Mock()

        def abort(failure: runner.AbortContext) -> None:
            failures.append(failure)
            lifetime.notify_abort(context, failure, {"schema": 1, "outcomes": []})

        result = self.invoke(
            mode, self.fixture_program(ending), consumer=consumer, abort=abort, monitor=monitor
        )
        consumer.assert_not_called()
        self.assertEqual(len(failures), 1)
        self.assertIsInstance(failures[0], runner.OpaqueAbortContext)
        self.assertEqual(os.read(read_fd, 1), b"A")
        preview = json.loads(Path(context["result"]).read_text())
        self.assertEqual(preview["status"], "failure")
        self.assertEqual(preview["command_failure_stage"], mode.stage)
        self.assert_private_absent(result, failures, preview)
        return result, cast(runner.OpaqueAbortContext, failures[0]), preview

    def publish(self, report: dict[str, object]) -> None:
        """Exercise both actual CLI writers and their normal/overflow serializers."""
        for compact in (False, True):
            selected = dict(report)
            if compact:
                selected["unrelated_safe_padding"] = "x" * (validation.MAX_REPORT + 1)
            encoded = validation.serialize_report(selected)
            expected_status = "failure" if compact else selected["status"]
            self.assertLessEqual(len(encoded.encode()), validation.MAX_REPORT)
            decoded = json.loads(encoded)
            self.assertEqual(decoded["status"], expected_status)
            if compact:
                for key, value in selected.items():
                    if key.startswith("opaque_failure_") and value is not None:
                        self.assertEqual(decoded.get(key), value)
                for key in ("command_failure_cause", "cleanup_cause", "abort_hook_failure"):
                    if key in selected:
                        self.assertEqual(decoded.get(key), selected[key])
            self.assert_private_absent(selected, encoded)
            for frontend in ("validation", "local"):
                with self.subTest(compact=compact, frontend=frontend):
                    output = self.root / (frontend + "-" + str(compact) + ".json")
                    stdout, stderr = io.StringIO(), io.StringIO()
                    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                        if frontend == "validation":
                            with patch.object(validation, "qualify", return_value=selected):
                                status = validation.main(
                                    [
                                        "qualify",
                                        "--repository",
                                        "boxferry",
                                        "--candidate-sha",
                                        "a" * 40,
                                        "--output",
                                        str(output),
                                        "--source",
                                        str(self.root),
                                        "--policy",
                                        str(self.root),
                                        "--automation",
                                        str(self.root),
                                        "--policy-sha",
                                        "b" * 40,
                                        "--automation-sha",
                                        "c" * 40,
                                        "--engine",
                                        "podman",
                                        "--sources-json",
                                        "{}",
                                    ]
                                )
                        else:
                            with (
                                patch.object(local, "_owned_output"),
                                patch.object(local, "execute", return_value=selected),
                            ):
                                status = local.main(
                                    [
                                        "--repository-root",
                                        str(self.root),
                                        "--config",
                                        "unused.json",
                                        "--output",
                                        str(output),
                                    ]
                                )
                    self.assertEqual(status, 0 if expected_status == "success" else 1)
                    written = output.read_text()
                    self.assertEqual(json.loads(written)["status"], expected_status)
                    self.assert_private_absent(written, stdout.getvalue(), stderr.getvalue())

    def test_all_modes_unknown_partial_nonzero_across_persisted_and_rendered_sinks(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode):
                result, failure, preview = self.persisted_failure(mode, "sys.exit(19)")
                self.assertEqual((result.status, result.returncode), ("failed", 19))
                self.assertEqual(
                    result.metrics.stdout_prefix_bytes,
                    len(('{"unknown":"' + CANARIES["stdout"]).encode()),
                )
                self.assertEqual(
                    result.metrics.stderr_prefix_sha256,
                    hashlib.sha256(CANARIES["stderr"].encode()).hexdigest(),
                )
                self.assertEqual(preview["command_failure_cause"], failure.cause)
                self.publish(preview)

    def test_all_modes_partial_timeout_and_aggregate_overflow(self) -> None:
        for mode in MODES:
            for ending, reason in (
                ("time.sleep(7)", runner.OpaqueReason.TIMEOUT),
                (f"os.write(2, b'x' * {mode.output_cap + 1})", runner.OpaqueReason.OUTPUT_LIMIT),
            ):
                with self.subTest(mode=mode, reason=reason):
                    result, _failure, preview = self.persisted_failure(mode, ending)
                    self.assertEqual(result.status, "infrastructure")
                    self.assertEqual(result.metrics.reason, reason)
                    self.publish(preview)

    def test_all_modes_unknown_cancellation_and_hook_error(self) -> None:
        for mode in MODES:
            for exception in (runner.RunCancelled, KeyboardInterrupt):
                with self.subTest(mode=mode, exception=exception):
                    failures: list[runner.AbortContext] = []
                    consumer = Mock()
                    # Cancel only once output has traversed the real drain loop.
                    count = 0

                    def monitor(exception: type[BaseException] = exception) -> None:
                        nonlocal count
                        count += 1
                        if count >= 4:
                            raise exception(CANARIES["hook"])

                    def abort(
                        failure: runner.AbortContext,
                        failures: list[runner.AbortContext] = failures,
                    ) -> None:
                        failures.append(failure)
                        raise ValueError(CANARIES["hook"])

                    result = self.invoke(
                        mode,
                        self.fixture_program("time.sleep(7)"),
                        consumer=consumer,
                        abort=abort,
                        monitor=monitor,
                    )
                    self.assertEqual(result.status, "cancelled")
                    self.assertEqual(result.metrics.reason, runner.OpaqueReason.CANCELLED)
                    self.assertTrue(result.abort_hook_failure)
                    self.assertEqual(len(failures), 1)
                    consumer.assert_not_called()
                    self.assert_private_absent(result, failures)
                    self.publish({"status": "failure", "outcomes": [result.to_dict()]})

    def cancellation_at_late_boundary(
        self,
        mode: runner.OpaqueObservation,
        exception: type[BaseException],
        phase: str,
        *,
        original_nonzero: bool = False,
        original_timeout: bool = False,
        original_parser: bool = False,
    ) -> None:
        prior_failure = original_nonzero or original_timeout or original_parser
        events: list[str] = []
        failures: list[runner.AbortContext] = []
        children: list[subprocess.Popen[bytes]] = []
        parsed = injected = False
        ready_to_parse = False
        real_cleanup = runner._cleanup
        real_selector = runner.selectors.DefaultSelector
        real_clock = runner.time.monotonic
        fixture_started = real_clock()

        def cancel() -> None:
            nonlocal injected
            if injected:
                return
            injected = True
            events.append("cancel")
            try:
                raise ValueError(CANARIES["hook"])
            except ValueError as original:
                raise exception(CANARIES["cleanup"]) from original

        def consume(content: bytes) -> None:
            nonlocal parsed
            if original_parser:
                validation._live_json(content)
            self.assertEqual(json.loads(content)["schema"], 1)
            parsed = True
            events.append("parsed")
            if phase == "consumer":
                cancel()

        def abort(failure: runner.AbortContext) -> None:
            events.append("abort")
            failures.append(failure)

        def cleanup(process: subprocess.Popen[bytes], grace: float) -> None:
            events.append("cleanup")
            real_cleanup(process, grace)
            if phase == "cleanup" and (parsed or prior_failure):
                cancel()

        def transform(process: subprocess.Popen[bytes]) -> None:
            children.append(process)
            for name, stream in (("stdout", process.stdout), ("stderr", process.stderr)):
                assert stream is not None
                real_close = stream.close

                def close(
                    name: str = name,
                    real_close: Callable[[], None] = real_close,
                ) -> None:
                    events.append(name + "-close")
                    real_close()
                    if phase == "stream-close" and name == "stdout" and parsed:
                        cancel()

                replacement = patch.object(stream, "close", side_effect=close)
                replacement.start()
                self.addCleanup(replacement.stop)

        def selector() -> runner.selectors.BaseSelector:
            selected = real_selector()
            real_close = selected.close
            real_get_map = selected.get_map

            def close() -> None:
                events.append("selector-close")
                real_close()
                if phase == "selector-close" and parsed:
                    cancel()

            def get_map() -> object:
                nonlocal ready_to_parse
                result = real_get_map()
                if not result:
                    ready_to_parse = True
                return result

            for name, operation in (("close", close), ("get_map", get_map)):
                replacement = patch.object(selected, name, side_effect=operation)
                replacement.start()
                self.addCleanup(replacement.stop)
            return selected

        def clock() -> float:
            if (
                (phase == "parse-clock-before" and ready_to_parse)
                or (phase == "parse-clock-after" and parsed)
                or (phase == "result-clock" and parsed and "selector-close" in events)
            ):
                cancel()
            now = real_clock()
            if original_timeout and children and now - fixture_started >= 0.03:
                return now + mode.timeout + 1
            return now

        content = (
            b'{"schema":1}'
            if not original_parser
            else ('{"unknown":"' + CANARIES["stdout"]).encode()
        )
        ending = (
            "time.sleep(7)" if original_timeout else f"sys.exit({19 if original_nonzero else 0})"
        )
        code = (
            "import os,sys,time; "
            f"# {CANARIES['program']}\n"
            f"os.write(1, {content!r}); "
            f"os.write(2, {CANARIES['stderr'].encode()!r}); " + ending
        )
        outcome: object = None
        with (
            patch.object(runner, "_cleanup", side_effect=cleanup),
            patch.object(runner.selectors, "DefaultSelector", side_effect=selector),
            patch.object(runner.time, "monotonic", side_effect=clock),
        ):
            try:
                outcome = self.invoke(
                    mode, code, consumer=consume, abort=abort, transform=transform
                )
            except BaseException as error:
                outcome = error
        self.assertTrue(injected, "fixture must reach its intended cancellation boundary")
        self.assert_private_absent(outcome, failures)
        expected = (
            runner.OpaqueReason.EXIT_NONZERO if original_nonzero else runner.OpaqueReason.CANCELLED
        )
        if original_nonzero and mode is runner.OpaqueObservation.PROCESSES:
            expected = runner.OpaqueReason.OBSERVER_FAILED
        expected_status = "failed" if original_nonzero else "cancelled"
        if original_timeout:
            expected, expected_status = runner.OpaqueReason.TIMEOUT, "infrastructure"
        elif original_parser:
            expected, expected_status = runner.OpaqueReason.INVALID_JSON, "infrastructure"
        if isinstance(outcome, runner.OpaqueRunResult):
            self.assertEqual(outcome.status, expected_status)
            metrics = outcome.metrics
        elif isinstance(outcome, runner.OpaqueRunError):
            self.assertEqual(outcome.reason, expected)
            metrics = outcome.metrics
            self.assertIsNotNone(metrics)
        else:
            self.fail("runner must expose only fixed typed failure metadata")
        assert metrics is not None
        self.assertEqual(metrics.status, expected_status)
        self.assertEqual(metrics.reason, expected)
        if prior_failure:
            self.assertTrue(metrics.cleanup_failed)
        self.assertEqual(len(failures), 1, "the first abort notification must remain immutable")
        self.assertEqual(failures[0].cause, expected.value)
        self.assertEqual(failures[0].status, expected_status)
        if not prior_failure:
            known = events.index("cancel")
            abort_index = events.index("abort")
            self.assertGreater(abort_index, known)
            for index, event in enumerate(events):
                if index > known and event in {
                    "cleanup",
                    "stdout-close",
                    "stderr-close",
                    "selector-close",
                }:
                    self.assertLess(abort_index, index, "notify as soon as cancellation is known")
        self.assertTrue(children)
        for child in children:
            self.assertIsNotNone(child.poll())
            assert child.stdout is not None and child.stderr is not None
            self.assertTrue(child.stdout.closed and child.stderr.closed)
        report: dict[str, object] = {
            "status": "failure",
            "cause": expected.value,
            "opaque_observation": asdict(metrics),
        }
        self.publish(report)

    def test_all_modes_real_cancellation_at_parser_cleanup_and_close_boundaries(self) -> None:
        for mode in MODES:
            for exception in (runner.RunCancelled, KeyboardInterrupt):
                for phase in (
                    "consumer",
                    "cleanup",
                    "stream-close",
                    "selector-close",
                    "parse-clock-before",
                    "parse-clock-after",
                    "result-clock",
                ):
                    with self.subTest(mode=mode, exception=exception, phase=phase):
                        self.cancellation_at_late_boundary(mode, exception, phase)

    def test_late_cleanup_cancellation_preserves_original_first_abort(self) -> None:
        for mode in MODES:
            for exception in (runner.RunCancelled, KeyboardInterrupt):
                for primary in ("exit", "timeout", "parser"):
                    with self.subTest(mode=mode, exception=exception, primary=primary):
                        self.cancellation_at_late_boundary(
                            mode,
                            exception,
                            "cleanup",
                            original_nonzero=primary == "exit",
                            original_timeout=primary == "timeout",
                            original_parser=primary == "parser",
                        )

    def assert_fixed_cancellation(self, outcome: object) -> runner.OpaqueMetrics | None:
        self.assert_private_absent(outcome)
        metrics: runner.OpaqueMetrics | None = None
        if isinstance(outcome, runner.OpaqueRunResult):
            self.assertEqual(outcome.status, "cancelled")
            metrics = outcome.metrics
        elif isinstance(outcome, runner.OpaqueRunError):
            self.assertEqual(outcome.reason, runner.OpaqueReason.CANCELLED)
            metrics = outcome.metrics
        elif isinstance(outcome, (runner.RunCancelled, KeyboardInterrupt)):
            self.assertEqual(outcome.args, (runner.OpaqueReason.CANCELLED.value,))
            self.assertIsNone(outcome.__cause__)
            self.assertIsNone(outcome.__context__)
        else:
            self.fail("first cancellation must remain a fixed typed cancellation")
        if metrics is not None:
            self.assertEqual(metrics.reason, runner.OpaqueReason.CANCELLED)
            self.assertEqual(metrics.status, "cancelled")
        return metrics

    def test_all_modes_prelaunch_cancellation_is_safe_and_never_spawns(self) -> None:
        for mode in MODES:
            for exception in (runner.RunCancelled, KeyboardInterrupt):
                phases = [
                    "setup-json",
                    "setup-hash",
                    "stdout-prefix-hash",
                    "stderr-prefix-hash",
                    "environment",
                    "selector",
                    "initial-clock",
                ]
                if mode is runner.OpaqueObservation.PROCESSES:
                    phases.append("observer-import")
                for phase in phases:
                    with self.subTest(mode=mode, exception=exception, phase=phase):
                        argv = (sys.executable, "-c", "pass", CANARIES["request"])
                        if mode is runner.OpaqueObservation.PROCESSES:
                            argv = (
                                "/usr/bin/python3",
                                "-I",
                                "-S",
                                "-c",
                                validation.LIVE_PROCESS_OBSERVER_PROGRAM,
                                CANARIES["request"],
                            )
                        error = exception(CANARIES["hook"])
                        error.__cause__ = ValueError(CANARIES["request"])
                        environment: Mapping[str, str] | None = None
                        if phase == "environment":
                            environment = CancellingEnvironment(error)
                        outcome: object = None
                        notifications: list[runner.AbortContext] = []
                        with contextlib.ExitStack() as stack:
                            launched = stack.enter_context(
                                patch.object(
                                    runner.subprocess,
                                    "Popen",
                                    side_effect=AssertionError("prelaunch"),
                                )
                            )
                            target: object | None = None
                            name = ""
                            if phase == "setup-json":
                                target, name = runner.json, "dumps"
                            elif phase in (
                                "setup-hash",
                                "stdout-prefix-hash",
                                "stderr-prefix-hash",
                            ):
                                target, name = runner.hashlib, "sha256"
                            elif phase == "selector":
                                target, name = runner.selectors, "DefaultSelector"
                            elif phase == "initial-clock":
                                target, name = runner.time, "monotonic"
                            if target is not None:
                                operation = getattr(target, name)
                                injected = False
                                empty_factories = 0

                                def cancel_once(
                                    *args: object,
                                    operation: object = operation,
                                    error: BaseException = error,
                                    phase: str = phase,
                                    **kwargs: object,
                                ) -> object:
                                    nonlocal injected, empty_factories
                                    selected = True
                                    if phase in ("stdout-prefix-hash", "stderr-prefix-hash"):
                                        selected = False
                                        if (not args or args == (b"",)) and not kwargs:
                                            empty_factories += 1
                                            selected = empty_factories == (
                                                1 if phase == "stdout-prefix-hash" else 2
                                            )
                                    if selected and not injected:
                                        injected = True
                                        raise error
                                    return cast(Any, operation)(*args, **kwargs)

                                stack.enter_context(
                                    patch.object(target, name, side_effect=cancel_once)
                                )
                            elif phase == "observer-import":
                                import builtins

                                original_import = builtins.__import__

                                def import_observer(
                                    name: str,
                                    *args: object,
                                    error: BaseException = error,
                                    original_import: object = original_import,
                                    **kwargs: object,
                                ) -> object:
                                    if name == "devcontainer_validation":
                                        raise error
                                    return cast(Any, original_import)(name, *args, **kwargs)

                                stack.enter_context(
                                    patch.object(
                                        builtins, "__import__", side_effect=import_observer
                                    )
                                )
                            try:
                                outcome = runner.run(
                                    argv,
                                    cwd=self.root,
                                    stage=mode.stage,
                                    timeout_seconds=mode.timeout,
                                    max_output_bytes=mode.output_cap,
                                    opaque_observation=mode,
                                    on_private_stdout=Mock(),
                                    on_abort=notifications.append,
                                    environment=environment,
                                )
                            except BaseException as observed:
                                outcome = observed
                        launched.assert_not_called()
                        self.assert_fixed_cancellation(outcome)
                        self.assert_private_absent(notifications)
                        self.assertTrue(all(f.status == "cancelled" for f in notifications))

    def test_facade_preflight_and_postparse_cancellation_have_cancelled_abort_metadata(
        self,
    ) -> None:
        held = validation.LiveHeldIdentity(
            "a" * 64,
            "sha256:" + "b" * 64,
            "c" * 32,
            1000,
            "d" * 64,
            "podman version 5.8.1",
        )
        request = validation.LiveProcessRequest(
            101, 102, 1000, "pid:[1001]", "mnt:[1002]", "pid:[1003]", "mnt:[1004]"
        )
        for mode in MODES:
            for exception in (runner.RunCancelled, KeyboardInterrupt):
                for phase in ("budget", "builder", "postparse-budget"):
                    with self.subTest(mode=mode, exception=exception, phase=phase):
                        private = self.root / (mode.value + "-" + exception.__name__ + "-" + phase)
                        private.mkdir(mode=0o700)
                        commands = validation.Commands(private)
                        commands.lifetime_context = {"private": str(private)}
                        notifications: list[runner.AbortContext] = []
                        error = exception(CANARIES["hook"])
                        error.__cause__ = ValueError(CANARIES["parser"])
                        budget = Mock()
                        commands.budget = cast(validation.Budget, budget)
                        if phase == "budget":
                            budget.check.side_effect = error
                        elif phase == "postparse-budget":
                            budget.check.side_effect = [None, error]

                        def notify(
                            context: dict[str, Any],
                            observed: runner.AbortContext,
                            report: object,
                            notifications: list[runner.AbortContext] = notifications,
                        ) -> None:
                            context["abort_notified"] = True
                            notifications.append(observed)
                            self.assert_private_absent(observed, report)

                        def completed_runner(
                            _argv: tuple[str, ...],
                            mode: runner.OpaqueObservation = mode,
                            **kwargs: object,
                        ) -> runner.OpaqueRunResult:
                            cast(Callable[[bytes], None], kwargs["on_private_stdout"])(b"{}")
                            empty_hash = hashlib.sha256(b"").hexdigest()
                            metrics = runner.OpaqueMetrics(
                                mode,
                                runner.OpaqueReason.COMPLETED,
                                "success",
                                "e" * 64,
                                validation.LIVE_PROCESS_OBSERVER_SHA256
                                if mode is runner.OpaqueObservation.PROCESSES
                                else None,
                                0,
                                0,
                                empty_hash,
                                empty_hash,
                            )
                            return runner.OpaqueRunResult(
                                "success",
                                0,
                                "",
                                "",
                                0.01,
                                runner.Diagnostic(
                                    mode.stage, "completed", "held", "safe", "review"
                                ),
                                ("opaque-observation", mode.value, "e" * 64),
                                metrics=metrics,
                            )

                        with contextlib.ExitStack() as stack:
                            stack.enter_context(
                                patch.object(lifetime, "notify_abort", side_effect=notify)
                            )
                            launched = stack.enter_context(
                                patch.object(
                                    validation,
                                    "run",
                                    side_effect=completed_runner
                                    if phase == "postparse-budget"
                                    else AssertionError("prelaunch"),
                                )
                            )
                            stack.enter_context(
                                patch.object(
                                    validation,
                                    "LIVE_PID_SCHEMA",
                                    validation.LivePidSchema.PODMAN_STATE_FIELDS,
                                )
                            )
                            stack.enter_context(
                                patch.object(validation, "_rootful_bridge", return_value=((), ()))
                            )
                            if phase == "builder":
                                target = "_rootful_bridge" if mode is MODES[2] else "_live_held"
                                stack.enter_context(
                                    patch.object(validation, target, side_effect=error)
                                )
                            stack.enter_context(
                                patch.object(
                                    validation,
                                    "_parse_live_inspect",
                                    return_value=validation.LiveInspection(
                                        held, True, "running", 101, 102, "integer", "integer", ""
                                    ),
                                )
                            )
                            stack.enter_context(
                                patch.object(
                                    validation,
                                    "_parse_live_processes",
                                    return_value=validation.LiveProcessSample(
                                        103, request.pid_namespace, request.mount_namespace, ()
                                    ),
                                )
                            )
                            outcome: object = None
                            try:
                                if mode is runner.OpaqueObservation.PROCESSES:
                                    cast(Any, commands)._live_processes(request)
                                else:
                                    cast(Any, commands)._live_inspect(held, mode)
                            except BaseException as observed:
                                outcome = observed
                        self.assert_fixed_cancellation(outcome)
                        if phase == "postparse-budget":
                            launched.assert_called_once()
                        else:
                            launched.assert_not_called()
                        self.assertEqual(len(notifications), 1)
                        self.assertEqual(notifications[0].status, "cancelled")
                        self.assertEqual(
                            notifications[0].cause, runner.OpaqueReason.CANCELLED.value
                        )
                        self.assertEqual(commands.outcomes[-1]["status"], "cancelled")
                        self.assertEqual(
                            commands.outcomes[-1]["cause"], runner.OpaqueReason.CANCELLED.value
                        )
                        metadata = commands.outcomes[-1].get("opaque_observation")
                        if metadata is not None:
                            self.assertEqual(
                                cast(dict[str, object], metadata)["status"], "cancelled"
                            )
                            self.assertEqual(
                                cast(dict[str, object], metadata)["reason"],
                                runner.OpaqueReason.CANCELLED,
                            )
                        self.assert_private_absent(notifications, commands.outcomes)

    def test_success_stderr_and_raw_unknown_fields_are_absent_after_private_consumer(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode):
                accepted: list[dict[str, object]] = []

                def consume(content: bytes, accepted: list[dict[str, object]] = accepted) -> None:
                    document = json.loads(content)
                    if document["schema"] != 1:
                        raise runner.OpaqueParseError(runner.OpaqueReason.INVALID_JSON)
                    accepted.append({"schema": 1})

                content = json.dumps({"schema": 1, "unknown": CANARIES["stdout"]}).encode()
                program = (
                    "import os; "
                    f"# {CANARIES['program']}\n"
                    f"os.write(1, {content!r}); "
                    f"os.write(2, {CANARIES['stderr'].encode()!r})"
                )
                result = self.invoke(mode, program, consumer=consume)
                self.assertEqual(result.status, "success")
                self.assertEqual(accepted, [{"schema": 1}])
                self.assertEqual(result.metrics.stderr_prefix_bytes, len(CANARIES["stderr"]))
                self.assert_private_absent(result, accepted)
                self.publish({"status": "success", "outcomes": [result.to_dict()]})

    def test_all_modes_monitor_exceptions_drop_custom_attributes_and_chains(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode):
                failures: list[runner.AbortContext] = []
                consumer = Mock()
                count = 0

                def monitor() -> None:
                    nonlocal count
                    count += 1
                    if count >= 4:
                        try:
                            raise ValueError(CANARIES["hook"])
                        except ValueError as original:
                            error = UnprintablePrivateError(CANARIES["hook"])
                            error.private_payload = CANARIES["stdout"]
                            raise error from original

                with self.assertRaises(runner.OpaqueRunError) as caught:
                    self.invoke(
                        mode,
                        self.fixture_program("time.sleep(7)"),
                        consumer=consumer,
                        monitor=monitor,
                        abort=failures.append,
                    )
                consumer.assert_not_called()
                self.assertEqual(caught.exception.reason, runner.OpaqueReason.MONITOR_FAILED)
                self.assertEqual(len(failures), 1)
                self.assert_private_absent(caught.exception, failures)
                self.publish({"status": "failure", "cause": str(caught.exception)})

    def test_actual_private_json_decoder_never_publishes_malformed_payload(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode):
                context, read_fd = self.handoff()
                failures: list[runner.AbortContext] = []

                def consume(content: bytes) -> None:
                    validation._live_json(content)

                def abort(
                    failure: runner.AbortContext,
                    context: dict[str, Any] = context,
                    failures: list[runner.AbortContext] = failures,
                ) -> None:
                    failures.append(failure)
                    lifetime.notify_abort(context, failure, {"schema": 1, "outcomes": []})

                result = self.invoke(
                    mode, self.fixture_program("sys.exit(0)"), consumer=consume, abort=abort
                )
                self.assertEqual(result.status, "infrastructure")
                self.assertEqual(result.metrics.reason, runner.OpaqueReason.INVALID_JSON)
                self.assertEqual(os.read(read_fd, 1), b"A")
                preview = json.loads(Path(context["result"]).read_text())
                self.assertEqual(preview["status"], "failure")
                self.assert_private_absent(result, failures, preview)
                self.publish(preview)

    def test_parser_unprintable_attrs_and_chains_never_reach_public_surfaces(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode):
                failures: list[runner.AbortContext] = []

                def parser(_content: bytes) -> None:
                    try:
                        raise ValueError(CANARIES["parser"])
                    except ValueError as original:
                        error = UnprintablePrivateError(CANARIES["parser"])
                        error.private_payload = CANARIES["stdout"]
                        raise error from original

                result = self.invoke(
                    mode,
                    self.fixture_program("sys.exit(0)"),
                    consumer=parser,
                    abort=failures.append,
                )
                self.assertEqual(result.status, "infrastructure")
                self.assertEqual(result.metrics.reason, runner.OpaqueReason.PRIVATE_PARSER_FAILED)
                self.assertEqual(len(failures), 1)
                self.assert_private_absent(result, failures)
                self.publish({"status": "failure", "outcomes": [result.to_dict()]})

    def test_spawn_error_filename_and_cleanup_exception_chains_are_sanitized(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode, phase="spawn"):
                consumer = Mock()
                argv = (sys.executable, "-c", "pass", CANARIES["request"])
                if mode is runner.OpaqueObservation.PROCESSES:
                    argv = (
                        "/usr/bin/python3",
                        "-I",
                        "-S",
                        "-c",
                        validation.LIVE_PROCESS_OBSERVER_PROGRAM,
                        CANARIES["request"],
                    )
                with patch.object(
                    runner.subprocess,
                    "Popen",
                    side_effect=OSError(2, CANARIES["spawn"], CANARIES["program"]),
                ):
                    result = runner.run(
                        argv,
                        cwd=self.root,
                        stage=mode.stage,
                        timeout_seconds=mode.timeout,
                        max_output_bytes=mode.output_cap,
                        opaque_observation=mode,
                        on_private_stdout=consumer,
                    )
                assert isinstance(result, runner.OpaqueRunResult)
                self.assertEqual(result.metrics.reason, runner.OpaqueReason.SPAWN_UNAVAILABLE)
                self.assertEqual(result.metrics.errno, 2)
                consumer.assert_not_called()
                self.assert_private_absent(result)
            with self.subTest(mode=mode, phase="cleanup"):
                real_cleanup = runner._cleanup

                def cleanup(
                    process: subprocess.Popen[bytes],
                    grace: float,
                    real_cleanup: Callable[[subprocess.Popen[bytes], float], None] = real_cleanup,
                ) -> None:
                    real_cleanup(process, grace)
                    try:
                        raise ValueError(CANARIES["cleanup"])
                    except ValueError as original:
                        raise OSError(5, CANARIES["cleanup"]) from original

                with patch.object(runner, "_cleanup", side_effect=cleanup):
                    with self.assertRaises(runner.OpaqueRunError) as caught:
                        self.invoke(mode, self.fixture_program("sys.exit(19)"))
                self.assert_private_absent(caught.exception)
                assert caught.exception.metrics is not None
                self.assertTrue(caught.exception.metrics.cleanup_failed)
                self.assert_private_absent(caught.exception.metrics)

    def test_actual_abort_handoff_secondary_errors_keep_fixed_four_key_fallback(self) -> None:
        for mode in MODES:
            for phase in ("open", "serialize", "write"):
                with self.subTest(mode=mode, phase=phase):
                    _result, failure, _preview = self.persisted_failure(mode, "sys.exit(19)")
                    context, read_fd = self.handoff()
                    stderr: list[bytes] = []
                    real_write = os.write

                    def write(
                        fd: int,
                        content: bytes,
                        stderr: list[bytes] = stderr,
                        phase: str = phase,
                        context: dict[str, Any] = context,
                        real_write: Callable[[int, bytes], int] = real_write,
                    ) -> int:
                        if fd == 2:
                            stderr.append(content)
                            return len(content)
                        if phase == "write" and fd != context["abort_fd"]:
                            raise OSError(5, CANARIES["hook"])
                        return real_write(fd, content)

                    with contextlib.ExitStack() as stack:
                        stack.enter_context(patch.object(lifetime.os, "write", side_effect=write))
                        if phase == "open":
                            stack.enter_context(
                                patch.object(
                                    lifetime.os, "open", side_effect=OSError(5, CANARIES["hook"])
                                )
                            )
                        if phase == "serialize":
                            stack.enter_context(
                                patch.object(
                                    validation,
                                    "serialize_report",
                                    side_effect=ValueError(CANARIES["hook"]),
                                )
                            )
                        try:
                            lifetime.notify_abort(context, failure, {"schema": 1})
                        except BaseException as error:
                            self.assert_private_absent(error)
                    self.assertEqual(os.read(read_fd, 1), b"A")
                    self.assertEqual(len(stderr), 1)
                    prefix = b"BOXFERRY_LIFETIME_FAILURE "
                    self.assertTrue(stderr[0].startswith(prefix))
                    fallback = json.loads(stderr[0].removeprefix(prefix))
                    self.assertEqual(
                        set(fallback), {"stage", "status", "cause", "abort_hook_failure"}
                    )
                    self.assertEqual(fallback["cause"], failure.cause)
                    self.assert_private_absent(stderr, fallback)

    def test_abort_byte_exception_is_sanitized_by_runner_after_safe_preview(self) -> None:
        for mode in MODES:
            with self.subTest(mode=mode):
                context, _read_fd = self.handoff()
                real_write = os.write

                def write(
                    fd: int,
                    content: bytes,
                    context: dict[str, Any] = context,
                    real_write: Callable[[int, bytes], int] = real_write,
                ) -> int:
                    if fd == context["abort_fd"]:
                        raise OSError(5, CANARIES["hook"])
                    return real_write(fd, content)

                def abort(
                    failure: runner.AbortContext,
                    context: dict[str, Any] = context,
                    write: Callable[[int, bytes], int] = write,
                ) -> None:
                    with patch.object(lifetime.os, "write", side_effect=write):
                        lifetime.notify_abort(context, failure, {"schema": 1})

                result = self.invoke(mode, self.fixture_program("sys.exit(19)"), abort=abort)
                self.assertEqual(result.status, "failed")
                self.assertTrue(result.abort_hook_failure)
                preview = json.loads(Path(context["result"]).read_text())
                self.assert_private_absent(result, preview)
                self.publish(preview)

    def test_facade_budget_and_builder_canaries_are_sanitized_before_any_runner(self) -> None:
        held = validation.LiveHeldIdentity(
            "a" * 64,
            "sha256:" + "b" * 64,
            "c" * 32,
            1000,
            "d" * 64,
            "podman version 5.8.1",
        )
        request = validation.LiveProcessRequest(
            101, 102, 1000, "pid:[1001]", "mnt:[1002]", "pid:[1003]", "mnt:[1004]"
        )
        for mode in MODES:
            phases = ("budget", "bridge", "json") if mode is MODES[2] else ("budget", "held")
            for phase in phases:
                with self.subTest(mode=mode, phase=phase):
                    private = self.root / (mode.value + "-" + phase)
                    private.mkdir(mode=0o700)
                    commands = validation.Commands(private)
                    commands.lifetime_context = {"private": str(private)}
                    commands.budget = cast(validation.Budget, Mock())
                    failure = UnprintablePrivateError(CANARIES["request"])
                    failure.private_payload = CANARIES["program"]
                    if phase == "budget":
                        cast(Mock, commands.budget).check.side_effect = failure
                    notifications: list[runner.AbortContext] = []

                    def notify(
                        _context: dict[str, Any],
                        observed: runner.AbortContext,
                        report: object,
                        notifications: list[runner.AbortContext] = notifications,
                    ) -> None:
                        notifications.append(observed)
                        self.assert_private_absent(observed, report)

                    with contextlib.ExitStack() as stack:
                        stack.enter_context(
                            patch.object(lifetime, "notify_abort", side_effect=notify)
                        )
                        launched = stack.enter_context(
                            patch.object(
                                validation,
                                "run",
                                side_effect=AssertionError("must fail before launch"),
                            )
                        )
                        rendered = stack.enter_context(
                            patch.object(
                                validation,
                                "_redacted_argv",
                                side_effect=AssertionError("opaque argv must not be rendered"),
                            )
                        )
                        excerpted = stack.enter_context(
                            patch.object(
                                validation,
                                "failure_evidence",
                                side_effect=AssertionError("opaque error must not be excerpted"),
                            )
                        )
                        stack.enter_context(
                            patch.object(
                                validation,
                                "LIVE_PID_SCHEMA",
                                validation.LivePidSchema.PODMAN_STATE_FIELDS,
                            )
                        )
                        if phase == "held":
                            stack.enter_context(
                                patch.object(validation, "_live_held", side_effect=failure)
                            )
                        elif phase == "bridge":
                            stack.enter_context(
                                patch.object(validation, "_rootful_bridge", side_effect=failure)
                            )
                        elif phase == "json":
                            stack.enter_context(
                                patch.object(validation, "_rootful_bridge", return_value=((), ()))
                            )
                            stack.enter_context(
                                patch.object(validation.json, "dumps", side_effect=failure)
                            )
                        with self.assertRaises(runner.OpaqueRunError) as caught:
                            if mode is runner.OpaqueObservation.PROCESSES:
                                cast(Any, commands)._live_processes(request)
                            else:
                                cast(Any, commands)._live_inspect(held, mode)
                    launched.assert_not_called()
                    rendered.assert_not_called()
                    excerpted.assert_not_called()
                    self.assertTrue(commands.outcomes)
                    self.assertTrue(all(o["status"] != "success" for o in commands.outcomes))
                    self.assertEqual(
                        caught.exception.reason, runner.OpaqueReason.INVOCATION_UNAVAILABLE
                    )
                    self.assert_private_absent(caught.exception, commands.outcomes, notifications)
                    self.publish({"status": "failure", "outcomes": commands.outcomes})

    def test_cli_exception_paths_emit_only_sanitized_runner_exception(self) -> None:
        def monitor() -> None:
            raise ValueError(CANARIES["hook"])

        with self.assertRaises(runner.OpaqueRunError) as caught:
            self.invoke(MODES[0], self.fixture_program("time.sleep(7)"), monitor=monitor)
        error = caught.exception
        self.assert_private_absent(error)
        for frontend in ("validation", "local"):
            with self.subTest(frontend=frontend):
                output = self.root / (frontend + "-exception.json")
                stdout, stderr = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    if frontend == "validation":
                        with patch.object(validation, "prepare_hosted_podman", side_effect=error):
                            status = validation.main(
                                [
                                    "prepare-podman",
                                    "--repository",
                                    "boxferry",
                                    "--candidate-sha",
                                    "a" * 40,
                                    "--output",
                                    str(output),
                                ]
                            )
                    else:
                        with (
                            patch.object(local, "_owned_output"),
                            patch.object(local, "execute", side_effect=error),
                        ):
                            status = local.main(
                                [
                                    "--repository-root",
                                    str(self.root),
                                    "--config",
                                    "unused.json",
                                    "--output",
                                    str(output),
                                ]
                            )
                self.assertEqual(status, 1)
                self.assertEqual(json.loads(output.read_text())["status"], "failure")
                self.assert_private_absent(
                    output.read_bytes(), stdout.getvalue(), stderr.getvalue()
                )

    def actual_fallback(self, failure: runner.OpaqueAbortContext) -> str:
        context, read_fd = self.handoff()
        stderr: list[bytes] = []
        real_write = os.write

        def write(fd: int, content: bytes) -> int:
            if fd == 2:
                stderr.append(content)
                return len(content)
            return real_write(fd, content)

        with (
            patch.object(validation, "serialize_report", side_effect=ValueError(CANARIES["hook"])),
            patch.object(lifetime.os, "write", side_effect=write),
        ):
            try:
                lifetime.notify_abort(context, failure, {"schema": 1})
            except BaseException as error:
                self.assert_private_absent(error)
        self.assertEqual(os.read(read_fd, 1), b"A")
        self.assertEqual(len(stderr), 1)
        self.assert_private_absent(stderr)
        return stderr[0].decode()

    def test_actual_outer_receipt_and_execute_merge_without_runtime_execution(self) -> None:
        for handoff_kind in ("preview", "fallback", "missing"):
            with self.subTest(handoff=handoff_kind):
                _result, failure, preview = self.persisted_failure(MODES[0], "sys.exit(19)")
                receipt = self.root / (handoff_kind + "-outer.json")
                close = Mock()
                outer = lifetime._OuterAbortEvidence(receipt, "f" * 32, close)
                outer.abort(failure)
                close.assert_called_once()
                self.assertTrue(outer.fields()["outer_abort_persisted"])
                self.assert_private_absent(receipt.read_bytes(), vars(outer), outer.fields())
                self.publish({**preview, **outer.fields()})

                fallback_line = self.actual_fallback(failure) if handoff_kind == "fallback" else ""

                # Actual execute owns setup/persistence/merge/closure. Only the root
                # subprocess and witness are mocked; no sudo or runtime is invoked.
                created: list[Path] = []
                real_mkdtemp = tempfile.mkdtemp

                def mkdtemp(
                    *args: object,
                    created: list[Path] = created,
                    real_mkdtemp: Callable[..., str] = real_mkdtemp,
                    **kwargs: object,
                ) -> str:
                    kwargs["dir"] = self.root
                    path = cast(Any, real_mkdtemp)(*args, **kwargs)
                    created.append(Path(path))
                    return path

                def root_call(
                    _argv: tuple[str, ...],
                    handoff_kind: str = handoff_kind,
                    preview: dict[str, object] = preview,
                    failure: runner.OpaqueAbortContext = failure,
                    fallback_line: str = fallback_line,
                    **kwargs: object,
                ) -> runner.RunResult:
                    self.assertEqual(kwargs["stage"], "local-rootful-lifetime")
                    preparation = Path(cast(Path, kwargs["cwd"]))
                    if handoff_kind == "preview":
                        (preparation / "result").write_text(validation.serialize_report(preview))
                    cast(Callable[[runner.AbortContext], None], kwargs["on_abort"])(
                        runner.AbortContext("local-rootful-lifetime", "failed", "command exited 19")
                    )
                    stderr = fallback_line
                    return runner.RunResult(
                        "failed",
                        19,
                        "",
                        stderr,
                        0.01,
                        runner.Diagnostic(
                            "local-rootful-lifetime", "command exited 19", "held", "local", "review"
                        ),
                        ("fixed-root-envelope",),
                    )

                with (
                    patch.object(lifetime.tempfile, "mkdtemp", side_effect=mkdtemp),
                    patch.object(lifetime, "run", side_effect=root_call) as mocked_run,
                    patch.object(lifetime, "_witness", return_value=[{"event": "uncertain"}]),
                    patch.object(lifetime, "CLOSURE_SECONDS", 0.01),
                ):
                    merged = lifetime.execute(
                        {
                            "automation": Path(__file__).resolve().parents[2],
                            "work": self.root,
                        }
                    )
                self.assertEqual(mocked_run.call_count, 1)
                self.assertEqual(merged["status"], "failure")
                self.assertFalse(merged["namespace_closed"])
                self.assertEqual(merged["cleanup"], "failure")
                self.assertTrue(merged["outer_abort_persisted"])
                if handoff_kind != "missing":
                    self.assertEqual(merged["command_failure_stage"], failure.stage)
                self.assert_private_absent(merged)
                self.publish(merged)
                for path in created:
                    shutil.rmtree(path)


if __name__ == "__main__":
    unittest.main()
