#!/usr/bin/env python3
"""Independent binary-header, archive, six-target and bounded-execution regressions."""

from __future__ import annotations

import argparse
import copy
import gzip
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("release_binaries", ROOT / "scripts/release-binaries.py")
assert SPEC is not None and SPEC.loader is not None
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
VERSION = "0.10.0"
REVISION = "0123456789abcdef0123456789abcdef01234567"
RUN = "15842"
# Independent expected distribution, not taken from the implementation's target table.
TARGETS = (
    "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
    "i686-unknown-linux-musl", "armv7-unknown-linux-musleabihf",
    "x86_64-apple-darwin", "aarch64-apple-darwin",
)


def options(**changes: object) -> argparse.Namespace:
    return argparse.Namespace(**{"version": VERSION, "revision": REVISION, "run_id": RUN, "target": TARGETS[0], **changes})


def elf(bits: int = 64, cpu: int = 62, *, kind: int = 2, dynamic: tuple[int, ...] | None = None, interpreter: bool = False, flags: int = 0) -> bytes:
    """Minimal independently authored ELF with one executable segment and optional loader table."""
    header_size, entry_size = (64, 56) if bits == 64 else (52, 32)
    count = 1 + (dynamic is not None) + interpreter
    content_offset = header_size + entry_size * count
    entry_format = "<qQ" if bits == 64 else "<iI"
    dynamic_bytes = b"".join(struct.pack(entry_format, tag, 0) for tag in dynamic or ())
    payload = b"fixture-code" + dynamic_bytes
    total = content_offset + len(payload)
    ident = b"\x7fELF" + bytes([2 if bits == 64 else 1, 1, 1, 0]) + b"\0" * 8
    header = struct.pack("<HHIQQQIHHHHHH" if bits == 64 else "<HHIIIIIHHHHHH", kind, cpu, 1, 0, header_size, 0, flags, header_size, entry_size, count, 0, 0, 0)

    def segment(ptype: int, offset: int, length: int, permissions: int) -> bytes:
        if bits == 64:
            return struct.pack("<IIQQQQQQ", ptype, permissions, offset, 0, 0, length, length, 1)
        return struct.pack("<IIIIIIII", ptype, offset, 0, 0, length, length, permissions, 1)

    programs = segment(1, 0, total, 5)
    if dynamic is not None:
        programs += segment(2, content_offset + len(b"fixture-code"), len(dynamic_bytes), 6)
    if interpreter:
        programs += segment(3, content_offset, len(b"fixture-code"), 4)
    return ident + header + programs + payload


def macho(cpu: int = 0x01000007, subtype: int = 3) -> bytes:
    header = struct.pack("<IIIIIIII", 0xFEEDFACF, cpu, subtype, 2, 1, 72, 0, 0)
    segment = struct.pack("<II16sQQQQiiII", 0x19, 72, b"__TEXT", 0, 104, 0, 104, 7, 5, 0, 0)
    return header + segment


def binary_for(target: str) -> bytes:
    return {
        TARGETS[0]: elf(), TARGETS[1]: elf(cpu=183), TARGETS[2]: elf(32, 3),
        TARGETS[3]: elf(32, 40, flags=0x05000400), TARGETS[4]: macho(),
        TARGETS[5]: macho(0x0100000C, 0),
    }[target]


def expected_info(target: str, binary: bytes) -> dict:
    import hashlib
    emulator = {TARGETS[2]: "qemu-i386", TARGETS[3]: "qemu-arm"}.get(target)
    return {"schema_version": 1, "version": VERSION, "target": target, "revision": REVISION,
            "github_run_id": RUN, "binary_sha256": hashlib.sha256(binary).hexdigest(),
            "execution": {"mode": "qemu-user" if emulator else "native", "emulator": emulator},
            "smoke_tests": ["version", "capabilities-nine-routes", "compose-quadlet-compose"]}


