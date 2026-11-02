#!/usr/bin/env python3
"""Finite caller for an already owned Paperless source fixture, never a driver.

The caller initializes a private executable inside the original owner session.
Only the existing preflight/inspect callback envelopes are accepted. No runtime
discovery, provisioning, image pull, document probe, target or cleanup action is
implemented. Failure requires the caller's existing whole-owner recovery.
"""
from __future__ import annotations

import contextlib
import fcntl
import hashlib
import hmac
import importlib.util
import io
import json
import os
import pathlib
import re
import secrets
import shlex
import socket
import stat
import struct
import sys
import tarfile
import tempfile
import time
import types

ROOT = pathlib.Path(__file__).resolve().parents[2]
LIMIT = 1_048_576
ARCHIVE_LIMIT = 2_684_354_560
SOURCE_NAMES = (
    "scripts/lib/docker-paperless-transport.py", "scripts/lib/docker-application-contract.py",
    "scripts/lib/docker-paperless-volume-custody.py",
    "scripts/lib/docker-core-artifact.py", "scripts/lib/docker-application-schedule.py",
    "scripts/lib/docker-application-expectations.py", "scripts/lib/docker-paperless-application.sh",
    "scripts/lib/paperless-application.sh", "scripts/lib/paperless-application-probes.sh",
    "fixtures/conformance/docker-application/expected-applications.json",
    "fixtures/conformance/paperless-ngx-application/compose.yaml",
    "fixtures/conformance/paperless-ngx-application/images.tsv",
    "fixtures/conformance/paperless-ngx-application/providers.tsv",
    "fixtures/conformance/paperless-ngx-application/document-probe.py",
)

# Fixed read-only guest budget observation. Refuse unavailable cgroup bounds.
BUDGET = r'''
set -eu
cpus=$(awk '/^processor[[:space:]]*:/ {n++} END {print n+0}' /proc/cpuinfo)
allowed=$(awk '/^Cpus_allowed_list:/ {print $2}' /proc/self/status | awk -F, '{n=0; for(i=1;i<=NF;i++){split($i,r,"-"); n+=(r[2]=="" ? 1 : r[2]-r[1]+1)} print n}')
if [ "$allowed" -lt "$cpus" ]; then cpus=$allowed; fi
read -r quota period < /sys/fs/cgroup/cpu.max
case "$quota" in max) ;; *)
  bounded=$(awk -v q="$quota" -v p="$period" 'BEGIN {if(p<=0) exit 1; print int(q/p)}')
  if [ "$bounded" -lt "$cpus" ]; then cpus=$bounded; fi ;;
esac
memory=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
read -r maximum < /sys/fs/cgroup/memory.max
read -r current < /sys/fs/cgroup/memory.current
case "$maximum" in max) ;; *)
  bounded=$(awk -v m="$maximum" -v c="$current" 'BEGIN {print int((m-c)/1024)}')
  if [ "$bounded" -lt "$memory" ]; then memory=$bounded; fi ;;
esac
temporary=$(df -Pk /tmp | awk 'END {print $4}')
graph=$(df -Pk /home/docker/.local/share/docker | awk 'END {print $4}')
printf '%s %s %s %s\n' "$cpus" "$memory" "$temporary" "$graph"
'''

# Existing canonical application-user check, with effective consumer exclusion,
# exclusive creation and exact inode cleanup. No supplied code/user/path exists.
STORAGE = r'''
import os,stat
from pathlib import Path
os.environ.setdefault("DJANGO_SETTINGS_MODULE", "paperless.settings")
import django
django.setup()
from django.conf import settings
from documents.management.commands.document_consumer import ConsumerFilter
from watchfiles import Change
def require(condition):
    if not condition: raise RuntimeError("storage access refused")
require(os.geteuid() == 1000 and os.getegid() == 1000)
consumer = ConsumerFilter(ignore_patterns=settings.CONSUMER_IGNORE_PATTERNS,
                          ignore_dirs=settings.CONSUMER_IGNORE_DIRS)
for name in ("data", "media", "consume", "export"):
    require(not os.path.lexists(Path("/usr/src/paperless") / name / ".boxferry-storage-probe"))
require(not consumer(Change.added, "/usr/src/paperless/consume/.boxferry-storage-probe"))
for name in ("data", "media", "consume", "export"):
    path = Path("/usr/src/paperless") / name
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    created = None
    sentinel = ".boxferry-storage-probe"
    try:
        directory = os.fstat(descriptor)
        require(directory.st_uid == directory.st_gid == 1000 and not directory.st_mode & 2)
        require(os.access(path, os.R_OK | os.W_OK) and not os.path.lexists(path / sentinel))
        # Check the ACTUAL configured v3.1.3 filter, not an assumed dotfile default.
        if name == "consume": require(not consumer(Change.added, str(path / sentinel)))
        fd = os.open(sentinel, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=descriptor)
        try:
            created = os.fstat(fd)
            require(stat.S_ISREG(created.st_mode) and created.st_uid == created.st_gid == 1000)
            require(stat.S_IMODE(created.st_mode) == 0o600 and created.st_nlink == 1)
        finally:
            os.close(fd)
        if name == "consume": require(not consumer(Change.added, str(path / sentinel)))
        current = path.stat(follow_symlinks=False)
        require((current.st_dev, current.st_ino) == (directory.st_dev, directory.st_ino))
    finally:
        try:
            if created is not None:
                current = os.stat(sentinel, dir_fd=descriptor, follow_symlinks=False)
                require((current.st_dev, current.st_ino) == (created.st_dev, created.st_ino))
                os.unlink(sentinel, dir_fd=descriptor)
                try: os.stat(sentinel, dir_fd=descriptor, follow_symlinks=False)
                except FileNotFoundError: pass
                else: raise RuntimeError("probe retained")
        finally:
            os.close(descriptor)
'''


class Refused(Exception):
    """Value-free refusal, including all native/helper/private failures."""


