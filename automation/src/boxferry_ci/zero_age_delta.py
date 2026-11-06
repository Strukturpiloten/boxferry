"""Read-only exact-repair bootstrap proof; A owns age policy, live S owns this proof.

No dependency acquisition, persistent eligibility state or trusted-service receipt is
created. Only an explicit proved-zero result may bypass the original A receipt wait.
See ADR 0097. Complete immutable trees, never candidate commands, supply the inputs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

from boxferry_ci.admission import (
    AdmissionContext,
    AdmissionError,
    parse_manifest,
    resolve_admission,
)

AUTHORITY = "1cf96f5a667d96e66c119753cf7814e22e0dee7e"
PRODUCER = "Strukturpiloten/boxferry"
SHA = re.compile(r"[0-9a-f]{40}\Z")
CALLER = ".github/workflows/dependency-eligibility.yml"
CONTRACT = "scripts/test-automation-contract.py"
MANIFEST = ".github/automation.json"
POLICY = "scripts/validation-policy.json"
FIXTURES = "automation/tests/fixtures/zero-age-delta/"
SOURCE_PATHS = (
    MANIFEST,
    ".github/renovate.json",
    ".github/workflows/ci.yml",
    CALLER,
    ".github/workflows/release-plz.yml",
    ".github/workflows/release.yml",
)
PROOF_PATH = "automation/src/boxferry_ci/zero_age_delta.py"
WAITER = ".github/workflows/shared-dependency-wait.yml"
# These paths are the complete reviewed 606 implementation surface. There is no
# broad source/test/docs exemption and no dependency declaration in this surface.
IMPLEMENTATION_PATHS = frozenset(
    {
        PROOF_PATH,
        "automation/tests/test_zero_age_delta.py",
        "docs/decisions/0097-read-only-zero-age-delta-bootstrap.md",
        "docs/decisions/README.md",
        "docs/automation.md",
        FIXTURES + "README.md",
        FIXTURES + "provenance.json",
    }
)
REQUIRED_604 = {
    ".github/workflows/shared-release-prepare.yml": (
        3982,
        "9b0d8b73511a5d924dbb7349dd6d7fe35fefb0e59ad97f8db41973fa60b3a6e5",
    ),
    "automation/src/boxferry_ci/release.py": (
        32964,
        "bc4fb17aee42d4ffca9e56c73f534bfdb95c9401cb872572a103b380e52f28f3",
    ),
    "automation/tests/test_release.py": (
        46975,
        "44a52a8558b99dfe8e0918b5ab76a34d2f96512d9e201da51645135bbdb13c20",
    ),
}


# Zero-based literal positions in the exact immutable known-base workflow bytes.
# A's general repin helper predates these local Action inputs. Full-file anchors
# make this a closed extension for actual owned slots, never a YAML rewrite.
ADDITIONAL_SOURCE_SLOTS = {
    (
        ".github/workflows/ci.yml",
        "ec27df5629f0583e55520517f64ddc3adbaedd9347d1a15886463d55233a2ba5",
    ): (171,),
    (
        ".github/workflows/release.yml",
        "9490679017f61d855ae8cd991c3269cc0757384445fa4862e44d96c14e8f5593",
    ): (229, 288),
    (
        ".github/workflows/ci.yml",
        "74b62b7b696d4e1915dbbd86362e6bf7c3da66b14f0ae2503d650b3d394d0500",
    ): (196,),
    (
        ".github/workflows/release.yml",
        "481c3108d34e431bec93dafd3842cdac3fc0720a7ed9029b75066d44efd77522",
    ): (173, 445),
}


class ProofError(ValueError):
    """A concrete reason to refuse proof, with a stable disposition."""

    def __init__(
        self,
        reason: str,
        *,
        invalid: bool = False,
        details: dict[str, object] | None = None,
    ) -> None:
        super().__init__(reason)
        self.reason = reason
        self.invalid = invalid
        self.details = details or {}


def require(condition: object, reason: str, *, invalid: bool = False) -> None:
    if not condition:
        raise ProofError(reason, invalid=invalid)


def revision(value: object) -> str:
    require(isinstance(value, str) and SHA.fullmatch(value), "malformed-revision", invalid=True)
    return str(value)


@dataclass(frozen=True)
class Repair:
    repository: str
    number: int
    base: str
    name: str
    test_mode: int
    caller_before: str
    caller_after: str
    test_before: str
    test_after: str
    policy: str


REGISTRY = {
    "Strukturpiloten/podman-lens": Repair(
        "Strukturpiloten/podman-lens",
        159,
        "2d3d36d62bf7aa3ad2a10d3b421306552ffe3570",
        "podman",
        0o100644,
        "df4b34e56830fa964b90e7e03f35a3527e1fd838966a7453d771d6dfaaaaec7b",
        "942ae4243dd96222761f4299b959d48d2d4638f8187121e1e0bde90b7eaf2864",
        "e11b7ea956dfdaa8e6713b28f22d3b1dd0cfa049a944471be8f7563d936316a6",
        "7d01ffc1319a79407ebf1589a5e12b1088627975aefc965491184f0c5b4791db",
        "0e80da07f01f66e5963c423769d0b9d6ecf92fb131437a76b2828c84c979462e",
    ),
    "Strukturpiloten/quadlet-lens": Repair(
        "Strukturpiloten/quadlet-lens",
        175,
        "3c361134709a6d1654e0f10c83983999db24cfef",
        "quadlet",
        0o100755,
        "ca457299c16eaa2dfbd505637529e7606ba4decefde68406a41c92ba098ea2b2",
        "4308d0bc18170b603a1eef509aa6b6f68b59868692394331bf238a926b5e139b",
        "ea144a0bd486d502691070cf5f1a8244f298ef008bb5f19609250111f12f9c0f",
        "e54314286484d5a896bf4a1352cac1bf298f7f433adc41fd8a50dcdade748598",
        "d4a88fb53131ebbea02578d532d14ed8e1c5581efc162a30a9f61c7cc522ee90",
    ),
}


@dataclass(frozen=True)
class Entry:
    mode: int
    oid: str
    content: bytes

    @property
    def digest(self) -> str:
        return hashlib.sha256(self.content).hexdigest()


@dataclass(frozen=True)
class Tree:
    oid: str
    entries: dict[str, Entry]

    @property
    def digest(self) -> str:
        records = [(p, e.mode, e.oid, e.digest) for p, e in sorted(self.entries.items())]
        return hashlib.sha256(json.dumps(records, separators=(",", ":")).encode()).hexdigest()


class Git:
    """Hash-verified bounded object reader; no fetch, replacement, hooks or config inheritance."""

    def __init__(
        self, root: Path, deadline: float, *, child_signal_mask: set[int] | None = None
    ) -> None:
        environment = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "PYTHON"))}
        environment.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_CONFIG_SYSTEM=os.devnull,
            GIT_TERMINAL_PROMPT="0",
            GIT_NO_REPLACE_OBJECTS="1",
            GIT_NO_LAZY_FETCH="1",
            GIT_OPTIONAL_LOCKS="0",
        )
        self.command = [
            "git",
            "--no-replace-objects",
            "--literal-pathspecs",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.allow=never",
            "-C",
            str(root),
        ]
        self.env = environment
        self.deadline = deadline
        process: subprocess.Popen[bytes] | None = None
        selector: selectors.BaseSelector | None = None
        try:
            with _blocked_cancellation() as previous_signals:
                # held() supplies the mask from before its outer registration
                # guard; restoring this nested guard's mask would block signals
                # permanently in the child.
                inherited = set(
                    previous_signals if child_signal_mask is None else child_signal_mask
                )

                def restore_signals() -> None:
                    signal.pthread_sigmask(signal.SIG_SETMASK, inherited)

                process = subprocess.Popen(
                    [*self.command, "cat-file", "--batch"],
                    env=environment,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    preexec_fn=restore_signals,
                )
                self.process = process
            selector = selectors.DefaultSelector()
            self.selector = selector
            assert process.stdout is not None
            selector.register(process.stdout, selectors.EVENT_READ)
            self.buffer = bytearray()
            self.total = 0
            self.cache: dict[str, tuple[str, bytes]] = {}
        except BaseException:
            with _blocked_cancellation():
                try:
                    if process is not None:
                        if process.poll() is None:
                            process.kill()
                        process.wait(timeout=2)
                finally:
                    if process is not None:
                        if process.stdin is not None:
                            process.stdin.close()
                        if process.stdout is not None:
                            process.stdout.close()
                    if selector is not None:
                        selector.close()
            raise

    def close(self) -> None:
        with _blocked_cancellation():
            try:
                if self.process.poll() is None:
                    self.process.kill()
                self.process.wait(timeout=2)
            finally:
                self.selector.close()
                if self.process.stdin is not None:
                    self.process.stdin.close()
                if self.process.stdout is not None:
                    self.process.stdout.close()

    def _read(self, length: int, *, line: bool = False) -> bytes:
        while True:
            if line and b"\n" in self.buffer:
                end = self.buffer.index(b"\n") + 1
                require(end <= length, "git-header-budget")
                result = bytes(self.buffer[:end])
                del self.buffer[:end]
                return result
            if not line and len(self.buffer) >= length:
                result = bytes(self.buffer[:length])
                del self.buffer[:length]
                return result
            require(len(self.buffer) <= length, "git-stream-budget")
            left = self.deadline - time.monotonic()
            require(left > 0 and self.selector.select(left), "proof-time-budget")
            assert self.process.stdout is not None
            chunk = os.read(self.process.stdout.fileno(), min(65536, length + 1 - len(self.buffer)))
            require(chunk, "git-truncated-data", invalid=True)
            self.buffer.extend(chunk)

    def object(self, oid: str, expected: str) -> bytes:
        revision(oid)
        if oid not in self.cache:
            require(len(self.cache) < 16384, "git-object-count-budget")
            assert self.process.stdin is not None
            self.process.stdin.write(oid.encode() + b"\n")
            self.process.stdin.flush()
            fields = self._read(128, line=True).strip().split(b" ")
            require(
                len(fields) == 3 and fields[0] == oid.encode(), "git-object-header", invalid=True
            )
            try:
                kind = fields[1].decode("ascii")
                size = int(fields[2])
            except (UnicodeError, ValueError) as error:
                raise ProofError("git-object-header", invalid=True) from error
            require(0 <= size <= 2 * 1024 * 1024, "git-object-size-budget")
            self.total += size
            require(self.total <= 40 * 1024 * 1024, "git-total-byte-budget")
            body = self._read(size)
            require(self._read(1) == b"\n", "git-object-terminator", invalid=True)
            actual = hashlib.sha1(kind.encode() + b" " + str(size).encode() + b"\0" + body)
            require(actual.hexdigest() == oid, "git-object-hash", invalid=True)
            self.cache[oid] = kind, body
        kind, body = self.cache[oid]
        require(kind == expected, "git-object-kind", invalid=True)
        return body

    def small(self, args: tuple[str, ...], limit: int = 256) -> bytes:
        # Admission uses this adapter only for small, explicitly capped metadata.
        process: subprocess.Popen[bytes] | None = None
        selector: selectors.BaseSelector | None = None
        try:
            with _blocked_cancellation() as previous_signals:

                def restore_signals() -> None:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_signals)

                process = subprocess.Popen(
                    [*self.command, *args],
                    env=self.env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    preexec_fn=restore_signals,
                )
            selector = selectors.DefaultSelector()
            assert process.stdout is not None
            selector.register(process.stdout, selectors.EVENT_READ)
            result = bytearray()
            while True:
                left = self.deadline - time.monotonic()
                require(left > 0 and selector.select(left), "proof-time-budget")
                chunk = os.read(process.stdout.fileno(), min(65536, limit + 1 - len(result)))
                if not chunk:
                    break
                result.extend(chunk)
                require(len(result) <= limit, "git-metadata-budget")
            require(
                process.wait(timeout=max(0.01, self.deadline - time.monotonic())) == 0,
                "git-metadata-unavailable",
            )
            return bytes(result)
        finally:
            with _blocked_cancellation():
                try:
                    if process is not None:
                        if process.poll() is None:
                            process.kill()
                        process.wait(timeout=2)
                finally:
                    if process is not None and process.stdout is not None:
                        process.stdout.close()
                    if selector is not None:
                        selector.close()

    def head(self) -> str:
        return revision(self.small(("rev-parse", "HEAD")).decode("ascii").strip())

    def commit(self, oid: str) -> tuple[str, list[str]]:
        raw = self.object(oid, "commit")
        header, separator, _ = raw.partition(b"\n\n")
        require(separator, "git-commit-header", invalid=True)
        rows = header.splitlines()
        trees = [line[5:].decode("ascii") for line in rows if line.startswith(b"tree ")]
        parents = [
            revision(line[7:].decode("ascii")) for line in rows if line.startswith(b"parent ")
        ]
        require(len(trees) == 1 and rows[0].startswith(b"tree "), "git-commit-tree", invalid=True)
        return revision(trees[0]), parents

    def tree(self, oid: str) -> Tree:
        root, _ = self.commit(oid)
        entries: dict[str, Entry] = {}

        def visit(tree_id: str, prefix: str, depth: int) -> None:
            require(depth <= 24, "git-tree-depth-budget")
            raw = self.object(tree_id, "tree")
            offset = 0
            names: set[str] = set()
            while offset < len(raw):
                space = raw.find(b" ", offset)
                nul = raw.find(b"\0", space + 1)
                require(
                    space > offset and nul > space and nul + 21 <= len(raw),
                    "git-tree-encoding",
                    invalid=True,
                )
                try:
                    token = raw[offset:space]
                    mode = int(token, 8)
                    name = raw[space + 1 : nul].decode("utf-8")
                except (ValueError, UnicodeError) as error:
                    raise ProofError("git-tree-encoding", invalid=True) from error
                require(
                    token in {b"40000", b"100644", b"100755", b"120000", b"160000"},
                    "git-tree-mode",
                    invalid=True,
                )
                require(
                    name
                    and len(name.encode()) <= 255
                    and name not in {".", ".."}
                    and name.casefold() != ".git"
                    and "/" not in name
                    and "\\" not in name
                    and not any(ord(c) < 32 or ord(c) == 127 for c in name)
                    and name not in names,
                    "git-unsafe-path",
                    invalid=True,
                )
                names.add(name)
                path = prefix + name
                require(
                    len(path.encode()) <= 1024 and path not in entries,
                    "git-unsafe-path",
                    invalid=True,
                )
                object_id = raw[nul + 1 : nul + 21].hex()
                offset = nul + 21
                require(len(entries) < 8192, "git-tree-entry-budget")
                if mode == 0o40000:
                    entries[path] = Entry(mode, object_id, b"")
                    visit(object_id, path + "/", depth + 1)
                elif mode == 0o160000:
                    entries[path] = Entry(mode, object_id, b"")
                else:
                    entries[path] = Entry(mode, object_id, self.object(object_id, "blob"))

        visit(root, "", 0)
        return Tree(root, entries)


def changed_paths(before: Tree, after: Tree) -> set[str]:
    changed: set[str] = set()
    for path in before.entries.keys() | after.entries.keys():
        a, b = before.entries.get(path), after.entries.get(path)
        # Directory OIDs derive from descendants; their existence/modes still count.
        if a is not None and b is not None and a.mode == b.mode == 0o40000:
            continue
        if a != b:
            changed.add(path)
    return changed


def regular(tree: Tree, path: str, mode: int = 0o100644) -> Entry:
    entry = tree.entries.get(path)
    require(entry is not None and entry.mode == mode, "required-regular-file")
    assert entry is not None
    return entry


def template(producer: Tree, repair: Repair, kind: str, phase: str, digest: str) -> bytes:
    data = regular(producer, f"{FIXTURES}{repair.name}/{kind}-{phase}.txt").content
    require(hashlib.sha256(data).hexdigest() == digest, "reviewed-template-hash")
    return data


def repin(data: bytes, path: str, source: str) -> bytes:
    """Generate actual-S expectation from held A, never from candidate content."""
    if path == MANIFEST:
        old, policy = parse_manifest(data)
        require(old == AUTHORITY and policy == POLICY, "manifest-anchor")
        slot = re.compile(rb'("revision"[ \t]*:[ \t]*")' + AUTHORITY.encode() + rb'(")')
        require(len(slot.findall(data)) == 1, "manifest-owned-slot")
        result = slot.sub(lambda m: m[1] + source.encode() + m[2], data)
        require(parse_manifest(result) == (source, POLICY), "manifest-owned-slot")
        return result
    if path == ".github/renovate.json":
        value = json.loads(data)
        expected = [
            f"github>Strukturpiloten/boxferry//renovate-presets/{name}#{AUTHORITY}"
            for name in ("common", "rust")
        ]
        require(isinstance(value, dict) and value.get("extends") == expected, "renovate-owned-slot")
        result = data
        for item in expected:
            literal = json.dumps(item).encode()
            require(result.count(literal) == 1, "renovate-owned-slot")
            result = result.replace(literal, json.dumps(item[:-40] + source).encode(), 1)
        return result
    from boxferry_ci.eligibility import expected_ci_upgrade

    result = expected_ci_upgrade(data, AUTHORITY, source)
    require(result is not None, "workflow-owned-slot")
    assert result is not None
    lines = result.splitlines(keepends=True)
    slots = ADDITIONAL_SOURCE_SLOTS.get((path, hashlib.sha256(data).hexdigest()), ())
    for index in slots:
        require(
            lines[index] == b"          automation-sha: " + AUTHORITY.encode() + b"\n",
            "local-action-owned-slot",
        )
        lines[index] = b"          automation-sha: " + source.encode() + b"\n"
    result = b"".join(lines)
    if any(
        b"uses: ./_automation/.github/actions/" + action in data
        for action in (b"shared-node", b"shared-crate-publish")
    ):
        require(
            (path, hashlib.sha256(data).hexdigest()) in ADDITIONAL_SOURCE_SLOTS,
            "unrecognized-local-action-source-slot",
        )
    return result


def match_repair_shape(
    before: Tree, after: Tree, producer: Tree, repair: Repair, source: str
) -> dict[str, Any]:
    require(regular(before, CALLER).digest == repair.caller_before, "base-caller-anchor")
    require(
        regular(before, CONTRACT, repair.test_mode).digest == repair.test_before, "base-test-anchor"
    )
    require(regular(before, POLICY).digest == repair.policy, "base-policy-anchor")
    require(
        regular(before, MANIFEST).digest
        == "3ea3aae7a5d63a0c305efb60fad4b2390992a993b8eece956ab3aa8ada465389",
        "base-manifest-anchor",
    )
    expected_caller = template(producer, repair, "caller", "after", repair.caller_after)
    expected_test = template(producer, repair, "contract", "after", repair.test_after)
    require(
        regular(after, CONTRACT, repair.test_mode).content == expected_test, "repair-test-shape"
    )
    require(regular(after, POLICY) == regular(before, POLICY), "policy-changed")
    modes: set[str] = set()
    for path in SOURCE_PATHS:
        baseline = expected_caller if path == CALLER else regular(before, path).content
        actual = regular(after, path).content
        if actual == baseline:
            modes.add("A")
        elif actual == repin(baseline, path, source):
            modes.add("S")
        else:
            raise ProofError("consumer-source-or-repair-shape")
    require(len(modes) == 1, "mixed-source-references")
    expected_changes = {CALLER, CONTRACT} | (set(SOURCE_PATHS) if modes == {"S"} else set())
    require(changed_paths(before, after) == expected_changes, "consumer-extra-tree-delta")
    # A complete path/mode/blob proof covers all dependency-bearing inputs, not
    # merely files understood by today's package checker. Record the full digests.
    return {
        "registry": f"{repair.repository}#{repair.number}",
        "source_mode": next(iter(modes)),
        "consumer_before_tree": before.oid,
        "consumer_after_tree": after.oid,
        "consumer_before_digest": before.digest,
        "consumer_after_digest": after.digest,
        "consumer_entries": len(before.entries),
        "changed_paths": sorted(expected_changes),
    }


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(
        self, req: Request, fp: object, code: int, msg: str, headers: object, newurl: str
    ) -> None:
        raise ProofError("metadata-redirect-unavailable")


class Metadata:
    """Fixed-host GET-only reader; public custody requests never carry credentials."""

    def __init__(self, deadline: float, *, public: bool = False) -> None:
        self.deadline = deadline
        self.public = public
        self.requests = 0
        self.opener = build_opener(ProxyHandler({}), NoRedirect())

    def get(self, path: str) -> dict[str, Any]:
        allowed = re.fullmatch(
            r"/repos/Strukturpiloten/(?:boxferry|podman-lens|quadlet-lens)"
            r"(?:/(?:actions/runs/[1-9][0-9]*/attempts/[1-9][0-9]*|pulls/[1-9][0-9]*"
            r"|git/commits/[0-9a-f]{40}|git/ref/heads/automation-v1"
            r"|compare/[0-9a-f]{40}\.\.\.[0-9a-f]{40}))?",
            path,
        )
        require(allowed, "metadata-endpoint", invalid=True)
        self.requests += 1
        require(self.requests <= 6, "metadata-request-budget")
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, "proof-time-budget")
        headers = {"Accept": "application/vnd.github+json", "User-Agent": "BoxFerry-zero-age-delta"}
        if not self.public:
            token = os.environ.get("GH_TOKEN", "")
            if token:
                headers["Authorization"] = f"Bearer {token}"
        request = Request("https://api.github.com" + path, headers=headers, method="GET")
        try:
            with self.opener.open(request, timeout=min(6, remaining)) as response:
                if response.status != 200:
                    raise ProofError(
                        "metadata-unavailable",
                        details={
                            "http_status": response.status,
                            "error_class": "HTTPError",
                            "transport": "public-metadata" if self.public else "source-metadata",
                        },
                    )
                raw = response.read(1024 * 1024 + 1)
        except HTTPError as error:
            raise ProofError(
                "public-metadata-unavailable" if self.public else "metadata-unavailable",
                details={
                    "http_status": error.code,
                    "error_class": "HTTPError",
                    "transport": "public-metadata" if self.public else "source-metadata",
                },
            ) from error
        except (OSError, URLError, TimeoutError) as error:
            raise ProofError(
                "public-metadata-unavailable" if self.public else "metadata-unavailable",
                details={
                    "error_class": type(error).__name__,
                    "transport": "public-metadata" if self.public else "source-metadata",
                },
            ) from error
        require(len(raw) <= 1024 * 1024, "metadata-byte-budget")
        try:
            data = json.loads(raw)
        except (UnicodeError, ValueError) as error:
            raise ProofError("metadata-malformed", invalid=True) from error
        require(isinstance(data, dict), "metadata-malformed", invalid=True)
        return data


def canonical(api: Metadata) -> str:
    ref = api.get(f"/repos/{PRODUCER}/git/ref/heads/automation-v1")
    require(ref.get("ref") == "refs/heads/automation-v1", "canonical-ref-custody", invalid=True)
    obj = ref.get("object")
    require(
        isinstance(obj, dict) and obj.get("type") == "commit", "canonical-ref-kind", invalid=True
    )
    assert isinstance(obj, dict)
    sha = revision(obj.get("sha"))
    commit = api.get(f"/repos/{PRODUCER}/git/commits/{sha}")
    require(commit.get("sha") == sha, "canonical-commit-custody", invalid=True)
    return sha


class ProofCancelled(BaseException):
    """Cancellation propagates through resource cleanup and never becomes proof."""

    def __init__(self, signum: int) -> None:
        self.signum = signum


@contextmanager
def _blocked_cancellation() -> Generator[set[int]]:
    previous = signal.pthread_sigmask(
        signal.SIG_BLOCK, {signal.SIGALRM, signal.SIGTERM, signal.SIGINT}
    )
    try:
        yield previous
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


def _validate_object_database(bare: Path) -> None:
    """Validate live and terminal state, including directory links and fast exits."""
    state = bare.lstat()
    require(not stat.S_ISLNK(state.st_mode), "candidate-object-db-link", invalid=True)
    require(stat.S_ISDIR(state.st_mode), "candidate-object-db-kind", invalid=True)
    files, directories, size = 0, 0, 0
    for directory, names, filenames in os.walk(bare, followlinks=False):
        for name in [*names, *filenames]:
            path = Path(directory) / name
            try:
                state = path.lstat()
            except FileNotFoundError:
                continue  # Git atomically renames temporary pack/index files.
            require(not stat.S_ISLNK(state.st_mode), "candidate-object-db-link", invalid=True)
            if stat.S_ISDIR(state.st_mode):
                directories += 1
            else:
                require(stat.S_ISREG(state.st_mode), "candidate-object-db-kind", invalid=True)
                files += 1
                size += state.st_size
                require(
                    state.st_size <= 32 * 1024 * 1024, "candidate-object-acquisition-disk-budget"
                )
            require(
                files <= 8192 and directories <= 8192 and size <= 64 * 1024 * 1024,
                "candidate-object-acquisition-disk-budget",
            )


def _reap_fetch(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=2)


def _git_diagnostics(stderr: bytes, returncode: int | None) -> dict[str, object]:
    """Classify only bounded known Git errors; never echo URLs/headers/paths/data."""
    prefix = stderr[:4096].lower()
    result: dict[str, object] = {
        "git_exit_code": returncode,
        "stderr_captured_bytes": len(prefix),
        "stderr_truncated": len(stderr) > 4096,
    }
    statuses = re.findall(rb"(?:http[/ ]|returned error[: ]+)([45][0-9]{2})(?:\D|$)", prefix)
    if statuses:
        result["http_statuses"] = sorted({int(status) for status in statuses})
    classes = {
        "http-error": (b"rpc failed", b"returned error", b"http 4", b"http 5"),
        "repository-unavailable": (
            b"repository not found",
            b"could not read from remote repository",
        ),
        "object-unavailable": (b"not our ref", b"couldn't find remote ref", b"unadvertised object"),
        "connection-failed": (b"could not resolve host", b"failed to connect", b"couldn't connect"),
        "tls-failed": (b"ssl certificate", b"certificate verify failed", b"tls connection"),
        "pack-invalid": (b"index-pack failed", b"early eof", b"invalid index-pack output"),
        "local-storage-failed": (b"no space left", b"file size limit", b"unable to write"),
    }
    result["git_error_classes"] = [
        name for name, markers in classes.items() if any(marker in prefix for marker in markers)
    ] or ["unclassified"]
    return result


def acquire_consumer(root: Path, repair: Repair, tested: str, deadline: float) -> Path:
    """Standard anonymous Git fetch into a bounded bare DB; no candidate worktree."""
    import pwd

    revision(tested)
    require(repair.repository in REGISTRY, "object-acquisition-repository", invalid=True)
    homes = {Path.home(), Path(pwd.getpwuid(os.getuid()).pw_dir)}
    require(
        not any(
            (home / name).exists() or (home / name).is_symlink()
            for home in homes
            for name in (".netrc", "_netrc")
        ),
        "ambient-netrc-prevents-anonymous-acquisition",
    )
    # Keep only ordinary process location/locale settings. No token, proxy,
    # askpass, SSH, curl, Git override or ambient credential setting is inherited.
    environment = {
        k: v
        for k, v in os.environ.items()
        if k in {"PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL"}
    }
    environment.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_TERMINAL_PROMPT="0",
        GIT_ASKPASS="/bin/false",
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_NO_LAZY_FETCH="1",
        GIT_OPTIONAL_LOCKS="0",
    )
    bare = root / "objects.git"
    command = [
        "git",
        "--no-replace-objects",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "credential.helper=",
        "-c",
        "credential.interactive=false",
        "-c",
        "core.askPass=/bin/false",
        "-c",
        "http.extraHeader=",
        "-c",
        "http.extraHeader=Authorization:",
        "-c",
        "http.followRedirects=false",
        "-c",
        "http.proxy=",
        "-c",
        "http.cookieFile=/dev/null",
        "-c",
        "http.saveCookies=false",
        "-c",
        "protocol.allow=never",
        "-c",
        "protocol.https.allow=always",
    ]
    initializer: subprocess.Popen[bytes] | None = None
    try:
        with _blocked_cancellation() as initial_signals:

            def restore_initial_signals() -> None:
                signal.pthread_sigmask(signal.SIG_SETMASK, initial_signals)

            initializer = subprocess.Popen(
                [*command, "init", "--bare", "--template=", "--quiet", str(bare)],
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                preexec_fn=restore_initial_signals,
            )
        require(
            initializer.wait(timeout=max(0.01, min(3, deadline - time.monotonic()))) == 0,
            "candidate-object-initialization-unavailable",
        )
    finally:
        with _blocked_cancellation():
            if initializer is not None:
                if initializer.poll() is None:
                    initializer.kill()
                initializer.wait(timeout=2)

    def limits() -> None:
        import resource

        resource.setrlimit(resource.RLIMIT_AS, (256 * 1024 * 1024, 256 * 1024 * 1024))
        resource.setrlimit(resource.RLIMIT_FSIZE, (32 * 1024 * 1024, 32 * 1024 * 1024))
        resource.setrlimit(resource.RLIMIT_CPU, (15, 15))
        signal.pthread_sigmask(signal.SIG_SETMASK, previous_signals)

    process: subprocess.Popen[bytes] | None = None
    selector: selectors.BaseSelector | None = None
    reaped = False
    failure: ProofError | None = None
    try:
        # Ownership is established before pending cancellation can be delivered.
        with _blocked_cancellation() as previous_signals:
            process = subprocess.Popen(
                [
                    *command,
                    "-C",
                    str(bare),
                    "fetch",
                    "--no-tags",
                    "--no-recurse-submodules",
                    "--no-auto-maintenance",
                    "--no-write-fetch-head",
                    "--no-progress",
                    "--depth=1",
                    f"https://github.com/{repair.repository}.git",
                    tested,
                ],
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                preexec_fn=limits,
                start_new_session=True,
            )
        selector = selectors.DefaultSelector()
        assert process.stdout is not None and process.stderr is not None
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        total_output = 0
        stderr = bytearray()

        def read_output(key: selectors.SelectorKey) -> None:
            nonlocal total_output
            used = len(stderr) if key.data == "stderr" else total_output
            chunk = os.read(key.fd, 4097 - used)
            if not chunk:
                assert selector is not None
                selector.unregister(key.fileobj)
            elif key.data == "stderr":
                stderr.extend(chunk)
                if len(stderr) > 4096:
                    raise ProofError(
                        "candidate-object-acquisition-stderr-budget",
                        details=_git_diagnostics(bytes(stderr), None),
                    )
            else:
                total_output += len(chunk)
                require(total_output <= 4096, "candidate-object-acquisition-output-budget")

        while process.poll() is None:
            require(time.monotonic() < deadline, "candidate-object-acquisition-time-budget")
            _validate_object_database(bare)
            for key, _ in selector.select(min(0.05, max(0.001, deadline - time.monotonic()))):
                read_output(key)
        # Stop/reap the whole owned session before final validation and pipe EOF:
        # a fast parent exit must not leave descendants changing the object DB.
        with _blocked_cancellation():
            _reap_fetch(process)
            reaped = True
        while selector.get_map():
            remaining = deadline - time.monotonic()
            require(remaining > 0, "candidate-object-acquisition-time-budget")
            ready = selector.select(remaining)
            require(ready, "candidate-object-acquisition-time-budget")
            for key, _ in ready:
                read_output(key)
        _validate_object_database(bare)
        if process.returncode != 0:
            raise ProofError(
                "candidate-object-acquisition-unavailable",
                details=_git_diagnostics(bytes(stderr), process.returncode),
            )
        require(not (bare / ".git").exists(), "candidate-worktree-created", invalid=True)
        return bare
    except ProofError as error:
        failure = error
        raise
    finally:
        # Covers constructor-return assignment, selector creation/register,
        # alarm/TERM/INT, terminal validation and every failure after spawn.
        with _blocked_cancellation():
            try:
                if process is not None and not reaped:
                    _reap_fetch(process)
            finally:
                if process is not None:
                    if failure is not None and failure.reason.startswith(
                        "candidate-object-acquisition"
                    ):
                        failure.details["git_exit_code"] = process.returncode
                    if process.stdout is not None:
                        process.stdout.close()
                    if process.stderr is not None:
                        process.stderr.close()
                if selector is not None:
                    selector.close()


def repository_identity(value: object, repository: str) -> bool:
    return isinstance(value, dict) and value.get("full_name") == repository


def normalized_ref(value: object) -> str:
    require(isinstance(value, str), "run-ref-custody", invalid=True)
    return str(value).removeprefix("refs/heads/")


def pull_identity(value: dict[str, Any], repair: Repair) -> str:
    require(value.get("number") == repair.number, "pr-number-custody", invalid=True)
    require(
        value.get("state") == "open"
        and value.get("merged") is False
        and value.get("merged_at") is None,
        "pr-no-longer-open",
    )
    base, head = value.get("base"), value.get("head")
    require(isinstance(base, dict) and isinstance(head, dict), "pr-metadata-custody", invalid=True)
    assert isinstance(base, dict) and isinstance(head, dict)
    require(
        repository_identity(base.get("repo"), repair.repository) and base.get("ref") == "main",
        "pr-repository-custody",
        invalid=True,
    )
    # This bootstrap covers the two reviewed same-repository repair branches only.
    require(
        repository_identity(head.get("repo"), repair.repository), "pr-head-repository", invalid=True
    )
    return revision(head.get("sha"))


def bind_context(
    context: AdmissionContext,
    event: dict[str, Any],
    run: dict[str, Any],
    repair: Repair,
    run_id: int,
    attempt: int,
    ref: str,
) -> None:
    require(
        repository_identity(event.get("repository"), context.repository),
        "event-repository",
        invalid=True,
    )
    require(
        run.get("id") == run_id and run.get("run_attempt") == attempt,
        "run-attempt-custody",
        invalid=True,
    )
    require(
        repository_identity(run.get("repository"), context.repository),
        "run-repository",
        invalid=True,
    )
    require(
        run.get("event") == context.event,
        "run-candidate-custody",
        invalid=True,
    )
    require(run.get("path") == ".github/workflows/ci.yml", "run-workflow-custody", invalid=True)
    refs = run.get("referenced_workflows")
    require(isinstance(refs, list), "run-shared-source-custody", invalid=True)
    assert isinstance(refs, list)
    expected = f"{PRODUCER}/{WAITER}@{context.automation_sha}"
    matches = [
        item
        for item in refs
        if isinstance(item, dict) and str(item.get("path", "")).startswith(f"{PRODUCER}/{WAITER}@")
    ]
    require(
        len(matches) == 1
        and matches[0].get("path") == expected
        and matches[0].get("sha") == context.automation_sha,
        "run-shared-source-custody",
        invalid=True,
    )
    if context.event == "pull_request":
        pull = event.get("pull_request")
        require(
            isinstance(pull, dict) and event.get("number") == repair.number,
            "event-pr-custody",
            invalid=True,
        )
        assert isinstance(pull, dict)
        require(
            pull.get("number") == repair.number
            and isinstance(pull.get("base"), dict)
            and pull["base"].get("sha") == context.policy_sha,
            "event-pr-base",
            invalid=True,
        )
        require(ref == f"refs/pull/{repair.number}/merge", "event-pr-ref", invalid=True)
        pulls = run.get("pull_requests")
        require(
            isinstance(pulls, list) and len(pulls) == 1 and pulls[0].get("number") == repair.number,
            "run-pr-custody",
            invalid=True,
        )
        head = pull.get("head")
        require(
            isinstance(head, dict)
            and run.get("head_branch") == head.get("ref")
            and run.get("head_sha") == head.get("sha"),
            "run-pr-ref",
            invalid=True,
        )
    else:
        group = event.get("merge_group")
        require(isinstance(group, dict), "event-queue-custody", invalid=True)
        assert isinstance(group, dict)
        require(
            run.get("head_sha") == context.tested_sha
            and group.get("head_sha") == context.tested_sha
            and group.get("base_sha") == context.policy_sha
            and group.get("base_ref") == "refs/heads/main",
            "event-queue-custody",
            invalid=True,
        )
        actual = normalized_ref(ref)
        require(
            normalized_ref(group.get("head_ref")) == actual
            and normalized_ref(run.get("head_branch")) == actual,
            "run-queue-ref",
            invalid=True,
        )
        match = re.fullmatch(r"gh-readonly-queue/main/pr-([1-9][0-9]*)-([0-9a-f]{40})", actual)
        require(match is not None, "unsupported-queue-shape")
        assert match is not None
        require(
            int(match[1]) == repair.number and match[2] == repair.base, "unsupported-queue-repair"
        )


def public_custody(
    api: Metadata, context: AdmissionContext, repair: Repair, event: dict[str, Any], consumer: Git
) -> tuple[str, Callable[[], None]]:
    prefix = f"/repos/{repair.repository}"
    metadata = api.get(prefix)
    require(
        repository_identity(metadata, repair.repository), "public-repository-custody", invalid=True
    )
    require(
        metadata.get("private") is False and metadata.get("visibility", "public") == "public",
        "public-metadata-unavailable",
    )
    require(metadata.get("default_branch") == "main", "unsupported-default-branch")
    initial = api.get(f"{prefix}/pulls/{repair.number}")
    source = pull_identity(initial, repair)
    tested = api.get(f"{prefix}/git/commits/{context.tested_sha}")
    head = api.get(f"{prefix}/git/commits/{source}")
    require(
        tested.get("sha") == context.tested_sha and head.get("sha") == source,
        "public-commit-custody",
        invalid=True,
    )
    tested_tree, tested_parents = consumer.commit(context.tested_sha)
    for value, tree in ((tested, tested_tree), (head, head.get("tree", {}).get("sha"))):
        require(
            isinstance(value.get("tree"), dict) and revision(value["tree"].get("sha")) == tree,
            "public-tree-custody",
            invalid=True,
        )
    require(
        isinstance(tested.get("parents"), list)
        and [p.get("sha") for p in tested["parents"]] == tested_parents,
        "public-parent-custody",
        invalid=True,
    )
    if context.event == "merge_group":
        require(tested_parents == [repair.base], "unsupported-queue-parent-shape")
        require(head["tree"]["sha"] == tested_tree, "queue-current-head-tree-changed")
    else:
        pull = event["pull_request"]
        require(pull["head"].get("sha") == source, "pr-current-head-changed")
        require(tested_parents == [repair.base, source], "pr-merge-custody", invalid=True)
        require(initial.get("merge_commit_sha") == context.tested_sha, "pr-current-merge-changed")
    comparison = api.get(f"{prefix}/compare/{repair.base}...{source}")
    require(
        isinstance(comparison.get("base_commit"), dict)
        and comparison["base_commit"].get("sha") == repair.base,
        "comparison-base-custody",
        invalid=True,
    )
    require(
        isinstance(comparison.get("merge_base_commit"), dict)
        and comparison["merge_base_commit"].get("sha") == repair.base
        and comparison.get("status") == "ahead",
        "source-base-not-ancestor",
    )
    # GitHub compare returns the head identity in the final commit when its list
    # is complete. Refuse truncation rather than inferring identity from a URL.
    commits = comparison.get("commits")
    require(
        isinstance(commits, list)
        and bool(commits)
        and type(comparison.get("total_commits")) is int
        and comparison["total_commits"] == len(commits)
        and len(commits) <= 100
        and commits[-1].get("sha") == source,
        "comparison-endpoint-unavailable",
    )

    def recheck() -> None:
        require(
            pull_identity(api.get(f"{prefix}/pulls/{repair.number}"), repair) == source,
            "pr-current-head-changed",
        )

    return source, recheck


# The integration is generated from exact A bytes. The same fixed transformation
# is used during producer closure: arbitrary waiter edits cannot be exempted.
WORKFLOW_OUTPUTS = """      admission_evidence_kind:
        description: Exact-A receipt or computed read-only zero-age-delta evidence.
        value: ${{ jobs.check.outputs.admission_evidence_kind }}
      admission_proof_implementation_sha:
        description: Actual live proof implementation S; empty for receipt evidence.
        value: ${{ jobs.check.outputs.admission_proof_implementation_sha }}
