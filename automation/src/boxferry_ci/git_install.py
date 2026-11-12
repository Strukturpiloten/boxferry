"""Opt-in, checksum-first Git source build into a new, complete runtime prefix.

This capability adds no operational source pin. Native compilation, base-owned
libraries/CA trust and image qualification remain separate from installer tests.
"""

from __future__ import annotations

import ctypes
import hashlib
import http.server
import json
import os
import re
import shutil
import signal
import ssl
import stat
import sys
import tarfile
import tempfile
import threading
import time
from collections.abc import Callable, Generator, Mapping
from contextlib import contextmanager
from http.client import HTTPMessage, IncompleteRead
from pathlib import Path
from typing import IO, Any
from urllib.error import HTTPError
from urllib.request import HTTPRedirectHandler, Request, build_opener

from boxferry_ci import runner, tooling
from boxferry_ci.bootstrap import bounded_sha256, safe_cache_directory
from boxferry_ci.git_source import GitSource, SourceError, inventory, source_record

MAX_ARCHIVE = 32 * 1024 * 1024
MAX_MEMBERS = 16384
MAX_SOURCE_FILE = 16 * 1024 * 1024
MAX_BUILD_BYTES = 2 * 1024 * 1024 * 1024
MAX_BUILD_FILES = 32768
MAX_RUNTIME_BYTES = 256 * 1024 * 1024
MAX_RUNTIME_FILE = 64 * 1024 * 1024
BUILD_SECONDS = 1800
INSTALL_SECONDS = 2100
# The shared Linux image owns this bundle; interpreter defaults and caller
# SSL_CERT_FILE/SSL_CERT_DIR must not choose trust for the runtime probe.
CA_BUNDLE = Path("/etc/ssl/certs/ca-certificates.crt")
# Child limits survive exec; the existing runner owns pipes, wall time and groups.
# make -j2 bounds parallel compilation. The monitor bounds aggregate working files.
# No RLIMIT_NPROC: it counts every process/thread for the real UID, can reject
# unrelated concurrent work, and is not enforced for privileged/root image builds.
# This installer is not a process-count/network isolation sandbox.
_LIMIT_EXEC = (
    "import os,resource,sys; "
    "resource.setrlimit(resource.RLIMIT_CORE,(0,0)); "
    "resource.setrlimit(resource.RLIMIT_AS,(3221225472,3221225472)); "
    "resource.setrlimit(resource.RLIMIT_FSIZE,(268435456,268435456)); "
    "resource.setrlimit(resource.RLIMIT_CPU,(1800,1800)); "
    "resource.setrlimit(resource.RLIMIT_NOFILE,(256,256)); "
    "os.execv(sys.argv[1],sys.argv[1:])"
)
CommandRunner = Callable[..., runner.RunResult]
Downloader = Callable[[GitSource, Path], None]


def validated(source: GitSource) -> GitSource:
    """Typed callers cannot bypass the canonical provider/URL/integrity reader."""
    value = source_record(
        {
            "tool": "git",
            "provider": "kernel.org",
            "currentVersion": source.version,
            "filename": source.filename,
            "url": source.url,
            "sha256": source.sha256,
        }
    )
    if tuple(map(int, value.version.split("."))) < (2, 48, 0):
        raise tooling.ToolError("Git source builds require Git 2.48.0 or later")
    return value


def load_source(path: Path) -> GitSource:
    """Read only an explicitly configured source; never discover or supply a pin."""
    if path.is_symlink() or not path.is_file():
        raise tooling.ToolError("Git source inventory must be a regular file")
    if path.stat().st_size > tooling.MAX_MANIFEST:
        raise tooling.ToolError("Git source inventory exceeds its byte bound")
    try:
        _document, sources = inventory(path.read_bytes())
    except SourceError as error:
        raise tooling.ToolError(f"Git source inventory is invalid: {error}") from error
    if len(sources) != 1:
        raise tooling.ToolError("Git source build requires exactly one canonical Git source")
    return validated(sources[0])


