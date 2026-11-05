"""Bounded byte sanitization and reserved unittest-report data, without raw log files."""

from __future__ import annotations

import codecs
import json
import os
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import TypedDict

from boxferry_ci.runner import protected_values

CHUNK = 32 * 1024
RESULT_BYTES = 512 * 1024
PRIMARY_BYTES = 32 * 1024
SUMMARY_BYTES = 64 * 1024
TRACE_BYTES = 12 * 1024
RECORDS = 24


class Record(TypedDict):
    test_id: str
    exception: str
    cause: str
    location: str
    traceback: str
    omitted_trace_bytes: int


class TestData(TypedDict):
    schema: int
    test_returncode: int
    tests_run: int
    tests_discovered: int
    counts: dict[str, int]
    records: list[Record]
    omitted_records: int
    omitted_trace_bytes: int
    report_errors: list[str]
    duration_seconds: float


@dataclass(frozen=True)
class Bounds:
    original: int = 2_000_000
    head: int = 64 * 1024
    tail: int = 384 * 1024

    def validate(self) -> None:
        if not 0 < self.original <= 2_000_000:
            raise ValueError("original-output bound is outside the existing runner limit")
        if not 0 <= self.head <= 64 * 1024 or not 0 <= self.tail <= 384 * 1024:
            raise ValueError("retention bounds exceed the reviewed head/tail reserve")


