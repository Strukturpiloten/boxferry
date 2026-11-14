"""Offline exact-source descriptor checks for the separately authorized manual probe."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
PACKAGE = tuple(
    "automation/src/boxferry_ci/" + name + ".py"
    for name in (
        "__init__",
        "provider_probe",
        "provider_probe_source",
        "provider_probe_controller",
        "provider_probe_observer",
    )
)
WORKFLOWS = tuple(
    ".github/workflows/" + name + ".yml"
    for name in (
        "recovery-cancellation-provider-probe",
        "recovery-cancellation-probe-prepare",
        "recovery-cancellation-probe-effect",
    )
)


def require(value: object, message: str) -> None:
    if not value:
        raise ValueError(message)


def git(*arguments: str) -> bytes:
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(
        GIT_NO_LAZY_FETCH="1",
        GIT_NO_REPLACE_OBJECTS="1",
        GIT_TERMINAL_PROMPT="0",
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
        GIT_ALLOW_PROTOCOL="",
        GIT_OPTIONAL_LOCKS="0",
    )
    result = subprocess.run(
        [
            "git",
            "--no-replace-objects",
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
            "-C",
            str(ROOT),
            *arguments,
        ],
        env=environment,
        capture_output=True,
        timeout=5,
        check=False,
    )
    require(
        result.returncode == 0 and len(result.stdout) <= 512 * 1024,
        "held source objects unavailable; implicit acquisition is forbidden",
    )
    return result.stdout


def held(source: str, path: str) -> tuple[bytes, dict[str, str]]:
    record = git("ls-tree", source, "--", path).decode().strip().split()
    require(
        len(record) == 4 and record[0:2] == ["100644", "blob"] and record[3] == path,
        "held source mode/path mismatch",
    )
    size = git("cat-file", "-s", source + ":" + path).decode().strip()
    require(size.isdecimal() and int(size) <= 512 * 1024, "held source blob exceeds byte bound")
    data = git("cat-file", "blob", source + ":" + path)
    candidate = ROOT / path
    require(
        all(not part.is_symlink() for part in (candidate, *candidate.parents)),
        "package/source path contains a symlink",
    )
    metadata = candidate.stat()
    require(
        stat.S_ISREG(metadata.st_mode)
        and metadata.st_mode & 0o111 == 0
        and metadata.st_size == len(data) <= 512 * 1024,
        "package/source mode or size mismatch",
    )
    require(candidate.read_bytes() == data, "on-disk source differs from admitted Git bytes")
    return data, {"mode": "100644", "blob": record[2], "sha256": hashlib.sha256(data).hexdigest()}


def describe(source: str, main_ci_run: int) -> dict[str, Any]:
    require(re.fullmatch(r"[a-f0-9]{40}", source), "exact source revision required")
    require(type(main_ci_run) is int and 0 < main_ci_run < 10**20, "exact main CI run required")
    require(
        git("rev-parse", "HEAD").decode().strip() == source, "checkout must hold exact admitted S"
    )
    implementation = {path: held(source, path)[1] for path in PACKAGE}
    workflows = {}
    uploaders = set()
    for path in WORKFLOWS:
        data, binding = held(source, path)
        workflows[path] = binding
        matches = re.findall(
            r"uses: (actions/upload-artifact@[a-f0-9]{40}) +# +(v[0-9]+\.[0-9]+\.[0-9]+)",
            data.decode(),
        )
        if path != WORKFLOWS[0]:
            require(len(matches) == 1, "exact source uploader/tag unavailable")
            uploaders.update(matches)
        else:
            for callee in WORKFLOWS[1:]:
                require(("uses: ./" + callee) in data.decode(), "local same-source callee mismatch")
    require(len(uploaders) == 1, "source uploader/tag contradiction")
    uploader, tag = uploaders.pop()
    return {
        "source_sha": source,
        "repository": "Strukturpiloten/boxferry",
        "main_ci_run": main_ci_run,
        "implementation": implementation,
        "workflows": workflows,
        "uploader": uploader,
        "uploader_tag": tag,
    }


def verify_descriptor(value: dict[str, Any], raw: bytes, expected: str) -> None:
    require(
        re.fullmatch(r"[a-f0-9]{64}", expected) and hashlib.sha256(raw).hexdigest() == expected,
        "reviewed descriptor digest mismatch",
    )
    require(
        set(value)
        == {
            "source_sha",
            "repository",
            "main_ci_run",
            "implementation",
            "workflows",
            "uploader",
            "uploader_tag",
        },
        "source descriptor keys mismatch",
    )
    require(
        value == describe(value["source_sha"], value["main_ci_run"]),
        "reviewed descriptor differs from held implementation/workflow closure",
    )


def encoded(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
