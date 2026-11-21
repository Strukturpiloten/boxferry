"""Supply exact Git preset inputs to the native Renovate contract tests."""

from __future__ import annotations

import json
import re
import tempfile
from functools import cache
from pathlib import Path
from typing import Never

from boxferry_ci.bootstrap import (
    REPOSITORY,
    BootstrapError,
    load_config,
    prepare_object_cache,
    run_git,
)

PRESETS = ("common", "rust")
MAX_PRESET_BYTES = 65536


class PresetSourceError(ValueError):
    """The contract cannot bind an inherited preset to the selected Git source."""


def _reject_json_constant(value: str) -> Never:
    raise ValueError(f"invalid JSON constant {value}")


def _preset_object(data: bytes, source: str) -> str:
    if len(data) > MAX_PRESET_BYTES:
        raise PresetSourceError(f"{source}: preset exceeds the byte limit")
    try:
        text = data.decode("utf-8")
        value = json.loads(text, parse_constant=_reject_json_constant)
    except ValueError as error:
        raise PresetSourceError(f"{source}: preset is not valid UTF-8 JSON") from error
    if not isinstance(value, dict):
        raise PresetSourceError(f"{source}: preset must be a JSON object")
    return text


@cache
def _preset_data(root: Path, revision: str) -> dict[str, str]:
    """Prefer local objects. For a shallow checkout, use the verified bootstrap fetch."""
    if run_git(root, "rev-parse", "--show-toplevel") != str(root):
        raise PresetSourceError("preset source must name its own Git checkout root")
    try:
        kind = run_git(root, "--no-lazy-fetch", "cat-file", "-t", revision)
    except BootstrapError:
        kind = None
    if kind is None:
        # Use an isolated cache so test acquisition cannot replace another validation's archive.
        with tempfile.TemporaryDirectory(prefix="boxferry-native-presets-") as directory:
            try:
                objects = prepare_object_cache(Path(directory), revision)
            except BootstrapError as error:
                raise PresetSourceError(
                    f"{REPOSITORY}@{revision}: cannot acquire pinned presets: {error}"
                ) from error
            return _read_blobs(objects, revision)
    if kind != "commit":
        raise PresetSourceError(f"{REPOSITORY}@{revision}: expected a Git commit")
    return _read_blobs(root, revision)


def _read_blobs(root: Path, revision: str) -> dict[str, str]:
    result = {}
    for name in PRESETS:
        path = f"renovate-presets/{name}.json"
        source = f"{REPOSITORY}@{revision}:{path}"
        entry = run_git(
            root, "--no-lazy-fetch", "ls-tree", "-l", "-z", revision, "--", path, binary=True
        )
        assert isinstance(entry, bytes)
        match = re.fullmatch(
            rb"100(?:644|755) blob ([a-f0-9]{40}) +([0-9]+)\t" + re.escape(path.encode()) + rb"\0",
            entry,
        )
        if not match:
            raise PresetSourceError(f"{source}: missing regular tracked preset")
        size = int(match[2])
        if size > MAX_PRESET_BYTES:
            raise PresetSourceError(f"{source}: preset exceeds the byte limit")
        data = run_git(root, "--no-lazy-fetch", "cat-file", "blob", match[1].decode(), binary=True)
        assert isinstance(data, bytes)
        if len(data) != size:
            raise PresetSourceError(f"{source}: preset blob size changed")
        result[name] = _preset_object(data, source)
    return result


def pinned_preset_input(root: Path, caller: Path | None = None) -> str:
    """Bind both native inputs to the caller manifest without changing its extends order."""
    caller = root if caller is None else caller
    revision = load_config(caller)["revision"]
    references = {
        f"github>{REPOSITORY}//renovate-presets/{name}#{revision}": name for name in PRESETS
    }
    config_path = caller / ".github/renovate.json"
    try:
        config = json.loads(config_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PresetSourceError(f"{config_path}: unreadable native Renovate config") from error
    extends = config.get("extends") if isinstance(config, dict) else None
    if (
        not isinstance(extends, list)
        or len(extends) != len(references)
        or not all(isinstance(reference, str) and reference in references for reference in extends)
        or len(set(extends)) != len(references)
    ):
        raise PresetSourceError(
            f"{config_path}: expected exactly the common and rust presets "
            f"from {REPOSITORY}@{revision}"
        )
    data = _preset_data(root.resolve(), revision)
    return json.dumps({reference: data[name] for reference, name in references.items()})
