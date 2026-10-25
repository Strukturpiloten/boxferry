#!/usr/bin/env python3
"""Real-context Box admission plus explicit literal DockerLens dependency admission."""

from __future__ import annotations

import argparse
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import types
from urllib.parse import quote


SPEC = importlib.util.spec_from_file_location(
    "reviewed_docker_authorization", Path(__file__).with_name("reviewed-docker-consumer-authorization.py"))
assert SPEC is not None and SPEC.loader is not None
auth = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(auth)
BOX = "Strukturpiloten/boxferry"
LENS = "Strukturpiloten/docker-lens"
PHASES = frozenset({
    "authorization", "workflow-pin", "helper-load", "context/Box-admission",
    "Box-permission", "Box-pull", "Lens-permission", "Lens-pull",
})
# Exact value-free AdmissionError constants in the independently pinned helper.
# Do not stringify unknown errors or infer HTTP status: fetch_json discards it.
HELPER_REASONS = frozenset({
    "invalid-api-path", "api-unavailable", "api-response-too-large", "api-invalid-response",
    "invalid-repository-policy", "untrusted-dispatch-ref", "invalid-input", "unknown-actor",
    "insufficient-actor-permission", "invalid-pull-response", "pull-head-mismatch",
})


class ClosedArgumentParser(argparse.ArgumentParser):
    def error(self, message):
        # argparse's normal error contains caller-supplied argument text.
        raise ValueError("rejected") from None


def failure_message(phase, error, helper=None) -> str:
    """Only fixed labels and explicitly allowed pinned-helper reasons reach logs."""
    if type(phase) is not str or phase not in PHASES:
        phase = "authorization"
    reason = "rejected"
    if (type(error) is getattr(helper, "AdmissionError", None)
            and type(error.args) is tuple and len(error.args) == 1
            and type(error.args[0]) is str and error.args[0] in HELPER_REASONS):
        reason = error.args[0]
    return f"reviewed Docker consumer admission denied: phase={phase} reason={reason}"


def shared_helper(root: Path, revision: str):
    auth.sha(revision, auth.SHA)
    auth.require(root.is_absolute() and root.resolve(strict=True) == root)
    auth.require(auth.git(root, "rev-parse", "--show-toplevel") == str(root)
                 and auth.git(root, "rev-parse", "HEAD") == revision
                 and auth.git(root, "status", "--porcelain", "--untracked-files=all") == "")
    path = root / "scripts/native-dispatch-admission.py"
    # Compile held bytes of the independently pinned helper, not a candidate path.
    raw = auth.read(path, 128 * 1024)
    module = types.ModuleType("pinned_reviewed_native_admission")
    module.__file__ = str(path)
    exec(compile(raw, str(path), "exec"), module.__dict__)
    auth.require(module.REPOSITORY == LENS and module.BOXFERRY_REPOSITORY == BOX)
    return module


def admit_pair(env, pair, helper, get, *, phase=None) -> None:
    """Never replace GITHUB_REPOSITORY, IDs, ref, event or either real actor."""
    def mark(name):
        if phase is not None:
            phase(name)

    mark("context/Box-admission")
    auth.keys(pair, ("boxferry", "docker_lens"))
    auth.entry(pair["boxferry"], BOX)
    auth.entry(pair["docker_lens"], LENS)
    box, lens = pair["boxferry"], pair["docker_lens"]
    auth.require(env.get("GITHUB_EVENT_NAME") == "workflow_dispatch"
                 and env.get("GITHUB_REF") == "refs/heads/main"
                 and env.get("GITHUB_REPOSITORY") == BOX
                 and env.get("GITHUB_REPOSITORY_ID") == str(box["repository_id"]))
    auth.require(box["repository"] == BOX and lens["repository"] == LENS)
    auth.require(helper.PR_RE.fullmatch(str(lens["pr"])) is not None
                 and helper.SHA_RE.fullmatch(lens["revision"]) is not None
                 and helper.ID_RE.fullmatch(str(lens["repository_id"])) is not None)
    actors = (env.get("GITHUB_ACTOR", ""), env.get("GITHUB_TRIGGERING_ACTOR", ""))
    auth.require(all(helper.ACTOR_RE.fullmatch(actor) is not None for actor in actors))
    box_env = {**env, "PR_NUMBER": str(box["pr"]), "EXPECTED_SHA": box["revision"],
               "EXPECTED_REPOSITORY": BOX}
    box_permissions = {f"/repos/{BOX}/collaborators/{quote(actor, safe='')}/permission" for actor in actors}
    box_pull = f"/repos/{BOX}/pulls/{box['pr']}"

    def box_get(path):
        # Label only known literal request classes; fetching remains wholly canonical.
        if path in box_permissions:
            mark("Box-permission")
        elif path == box_pull:
            mark("Box-pull")
        else:
            mark("context/Box-admission")
        return get(path, BOX)

    helper.admit(box_env, box_get, BOX)
    mark("Lens-permission")
    for actor in set(actors):
        permission = get(f"/repos/{LENS}/collaborators/{quote(actor, safe='')}/permission", LENS)
        auth.require(type(permission) is dict and type(permission.get("permission")) is str
                     and permission["permission"] in helper.ALLOWED_ROLES)
    mark("Lens-pull")
    pull = get(f"/repos/{LENS}/pulls/{lens['pr']}", LENS)
    auth.require(type(pull) is dict and type(pull.get("number")) is int
                 and pull["number"] == lens["pr"] and pull.get("state") == "open")
    head, base = pull.get("head"), pull.get("base")
    auth.require(type(head) is dict and type(base) is dict
                 and base.get("ref") == "main" and head.get("sha") == lens["revision"])
    for repository in (head.get("repo"), base.get("repo")):
        auth.require(type(repository) is dict and type(repository.get("id")) is int
                     and repository["id"] == lens["repository_id"] and repository.get("full_name") == LENS)


def main():
    phase = "authorization"
    helper = None

    def mark(name):
        nonlocal phase
        phase = name

    try:
        parser = ClosedArgumentParser(description=__doc__)
        parser.add_argument("--trusted-root", type=Path, required=True)
        parser.add_argument("--admission-root", type=Path, required=True)
        parser.add_argument("--admission-revision", required=True)
        parser.add_argument("--box-pr", type=int, required=True)
        parser.add_argument("--box-sha", required=True)
        parser.add_argument("--lens-pr", type=int, required=True)
        parser.add_argument("--lens-sha", required=True)
        args = parser.parse_args()
        pair, _ = auth.authorize(args.trusted_root, args.box_pr, args.box_sha, args.lens_pr, args.lens_sha)
        mark("workflow-pin")
        workflow = auth.read(args.trusted_root / ".github/workflows/reviewed-docker-consumer-validation.yml").decode()
        pins = re.findall(r"(?m)^  ADMISSION_REVISION: ([0-9a-f]{40})$", workflow)
        auth.require(pins == [args.admission_revision])
        mark("helper-load")
        helper = shared_helper(args.admission_root, args.admission_revision)
        mark("context/Box-admission")
        token = os.environ.get("GITHUB_TOKEN", "")
        auth.require(bool(token))
        admit_pair(os.environ, pair, helper,
                   lambda path, repository: helper.fetch_json(path, token, expected_repository=repository),
                   phase=mark)
    except Exception as error:
        print(failure_message(phase, error, helper), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
