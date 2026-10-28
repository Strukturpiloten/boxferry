#!/usr/bin/env bash
# Hermetic source-profile controls. No Docker/Podman/client/container is launched.
set -Eeuo pipefail
script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
python3 - "${script_directory}/.." << 'PY'
import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(sys.argv[1]).resolve()
MODULE = ROOT / "scripts/lib/docker-forgejo-application.sh"
spec = importlib.util.spec_from_file_location("independent_forgejo_fixture", ROOT / "scripts/test-docker-forgejo-authored-fields.py")
fixture_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture_module)

# Reuse the existing independently authored test inputs, never an exporter,
# schedule projection or the new profile's expected-value implementation.
fixture = fixture_module.ForgejoAuthoredFields()
fixture.setUp()

FAKE = r'''#!/usr/bin/env python3
import hashlib, json, os, pathlib, re, signal, sys, time
root = pathlib.Path(__file__).parent
state_path = root / "state.json"
s = json.loads(state_path.read_text())
context, deadline, operation, *args = sys.argv[1:]
assert context == "owned-context"
assert int(deadline) == s["cleanup" if s["cleanup_started"] else "execution"] or int(deadline) == s["cleanup"]
assert time.clock_gettime(time.CLOCK_BOOTTIME) < int(deadline)
if int(deadline) == s["cleanup"]:
    s["cleanup_started"] = True
body = sys.stdin.buffer.read()
def save():
    state_path.write_text(json.dumps(s))
def emit(value):
    save()
    print(json.dumps(value))
def reply(status=200, value=None):
    emit({"status":status,"body":value})
def labels(name):
    return {"io.boxferry.live-run":"offline-run", "io.boxferry.application":
            "unit-peer" if name in {"unit-shared-edge","unit-peer-app"} else "unit-forgejo"}
def observed(row):
    name, kind = row["name"], row["kind"]
    if kind == "volume":
        value = {"Name":name,"Labels":labels(name)}
        after_stop = "stop:unit-forge-db" in s["events"] and "container:unit-forge-db" not in s["resources"]
        drift = s["fault"] == "db-volume-initial" or s["fault"] in {"db-volume-recreate","data-volume-recreate"} and after_stop
        if drift and name == ("unit-forge-data" if s["fault"] == "data-volume-recreate" else "unit-forge-db"):
            value["Labels"]["io.boxferry.live-run"] = "foreign-run"
        if s["fault"] == "db-volume-name" and name == "unit-forge-db":
            value["Name"] = "foreign-volume"
        return value
    if kind == "network":
        names = {"unit-forge-app", "unit-forge-db" if name == "unit-forge-backend" else "unit-peer-app"}
        members = {r["id"]:{} for r in s["resources"].values() if r["kind"] == "container" and r["name"] in names}
        return {"Name":name,"Id":row["id"],"Labels":labels(name),"Driver":"bridge",
                "Internal":name == "unit-forge-backend", "Containers":members}
    if name == "unit-isolation-probe":
        return {"Name":"/"+name,"Id":row["id"],"Config":{"Labels":labels(name)},"State":{"Running":True}}
    networks = {"unit-forge-db":["unit-forge-backend"],
                "unit-forge-app":["unit-forge-backend", "unit-shared-edge"],
                "unit-peer-app":["unit-shared-edge"]}[name]
    if s["fault"] == "peer-isolation" and name == "unit-peer-app" and row["running"]:
        networks.append("unit-forge-backend")
    ports = {"3000/tcp":[{"HostIp":"127.0.0.1","HostPort":"13000"}],
             "2222/tcp":[{"HostIp":"127.0.0.1","HostPort":"12222"}]} if name == "unit-forge-app" else {}
    if s["fault"] == "wildcard-port" and name == "unit-forge-app":
        ports["3000/tcp"][0]["HostIp"] = "0.0.0.0"
    mounts = [] if name == "unit-peer-app" else [{"Type":"volume", "RW":True,
        "Name":"unit-forge-db" if name == "unit-forge-db" else "unit-forge-data",
        "Destination":"/var/lib/postgresql/data" if name == "unit-forge-db" else "/var/lib/gitea"}]
    endpoints = {n:{"NetworkID":s["resources"]["network:"+n]["id"],
                    "IPAddress":"172.29.0.11" if name == "unit-forge-db" else "172.30.0.12",
                    "Aliases":[{"unit-forge-db":"db","unit-forge-app":"forgejo","unit-peer-app":"peer"}[name]]}
                 for n in networks}
    value = {"Name":"/"+name,"Id":row["id"], "Config":{"Labels":labels(name)},
        "State":{"Running":row["running"],"Health":{"Status":"healthy"}},
        "Mounts":mounts,"NetworkSettings":{"Networks":endpoints,"Ports":ports}}
    if name == "unit-forge-app" and "admin" in s["events"]:
        if s["fault"] == "git-app-stopped": value["State"]["Running"] = False
        if s["fault"] == "git-publication-drift": value["NetworkSettings"]["Ports"]["3000/tcp"][0]["HostIp"] = "0.0.0.0"
    if s["fault"] == "foreign-owner" and name == "unit-forge-app" and row["running"]:
        value["Config"]["Labels"]["io.boxferry.live-run"] = "other-run"
    return value
def boundary_class():
    if operation != "api" and operation != "cli":
        return operation
    if operation == "cli":
        if args[0] == "exec":
            return "cli-health" if "wget" in args else "cli-admin" if args[1] == "--user" else "cli-sql"
        if "unit-isolation-probe" in args:
            return "cli-isolation-version" if args[-1] == "--version" else "cli-isolation-peer" if any(a.startswith("container:") for a in args) else "cli-isolation-backend"
        return "cli-git-" + args[-1]
    method, path = args
    parts = path.removeprefix("/v1.56/").split("/")
    if method == "GET":
        if parts[0] not in {"containers","networks","volumes"}:
            return "api-read"
        kind = {"containers":"container","networks":"network","volumes":"volume"}[parts[0]]
        row = next((r for r in s["resources"].values() if r["kind"] == kind and parts[1] in {r["name"],r["id"]}),None)
        return "api-read-absence" if row is None else "api-read-binding" if re.fullmatch("[0-9a-f]{64}",parts[1]) else "api-read-owned"
    kind = {"containers":"container","networks":"network","volumes":"volume"}[parts[0]]
    if method == "DELETE":
        return "delete-"+kind+("-cleanup" if s["cleanup_started"] else "-recreate")
    assert method == "POST"
    if parts[-1] == "connect": return "attach-network"
    if parts[-1] == "start": return "start-container"
    if parts[-1].startswith("stop?"): return "stop-container"
    return "create-"+kind
s["calls"] += 1
classification = boundary_class()
s["class_calls"].setdefault(classification,s["calls"])
if s["fail_call"] == s["calls"]:
    s["failed_class"] = classification
    s["recovered_health"] = operation == "cli" and "wget" in args
    save()
    print("private-db-canary=密碼 private-key-canary=κλειδί admin-canary", file=sys.stderr)
    sys.exit(37)
if operation == "attest":
    expected = json.loads(body)
    assert expected["owner"] == "unit-forgejo" and expected["lane"] == s["lane"]
    assert expected["isolated_inner_runtime"] and expected["no_host_socket"]
    assert {name:expected[name] for name in s["bindings"]} == s["bindings"]
    if s["fault"] == "attest": expected["prefix"] = "foreign"
    emit(expected)
elif operation == "image-provision":
    reference, alias = args
    assert reference in s["references"].values()
    assert alias == s["aliases"][next(k for k,v in s["references"].items() if v == reference)]
    assert alias not in s["images"]
    s["images"].append(alias)
    s["events"].append("image:" + next(k for k,v in s["references"].items() if v == reference))
    save()
    if s["fault"] == "overflow":
        sys.stdout.write("private-db-canary=密碼" * 100000)
        sys.exit(0)
    if s["fault"] == "blocked": time.sleep(20)
    if s["fault"].startswith("cancel-"):
        os.kill(os.getppid(), getattr(signal,s["fault"].removeprefix("cancel-")))
        time.sleep(5)
elif operation == "image-release":
    assert args[0] in s["aliases"].values()
    if args[0] in s["images"]: s["images"].remove(args[0])
    s["events"].append("release-image")
    save()
elif operation == "fixture-create":
    assert args == ["/tmp/offline-fixture"] and not s["fixture"]
    s["fixture"] = True
    s["events"].append("fixture")
    save()
elif operation == "fixture-install":
    assert args[0] == "/tmp/offline-fixture" and args[1] in {"git-probe.sh","repository-proof.txt"}
    assert hashlib.sha256(body).hexdigest() == args[2] == s["fixture_digests"][args[1]]
    save()
elif operation == "fixture-remove":
    assert args == ["/tmp/offline-fixture"]
    s["fixture"] = False
    save()
elif operation == "cli":
    assert args
    if args[0] == "run" and "unit-isolation-probe" in args:
        network = args[args.index("--network")+1]
        backend = s["resources"]["network:unit-forge-backend"]["id"]
        peer = s["resources"]["container:unit-peer-app"]["id"]
        assert network in {backend,"container:"+peer}
        base = ["run","--rm","--pull=never","--name","unit-isolation-probe",
          "--label","io.boxferry.live-run=offline-run","--label","io.boxferry.application=unit-forgejo",
          "--read-only","--cap-drop","ALL","--security-opt","no-new-privileges","--network",network,
          "--mount","type=tmpfs,destination=/var/lib/postgresql/data,tmpfs-size=65536,tmpfs-mode=0700",
          "--env","LC_ALL=C","--entrypoint","pg_isready","registry.invalid/offline/postgres:reviewed"]
        command = args[len(base):]
        assert args[:len(base)] == base
        capability = command == ["--version"]
        if not capability:
            assert command == ["--host","172.29.0.11","--port","5432","--username","forgejo","--dbname","forgejo","--timeout","2"]
        is_peer = network.startswith("container:")
        event = "isolation:version" if capability else "isolation:peer" if is_peer else "isolation:backend"
        s["events"].append(event)
        if capability: s["isolation_positive"] = 0
        elif not is_peer: s["isolation_positive"] += 1
        identity = hashlib.sha256(("probe"+str(s["calls"])).encode()).hexdigest()
        s["resources"]["container:unit-isolation-probe"] = {"kind":"container","name":"unit-isolation-probe","id":identity,"running":True}
        save()
        if is_peer and s["fault"] == "isolation-cancel":
            os.kill(os.getppid(),signal.SIGTERM)
            time.sleep(20)
        if is_peer and s["fault"] == "isolation-timeout": time.sleep(20)
        del s["resources"]["container:unit-isolation-probe"]
        save()
        if s["fault"] == "probe-unavailable": sys.exit(127)
        if capability:
            print("pg_isready (PostgreSQL) 0.0" if s["fault"] == "probe-version" else "pg_isready (PostgreSQL) 17.6")
            sys.exit(0)
        if s["fault"] == "probe-invalid":
            print("invalid parameters"); sys.exit(3)
        if s["fault"] == "db-rejecting" and not is_peer:
            print("172.29.0.11:5432 - rejecting connections"); sys.exit(1)
        denied = is_peer and s["fault"] != "reachable-peer"
        if not is_peer and (s["fault"] == "db-down" or (s["fault"] == "db-down-after" and s["isolation_positive"] == 2)):
            denied = True
        print("172.29.0.11:5432 - " + ("no response" if denied else "accepting connections"))
        sys.exit(2 if denied else 0)
    elif args[0] == "exec":
        if "wget" in args:
            assert args == ["exec",s["resources"]["container:unit-forge-app"]["id"],"wget","--quiet","--output-document=-","http://127.0.0.1:3000/api/healthz"]
            event = "health"
        elif args[:2] == ["exec", "--user"]:
            assert args == ["exec","--user","1000:1000",s["resources"]["container:unit-forge-app"]["id"],"forgejo","admin","user","create",
              "--username","boxferry-live","--password","admin-canary","--email","boxferry@example.invalid","--admin","--must-change-password=false"]
            event = "admin"
        else:
            assert args == ["exec","--env","PGPASSWORD=private-db-canary=密碼",s["resources"]["container:unit-forge-db"]["id"],"psql","-U","forgejo","-d","forgejo","-Atc",
              "SELECT count(*) FROM repository WHERE lower_name = 'migration-baseline';"]
            event = "sql"
    else:
        mode = args[-1]
        assert args == ["run","--rm","--pull=never","--name","unit-git-probe",
          "--label","io.boxferry.live-run=offline-run","--label","io.boxferry.application=unit-forgejo",
          "--network","host","--volume","/tmp/offline-fixture:/fixture:rw",
          "--env","BF_FORGEJO_USER=boxferry-live","--env","BF_FORGEJO_PASSWORD=admin-canary",
          "--env","BF_HTTP_PORT=13000","--env","BF_SSH_PORT=12222","--entrypoint","/bin/sh",
          "registry.invalid/unit/git-client:owned","/fixture/git-probe.sh",mode]
        assert mode in {"seed","verify"}
        event = "git:"+mode
    s["events"].append(event)
    if event == "sql" and s["first_sql_call"] is None:
        s["first_sql_call"] = s["calls"]
    save()
    if s["fault"] == "git-failure" and event == "git:seed": sys.exit(51)
    if event == "sql": print("2" if s["fault"] == "db-row" else "1")
elif operation == "api":
    method, path = args
    assert path.startswith("/v1.56/")
    path = path.removeprefix("/v1.56")
    data = json.loads(body) if body else None
    if method == "GET" and path == "/version":
        reply(value={"Version":"0.0.0" if s["fault"] == "native-version" else "29.8.1","ApiVersion":"1.56"})
    elif method == "GET" and path == "/info":
        rootless = s["lane"] == "upstream-rootless"
        if s["fault"] == "mode": rootless = not rootless
        reply(value={"SecurityOptions":["name=rootless"] if rootless else []})
    elif method == "GET" and path.startswith("/images/"):
        alias = path.removeprefix("/images/").removesuffix("/json")
        assert alias in s["aliases"].values()
        mounts = {"/var/lib/postgresql/data":{}} if alias == s["aliases"]["postgres"] else {"/var/lib/gitea":{}} if alias == s["aliases"]["forgejo"] else {}
        reply(value={"Id":"sha256:"+"a"*64,"Config":{"Volumes":{"/unowned":{}} if s["fault"] == "anonymous-volume" else mounts}})
    elif method == "GET":
        parts = path.split("/")
        key = parts[2]
        row = next((r for r in s["resources"].values() if (r["name"] == key or r["id"] == key) and r["kind"] == {"containers":"container","networks":"network","volumes":"volume"}[parts[1]]),None)
        if s["fault"] == "collision" and key == "unit-forge-app" and not row:
            reply(value={"Id":"f"*64})
        elif row:
            value = observed(row)
            before_health = s["fault"] == "drift-health" and key == "unit-forge-app" and row["running"] and s["resources"].get("container:unit-peer-app",{}).get("running")
            before_admin = s["fault"] == "drift-admin" and key == "unit-forge-app" and "health" in s["events"]
            before_sql = s["fault"] == "drift-sql" and key == "unit-forge-db" and parts[1] == "containers" and "git:verify" in s["events"]
            if before_health or before_admin or before_sql:
                value["Id"] = "f"*64
                value["Config"]["Labels"]["io.boxferry.live-run"] = "foreign-run"
            reply(value=value)
        else: reply(404)
    elif method == "DELETE":
        key = path.split("/")[2].split("?")[0]
        row = next(r for r in s["resources"].values() if (r["id"] == key or r["name"] == key) and r["kind"] == {"containers":"container","networks":"network","volumes":"volume"}[path.split("/")[1]])
        assert row["name"].startswith("unit-")
        if s["fault"] == "cleanup" and s["cleanup_started"]:
            save(); sys.exit(61)
        if row["kind"] == "container":
            assert path.endswith("&v=false")
            if not s["cleanup_started"]: assert "force=false" in path and not row["running"]
        if row["kind"] == "volume": assert s["cleanup_started"] and path.endswith("?force=false")
        s["events"].append("delete:"+row["name"])
        del s["resources"][row["kind"]+":"+row["name"]]
        reply(204)
    elif method == "POST" and ("/start" in path or "/stop" in path):
        key = path.split("/")[2]
        row = next(r for r in s["resources"].values() if r["id"] == key)
        row["running"] = "/start" in path
        s["events"].append(("start:" if row["running"] else "stop:")+row["name"])
        reply(204)
    elif method == "POST" and path.endswith("/connect"):
        assert path == "/networks/unit-shared-edge/connect"
        assert data == {"Container":"unit-forge-app","EndpointConfig":{"Aliases":["forgejo"]}}
        s["events"].append("attach")
        reply()
    elif method == "POST":
        kind = {"networks":"network","volumes":"volume","containers":"container"}[path.split("/")[1]]
        name = path.split("?name=")[1] if kind == "container" else data["Name"]
        if kind+":"+name in s["resources"]:
            assert name == "unit-forge-app" and data == s["app_request"]["body"]
            s["events"].append("collision-proof")
            reply(409)
        else:
            if name == "unit-shared-edge":
                assert data == {"Name":name,"Driver":"bridge","Internal":False,"CheckDuplicate":True,"Labels":labels(name)}
            elif name == "unit-peer-app":
                assert data == {"Image":"registry.invalid/unit/git-client:owned","Labels":labels(name),
                  "Entrypoint":["/bin/sh","-ceu"],"Cmd":["sleep 3600"],
                  "HostConfig":{"NetworkMode":"unit-shared-edge"},
                  "NetworkingConfig":{"EndpointsConfig":{"unit-shared-edge":{"Aliases":["peer"]}}}}
            else:
                assert {"method":method,"path":"/v1.56"+path,"body":data} in s["plan_requests"]
            s["serial"] += 1
            identity = name if kind == "volume" else hashlib.sha256((name+str(s["serial"])).encode()).hexdigest()
            s["resources"][kind+":"+name] = {"kind":kind,"name":name,"id":identity,"running":False}
            s["events"].append("create:"+name)
            if s["fault"] == "partial-create" and name == "unit-forge-app":
                save(); sys.exit(71)
            reply(201,{"Name" if kind == "volume" else "Id":identity})
    else: raise AssertionError("unreviewed API operation")
else: raise AssertionError("unreviewed transport operation")
'''

