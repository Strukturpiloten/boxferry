#!/usr/bin/env python3
"""Forgejo authored-field offline prerequisite; never execution or native admission."""

from __future__ import annotations
import argparse
import importlib.util
import json
import pathlib
import sys
from typing import Any

SPEC = importlib.util.spec_from_file_location(
    "docker_application_schedule", pathlib.Path(__file__).with_name("docker-application-schedule.py"))
assert SPEC is not None and SPEC.loader is not None
schedule = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(schedule)
topology = schedule.topology


def private_string(value: Any) -> bool:
    """Validate independently supplied interpolation without exposing its contents."""
    if not isinstance(value, str) or not value or "\x00" in value:
        return False
    try:
        return len(value.encode("utf-8")) <= 4096
    except UnicodeError:
        return False


def environment(body: dict[str, Any], expected: dict[str, str]) -> None:
    """Require authored assignments, without assessing extra image/environment intent."""
    values = body.get("Env")
    topology.require(isinstance(values, list), "authored environment assignments missing")
    actual: dict[str, str] = {}
    for assignment in values:
        # The topology validator has already checked list bounds and string/NUL types.
        topology.require(isinstance(assignment, str) and "=" in assignment,
                         "native environment assignment malformed")
        name, value = assignment.split("=", 1)
        topology.require(bool(name) and name not in actual, "native environment name missing or duplicated")
        try:
            assignment.encode("utf-8")
        except UnicodeError:
            raise topology.ExpectationError("native environment assignment malformed") from None
        actual[name] = value
    topology.require(all(name in actual and actual[name] == value for name, value in expected.items()),
                     "authored environment assignment missing or differs")


def validate_authored_fields(
    plan_bytes: bytes, admission_bytes: bytes, sidecar_bytes: bytes | None, interpolation_bytes: bytes, *,
    catalogue_bytes: bytes, application: str, lane: str, profile: dict[str, Any],
    image_aliases: dict[str, str], prefix: str, run_id: str, fixture_root: str,
) -> dict[str, Any]:
    """Pure check of independently authored values, never inferred from the artifact.

    Callers must separately check canonical source bytes with topology.check_sources.
    Catalogue, profile, image aliases and interpolation are caller trust inputs, not
    authority established by this result. Image defaults and extra well-formed Env
    assignments remain unassessed. No protected values or their input digest are returned.
    """
    result = schedule.validate_schedule(plan_bytes, admission_bytes, sidecar_bytes,
        catalogue_bytes=catalogue_bytes, application=application, lane=lane, profile=profile,
        image_aliases=image_aliases, prefix=prefix, run_id=run_id, fixture_root=fixture_root)
    expectations = topology.document(interpolation_bytes)
    topology.fields(expectations, {"schema_version", "kind", "context", "interpolation"},
                    "interpolation expectation fields differ")
    topology.require(type(expectations["schema_version"]) is int and expectations["schema_version"] == 1
                     and expectations["kind"] == "boxferry-docker-forgejo-interpolation-expectations",
                     "interpolation expectation schema differs")
    context = {"application": application, "lane": lane, "profile": profile, "image_aliases": image_aliases,
               "prefix": prefix, "run_id": run_id, "fixture_root": fixture_root}
    topology.require(topology.same(expectations["context"], context), "interpolation expectation context differs")
    protected = expectations["interpolation"]
    topology.fields(protected, {"BF_DB_PASSWORD", "BF_FORGEJO_SECRET_KEY"}, "interpolation value names differ")
    topology.require(all(private_string(value) for value in protected.values()), "interpolation value malformed or exceeds bound")

    # These literal assertions come from the authored Compose fixture, not a renderer,
    # reacquisition, or values discovered in the native artifact being checked.
    expected_db = {"POSTGRES_DB": "forgejo", "POSTGRES_USER": "forgejo",
                   "POSTGRES_PASSWORD": protected["BF_DB_PASSWORD"]}
    expected_app = {
        "FORGEJO__database__DB_TYPE": "postgres", "FORGEJO__database__HOST": "db:5432",
        "FORGEJO__database__NAME": "forgejo", "FORGEJO__database__USER": "forgejo",
        "FORGEJO__database__PASSWD": protected["BF_DB_PASSWORD"], "FORGEJO__database__SSL_MODE": "disable",
        "FORGEJO__security__INSTALL_LOCK": "true", "FORGEJO__security__SECRET_KEY": protected["BF_FORGEJO_SECRET_KEY"],
        "FORGEJO__service__DISABLE_REGISTRATION": "true", "FORGEJO__server__DOMAIN": "127.0.0.1",
        "FORGEJO__server__ROOT_URL": "http://127.0.0.1:13000/", "FORGEJO__server__SSH_DOMAIN": "127.0.0.1",
        "FORGEJO__server__SSH_PORT": "12222", "FORGEJO__server__SSH_LISTEN_PORT": "2222",
        "FORGEJO__server__START_SSH_SERVER": "true",
    }
    plan = topology.document(plan_bytes)
    containers = {request["path"].split("?name=", 1)[1]: request["body"] for request in plan["requests"]
                  if request["path"].split("/", 2)[2].startswith("containers/create?name=")}
    db, app = containers[f"{prefix}-forge-db"], containers[f"{prefix}-forge-app"]
    environment(db, expected_db)
    environment(app, expected_app)
    health = db.get("Healthcheck")
    topology.require(isinstance(health, dict), "authored database healthcheck missing")
    topology.require(topology.same(health.get("Test"), ["CMD-SHELL", "pg_isready -U forgejo -d forgejo"]),
                     "authored database health command differs")
    topology.require(all(type(health.get(key)) is int and health[key] == value for key, value in
                         {"Interval": 2_000_000_000, "Timeout": 5_000_000_000, "Retries": 60}.items()),
                     "authored database health settings missing or differ")
    topology.require(app.get("User") == "1000:1000", "authored Forgejo user missing or differs")
    return {**result, "kind": "boxferry-docker-forgejo-authored-fields",
            "source_kind": "authored-compose",
            "authored_checks": ["db.environment", "forgejo.environment", "db.healthcheck", "forgejo.user"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[2])
    for name in ("plan", "admission", "sidecar", "profile", "image-aliases", "interpolation-expectations"):
        parser.add_argument(f"--{name}", type=pathlib.Path, required=True)
    for name in ("lane", "prefix", "run-id", "fixture-root"):
        parser.add_argument(f"--{name}", required=True)
    args = parser.parse_args()
    try:
        catalogue_bytes = schedule.read_document(args.repository / topology.CATALOGUE_PATH)
        topology.check_sources(args.repository, catalogue_bytes)
        result = validate_authored_fields(schedule.read_document(args.plan), schedule.read_document(args.admission),
            schedule.read_document(args.sidecar), schedule.read_document(args.interpolation_expectations),
            catalogue_bytes=catalogue_bytes, application="forgejo", lane=args.lane,
            profile=topology.document(schedule.read_document(args.profile)),
            image_aliases=topology.document(schedule.read_document(args.image_aliases)),
            prefix=args.prefix, run_id=args.run_id, fixture_root=args.fixture_root)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (topology.ExpectationError, OSError, UnicodeError, ValueError, TypeError, KeyError):
        # Never print input paths, parse exceptions, or private interpolation/artifact values.
        print("offline Forgejo authored-field prerequisite rejected: check source, context and authored fields", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
