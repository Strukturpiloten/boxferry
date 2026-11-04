"""Trusted four-repository Dev Container feature updates, using the official CLI.

Candidate documents are data. Only the admitted generator and this helper execute;
all graph publication age decisions remain with required dependency eligibility.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Mapping
from email.message import Message
from pathlib import Path
from typing import IO, Any

from boxferry_ci import runner
from boxferry_ci.dependencies import fetch_metadata

REPOSITORIES = frozenset(
    f"Strukturpiloten/{name}"
    for name in ("boxferry", "compose-lens", "podman-lens", "quadlet-lens")
)
DECLARATION = ".devcontainer/devcontainer.json"
LOCK = ".devcontainer/devcontainer-lock.json"
BRANCH = "renovate-devcontainer/features"
MAINTENANCE = "renovate-devcontainer/lock-file-maintenance"
BOT = "strukturpiloten-release-plz[bot]"
MAX_BYTES = 2 * 1024 * 1024
Run = Callable[..., runner.RunResult]


class UpdateError(ValueError):
    """An unsupported input, failed generator or publication race."""


def _read(path: Path) -> bytes:
    if path.is_symlink() or not path.is_file():
        raise UpdateError(f"expected a regular file: {path.name}")
    with path.open("rb") as stream:
        value = stream.read(MAX_BYTES + 1)
    if len(value) > MAX_BYTES:
        raise UpdateError(f"document exceeds byte bound: {path.name}")
    return value


def _execute(
    argv: tuple[str, ...],
    cwd: Path,
    stage: str,
    env: Mapping[str, str],
    command: Run = runner.run,
    timeout: int = 300,
) -> str:
    result = command(argv, cwd=cwd, stage=stage, environment=env, timeout_seconds=timeout)
    if result.status != "success":
        raise UpdateError(
            f"{stage}: {result.diagnostic.cause}; command={result.argv}; "
            f"{result.stderr}; inspect the generated graph and retry the scheduled updater"
        )
    return result.stdout


def clean_environment(home: Path) -> dict[str, str]:
    """Do not inherit App tokens, Git config, Docker credentials or Node hooks."""
    return {
        "PATH": os.defpath,
        "HOME": str(home),
        "DOCKER_CONFIG": str(home / "docker"),
        "XDG_CONFIG_HOME": str(home / "config"),
        "LANG": "C.UTF-8",
        "CI": "true",
    }


def generate_lock(
    candidate: Path,
    automation: Path,
    *,
    command: Run = runner.run,
    validate: Callable[[Path], object] | None = None,
) -> bool:
    """Generate in private scratch, validate the complete graph, replace only lock.

    CLI upgrade ignores the previous lock and resolves all transitive Features.
    Neither candidate lifecycle hooks nor Feature installers run in this command.
    """
    if (candidate / ".devcontainer").is_symlink():
        raise UpdateError("candidate Dev Container directory must not be a symbolic link")
    declaration = _read(candidate / DECLARATION)
    document = json.loads(declaration)
    roots = document.get("features") if isinstance(document, dict) else None
    if (
        not isinstance(roots, dict)
        or not roots
        or len(roots) > 256
        or any(
            re.fullmatch(r"ghcr\.io/[a-z0-9_./-]+:[0-9]+\.[0-9]+\.[0-9]+", ref) is None
            or not isinstance(options, dict)
            for ref, options in roots.items()
        )
    ):
        raise UpdateError("generator accepts only bounded exact-version public GHCR Feature roots")
    original = _read(candidate / LOCK) if (candidate / LOCK).exists() else None
    cli = (
        automation.resolve()
        / "automation/devcontainer/node_modules/@devcontainers/cli/devcontainer.js"
    )
    if not cli.is_file():
        raise UpdateError(
            "trusted official CLI missing; run npm ci --ignore-scripts in automation/devcontainer"
        )
    node = os.environ.get("BOXFERRY_NODE", "")
    if not node or not Path(node).is_absolute() or not Path(node).is_file():
        raise UpdateError("BOXFERRY_NODE must identify the trusted setup-node executable")
    with tempfile.TemporaryDirectory(prefix="boxferry-feature-graph-") as temporary:
        scratch = Path(temporary)
        (scratch / ".devcontainer").mkdir()
        (scratch / DECLARATION).write_bytes(declaration)
        home = scratch / "home"
        home.mkdir(mode=0o700)
        env = clean_environment(home)
        # One bounded retry repairs transient public registry failures; never partial output.
        for attempt in range(2):
            try:
                _execute(
                    (node, str(cli), "upgrade", "--workspace-folder", str(scratch)),
                    scratch,
                    "official Feature graph generation",
                    env,
                    command,
                )
                break
            except UpdateError:
                (scratch / LOCK).unlink(missing_ok=True)
                if attempt:
                    raise
        generated = _read(scratch / LOCK)
        if _read(scratch / DECLARATION) != declaration:
            raise UpdateError("official generator unexpectedly changed feature declarations")
        allowed = {DECLARATION, LOCK}
        unexpected = [
            p
            for p in (scratch / ".devcontainer").rglob("*")
            if p.is_file() and p.relative_to(scratch).as_posix() not in allowed
        ]
        if unexpected:
            raise UpdateError("official generator produced unexpected files")
        if validate is None:
            from boxferry_ci.operational import validate_feature_graph

            validate_feature_graph(scratch, fetch_metadata)
        else:
            validate(scratch)
        if _read(candidate / DECLARATION) != declaration:
            raise UpdateError("candidate declarations changed during graph generation; retry")
        if original == generated:
            return False
        if (candidate / LOCK).is_symlink():
            raise UpdateError("candidate lock changed into a symbolic link")
        current = _read(candidate / LOCK) if (candidate / LOCK).exists() else None
        if current != original:
            raise UpdateError("candidate lock changed during generation; retry")
        with tempfile.NamedTemporaryFile(dir=candidate / ".devcontainer", delete=False) as output:
            replacement = Path(output.name)
            try:
                output.write(generated)
                output.flush()
                os.replace(replacement, candidate / LOCK)
            finally:
                replacement.unlink(missing_ok=True)
        return True


def limited_config(repository: str, automation: Path, python: Path) -> dict[str, Any]:
    """Ignore candidate Renovate configuration; admit only two feature owners."""
    if repository not in REPOSITORIES:
        raise UpdateError("repository is outside the four Dev Container owners")
    if not python.is_absolute() or not automation.is_absolute():
        raise UpdateError("trusted command paths must be absolute")
    # Reject shell metacharacters even though these are trusted infrastructure paths.
    for path in (str(python), str(automation)):
        if re.fullmatch(r"/[A-Za-z0-9_./-]+", path) is None:
            raise UpdateError("trusted command path has unsupported characters")
    node = os.environ.get("BOXFERRY_NODE", "")
    if re.fullmatch(r"/[A-Za-z0-9_./-]+", node) is None:
        raise UpdateError("trusted absolute Node executable required")
    helper = (
        f"{python} -I {automation}/automation/src/boxferry_ci/devcontainer_updates.py "
        f"generate --automation {automation} --node {node}"
    )
    return {
        "platform": "github",
        "repositories": [repository],
        "onboarding": False,
        "requireConfig": "ignored",
        "extends": [],
        "enabledManagers": ["devcontainer", "custom.regex"],
        "dependencyDashboard": False,
        "dependencyDashboardApproval": False,
        "automerge": True,
        "automergeType": "pr",
        "platformAutomerge": False,
        "minimumReleaseAge": "3 days",
        "minimumReleaseAgeBehaviour": "timestamp-required",
        "internalChecksFilter": "strict",
        "branchPrefix": "renovate-devcontainer/",
        "branchTopic": "features",
        "groupName": "Dev Container features",
        "groupSlug": "features",
        "prConcurrentLimit": 1,
        "prHourlyLimit": 1,
        "lockFileMaintenance": {"enabled": False},
        "rebaseWhen": "behind-base-branch",
        "gitAuthor": (
            "strukturpiloten-release-plz[bot] "
            "<317908760+strukturpiloten-release-plz[bot]@users.noreply.github.com>"
        ),
        "username": BOT,
        "allowedCommands": ["^" + re.escape(helper) + "$"],
        "allowCommandTemplating": False,
        "allowedEnv": [],
        "binarySource": "global",
        "postUpgradeTasks": {
            "commands": [helper],
            "fileFilters": [LOCK],
            "executionMode": "branch",
        },
        "customManagers": [
            {
                "customType": "regex",
                "managerFilePatterns": ["/^\\.devcontainer/devcontainer\\.json$/"],
                "matchStrings": [
                    r'"ghcr\.io/devcontainers/features/github-cli:[^"]+"\s*:\s*\{\s*"version"\s*:\s*"(?<currentValue>[0-9]+\.[0-9]+\.[0-9]+)"'
                ],
                "depNameTemplate": "cli/cli",
                "datasourceTemplate": "github-releases",
            }
        ],
        "packageRules": [
            {"matchManagers": ["devcontainer"], "enabled": False},
            {
                "matchManagers": ["devcontainer"],
                "matchDepTypes": ["feature"],
                "matchFileNames": [DECLARATION],
                "enabled": True,
                "pinDigests": False,
                # Required trusted graph guard owns missing-date exact-digest quarantine.
                "minimumReleaseAgeBehaviour": "timestamp-optional",
            },
        ],
    }


class GitHub:
    """Finite Git data requests for this single fixed feature branch only."""

    def __init__(self, repository: str, token: str) -> None:
        if repository not in REPOSITORIES or not token:
            raise UpdateError("allowlisted repository and scoped App token required")
        self.repository = repository
        self.token = token
        self.requests = 0

    def graphql(self, query: str, variables: dict[str, str]) -> dict[str, Any]:
        return self._request(
            "https://api.github.com/graphql", "POST", {"query": query, "variables": variables}
        )

    def request(self, path: str, method: str = "GET", body: object = None) -> dict[str, Any]:
        if path and not path.startswith("/"):
            raise UpdateError("feature updater API path refused")
        return self._request(f"https://api.github.com/repos/{self.repository}{path}", method, body)

    def _request(self, url: str, method: str, body: object) -> dict[str, Any]:
        self.requests += 1
        if self.requests > 60:
            raise UpdateError("feature updater API request bound/path refused")
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(
            url,
            data=data,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
                "Content-Type": "application/json",
            },
        )

        # Never forward the token to a redirect destination.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(
                self,
                req: urllib.request.Request,
                fp: IO[bytes],
                code: int,
                msg: str,
                headers: Message,
                newurl: str,
            ) -> None:
                raise UpdateError("GitHub API redirect refused")

        try:
            with urllib.request.build_opener(NoRedirect()).open(request, timeout=20) as response:
                value = response.read(MAX_BYTES + 1)
            if len(value) > MAX_BYTES:
                raise UpdateError("GitHub API response exceeds byte bound")
            decoded = json.loads(value)
            if isinstance(decoded, list):
                return {"items": decoded}
            if not isinstance(decoded, dict):
                raise UpdateError("GitHub returned an unsupported JSON response")
            return decoded
        except urllib.error.HTTPError as error:
            # Identify permission failures without exposing query credentials or response bodies.
            path = urllib.parse.urlsplit(url).path
            raise UpdateError(
                f"GitHub API {method} {path}: HTTP {error.code}; retry updater"
            ) from error

    def sha(self, branch: str) -> str:
        sha = self.request("/git/ref/heads/" + branch)["object"]["sha"]
        if not isinstance(sha, str) or re.fullmatch(r"[a-f0-9]{40}", sha) is None:
            raise UpdateError("GitHub returned an invalid commit identity")
        return sha

    def tree(self, commit: str) -> str:
        """Resolve the held commit to its Git tree object; the APIs use distinct IDs."""
        value = self.request("/git/commits/" + commit)
        tree = value.get("tree")
        sha = tree.get("sha") if isinstance(tree, dict) else None
        if (
            value.get("sha") != commit
            or not isinstance(sha, str)
            or re.fullmatch(r"[a-f0-9]{40}", sha) is None
        ):
            raise UpdateError(
                "GitHub returned an invalid source commit/tree identity; retry updater"
            )
        return sha

    def document(self, path: str, sha: str) -> bytes:
        value = self.request(f"/contents/{path}?ref={sha}")
        if value.get("type") != "file" or value.get("encoding") != "base64":
            raise UpdateError("feature document is not a regular GitHub file")
        content = base64.b64decode(value["content"])
        if len(content) > MAX_BYTES:
            raise UpdateError("feature document exceeds byte bound")
        return content


def _owned_head(api: GitHub, pull: dict[str, Any], base: str, branch: str) -> str:
    if (
        pull["user"]["login"] != BOT
        or pull["base"]["ref"] != "main"
        or pull["head"]["repo"]["full_name"] != api.repository
        or pull["head"].get("ref") != branch
    ):
        raise UpdateError("feature branch is not owned by the configured App")
    head = api.sha(branch)
    if pull["head"]["sha"] != head:
        raise UpdateError("feature PR head moved during read; retry")
    comparison = api.request(f"/compare/{base}...{head}")
    files = comparison.get("files", [])
    allowed = {LOCK} if branch == MAINTENANCE else {DECLARATION, LOCK}
    statuses = {"ahead", "diverged", "behind", "identical"} if branch == MAINTENANCE else {"ahead"}
    if comparison.get("status") not in statuses or any(
        item.get("filename") not in allowed or item.get("status") != "modified" for item in files
    ):
        raise UpdateError(
            "feature branch is stale or contains unowned changes; native Renovate must rebase"
        )
    commits = comparison.get("commits", [])
    if comparison.get("total_commits") != len(commits) or any(
        commit.get("committer") is None or commit["committer"]["login"] != BOT for commit in commits
    ):
        raise UpdateError("feature branch contains a differently owned or truncated commit history")
    # Even a merged/behind branch must still end at its recorded App-created commit.
    commit = api.request("/commits/" + head)
    committer = commit.get("committer")
    if not isinstance(committer, dict) or committer.get("login") != BOT:
        raise UpdateError("feature branch tip is not owned by the configured App")
    return head


def _queue_ready(api: GitHub, number: int, head: str, base: str, branch: str) -> bool:
    """Enqueue only the exact owned head after every required context succeeds.

    Ordinary branch rules and queue checks still decide integration. This helper
    cannot fabricate a check or override review/protection and never reports merge.
    """
    pull = api.request(f"/pulls/{number}")
    if (
        pull.get("state") != "open"
        or pull.get("draft")
        or _owned_head(api, pull, base, branch) != head
    ):
        return False
    rules = api.request("/rules/branches/main")["items"]
    if len(rules) >= 100 or sum(rule.get("type") == "merge_queue" for rule in rules) != 1:
        raise UpdateError("feature maintenance requires the existing protected merge queue")
    required = {"Required CI", "Dependency eligibility"}
    for rule in rules:
        if rule.get("type") == "required_status_checks":
            required.update(
                item["context"] for item in rule["parameters"]["required_status_checks"]
            )
    checks = api.request(f"/commits/{head}/check-runs?per_page=100")
    runs = checks.get("check_runs", [])
    if checks.get("total_count") != len(runs) or len(runs) >= 100:
        return False
    states = {
        check["name"]: check.get("status") == "completed" and check.get("conclusion") == "success"
        for check in runs
    }
    statuses = api.request(f"/commits/{head}/status?per_page=100")
    contexts = statuses.get("statuses", [])
    if statuses.get("total_count") != len(contexts) or len(contexts) >= 100:
        return False
    states.update({status["context"]: status.get("state") == "success" for status in contexts})
    if any(not states.get(name, False) for name in required):
        return False
    if api.sha("main") != base or api.sha(branch) != head:
        raise UpdateError("feature base/head moved before queue admission; retry")
    query = (
        "mutation($id:ID!,$head:GitObjectID!){enqueuePullRequest(input:{"
        "pullRequestId:$id,expectedHeadOid:$head}){mergeQueueEntry{id}}}"
    )
    response = api.graphql(query, {"id": pull["node_id"], "head": head})
    if response.get("errors") or not response.get("data", {}).get("enqueuePullRequest", {}).get(
        "mergeQueueEntry", {}
    ).get("id"):
        raise UpdateError(
            "protected exact-head queue admission refused; "
            "inspect required checks/reviews and retry"
        )
    observed = api.graphql(
        "query($id:ID!){node(id:$id){... on PullRequest{headRefOid mergeQueueEntry{id}}}}",
        {"id": pull["node_id"]},
    )
    node = observed.get("data", {}).get("node", {})
    if observed.get("errors") or node.get("headRefOid") != head or not node.get("mergeQueueEntry"):
        raise UpdateError("queue admission readback does not identify the held feature head")
    return True


def maintenance(
    api: GitHub, automation: Path, generator: Callable[[Path, Path], bool] = generate_lock
) -> bool:
    """Refresh exactly one Feature stream and enqueue only protected exact heads.

    Native version updates take precedence. Own stale maintenance branches receive
    a non-force merge-parent commit whose tree is current main plus generated lock.
    Closed branches are reused only with their recorded PR and App-owned history.
    """
    metadata = api.request("")
    if metadata.get("default_branch") != "main":
        raise UpdateError("only the real main default branch is admitted")
    base = api.sha("main")
    owner = api.repository.split("/")[0]
    versioned = api.request("/pulls?state=open&head=" + owner + ":" + BRANCH + "&per_page=2")[
        "items"
    ]
    history = api.request(
        "/pulls?state=all&head="
        + owner
        + ":"
        + MAINTENANCE
        + "&per_page=2&sort=created&direction=desc"
    )["items"]
    active_maintenance = [pull for pull in history if pull.get("state") == "open"]
    if len(versioned) > 1 or len(active_maintenance) > 1:
        raise UpdateError("multiple feature PRs found")
    branch = BRANCH if versioned else MAINTENANCE
    pull = versioned[0] if versioned else history[0] if history else None
    head = _owned_head(api, pull, base, branch) if pull else base
    source = head if versioned else base
    with tempfile.TemporaryDirectory(prefix="boxferry-feature-maintenance-") as temporary:
        candidate = Path(temporary)
        (candidate / ".devcontainer").mkdir()
        for path in (DECLARATION, LOCK):
            (candidate / path).write_bytes(api.document(path, source))
        changed = generator(candidate, automation)
        lock = _read(candidate / LOCK).decode()
    if api.sha("main") != base or (pull and api.sha(branch) != head):
        raise UpdateError("feature base/head moved during generation; retry updater")
    if versioned and active_maintenance:
        stale = active_maintenance[0]
        stale_head = _owned_head(api, stale, base, MAINTENANCE)
        if api.sha(MAINTENANCE) != stale_head or api.sha(BRANCH) != head:
            raise UpdateError("overlapping feature branch moved; retry")
        api.request(f"/pulls/{stale['number']}", "PATCH", {"state": "closed"})
    if not changed:
        if pull and pull.get("state") == "open":
            if branch == MAINTENANCE:
                # Exact current main already carries the official graph.
                api.request(f"/pulls/{pull['number']}", "PATCH", {"state": "closed"})
            else:
                _queue_ready(api, pull["number"], head, base, branch)
        return False
    if (
        pull
        and pull.get("state") == "open"
        and branch == MAINTENANCE
        and api.document(LOCK, head).decode() == lock
        and api.request(f"/compare/{base}...{head}").get("status") == "ahead"
    ):
        # The active proposal already has this entire graph on the current base.
        _queue_ready(api, pull["number"], head, base, branch)
        return False
    source_tree = api.tree(source)
    tree = api.request(
        "/git/trees",
        "POST",
        {
            "base_tree": source_tree,
            "tree": [{"path": LOCK, "mode": "100644", "type": "blob", "content": lock}],
        },
    )["sha"]
    parents = [head] if head == source else [head, source]
    commit = api.request(
        "/git/commits",
        "POST",
        {"message": "chore(deps): update Dev Container features", "tree": tree, "parents": parents},
    )["sha"]
    if api.sha("main") != base or (pull and api.sha(branch) != head):
        raise UpdateError("feature base/head moved before publication; retry updater")
    if pull:
        api.request("/git/refs/heads/" + branch, "PATCH", {"sha": commit, "force": False})
    else:
        # Existing unknown refs fail creation; no force update or deletion is allowed.
        api.request("/git/refs", "POST", {"ref": "refs/heads/" + branch, "sha": commit})
    if pull is None or pull.get("state") != "open":
        api.request(
            "/pulls",
            "POST",
            {
                "title": "chore(deps): update Dev Container features",
                "head": branch,
                "base": "main",
                "body": (
                    "Official Dev Container CLI whole-graph refresh. Required native container "
                    "validation and 72-hour dependency eligibility must succeed before merge."
                ),
            },
        )
    # Fresh App-created commits trigger ordinary CI; a later scheduled pass enqueues.
    return True


def service(repository: str, automation: Path, runtime: Path, *, command: Run = runner.run) -> None:
    """Run the one trusted Renovate configuration, then bounded maintenance."""
    if (
        repository not in REPOSITORIES
        or os.environ.get("GITHUB_REF") != "refs/heads/main"
        or os.environ.get("GITHUB_EVENT_NAME") not in {"schedule", "workflow_dispatch"}
    ):
        raise UpdateError(
            "feature service requires an allowlisted default-branch schedule/manual event"
        )
    token = os.environ.get("RENOVATE_TOKEN", "")
    if not token:
        raise UpdateError("scoped App token unavailable")
    expected = json.loads(_read(automation / "automation/renovate-runtime.json"))["image"]
    if _read(runtime / "source-image.txt").decode().strip() != expected:
        raise UpdateError("Renovate runtime differs from the canonical integrity pin")
    version = expected.split(":", 1)[1].split("@", 1)[0]
    if json.loads(_read(runtime / "package.json"))["version"] != version:
        raise UpdateError("Renovate package version differs from its image")
    api = GitHub(repository, token)
    if api.request("").get("default_branch") != "main":
        raise UpdateError("the feature service requires the real main default branch")
    node = os.environ.get("BOXFERRY_NODE", "")
    config = limited_config(repository, automation, Path(sys.executable))
    with tempfile.TemporaryDirectory(prefix="boxferry-limited-renovate-") as temporary:
        owned = Path(temporary)
        config["baseDir"] = str(owned / "base")
        config["cacheDir"] = str(owned / "cache")
        config_file = owned / "config.json"
        config_file.write_text(json.dumps(config))
        env = clean_environment(owned / "home")
        Path(env["HOME"]).mkdir(mode=0o700)
        env.update(
            {
                "RENOVATE_TOKEN": token,
                "RENOVATE_CONFIG_FILE": str(config_file),
                "PATH": str(Path(node).parent) + os.pathsep + os.defpath,
            }
        )
        _execute(
            (node, str(runtime / "dist/renovate.js")),
            automation,
            "limited native Renovate feature update",
            env,
            command,
            timeout=900,
        )
    maintenance(api, automation)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("generate", "config", "maintenance", "service"))
    parser.add_argument("--automation", type=Path, required=True)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--runtime", type=Path)
    options = parser.parse_args()
    try:
        if options.node is not None:
            os.environ["BOXFERRY_NODE"] = str(options.node)
        if options.mode == "generate":
            generate_lock(Path.cwd(), options.automation)
        elif options.mode == "config":
            if options.output is None:
                raise UpdateError("config needs an owned output path")
            options.output.write_text(
                json.dumps(
                    limited_config(options.repository, options.automation, Path(sys.executable)),
                    indent=2,
                )
                + "\n"
            )
        elif options.mode == "service":
            if options.runtime is None:
                raise UpdateError("service requires the verified runtime directory")
            service(options.repository, options.automation, options.runtime)
        else:
            maintenance(
                GitHub(options.repository, os.environ.get("RENOVATE_TOKEN", "")), options.automation
            )
    except (TypeError, AttributeError, KeyError) as error:
        print(
            f"Dev Container updater: malformed API/input evidence ({type(error).__name__}); "
            "inspect the failed stage and retry",
            file=sys.stderr,
        )
        return 1
    except (UpdateError, OSError, ValueError) as error:
        print(f"Dev Container updater: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