def encode(value):
    return (json.dumps(value, separators=(",", ":")) + "\n").encode()


def run_case(fault="", fail_call=0, change=None, expired=False, lane="upstream-rootless", tamper=None):
    with tempfile.TemporaryDirectory(prefix="boxferry-forgejo-hermetic-") as directory:
        directory = pathlib.Path(directory)
        os.chmod(directory, 0o700)
        transport = directory / "transport"
        transport.write_text(FAKE)
        transport.chmod(0o700)
        plan = copy.deepcopy(fixture.plan)
        if change:
            change(plan)
        raw = encode(plan)
        documents = {"plan": plan, "admission": fixture.admission(raw), "sidecar": fixture.sidecar(raw),
                     "profile": fixture.profile, "aliases": fixture.images, "interpolation": fixture.interpolation}
        documents = copy.deepcopy(documents)
        documents["profile"]["daemon_mode"] = lane.rsplit("-", 1)[1]
        documents["plan"]["context"] = copy.deepcopy(documents["profile"])
        documents["interpolation"]["context"]["lane"] = lane
        documents["interpolation"]["context"]["profile"] = copy.deepcopy(documents["profile"])
        documents["admission"]["lane"] = lane
        digest = hashlib.sha256(encode(documents["plan"])).hexdigest()
        documents["admission"]["docker_plan_sha256"] = digest
        documents["sidecar"]["docker_plan_sha256"] = digest
        if tamper:
            tamper(documents)
        for name, document in documents.items():
            (directory / name).write_bytes(encode(document))
        deadline = int(time.clock_gettime(time.CLOCK_BOOTTIME)) + (-1 if expired else 2 if fault == "blocked" else 15 if fault == "isolation-timeout" else 120)
        aliases = {**fixture.images,"git-client":"registry.invalid/unit/git-client:owned"}
        references = {}
        for row in (ROOT / "fixtures/conformance/forgejo-application/images.tsv").read_text().splitlines():
            if row and not row.startswith("#"):
                key, reference, *_ = row.split()
                references[key] = reference
        state = {"fault":fault,"fail_call":fail_call,"calls":0,"cleanup_started":False,"lane":lane,"recovered_health":False,
                 "execution":deadline,"cleanup":deadline+60,"resources":{},"images":[],"fixture":False,
                 "events":[],"serial":0,"references":references,"aliases":aliases,"isolation_positive":0,"first_sql_call":None,
                 "class_calls":{},"failed_class":None,
                 "bindings":{"docker_plan_sha256":digest,
                             "dependency_sidecar_sha256":hashlib.sha256(encode(documents["sidecar"])).hexdigest(),
                             "expectations_sha256":hashlib.sha256(fixture_module.CATALOGUE).hexdigest()},
                 "plan_requests":fixture.plan["requests"], "app_request":fixture.plan["requests"][4],
                 "fixture_digests":{name:hashlib.sha256((ROOT / "fixtures/conformance/forgejo-application" / name).read_bytes()).hexdigest()
                                    for name in ("git-probe.sh","repository-proof.txt")}}
        (directory / "state.json").write_text(json.dumps(state))
        command = ["bash","--noprofile","--norc","-c",'source "$1"; shift; docker_forgejo_application_run "$@"',
                   "hermetic",str(MODULE),str(transport),"owned-context",str(ROOT),
                   *[str(directory/name) for name in ("plan","admission","sidecar","profile","aliases","interpolation")],
                   lane,"offline-run","unit","/tmp/offline-fixture",str(deadline),str(deadline+60),"admin-canary"]
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=25,
                                env={**os.environ,"BASH_ENV":"/dev/null"})
        for secret in (fixture.password, fixture.key, "admin-canary", str(directory)):
            assert secret.encode() not in result.stdout + result.stderr
        assert not result.stdout
        final = json.loads((directory / "state.json").read_text())
        return result, final