class _NoRedirect(HTTPRedirectHandler):
    def redirect_request(
        self,
        req: Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: HTTPMessage,
        newurl: str,
    ) -> None:
        raise tooling.ToolError("Git source download must not redirect from its canonical URL")


@contextmanager
def _download_deadline() -> Generator[float]:
    """Own one Linux main-thread alarm across opening and every blocking read.

    A socket timeout is only an inactivity bound. Refuse another owner's timer
    or a blocked/pending alarm rather than silently weakening the absolute cap.
    """
    if sys.platform != "linux" or threading.current_thread() is not threading.main_thread():
        raise tooling.ToolError("Git source acquisition requires a Linux main-thread wall guard")
    if any(signal.getitimer(signal.ITIMER_REAL)):
        raise tooling.ToolError("Git source acquisition cannot replace an existing alarm")
    blocked = signal.pthread_sigmask(signal.SIG_BLOCK, ())
    if signal.SIGALRM in blocked or signal.SIGALRM in signal.sigpending():
        raise tooling.ToolError("Git source acquisition requires an unblocked, non-pending alarm")
    seconds = tooling.DOWNLOAD_SECONDS
    if seconds <= 0:
        raise tooling.ToolError("Git source acquisition requires a positive absolute deadline")
    previous = signal.getsignal(signal.SIGALRM)

    def exhausted(_number: int, _frame: object) -> None:
        raise tooling.ToolError("Git source acquisition exceeded its absolute download deadline")

    signal.signal(signal.SIGALRM, exhausted)
    try:
        deadline = time.monotonic() + seconds
        signal.setitimer(signal.ITIMER_REAL, seconds)
        yield deadline
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def download(source: GitSource, destination: Path) -> None:
    """One bounded unauthenticated request to the exact canonical HTTPS archive."""
    source = validated(source)
    if destination.exists() or destination.is_symlink():
        raise tooling.ToolError("Git source download destination already exists")
    identity = None
    complete = False
    try:
        with _download_deadline() as deadline:
            request = Request(source.url, headers={"User-Agent": "BoxFerry-canonical-tools/1"})
            with build_opener(_NoRedirect()).open(request, timeout=20) as response:
                if response.geturl() != source.url:
                    raise tooling.ToolError("Git source response differs from its canonical URL")
                length = response.headers.get("Content-Length")
                if length is not None and (not length.isdecimal() or int(length) > MAX_ARCHIVE):
                    raise tooling.ToolError("Git source Content-Length exceeds its byte bound")
                consumed = 0
                digest = hashlib.sha256()
                with destination.open("xb") as output:
                    facts = os.fstat(output.fileno())
                    identity = (facts.st_dev, facts.st_ino)
                    while chunk := response.read(tooling.CHUNK):
                        consumed += len(chunk)
                        if consumed > MAX_ARCHIVE or time.monotonic() > deadline:
                            raise tooling.ToolError(
                                "Git source download exceeded its byte/time bound"
                            )
                        output.write(chunk)
                        digest.update(chunk)
                if (
                    not consumed
                    or (length is not None and consumed != int(length))
                    or digest.hexdigest() != source.sha256
                    or time.monotonic() > deadline
                ):
                    raise tooling.ToolError(
                        "Git source download is truncated or has a SHA256 mismatch"
                    )
        complete = True
    except HTTPError as error:
        error.close()
        raise tooling.ToolError(f"Git source acquisition failed: HTTP {error.code}") from None
    except (OSError, IncompleteRead) as error:
        raise tooling.ToolError(f"Git source acquisition failed: {type(error).__name__}") from error
    finally:
        if not complete:
            tooling._remove_partial(destination, identity)


