#!/usr/bin/env bash
# Test-only inner-Docker Forgejo profile. Sourcing defines one entry point only.
# See fixtures/conformance/forgejo-application/docker-profile-contract.md before supplying a transport.
docker_forgejo_application_run() (
  set +x
  if [[ $# != 16 ]]; then
    printf '%s\n' 'Forgejo profile rejected: argument contract differs.' >&2
    return 2
  fi
  python3 - "$@" << 'PY'
"""No Docker discovery: all native access belongs to the explicit transport."""
import hashlib
import importlib.util
import ipaddress
import json
import os
import pathlib
import re
import selectors
import signal
import subprocess
import sys
import time


class Refused(Exception):
    pass


class OwnershipRefused(Refused):
    pass


class Cancelled(Refused):
    pass


def require(condition):
    if not condition:
        raise Refused()


def clock():
    return time.clock_gettime(time.CLOCK_BOOTTIME)


def encoded(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True).encode()


cancelled = False
cleaning = False


def cancel(_number, _frame):
    global cancelled
    cancelled = True
    if not cleaning:
        raise Cancelled()


for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT):
    signal.signal(number, cancel)


# Capture both streams without retaining unbounded/private diagnostics. A new
# session lets timeout/cancellation reap the entire owned transport process group.
def execute(argv, payload, deadline, statuses=(0,), return_status=False):
    require(clock() < deadline)
    environment = {key: value for key, value in os.environ.items()
                   if key not in {"BASH_ENV", "ENV", "SHELLOPTS", "BASHOPTS"}}
    process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True, env=environment)
    output = bytearray()
    size = 0
    try:
        # Payload is at most the already bounded plan; nonblocking input prevents
        # a transport which refuses to read from defeating the original deadline.
        selector = selectors.DefaultSelector()
        for stream in (process.stdout, process.stderr, process.stdin):
            os.set_blocking(stream.fileno(), False)
        selector.register(process.stdout, selectors.EVENT_READ, "out")
        selector.register(process.stderr, selectors.EVENT_READ, "err")
        pending = memoryview(payload)
        if pending:
            selector.register(process.stdin, selectors.EVENT_WRITE, "in")
        else:
            process.stdin.close()
        try:
            while selector.get_map():
                require(clock() < deadline)
                for key, _events in selector.select(min(0.05, max(0, deadline - clock()))):
                    if key.data == "in":
                        try:
                            count = os.write(key.fd, pending[:65536])
                            pending = pending[count:]
                        except BrokenPipeError:
                            pending = pending[len(pending):]
                        if not pending:
                            selector.unregister(key.fileobj)
                            key.fileobj.close()
                    else:
                        chunk = os.read(key.fd, 65536)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            key.fileobj.close()
                            continue
                        size += len(chunk)
                        require(size <= 1_048_576)
                        if key.data == "out":
                            output.extend(chunk)
            while process.poll() is None:
                require(clock() < deadline)
                time.sleep(0.01)
            require(process.returncode in statuses)
            return (process.returncode, bytes(output)) if return_status else bytes(output)
        finally:
            selector.close()
    finally:
        # Even a normally exited parent may leave children holding resources.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        for stream in (process.stdin, process.stdout, process.stderr):
            if not stream.closed:
                stream.close()


PROBE = r'''
set +x
set -eu
source "$1"
shift
transport=$1 context=$2 deadline=$3 operation=$4
shift 4
native() { "$transport" "$context" "$deadline" cli "$@"; }
once() { shift 2; "$@"; }
health() { native exec "$2" wget --quiet --output-document=- "$3"; }
admin() {
    native exec --user 1000:1000 "$2" forgejo admin user create \
        --username "$3" --password "$4" --email boxferry@example.invalid \
        --admin --must-change-password=false
}
git_probe() {
    native run --rm --pull=never --name "$2-git-probe" \
        --label "io.boxferry.live-run=$profile_run" \
        --label "io.boxferry.application=$2-forgejo" \
        --network host --volume "$profile_fixture:/fixture:rw" \
        --env "BF_FORGEJO_USER=$4" --env "BF_FORGEJO_PASSWORD=$5" \
        --env "BF_HTTP_PORT=$6" --env "BF_SSH_PORT=$7" \
        --entrypoint /bin/sh "$profile_git_image" /fixture/git-probe.sh "$3"
}
query() { native exec --env "PGPASSWORD=$3" "$2" psql -U forgejo -d forgejo -Atc "$4"; }
case "$operation" in
    health) forgejo_probe_wait_application once health "$@" ;;
    admin) forgejo_probe_create_admin admin "$@" ;;
    git) forgejo_probe_git git_probe "$@" ;;
    sql) forgejo_probe_assert_database query "$@" ;;
    *) exit 2 ;;
esac
'''


class Profile:
    def __init__(self, arguments):
        (transport, self.context, repository, plan, admission, sidecar, profile,
         aliases, interpolation, self.lane, self.run, self.prefix, self.fixture,
         execution, cleanup, self.admin_password) = arguments
        self.transport = pathlib.Path(transport)
        require(self.transport.is_absolute() and self.transport.is_file()
                and not self.transport.is_symlink() and os.access(self.transport, os.X_OK))
        self.root = pathlib.Path(repository).resolve(strict=True)
        require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", self.context))
        require(re.fullmatch(r"/[A-Za-z0-9_./-]+", self.fixture))
        require(re.fullmatch(r"[1-9][0-9]{0,12}", execution)
                and re.fullmatch(r"[1-9][0-9]{0,12}", cleanup))
        self.execution, self.cleanup_deadline = int(execution), int(cleanup)
        require(clock() < self.execution <= clock() + 3600
                and self.execution < self.cleanup_deadline <= self.execution + 600)
        require(0 < len(self.admin_password.encode()) <= 4096 and "\x00" not in self.admin_password)
        spec = importlib.util.spec_from_file_location(
            "forgejo_authored", self.root / "scripts/lib/docker-forgejo-authored-fields.py")
        self.contract = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.contract)
        read = self.contract.schedule.read_document
        self.catalogue = read(self.root / self.contract.topology.CATALOGUE_PATH)
        self.contract.topology.check_sources(self.root, self.catalogue)
        raw, admitted, dependencies = read(pathlib.Path(plan)), read(pathlib.Path(admission)), read(pathlib.Path(sidecar))
        self.profile = self.contract.topology.document(read(pathlib.Path(profile)))
        self.aliases = self.contract.topology.document(read(pathlib.Path(aliases)))
        protected = read(pathlib.Path(interpolation))
        self.schedule = self.contract.validate_authored_fields(
            raw, admitted, dependencies, protected, catalogue_bytes=self.catalogue,
            application="forgejo", lane=self.lane, profile=self.profile,
            image_aliases=self.aliases, prefix=self.prefix, run_id=self.run, fixture_root=self.fixture)
        self.plan = self.contract.topology.document(raw)
        self.password = self.contract.topology.document(protected)["interpolation"]["BF_DB_PASSWORD"]
        # Offline prerequisites deliberately leave extra Env/image defaults
        # unassessed. Execution closes those fields against authored intent.
        db_names = {"POSTGRES_DB", "POSTGRES_USER", "POSTGRES_PASSWORD"}
        app_names = {"FORGEJO__database__" + key for key in
                     ("DB_TYPE", "HOST", "NAME", "USER", "PASSWD", "SSL_MODE")}
        app_names |= {"FORGEJO__security__INSTALL_LOCK", "FORGEJO__security__SECRET_KEY",
                      "FORGEJO__service__DISABLE_REGISTRATION"}
        app_names |= {"FORGEJO__server__" + key for key in
                      ("DOMAIN", "ROOT_URL", "SSH_DOMAIN", "SSH_PORT", "SSH_LISTEN_PORT", "START_SSH_SERVER")}
        for request in self.plan["requests"]:
            if "/containers/create?name=" not in request["path"]:
                continue
            body = request["body"]
            database = request["path"].endswith(self.prefix + "-forge-db")
            require({row.split("=", 1)[0] for row in body["Env"]} == (db_names if database else app_names))
            require(set(body) == {"Image", "Labels", "HostConfig", "NetworkingConfig", "Env"}
                    | ({"Healthcheck"} if database else {"User", "ExposedPorts"}))
            if database:
                require(set(body["Healthcheck"]) == {"Test", "Interval", "Timeout", "Retries"})
        self.api_prefix = "/v" + self.profile["rendering_api_version"]
        self.owner = self.prefix + "-forgejo"
        self.labels = {"io.boxferry.live-run": self.run, "io.boxferry.application": self.owner}
        self.peer_labels = {**self.labels, "io.boxferry.application": self.prefix + "-peer"}
        self.resources = []
        self.images = []
        self.image_ids = {}
        self.fixture_owned = False
        self.attested = False
        # Snapshot only catalogue-bound fixture files before any native access.
        self.files = {name: read(self.root / "fixtures/conformance/forgejo-application" / name)
                      for name in ("git-probe.sh", "repository-proof.txt")}
        self.references = {}
        self.postgres_version = None
        image_rows = read(self.root / "fixtures/conformance/forgejo-application/images.tsv").decode().splitlines()
        for row in image_rows:
            if not row or row.startswith("#"):
                continue
            key, reference, version, *_ = row.split()
            require(key not in self.references and re.search(r"@sha256:[0-9a-f]{64}$", reference))
            self.references[key] = reference
            if key == "postgres":
                self.postgres_version = version.split("-", 1)[0]
                require(re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", self.postgres_version))
        require(set(self.references) == {"forgejo", "postgres", "git-client"})
        self.git_image = "registry.invalid/" + self.prefix.lower() + "/git-client:owned"
        self.aliases = {**self.aliases, "git-client": self.git_image}
        require(len(set(self.aliases.values())) == 3)

    def boundary(self, operation, *arguments, payload=b""):
        deadline = self.cleanup_deadline if cleaning else self.execution
        return execute([str(self.transport), self.context, str(deadline), operation, *arguments], payload, deadline)

    def api(self, method, path, body=None, expected=(200, 201, 204)):
        raw = self.boundary("api", method, self.api_prefix + path,
                            payload=b"" if body is None else encoded(body))
        result = self.contract.topology.document(raw)
        require(isinstance(result, dict) and set(result) == {"status", "body"}
                and type(result["status"]) is int and result["status"] in expected)
        return result["status"], result["body"]

    def path(self, kind, identity):
        return {"container": "/containers/", "network": "/networks/", "volume": "/volumes/"}[kind] + identity + ("/json" if kind == "container" else "")

    def observe(self, resource, missing=False):
        kind, name, labels = resource["kind"], resource["name"], resource["labels"]
        status, body = self.api("GET", self.path(kind, resource.get("id", name)),
                                expected=(200, 404) if missing else (200,))
        if status == 404:
            return None
        require(isinstance(body, dict))
        actual_labels = body.get("Config", {}).get("Labels") if kind == "container" else body.get("Labels")
        require(actual_labels == labels)
        identity = body.get("Name") if kind == "volume" else body.get("Id")
        require(identity == name if kind == "volume" else
                isinstance(identity, str) and re.fullmatch(r"[0-9a-f]{64}", identity))
        if "id" in resource:
            require(resource["id"] == identity)
        require(body.get("Name") == ("/" + name if kind == "container" else name))
        resource["id"] = identity
        return body

    def absent(self, kind, name):
        self.api("GET", self.path(kind, name), expected=(404,))

    def liability(self, kind, name, labels=None):
        row = {"kind": kind, "name": name, "labels": labels or self.labels}
        self.resources.append(row)
        return row

    def create(self, request):
        path, body = request["path"][len(self.api_prefix):], request["body"]
        resource = None
        if path == "/networks/create":
            resource = {"kind": "network", "name": body["Name"], "labels": body["Labels"]}
        elif path == "/volumes/create":
            resource = {"kind": "volume", "name": body["Name"], "labels": body["Labels"]}
        elif path.startswith("/containers/create?name="):
            resource = {"kind": "container", "name": path.split("=", 1)[1], "labels": body["Labels"]}
        if resource:
            self.absent(resource["kind"], resource["name"])
            if resource["kind"] == "container":
                self.image_bound(body["Image"])
                # Both named volumes are prerequisites of the complete graph,
                # including before either recreation writes any retained data.
                for suffix in ("forge-db", "forge-data"):
                    self.bound(self.named("volume", self.prefix + "-" + suffix))
                for name in body["NetworkingConfig"]["EndpointsConfig"]:
                    self.bound(self.named("network", name))
            self.resources.append(resource)
        else:
            require(path.endswith("/connect"))
            self.bound(self.named("network", path.split("/")[2]))
            self.bound(self.named("container", body["Container"]))
        _status, response = self.api(request["method"], path, body)
        if resource:
            self.observe(resource)
            require(isinstance(response, dict) and response.get(
                "Name" if resource["kind"] == "volume" else "Id") == resource["id"])
        return resource

    def named(self, kind, name):
        return next(row for row in reversed(self.resources) if row["kind"] == kind and row["name"] == name and not row.get("removed"))

    def bound(self, resource):
        # Guard both the immutable object and any emitted name reference before
        # use. Never dispatch a secret to a replacement found by name.
        try:
            self.observe(resource)
            current = {key: resource[key] for key in ("kind", "name", "labels")}
            self.observe(current)
            require(current["id"] == resource["id"])
            return resource["id"]
        except Cancelled:
            raise
        except Exception as error:
            raise OwnershipRefused() from error

    def image_bound(self, alias):
        _status, body = self.api("GET", "/images/" + alias + "/json")
        require(body["Id"] == self.image_ids[alias])

    def remove(self, resource, force):
        if self.observe(resource, missing=True) is None:
            resource["removed"] = True
            return
        kind, identity = resource["kind"], resource["id"]
        path = self.path(kind, identity).removesuffix("/json")
        if kind == "container":
            path += "?force=" + ("true" if force else "false") + "&v=false"
        if kind == "volume":
            path += "?force=false"
        self.api("DELETE", path)
        self.api("GET", self.path(kind, identity), expected=(404,))
        resource["removed"] = True

    def wait(self, condition):
        while not condition():
            require(clock() < self.execution)
            time.sleep(min(0.2, max(0, self.execution - clock())))

    def start(self, name, database=False):
        resource = self.named("container", name)
        self.bound(resource)
        self.api("POST", "/containers/" + resource["id"] + "/start")
        if database:
            def healthy():
                body = self.observe(resource)
                require(body["State"]["Running"] is True)
                status = body["State"]["Health"]["Status"]
                require(status in {"starting", "healthy"})
                return status == "healthy"
            self.wait(healthy)

    def probe(self, operation, *arguments):
        # Bash invokes the existing semantic callbacks; argv and placement are
        # Docker-profile owned. No host credentials or implicit runtime chosen.
        if operation in {"health", "admin", "sql"}:
            name = self.prefix + ("-forge-db" if operation == "sql" else "-forge-app")
            require(arguments[0] == name)
            arguments = (self.bound(self.named("container", name)), *arguments[1:])
        else:
            require(operation == "git")
            # Git carries credentials to declared loopback publications. Refuse
            # a stopped app or changed publication before starting that client.
            self.topology()
            self.bound(self.named("container", self.prefix + "-forge-app"))
            self.bound(self.named("container", self.prefix + "-forge-db"))
            self.image_bound(self.git_image)
        exports = {"profile_run": self.run, "profile_fixture": self.fixture,
                   "profile_git_image": self.git_image}
        script = "\n".join(key + "=$" + str(index + 1) for index, key in enumerate(exports))
        script += "\nshift 3\n" + PROBE
        argv = ["bash", "--noprofile", "--norc", "-c", script, "forgejo-probe", *exports.values(),
                str(self.root / "scripts/lib/forgejo-application-probes.sh"), str(self.transport),
                self.context, str(self.execution), operation, self.context, *arguments]
        return execute(argv, b"", self.execution)

    def health(self):
        try:
            self.probe("health", self.prefix + "-forge-app", "http://127.0.0.1:3000/api/healthz")
            return True
        except (Cancelled, OwnershipRefused):
            raise
        except Refused:
            return False

    def git(self, mode):
        self.absent("container", self.prefix + "-git-probe")
        row = self.liability("container", self.prefix + "-git-probe")
        self.probe("git", self.prefix, mode, "boxferry-live", self.admin_password, "13000", "12222")
        self.absent("container", row["name"])
        row["removed"] = True

    def database(self):
        self.probe("sql", self.prefix + "-forge-db", self.password)

    def isolation_probe(self, network, arguments):
        name = self.prefix + "-isolation-probe"
        self.absent("container", name)
        resource = self.liability("container", name)
        self.image_bound(self.aliases["postgres"])
        # Bypass the PostgreSQL entrypoint, close image-default VOLUME behavior,
        # and use only the catalogue-bound PostgreSQL client's known utility.
        # The caller's transport must propagate the original container exit code.
        argv = [str(self.transport), self.context, str(self.execution), "cli",
                "run", "--rm", "--pull=never", "--name", name,
                "--label", "io.boxferry.live-run=" + self.run,
                "--label", "io.boxferry.application=" + self.owner,
                "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                "--network", network,
                "--mount", "type=tmpfs,destination=/var/lib/postgresql/data,tmpfs-size=65536,tmpfs-mode=0700",
                "--env", "LC_ALL=C", "--entrypoint", "pg_isready", self.aliases["postgres"], *arguments]
        result = execute(argv, b"", self.execution, statuses=(0, 1, 2, 3, 125, 126, 127), return_status=True)
        self.absent("container", name)
        resource["removed"] = True
        return result

    def isolation(self):
        backend = self.named("network", self.prefix + "-forge-backend")
        peer = self.named("container", self.prefix + "-peer-app")
        edge = self.named("network", self.prefix + "-shared-edge")
        database = self.named("container", self.prefix + "-forge-db")
        self.observe(backend)
        self.observe(edge)
        peer_endpoints = self.observe(peer)["NetworkSettings"]["Networks"]
        require(set(peer_endpoints) == {edge["name"]}
                and peer_endpoints[edge["name"]]["NetworkID"] == edge["id"])
        body = self.observe(database)
        require(set(body["NetworkSettings"]["Networks"]) == {backend["name"]})
        endpoint = body["NetworkSettings"]["Networks"][backend["name"]]
        address = endpoint["IPAddress"]
        parsed = ipaddress.ip_address(address)
        require(parsed.version == 4 and str(parsed) == address and parsed.is_private
                and not (parsed.is_unspecified or parsed.is_loopback or parsed.is_link_local
                         or parsed.is_multicast) and endpoint["NetworkID"] == backend["id"])
        status, output = self.isolation_probe(backend["id"], ["--version"])
        require(status == 0 and output == ("pg_isready (PostgreSQL) " + self.postgres_version + "\n").encode())
        # Numeric address avoids DNS ambiguity. Backend successes bracket the
        # peer's no-response result; a stopped/rejecting DB or unavailable client
        # cannot become isolation success. The peer test shares its exact netns.
        for network, expected in ((backend["id"], 0), ("container:" + peer["id"], 2), (backend["id"], 0)):
            self.observe(backend)
            self.observe(edge)
            peer_body = self.observe(peer)
            db_body = self.observe(database)
            require(peer_body["State"]["Running"] is True
                    and peer_body["NetworkSettings"]["Networks"] == peer_endpoints
                    and db_body["State"]["Running"] is True
                    and db_body["State"]["Health"]["Status"] == "healthy"
                    and db_body["NetworkSettings"]["Networks"][backend["name"]] == endpoint)
            status, output = self.isolation_probe(network, ["--host", address, "--port", "5432",
                "--username", "forgejo", "--dbname", "forgejo", "--timeout", "2"])
            message = "accepting connections" if expected == 0 else "no response"
            require(status == expected and output == (address + ":5432 - " + message + "\n").encode())
        self.topology()

    def topology(self):
        backend, edge = self.prefix + "-forge-backend", self.prefix + "-shared-edge"
        for name, internal in ((backend, True), (edge, False)):
            body = self.observe(self.named("network", name))
            require(body["Driver"] == "bridge" and body["Internal"] is internal)
            members = {self.named("container", self.prefix + "-forge-app")["id"]}
            members.add(self.named("container", self.prefix + ("-forge-db" if internal else "-peer-app"))["id"])
            require(set(body["Containers"]) == members)
        for suffix, networks, mount, target in (
            ("forge-db", {backend}, self.prefix + "-forge-db", "/var/lib/postgresql/data"),
            ("forge-app", {backend, edge}, self.prefix + "-forge-data", "/var/lib/gitea"),
            ("peer-app", {edge}, None, None),
        ):
            body = self.observe(self.named("container", self.prefix + "-" + suffix))
            require(body["State"]["Running"] is True
                    and set(body["NetworkSettings"]["Networks"]) == networks)
            alias = {"forge-db": "db", "forge-app": "forgejo", "peer-app": "peer"}[suffix]
            for name, endpoint in body["NetworkSettings"]["Networks"].items():
                require(endpoint["NetworkID"] == self.named("network", name)["id"]
                        and alias in endpoint["Aliases"])
            ports = body["NetworkSettings"]["Ports"]
            expected = {"3000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "13000"}],
                        "2222/tcp": [{"HostIp": "127.0.0.1", "HostPort": "12222"}]}
            if suffix == "forge-app":
                require(ports == expected)
            else:
                require(all(value is None or value == [] for value in ports.values()))
            if mount:
                mounts = body["Mounts"]
                require(len(mounts) == 1 and mounts[0]["Type"] == "volume"
                        and mounts[0]["Name"] == mount and mounts[0]["Destination"] == target
                        and mounts[0]["RW"] is True)
        for suffix in ("forge-db", "forge-data"):
            self.observe(self.named("volume", self.prefix + "-" + suffix))

    def run_profile(self):
        expected = {"protocol": "boxferry-docker-forgejo-transport-v1", "context": self.context,
                    "run_id": self.run, "prefix": self.prefix, "owner": self.owner,
                    "lane": self.lane, "profile": self.profile, "fixture_root": self.fixture,
                    "docker_plan_sha256": self.schedule["docker_plan_sha256"],
                    "dependency_sidecar_sha256": self.schedule["dependency_sidecar_sha256"],
                    "expectations_sha256": self.schedule["expectations_sha256"],
                    "execution_deadline": self.execution, "cleanup_deadline": self.cleanup_deadline,
                    "isolated_inner_runtime": True, "caller_owns_cleanup_guard": True,
                    "no_host_socket": True, "no_credential_discovery": True}
        require(self.contract.topology.document(self.boundary("attest", payload=encoded(expected))) == expected)
        self.attested = True
        _status, version = self.api("GET", "/version")
        _status, info = self.api("GET", "/info")
        require(version["Version"] == self.profile["engine_release"]
                and version["ApiVersion"] == self.profile["advertised_api_version"])
        options = info["SecurityOptions"]
        require(isinstance(options, list) and all(isinstance(value, str) for value in options))
        rootless = any(value == "name=rootless" or value.startswith("name=rootless,") for value in options)
        require(rootless == (self.profile["daemon_mode"] == "rootless"))
        for kind, suffix in (("network", "shared-edge"), ("network", "forge-backend"),
                             ("volume", "forge-db"), ("volume", "forge-data"),
                             ("container", "forge-db"), ("container", "forge-app"),
                             ("container", "peer-app"), ("container", "git-probe"),
                             ("container", "isolation-probe")):
            self.absent(kind, self.prefix + "-" + suffix)
        for key in ("postgres", "forgejo", "git-client"):
            # Transport must refuse an existing alias, verify the exact catalogue
            # digest after loading, and retain a recoverable liability before load.
            self.images.append(self.aliases[key])
            self.boundary("image-provision", self.references[key], self.aliases[key])
            _status, image = self.api("GET", "/images/" + self.aliases[key] + "/json")
            require(isinstance(image, dict) and isinstance(image.get("Id"), str)
                    and re.fullmatch(r"sha256:[0-9a-f]{64}", image["Id"]))
            self.image_ids[self.aliases[key]] = image["Id"]
            volumes = image["Config"].get("Volumes")
            require(volumes is None or isinstance(volumes, dict))
            volumes = volumes or {}
            declared = {"postgres": {"/var/lib/postgresql/data"}, "forgejo": {"/var/lib/gitea"}, "git-client": set()}[key]
            # Refuse image-default anonymous volumes before any container create;
            # container cleanup deliberately never deletes undeclared volumes.
            require(isinstance(volumes, dict) and set(volumes) <= declared)
        self.fixture_owned = True
        self.boundary("fixture-create", self.fixture)
        for name, data in self.files.items():
            self.boundary("fixture-install", self.fixture, name, hashlib.sha256(data).hexdigest(), payload=data)
        self.create({"method": "POST", "path": self.api_prefix + "/networks/create",
                     "body": {"Name": self.prefix + "-shared-edge", "Driver": "bridge", "Internal": False,
                              "CheckDuplicate": True, "Labels": self.peer_labels}})
        # Plan bytes were validated independently; preserve their original order
        # and exact request method/path/body instead of synthesizing a fixture.
        for request in self.plan["requests"]:
            self.create(request)
        peer = {"Image": self.git_image, "Labels": self.peer_labels,
                "Entrypoint": ["/bin/sh", "-ceu"], "Cmd": ["sleep 3600"],
                "HostConfig": {"NetworkMode": self.prefix + "-shared-edge"},
                "NetworkingConfig": {"EndpointsConfig": {self.prefix + "-shared-edge": {"Aliases": ["peer"]}}}}
        self.create({"method": "POST", "path": self.api_prefix + "/containers/create?name=" + self.prefix + "-peer-app", "body": peer})
        self.start(self.prefix + "-forge-db", database=True)
        self.start(self.prefix + "-forge-app")
        self.start(self.prefix + "-peer-app")
        self.wait(self.health)
        self.probe("admin", self.prefix + "-forge-app", "boxferry-live", self.admin_password)
        self.git("seed")
        self.git("verify")
        self.database()
        self.topology()
        self.isolation()
        # Duplicate emitted container creation must fail without changing identity.
        app_request = next(row for row in self.plan["requests"] if row["path"].endswith("containers/create?name=" + self.prefix + "-forge-app"))
        app = self.named("container", self.prefix + "-forge-app")
        old_id = app["id"]
        self.bound(app)
        self.api(app_request["method"], app_request["path"][len(self.api_prefix):], app_request["body"], expected=(409,))
        self.observe(app)
        require(app["id"] == old_id)
        for suffix in ("forge-app", "forge-db"):
            row = self.named("container", self.prefix + "-" + suffix)
            self.bound(row)
            self.api("POST", "/containers/" + row["id"] + "/stop?t=30", expected=(204,))
            self.remove(row, False)
        # Volumes and external edge are retained; recreate only the exact emitted
        # container and attachment operations, with the same dependency readiness.
        for suffix in ("forge-db", "forge-app"):
            name = self.prefix + "-" + suffix
            request = next(row for row in self.plan["requests"] if row["path"].endswith("containers/create?name=" + name))
            self.create(request)
            for attachment in self.plan["requests"]:
                if attachment["path"].endswith("/connect") and attachment["body"]["Container"] == name:
                    self.create(attachment)
            self.start(name, database=suffix == "forge-db")
        self.wait(self.health)
        self.git("verify")
        self.database()
        self.topology()
        self.isolation()

    def cleanup(self):
        okay = True
        if not self.attested:
            return True
        for row in reversed(self.resources):
            if not row.get("removed"):
                try:
                    self.remove(row, True)
                except Exception:
                    okay = False
        if self.fixture_owned:
            try:
                self.boundary("fixture-remove", self.fixture)
            except Exception:
                okay = False
        for alias in reversed(self.images):
            try:
                self.boundary("image-release", alias)
            except Exception:
                okay = False
        return okay


instance = None
status = 1
try:
    instance = Profile(sys.argv[1:])
    instance.run_profile()
    status = 0
except Cancelled:
    status = 130
except Exception:
    status = 1
finally:
    cleaning = True
    if instance is not None and not instance.cleanup():
        status = 1
    if cancelled and status == 0:
        status = 130
if status:
    print("Forgejo profile incomplete: inspect the caller-owned private recovery ledger.", file=sys.stderr)
sys.exit(status)
PY
)