# Source-only loading must not spawn even Python or consult a runtime.
source = subprocess.run(["bash","--noprofile","--norc","-c",
                         'PATH=/nonexistent; source "$1"; [[ $(type -t docker_forgejo_application_run) == function ]]',
                         "source-only",str(MODULE)], capture_output=True)
assert source.returncode == 0 and not source.stdout and not source.stderr
result, final = run_case()
assert result.returncode == 0, result.stderr.decode()
expected = ["image:postgres","image:forgejo","image:git-client","fixture",
            "create:unit-shared-edge","create:unit-forge-backend","create:unit-forge-db","create:unit-forge-data",
            "create:unit-forge-db","create:unit-forge-app","attach","create:unit-peer-app",
            "start:unit-forge-db","start:unit-forge-app","start:unit-peer-app","health","admin",
            "git:seed","git:verify","sql","isolation:version","isolation:backend","isolation:peer","isolation:backend",
            "collision-proof","stop:unit-forge-app","delete:unit-forge-app",
            "stop:unit-forge-db","delete:unit-forge-db","create:unit-forge-db","start:unit-forge-db",
            "create:unit-forge-app","attach","start:unit-forge-app","health","git:verify","sql",
            "isolation:version","isolation:backend","isolation:peer","isolation:backend",
            "delete:unit-forge-app","delete:unit-forge-db","delete:unit-peer-app",
            "delete:unit-forge-data","delete:unit-forge-db","delete:unit-forge-backend","delete:unit-shared-edge",
            "release-image","release-image","release-image"]