def extract(source: GitSource, archive: Path, destination: Path) -> Path:
    """No archive code runs until the complete artifact passes its owned SHA256."""
    source = validated(source)
    if bounded_sha256(archive, MAX_ARCHIVE) != source.sha256:
        raise tooling.ToolError("Git source SHA256 mismatch before extraction")
    expansion = tempfile.TemporaryDirectory(prefix=".boxferry-git-expand-", dir=destination.parent)
    expanded = Path(expansion.name) / "source.tar"
    # Reuse the unchanged binary decompression limits: 128 MiB, 60 seconds,
    # bounded XZ dictionary/output and refusal of truncation or trailing data.
    root = f"git-{source.version}"
    # Exact inert omissions observed in the verified 2.56.0 source. No generic
    # link allowance: release notes and Tcl/Tk GUI subprojects are not built.
    omitted = {
        f"{root}/RelNotes": f"Documentation/RelNotes/{source.version}.adoc",
        f"{root}/subprojects/git-gui": "../git-gui",
        f"{root}/subprojects/gitk": "../gitk-git",
    }
    seen: set[str] = set()
    deadline = time.monotonic() + 60
    total = 0
    try:
        tooling._expand(archive, "tar.xz", expanded)
        destination.mkdir()
        with tarfile.open(expanded, mode="r:") as incoming:
            for count, member in enumerate(incoming, 1):
                name = tooling._path(member.name.rstrip("/") if member.isdir() else member.name)
                if (
                    name in seen
                    or count > MAX_MEMBERS
                    or time.monotonic() > deadline
                    or not (name == root or name.startswith(root + "/"))
                ):
                    raise tooling.ToolError(
                        "Git source has repeated/foreign members or exceeds bounds"
                    )
                seen.add(name)
                if member.issym() and omitted.get(name) == member.linkname and member.size == 0:
                    continue
                if not (member.isdir() or member.isfile()) or member.size < 0:
                    raise tooling.ToolError(
                        "Git source contains a link, device or unsupported member"
                    )
                total += member.size
                if (
                    total > tooling.MAX_UNPACKED
                    or member.size > MAX_SOURCE_FILE
                    or (member.isdir() and member.size)
                ):
                    raise tooling.ToolError("Git source member data exceeds its size bound")
                target = destination / name
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    contents = incoming.extractfile(member)
                    if contents is None:
                        raise tooling.ToolError("Git source member data is unavailable")
                    with contents, target.open("xb") as output:
                        shutil.copyfileobj(contents, output, length=tooling.CHUNK)
                    if target.stat().st_size != member.size:
                        raise tooling.ToolError("Git source member is truncated")
                    target.chmod(0o755 if member.mode & 0o111 else 0o644)
        tree = destination / root
        for name in ("Makefile", "COPYING", "GIT-VERSION-GEN"):
            path = tree / name
            if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
                raise tooling.ToolError("Git source lacks its build/license/version closure")
        return tree
    except (OSError, tarfile.TarError, EOFError) as error:
        raise tooling.ToolError(f"Git source extraction failed: {type(error).__name__}") from error
    finally:
        expansion.cleanup()


def _files(root: Path, max_bytes: int, max_files: int) -> list[Path]:
    """Bound logical entries and physical bytes without following generated links."""
    files: list[Path] = []
    size = 0
    inodes: set[tuple[int, int]] = set()
    for directory, children, names in os.walk(root, followlinks=False):
        for name in (*children, *names):
            path = Path(directory) / name
            facts = path.lstat()
            files.append(path)
            key = (facts.st_dev, facts.st_ino)
            if key not in inodes:
                size += facts.st_size
                inodes.add(key)
            if len(files) > max_files or size > max_bytes:
                raise tooling.ToolError("Git build/runtime tree exceeded its file/byte budget")
    return files


def _invoke(
    argv: tuple[str, ...],
    cwd: Path,
    environment: Mapping[str, str],
    deadline: float,
    execute: CommandRunner,
    *,
    required: bool = True,
    budget_root: Path | None = None,
) -> runner.RunResult:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise tooling.ToolError("Git source installation exceeded its total deadline")

    def monitor() -> None:
        _files(budget_root or cwd, MAX_BUILD_BYTES, MAX_BUILD_FILES)

    result = execute(
        (sys.executable, "-I", "-c", _LIMIT_EXEC, *argv),
        cwd=cwd,
        environment=environment,
        stage="canonical Git source build/runtime validation",
        timeout_seconds=min(remaining, BUILD_SECONDS),
        max_output_bytes=1024 * 1024,
        monitor=monitor,
    )
    if result.status != "success" and (required or result.status != "failed"):
        raise tooling.ToolError(f"Git source command failed: {result.diagnostic.cause}")
    return result


