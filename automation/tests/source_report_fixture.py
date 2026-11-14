"""Authored finite HTTP-boundary controls; never operational source or native receipts.

The CLI and metadata reader are real selected producer code. Only the external
HTTP response is controlled. Provenance: authored, MPL-2.0, redistributable;
no external implementation was copied and no operational version is invented.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import io
import json
import os
import sys
import urllib.request
from pathlib import Path
from typing import Any
from unittest.mock import patch

CASES = ("success", "unavailable", "malformed", "invalid-revision", "missing-report")
SECRET = "authored-source-report-canary"


def exercise(
    automation: Path,
    directory: Path,
    case: str,
    candidate: str,
    policy: str,
    automation_sha: str,
) -> dict[str, Any]:
    """Supervise actual CLI outcomes; expected failures remain failures in their reports."""
    sys.path.insert(0, str(automation / "automation/src"))
    from boxferry_ci import dependencies
    from boxferry_ci import devcontainer_validation as validation

    module = Path(validation.__file__).resolve()
    expected = (automation / "automation/src/boxferry_ci/devcontainer_validation.py").resolve()
    if module != expected:
        raise RuntimeError("fixture imported a different automation source")
    directory.mkdir(parents=True, exist_ok=False)
    output = directory / "devcontainer-sources.json"
    exports = directory / "github-output"
    calls: list[str] = []

    class Opener:
        def open(self, request: urllib.request.Request, *, timeout: int) -> io.BytesIO:
            url = request.full_url
            prefix = "https://api.github.com/repos/Strukturpiloten/"
            if not url.startswith(prefix) or timeout != 20 or len(calls) >= 12:
                raise AssertionError("fixture metadata boundary exceeded")
            suffix = url.removeprefix(prefix).split("/")
            if suffix[0] not in validation.SIBLINGS or suffix[1:] not in ([], ["commits", "main"]):
                raise AssertionError("fixture received an unrelated metadata request")
            calls.append(url)
            if case == "unavailable":
                raise OSError("controlled metadata unavailable " + SECRET)
            if case == "malformed":
                return io.BytesIO(b'{"default_branch":null}')
            if len(suffix) == 1:
                data = {"default_branch": "main"}
            else:
                data = {"sha": "not-a-revision" if case == "invalid-revision" else candidate}
            return io.BytesIO(json.dumps(data).encode())

    stderr = io.StringIO()
    argv = [
        "resolve",
        "--repository",
        "boxferry",
        "--candidate-sha",
        candidate,
        "--policy-sha",
        policy,
        "--automation-sha",
        automation_sha,
        "--automation",
        str(automation),
        "--output",
        str(output),
        "--github-output",
        str(exports),
    ]
    with (
        patch.object(dependencies.urllib.request, "build_opener", return_value=Opener()),
        patch.dict(os.environ, {"SOURCE_REPORT_TEST_TOKEN": SECRET}),
        contextlib.redirect_stderr(stderr),
    ):
        code = validation.main(argv)
    good = case in ("success", "missing-report")
    if code != (0 if good else 1):
        raise AssertionError("actual resolver CLI differed from the independently expected exit")
    report = json.loads(output.read_text())
    if good:
        validation.parse_sources(report, "boxferry", candidate)
        if len(calls) != 12 or "sources=" not in exports.read_text():
            raise AssertionError("successful real CLI did not export the complete held closure")
    elif (
        report.get("status") != "failure"
        or report.get("candidate_sha") != candidate
        or report.get("policy_sha") != policy
        or report.get("automation_sha") != automation_sha
        or exports.exists()
        or SECRET in output.read_text()
        or not report.get("next_action")
    ):
        raise AssertionError("failure lost its cause/bindings/privacy or exported acceptance")
    if len(output.read_bytes()) > validation.MAX_REPORT:
        raise AssertionError("actual report exceeded its bound")
    if case == "missing-report":
        output.unlink()  # Exact fixture-owned file: simulate loss before upload, never outside it.
    summary: dict[str, Any] = {
        "schema": 1,
        "kind": "diagnostic-fixture-not-native-acceptance",
        "case": case,
        "candidate_sha": candidate,
        "policy_sha": policy,
        "automation_sha": automation_sha,
        "actual_cli_exit": code,
        "metadata_requests": len(calls),
        "report_present": output.exists(),
        "exports_present": exports.exists(),
        "controlled_report_removal": case == "missing-report",
        "module_sha256": hashlib.sha256(module.read_bytes()).hexdigest(),
        "fixture_provenance": "authored MPL-2.0; only public HTTP responses are controlled",
    }
    (directory / "fixture.json").write_text(json.dumps(summary, sort_keys=True) + "\n")
    (directory / "resolver.stderr").write_text(stderr.getvalue())
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--automation", required=True, type=Path)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--case", required=True, choices=CASES)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--policy-sha", required=True)
    parser.add_argument("--automation-sha", required=True)
    parser.add_argument("--propagate-failure", action="store_true")
    args = parser.parse_args()
    result = exercise(
        args.automation.resolve(),
        args.directory,
        args.case,
        args.candidate_sha,
        args.policy_sha,
        args.automation_sha,
    )
    print(json.dumps(result, sort_keys=True))
    return int(result["actual_cli_exit"]) if args.propagate_failure else 0


if __name__ == "__main__":
    raise SystemExit(main())