assert final["events"] == expected, final["events"]
assert not final["resources"] and not final["images"] and not final["fixture"]
first_sql_boundary = final["first_sql_call"]
assert isinstance(first_sql_boundary, int) and first_sql_boundary > 0
representatives = final["class_calls"]
required_classes = {
    "attest", "api-read", "api-read-absence", "api-read-owned", "api-read-binding",
    "image-provision", "fixture-create", "fixture-install",
    "create-network", "create-volume", "create-container", "attach-network", "start-container", "stop-container",
    "cli-health", "cli-admin", "cli-git-seed", "cli-git-verify", "cli-sql",
    "cli-isolation-version", "cli-isolation-backend", "cli-isolation-peer",
    "delete-container-recreate", "delete-container-cleanup", "delete-volume-cleanup", "delete-network-cleanup",
    "fixture-remove", "image-release",
}
assert set(representatives) == required_classes, (set(representatives) - required_classes, required_classes - set(representatives))
assert representatives["cli-sql"] == first_sql_boundary
checks = 2
result, final = run_case(lane="upstream-rootful")
assert result.returncode == 0 and not final["resources"] and not final["images"] and not final["fixture"]
checks += 1
for fault in ("attest","mode","native-version","collision","peer-isolation","wildcard-port","db-row","git-failure","partial-create","overflow","blocked","anonymous-volume",
              "reachable-peer","db-down","db-down-after","db-rejecting","probe-unavailable","probe-version","probe-invalid","isolation-timeout",
              "git-app-stopped","git-publication-drift"):
    result, final = run_case(fault)
    assert result.returncode != 0, fault
    assert not final["resources"] and not final["images"] and not final["fixture"], fault
    if fault == "isolation-timeout":
        assert "isolation:peer" in final["events"] and "delete:unit-isolation-probe" in final["events"]
    if fault in {"git-app-stopped","git-publication-drift"}:
        assert "git:seed" not in final["events"]
    checks += 1