class Shield:
    """Literal byte patterns; preflight bounds apply before discovery or retention."""

    def __init__(self, root: Path, environment: Mapping[str, str]) -> None:
        secrets = sorted(set(protected_values(environment)), key=len, reverse=True)
        root_text = str(root.resolve())
        if len(secrets) + 1 > 64:
            raise ValueError("protected-pattern count budget exceeded")
        encoded: list[bytes] = []
        total = 0
        for value in (*secrets, root_text):
            if len(value) > 16 * 1024:
                raise ValueError("protected-pattern length budget exceeded")
            data = value.encode("utf-8")
            total += len(data)
            if len(data) > 16 * 1024 or total > 64 * 1024:
                raise ValueError("protected-pattern byte budget exceeded")
            encoded.append(data)
        checkout = encoded.pop()
        values = [*encoded, checkout]
        label = b"<repository>"
        if any(value in label for value in encoded):
            label = b"*"
        self.patterns = sorted(
            [(value, b"*") for value in encoded] + [(checkout, label)],
            key=lambda item: len(item[0]),
            reverse=True,
        )
        self.overlap = max(3, *(len(value) for value in values)) - 1

    def stream(self) -> Sanitizer:
        return Sanitizer(self)

    def text(self, value: str, limit: int = 2048) -> str:
        return self.parts((value,), limit)[0]

    def line(self, value: str, limit: int = 2048) -> str:
        return self.text(value, limit).replace("\r", " ").replace("\n", " ")

    def parts(self, values: Iterable[str], limit: int) -> tuple[str, int]:
        if limit < 128:
            raise ValueError("rendered text reserve is too small")
        stream = self.stream()
        retained = Window(limit // 2, limit - limit // 2)
        for value in values:
            # At most8192 Unicode scalars produce one32KiB raw byte chunk.
            for offset in range(0, len(value), CHUNK // 4):
                data = value[offset : offset + CHUNK // 4].encode("utf-8", "replace")
                retained.add(stream.feed(data))
        retained.add(stream.finish())
        if retained.omitted:
            # Reserve the omission marker and at most two UTF8 clipping edges
            # inside, rather than in addition to, the rendered-text cap.
            reserve = min(64, limit // 4)
            del retained.head[max(0, len(retained.head) - reserve) :]
            del retained.tail[: min(reserve, len(retained.tail))]
        text = retained.render().decode("utf-8", "replace")
        if len(text.encode("utf-8")) > limit:
            raise ValueError("rendered text byte budget exceeded")
        return text, retained.omitted


class Sanitizer:
    def __init__(self, shield: Shield) -> None:
        self.shield = shield
        self.pending = bytearray()

    def feed(self, value: bytes, *, final: bool = False) -> bytes:
        if len(value) > CHUNK:
            raise ValueError("capture chunk budget exceeded")
        self.pending.extend(value)
        data = self.pending
        end = len(data) if final else max(0, len(data) - self.shield.overlap)
        position = 0
        result = bytearray()
        while position < end:
            matched = False
            for pattern, replacement in self.shield.patterns:
                if data.startswith(pattern, position):
                    result.extend(replacement)
                    position += len(pattern)
                    matched = True
                    break
            if matched:
                continue
            if data.startswith(b"::", position):
                result.extend(b": :")
                position += 2
            elif data.startswith(b"##[", position):
                result.extend(b"# #[")
                position += 3
            else:
                result.append(data[position])
                position += 1
        del self.pending[:position]
        return bytes(result)

    def finish(self) -> bytes:
        return self.feed(b"", final=True)


class Window:
    """Retention is nonfatal and accounts for every sanitized byte excluded."""

    def __init__(self, head: int, tail: int) -> None:
        self.head_limit = head
        self.tail_limit = tail
        self.head = bytearray()
        self.tail = bytearray()
        self.total = 0

    def add(self, data: bytes) -> None:
        self.total += len(data)
        room = max(0, self.head_limit - len(self.head))
        self.head.extend(data[:room])
        if self.tail_limit:
            self.tail.extend(data[room:])
            del self.tail[: max(0, len(self.tail) - self.tail_limit)]

    @property
    def omitted(self) -> int:
        return self.total - len(self.head) - len(self.tail)

    def render(self) -> bytes:
        marker = f"\n[omitted {self.omitted} sanitized bytes]\n".encode() if self.omitted else b""
        return bytes(self.head) + marker + bytes(self.tail)


class Logs:
    """Separate channels share one head reserve and bounded halves of the tail."""

    def __init__(self, shield: Shield, bounds: Bounds) -> None:
        self.bounds = bounds
        self.streams = {name: shield.stream() for name in ("stdout", "stderr")}
        self.windows = {name: Window(0, bounds.tail // 2) for name in ("stdout", "stderr")}
        # Decoder carry contains only already-redacted bytes (at most3/channel).
        self.decoders = {
            name: codecs.getincrementaldecoder("utf-8")("replace") for name in ("stdout", "stderr")
        }
        self.head_left = bounds.head
        self.original = 0
        self.exceeded = False

    def add(self, name: str, data: bytes) -> None:
        self.original += len(data)
        self.exceeded |= self.original > self.bounds.original
        safe = self.streams[name].feed(data)
        self._retain(name, self.decoders[name].decode(safe).encode("utf-8"))

    def _retain(self, name: str, data: bytes) -> None:
        window = self.windows[name]
        if self.exceeded:
            # Continue transforming/draining and counting, but stop accepting
            # ordinary chatter. Failure metadata/tail has its separate reserve.
            window.total += len(data)
            return
        take = min(self.head_left, len(data))
        window.head.extend(data[:take])
        window.total += take
        self.head_left -= take
        window.add(data[take:])

    def finish(self) -> None:
        for name, stream in self.streams.items():
            safe = self.decoders[name].decode(stream.finish(), final=True).encode("utf-8")
            self._retain(name, safe)

    def report(self) -> dict[str, object]:
        return {
            "original_bytes": self.original,
            "sanitized_bytes": sum(window.total for window in self.windows.values()),
            "omitted_bytes": sum(window.omitted for window in self.windows.values()),
            "transcript": "\n".join(
                f"[{name}]\n{window.render().decode('utf-8', 'replace')}"
                for name, window in self.windows.items()
            ),
        }


def encoded(value: object, limit: int, *, ascii_only: bool = True) -> bytes:
    data = json.dumps(value, ensure_ascii=ascii_only, separators=(",", ":")).encode("utf-8")
    if len(data) > limit:
        raise ValueError("serialized-result byte budget exceeded")
    return data


def write_json(path: Path, value: object, limit: int) -> None:
    data = encoded(value, limit)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)


def read_json(path: Path, limit: int) -> dict[str, object]:
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("handoff byte budget exceeded")
    value = json.loads(data)
    if not isinstance(value, dict):
        raise ValueError("handoff must be a JSON object")
    return value
