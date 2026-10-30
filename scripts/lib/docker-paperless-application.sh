#!/usr/bin/env bash
# Inert Paperless application choreography; the caller owns all native transport.
# shellcheck source=scripts/lib/paperless-application-probes.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/paperless-application-probes.sh" || return 1

docker_paperless_contract() {
  PYTHONDONTWRITEBYTECODE=1 python3 - "$@" << 'PY'
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import stat
import sys
import time


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path):
    path = pathlib.Path(path)
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= 1048576,
            "input is not a bounded regular file")
    with path.open("rb") as stream:
        raw = stream.read(1048577)
    require(0 < len(raw) <= 1048576, "input size differs")
    return raw


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def document(raw):
    return json.loads(raw, object_pairs_hook=pairs,
                      parse_constant=lambda _: require(False, "nonfinite JSON"))


def write(path, value):
    raw = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        require(stat.S_ISREG(os.fstat(stream.fileno()).st_mode), "output is not a regular file")
        stream.write(raw)
        os.fchmod(stream.fileno(), 0o600)
    return digest(raw)


def same(left, right):
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def fields(value, names):
    require(isinstance(value, dict) and set(value) == set(names.split()), "fields differ")


def sha(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def source_state(manifest, state, count):
    fields(state, "schema documents")
    require(type(state["schema"]) is int and state["schema"] == 1
            and isinstance(state["documents"], list) and len(state["documents"]) == count,
            "source document state inventory differs")
    expected = manifest["document_expectations"]
    ids, seen = set(), set()
    for row in state["documents"]:
        require(isinstance(row, dict), "document record differs")
        phase, extension = row.get("phase"), row.get("extension")
        require((phase, extension) not in seen and phase in ("baseline", "second")
                and extension in ("pdf", "docx", "odt"), "document record identity differs")
        require(phase == "baseline" or count == 6, "unexpected second batch")
        seen.add((phase, extension))
        fields(row, "phase extension title marker original_sha256 id" + (" archive_sha256" if extension != "pdf" else ""))
        require(type(row["id"]) is int and row["id"] > 0 and row["id"] not in ids, "document IDs differ")
        ids.add(row["id"])
        for name, value in expected[phase][extension].items():
            require(row[name] == value, "canonical document bytes or search identity differ")
        if extension != "pdf":
            require(sha(row["archive_sha256"]), "PDF archive evidence absent")
    baseline = [row for row in state["documents"] if row["phase"] == "baseline"]
    require(len(baseline) == 3, "baseline source document set differs")
    if manifest["config"]["stage"] == "generated-target":
        handoff = document(read(manifest["config"]["target"]["source_handoff"]))
        require(same(baseline, handoff["source_state"]["documents"]), "pre-existing source documents changed")


def handoff(manifest, value):
    fields(value, "schema_version kind application prefix run_id plan_sha256 probe_sha256 source_state")
    require(type(value["schema_version"]) is int and value["schema_version"] == 1
            and value["kind"] == "paperless-source-handoff" and value["application"] == "paperless-ngx"
            and all(isinstance(value[key], str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", value[key])
                    for key in ("prefix", "run_id")) and sha(value["plan_sha256"])
            and value["probe_sha256"] == manifest["probe_sha256"], "source handoff identity differs")
    # Validate independently of target comparison, avoiding a self-comparison.
    source_manifest = dict(manifest, config=dict(manifest["config"], stage="source-fixture"))
    source_state(source_manifest, value["source_state"], 3)


def initialize(root, directory, config_path):
    root, directory = pathlib.Path(root), pathlib.Path(directory)
    spec = importlib.util.spec_from_file_location(
        "paperless_expectations", root / "scripts/lib/docker-application-expectations.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    config = module.document(read(config_path))
    fields(config, "schema_version stage prefix run_id fixture_root endpoint profile image_aliases "
                   "image_config_ids plan admission work_deadline_ms cleanup_deadline_ms target")
    require(type(config["schema_version"]) is int and config["schema_version"] == 1, "schema differs")
    require(config["stage"] in ("source-fixture", "generated-target"), "stage differs")
    endpoint = config["endpoint"]
    fields(endpoint, "id uri scope mode ownership_sha256")
    require(module.identity(endpoint["id"]) and endpoint["scope"] == "owned-inner"
            and endpoint["mode"] == "rootless" and sha(endpoint["ownership_sha256"]), "endpoint custody differs")
    require(isinstance(endpoint["uri"], str) and re.fullmatch(r"unix:///[A-Za-z0-9_./-]+", endpoint["uri"])
            and ".." not in endpoint["uri"].split("/")
            and endpoint["uri"] not in ("unix:///var/run/docker.sock", "unix:///run/docker.sock"),
            "endpoint is not an explicit owned inner socket")
    now = time.time_ns() // 1000000
    work, cleanup = config["work_deadline_ms"], config["cleanup_deadline_ms"]
    require(type(work) is int and type(cleanup) is int and now < work < cleanup
            and cleanup - now <= 86400000, "absolute deadline or cleanup reserve differs")
    catalogue_raw = read(root / module.CATALOGUE_PATH)
    module.check_sources(root, catalogue_raw)
    app = module.catalogue(catalogue_raw)["applications"]["paperless-ngx"]
    plan_raw, admission_raw = read(config["plan"]), read(config["admission"])
    module.validate_application(plan_raw, admission_raw, catalogue_bytes=catalogue_raw,
                                application="paperless-ngx", lane="upstream-rootless",
                                profile=config["profile"], image_aliases=config["image_aliases"],
                                prefix=config["prefix"], run_id=config["run_id"],
                                fixture_root=config["fixture_root"])
    plan = module.document(plan_raw)
    held = {}

    def hold(name, raw):
        path = directory / name
        path.write_bytes(raw)
        os.chmod(path, 0o600)
        held[name] = digest(raw)
        return str(path)

    source_files = {}
    for index, row in enumerate(app["sources"]):
        source_files[row["path"]] = hold(f"source-{index}", read(root / row["path"]))
    hold("catalogue.json", catalogue_raw)
    config["plan"] = hold("plan.json", plan_raw)
    config["admission"] = hold("admission.json", admission_raw)
    # Read only literal public test declarations; never source the live module.
    canaries = {}
    legacy = read(root / "scripts/lib/paperless-application.sh").decode()
    for name in ("ADMIN_USER", "ADMIN_PASSWORD", "DB_PASSWORD", "REDIS_PASSWORD", "SECRET_KEY"):
        values = re.findall(r'^readonly PAPERLESS_' + name + r'="([^"$`\\\n]+)"$', legacy, re.M)
        require(len(values) == 1, "canonical public credential declaration differs")
        canaries[name] = values[0]
    hold("public-credentials-source", legacy.encode())
    budget = {}
    for key, declaration in (("cpus", "MIN_CPUS"), ("memory_kib", "MIN_MEMORY_KIB"),
                             ("disk_kib", "MIN_DISK_KIB"), ("image_archive_bytes", "ARCHIVE_MAX_BYTES")):
        matches = re.findall(r'^readonly PAPERLESS_' + declaration + r'="([1-9][0-9]*)"$', legacy, re.M)
        require(len(matches) == 1, "canonical application budget declaration differs")
        budget[key] = int(matches[0])
    image_raw = read(root / app["image_catalogue"])
    images = {row.split("\t")[0]: row.split("\t")[1]
              for row in image_raw.decode().splitlines() if row and not row.startswith("#")}
    require(set(config["image_config_ids"]) == set(images), "image configuration inventory differs")
    require(all(isinstance(value, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", value)
                for value in config["image_config_ids"].values()), "image configuration identity differs")
    prefix = config["prefix"]
    values = {"BF_PREFIX": prefix, "BF_FIXTURE_ROOT": config["fixture_root"],
              "BF_RUN": config["run_id"], **{f"BF_{key}": value for key, value in canaries.items()}}
    values.update({f"BF_{key.upper()}_IMAGE": value for key, value in config["image_aliases"].items()})
    compose = read(root / "fixtures/conformance/paperless-ngx-application/compose.yaml").decode()

    def expand(value):
        return re.sub(r"\$\{([A-Z_]+)(?::\?required)?\}", lambda match: values[match[1]], value)

    # This deliberately narrow reader accepts only the already hash-bound authored
    # fixture's scalar environment and command forms. It is not a Compose parser.
    settings = {}
    for role in app["services"]:
        block = re.search(r"^  " + role + r":\n(.*?)(?=^  [a-z]|^networks:)", compose, re.M | re.S)[1]
        env = re.search(r"^    environment:[^\n]*\n((?:      [^\n]+\n)+)", block, re.M)
        environment = {}
        if env:
            for line in env[1].splitlines():
                key, value = line.strip().split(": ", 1)
                environment[key] = expand(json.loads(value) if value.startswith('"') else value)
        command = re.search(r"^    command: (\[[^\n]+\])$", block, re.M)
        multiline = re.search(r"^    command:\n((?:      - [^\n]+\n)+)", block, re.M)
        argv = json.loads(command[1]) if command else ([line[8:] for line in multiline[1].splitlines()] if multiline else None)
        settings[role] = {"environment": environment, "command": [expand(item) for item in argv] if argv else None,
                          "pull_policy": "never"}
    probe_path = root / "fixtures/conformance/paperless-ngx-application/document-probe.py"
    probe_spec = importlib.util.spec_from_file_location("held_canonical_paperless_probe", probe_path)
    probe = importlib.util.module_from_spec(probe_spec)
    probe_spec.loader.exec_module(probe)
    document_expectations = {}
    for phase in ("baseline", "second"):
        document_expectations[phase] = {extension: {
            "phase": phase, "extension": extension, "title": f"BoxFerry migration {phase} {extension.upper()}",
            "marker": marker, "original_sha256": digest(payload)}
            for extension, (marker, payload) in probe.generated_documents(phase).items()}
    # Close the deferred native fields: topology validation alone does not review
    # commands, environment, entrypoints, users, or image-default overrides.
    for row in plan["requests"]:
        if "containers/create?name=" not in row["path"]:
            continue
        body = row["body"]
        role = next(key for key, service in app["services"].items()
                    if row["path"].endswith("name=" + prefix + "-" + service["runtime_suffix"]))
        expected = settings[role]
        env = body.get("Env", [])
        require(len(env) == len(set(item.split("=", 1)[0] for item in env))
                and all("=" in item for item in env)
                and dict(item.split("=", 1) for item in env) == expected["environment"],
                "held plan authored environment differs")
        require(same(body.get("Cmd"), expected["command"]), "held plan authored command differs")
        require(not set(body).intersection({"Entrypoint", "User", "WorkingDir", "StopSignal", "StopTimeout"}),
                "unreviewed image default override")
        require(("Healthcheck" in body) == (role in ("db", "broker")), "healthcheck inventory differs")
        if "Healthcheck" in body:
            expected_test = {"db": ["CMD-SHELL", "pg_isready -U paperless -d paperless"],
                             "broker": ["CMD", "valkey-cli", "-a", canaries["REDIS_PASSWORD"], "ping"]}
            require(role in expected_test and same(body["Healthcheck"], {
                "Test": expected_test[role], "Interval": 2000000000, "Timeout": 2000000000, "Retries": 90}),
                "held plan healthcheck differs")
    target = config["target"]
    if config["stage"] == "generated-target":
        fields(target, "artifact artifact_sha256 neutral_expectations neutral_expectations_sha256 authorization authorization_sha256 source_handoff source_handoff_sha256")
        for name in ("artifact", "neutral_expectations", "authorization", "source_handoff"):
            raw = read(target[name])
            require(sha(target[name + "_sha256"]) and digest(raw) == target[name + "_sha256"], "target custody bytes differ")
            target[name] = hold("target-" + name, raw)
        authorization = module.document(read(target["authorization"]))
        require(same(authorization, {
            "schema_version": 1, "kind": "caller-owned-allowlisted-paperless-target",
            "application": "paperless-ngx", "prefix": prefix, "run_id": config["run_id"],
            "endpoint": endpoint, "profile": config["profile"], "plan_sha256": digest(plan_raw),
            "artifact_sha256": target["artifact_sha256"],
            "neutral_expectations_sha256": target["neutral_expectations_sha256"],
            "source_handoff_sha256": target["source_handoff_sha256"],
            "authored_fields_reviewed": True, "loss_policy_reviewed": True,
            "isolated_test_harness_only": True}), "target allowlist custody differs")
    else:
        require(target is None, "source fixture cannot claim generated target custody")
    manifest = {"schema_version": 1, "kind": "prepared-paperless-application-source",
                "native_execution": False, "config": config, "held": held,
                "sources": source_files, "application": app, "images": images,
                "settings": settings, "canaries": canaries, "budget": budget, "document_expectations": document_expectations,
                "probe_sha256": digest(read(probe_path))}
    if target is not None:
        handoff(manifest, module.document(read(target["source_handoff"])))
    print(write(directory / "manifest.json", manifest))


def load(directory, expected_hash, initial_hash, recreated_hash, verified_source_hash):
    directory = pathlib.Path(directory)
    raw = read(directory / "manifest.json")
    require(digest(raw) == expected_hash, "held manifest changed")
    manifest = document(raw)
    for name, expected in manifest["held"].items():
        require(digest(read(directory / name)) == expected, "held input changed")
    snapshots = []
    for name, expected in (("initial.json", initial_hash), ("recreated.json", recreated_hash)):
        if expected:
            raw = read(directory / name)
            require(digest(raw) == expected, "held resource receipt changed")
            snapshots.append(document(raw))
    verified_source_state = None
    if verified_source_hash:
        raw = read(directory / "verified-source-state.json")
        require(digest(raw) == verified_source_hash, "held verified source state changed")
        verified_source_state = document(raw)
    return manifest, snapshots, verified_source_state


def graph(manifest, payload, previous):
    fields(payload, "resources dependencies storage image_bindings")
    config, app = manifest["config"], manifest["application"]
    prefix = config["prefix"]
    labels = {"io.boxferry.live-run": config["run_id"], "io.boxferry.application": prefix + "-paperless"}
    resources = payload["resources"]
    require(isinstance(resources, dict), "resource inventory differs")
    wanted = {}
    for group, kind in (("networks", "network"), ("volumes", "volume"), ("services", "container")):
        for role, node in app[group].items():
            name = prefix + "-" + node["runtime_suffix"]
            wanted[name] = kind
            row = resources.get(name)
            fields(row, "kind identity labels specification" + (" image_config_id running healthy settings" if kind == "container" else ""))
            require(row["kind"] == kind and same(row["labels"], labels) and same(row["specification"], node),
                    "resource graph, ownership or isolation differs")
            require(isinstance(row["identity"], str) and re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", row["identity"]),
                    "resource identity absent")
            if kind != "volume":
                require(re.fullmatch(r"[0-9a-f]{64}", row["identity"]) is not None, "native resource ID differs")
            if kind == "container":
                require(row["image_config_id"] == config["image_config_ids"][node["image_key"]]
                        and row["running"] is True and row["healthy"] is (role in ("db", "broker"))
                        and same(row["settings"], manifest["settings"][role]), "container settings or image differ")
            if previous:
                old = previous["resources"][name]["identity"]
                require((old != row["identity"]) if kind == "container" else (old == row["identity"]),
                        "recreation did not replace every container or retained storage/network changed")
                if kind == "container":
                    require(row["identity"] not in {item["identity"] for item in previous["resources"].values()
                                                    if item["kind"] == "container"}, "recreation reused an old container")
    require(set(resources) == set(wanted), "resource inventory differs")
    identities = [row["identity"] for row in resources.values()]
    require(len(identities) == len(set(identities)), "resource identities collide")
    require(same(payload["dependencies"], app["dependencies"]), "dependency barrier receipt differs")
    expected_images = {key: {"reference": value, "alias": config["image_aliases"][key],
                              "config_id": config["image_config_ids"][key]} for key, value in manifest["images"].items()}
    require(same(payload["image_bindings"], expected_images), "image pin/configuration binding differs")
    expected_storage = {prefix + "-paper-" + name: {"uid": 1000, "gid": 1000,
                        "world_writable": False, "application_uid": 1000, "application_gid": 1000,
                        "read": True, "write": True, "create_remove": True}
                        for name in ("data", "media", "consume", "export")}
    require(same(payload["storage"], expected_storage), "application storage ownership/access differs")


def receipt(manifest, snapshots, verified_source_state, request, reply, directory, request_sha):
    config, action = manifest["config"], request["action"]
    fields(reply, "schema_version request_sha256 manifest_sha256 action stage endpoint payload")
    require(type(reply["schema_version"]) is int and reply["schema_version"] == 1
            and reply["request_sha256"] == request_sha == digest(read(directory / "request.json"))
            and reply["manifest_sha256"] == request["manifest_sha256"]
            and reply["action"] == action and reply["stage"] == config["stage"]
            and same(reply["endpoint"], config["endpoint"]), "transport receipt custody differs")
    require(time.time_ns() // 1000000 < request["deadline_ms"], "absolute deadline expired")
    payload = reply["payload"]
    if action == "preflight":
        fields(payload, "collisions process_tree_watchdog absolute_deadlines cleanup_reserved no_fallback no_implicit_pulls budget")
        require(same({key: value for key, value in payload.items() if key != "budget"},
                     {"collisions": [], "process_tree_watchdog": True,
                              "absolute_deadlines": True, "cleanup_reserved": True,
                              "no_fallback": True, "no_implicit_pulls": True}), "preflight custody differs")
        measured, required = payload["budget"], manifest["budget"]
        fields(measured, "cpus memory_kib disk_kib image_archive_bytes")
        require(all(type(value) is int and value > 0 for value in measured.values())
                and all(measured[key] >= required[key] for key in ("cpus", "memory_kib", "disk_kib"))
                and measured["image_archive_bytes"] <= required["image_archive_bytes"], "application resource budget differs")
    elif action in ("provision", "recreate", "inspect"):
        graph(manifest, payload, snapshots[0] if action == "recreate" else None)
        if action == "inspect":
            require(snapshots and same(payload, snapshots[-1]), "observed graph drifted")
        else:
            print(write(directory / ("initial.json" if action == "provision" else "recreated.json"), payload))
    elif action == "probe":
        fields(payload, "exit_code probe_sha256 state")
        require(type(payload["exit_code"]) is int and payload["exit_code"] == 0
                and payload["probe_sha256"] == manifest["probe_sha256"], "canonical document probe failed")
        if request["arguments"][2] == "ready":
            require(payload["state"] is None, "readiness cannot claim document evidence")
        else:
            source_state(manifest, payload["state"], request["expected_documents"])
            if config["stage"] == "source-fixture" and request["arguments"][2] == "verify":
                # Hold the genuine verification observation independently of any
                # later handoff assertion; the Bash owner retains its byte hash.
                print(write(directory / "verified-source-state.json", payload["state"]))
    elif action == "handoff":
        handoff(manifest, payload)
        require(config["stage"] == "source-fixture" and payload["prefix"] == config["prefix"]
                and payload["run_id"] == config["run_id"]
                and payload["plan_sha256"] == manifest["held"]["plan.json"], "source handoff origin differs")
        require(verified_source_state is not None and same(payload["source_state"], verified_source_state),
                "source handoff differs from last verified document state")
        print(digest((json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n").encode()))
    elif action == "broker":
        fields(payload, "lpush_calls")
        count = payload["lpush_calls"]
        require(type(count) is int and 0 <= count <= 9223372036854775807, "actual LPUSH count differs")
        print(count)
    elif action == "query":
        fields(payload, "rows")
        require(payload["rows"] in ("3", "6"), "database result differs")
        print(payload["rows"])
    elif action == "privacy":
        # Actual bounded default-export/report bytes, not a boolean privacy claim.
        fields(payload, "report artifacts")
        report, artifacts = payload["report"], payload["artifacts"]
        require(isinstance(report, dict) and isinstance(artifacts, list) and artifacts
                and all(isinstance(item, str) and item for item in artifacts), "privacy evidence absent")
        require(type(report.get("schema_version")) is int and report["schema_version"] == 1 and report.get("status") == "success"
                and report.get("exit_category") == "success" and report.get("fidelity", {}).get("invalid", 0) == 0
                and report.get("output_artifacts") and isinstance(report.get("diagnostics"), list), "privacy report differs")
        diagnostics = report["diagnostics"]
        require(not any(row.get("severity") == "error" for row in diagnostics), "privacy export failed")
        expected_fields = {"subject": f"services.{config['prefix']}-paper-web.environment.PAPERLESS_DBPASS",
                           "decision": "omitted", "required_loss_policy": "partial"}
        matches = [row for row in diagnostics if row.get("code") == "BFP0002" and row.get("severity") == "warning"
                   and all({"name": key, "value": value} in row.get("fields", []) for key, value in expected_fields.items())]
        require(len(matches) == 1, "default withholding diagnostic differs")
        raw = json.dumps(payload)
        require(all(value not in raw for key, value in manifest["canaries"].items() if key != "ADMIN_USER"),
                "protected test value survived export")
    elif action == "cleanup":
        fields(payload, "created removed remaining unowned_touched reconciled uncertain")
        require(payload["reconciled"] is True and payload["uncertain"] is False
                and payload["remaining"] == [] and payload["unowned_touched"] == []
                and same(payload["created"], payload["removed"]), "cleanup uncertain or incomplete")
        created = payload["created"]
        require(isinstance(created, list), "cleanup ledger differs")
        known = []
        for snapshot in snapshots:
            for name, row in snapshot["resources"].items():
                entry = {"name": name, "kind": row["kind"], "identity": row["identity"]}
                if entry not in known:
                    known.append(entry)
        app = manifest["application"]
        names = {config["prefix"] + "-" + row["runtime_suffix"]: kind
                 for group, kind in (("networks", "network"), ("volumes", "volume"), ("services", "container"))
                 for row in app[group].values()}
        require(len(created) == len({json.dumps(row, sort_keys=True) for row in created}), "duplicate cleanup identity")
        for row in created:
            fields(row, "name kind identity")
            require(row["name"] in names and row["kind"] == names[row["name"]]
                    and isinstance(row["identity"], str) and row["identity"], "cleanup exceeded owned inventory")
        require(all(row in created for row in known), "cleanup omitted observed identity")
        if len(snapshots) == 2:
            require(len(created) == len(known), "cleanup ledger contains unobserved creations")
    else:
        raise ValueError("unknown transport action")


try:
    command, *args = sys.argv[1:]
    if command == "prepare":
        initialize(*args)
    else:
        directory, expected_hash, initial_hash, recreated_hash, verified_source_hash, *args = args
        directory = pathlib.Path(directory)
        manifest, snapshots, verified_source_state = load(directory, expected_hash, initial_hash, recreated_hash, verified_source_hash)
        if command == "request":
            expected_documents, action, *arguments = args
            config = manifest["config"]
            deadline = config["cleanup_deadline_ms" if action == "cleanup" else "work_deadline_ms"]
            require(time.time_ns() // 1000000 < deadline, "absolute deadline expired")
            request = {
                "schema_version": 1, "action": action, "arguments": arguments,
                "stage": config["stage"], "manifest": str(directory / "manifest.json"),
                "manifest_sha256": expected_hash, "endpoint": config["endpoint"],
                "deadline_ms": deadline, "work_deadline_ms": config["work_deadline_ms"],
                "cleanup_deadline_ms": config["cleanup_deadline_ms"]}
            if action == "probe":
                require(expected_documents in ("3", "6"), "expected document count differs")
                request["expected_documents"] = int(expected_documents)
            if action == "cleanup":
                request["cleanup_scope"] = {
                    "labels": {"io.boxferry.live-run": config["run_id"],
                               "io.boxferry.application": config["prefix"] + "-paperless"},
                    "resources": {config["prefix"] + "-" + row["runtime_suffix"]: kind
                                  for group, kind in (("networks", "network"), ("volumes", "volume"), ("services", "container"))
                                  for row in manifest["application"][group].values()}}
            print(write(directory / "request.json", request))
        elif command == "receipt":
            receipt(manifest, snapshots, verified_source_state, document(read(directory / "request.json")),
                    document(read(directory / "reply.json")), directory, args[0])
        elif command == "database-password":
            print(manifest["canaries"]["DB_PASSWORD"])
        else:
            raise ValueError("unknown local operation")
except (ValueError, KeyError, TypeError, OSError, RecursionError, AttributeError, IndexError, StopIteration):
    print("prepared Paperless contract rejected (private details withheld)", file=sys.stderr)
    raise SystemExit(1)
PY
}

docker_paperless_call() {
  local request_sha
  # Never truncate through a path left as a symlink by a failed transport.
  rm -f -- "${paperless_directory}/request.json" "${paperless_directory}/reply.json" || return $?
  if [[ "${1}" == cleanup ]]; then
    # Keep cleanup reachable after missing/failed/changed provisioning receipts.
    # Caller reconciles its own ledger from this pre-held exact ownership scope.
    printf '%s\n' "${paperless_cleanup_request}" > "${paperless_directory}/request.json" || return $?
    request_sha=${paperless_cleanup_sha}
  else
    request_sha="$(docker_paperless_contract request "${paperless_directory}" "${paperless_manifest_sha}" \
      "${paperless_initial_sha}" "${paperless_recreated_sha}" "${paperless_verified_source_sha}" "${paperless_expected_documents}" "$@")" || return $?
  fi
  : > "${paperless_directory}/reply.json" || return $?
  "${paperless_transport}" "${paperless_context}" "${paperless_directory}/request.json" \
    "${paperless_directory}/reply.json" > /dev/null 2>&1 || return $?
  docker_paperless_contract receipt "${paperless_directory}" "${paperless_manifest_sha}" \
    "${paperless_initial_sha}" "${paperless_recreated_sha}" "${paperless_verified_source_sha}" "${request_sha}"
}

docker_paperless_probe() {
  local context=$1 prefix=$2
  shift 2
  if [[ "${stage}" == source-fixture && "${1}" == verify ]]; then
    local verified_sha
    verified_sha="$(docker_paperless_call probe "${context}" "${prefix}" "$@")" || return $?
    [[ "${verified_sha}" =~ ^[0-9a-f]{64}$ ]] || return 1
    paperless_verified_source_sha=${verified_sha}
  else
    docker_paperless_call probe "${context}" "${prefix}" "$@"
  fi
}

docker_paperless_broker() {
  docker_paperless_call broker "$@"
}

docker_paperless_query() {
  docker_paperless_call query "$@"
}

docker_paperless_application_run() (
  # Subshell scopes callbacks' dynamic context and cancellation traps to this run.
  umask 077
  [[ $# == 4 ]] || return 2
  local repository=$1 paperless_transport=$2 paperless_context=$3 config=$4
  local paperless_directory paperless_manifest_sha paperless_initial_sha='' paperless_recreated_sha=''
  local paperless_verified_source_sha=''
  local paperless_cleanup_sha paperless_cleanup_request
  local password prefix stage paperless_expected_documents=3 liability=0 status=0
  declare -F "${paperless_transport}" > /dev/null || return 2
  paperless_directory="$(mktemp -d "${TMPDIR:-/tmp}/boxferry-paperless.XXXXXXXX")" || return $?
  trap 'exit 130' INT
  trap 'exit 143' TERM
  trap '
    status=$?
    trap - EXIT INT TERM
    if [[ "${liability}" == 1 ]]; then
      if ! docker_paperless_call cleanup; then
        printf "Prepared Paperless cleanup is uncertain.\n" >&2
        [[ "${status}" != 0 ]] || status=1
      fi
    fi
    rm -rf -- "${paperless_directory}"
    exit "${status}"
  ' EXIT
  paperless_manifest_sha="$(docker_paperless_contract prepare "${repository}" "${paperless_directory}" "${config}")" || return $?
  paperless_cleanup_sha="$(docker_paperless_contract request "${paperless_directory}" "${paperless_manifest_sha}" '' '' '' 3 cleanup)" || return $?
  paperless_cleanup_request="$(< "${paperless_directory}/request.json")"
  password="$(docker_paperless_contract database-password "${paperless_directory}" "${paperless_manifest_sha}" '' '' '')" || return $?
  prefix="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["config"]["prefix"])' "${paperless_directory}/manifest.json")" || return $?
  stage="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["config"]["stage"])' "${paperless_directory}/manifest.json")" || return $?
  docker_paperless_call preflight || return $?
  # Liability starts before the first provision attempt, including absent replies.
  liability=1
  paperless_initial_sha="$(docker_paperless_call provision)" || return $?
  docker_paperless_call inspect || return $?
  paperless_probe_wait_application docker_paperless_probe "${paperless_context}" "${prefix}" || return $?
  if [[ "${stage}" == source-fixture ]]; then
    paperless_probe_ingest_phase docker_paperless_probe docker_paperless_broker "${paperless_context}" "${prefix}" baseline || return $?
  fi
  paperless_probe_verify_documents docker_paperless_probe "${paperless_context}" "${prefix}" || return $?
  paperless_probe_assert_database docker_paperless_query "${paperless_context}" paperless "${password}" 3 || return $?
  docker_paperless_call privacy || return $?
  if [[ "${stage}" == source-fixture ]]; then
    docker_paperless_call handoff
    return $?
  fi
  paperless_recreated_sha="$(docker_paperless_call recreate)" || return $?
  docker_paperless_call inspect || return $?
  paperless_probe_wait_application docker_paperless_probe "${paperless_context}" "${prefix}" || return $?
  paperless_probe_verify_documents docker_paperless_probe "${paperless_context}" "${prefix}" || return $?
  paperless_probe_assert_database docker_paperless_query "${paperless_context}" paperless "${password}" 3 || return $?
  paperless_expected_documents=6
  paperless_probe_ingest_phase docker_paperless_probe docker_paperless_broker "${paperless_context}" "${prefix}" second || return $?
  paperless_probe_verify_documents docker_paperless_probe "${paperless_context}" "${prefix}" || return $?
  paperless_probe_assert_database docker_paperless_query "${paperless_context}" paperless "${password}" 6 || return $?
  docker_paperless_call inspect || return $?
  docker_paperless_call privacy || return $?
)