result, final = run_case("isolation-cancel")
assert result.returncode == 130 and not final["resources"] and not final["images"] and not final["fixture"]
assert "delete:unit-isolation-probe" in final["events"]
checks += 1
for fault in ("foreign-owner","cleanup"):
    result, final = run_case(fault)
    assert result.returncode != 0 and final["resources"], fault
    if fault == "foreign-owner":
        assert "container:unit-forge-app" in final["resources"] and "delete:unit-forge-app" not in final["events"]
    checks += 1
for fault in ("db-volume-initial","db-volume-name","db-volume-recreate","data-volume-recreate"):
    result, final = run_case(fault)
    assert result.returncode != 0, fault
    if fault in {"db-volume-recreate","data-volume-recreate"}:
        assert final["events"].count("create:unit-forge-db") == 2
        assert final["events"].count("create:unit-forge-app") == 1
        assert "container:unit-forge-db" not in final["resources"] and "container:unit-forge-app" not in final["resources"]
    else:
        assert "create:unit-forge-app" not in final["events"]
    assert ("volume:unit-forge-data" if fault == "data-volume-recreate" else "volume:unit-forge-db") in final["resources"]
    checks += 1
for fault, withheld in (("drift-health","health"),("drift-admin","admin"),("drift-sql","sql")):
    result, final = run_case(fault)
    assert result.returncode != 0 and withheld not in final["events"], fault
    assert not final["resources"] and not final["images"] and not final["fixture"], fault
    checks += 1
