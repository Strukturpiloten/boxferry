"""Bounded stopped-image acquisition, cache identity, and cleanup behavior."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from collections.abc import Sequence
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from boxferry_ci.renovate_runtime import RenovateRuntimeError, prepare_runtime, run_command

IMAGE = "ghcr.io/renovatebot/renovate:44.139.0@sha256:" + "a" * 64
IDENTITY = "ghcr.io/renovatebot/renovate@sha256:" + "a" * 64


class FakeEngine:
    def __init__(self, *, failure: str | None = None, version: str = "44.139.0") -> None:
        self.calls: list[list[str]] = []
        self.failure = failure
        self.version = version

    def __call__(self, command: Sequence[str], timeout: float) -> str:
        parts = list(command)
        self.calls.append(parts)
        if not 0 < timeout <= 300:
            raise AssertionError("unbounded runtime command")
        operation = "node" if parts[0] == "node" else parts[1]
        if operation == self.failure:
            raise RenovateRuntimeError("injected failure")
        if operation == "image":
            return json.dumps([IDENTITY])
        if operation == "create":
            return "c" * 64
        if operation == "cp":
            destination = Path(parts[-1])
            (destination / "package.json").write_text(json.dumps({"version": self.version}))
        if operation == "node":
            return "native-re2-available"
        return ""


class RenovateRuntimeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.pin = self.root / "runtime.json"
        self.pin.write_text(json.dumps({"image": IMAGE}))
        self.destination = self.root / "target/tools/renovate"

    def test_explicit_engine_immutable_identity_and_stopped_copy(self) -> None:
        for engine in ("podman", "docker"):
            with self.subTest(engine=engine):
                fake = FakeEngine()
                target = self.destination / engine
                result = prepare_runtime(self.pin, target, engine=engine, command=fake)
                self.assertEqual(result, target.absolute())
                self.assertEqual((target / "source-image.txt").read_text(), IMAGE + "\n")
                create = next(
                    call for call in fake.calls if call[0] == engine and call[1] == "create"
                )
                self.assertEqual(create[-3:], ["--network", "none", IMAGE])
                self.assertIn("--name", create)
                self.assertTrue(any(call[1:3] == ["image", "inspect"] for call in fake.calls))
                self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
                self.assertFalse(any(call[1] in {"run", "start", "build"} for call in fake.calls))

    def test_verified_cache_hit_uses_no_container_engine(self) -> None:
        first = FakeEngine()
        prepare_runtime(self.pin, self.destination, engine="podman", command=first)
        second = FakeEngine()
        prepare_runtime(self.pin, self.destination, engine="podman", command=second)
        self.assertEqual(len(second.calls), 1)
        self.assertEqual(second.calls[0][0], "node")

    def test_damaged_managed_cache_is_reacquired(self) -> None:
        prepare_runtime(self.pin, self.destination, engine="podman", command=FakeEngine())
        (self.destination / "package.json").write_text('{"version":"wrong"}')
        fake = FakeEngine()
        prepare_runtime(self.pin, self.destination, engine="podman", command=fake)
        self.assertTrue(any(call[1] == "pull" for call in fake.calls))

    def test_failures_after_create_clean_up_stopped_container(self) -> None:
        for failure in ("cp", "node"):
            with self.subTest(failure=failure):
                fake = FakeEngine(failure=failure)
                with self.assertRaises(RenovateRuntimeError):
                    prepare_runtime(self.pin, self.destination, engine="podman", command=fake)
                self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
                self.assertFalse(self.destination.exists())
                self.assertFalse(list(self.destination.parent.glob(".renovate-extract-*")))

    def test_version_or_image_identity_mismatch_blocks_installation(self) -> None:
        fake = FakeEngine(version="99.0.0")
        with self.assertRaisesRegex(RenovateRuntimeError, "version differs"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=fake)
        self.assertEqual(fake.calls[-1][1], "rm")

        def wrong_identity(command: Sequence[str], timeout: float) -> str:
            return "[]" if command[1] == "image" else FakeEngine()(command, timeout)

        with self.assertRaisesRegex(RenovateRuntimeError, "OCI image identity"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=wrong_identity)

    def test_cleanup_failure_is_not_success(self) -> None:
        with self.assertRaises(RenovateRuntimeError):
            prepare_runtime(
                self.pin, self.destination, engine="podman", command=FakeEngine(failure="rm")
            )
        self.assertFalse(self.destination.exists())

    def test_unknown_engine_and_mutable_pin_are_rejected(self) -> None:
        with self.assertRaisesRegex(RenovateRuntimeError, "explicitly"):
            prepare_runtime(self.pin, self.destination, engine="auto", command=FakeEngine())
        for image in (
            "ghcr.io/renovatebot/renovate:latest",
            "someone/renovate:1.2.3@sha256:" + "a" * 64,
        ):
            self.pin.write_text(json.dumps({"image": image}))
            with self.assertRaises(RenovateRuntimeError):
                prepare_runtime(self.pin, self.destination, engine="podman", command=FakeEngine())

    def test_output_and_time_limits_are_enforced_for_real_subprocesses(self) -> None:
        self.assertEqual(
            run_command([sys.executable, "-c", "print('bounded')"], 5).strip(), "bounded"
        )
        for stream in ("stdout", "stderr"):
            with (
                self.subTest(stream=stream),
                self.assertRaisesRegex(RenovateRuntimeError, "byte bound"),
            ):
                run_command([sys.executable, "-c", f"import sys; sys.{stream}.write('x'*70000)"], 5)
        with self.assertRaisesRegex(RenovateRuntimeError, "time bound"):
            run_command([sys.executable, "-c", "import time; time.sleep(5)"], 0.01)

    def test_invalid_create_output_still_cleans_owned_container_name(self) -> None:
        fake = FakeEngine()

        def invalid_id(command: Sequence[str], timeout: float) -> str:
            value = fake(command, timeout)
            return "invalid" if command[1] == "create" else value

        with self.assertRaisesRegex(RenovateRuntimeError, "invalid stopped-container"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=invalid_id)
        self.assertEqual(fake.calls[-1][1:3], ["rm", "--force"])
        self.assertTrue(fake.calls[-1][-1].startswith("boxferry-renovate-"))

    def test_unrelated_existing_destination_is_preserved(self) -> None:
        self.destination.mkdir(parents=True)
        existing = self.destination / "important.txt"
        existing.write_text("preserve")
        with self.assertRaisesRegex(RenovateRuntimeError, "not a managed"):
            prepare_runtime(self.pin, self.destination, engine="podman", command=FakeEngine())
        self.assertEqual(existing.read_text(), "preserve")


if __name__ == "__main__":
    unittest.main()
