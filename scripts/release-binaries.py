#!/usr/bin/env python3
"""Build, bind and validate the six portable CLI release archives (no publication)."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import zlib


ROOT = Path(__file__).resolve().parent.parent
TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "i686-unknown-linux-musl",
    "armv7-unknown-linux-musleabihf",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)
ELF = {TARGETS[0]: (2, 62), TARGETS[1]: (2, 183), TARGETS[2]: (1, 3), TARGETS[3]: (1, 40)}
MACH = {TARGETS[4]: 0x01000007, TARGETS[5]: 0x0100000C}
EMULATORS = {TARGETS[2]: "qemu-i386", TARGETS[3]: "qemu-arm"}
MAX_BINARY = 256 * 1024 * 1024
MAX_ARCHIVE = MAX_BINARY + 4 * 1024 * 1024
MAX_TAR = MAX_ARCHIVE + 1024 * 1024
MAX_TEXT = 1024 * 1024
MAX_OUTPUT = 1024 * 1024
COMMAND_TIMEOUT = 30
SMOKE_TESTS = ["version", "capabilities-sixteen-routes", "compose-quadlet-compose"]
MEMBERS = {"boxferry": 0o755, "LICENSE": 0o644, "README.md": 0o644, "BUILD-INFO.json": 0o644}


class ArtifactError(Exception):
    """A bounded, privacy-safe artifact contract failure."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ArtifactError(message)


def binding(version: str, revision: str, run_id: str) -> None:
    core = r"(?:0|[1-9][0-9]*)"
    identifier = r"(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
    pattern = rf"{core}\.{core}\.{core}(?:-{identifier}(?:\.{identifier})*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    require(len(version) <= 128 and bool(re.fullmatch(pattern, version)), "invalid release version")
    require(bool(re.fullmatch(r"[0-9a-f]{40}", revision)), "revision must be a full lowercase Git SHA")
    require(bool(re.fullmatch(r"[1-9][0-9]{0,19}", run_id)), "run ID must be a positive decimal integer")


def archive_name(version: str, target: str) -> str:
    require(target in TARGETS, "unsupported binary target")
    return f"boxferry-{version}-{target}.tar.gz"


def safe_path(path: Path) -> Path:
    """Reject symlinks in every existing path component, including parents."""
    path = Path(os.path.abspath(path))
    for part in reversed((path, *path.parents)):
        try:
            require(not stat.S_ISLNK(part.lstat().st_mode), "symlink paths are forbidden")
        except FileNotFoundError:
            pass
    return path


def read_regular(path: Path, limit: int) -> bytes:
    path = safe_path(path)
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode), "input must be a regular file")
        require(0 < info.st_size <= limit, "input file size exceeds its bound or is empty")
        data = stream.read(limit + 1)
        require(len(data) == info.st_size and len(data) <= limit, "input changed or exceeded its bound")
        return data


def directory(path: Path, *, create: bool = False, empty: bool = False) -> Path:
    path = safe_path(path)
    if create:
        path.mkdir(parents=True, exist_ok=True)
    require(stat.S_ISDIR(path.lstat().st_mode), "expected a directory")
    if empty:
        require(not any(path.iterdir()), "output directory must be empty")
    return path


def write_new(path: Path, data: bytes) -> None:
    path = safe_path(path)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
    except BaseException:
        path.unlink()
        raise


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode("ascii")


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    result: dict = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON metadata key")
        result[key] = value
    return result


def parse_json(data: bytes) -> object:
    require(len(data) <= MAX_TEXT, "JSON metadata exceeds its bound")
    try:
        value = json.loads(data, object_pairs_hook=unique_object)
        pending = [(value, 0)]
        while pending:
            node, depth = pending.pop()
            require(depth <= 32, "JSON metadata nesting exceeds its bound")
            if isinstance(node, dict):
                pending.extend((child, depth + 1) for child in node.values())
            elif isinstance(node, list):
                pending.extend((child, depth + 1) for child in node)
        return value
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ArtifactError("invalid JSON metadata") from error


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def region(data: bytes, offset: int, size: int) -> bytes:
    require(0 <= offset <= len(data) and 0 <= size <= len(data) - offset, "binary header points outside file")
    return data[offset:offset + size]


