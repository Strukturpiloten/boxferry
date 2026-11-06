"""Independent unknown-canary checks at opaque observation publication boundaries.

Only harmless nonroot fixture children run. Private inspection streams are tested
with unknown canaries through runner, serializer and CLI publication boundaries.
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
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

    def persisted_failure(
        self,
        mode: runner.OpaqueObservation,
        ending: str,
        *,
        monitor: Callable[[], None] | None = None,
    ) -> tuple[runner.OpaqueRunResult, runner.OpaqueAbortContext, dict[str, object]]:
        # Fixture persistence is deliberately independent of the retired local
        # namespace handoff; exercise the generic runner's safe abort contract.
        output = self.root / "runner-failure.json"
        failures: list[runner.AbortContext] = []
        consumer = Mock()

        def abort(failure: runner.AbortContext) -> None:
            failures.append(failure)
            assert isinstance(failure, runner.OpaqueAbortContext)
            output.write_text(
                validation.serialize_report(
                    {
                        "schema": 1,
                        "status": "failure",
                        "command_failure_stage": failure.stage,
                        "command_failure_cause": failure.cause,
                        **failure.safe_fields(),
                    }
                )
            )

        result = self.invoke(
            mode, self.fixture_program(ending), consumer=consumer, abort=abort, monitor=monitor
        )
        consumer.assert_not_called()
        self.assertEqual(len(failures), 1)
        self.assertIsInstance(failures[0], runner.OpaqueAbortContext)
        preview = json.loads(output.read_text())
        self.assertEqual(preview["status"], "failure")
        self.assertEqual(preview["command_failure_stage"], mode.stage)
        self.assert_private_absent(result, failures, preview)
        return result, cast(runner.OpaqueAbortContext, failures[0]), preview

    def publish(self, report: dict[str, object]) -> None:
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
            output = self.root / ("validation-" + str(compact) + ".json")
            stdout, stderr = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
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
                            "docker",
                            "--sources-json",
                            "{}",
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
                try:
                    json.loads(content)
                except json.JSONDecodeError:
                    raise runner.OpaqueParseError(runner.OpaqueReason.INVALID_JSON) from None
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
                for phase in phases:
                    with self.subTest(mode=mode, exception=exception, phase=phase):
                        argv = (sys.executable, "-c", "pass", CANARIES["request"])
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

    def test_private_json_decoder_never_publishes_malformed_payload(self) -> None:
        for mode in MODES:

            def consume(content: bytes) -> None:
                json.loads(content)

            result = self.invoke(mode, self.fixture_program("sys.exit(0)"), consumer=consume)
            self.assertEqual(result.metrics.reason, runner.OpaqueReason.PRIVATE_PARSER_FAILED)
            self.assert_private_absent(result)
            self.publish(
                {
                    "schema": 1,
                    "status": "failure",
                    **runner.OpaqueAbortContext(
                        mode.stage,
                        result.status,
                        result.metrics.reason.value,
                        result.metrics,
                    ).safe_fields(),
                }
            )

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

    def test_cli_exception_paths_emit_only_sanitized_runner_exception(self) -> None:
        def monitor() -> None:
            raise ValueError(CANARIES["hook"])

        with self.assertRaises(runner.OpaqueRunError) as caught:
            self.invoke(MODES[0], self.fixture_program("time.sleep(7)"), monitor=monitor)
        error = caught.exception
        self.assert_private_absent(error)
        output = self.root / "validation-exception.json"
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            with patch.object(validation, "prepare_hosted_podman", side_effect=error):
                status = validation.main(
                    [
                        "prepare-podman",
                        "--repository",
                        "podman-lens",
                        "--candidate-sha",
                        "a" * 40,
                        "--output",
                        str(output),
                    ]
                )
        self.assertEqual(status, 1)
        self.assertEqual(json.loads(output.read_text())["status"], "failure")
        self.assert_private_absent(output.read_bytes(), stdout.getvalue(), stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