def archive_fixture(target: str = TARGETS[0], *, mutate=None, extra=None, metadata=None, payload: bytes | None = None) -> bytes:
    binary = payload if payload is not None else binary_for(target)
    root = f"boxferry-{VERSION}-{target}"
    info = expected_info(target, binary)
    if metadata:
        metadata(info)
    contents = {"boxferry": binary, "LICENSE": (ROOT / "LICENSE").read_bytes(),
                "README.md": (ROOT / "README.md").read_bytes(), "BUILD-INFO.json": json.dumps(info).encode()}
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, content in contents.items():
            member = tarfile.TarInfo(f"{root}/{name}")
            member.mode = 0o755 if name == "boxferry" else 0o644
            member.size = len(content)
            if mutate:
                mutate(member)
            archive.addfile(member, io.BytesIO(content))
        if extra:
            member, content = extra(root)
            member.size = len(content)
            archive.addfile(member, io.BytesIO(content))
    return gzip.compress(raw.getvalue(), mtime=0)


class BinaryHeaderTests(unittest.TestCase):
    def test_all_six_independent_executable_headers(self) -> None:
        self.assertEqual(release.TARGETS, TARGETS)
        for target in TARGETS:
            with self.subTest(target=target):
                release.validate_binary(binary_for(target), target)

    def test_static_pie_with_dynamic_relocations_is_allowed(self) -> None:
        release.validate_binary(elf(kind=3, dynamic=(7, 8, 9, 0)), TARGETS[0])
        release.validate_binary(elf(32, 3, kind=3, dynamic=(17, 18, 0)), TARGETS[2])

    def test_linux_loader_and_needed_libraries_fail(self) -> None:
        for bits, cpu, target in ((64, 62, TARGETS[0]), (32, 3, TARGETS[2])):
            for data in (elf(bits, cpu, interpreter=True), elf(bits, cpu, dynamic=(1, 0)), elf(bits, cpu, dynamic=(7,)), elf(bits, cpu, dynamic=(0, 1))):
                with self.subTest(bits=bits, header=data[:20]), self.assertRaises(release.ArtifactError):
                    release.validate_binary(data, target)

    def test_malformed_and_wrong_architecture_headers_fail(self) -> None:
        bad = [b"", b"not-an-executable", elf(32, 3), elf(cpu=183), elf(kind=1), elf()[:63],
               elf()[:5] + b"\x02" + elf()[6:]]
        # Deliberate out-of-file program header table.
        corrupt = bytearray(elf())
        struct.pack_into("<Q", corrupt, 32, 2**63)
        bad.append(bytes(corrupt))
        corrupt = bytearray(elf())
        struct.pack_into("<H", corrupt, 56, 0)
        bad.append(bytes(corrupt))
        for data in bad:
            with self.subTest(data=data[:20]), self.assertRaises(release.ArtifactError):
                release.validate_binary(data, TARGETS[0])

    def test_arm_requires_hard_float_eabi5(self) -> None:
        for flags in (0, 0x04000400, 0x05000200, 0x05000600):
            with self.subTest(flags=flags), self.assertRaises(release.ArtifactError):
                release.validate_binary(elf(32, 40, flags=flags), TARGETS[3])

    def test_macho_rejects_fat_wrong_cpu_kind_subtype_and_bad_commands(self) -> None:
        cases = [b"\xca\xfe\xba\xbe" + macho()[4:], macho(0x0100000C, 0), macho(subtype=8), macho()[:-1]]
        for offset, value in ((12, 6), (16, 0), (20, 8), (28, 1), (36, 7)):
            malformed = bytearray(macho())
            struct.pack_into("<I", malformed, offset, value)
            cases.append(bytes(malformed))
        for data in cases:
            with self.subTest(data=data[:40]), self.assertRaises(release.ArtifactError):
                release.validate_binary(data, TARGETS[4])