def validate_binary(data: bytes, target: str) -> None:
    require(0 < len(data) <= MAX_BINARY, "binary size exceeds its bound")
    require(target in TARGETS, "unsupported binary target")
    if target in ELF:
        elf_class, machine = ELF[target]
        require(region(data, 0, 7) == b"\x7fELF" + bytes([elf_class, 1, 1]), "ELF class, byte order or version does not match target")
        fmt = "<HHIIIIIHHHHHH" if elf_class == 1 else "<HHIQQQIHHHHHH"
        values = struct.unpack(fmt, region(data, 16, struct.calcsize(fmt)))
        kind, cpu, version, _, phoff, _, flags, ehsize, phsize, phnum, _, _, _ = values
        expected_header, expected_program = (52, 32) if elf_class == 1 else (64, 56)
        require(kind in (2, 3) and cpu == machine and version == 1 and ehsize == expected_header, "ELF executable header does not match target")
        require(0 < phnum < 65535 and phsize == expected_program and phoff >= expected_header, "invalid ELF program header table")
        region(data, phoff, phsize * phnum)
        if target == TARGETS[3]:
            require(flags & 0xFF000000 == 0x05000000 and flags & 0x400 != 0 and flags & 0x200 == 0, "ARM binary must use EABI5 hard-float")
        executable_load = False
        for index in range(phnum):
            program = region(data, phoff + phsize * index, phsize)
            if elf_class == 1:
                ptype, offset, _, _, filesz, memsz, pflags, _ = struct.unpack("<IIIIIIII", program)
            else:
                ptype, pflags, offset, _, _, filesz, memsz, _ = struct.unpack("<IIQQQQQQ", program)
            require(filesz <= memsz, "invalid ELF segment sizes")
            segment = region(data, offset, filesz)
            require(ptype != 3, "Linux executable has a dynamic interpreter")
            executable_load |= ptype == 1 and bool(pflags & 1) and filesz > 0
            if ptype == 2:
                entry = 8 if elf_class == 1 else 16
                require(filesz > 0 and filesz % entry == 0, "invalid ELF dynamic table")
                terminated = False
                for position in range(0, filesz, entry):
                    tag, _ = struct.unpack("<iI" if elf_class == 1 else "<qQ", segment[position:position + entry])
                    require(tag != 1, "Linux executable depends on an external shared library")
                    if tag == 0:
                        terminated = True
                        require(not any(segment[position + entry:]), "nonzero entries after ELF dynamic terminator")
                        break
                require(terminated, "unterminated ELF dynamic table")
        require(executable_load, "ELF binary lacks an executable load segment")
    else:
        magic, cpu, subtype, kind, ncmds, sizeofcmds, _, reserved = struct.unpack("<IIIIIIII", region(data, 0, 32))
        require(magic == 0xFEEDFACF and cpu == MACH[target] and kind == 2 and reserved == 0, "Mach-O executable architecture does not match target")
        require(subtype & 0xFFFFFF == (3 if target == TARGETS[4] else 0), "Mach-O CPU subtype does not match baseline target")
        require(0 < ncmds <= 65535 and sizeofcmds >= ncmds * 8, "invalid Mach-O command table")
        region(data, 32, sizeofcmds)
        position = 32
        executable_segment = False
        for _ in range(ncmds):
            command, size = struct.unpack("<II", region(data, position, 8))
            require(size >= 8 and size % 8 == 0 and position + size <= 32 + sizeofcmds, "invalid Mach-O load command")
            if command == 0x19:
                require(size >= 72, "truncated Mach-O segment command")
                _, _, _, _, _, offset, filesz, _, initprot, nsects, _ = struct.unpack("<II16sQQQQiiII", region(data, position, 72))
                require(size == 72 + 80 * nsects, "invalid Mach-O section table")
                region(data, offset, filesz)
                executable_segment |= bool(initprot & 4) and filesz > 0
            position += size
        require(position == 32 + sizeofcmds and executable_segment, "Mach-O binary lacks a valid executable segment")


def execution(target: str, emulator: Path | None) -> dict:
    selected = EMULATORS.get(target)
    if selected:
        require(emulator is not None and emulator.name == selected, "32-bit targets require their explicitly selected QEMU user emulator")
        path = safe_path(emulator)
        require(stat.S_ISREG(path.stat().st_mode) and os.access(path, os.X_OK), "QEMU emulator must be an executable regular file")
        return {"mode": "qemu-user", "emulator": selected}
    require(emulator is None, "native targets must not select an emulator")
    return {"mode": "native", "emulator": None}