"""
JOB_OUTPUTS = (
    """      admission_evidence_kind: ${{ steps.result.outputs.admission_evidence_kind }}
      admission_proof_implementation_sha: """
    "${{ steps.result.outputs.admission_proof_implementation_sha }}\n"
)
PROOF_STEPS = """      - name: Compute the bounded read-only zero-age-delta proof
        if: steps.policy.outputs.required == 'true'
        id: zero
        env:
          GH_TOKEN: ${{ github.token }}
          CANDIDATE_SHA: ${{ inputs.candidate_sha }}
          AUTOMATION_SHA: ${{ inputs.automation_sha }}
          POLICY_SHA: ${{ inputs.policy_sha }}
          PR_BASE_SHA: ${{ github.event.pull_request.base.sha }}
          QUEUE_BASE_SHA: ${{ github.event.merge_group.base_sha }}
        shell: bash
        run: |
          set -euo pipefail
          PYTHONPATH="${GITHUB_WORKSPACE}/_proposed_automation/automation/src" \\
            uv run --locked --project _automation/automation python -P -B \\
            -m boxferry_ci.zero_age_delta \\
            --repository "${GITHUB_REPOSITORY}" --event-name "${GITHUB_EVENT_NAME}" \\
            --candidate-sha "${CANDIDATE_SHA}" --tested-sha "${GITHUB_SHA}" \\
            --automation-sha "${AUTOMATION_SHA}" --policy-sha "${POLICY_SHA}" \\
            --pr-base-sha "${PR_BASE_SHA}" --queue-base-sha "${QUEUE_BASE_SHA}" \\
            --implementation-root _proposed_automation --authority-root _automation \\
            --policy-root _policy \\
            --event-path "${GITHUB_EVENT_PATH}" --run-id "${GITHUB_RUN_ID}" \\
            --run-attempt "${GITHUB_RUN_ATTEMPT}" --ref "${GITHUB_REF}" \\
            --github-output "${GITHUB_OUTPUT}" --summary "${GITHUB_STEP_SUMMARY}"