def need(condition):
    if not condition:
        raise Refused("Paperless caller refused")


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def private(path, *, directory=False, executable=False):
    need(type(path) is pathlib.PosixPath and path.is_absolute() and path.resolve(strict=True) == path)
    info = path.lstat()
    need(info.st_uid == os.geteuid() and stat.S_IMODE(info.st_mode) == (0o700 if directory or executable else 0o600)
         and (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode) and info.st_nlink == 1))
    return info


def stamp(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid, info.st_nlink,
            info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def read(path, *, executable=False):
    before = private(path, executable=executable)
    need(0 < before.st_size <= LIMIT)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        need(stamp(before) == stamp(os.fstat(source.fileno())))
        raw = source.read(LIMIT + 1)
        need(stamp(before) == stamp(os.fstat(source.fileno())) == stamp(path.lstat()) and len(raw) == before.st_size)
    return raw


def source_check(hashes):
    need(type(hashes) is dict and set(hashes) == set(SOURCE_NAMES))
    for name, expected in hashes.items():
        path = ROOT / name
        need(path.resolve(strict=True) == path and path.is_file()
             and type(expected) is str and re.fullmatch(r"[0-9a-f]{64}", expected) is not None)
        need(path.stat().st_size <= LIMIT and digest(path.read_bytes()) == expected)


def contract_module(hashes):
    source_check(hashes)
    path = ROOT / "scripts/lib/docker-application-contract.py"
    spec = importlib.util.spec_from_file_location("paperless_owner_contract", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def socket_stamp(path):
    need(path.is_absolute() and path.resolve(strict=True) == path)
    info = path.lstat()
    need(stat.S_ISSOCK(info.st_mode))
    return {"device": info.st_dev, "inode": info.st_ino, "uid": info.st_uid,
            "gid": info.st_gid, "mode": stat.S_IMODE(info.st_mode)}


def prepared_contract():
    # Reuse the hash-checked existing finite schema/settings validator. The
    # command dispatch is deliberately excluded; no live shell module is sourced.
    raw = (ROOT / "scripts/lib/docker-paperless-application.sh").read_text()
    code = raw.split("<< 'PY'\n", 1)[1].split("\ntry:\n    command, *args = sys.argv[1:]", 1)[0]
    scope = {}
    exec(compile(code, "held-paperless-source-contract", "exec"), scope)
    return scope


class Caller:
    """Original-session object; call(context, request_path, reply_path) is finite.

    Initialization selects the caller's exact existing manifest/config and source
    closure independently of every subsequent request. The executable restores
    these frozen facts without repeating initialization or renewing allowances.
    """

    def __init__(self, *, custody_path, custody_sha256, source_hashes, manifest_path, manifest_sha256,
                 profile, aliases, endpoint, helper_path, helper_revision, helper_sha256,
                 socket_identity, inner_peer):
        self.failed = True
        self.hashes = dict(source_hashes)
        self.contract = contract_module(self.hashes)
        self.custody_path, self.custody_sha = custody_path, custody_sha256
        self.owner = owner = self.contract.owner_read(custody_path, custody_sha256)
        need(owner["lane"] == "upstream-rootless"
             and owner["fixture_root"] == "/tmp/boxferry-fixture/" + owner["prefix"])
        self.directory = pathlib.Path(owner["directory"])
        self.manifest_path, self.manifest_sha = manifest_path, manifest_sha256
        self.protocol_directory = manifest_path.parent
        protocol_info = private(self.protocol_directory, directory=True)
        self.protocol_identity = (protocol_info.st_dev, protocol_info.st_ino)
        need(self.protocol_directory.parent == self.directory
             and re.fullmatch(r"boxferry-paperless\.[A-Za-z0-9]{8}", self.protocol_directory.name) is not None
             and manifest_path.name == "manifest.json")
        self.source = prepared_contract()
        self.document = self.contract.core_artifact.topology.document
        self.manifest_raw = read(manifest_path)
        need(digest(self.manifest_raw) == manifest_sha256)
        self.manifest = self.document(self.manifest_raw)
        config = self.manifest["config"]
        need(config["stage"] == "source-fixture" and config["target"] is None
             and config["prefix"] == owner["prefix"] and config["run_id"] == owner["run"]
             and config["fixture_root"] == owner["fixture_root"]
             and config["profile"] == profile and config["image_aliases"] == aliases and config["endpoint"] == endpoint)
        need(all(all(part not in {"", ".", ".."} for part in alias.split("/")) for alias in aliases.values()))
        need(profile["engine_release"] == "29.8.1" and profile["advertised_api_version"] == "1.56"
             and profile["acquisition_api_version"] == "1.49" and profile["rendering_api_version"] == "1.56")
        self.socket_path = pathlib.Path(owner["socket_directory"]) / "docker.sock"
        self.socket_identity = dict(socket_identity)
        need(endpoint == {"id": owner["context"], "uri": "unix://" + str(self.socket_path), "scope": "owned-inner",
                          "mode": "rootless", "ownership_sha256": custody_sha256})
        need(type(inner_peer) is dict and set(inner_peer) == {"pid", "start", "uid", "gid"}
             and all(type(value) is int and value >= 0 for value in inner_peer.values()))
        self.inner_peer = dict(inner_peer)
        self.helper_path, self.helper_sha = helper_path, helper_sha256
        self.http = self.contract.bind_unix_http(helper_path, helper_revision, helper_sha256)
        now, unix = self.contract.owner_clock(), time.time_ns() // 1_000_000
        work, cleanup = config["work_deadline_ms"], config["cleanup_deadline_ms"]
        need(type(work) is int and type(cleanup) is int and unix < work < cleanup)
        need(now + (work - unix) / 1000 <= owner["execution_deadline"]
             and now + (cleanup - unix) / 1000 <= owner["cleanup_deadline"])
        # Freeze the wall/BOOTTIME bridge once, conservatively; never renew it.
        self.cutoff = min(owner["execution_deadline"], now + (work - unix) / 1000)
        self.cleanup_cutoff = min(owner["cleanup_deadline"], now + (cleanup - unix) / 1000)
        need(now < self.cutoff < self.cleanup_cutoff and self.cleanup_cutoff <= owner["cleanup_deadline"])
        self.preflight = None
        self.bound_graph = None
        self.requests = 0
        self.response_bytes = 0
        self.executions = set()
        self.failed = False
        self.custody_sequence = self.custody_calls = self.custody_bytes = 0
        self.binding_sha = None
        self.check()
        self.validate_manifest()
        need(self.manifest["budget"]["image_archive_bytes"] == ARCHIVE_LIMIT)
        self.capability()
        self.verify_daemon()
        self.images()

    def check(self):
        need(not self.failed)
        source_check(self.hashes)
        protocol_info = private(self.protocol_directory, directory=True)
        need((protocol_info.st_dev, protocol_info.st_ino) == self.protocol_identity)
        need(self.contract.owner_read(self.custody_path, self.custody_sha) == self.owner
             and read(self.manifest_path) == self.manifest_raw
             and os.getsid(0) == self.owner["parent_pid"]
             and self.contract.process_start(self.owner["parent_pid"]) == self.owner["parent_start"]
             and self.contract.owner_clock() < self.cutoff
             and time.time_ns() // 1_000_000 < self.manifest["config"]["work_deadline_ms"]
             and socket_stamp(self.socket_path) == self.socket_identity
             and self.helper_path.resolve(strict=True) == self.helper_path
             and digest(self.helper_path.read_bytes()) == self.helper_sha)
        for name, expected in self.manifest["held"].items():
            need(pathlib.Path(name).name == name and digest(read(self.protocol_directory / name)) == expected)

    def validate_manifest(self):
        # Reconstruct expected source evidence with the existing canonical
        # prepare function, not values learned from runtime or a reply.
        with tempfile.TemporaryDirectory(prefix="paperless-review.", dir=self.directory) as temporary:
            review = pathlib.Path(temporary)
            config = review / "config.json"
            config.write_bytes(encoded(self.manifest["config"]))
            config.chmod(0o600)
            with contextlib.redirect_stdout(io.StringIO()):
                self.source["initialize"](str(ROOT), str(review), str(config))
            expected = self.document(read(review / "manifest.json"))
            observed = self.document(self.manifest_raw)
            for name in ("plan", "admission"):
                need(observed["config"][name] == str(self.protocol_directory / (name + ".json")))
                observed["config"][name] = expected["config"][name]
            need(set(observed["sources"]) == set(expected["sources"]))
            for name in expected["sources"]:
                need(observed["sources"][name] == str(self.protocol_directory / pathlib.Path(expected["sources"][name]).name))
            observed["sources"] = expected["sources"]
            need(encoded(observed) == encoded(expected))
        self.check()

    def capability(self):
        self.check()
        self.contract.owner_capability(self.custody_path, self.custody_sha, phase="ready", context=self.owner["context"],
            execution_deadline=self.owner["execution_deadline"], cleanup_deadline=self.owner["cleanup_deadline"])
        self.check()

    def volume_check(self):
        """A saved sequence can only challenge the live original guard lease."""
        self.check()
        need(type(self.binding_sha) is str and self.owner.get("paperless_sources") == self.hashes)
        holding = self.custody_sequence == 0
        call_bound = 6 if holding else 3
        read_bound = (2 if holding else 1) * (5 * LIMIT + 34 * 4096 + 2 * 65536 + 16384 + 256 * 4096 + 42)
        # Refuse BEFORE work unless its complete worst case fits the original
        # remaining aggregate allowance; successful replies charge actual bytes.
        need(self.requests + call_bound <= 256 and self.response_bytes + read_bound <= 16 * LIMIT)
        result = self.contract.owner_capability(self.custody_path, self.custody_sha,
            phase="paperless-hold" if holding else "paperless-check",
            context=self.owner["context"], execution_deadline=self.owner["execution_deadline"],
            cleanup_deadline=self.owner["cleanup_deadline"], paperless_binding=self.binding_sha,
            paperless_sequence=self.custody_sequence)
        need(result["sequence"] == self.custody_sequence + 1 and result["calls"] - self.custody_calls == call_bound
             and 0 <= result["read_bytes"] - self.custody_bytes <= read_bound)
        self.custody_sequence = result["sequence"]
        # The existing aggregate native-call/read limits also pay for custody.
        self.requests += result["calls"] - self.custody_calls
        self.response_bytes += result["read_bytes"] - self.custody_bytes
        need(self.requests <= 256 and self.response_bytes <= 16 * LIMIT)
        self.custody_calls, self.custody_bytes = result["calls"], result["read_bytes"]
        self.check()
    def peer(self, stream):
        observed = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        expected = self.inner_peer
        need(observed == (expected["pid"], expected["uid"], expected["gid"]))
        descriptor = self.contract.verified_parent_fd(expected["pid"], expected["start"])
        caller = self
        class Lease:
            def check(self):
                caller.check()
                need(caller.contract.parent_alive(descriptor))
            def reserve_read(self, _size):
                self.check()
            def close(self):
                try:
                    self.check()
                finally:
                    os.close(descriptor)
        lease = Lease()
        try:
            lease.check()
        except BaseException:
            os.close(descriptor)
            raise
        return lease

    def get(self, tail):
        self.check()
        # Constructed internally only: no callback-supplied method, path or body.
        need(tail in {"version", "info"} or tail in self.get_paths())
        self.requests += 1
        need(self.requests <= 256)
        status, raw = self.http.request(str(self.socket_path), "GET", "/v1.56/" + tail,
            timeout=min(15, self.cutoff - self.contract.owner_clock()), max_response_bytes=LIMIT,
            max_request_bytes=LIMIT, peer_lease=self.peer)
        self.check()
        need(status == 200 and type(raw) is bytes and len(raw) <= LIMIT)
        self.response_bytes += len(raw)
        need(self.response_bytes <= 16 * LIMIT)
        return self.document(raw)

    def get_paths(self):
        config, app = self.manifest["config"], self.manifest["application"]
        paths = {"images/" + image + "/json" for image in (*config["image_aliases"].values(), *self.manifest["images"].values())}
        for group, kind in (("networks", "networks"), ("volumes", "volumes"), ("services", "containers")):
            for node in app[group].values():
                paths.add(kind + "/" + config["prefix"] + "-" + node["runtime_suffix"] + ("/json" if kind == "containers" else ""))
        paths.update("exec/" + identity + "/json" for identity in self.executions)
        return paths

    def native(self, arguments, *, archive_limit=None):
        references = set(self.manifest["images"].values())
        outer = self.contract.owner_cid(self.owner)
        need(outer is not None)
        readonly = (len(arguments) == 2 and arguments[0] == "image-info" and arguments[1] in references
                    or arguments == ["exec", outer, "sh", "-ec", BUDGET])
        save = (len(arguments) == 3 and arguments[0] == "image-save" and arguments[1] in references
                and arguments[2] == str(self.directory / "busybox.tar")
                and type(archive_limit) is int and 0 < archive_limit <= ARCHIVE_LIMIT)
        need(save or readonly and archive_limit is None)
        self.capability()
        self.requests += 1
        need(self.requests <= 256)
        status, raw = self.contract.owner_call(self.owner, arguments, self.cutoff,
            paperless_archive_limit=archive_limit, paperless_caller=True)
        self.check()
        need(status == 0)
        self.response_bytes += len(raw)
        need(self.response_bytes <= 16 * LIMIT)
        return raw

    def verify_daemon(self):
        version, info = self.get("version"), self.get("info")
        need(version["Version"] == "29.8.1" and version["ApiVersion"] == "1.56"
             and type(info["SecurityOptions"]) is list
             and any(value == "name=rootless" or value.startswith("name=rootless,") for value in info["SecurityOptions"])
             and info["DockerRootDir"] == "/home/docker/.local/share/docker")

    def images(self):
        bindings = {}
        config = self.manifest["config"]
        for key, reference in self.manifest["images"].items():
            alias, expected = config["image_aliases"][key], config["image_config_ids"][key]
            original, aliased = self.get("images/" + reference + "/json"), self.get("images/" + alias + "/json")
            need(original["Id"] == aliased["Id"] == expected
                 and reference.split("@", 1)[0].split(":", 1)[0] + "@" + reference.split("@", 1)[1] in original["RepoDigests"]
                 and original["Os"] == aliased["Os"] == "linux"
                 and original["Architecture"] == aliased["Architecture"] in {"amd64", "arm64"})
            bindings[key] = {"reference": reference, "alias": alias, "config_id": expected}
        return bindings

    def graph(self):
        self.volume_check()
        self.verify_daemon()
        bindings = self.images()
        config, app = self.manifest["config"], self.manifest["application"]
        labels = {"io.boxferry.live-run": config["run_id"], "io.boxferry.application": config["prefix"] + "-paperless"}
        resources, native = {}, {}
        for group, kind in (("networks", "network"), ("volumes", "volume"), ("services", "container")):
            for role, node in app[group].items():
                name = config["prefix"] + "-" + node["runtime_suffix"]
                observed = self.get(group.replace("services", "containers") + "/" + name + ("/json" if kind == "container" else ""))
                need(observed["Name"].removeprefix("/") == name
                     and (observed["Config"]["Labels"] if kind == "container" else observed["Labels"]) == labels)
                if kind == "network":
                    need(observed["Driver"] == "bridge" and observed["Internal"] is node["internal"]
                         and observed.get("Options") in ({}, None) and observed.get("Scope") == "local")
                    identity = observed["Id"]
                elif kind == "volume":
                    need(observed["Driver"] == "local" and observed.get("Options") in ({}, None)
                         and observed.get("Scope") == "local" and type(observed["Mountpoint"]) is str
                         and observed["Mountpoint"] == "/home/docker/.local/share/docker/volumes/" + name + "/_data")
                    identity = digest(encoded(observed))
                else:
                    identity = observed["Id"]
                    need(observed["State"]["Running"] is True and observed["Image"] == bindings[node["image_key"]]["config_id"]
                         and observed["Config"]["Image"] == bindings[node["image_key"]]["alias"])
                    settings = self.manifest["settings"][role]
                    environment = observed["Config"].get("Env", [])
                    need(len(environment) == len({item.split("=", 1)[0] for item in environment})
                         and all("=" in item for item in environment))
                    # Image defaults are allowed only when their actual image
                    # config supplies the same exact defaults; authored fields win.
                    image = self.get("images/" + bindings[node["image_key"]]["alias"] + "/json")["Config"]
                    expected_env = dict(item.split("=", 1) for item in image.get("Env", []))
                    expected_env.update(settings["environment"])
                    need(dict(item.split("=", 1) for item in environment) == expected_env
                         and observed["Config"].get("Cmd") == (settings["command"] if settings["command"] is not None else image.get("Cmd")))
                    for field in ("Entrypoint", "User", "WorkingDir", "StopSignal"):
                        need(observed["Config"].get(field) == image.get(field))
                    plan = self.document(read(pathlib.Path(config["plan"])))
                    authored = next(row["body"] for row in plan["requests"]
                                    if row["path"].endswith("containers/create?name=" + name))
                    need(observed["Config"].get("Healthcheck") == authored.get("Healthcheck", image.get("Healthcheck")))
                    healthy = role in {"db", "broker"}
                    if healthy:
                        need(observed["State"]["Health"]["Status"] == "healthy")
                    row_settings = {"environment": dict(settings["environment"]), "command": settings["command"], "pull_policy": "never"}
                need(type(identity) is str and re.fullmatch(r"[0-9a-f]{64}", identity) is not None)
                row = {"kind": kind, "identity": identity, "labels": dict(labels), "specification": node}
                if kind == "container":
                    row.update(image_config_id=observed["Image"], running=observed["State"]["Running"], healthy=healthy, settings=row_settings)
                resources[name], native[name] = row, observed
        for role, node in app["services"].items():
            name = config["prefix"] + "-" + node["runtime_suffix"]
            observed = native[name]
            networks = observed["NetworkSettings"]["Networks"]
            need(set(networks) == {config["prefix"] + "-" + app["networks"][key]["runtime_suffix"] for key in node["networks"]})
            for network, aliases in node["networks"].items():
                network_name = config["prefix"] + "-" + app["networks"][network]["runtime_suffix"]
                actual_aliases = set(networks[network_name]["Aliases"])
                need(networks[network_name]["NetworkID"] == resources[network_name]["identity"]
                     and set(aliases) <= actual_aliases <= set(aliases) | {name, observed["Id"][:12]})
            wanted = []
            for mount in node["mounts"]:
                source = config["fixture_root"] if mount["kind"] == "bind" else config["prefix"] + "-" + app["volumes"][mount["source"]]["runtime_suffix"]
                wanted.append((mount["kind"], source, mount["target"], not mount["read_only"]))
            actual = [(row["Type"], row["Source"] if row["Type"] == "bind" else row["Name"], row["Destination"], row["RW"]) for row in observed["Mounts"]]
            need(sorted(actual) == sorted(wanted))
            for row in observed["Mounts"]:
                if row["Type"] == "volume":
                    need(row["Driver"] == "local" and row["Source"] == native[row["Name"]]["Mountpoint"])
            ports = observed["NetworkSettings"].get("Ports", {})
            need({key: value for key, value in ports.items() if value is not None} == ({"8000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "18000"}]} if node["ingress"] else {}))
            host = observed["HostConfig"]
            need(not host.get("Privileged", False) and host.get("PidMode", "") == ""
                 and host.get("IpcMode") in ("private", "") and host.get("CapAdd") in (None, [])
                 and not host.get("PublishAllPorts", False) and host.get("Devices") in (None, []))
            backend = config["prefix"] + "-paper-backend"
            need(host["NetworkMode"] in {backend, resources[backend]["identity"]})
        # Network membership closes excluded peers; labels alone are insufficient.
        for role, node in app["networks"].items():
            name = config["prefix"] + "-" + node["runtime_suffix"]
            expected = {resources[config["prefix"] + "-" + service["runtime_suffix"]]["identity"]
                        for service in app["services"].values() if role in service["networks"]}
            need(set(native[name]["Containers"]) == expected)
        need(len({row["identity"] for row in resources.values()}) == len(resources))
        self.volume_check()
        return {"resources": resources, "dependencies": app["dependencies"], "storage": {}, "image_bindings": bindings}

    def archive_bytes(self):
        # Every file was produced by the existing pinned-reference save action;
        # no caller-supplied numeric size, image Size field or cached path is proof.
        staged, remaining = [], ARCHIVE_LIMIT
        bundle = self.directory / "paperless-images.tar"
        need(not os.path.lexists(bundle))
        try:
            for key, binding in self.images().items():
                self.check()
                current = self.directory / "busybox.tar"
                retained = self.directory / ("paperless-archive-" + key + ".tar")
                need(not os.path.lexists(current) and not os.path.lexists(retained))
                expected = binding["config_id"]
                def observe():
                    value = self.document(self.native(["image-info", binding["reference"]]))
                    need(value["Id"] == expected and value["Os"] == "linux" and value["Architecture"] in {"amd64", "arm64"})
                observe()
                self.native(["image-save", binding["reference"], str(current)], archive_limit=remaining)
                before = private(current)
                staged.append((current, before))
                need(0 < before.st_size <= remaining)
                checksum = self.file_digest(current, before)
                observe()
                need(self.images() == self.manifest_images())
                os.rename(current, retained)
                renamed = private(retained)
                need(stamp(renamed)[:-1] == stamp(before)[:-1])
                before = renamed
                staged[-1] = (retained, renamed)
                remaining -= before.st_size
                need(remaining > 0 and self.file_digest(retained, before) == checksum)
            fd = os.open(bundle, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            bundle_info = os.fstat(fd)
            staged.append((bundle, bundle_info))
            caller = self
            class BoundedWriter:
                position = 0
                def write(self, raw):
                    caller.check()
                    need(self.position + len(raw) <= ARCHIVE_LIMIT)
                    written = output.write(raw)
                    self.position += written
                    return written
                def tell(self):
                    return self.position
            with os.fdopen(fd, "wb") as output:
                with tarfile.open(fileobj=BoundedWriter(), mode="w|") as archive:
                    for path, info in staged[:-1]:
                        need(stamp(path.lstat()) == stamp(info))
                        entry = tarfile.TarInfo(path.name)
                        entry.size, entry.mode, entry.mtime = info.st_size, 0o600, 0
                        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
                        with os.fdopen(descriptor, "rb") as source:
                            need(stamp(os.fstat(source.fileno())) == stamp(info))
                            archive.addfile(entry, source)
                            need(stamp(os.fstat(source.fileno())) == stamp(info))
                        need(stamp(path.lstat()) == stamp(info))
                output.flush()
                os.fsync(output.fileno())
            actual = private(bundle)
            need((actual.st_dev, actual.st_ino) == (bundle_info.st_dev, bundle_info.st_ino)
                 and 0 < actual.st_size <= ARCHIVE_LIMIT)
            self.archive_digest = self.file_digest(bundle, actual)
            need(self.images() == self.manifest_images())
            return actual.st_size
        finally:
            # Incomplete/changed artifacts retain whole-owner custody. Never
            # delete a replacement or translate uncertain cleanup into success.
            for path, before in reversed(staged):
                info = private(path)
                need((info.st_dev, info.st_ino) == (before.st_dev, before.st_ino))
                path.unlink()
                need(not os.path.lexists(path))

    def file_digest(self, path, expected):
        need(stamp(private(path)) == stamp(expected))
        value = hashlib.sha256()
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, "rb") as stream:
            need(stamp(os.fstat(stream.fileno())) == stamp(expected))
            while True:
                self.check()
                raw = stream.read(1_048_576)
                if not raw:
                    break
                value.update(raw)
            need(stamp(os.fstat(stream.fileno())) == stamp(expected) == stamp(path.lstat()))
        return value.hexdigest()

    def manifest_images(self):
        config = self.manifest["config"]
        return {key: {"reference": value, "alias": config["image_aliases"][key], "config_id": config["image_config_ids"][key]}
                for key, value in self.manifest["images"].items()}

    def perform_preflight(self):
        graph = self.graph()
        raw = self.native(["exec", self.contract.owner_cid(self.owner), "sh", "-ec", BUDGET])
        need(re.fullmatch(rb"[1-9][0-9]{0,18}(?: [1-9][0-9]{0,18}){3}\n", raw) is not None)
        cpus, memory, temporary, graph_disk = map(int, raw.split())
        disk = os.statvfs(self.directory)
        budget = {"cpus": cpus, "memory_kib": memory,
                  "disk_kib": min(temporary, graph_disk, disk.f_bavail * disk.f_frsize // 1024),
                  "image_archive_bytes": 0}
        expected = self.manifest["budget"]
        need(all(budget[key] >= expected[key] for key in ("cpus", "memory_kib", "disk_kib"))
             and disk.f_bavail * disk.f_frsize >= 2 * ARCHIVE_LIMIT)
        budget["image_archive_bytes"] = self.archive_bytes()
        need(budget["image_archive_bytes"] <= expected["image_archive_bytes"])
        self.bound_graph = graph
        return {"collisions": [], "process_tree_watchdog": True, "absolute_deadlines": True,
                "cleanup_reserved": True, "no_fallback": True, "no_implicit_pulls": True, "budget": budget}

    def perform_inspect(self):
        need(self.preflight is not None and self.bound_graph is not None)
        before = self.graph()
        need(encoded(before) == encoded(self.bound_graph))
        web = self.manifest["config"]["prefix"] + "-paper-web"
        cid = before["resources"][web]["identity"]
        self.storage_access(cid)
        after = self.graph()
        need(encoded(before) == encoded(after))
        prefix = self.manifest["config"]["prefix"]
        after["storage"] = {prefix + "-paper-" + name: {"uid": 1000, "gid": 1000, "world_writable": False,
            "application_uid": 1000, "application_gid": 1000, "read": True, "write": True, "create_remove": True}
            for name in ("data", "media", "consume", "export")}
        self.source["graph"](self.manifest, after, None)
        return after

    def storage_access(self, cid):
        self.volume_check()
        need(self.preflight is not None and self.bound_graph is not None
             and cid == self.bound_graph["resources"][self.owner["prefix"] + "-paper-web"]["identity"])
        create = {"AttachStdin": False, "AttachStdout": False, "AttachStderr": False,
                  "Tty": False, "User": "1000:1000", "Cmd": ["python3", "-c", STORAGE]}
        # Fixed ExecCreate/Start only; the HTTP helper verifies the actual peer
        # before sending each request, including these two narrow mutations.
        self.capability()
        def post(tail, body):
            self.check()
            self.requests += 1
            need(self.requests <= 256)
            status, raw = self.http.request(str(self.socket_path), "POST", "/v1.56/" + tail, encoded(body),
                timeout=min(15, self.cutoff - self.contract.owner_clock()), max_response_bytes=LIMIT,
                max_request_bytes=LIMIT, content_type="application/json", peer_lease=self.peer)
            self.check()
            self.response_bytes += len(raw)
            need(status in {200, 201} and len(raw) <= LIMIT and self.response_bytes <= 16 * LIMIT)
            return raw
        created = self.document(post("containers/" + cid + "/exec", create))
        need(set(created) == {"Id"} and type(created["Id"]) is str
             and re.fullmatch(r"[0-9a-f]{64}", created["Id"]) is not None and created["Id"] not in self.executions)
        identity = created["Id"]
        self.executions.add(identity)
        self.capability()
        need(post("exec/" + identity + "/start", {"Detach": True, "Tty": False}) == b"")
        while True:
            value = self.get("exec/" + identity + "/json")
            need(value["ID"] == identity and value["ContainerID"] == cid and type(value["Running"]) is bool)
            if not value["Running"]:
                need(type(value["ExitCode"]) is int and value["ExitCode"] == 0)
                break
            self.check()
            time.sleep(min(0.02, max(0, self.cutoff - self.contract.owner_clock())))
        self.volume_check()

    def call(self, context, request_path, reply_path):
        """Existing three-argument callback. Every other action fails closed."""
        try:
            self.check()
            need(context == self.owner["context"] and request_path == self.protocol_directory / "request.json"
                 and reply_path == self.protocol_directory / "reply.json")
            raw = read(request_path)
            request = self.document(raw)
            config = self.manifest["config"]
            need(set(request) == {"schema_version", "action", "arguments", "stage", "manifest", "manifest_sha256",
                 "endpoint", "deadline_ms", "work_deadline_ms", "cleanup_deadline_ms"}
                 and type(request["schema_version"]) is int and request["schema_version"] == 1
                 and request["action"] in {"preflight", "inspect"} and request["arguments"] == []
                 and request["stage"] == "source-fixture" and request["manifest"] == str(self.manifest_path)
                 and request["manifest_sha256"] == self.manifest_sha and request["endpoint"] == config["endpoint"]
                 and type(request["deadline_ms"]) is int and request["deadline_ms"] == config["work_deadline_ms"]
                 and type(request["work_deadline_ms"]) is int and request["work_deadline_ms"] == config["work_deadline_ms"]
                 and type(request["cleanup_deadline_ms"]) is int and request["cleanup_deadline_ms"] == config["cleanup_deadline_ms"])
            reply_info = private(reply_path)
            need(reply_info.st_size == 0)
            self.capability()
            self.volume_check()
            if request["action"] == "preflight":
                need(self.preflight is None)
                payload = self.perform_preflight()
                self.preflight = payload
            else:
                payload = self.perform_inspect()
            self.capability()
            self.volume_check()
            need(read(request_path) == raw and stamp(reply_path.lstat()) == stamp(reply_info))
            reply = {"schema_version": 1, "request_sha256": digest(raw), "manifest_sha256": self.manifest_sha,
                     "action": request["action"], "stage": "source-fixture", "endpoint": config["endpoint"], "payload": payload}
            reply_raw = encoded(reply)
            need(len(reply_raw) <= LIMIT)
            descriptor = os.open(reply_path, os.O_WRONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(descriptor, "wb") as output:
                need(stamp(os.fstat(output.fileno())) == stamp(reply_info))
                output.write(reply_raw)
                output.flush()
            self.check()
            return None
        except BaseException:
            self.failed = True
            if self.custody_sequence:
                try:
                    self.contract.owner_capability(self.custody_path, self.custody_sha, phase="paperless-close",
                        context=self.owner["context"], execution_deadline=self.owner["execution_deadline"],
                        cleanup_deadline=self.owner["cleanup_deadline"], paperless_binding=self.binding_sha,
                        paperless_sequence=self.custody_sequence)
                except BaseException:
                    pass  # Original owner recovery still closes every known FD.
            raise Refused("Paperless caller refused") from None


def document(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            need(key not in result)
            result[key] = value
        return result
    need(type(raw) is bytes and 0 < len(raw) <= LIMIT)
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=lambda _value: need(False))


def write_new(path, raw, mode=0o600):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(descriptor, "wb") as output:
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())


def wrapper(path, binding_sha):
    return ("#!/bin/sh\nexec " + shlex.quote(sys.executable) + " "
            + shlex.quote(str(ROOT / SOURCE_NAMES[0])) + " --binding " + shlex.quote(str(path))
            + " --binding-sha256 " + binding_sha + ' "$@"\n').encode()


def caller_state(caller):
    return {"failed": caller.failed, "preflight": caller.preflight, "bound_graph": caller.bound_graph,
            "requests": caller.requests, "response_bytes": caller.response_bytes,
            "executions": sorted(caller.executions), "archive_digest": getattr(caller, "archive_digest", None),
            "custody_sequence": caller.custody_sequence, "custody_calls": caller.custody_calls,
            "custody_bytes": caller.custody_bytes}


class Bridge:
    """Paperless-only private callback ledger, not an operation dispatcher."""

    def __init__(self, path, binding_sha):
        raw = read(path)
        need(digest(raw) == binding_sha)
        self.binding = b = document(raw)
        need(type(b) is dict and set(b) == {"arguments", "owner", "protocol_identity", "directory_identity",
             "lock_identity", "cutoff", "cleanup_cutoff", "ledger_key"})
        self.path, self.binding_sha, self.directory = path, binding_sha, path.parent
        self.check()
        self.state_stamp = None

    def check(self):
        b = self.binding
        info = private(self.directory, directory=True)
        need(self.path.name == "binding.json" and self.directory == pathlib.Path(b["owner"]["directory"]) / "paperless-transport"
             and [info.st_dev, info.st_ino] == b["directory_identity"] and digest(read(self.path)) == self.binding_sha)
        executable = self.directory / "transport"
        need(read(executable, executable=True) == wrapper(self.path, self.binding_sha))

    def state_bytes(self, data):
        body = {"binding_sha256": self.binding_sha, "data": data}
        raw = encoded({**body, "mac": hmac.new(bytes.fromhex(self.binding["ledger_key"]), encoded(body), hashlib.sha256).hexdigest()})
        need(len(raw) <= LIMIT)
        return raw

    def save(self, data):
        self.check()
        path = self.directory / "state.json"
        need((stamp(private(path)) if os.path.lexists(path) else None) == self.state_stamp)
        raw = self.state_bytes(data)
        temporary = self.directory / (".state-" + secrets.token_hex(16))
        write_new(temporary, raw)
        published = stamp(private(temporary))
        publication = os.open(temporary, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            need(stamp(os.fstat(publication)) == published)
            replaced = False
            try:
                os.replace(temporary, path)
                replaced = True
                current = stamp(private(path))
                # Rename may change ctime; never adopt a foreign named inode.
                need(current[:-1] == published[:-1])
                self.state_stamp = current
                descriptor = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                try:
                    os.fsync(descriptor)
                finally:
                    os.close(descriptor)
                need(stamp(private(path)) == self.state_stamp)
            except BaseException:
                if replaced and data["failed"] is False:
                    # Metadata reads and directory sync can fail after rename.
                    # Poison ONLY the still-held inode we created, even if an
                    # unrelated file has replaced its name. Truncation first
                    # prevents a short write from retaining old clean bytes.
                    try:
                        poison = self.state_bytes({**data, "failed": True})
                        os.lseek(publication, 0, os.SEEK_SET)
                        os.ftruncate(publication, 0)
                        need(os.write(publication, poison) == len(poison))
                        os.fsync(publication)
                        self.state_stamp = stamp(os.fstat(publication))
                    except BaseException:
                        # The original failure still requires whole-owner
                        # recovery; uncertain repair never supplies success.
                        pass
                raise
        finally:
            os.close(publication)

    def load(self):
        path = self.directory / "state.json"
        before = stamp(private(path))
        value = document(read(path))
        need(stamp(private(path)) == before and type(value) is dict and set(value) == {"binding_sha256", "data", "mac"}
             and value["binding_sha256"] == self.binding_sha and type(value["mac"]) is str)
        body = {"binding_sha256": value["binding_sha256"], "data": value["data"]}
        expected = hmac.new(bytes.fromhex(self.binding["ledger_key"]), encoded(body), hashlib.sha256).hexdigest()
        need(hmac.compare_digest(expected, value["mac"]))
        data = value["data"]
        need(type(data) is dict and set(data) == {"failed", "preflight", "bound_graph", "requests", "response_bytes", "executions", "archive_digest",
                                                "custody_sequence", "custody_calls", "custody_bytes"}
             and all(type(data[key]) is int and 0 <= data[key] <= bound
                     for key, bound in (("custody_sequence", 256), ("custody_calls", 256), ("custody_bytes", 16 * LIMIT)))
             and data["failed"] is False and type(data["requests"]) is int and 0 <= data["requests"] <= 256
             and type(data["response_bytes"]) is int and 0 <= data["response_bytes"] <= 16 * LIMIT
             and type(data["executions"]) is list and len(data["executions"]) == len(set(data["executions"])) <= 256
             and all(type(value) is str and re.fullmatch(r"[0-9a-f]{64}", value) for value in data["executions"])
             and (data["preflight"] is None and data["bound_graph"] is None and data["archive_digest"] is None
                  or type(data["preflight"]) is dict and type(data["bound_graph"]) is dict
                  and type(data["archive_digest"]) is str and re.fullmatch(r"[0-9a-f]{64}", data["archive_digest"])))
        self.state_stamp = before
        return data

    def restore(self, data):
        b = self.binding
        arguments = b["arguments"]
        caller = Caller.__new__(Caller)
        caller.hashes = arguments["source_hashes"]
        caller.contract = contract_module(caller.hashes)
        caller.owner = b["owner"]
        caller.directory = pathlib.Path(caller.owner["directory"])
        caller.custody_path, caller.custody_sha = pathlib.Path(arguments["custody_path"]), arguments["custody_sha256"]
        caller.manifest_path, caller.manifest_sha = pathlib.Path(arguments["manifest_path"]), arguments["manifest_sha256"]
        caller.protocol_directory = caller.manifest_path.parent
        caller.protocol_identity = tuple(b["protocol_identity"])
        caller.manifest_raw = read(caller.manifest_path)
        need(digest(caller.manifest_raw) == caller.manifest_sha)
        caller.source = prepared_contract()
        caller.document = caller.contract.core_artifact.topology.document
        caller.manifest = caller.document(caller.manifest_raw)
        caller.socket_path = pathlib.Path(caller.owner["socket_directory"]) / "docker.sock"
        caller.socket_identity, caller.inner_peer = arguments["socket_identity"], arguments["inner_peer"]
        caller.helper_path, caller.helper_sha = pathlib.Path(arguments["helper_path"]), arguments["helper_sha256"]
        raw = read(self.directory / "native-unix-http.py")
        need(digest(raw) == caller.helper_sha)
        caller.http = types.ModuleType("held_paperless_unix_http")
        caller.http.__file__ = str(self.directory / "native-unix-http.py")
        exec(compile(raw, caller.http.__file__, "exec"), caller.http.__dict__)
        caller.cutoff, caller.cleanup_cutoff = b["cutoff"], b["cleanup_cutoff"]
        need(type(caller.cutoff) in (int, float) and type(caller.cleanup_cutoff) in (int, float)
             and caller.cutoff < caller.cleanup_cutoff <= caller.owner["cleanup_deadline"]
             and caller.cutoff <= caller.owner["execution_deadline"])
        caller.binding_sha = self.binding_sha
        for name in ("failed", "preflight", "bound_graph", "requests", "response_bytes", "archive_digest",
                     "custody_sequence", "custody_calls", "custody_bytes"):
            setattr(caller, name, data[name])
        caller.executions = set(data["executions"])
        caller.check()
        # Preserve the original SID, with a child PGID the owner guard can quiesce.
        group, parent = os.getpgrp(), os.getpid()
        need(group != caller.owner["parent_pid"])
        while parent != group:
            fields = pathlib.Path(f"/proc/{parent}/stat").read_text().rsplit(")", 1)[1].split()
            need(int(fields[3]) == caller.owner["parent_pid"])
            parent = int(fields[1])
            need(parent > 1 and parent != caller.owner["parent_pid"])
        need(os.getsid(group) == caller.owner["parent_pid"])
        return caller


def initialize(**arguments):
    """Freeze one verified caller into a private executable; create no runtime."""
    try:
        caller = Caller(**arguments)
        directory = caller.directory / "paperless-transport"
        directory.mkdir(mode=0o700)
        info = private(directory, directory=True)
        write_new(directory / "native-unix-http.py", caller.helper_path.read_bytes())
        need(digest(read(directory / "native-unix-http.py")) == caller.helper_sha)
        lock = directory / "lock"
        write_new(lock, b"")
        binding = {"arguments": {key: str(value) if isinstance(value, pathlib.Path) else value for key, value in arguments.items()},
            "owner": caller.owner, "protocol_identity": list(caller.protocol_identity),
            "directory_identity": [info.st_dev, info.st_ino], "lock_identity": list(stamp(private(lock))),
            "cutoff": caller.cutoff, "cleanup_cutoff": caller.cleanup_cutoff, "ledger_key": secrets.token_hex(32)}
        path = directory / "binding.json"
        raw = encoded(binding)
        need(len(raw) <= LIMIT)
        write_new(path, raw)
        binding_sha = digest(raw)
        caller.binding_sha = binding_sha
        write_new(directory / "transport", wrapper(path, binding_sha), 0o700)
        bridge = Bridge(path, binding_sha)
        bridge.save(caller_state(caller))
        return directory / "transport"
    except BaseException:
        raise Refused("Paperless caller refused") from None


def main():
    os.umask(0o077)
    try:
        need(len(sys.argv) == 8 and sys.argv[1] == "--binding" and sys.argv[3] == "--binding-sha256"
             and re.fullmatch(r"[0-9a-f]{64}", sys.argv[4]) is not None)
        bridge = Bridge(pathlib.Path(sys.argv[2]), sys.argv[4])
        lock_path = bridge.directory / "lock"
        before = private(lock_path)
        need(list(stamp(before)) == bridge.binding["lock_identity"])
        descriptor = os.open(lock_path, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            need(stamp(os.fstat(descriptor)) == stamp(before) == stamp(private(lock_path)))
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            data = bridge.load()
            # Crash, signal or uncertain work must never leave a reusable ledger.
            bridge.save({**data, "failed": True})
            caller = bridge.restore(data)
            try:
                caller.call(sys.argv[5], pathlib.Path(sys.argv[6]), pathlib.Path(sys.argv[7]))
                bridge.save(caller_state(caller))
                caller.check()
            except BaseException:
                caller.failed = True
                bridge.save(caller_state(caller))
                raise
        finally:
            os.close(descriptor)
        return 0
    except BaseException:
        print("Paperless caller refused; private owner recovery remains authoritative.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