def _https_probe(
    git: Path,
    cwd: Path,
    environment: Mapping[str, str],
    deadline: float,
    execute: CommandRunner,
) -> None:
    """Exercise the installed helper over local HTTPS, including rejected trust."""
    base_ca = CA_BUNDLE
    if not base_ca.is_file() or not 0 < base_ca.stat().st_size <= MAX_SOURCE_FILE:
        raise tooling.ToolError("Git HTTPS validation requires the base-owned CA trust bundle")
    cert, key = cwd / "probe-cert.pem", cwd / "probe-key.pem"
    _invoke(
        (
            "/usr/bin/openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            str(key),
            "-out",
            str(cert),
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost",
        ),
        cwd,
        environment,
        deadline,
        execute,
    )

    class Repository(http.server.BaseHTTPRequestHandler):
        timeout = 5

        def do_GET(self) -> None:
            payload = {"/info/refs": b"", "/HEAD": b"ref: refs/heads/main\n"}.get(
                self.path.split("?", 1)[0]
            )
            if payload is None:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def handle(self) -> None:
            try:
                super().handle()
            except (ssl.SSLError, BrokenPipeError, ConnectionResetError, TimeoutError):
                # Expected client abort during the negative certificate test.
                pass

        def log_message(self, format: str, *args: object) -> None:
            pass

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Repository) as server:
        server.socket = context.wrap_socket(
            server.socket, server_side=True, do_handshake_on_connect=False
        )
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"https://localhost:{server.server_port}/"
            common = (str(git), "-c", "http.sslVerify=true", "-c", "http.proxy=")
            _invoke(
                (*common, "-c", f"http.sslCAInfo={cert}", "ls-remote", url),
                cwd,
                environment,
                min(deadline, time.monotonic() + 30),
                execute,
            )
            rejected = _invoke(
                (*common, "-c", f"http.sslCAInfo={base_ca}", "ls-remote", url),
                cwd,
                environment,
                min(deadline, time.monotonic() + 30),
                execute,
                required=False,
            )
            if rejected.status == "success":
                raise tooling.ToolError("Git HTTPS accepted the deliberately untrusted test CA")
            if (
                re.search(
                    r"SSL certificate|certificate verify failed|"
                    r"server certificate verification failed",
                    rejected.stderr,
                )
                is None
            ):
                raise tooling.ToolError("Git HTTPS failed without proving untrusted-CA rejection")
        finally:
            server.shutdown()
            thread.join(timeout=5)
            if thread.is_alive():
                raise tooling.ToolError("Git HTTPS test server did not stop")


