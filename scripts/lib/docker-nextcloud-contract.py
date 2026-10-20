#!/usr/bin/env python3
"""Pure comparison of six-role Nextcloud explicit authored override predicates.

Input bytes require prior caller custody admission. None means ONLY equality of
cache/init/cron Cmd, init User, and db/cache/app Healthcheck.Test plus the authored
Interval/Timeout/Retries. It proves neither complete native semantics, image
inheritance/defaults, readiness, lifecycle, neutral conversion nor admission.
All Entrypoint fields, db/app/frontend Cmd, other User fields, optional health
defaults (including StartPeriod/StartInterval), and all other fields are unchecked
except for bounded JSON validity. No fields are filled, merged or normalized.

Each raw inspect body is at most 262144 bytes. Native JSON budgets per body are
depth16,4096 nodes including keys,256 members/collection,65536 aggregate UTF-8
string bytes and256-bit integers. Unrelated finite floats are permitted. The
private redis password is nonempty valid Unicode, at most4096 UTF-8 bytes.
No files, callbacks, transport, runtime, receipts or console I/O belong here.
"""
from __future__ import annotations

import json
import math
import re

__all__ = ("NextcloudContractError", "validate_authored_overrides")
_ROLES = ("db", "cache", "app", "init", "cron", "frontend")
_CODES = frozenset({"invalid-input", "invalid-json", "budget-exceeded", "role-membership",
                    "container-identity", "authored-command", "authored-user", "authored-health"})


class NextcloudContractError(ValueError):
    """A closed value-free code, without retained private exception chaining."""

    def __init__(self, code="invalid-input"):
        self.code = code if type(code) is str and code in _CODES else "invalid-input"
        super().__init__(self.code)


def _require(condition, code):
    if not condition:
        raise NextcloudContractError(code)


def _string_bytes(value, limit, code):
    _require(type(value) is str and len(value) <= limit
             and not any(0xD800 <= ord(char) <= 0xDFFF for char in value), code)
    size = len(value.encode("utf-8"))
    _require(size <= limit, code)
    return size


def _budget(root):
    pending = [(root, 0)]
    nodes, string_bytes = 0, 0
    while pending:
        value, depth = pending.pop()
        nodes += 1
        _require(depth <= 16 and nodes <= 4096, "budget-exceeded")
        kind = type(value)
        if kind is str:
            _require(len(value) <= 65536, "budget-exceeded")
            string_bytes += _string_bytes(value, 262144, "invalid-json")
            _require(string_bytes <= 65536, "budget-exceeded")
        elif kind is dict:
            _require(len(value) <= 256, "budget-exceeded")
            pending.extend((item, depth + 1) for pair in value.items() for item in pair)
        elif kind is list:
            _require(len(value) <= 256, "budget-exceeded")
            pending.extend((item, depth + 1) for item in value)
        elif kind is int:
            _require(value.bit_length() <= 256, "budget-exceeded")
        elif kind is float:
            _require(math.isfinite(value), "invalid-json")
        else:
            _require(value is None or kind is bool, "invalid-json")


def _pairs(items):
    result = {}
    for key, value in items:
        _require(key not in result, "invalid-json")
        result[key] = value
    return result


def _nonfinite(_value):
    raise NextcloudContractError("invalid-json")


def _float(text):
    value = float(text)
    _require(math.isfinite(value), "invalid-json")
    return value


def _integer(text):
    _require(len(text.lstrip("-")) <= 78, "budget-exceeded")
    value = int(text)
    _require(value.bit_length() <= 256, "budget-exceeded")
    return value


def _decode(raw):
    _require(type(raw) is bytes, "invalid-input")
    _require(0 < len(raw) <= 262144, "budget-exceeded")
    value = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs, parse_constant=_nonfinite,
                       parse_float=_float, parse_int=_integer)
    _budget(value)
    return value


def _vector(value, expected, code):
    _require(type(value) is list and all(type(item) is str for item in value) and value == expected, code)


def _validate(role_ids, container_json, redis_password):
    _require(type(role_ids) is dict and len(role_ids) == 6 and all(type(key) is str for key in role_ids)
             and set(role_ids) == set(_ROLES), "role-membership")
    for role in _ROLES:
        identity = role_ids[role]
        _require(type(identity) is str and re.fullmatch(r"[0-9a-f]{64}", identity) is not None, "container-identity")
    _require(len(set(role_ids.values())) == 6, "container-identity")
    _require(type(container_json) is dict and len(container_json) == 6 and all(type(key) is str for key in container_json)
             and set(container_json) == set(_ROLES), "role-membership")
    _require(_string_bytes(redis_password, 4096, "invalid-input") > 0, "invalid-input")
    records = {}
    for role in _ROLES:
        record = _decode(container_json[role])
        _require(type(record) is dict and type(record.get("Id")) is str and record["Id"] == role_ids[role],
                 "container-identity")
        records[role] = record
    for role, expected in (("cache", ["redis-server", "--requirepass", redis_password]),
                           ("init", ["php", "/var/www/html/occ", "status"]), ("cron", ["/cron.sh"])):
        config = records[role].get("Config")
        _require(type(config) is dict, "authored-command")
        _vector(config.get("Cmd"), expected, "authored-command")
    user = records["init"]["Config"].get("User")
    _require(type(user) is str and user == "www-data", "authored-user")
    for role, test, interval, retries in (
            ("db", ["CMD-SHELL", "pg_isready -U boxferry -d nextcloud"], 2000000000, 60),
            ("cache", ["CMD", "redis-cli", "-a", redis_password, "ping"], 2000000000, 60),
            ("app", ["CMD-SHELL", "php -r '$s=json_decode(@file_get_contents(\"http://localhost/status.php\"),true); exit(is_array($s)&&($s[\"installed\"]??false)&&!($s[\"maintenance\"]??true)&&!($s[\"needsDbUpgrade\"]??true)?0:1);'"], 5000000000, 120)):
        config = records[role].get("Config")
        _require(type(config) is dict and type(config.get("Healthcheck")) is dict, "authored-health")
        health = config["Healthcheck"]
        _vector(health.get("Test"), test, "authored-health")
        for field, expected in (("Interval", interval), ("Timeout", interval), ("Retries", retries)):
            _require(type(health.get(field)) is int and health[field] == expected, "authored-health")


def validate_authored_overrides(*, role_ids: dict[str, str], container_json: dict[str, bytes], redis_password: str) -> None:
    """Compare ONLY the explicit authored overrides, using already-admitted bytes.

    Exactly six unique held IDs and matching inspect roots are mandatory. Raw
    bodies are not reserialized or changed. No output is emitted, even on failure.
    Other roles' fields and optional health defaults confer no positive claim.
    """
    try:
        _validate(role_ids, container_json, redis_password)
    except NextcloudContractError as error:
        code = error.code
    except (RecursionError, OverflowError):
        code = "budget-exceeded"
    except (UnicodeError, ValueError):
        code = "invalid-json"
    except Exception:
        code = "invalid-input"
    else:
        return None
    # Leave handlers before constructing the public error: private JSON parser
    # exceptions and supplied values must not survive in __context__/__cause__.
    raise NextcloudContractError(code) from None
