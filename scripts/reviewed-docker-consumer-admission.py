#!/usr/bin/env python3
"""Real-context Box admission plus explicit literal DockerLens dependency admission."""

from __future__ import annotations

import argparse
import importlib.util
import os
from pathlib import Path
import subprocess
import types
from urllib.parse import quote


SPEC = importlib.util.spec_from_file_location(
    "reviewed_docker_authorization", Path(__file__).with_name("reviewed-docker-consumer-authorization.py"))
assert SPEC is not None and SPEC.loader is not None
auth = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(auth)
BOX = "Strukturpiloten/boxferry"
LENS = "Strukturpiloten/docker-lens"


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


def admit_pair(env, pair, helper, get) -> None:
    """Never replace GITHUB_REPOSITORY, IDs, ref, event or either real actor."""
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
    helper.admit(box_env, lambda path: get(path, BOX), BOX)
    for actor in set(actors):
        permission = get(f"/repos/{LENS}/collaborators/{quote(actor, safe='')}/permission", LENS)
        auth.require(type(permission) is dict and type(permission.get("permission")) is str
                     and permission["permission"] in helper.ALLOWED_ROLES)
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trusted-root", type=Path, required=True)
    parser.add_argument("--admission-root", type=Path, required=True)
    parser.add_argument("--admission-revision", required=True)
    parser.add_argument("--box-pr", type=int, required=True)
    parser.add_argument("--box-sha", required=True)
    parser.add_argument("--lens-pr", type=int, required=True)
    parser.add_argument("--lens-sha", required=True)
    args = parser.parse_args()
    pair, _ = auth.authorize(args.trusted_root, args.box_pr, args.box_sha, args.lens_pr, args.lens_sha)
    workflow = auth.read(args.trusted_root / ".github/workflows/reviewed-docker-consumer-validation.yml").decode()
    import re
    pins = re.findall(r"(?m)^  ADMISSION_REVISION: ([0-9a-f]{40})$", workflow)
    auth.require(pins == [args.admission_revision])
    helper = shared_helper(args.admission_root, args.admission_revision)
    token = os.environ.get("GITHUB_TOKEN", "")
    auth.require(bool(token))
    admit_pair(os.environ, pair, helper,
               lambda path, repository: helper.fetch_json(path, token, expected_repository=repository))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Pinned helper errors are deliberately closed; never echo API data/token.
        raise SystemExit("reviewed Docker consumer admission denied") from None