def _runtime(
    prefix: Path,
    source: GitSource,
    work: Path,
    environment: dict[str, str],
    deadline: float,
    execute: CommandRunner,
) -> list[dict[str, Any]]:
    entries = _files(prefix, MAX_RUNTIME_BYTES, MAX_MEMBERS)
    records: list[dict[str, Any]] = []
    hashes: dict[tuple[int, int], str] = {}
    for path in sorted(entries):
        facts = path.lstat()
        relative = tooling._path(str(path.relative_to(prefix)))
        if stat.S_ISLNK(facts.st_mode):
            target = os.readlink(path)
            if Path(target).is_absolute() or not path.resolve(strict=True).is_relative_to(prefix):
                raise tooling.ToolError("Git runtime link escapes its complete prefix")
            records.append({"path": relative, "link": target})
        elif stat.S_ISREG(facts.st_mode):
            if not 0 <= facts.st_size <= MAX_RUNTIME_FILE or facts.st_mode & 0o7000:
                raise tooling.ToolError("Git runtime file has an unsafe mode/size")
            key = (facts.st_dev, facts.st_ino)
            if key not in hashes:
                hashes[key] = bounded_sha256(path, MAX_RUNTIME_FILE)
            records.append({"path": relative, "sha256": hashes[key], "mode": facts.st_mode & 0o777})
        elif not stat.S_ISDIR(facts.st_mode):
            raise tooling.ToolError("Git runtime contains a non-file component")
    required = (
        "bin/git",
        "libexec/git-core/git-remote-http",
        "libexec/git-core/git-remote-https",
        "libexec/git-core/git-http-fetch",
        "libexec/git-core/git-http-push",
        "libexec/git-core/git-send-email",
        "share/perl5/Git.pm",
        "share/perl5/Git/I18N.pm",
        "share/git-core/templates/description",
        "share/git-core/templates/info/exclude",
        "share/git-core/templates/hooks/applypatch-msg.sample",
    )
    for relative in required:
        path = prefix / relative
        if (
            not path.is_file()
            or path.stat().st_size == 0
            or not path.resolve(strict=True).is_relative_to(prefix)
        ):
            raise tooling.ToolError(f"Git runtime is incomplete: {relative}")
        if relative.startswith(("bin/", "libexec/")) and not os.access(path, os.X_OK):
            raise tooling.ToolError("Git runtime executable is not executable")
    git = prefix / "bin/git"
    environment = {**environment, "PATH": str(prefix / "bin") + ":/usr/bin:/bin"}
    for args, expected in (
        (("--version",), f"git version {source.version}\n"),
        (("--exec-path",), str(prefix / "libexec/git-core") + "\n"),
        (("var", "GIT_TEMPLATE_DIR"), str(prefix / "share/git-core/templates") + "\n"),
    ):
        result = _invoke((str(git), *args), work, environment, deadline, execute)
        if result.stdout != expected:
            raise tooling.ToolError(
                "Git runtime version/helper/template prefix differs from selection"
            )
    result = _invoke(
        (
            "/usr/bin/perl",
            "-I" + str(prefix / "share/perl5"),
            "-MGit",
            "-e",
            'print "git-perl-ok\\n"',
        ),
        work,
        environment,
        deadline,
        execute,
    )
    if result.stdout != "git-perl-ok\n":
        raise tooling.ToolError("Git runtime Perl module validation failed")
    _https_probe(git, work, environment, deadline, execute)
    return records


def _publish(prefix: Path, destination: Path) -> None:
    """Linux atomic no-replace: a racing destination is preserved, even if empty."""
    library = ctypes.CDLL(None, use_errno=True)
    operation = getattr(library, "renameat2", None)
    if operation is None:
        raise tooling.ToolError("Git prefix publication requires Linux renameat2 no-replace")
    operation.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    ]
    operation.restype = ctypes.c_int
    if operation(-100, os.fsencode(prefix), -100, os.fsencode(destination), 1) != 0:
        raise OSError(ctypes.get_errno(), "Git prefix atomic no-replace publication failed")


