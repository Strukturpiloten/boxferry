#!/usr/bin/env python3
"""Closed protected-service-alias contracts shared by the application harnesses."""

import hashlib
import json
import os
from pathlib import Path
import stat
import sys

def read_regular(path, limit):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError("nonregular source")
        raw = stream.read(limit + 1)
    if not raw or len(raw) > limit:
        raise ValueError("unbounded source")
    return raw


def read_unit(path):
    try:
        raw = read_regular(path, 1048576)
        return raw, raw.decode("utf-8").splitlines()
    except (OSError, UnicodeError, ValueError):
        raise SystemExit("invalid protected service-alias source") from None

def closed_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate protected-alias refusal field")
        result[key] = value
    return result


def inventory(args):
    mode, source, stem, specification = args
    try:
        role_aliases = json.loads(specification)[mode]
    except (ValueError, KeyError, TypeError):
        raise SystemExit("invalid protected service-alias specification") from None
    root = Path(source)
    if mode not in ("cli", "compose") or not root.is_dir() or root.is_symlink():
        raise SystemExit("invalid protected service-alias source")
    paths = sorted(root.glob("*.container"))
    if not paths or len(paths) > 16:
        raise SystemExit("invalid protected service-alias inventory")
    subjects = []
    digest = hashlib.sha256()
    for path in paths:
        role = path.stem.removeprefix(stem)
        if path.is_symlink() or not path.stem.startswith(stem) or role not in role_aliases:
            raise SystemExit("unreviewed protected service-alias source")
        raw, lines = read_unit(path)
        digest.update(path.name.encode() + b"\0" + raw + b"\0")
        container_lines, active = [], False
        if lines.count("[Container]") != 1:
            raise SystemExit("ambiguous protected service-alias source section")
        for line in lines:
            if line.startswith("[") and line.endswith("]"):
                active = line == "[Container]"
            elif active:
                container_lines.append(line)
        # Group/pod NetworkAlias fields are outside ADR 0073's service-only guard.
        aliases = [line.removeprefix("NetworkAlias=") for line in container_lines if line.startswith("NetworkAlias=")]
        networks = [line.removeprefix("Network=") for line in container_lines if line.startswith("Network=")]
        expected = [stem + role if alias == "@service" else alias for alias in role_aliases[role]]
        if aliases != expected:
            raise SystemExit("protected service-alias source differs from independent literal inventory")
        if aliases:
            if networks != [stem + "backend.network"]:
                raise SystemExit("protected service aliases have ambiguous network association")
            subjects.extend(f"services.{path.stem}.networks.{stem}backend.aliases[{index}]"
                            for index in range(len(aliases)))
    print(json.dumps({"subjects": subjects, "source_sha256": digest.hexdigest()}, separators=(",", ":")))


def assert_refusal(args):
    try:
        report, destination, inventory, expected = args
        raw = read_regular(report, 8388608)
        data = json.loads(raw, object_pairs_hook=closed_object)
        subjects = json.loads(inventory)["subjects"]
        baseline = json.loads(expected)
        expected_help = "Review the named subject. Protected service network aliases block every loss policy; keep private alias configuration outside the generated document. Otherwise, use --loss-policy partial only when a candidate exists and omitted intent is acceptable."
        if data.get("source_type") != "quadlet" or data.get("target_type") != "compose":
            raise ValueError("wrong refusal route")
        if not isinstance(data.get("fix_first"), dict) or data["fix_first"].get("code") != "BFC0007" or data["fix_first"].get("help") != expected_help:
            raise ValueError("wrong refusal remediation")
        if not subjects or type(data.get("schema_version")) is not int or data.get("schema_version") != 1 or data.get("status") != "blocked" or data.get("exit_category") != "policy-blocked":
            raise ValueError("wrong refusal status")
        if data.get("output_artifacts") != [] or os.path.lexists(destination):
            raise ValueError("refusal emitted output")
        facts, refusals = [], []
        for item in data["diagnostics"]:
            fields = {field["name"]: field["value"] for field in item["fields"]}
            if len(fields) != len(item["fields"]) or not item.get("name"):
                raise ValueError("malformed refusal diagnostic")
            subject = fields.get("subject")
            if subject in subjects:
                if (item["code"] != "BFC0007" or item["severity"] != "warning"
                        or item.get("help") != expected_help
                        or set(fields) != {"subject", "reason"}
                        or fields["reason"] != "protected network aliases cannot retain sensitivity in fresh Compose text; keep protected alias configuration outside this generated document"):
                    raise ValueError("wrong protected-alias diagnostic or help")
                refusals.append(subject)
            facts.append({"code": item["code"], "severity": item["severity"],
                          "subject": subject, "decision": fields.get("decision")})
        # The source inventory fixes deterministic alias-slot order independently of the failure report.
        if refusals != subjects:
            raise ValueError("missing, extra, duplicated or reordered refusal")
        alias_facts = [{"code":"BFC0007","severity":"warning","subject":subject,"decision":None} for subject in subjects]
        canonical = lambda values: sorted(json.dumps(value, sort_keys=True) for value in values)
        if canonical(facts) != canonical(baseline["diagnostics"] + alias_facts):
            raise ValueError("other diagnostics drifted")
        fidelity = data["fidelity"]
        if set(fidelity) != {"exact", "approximate", "unsupported", "invalid", "other"} or any(type(value) is not int or value < 0 for value in fidelity.values()):
            raise ValueError("malformed fidelity")
        for category in ("approximate", "unsupported", "invalid", "other"):
            wanted = baseline["fidelity"][category] + (len(subjects) if category == "unsupported" else 0)
            if fidelity[category] != wanted:
                raise ValueError("refusal fidelity drifted")
        if "protected-alias-canary-never-print" in raw.decode() or "unexpected-private-value" in raw.decode():
            raise ValueError("protected alias leaked")
    except (OSError, UnicodeError, ValueError, KeyError, TypeError):
        raise SystemExit("protected service-alias refusal contract rejected")


if __name__ == "__main__":
    if len(sys.argv) != 6 or sys.argv[1] not in ("inventory", "report-refusal"):
        raise SystemExit("invalid protected service-alias contract invocation")
    if sys.argv[1] == "inventory":
        inventory(sys.argv[2:])
    else:
        assert_refusal(sys.argv[2:])