"""
RESULT_ENV = """          ZERO_STATUS: ${{ steps.zero.outputs.status }}
          PROOF_SHA: ${{ steps.zero.outputs.proof_implementation_sha }}
"""
RESULT_OUTPUTS = """          if [[ "${ZERO_STATUS}" == proved-zero ]]; then
            printf 'admission_evidence_kind=read-only-zero-age-delta\\n' >> "${GITHUB_OUTPUT}"
            printf 'admission_proof_implementation_sha=%s\\n' "${PROOF_SHA}" >> "${GITHUB_OUTPUT}"
          elif [[ "${ADMISSION_REQUIRED}" == true ]]; then
            printf 'admission_evidence_kind=exact-A-service-receipt\\n' >> "${GITHUB_OUTPUT}"
          else
            printf 'admission_evidence_kind=not-applicable\\n' >> "${GITHUB_OUTPUT}"
          fi
"""


def expected_waiter(original: bytes) -> bytes:
    text = original.decode("utf-8")
    changes = (
        ("permissions:\n", WORKFLOW_OUTPUTS + "permissions:\n"),
        ("    steps:\n", JOB_OUTPUTS + "    steps:\n"),
        (
            "      # This artifact is an untrusted",
            PROOF_STEPS + "      # This artifact is an untrusted",
        ),
        (
            "      - name: Read the exact candidate receipt under A's policy\n"
            "        if: steps.policy.outputs.required == 'true'\n",
            "      - name: Read the exact candidate receipt under A's policy\n"
            "        if: steps.policy.outputs.required == 'true'"
            " && steps.zero.outputs.status != 'proved-zero'\n",
        ),
        (
            "          ADMISSION_REQUIRED: ${{ steps.policy.outputs.required }}\n",
            "          ADMISSION_REQUIRED: ${{ steps.policy.outputs.required }}\n" + RESULT_ENV,
        ),
    )
    for old, new in changes:
        require(text.count(old) == 1, "waiter-integration-anchor")
        text = text.replace(old, new, 1)
    return (text + RESULT_OUTPUTS).encode()


def verify_producer(before: Tree, after: Tree) -> dict[str, Any]:
    require(
        regular(after, WAITER).content == expected_waiter(regular(before, WAITER).content),
        "producer-waiter-shape",
    )
    for path, (size, digest) in REQUIRED_604.items():
        item = regular(after, path)
        require(len(item.content) == size and item.digest == digest, "required-604-postimage")
    fixture_paths: set[str] = set()
    for repair in REGISTRY.values():
        for kind, digest in (
            ("manifest", "3ea3aae7a5d63a0c305efb60fad4b2390992a993b8eece956ab3aa8ada465389"),
            ("policy", repair.policy),
        ):
            path = f"{FIXTURES}{repair.name}/{kind}.txt"
            require(regular(after, path).digest == digest, "reviewed-template-hash")
            fixture_paths.add(path)
    for repair in REGISTRY.values():
        for kind, phase, digest in (
            ("caller", "before", repair.caller_before),
            ("caller", "after", repair.caller_after),
            ("contract", "before", repair.test_before),
            ("contract", "after", repair.test_after),
        ):
            template(after, repair, kind, phase, digest)
            fixture_paths.add(f"{FIXTURES}{repair.name}/{kind}-{phase}.txt")
    allowed = set(IMPLEMENTATION_PATHS) | fixture_paths | set(REQUIRED_604) | {WAITER}
    for path in IMPLEMENTATION_PATHS:
        item = regular(after, path)
        require(len(item.content) <= 200000, "producer-implementation-budget")
    directories = {str(Path(path).parent) for path in allowed}
    for path in list(directories):
        directories.update(str(p) for p in Path(path).parents if str(p) != ".")
    changed = changed_paths(before, after)
    require(changed <= allowed | directories, "producer-extra-tree-delta")
    for path in changed & directories:
        require(
            path not in before.entries and after.entries[path].mode == 0o40000,
            "producer-directory-delta",
        )
    require(
        regular(after, PROOF_PATH).content == Path(__file__).read_bytes(),
        "executing-proof-source",
        invalid=True,
    )
    # Every already-loaded sibling module must belong to held S. Unchanged policy
    # modules are A-exact by the full closure, not by import-cache assumptions.
    root = Path(__file__).resolve().parents[3]
    for name, module in list(sys.modules.items()):
        if name == "boxferry_ci" or name.startswith("boxferry_ci."):
            require(module.__file__ is not None, "executing-module-origin", invalid=True)
            assert module.__file__ is not None
            file = Path(module.__file__).resolve()
            require(file.is_relative_to(root), "executing-module-origin", invalid=True)
            path = file.relative_to(root).as_posix()
            require(
                regular(after, path).content == file.read_bytes(),
                "executing-module-custody",
                invalid=True,
            )
    return {
        "producer_A_tree": before.oid,
        "producer_S_tree": after.oid,
        "producer_A_digest": before.digest,
        "producer_S_digest": after.digest,
        "producer_changed_paths": sorted(changed),
        "required_604": "exact-postimages",
    }


# Run in a fresh -I -B interpreter; only held A is inserted into its isolated path.
# The child does not import S and verifies every loaded package module afterwards.
CONFIRMER = r"""
import copy, hashlib, json, sys
from pathlib import Path
assert not any(n == 'boxferry_ci' or n.startswith('boxferry_ci.') for n in sys.modules)
source, base, head, inventory = map(Path, sys.argv[1:])
expected = json.loads(inventory.read_text())
sys.path.insert(0, str(source / 'automation/src'))
from boxferry_ci import dependencies as D, operational as O
calls = {'fetch': 0, 'fetch_request': 0}
def fetch(*args, **kwargs):
    calls['fetch'] += 1
    raise RuntimeError('zero-proof-acquisition-forbidden')