for number in ("SIGINT","SIGTERM","SIGHUP","SIGQUIT"):
    result, final = run_case("cancel-"+number)
    assert result.returncode == 130 and not final["resources"] and not final["images"], number
    checks += 1
for mutate in (lambda p:p["requests"][4]["body"]["Env"].append("UNREVIEWED=private"),
               lambda p:p["requests"][4]["body"].update(Cmd=["unreviewed"]),
               lambda p:p["requests"][4]["body"].update(User="0:0"),
               lambda p:p["requests"][4]["body"]["HostConfig"]["PortBindings"]["3000/tcp"][0].update(HostIp="0.0.0.0")):
    result, final = run_case(change=mutate)
    assert result.returncode != 0 and final["calls"] == 0
    checks += 1
result, final = run_case(expired=True)
assert result.returncode != 0 and final["calls"] == 0
checks += 1
for tamper in (lambda d:d["sidecar"].update(docker_plan_sha256="00"*32),
               lambda d:d["admission"].update(docker_plan_sha256="00"*32),
               lambda d:d["sidecar"].update(decisions=[]),
               lambda d:d["profile"].update(rendering_api_version="1.01")):
    result, final = run_case(tamper=tamper)
    assert result.returncode != 0 and final["calls"] == 0
    checks += 1
# Representative independently named error classes from the complete successful
# journey, including the first actual SQL dispatch and cleanup-only operations.
# Equivalent transport reads/mutations are exercised once; this is not exhaustive
# prefix replay. Explicit hostile/partial-create/state-transition controls above
# cover state-dependent ownership, isolation, ambiguity, timeout and cancellation.
for classification, index in sorted(representatives.items(), key=lambda item:item[1]):
    result, final = run_case(fail_call=index)
    assert final["failed_class"] == classification, classification
    assert result.returncode == 0 if final["recovered_health"] else result.returncode != 0, index
    if classification.startswith("delete-") and classification.endswith("-cleanup"):
        assert final["resources"] and not final["images"] and not final["fixture"], classification
    elif classification == "fixture-remove":
        assert final["fixture"] and not final["resources"] and not final["images"]
    elif classification == "image-release":
        assert final["images"] and not final["resources"] and not final["fixture"]
    else:
        assert not final["resources"] and not final["images"] and not final["fixture"], classification
    checks += 1
print(f"Forgejo Docker profile: {checks} hermetic source controls passed; native/application acceptance unmeasured.")
PY
