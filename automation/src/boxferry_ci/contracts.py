"""Strict JSON contracts used at the untrusted workflow boundary."""

from __future__ import annotations

import re
from collections.abc import Mapping
from pathlib import PurePosixPath

type Json = bool | int | float | str | list[Json] | dict[str, Json] | None


class ContractError(ValueError):
    """An input cannot safely satisfy the validation contract."""


def object_value(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, Mapping) or any(not isinstance(key, str) for key in value):
        raise ContractError(f"{label} must be an object with string keys")
    return dict(value)


def strings(value: object, label: str, *, nonempty: bool = True) -> tuple[str, ...]:
    if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
        raise ContractError(f"{label} must be an array of nonempty strings")
    if nonempty and not value:
        raise ContractError(f"{label} may not be empty")
    if len(set(value)) != len(value):
        raise ContractError(f"{label} may not contain duplicates")
    return tuple(value)


def exact_sha(value: object, label: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise ContractError(f"{label} must be an exact lowercase 40-character Git SHA")
    return value


def relative_path(value: object, label: str, *, pattern: bool = False) -> str:
    if not isinstance(value, str) or not value or "\x00" in value or "\\" in value:
        raise ContractError(f"{label} must be a repository-relative POSIX path")
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or "." == value:
        raise ContractError(f"{label} escapes the repository")
    if not pattern and any(c in value for c in "*?["):
        raise ContractError(f"{label} is a path, not a pattern")
    return value