def run_bounded(command: list[str], cwd: Path) -> bytes:
    """Drain both pipes without unbounded communicate() buffers or inherited secrets."""
    environment = {"PATH": os.defpath, "HOME": str(cwd), "TMPDIR": str(cwd), "LANG": "C", "LC_ALL": "C"}
    process = subprocess.Popen(command, cwd=cwd, env=environment, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    output = bytearray()
    total = 0
    deadline = time.monotonic() + COMMAND_TIMEOUT
    try:
        with selectors.DefaultSelector() as selector:
            assert process.stdout is not None and process.stderr is not None
            for pipe in (process.stdout, process.stderr):
                os.set_blocking(pipe.fileno(), False)
                selector.register(pipe, selectors.EVENT_READ)
            while selector.get_map():
                require(time.monotonic() < deadline, "binary smoke command timed out")
                for key, _ in selector.select(min(0.1, max(0, deadline - time.monotonic()))):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    total += len(chunk)
                    require(total <= MAX_OUTPUT, "binary smoke output exceeded its bound")
                    if key.fileobj is process.stdout:
                        output.extend(chunk)
            remaining = deadline - time.monotonic()
            require(remaining > 0, "binary smoke command timed out")
            try:
                result = process.wait(timeout=remaining)
            except subprocess.TimeoutExpired as error:
                raise ArtifactError("binary smoke command timed out") from error
            require(result == 0, "binary smoke command failed")
    finally:
        # Kill the session even if the direct child exited but left pipe-holding children.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        for pipe in (process.stdout, process.stderr):
            if pipe is not None:
                pipe.close()
    return bytes(output)


def smoke(binary: Path, target: str, version: str, emulator: Path | None) -> dict:
    mode = execution(target, emulator)
    guest = str(safe_path(binary))
    # The helper's pinned tonistiigi/binfmt QEMU flavor preserves guest argv0:
    # supply it explicitly after the guest filename; no probing or fallback.
    command = [str(safe_path(emulator)), guest, guest] if emulator is not None else [guest]
    require(os.access(binary, os.X_OK), "binary input must be executable")
    with tempfile.TemporaryDirectory(prefix="boxferry-binary-smoke-") as temporary:
        # macOS's OS-owned /var and /tmp aliases must not make our own fresh
        # scratch directory look like an untrusted symlink input.
        work = Path(temporary).resolve()
        require(run_bounded(command + ["--version"], work).strip() == f"boxferry {version}".encode(), "binary version does not match release")
        capabilities = parse_json(run_bounded(command + ["capabilities", "--console-format", "json"], work))
        require(isinstance(capabilities, dict) and type(capabilities.get("schema_version")) is int and capabilities["schema_version"] == 1, "invalid binary capability schema")
        routes = capabilities.get("routes")
        require(isinstance(routes, list) and len(routes) == 16 and all(isinstance(route, dict) for route in routes), "binary must expose all sixteen default-feature routes")
        require(all(isinstance(route.get(key), str) for route in routes for key in ("input_type", "output_type")), "invalid binary route metadata")
        require({(route.get("input_type"), route.get("output_type")) for route in routes} == {(a, b) for a in ("compose", "quadlet", "podman", "docker") for b in ("compose", "quadlet", "podman", "docker")}, "binary route catalogue is incomplete")
        source = work / "compose.yaml"
        source.write_text("---\nname: release-smoke\nservices:\n  web:\n    image: docker.io/library/alpine:3.21\n", encoding="utf-8")
        quadlet, compose = work / "quadlet", work / "compose"
        run_bounded(command + ["convert", "compose", "quadlet", "--input-file", str(source), "--output-directory", str(quadlet)], work)
        require(names(directory(quadlet), 1) == {"web.container"}, "Compose-to-Quadlet smoke emitted an unexpected file set")
        unit = read_regular(quadlet / "web.container", MAX_TEXT).decode("utf-8")
        require("[Container]" in unit and re.search(r"(?m)^Image=docker\.io/library/alpine:3\.21\s*$", unit) is not None, "Compose-to-Quadlet smoke lost the expected image")
        run_bounded(command + ["convert", "quadlet", "compose", "--input-directory", str(quadlet), "--application-name", "release-smoke", "--output-directory", str(compose)], work)
        require(names(directory(compose), 1) == {"compose.yaml"}, "Quadlet-to-Compose smoke emitted an unexpected file set")
        document = read_regular(compose / "compose.yaml", MAX_TEXT).decode("utf-8")
        require(re.search(r"(?m)^\s+web:\s*$", document) is not None and re.search(r"(?m)^\s+image:\s*['\"]?docker\.io/library/alpine:3\.21['\"]?\s*$", document) is not None, "Quadlet-to-Compose smoke lost the expected service or image")
    return mode


def build_info(args: argparse.Namespace, binary: bytes, mode: dict) -> dict:
    return {"schema_version": 1, "version": args.version, "target": args.target, "revision": args.revision,
            "github_run_id": args.run_id, "binary_sha256": sha256(binary), "execution": mode, "smoke_tests": SMOKE_TESTS}


def make_archive(version: str, target: str, files: dict[str, bytes]) -> bytes:
    output = io.BytesIO()
    with gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            root = archive_name(version, target).removesuffix(".tar.gz")
            for name, mode in MEMBERS.items():
                member = tarfile.TarInfo(f"{root}/{name}")
                member.mode = mode
                member.size = len(files[name])
                member.mtime = 0
                archive.addfile(member, io.BytesIO(files[name]))
    return output.getvalue()


def validate_archive(data: bytes, args: argparse.Namespace, target: str) -> dict:
    require(0 < len(data) <= MAX_ARCHIVE, "archive exceeds its bound")
    require(data[:4] == b"\x1f\x8b\x08\x00" and data[4:8] == b"\0" * 4, "archive gzip header must be deterministic without private names")
    try:
        decompressor = zlib.decompressobj(wbits=31)
        unpacked = decompressor.decompress(data, MAX_TAR + 1)
        require(len(unpacked) <= MAX_TAR, "decompressed archive exceeds its bound")
        require(decompressor.eof and not decompressor.unused_data and not decompressor.unconsumed_tail, "archive must contain exactly one complete gzip stream")
        root = archive_name(args.version, target).removesuffix(".tar.gz")
        expected = {f"{root}/{name}" for name in MEMBERS}
        files: dict[str, bytes] = {}
        with tarfile.open(fileobj=io.BytesIO(unpacked), mode="r:") as archive:
            # Iterate rather than getmembers(), bounding malicious zero-length member floods.
            end = 0
            for member in archive:
                require(member.name in expected and member.name not in files, "archive member set contains an unexpected or duplicate path")
                name = member.name.rsplit("/", 1)[1]
                require(member.type == tarfile.REGTYPE and not member.linkname and not member.pax_headers, "archive members must be ordinary regular files")
                require(member.offset == end and member.offset_data == end + 512, "archive contains hidden extension headers")
                require(unpacked[end:end + 512] == member.tobuf(format=tarfile.USTAR_FORMAT), "archive member header must use canonical metadata")
                require(member.mode == MEMBERS[name], "archive member permissions do not match contract")
                require(member.uid == member.gid == member.mtime == 0 and not member.uname and not member.gname, "archive member metadata contains noncanonical identity or timestamps")
                require(0 < member.size <= (MAX_BINARY if name == "boxferry" else MAX_TEXT), "archive member exceeds its size bound")
                stream = archive.extractfile(member)
                require(stream is not None, "archive member has no content")
                with stream:
                    files[member.name] = stream.read(member.size + 1)
                require(len(files[member.name]) == member.size, "archive member is truncated")
                end = member.offset_data + ((member.size + 511) // 512) * 512
                require(not any(unpacked[member.offset_data + member.size:end]), "archive contains nonzero member padding")
            require(set(files) == expected, "archive member set is incomplete")
            require(len(unpacked) >= end + 1024 and not any(unpacked[end:]), "invalid archive end marker or trailing data")
        binary = files[f"{root}/boxferry"]
        validate_binary(binary, target)
        mode = {"mode": "qemu-user", "emulator": EMULATORS[target]} if target in EMULATORS else {"mode": "native", "emulator": None}
        expected_info = build_info(argparse.Namespace(**{**vars(args), "target": target}), binary, mode)
        require(json_bytes(parse_json(files[f"{root}/BUILD-INFO.json"])) == json_bytes(expected_info), "archive build metadata or binary digest does not match invocation")
        for name in ("LICENSE", "README.md"):
            require(files[f"{root}/{name}"] == read_regular(ROOT / name, MAX_TEXT), "archive documentation does not match the candidate checkout")
        return {"name": archive_name(args.version, target), "target": target, "sha256": sha256(data)}
    except (tarfile.TarError, EOFError, OSError, struct.error, UnicodeError, zlib.error) as error:
        raise ArtifactError("invalid or truncated release archive") from error


def package(args: argparse.Namespace) -> None:
    binary = read_regular(args.binary, MAX_BINARY)
    validate_binary(binary, args.target)
    mode = smoke(args.binary, args.target, args.version, args.emulator)
    require(read_regular(args.binary, MAX_BINARY) == binary, "binary changed during smoke validation")
    files = {"boxferry": binary, "LICENSE": read_regular(ROOT / "LICENSE", MAX_TEXT), "README.md": read_regular(ROOT / "README.md", MAX_TEXT),
             "BUILD-INFO.json": json_bytes(build_info(args, binary, mode))}
    data = make_archive(args.version, args.target, files)
    validate_archive(data, args, args.target)
    output = directory(args.output, create=True)
    write_new(output / archive_name(args.version, args.target), data)


def manifest(args: argparse.Namespace, records: list[dict]) -> dict:
    return {"schema_version": 1, "version": args.version, "revision": args.revision,
            "github_run_id": args.run_id, "archives": records}


def read_set(args: argparse.Namespace) -> list[tuple[str, bytes]]:
    source = directory(args.input)
    expected = {archive_name(args.version, target) for target in TARGETS} | {"binary-manifest.json"}
    require(names(source, len(expected)) == expected, "binary set must contain exactly six archives and its manifest")
    records, archives = [], []
    for target in TARGETS:
        name = archive_name(args.version, target)
        data = read_regular(source / name, MAX_ARCHIVE)
        records.append(validate_archive(data, args, target))
        archives.append((name, data))
    require(json_bytes(parse_json(read_regular(source / "binary-manifest.json", MAX_TEXT))) == json_bytes(manifest(args, records)), "binary manifest set or digest does not match current invocation")
    return archives


def names(path: Path, maximum: int) -> set[str]:
    entries: set[str] = set()
    for entry in path.iterdir():
        entries.add(entry.name)
        require(len(entries) <= maximum, "directory entry count exceeds artifact layout")
    return entries


def copy_new(output: Path, archives: list[tuple[str, bytes]]) -> None:
    # Check every collision before making the first asset visible.
    for name, _ in archives:
        path = safe_path(output / name)
        require(not path.exists(), "output asset collision")
    created: list[Path] = []
    try:
        for name, data in archives:
            write_new(output / name, data)
            created.append(output / name)
    except BaseException:
        for path in created:
            path.unlink()
        raise


def collect(args: argparse.Namespace) -> None:
    source = directory(args.input)
    expected = {f"boxferry-cli-{args.revision}-{args.run_id}-{target}" for target in TARGETS}
    require(names(source, len(expected)) == expected, "worker artifacts must match all six current-run target directories")
    records, archives = [], []
    for target in TARGETS:
        worker = directory(source / f"boxferry-cli-{args.revision}-{args.run_id}-{target}")
        name = archive_name(args.version, target)
        require(names(worker, 1) == {name}, "worker artifact directory must contain only its canonical archive")
        data = read_regular(worker / name, MAX_ARCHIVE)
        records.append(validate_archive(data, args, target))
        archives.append((name, data))
    require(not safe_path(args.output).is_relative_to(source), "collection output must be outside worker input")
    output = directory(args.output, create=True, empty=True)
    copy_new(output, archives + [("binary-manifest.json", json_bytes(manifest(args, records)))])


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    names = commands.add_parser("archive-names")
    names.add_argument("--version", required=True)
    for name in ("package", "verify", "collect", "verify-set", "stage"):
        command = commands.add_parser(name)
        for option in ("version", "revision", "run-id"):
            command.add_argument(f"--{option}", required=True)
        if name in ("package", "verify"):
            command.add_argument("--target", choices=TARGETS, required=True)
        if name == "package":
            command.add_argument("--binary", type=Path, required=True)
            command.add_argument("--emulator", type=Path)
        if name == "verify":
            command.add_argument("--archive", type=Path, required=True)
        if name in ("collect", "verify-set", "stage"):
            command.add_argument("--input", type=Path, required=True)
        if name in ("package", "collect", "stage"):
            command.add_argument("--output", type=Path, required=True)
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        binding(args.version, getattr(args, "revision", "0" * 40), getattr(args, "run_id", "1"))
        if args.command == "archive-names":
            print("\n".join(archive_name(args.version, target) for target in TARGETS))
        elif args.command == "package":
            package(args)
        elif args.command == "verify":
            require(args.archive.name == archive_name(args.version, args.target), "archive basename does not match target and version")
            validate_archive(read_regular(args.archive, MAX_ARCHIVE), args, args.target)
        elif args.command == "collect":
            collect(args)
        elif args.command == "verify-set":
            read_set(args)
        elif args.command == "stage":
            archives = read_set(args)
            copy_new(directory(args.output, create=True), archives)
    except (ArtifactError, OSError, ValueError, struct.error, RecursionError, TypeError) as error:
        # Never emit subprocess output, filenames, environment or low-level OS errors.
        message = str(error) if isinstance(error, ArtifactError) else "invalid input or filesystem operation failed"
        print(f"release-binaries: {message}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