def fetch_request(*args, **kwargs):
    calls['fetch_request'] += 1
    raise RuntimeError('zero-proof-acquisition-forbidden')
state = {'schema_version': 1, 'first_seen': {}}
initial = copy.deepcopy(state)
before = D.check_dependencies(base, base, state=state, context=None,
    fetch=fetch, fetch_request=fetch_request)
after = D.check_dependencies(base, head, state=state, context=None,
    fetch=fetch, fetch_request=fetch_request)
old_features, new_features = O._features(base), O._features(head)
origins = {}
for name, module in list(sys.modules.items()):
    if name == 'boxferry_ci' or name.startswith('boxferry_ci.'):
        path = Path(module.__file__).resolve()
        relative = path.relative_to(source).as_posix()
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        assert expected[relative] == digest
        origins[name] = {'path': relative, 'sha256': digest}
valid = all((before.get('status') == 'pass', after.get('status') == 'pass',
    before.get('first_party_exempt') is False, after.get('first_party_exempt') is False,
    before.get('results') == [], after.get('results') == [],
    before.get('state') == initial, after.get('state') == initial, state == initial,
    calls == {'fetch': 0, 'fetch_request': 0},
    before.get('operational_coverage_gaps') == after.get('operational_coverage_gaps'),
    old_features[2] == new_features[2]))