class ArchiveTests(unittest.TestCase):
    def test_valid_archives_bind_each_architecture(self) -> None:
        import hashlib
        for target in TARGETS:
            data = archive_fixture(target)
            result = release.validate_archive(data, options(), target)
            self.assertEqual(result, {"name": f"boxferry-{VERSION}-{target}.tar.gz", "target": target, "sha256": hashlib.sha256(data).hexdigest()})

    def test_permission_identity_timestamp_and_file_types_fail(self) -> None:
        for field, value in (("mode", 0o777), ("mode", 0o4755), ("uid", 1000), ("gname", "private-user"), ("mtime", 1234), ("type", tarfile.SYMTYPE), ("type", tarfile.LNKTYPE), ("type", tarfile.CHRTYPE), ("type", tarfile.DIRTYPE)):
            with self.subTest(field=field, value=value), self.assertRaises(release.ArtifactError):
                release.validate_archive(archive_fixture(mutate=lambda member: setattr(member, field, value)), options(), TARGETS[0])

    def test_traversal_absolute_duplicate_and_unexpected_members_fail(self) -> None:
        for name in ("../escape", "/absolute", "other/boxferry", f"boxferry-{VERSION}-{TARGETS[0]}/../boxferry"):
            with self.subTest(name=name), self.assertRaises(release.ArtifactError):
                release.validate_archive(archive_fixture(mutate=lambda member: setattr(member, "name", name)), options(), TARGETS[0])
        for suffix in ("boxferry", "extra"):
            with self.subTest(suffix=suffix), self.assertRaises(release.ArtifactError):
                release.validate_archive(archive_fixture(extra=lambda root: (tarfile.TarInfo(f"{root}/{suffix}"), b"unexpected")), options(), TARGETS[0])

    def test_metadata_binding_and_digest_fail_closed(self) -> None:
        for field, value in (("version", "0.9.0"), ("revision", "f" * 40), ("github_run_id", "1"), ("target", TARGETS[1]), ("binary_sha256", "0" * 64), ("execution", {"mode": "native", "emulator": "/private/qemu"}), ("smoke_tests", []), ("extra", "/private/build-path")):
            with self.subTest(field=field), self.assertRaises(release.ArtifactError):
                release.validate_archive(archive_fixture(metadata=lambda value_map: value_map.update({field: value})), options(), TARGETS[0])

    def test_schema_types_and_duplicate_json_keys_fail(self) -> None:
        for value in (True, 1.0):
            with self.subTest(value=value), self.assertRaises(release.ArtifactError):
                release.validate_archive(archive_fixture(metadata=lambda info: info.update(schema_version=value)), options(), TARGETS[0])
        with self.assertRaises(release.ArtifactError):
            release.parse_json(b'{"revision":"first", "revision":"second"}')
        with self.assertRaises(release.ArtifactError):
            release.parse_json(b"[" * 2000 + b"]" * 2000)

    def test_hidden_tar_extensions_and_nonzero_padding_fail(self) -> None:
        raw = gzip.decompress(archive_fixture())
        extension = tarfile.TarInfo("././@LongLink")
        extension.type = tarfile.GNUTYPE_LONGNAME
        long_name = f"boxferry-{VERSION}-{TARGETS[0]}/boxferry".encode() + b"\0"
        extension.size = len(long_name)
        hidden = extension.tobuf() + long_name.ljust(512, b"\0") + raw
        with self.assertRaises(release.ArtifactError):
            release.validate_archive(gzip.compress(hidden, mtime=0), options(), TARGETS[0])
        corrupt = bytearray(raw)
        corrupt[512 + len(binary_for(TARGETS[0]))] = 65
        with self.assertRaises(release.ArtifactError):
            release.validate_archive(gzip.compress(corrupt, mtime=0), options(), TARGETS[0])
        with self.assertRaises(release.ArtifactError):
            release.validate_archive(archive_fixture() + gzip.compress(b"", mtime=0), options(), TARGETS[0])

    def test_wrong_binary_cannot_be_rebound_by_metadata(self) -> None:
        with self.assertRaises(release.ArtifactError):
            release.validate_archive(archive_fixture(payload=elf(cpu=183)), options(), TARGETS[0])

    def test_damaged_oversized_and_trailing_archive_data_fail(self) -> None:
        good = archive_fixture()
        for data in (good[:-5], b"not gzip", good + b"garbage", gzip.compress(gzip.decompress(good) + b"nonzero" * 100, mtime=0)):
            with self.subTest(size=len(data)), self.assertRaises(release.ArtifactError):
                release.validate_archive(data, options(), TARGETS[0])
        with patch.object(release, "MAX_TAR", 1024), self.assertRaises(release.ArtifactError):
            release.validate_archive(good, options(), TARGETS[0])
        with patch.object(release, "MAX_ARCHIVE", 100), self.assertRaises(release.ArtifactError):
            release.validate_archive(good, options(), TARGETS[0])

    def test_private_gzip_names_and_nonzero_dates_fail(self) -> None:
        for kwargs in ({"filename": "private-builder-path", "mtime": 0}, {"filename": "", "mtime": 1}):
            output = io.BytesIO()
            with gzip.GzipFile(fileobj=output, mode="wb", **kwargs) as stream:
                stream.write(gzip.decompress(archive_fixture()))
            with self.assertRaises(release.ArtifactError):
                release.validate_archive(output.getvalue(), options(), TARGETS[0])

    def test_reproducible_packaging_and_no_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary = root / "boxferry"
            binary.write_bytes(elf())
            args = options(binary=binary, emulator=None, output=root / "first")
            with patch.object(release, "smoke", return_value={"mode": "native", "emulator": None}) as smoke:
                release.package(args)
                self.assertEqual(smoke.call_count, 1)
                args.output = root / "second"
                release.package(args)
                name = f"boxferry-{VERSION}-{TARGETS[0]}.tar.gz"
                self.assertEqual((root / "first" / name).read_bytes(), (root / "second" / name).read_bytes())
                with self.assertRaises(FileExistsError):
                    release.package(args)
                before = set(root.iterdir())
                args.output = root / "failed"
                smoke.side_effect = release.ArtifactError("smoke failed")
                with self.assertRaises(release.ArtifactError):
                    release.package(args)
                self.assertEqual(set(root.iterdir()), before)


    def test_package_refuses_binary_changed_during_execution(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary = root / "boxferry"
            binary.write_bytes(elf())
            args = options(binary=binary, emulator=None, output=root / "output")

            def mutate_binary(*unused: object) -> dict:
                binary.write_bytes(elf() + b"changed")
                return {"mode": "native", "emulator": None}

            with patch.object(release, "smoke", side_effect=mutate_binary), self.assertRaises(release.ArtifactError):
                release.package(args)
            self.assertFalse(args.output.exists())


class CollectionTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.workers, self.flat, self.assets = (self.root / name for name in ("workers", "flat", "assets"))
        self.workers.mkdir()
        for target in TARGETS:
            worker = self.workers / f"boxferry-cli-{REVISION}-{RUN}-{target}"
            worker.mkdir()
            (worker / f"boxferry-{VERSION}-{target}.tar.gz").write_bytes(archive_fixture(target))
        self.args = options(input=self.workers, output=self.flat)

    def test_exact_six_set_manifest_and_stage(self) -> None:
        release.collect(self.args)
        manifest = json.loads((self.flat / "binary-manifest.json").read_bytes())
        self.assertEqual(manifest["revision"], REVISION)
        self.assertEqual(manifest["github_run_id"], RUN)
        self.assertEqual([entry["target"] for entry in manifest["archives"]], list(TARGETS))
        self.assertEqual(len(release.read_set(options(input=self.flat))), 6)
        self.assets.mkdir()
        (self.assets / "boxferry-0.10.0.crate").write_bytes(b"existing crate")
        release.copy_new(self.assets, release.read_set(options(input=self.flat)))
        self.assertEqual(len(list(self.assets.glob("*.tar.gz"))), 6)
        self.assertFalse((self.assets / "binary-manifest.json").exists())
        self.assertEqual((self.assets / "boxferry-0.10.0.crate").read_bytes(), b"existing crate")
        with self.assertRaises(release.ArtifactError):
            release.copy_new(self.assets, release.read_set(options(input=self.flat)))

    def test_missing_extra_and_other_run_workers_fail_before_output(self) -> None:
        worker = next(self.workers.iterdir())
        worker.rename(self.workers / "boxferry-cli-stale-run-target")
        with self.assertRaises(release.ArtifactError):
            release.collect(self.args)
        self.assertFalse(self.flat.exists())

    def test_missing_and_extra_workers_and_stale_archive_fail(self) -> None:
        worker = next(self.workers.iterdir())
        moved = self.root / "removed-worker"
        worker.rename(moved)
        with self.assertRaises(release.ArtifactError):
            release.collect(self.args)
        moved.rename(worker)
        extra = self.workers / "extra-worker"
        extra.mkdir()
        with self.assertRaises(release.ArtifactError):
            release.collect(self.args)
        extra.rmdir()
        path = next(worker.iterdir())
        target = next(target for target in TARGETS if target in path.name)
        path.write_bytes(archive_fixture(target, metadata=lambda info: info.update(github_run_id="999")))
        with self.assertRaises(release.ArtifactError):
            release.collect(self.args)
        self.assertFalse(self.flat.exists())

    def test_full_cli_collect_verify_and_stage_contract(self) -> None:
        command = [sys.executable, str(ROOT / "scripts/release-binaries.py")]
        common = ["--version", VERSION, "--revision", REVISION, "--run-id", RUN]
        subprocess.run(command + ["collect", "--input", str(self.workers), "--output", str(self.flat)] + common, check=True, capture_output=True)
        subprocess.run(command + ["verify-set", "--input", str(self.flat)] + common, check=True, capture_output=True)
        subprocess.run(command + ["stage", "--input", str(self.flat), "--output", str(self.assets)] + common, check=True, capture_output=True)
        for target in TARGETS:
            subprocess.run(command + ["verify", "--archive", str(self.assets / f"boxferry-{VERSION}-{target}.tar.gz"), "--target", target] + common, check=True, capture_output=True)
        self.assertEqual(len(list(self.assets.iterdir())), 6)
        result = subprocess.run(command + ["stage", "--input", str(self.flat), "--output", str(self.root / "untrusted-output"), "--version", VERSION, "--revision", REVISION, "--run-id", "999"], check=False, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "untrusted-output").exists())

    def test_rollback_and_nonregular_output_collisions(self) -> None:
        output = self.root / "output"
        output.mkdir()
        write = release.write_new
        calls = 0

        def fail_second(path: Path, data: bytes) -> None:
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError("injected failure")
            write(path, data)

        with patch.object(release, "write_new", side_effect=fail_second), self.assertRaises(OSError):
            release.copy_new(output, [("first", b"a"), ("second", b"b")])
        self.assertEqual(list(output.iterdir()), [])
        (output / "second").mkdir()
        with self.assertRaises(release.ArtifactError):
            release.copy_new(output, [("first", b"a"), ("second", b"b")])
        self.assertFalse((output / "first").exists())

    def test_worker_extra_member_and_symlink_fail(self) -> None:
        worker = next(self.workers.iterdir())
        (worker / "unexpected").write_bytes(b"x")
        with self.assertRaises(release.ArtifactError):
            release.collect(self.args)
        (worker / "unexpected").unlink()
        archive = next(worker.iterdir())
        original = archive.read_bytes()
        archive.unlink()
        other = self.root / "untrusted.tar.gz"
        other.write_bytes(original)
        archive.symlink_to(other)
        with self.assertRaises(release.ArtifactError):
            release.collect(self.args)

    def test_stale_archive_and_manifest_tampering_fail(self) -> None:
        release.collect(self.args)
        manifest_path = self.flat / "binary-manifest.json"
        original = json.loads(manifest_path.read_bytes())
        for mutate in (lambda doc: doc.update(github_run_id="1"), lambda doc: doc["archives"].pop(), lambda doc: doc["archives"].append(doc["archives"][0]), lambda doc: doc["archives"][0].update(sha256="f" * 64)):
            document = copy.deepcopy(original)
            mutate(document)
            manifest_path.write_text(json.dumps(document))
            with self.assertRaises(release.ArtifactError):
                release.read_set(options(input=self.flat))
        manifest_path.write_text(json.dumps(original))
        (self.flat / "extra").write_bytes(b"x")
        with self.assertRaises(release.ArtifactError):
            release.read_set(options(input=self.flat))

    def test_collision_preflight_preserves_all_assets(self) -> None:
        release.collect(self.args)
        self.assets.mkdir()
        name = f"boxferry-{VERSION}-{TARGETS[-1]}.tar.gz"
        (self.assets / name).write_bytes(b"do not overwrite")
        with self.assertRaises(release.ArtifactError):
            release.copy_new(self.assets, release.read_set(options(input=self.flat)))
        self.assertEqual(list(self.assets.iterdir()), [self.assets / name])
        self.assertEqual((self.assets / name).read_bytes(), b"do not overwrite")

    def test_nonregular_and_parent_symlink_inputs_fail(self) -> None:
        fifo = self.root / "fifo"
        os.mkfifo(fifo)
        with self.assertRaises(release.ArtifactError):
            release.read_regular(fifo, 100)
        link = self.root / "linked"
        link.symlink_to(self.workers, target_is_directory=True)
        with self.assertRaises(release.ArtifactError):
            release.directory(link)
        with self.assertRaises(release.ArtifactError):
            release.write_new(link / "escaped", b"x")
        self.assertFalse((self.workers / "escaped").exists())


class ExecutionTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def test_minimal_environment_no_secret_inheritance_and_nonzero_privacy(self) -> None:
        with patch.dict(os.environ, {"PRIVATE_TOKEN": "must-not-escape"}):
            data = release.run_bounded([sys.executable, "-c", "import os; print(os.environ.get('PRIVATE_TOKEN', 'absent')); print(os.environ['HOME'])"], self.root)
        self.assertEqual(data.decode().splitlines(), ["absent", str(self.root)])
        with self.assertRaisesRegex(release.ArtifactError, "^binary smoke command failed$"):
            release.run_bounded([sys.executable, "-c", "import sys; print('private stderr', file=sys.stderr); sys.exit(7)"], self.root)

    def test_output_bound_and_pipe_holding_child_deadline(self) -> None:
        with patch.object(release, "MAX_OUTPUT", 128), self.assertRaisesRegex(release.ArtifactError, "output exceeded"):
            release.run_bounded([sys.executable, "-c", "print('x'*1000)"], self.root)
        start = time.monotonic()
        with patch.object(release, "COMMAND_TIMEOUT", 0.2), self.assertRaisesRegex(release.ArtifactError, "timed out"):
            release.run_bounded([sys.executable, "-c", "import subprocess,sys; subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'])"], self.root)
        self.assertLess(time.monotonic() - start, 3)

    def test_stderr_is_bounded_and_silent_commands_time_out(self) -> None:
        with patch.object(release, "MAX_OUTPUT", 128), self.assertRaisesRegex(release.ArtifactError, "output exceeded"):
            release.run_bounded([sys.executable, "-c", "import sys; print('x'*1000,file=sys.stderr)"], self.root)
        with patch.object(release, "COMMAND_TIMEOUT", 0.1), self.assertRaisesRegex(release.ArtifactError, "timed out"):
            release.run_bounded([sys.executable, "-c", "import time; time.sleep(30)"], self.root)

    def test_explicit_32bit_emulator_selection(self) -> None:
        emulator = self.root / "qemu-i386"
        emulator.write_bytes(b"fixture")
        emulator.chmod(0o755)
        self.assertEqual(release.execution(TARGETS[2], emulator), {"mode": "qemu-user", "emulator": "qemu-i386"})
        for target, selection in ((TARGETS[2], None), (TARGETS[3], emulator), (TARGETS[0], emulator)):
            with self.subTest(target=target), self.assertRaises(release.ArtifactError):
                release.execution(target, selection)

    def make_fake_cli(self) -> Path:
        executable = self.root / "fake-cli"
        executable.write_text(f"#!{sys.executable}\n" + '''import json,sys
from pathlib import Path
args=sys.argv[1:]
if args == ['--version']:
    print('boxferry 0.10.0')
elif args[:1] == ['capabilities']:
    print(json.dumps({'schema_version':1,'routes':[{'input_type':a,'output_type':b} for a in ('compose','quadlet','podman') for b in ('compose','quadlet','podman')]}))
elif args[:3] == ['convert','compose','quadlet']:
    source=Path(args[args.index('--input-file')+1]).read_text()
    assert 'image: docker.io/library/alpine:3.21' in source
    output=Path(args[args.index('--output-directory')+1]); output.mkdir()
    (output/'web.container').write_text('[Container]\\nImage=docker.io/library/alpine:3.21\\n')
elif args[:3] == ['convert','quadlet','compose']:
    source=Path(args[args.index('--input-directory')+1])/'web.container'
    assert source.is_file()
    output=Path(args[args.index('--output-directory')+1]); output.mkdir()
    (output/'compose.yaml').write_text('---\\nservices:\\n  web:\\n    image: docker.io/library/alpine:3.21\\n')
else:
    sys.exit(99)
''')
        executable.chmod(0o755)
        return executable

    def test_actual_smoke_processes_validate_version_capabilities_and_two_outputs(self) -> None:
        executable = self.make_fake_cli()
        self.assertEqual(release.smoke(executable, TARGETS[0], VERSION, None), {"mode": "native", "emulator": None})
        with self.assertRaisesRegex(release.ArtifactError, "version"):
            release.smoke(executable, TARGETS[0], "0.11.0", None)
        executable.write_text(f"#!{sys.executable}\nprint('boxferry 0.10.0')\n")
        with self.assertRaises(release.ArtifactError):
            release.smoke(executable, TARGETS[0], VERSION, None)

    def test_pinned_qemu_preserved_guest_argv0_for_both_32bit_targets(self) -> None:
        guest = self.make_fake_cli()
        for target, name in ((TARGETS[2], "qemu-i386"), (TARGETS[3], "qemu-arm")):
            emulator = self.root / name
            log = self.root / f"{name}.jsonl"
            emulator.write_text(f"#!{sys.executable}\n" + f"log = {str(log)!r}\n" + '''import json,os,sys
with open(log, 'a', encoding='utf-8') as stream:
    stream.write(json.dumps(sys.argv[1:]) + '\\n')
# Model the pinned tool: argv[1] selects the guest executable, argv[2]
# becomes the guest's argv0 and is therefore consumed, even if it was a flag.
os.execv(sys.executable, [sys.executable, sys.argv[1], *sys.argv[3:]])
''')
            emulator.chmod(0o755)
            # Without the explicit argv0 the version flag is consumed, which
            # reproduces the real pinned-emulator failure before this fix.
            with self.assertRaises(release.ArtifactError):
                release.run_bounded([str(emulator), str(guest), "--version"], self.root)
            log.unlink()
            self.assertEqual(release.smoke(guest, target, VERSION, emulator), {"mode": "qemu-user", "emulator": name})
            invocations = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertEqual(len(invocations), 4)
            self.assertTrue(all(arguments[:2] == [str(guest), str(guest)] for arguments in invocations))
            self.assertEqual(invocations[0][2:], ["--version"])
            self.assertEqual(invocations[1][2:], ["capabilities", "--console-format", "json"])
            self.assertEqual(invocations[2][2:5], ["convert", "compose", "quadlet"])
            self.assertEqual(invocations[3][2:5], ["convert", "quadlet", "compose"])

    def test_cli_names_binding_and_private_filesystem_errors(self) -> None:
        command = [sys.executable, str(ROOT / "scripts/release-binaries.py")]
        result = subprocess.run(command + ["archive-names", "--version", VERSION], capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.splitlines(), [f"boxferry-{VERSION}-{target}.tar.gz" for target in TARGETS])
        for version in ("../escape", "01.2.3", "1.2", "v1.2.3", "1.2.3/private", "1.2.3-01", "1.2.3-alpha..1", "1.2.3-" + "a" * 130):
            result = subprocess.run(command + ["archive-names", "--version", version], capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
        release.binding("1.2.3-alpha.1+build.23", REVISION, RUN)
        for revision, run in (("main", RUN), (REVISION.upper(), RUN), (REVISION, "0"), (REVISION, "01"), (REVISION, "../id")):
            with self.subTest(revision=revision, run=run), self.assertRaises(release.ArtifactError):
                release.binding(VERSION, revision, run)
        result = subprocess.run(command + ["verify", "--archive", str(self.root / "private-secret-directory" / f"boxferry-{VERSION}-{TARGETS[0]}.tar.gz"), "--target", TARGETS[0], "--version", VERSION, "--revision", REVISION, "--run-id", RUN], capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("private-secret", result.stderr)
        self.assertNotIn(str(self.root), result.stderr)


if __name__ == "__main__":
    unittest.main()