def install(
    destination: Path,
    source: GitSource,
    *,
    architecture: str | None = None,
    downloader: Downloader = download,
    execute: CommandRunner = runner.run,
) -> dict[str, Any]:
    """Publish only a validated complete prefix; never merge with a prior runtime."""
    try:
        source = validated(source)
        host = tooling.host_architecture()
        architecture = host if architecture is None else architecture
        if architecture not in tooling.ARCHITECTURES or architecture != host:
            raise tooling.ToolError("Git source architecture must match the supported native host")
        destination = destination.absolute()
        if (
            destination == Path("/")
            or len(str(destination)) > 240
            or re.fullmatch(r"/[A-Za-z0-9_./-]+", str(destination)) is None
            or ".." in destination.parts
            or any(path.is_symlink() for path in (destination, *destination.parents))
            or destination.exists()
        ):
            raise tooling.ToolError(
                "Git requires a new unlinked absolute prefix with safe path components"
            )
        safe_cache_directory(destination.parent)
        deadline = time.monotonic() + INSTALL_SECONDS
        with tempfile.TemporaryDirectory(
            prefix=".boxferry-git-", dir=destination.parent
        ) as temporary:
            work = Path(temporary)
            archive = work / source.filename
            downloader(source, archive)
            tree = extract(source, archive, work / "source")
            home = work / "home"
            home.mkdir()
            environment = {
                "PATH": "/usr/bin:/bin",
                "HOME": str(home),
                "TMPDIR": str(work),
                "LC_ALL": "C",
                "TZ": "UTC",
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CEILING_DIRECTORIES": str(work.parent),
                "GIT_TERMINAL_PROMPT": "0",
                "NO_PROXY": "*",
                "no_proxy": "*",
            }
            flags = (
                f"prefix={destination}",
                f"bindir={destination}/bin",
                "gitexecdir=libexec/git-core",
                "template_dir=share/git-core/templates",
                f"perllibdir={destination}/share/perl5",
                "RUNTIME_PREFIX=YesPlease",
                "NO_TCLTK=YesPlease",
                "NO_GETTEXT=YesPlease",
            )
            stage = work / "stage"
            stage.mkdir()
            for target in ("all", "install"):
                argv = ("/usr/bin/make", "-j2", *flags, f"DESTDIR={stage}", target)
                _invoke(argv, tree, environment, deadline, execute, budget_root=work)
            _files(work, MAX_BUILD_BYTES, MAX_BUILD_FILES)
            prefix = stage / destination.relative_to("/")
            if prefix.is_symlink() or not prefix.is_dir():
                raise tooling.ToolError("Git build did not install the selected complete prefix")
            for path in _files(stage, MAX_BUILD_BYTES, MAX_BUILD_FILES):
                if not (path.is_relative_to(prefix) or prefix.is_relative_to(path)):
                    raise tooling.ToolError(
                        "Git build installed outside the selected runtime prefix"
                    )
            records = _runtime(prefix, source, work, environment, deadline, execute)
            document = prefix / "share/doc/git"
            if any(path.is_symlink() for path in (document, *document.parents)):
                raise tooling.ToolError("Git license/provenance path must not follow runtime links")
            document.mkdir(parents=True, exist_ok=True)
            with (
                (tree / "COPYING").open("rb") as incoming,
                (document / "COPYING").open("xb") as out,
            ):
                shutil.copyfileobj(incoming, out, length=tooling.CHUNK)
            evidence = {
                "schema": 1,
                "source": {"version": source.version, "url": source.url, "sha256": source.sha256},
                "architecture": architecture,
                "prefix": str(destination),
                "make_flags": flags,
                "license": {
                    "file": "share/doc/git/COPYING",
                    "sha256": bounded_sha256(document / "COPYING", MAX_SOURCE_FILE),
                    "upstream": "Git COPYING (GPL-2.0)",
                    "redistribution": (
                        "Retain COPYING and the exact corresponding source URL/digest; "
                        "this installer does not publish images."
                    ),
                },
                "omitted": ["Tcl/Tk GUI", "translated messages", "generated documentation"],
                "validated": [
                    "exact version",
                    "relocatable helper/template prefix",
                    "Perl module",
                    "HTTPS verified local CA",
                    "untrusted TLS CA refused",
                ],
                "runtime": records,
            }
            with (document / "boxferry-source.json").open("x") as output:
                output.write(json.dumps(evidence, indent=2) + "\n")
            _files(prefix, MAX_RUNTIME_BYTES, MAX_MEMBERS)
            if time.monotonic() > deadline or destination.exists() or destination.is_symlink():
                raise tooling.ToolError(
                    "Git deadline expired or destination appeared before publication"
                )
            _publish(prefix, destination)
            return evidence
    except (OSError, ValueError) as error:
        raise tooling.ToolError(f"Git source installation refused: {error}") from error