print(json.dumps({'confirmed': valid, 'calls': calls,
    'introduced_identities': len(after.get('results', [])),
    'first_party_exempt': after.get('first_party_exempt'),
    'state_unchanged_empty': state == initial,
    'coverage_before': before.get('operational_coverage_gaps'),
    'coverage_after': after.get('operational_coverage_gaps'),
    'opaque_feature_errors_before': old_features[2], 'opaque_feature_errors_after': new_features[2],
    'origins': origins}, sort_keys=True))
"""


def materialize(tree: Tree, root: Path, *, source_only: bool = False) -> dict[str, str]:
    inventory: dict[str, str] = {}
    root.mkdir()
    for path, item in sorted(tree.entries.items()):
        if source_only and not path.startswith("automation/src/"):
            continue
        destination = root / path
        if item.mode == 0o40000:
            destination.mkdir(parents=True, exist_ok=True)
            continue
        require(item.mode in {0o100644, 0o100755}, "nonregular-confirmation-input")
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(item.content)
        destination.chmod(item.mode & 0o777)
        inventory[path] = item.digest
    return inventory


def confirm_exact_A(authority: Tree, before: Tree, after: Tree, deadline: float) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="boxferry-zero-proof-") as temporary:
        root = Path(temporary)
        inventory = materialize(authority, root / "A", source_only=True)
        materialize(before, root / "before")
        materialize(after, root / "after")
        manifest = root / "inventory.json"
        manifest.write_text(json.dumps(inventory), encoding="utf-8")
        environment = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("PYTHON", "GIT_")) and k not in {"GH_TOKEN", "GITHUB_TOKEN"}
        }
        environment.update(PYTHONDONTWRITEBYTECODE="1")
        process: subprocess.Popen[bytes] | None = None
        selector: selectors.BaseSelector | None = None
        try:
            with _blocked_cancellation() as previous_signals:

                def restore_signals() -> None:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_signals)

                process = subprocess.Popen(
                    [
                        sys.executable,
                        "-I",
                        "-B",
                        "-c",
                        CONFIRMER,
                        str(root / "A"),
                        str(root / "before"),
                        str(root / "after"),
                        str(manifest),
                    ],
                    cwd=root,
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    preexec_fn=restore_signals,
                )
            selector = selectors.DefaultSelector()
            assert process.stdout is not None
            selector.register(process.stdout, selectors.EVENT_READ)
            raw = bytearray()
            child_deadline = min(deadline, time.monotonic() + 20)
            while True:
                remaining = child_deadline - time.monotonic()
                require(remaining > 0 and selector.select(remaining), "confirmation-time-budget")
                chunk = os.read(process.stdout.fileno(), min(65536, 1024 * 1024 + 1 - len(raw)))
                if not chunk:
                    break
                raw.extend(chunk)
                require(len(raw) <= 1024 * 1024, "confirmation-byte-budget")
            require(
                process.wait(timeout=max(0.01, deadline - time.monotonic())) == 0,
                "exact-A-confirmation-unavailable",
            )
        finally:
            with _blocked_cancellation():
                try:
                    if process is not None:
                        if process.poll() is None:
                            process.kill()
                        process.wait(timeout=2)
                finally:
                    if process is not None and process.stdout is not None:
                        process.stdout.close()
                    if selector is not None:
                        selector.close()
        try:
            result = json.loads(raw)
        except (UnicodeError, ValueError) as error:
            raise ProofError("exact-A-confirmation-malformed") from error
        require(
            isinstance(result, dict) and result.get("confirmed") is True,
            "exact-A-confirmation-refused",
        )
        result["opaque_inputs"] = [
            {"path": path, "mode": item.mode, "sha256": item.digest, "graph_qualified": False}
            for path, item in sorted(before.entries.items())
            if path.startswith(".devcontainer/") and item.mode != 0o40000
        ]
        return result


def prove_zero(options: argparse.Namespace) -> dict[str, Any]:
    """Compute one bounded proof; never read candidate-produced proof/state files."""
    deadline = time.monotonic() + 65
    context = AdmissionContext(
        event=options.event_name,
        repository=options.repository,
        candidate_sha=options.candidate_sha,
        tested_sha=options.tested_sha,
        automation_sha=options.automation_sha,
        policy_sha=options.policy_sha,
        pr_base_sha=options.pr_base_sha,
        queue_base_sha=options.queue_base_sha,
    )
    report: dict[str, Any] = {
        "schema": 1,
        "kind": "read-only-zero-age-delta",
        "repository": context.repository,
        "event": context.event,
        "base_sha": context.policy_sha,
        "tested_sha": context.tested_sha,
        "policy_implementation_sha": AUTHORITY,
        "proof_implementation_sha": context.automation_sha,
        "run_id": options.run_id,
        "run_attempt": options.run_attempt,
    }
    readers: list[Git] = []
    acquisition: tempfile.TemporaryDirectory[str] | None = None

    def held(path: Path) -> Git:
        with _blocked_cancellation() as inherited_signals:
            reader = Git(path, deadline, child_signal_mask=inherited_signals)
            readers.append(reader)
        return reader

    try:
        for name, actual in (
            ("GITHUB_REPOSITORY", context.repository),
            ("GITHUB_EVENT_NAME", context.event),
            ("GITHUB_SHA", context.tested_sha),
            ("GITHUB_RUN_ID", str(options.run_id)),
            ("GITHUB_RUN_ATTEMPT", str(options.run_attempt)),
            ("GITHUB_REF", options.ref),
            ("GITHUB_EVENT_PATH", str(options.event_path)),
        ):
            require(os.environ.get(name) == actual, "runner-context-custody", invalid=True)
        require(options.run_id > 0 and options.run_attempt > 0, "run-attempt-custody", invalid=True)
        implementation, policy = held(options.implementation_root), held(options.policy_root)

        def admission_reader(root: Path, args: tuple[str, ...], limit: int) -> bytes:
            reader = implementation if root == options.implementation_root else policy
            return reader.small(args, limit)

        selection = resolve_admission(
            context, options.implementation_root, options.policy_root, reader=admission_reader
        )
        report["policy_implementation_sha"] = selection.policy_automation_sha
        require(selection.status == "required", "non-admission-event")
        repair = REGISTRY.get(context.repository)
        require(repair is not None, "unregistered-repository")
        assert repair is not None
        require(context.policy_sha == repair.base, "unregistered-base")
        require(selection.policy_automation_sha == AUTHORITY, "unsupported-authority-A")
        acquisition = tempfile.TemporaryDirectory(prefix="boxferry-zero-objects-")
        consumer = held(
            acquire_consumer(Path(acquisition.name), repair, context.tested_sha, deadline)
        )
        before, after = policy.tree(repair.base), consumer.tree(context.tested_sha)
        # Unknown repairs exit before producer-tree or public/API acquisition.
        require(
            regular(before, CALLER).digest == repair.caller_before
            and regular(before, CONTRACT, repair.test_mode).digest == repair.test_before,
            "unregistered-repair",
        )
        require(
            regular(after, CONTRACT, repair.test_mode).digest == repair.test_after
            and changed_paths(before, after) <= set(SOURCE_PATHS) | {CONTRACT},
            "unregistered-repair-variant",
        )
        with options.event_path.open("rb") as stream:
            raw = stream.read(1024 * 1024 + 1)
        require(len(raw) <= 1024 * 1024, "event-byte-budget", invalid=True)
        event = json.loads(raw)
        require(isinstance(event, dict), "event-malformed", invalid=True)
        api, public = Metadata(deadline), Metadata(deadline, public=True)
        run = api.get(
            f"/repos/{context.repository}/actions/runs/{options.run_id}/attempts/{options.run_attempt}"
        )
        bind_context(context, event, run, repair, options.run_id, options.run_attempt, options.ref)
        require(canonical(api) == context.automation_sha, "executing-source-not-live-canonical")
        authority_reader = held(options.authority_root)
        require(authority_reader.head() == AUTHORITY, "authority-checkout-custody", invalid=True)
        authority = authority_reader.tree(AUTHORITY)
        producer = implementation.tree(context.automation_sha)
        report.update(verify_producer(authority, producer))
        report.update(match_repair_shape(before, after, producer, repair, context.automation_sha))
        source, reread_pull = public_custody(public, context, repair, event, consumer)
        report.update(
            source_head_sha=source,
            source_provenance=(
                "current-pr-head-with-complete-tested-tree"
                if context.event == "merge_group"
                else "exact-pr-merge-parent"
            ),
        )
        report["confirmation"] = confirm_exact_A(authority, before, after, deadline)
        reread_pull()
        require(canonical(api) == context.automation_sha, "canonical-source-advanced")
        report.update(
            status="proved-zero",
            reason="complete-reviewed-repair-zero-age-delta",
            public_metadata_requests=public.requests,
            source_metadata_requests=api.requests,
        )
    except AdmissionError:
        report.update(status="invalid-context", reason="admission-context-invalid")
    except ProofError as error:
        report.update(
            status="invalid-context" if error.invalid else "needs-service", reason=error.reason
        )
        if error.details:
            report["diagnostics"] = error.details
    except (UnicodeError, ValueError, TypeError, KeyError, AttributeError):
        report.update(status="invalid-context", reason="malformed-context-or-custody-data")
    except (OSError, subprocess.SubprocessError):
        report.update(status="needs-service", reason="bounded-proof-input-unavailable")
    finally:
        with _blocked_cancellation():
            for reader in readers:
                reader.close()
            if acquisition is not None:
                acquisition.cleanup()
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for key in (
        "repository",
        "event-name",
        "candidate-sha",
        "tested-sha",
        "automation-sha",
        "policy-sha",
        "ref",
    ):
        parser.add_argument(f"--{key}", required=True)
    for key in ("pr-base-sha", "queue-base-sha"):
        parser.add_argument(f"--{key}", default="")
    for key in (
        "implementation-root",
        "authority-root",
        "policy-root",
        "event-path",
    ):
        parser.add_argument(f"--{key}", type=Path, required=True)
    for key in ("run-id", "run-attempt"):
        parser.add_argument(f"--{key}", type=int, required=True)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--summary", type=Path)
    options = parser.parse_args(argv)

    # A real-time deadline also bounds slow HTTP bodies, not just idle sockets.
    def expired(signum: int, frame: object) -> None:
        raise ProofError("proof-time-budget")

    def cancelled(signum: int, frame: object) -> None:
        raise ProofCancelled(signum)

    previous = signal.signal(signal.SIGALRM, expired)
    previous_term = signal.signal(signal.SIGTERM, cancelled)
    previous_int = signal.signal(signal.SIGINT, cancelled)
    signal.setitimer(signal.ITIMER_REAL, 65)
    try:
        report = prove_zero(options)
    except ProofCancelled as cancellation:
        return 128 + cancellation.signum
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
        signal.signal(signal.SIGTERM, previous_term)
        signal.signal(signal.SIGINT, previous_int)
    if options.github_output:
        with options.github_output.open("a", encoding="utf-8") as stream:
            for key in ("status", "reason"):
                stream.write(f"{key}={report[key]}\n")
            if report["status"] == "proved-zero":
                stream.write(f"proof_implementation_sha={report['proof_implementation_sha']}\n")
    if options.summary:
        with options.summary.open("a", encoding="utf-8") as stream:
            stream.write("### Computed read-only zero-age-delta proof\n\n")
            stream.write(
                "This computation creates no trusted-service receipt or persisted age state.\n\n"
            )
            stream.write("```json\n" + json.dumps(report, sort_keys=True, indent=2) + "\n```\n")
    print(json.dumps(report, sort_keys=True))
    return 2 if report["status"] == "invalid-context" else 0


if __name__ == "__main__":
    raise SystemExit(main())
