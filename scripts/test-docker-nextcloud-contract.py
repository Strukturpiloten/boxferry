#!/usr/bin/env python3
"""Authored inspect-shaped predicate controls; no native or full-semantic proof.

All expected literals are independently authored from the Compose fixture and
private semantic ledger, never loaded from comparator constants or producer
output. Synthetic credential bytes are never printed by test assertions.
"""
from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import json
import os
import pathlib
import socket
import subprocess
import traceback
import unittest
from unittest import mock


def forbidden(*_args, **_kwargs):
    raise AssertionError("filesystem/process/socket operation forbidden in pure comparator")


subprocess.Popen = forbidden
subprocess.run = forbidden
os.system = forbidden
socket.socket.connect = forbidden
socket.socket.connect_ex = forbidden
socket.socket.bind = forbidden
socket.socket.listen = forbidden
socket.create_connection = forbidden
SPEC = importlib.util.spec_from_file_location("nextcloud_contract", pathlib.Path(__file__).with_name("lib") / "docker-nextcloud-contract.py")
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
ROLES = ("db", "cache", "app", "init", "cron", "frontend")
PASSWORD = "authored-offline-cache-canary"
UNRELATED = "authored-unrelated-secret-canary"


def raw(value):
    return json.dumps(value, ensure_ascii=True, separators=(",", ":")).encode("utf-8")


def fixture(password=PASSWORD):
    ids = dict(zip(ROLES, [f"{number:064x}" for number in range(101, 107)]))
    # These are caller-assembled inspect-shaped values, not Engine observations.
    records = {
        "db": {"Id": ids["db"], "Config": {"Healthcheck": {
            "Test": ["CMD-SHELL", "pg_isready -U boxferry -d nextcloud"],
            "Interval": 2000000000, "Timeout": 2000000000, "Retries": 60}}},
        "cache": {"Id": ids["cache"], "Config": {
            "Cmd": ["redis-server", "--requirepass", password], "Healthcheck": {
                "Test": ["CMD", "redis-cli", "-a", password, "ping"],
                "Interval": 2000000000, "Timeout": 2000000000, "Retries": 60}}},
        "app": {"Id": ids["app"], "Config": {"Healthcheck": {
            "Test": ["CMD-SHELL", 'php -r \'$s=json_decode(@file_get_contents("http://localhost/status.php"),true); exit(is_array($s)&&($s["installed"]??false)&&!($s["maintenance"]??true)&&!($s["needsDbUpgrade"]??true)?0:1);\''],
            "Interval": 5000000000, "Timeout": 5000000000, "Retries": 120}}},
        "init": {"Id": ids["init"], "Config": {"Cmd": ["php", "/var/www/html/occ", "status"], "User": "www-data"}},
        "cron": {"Id": ids["cron"], "Config": {"Cmd": ["/cron.sh"]}},
        "frontend": {"Id": ids["frontend"]},
    }
    for record in records.values():
        record["PrivateExtra"] = {"credential": UNRELATED, "finite": 1.25}
    return ids, records


class AuthoredOverrideTests(unittest.TestCase):
    def call(self, ids=None, records=None, bodies=None, password=PASSWORD):
        if ids is None or records is None:
            default_ids, default_records = fixture(password if type(password) is str else PASSWORD)
            ids = default_ids if ids is None else ids
            records = default_records if records is None else records
        return MODULE.validate_authored_overrides(role_ids=ids,
            container_json=bodies if bodies is not None else {role: raw(value) for role, value in records.items()},
            redis_password=password)

    def refusal(self, code, **kwargs):
        with contextlib.redirect_stdout(io.StringIO()) as output, contextlib.redirect_stderr(io.StringIO()) as error:
            with self.assertRaises(MODULE.NextcloudContractError) as refused:
                self.call(**kwargs)
        failure = refused.exception
        self.assertEqual(failure.code, code)
        self.assertIsNone(failure.__context__)
        self.assertIsNone(failure.__cause__)
        rendered = str(failure) + repr(failure) + "".join(traceback.format_exception(failure))
        self.assertTrue(all(value not in rendered for value in (PASSWORD, UNRELATED)), "private data in error rendering")
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(error.getvalue(), "")
        return failure

    def test_inline_six_role_positive_and_inputs_unchanged(self):
        ids, records = fixture()
        original = copy.deepcopy((ids, records))
        bodies = {role: raw(value) for role, value in records.items()}
        held = dict(bodies)
        self.assertIsNone(self.call(ids=ids, records=records, bodies=bodies))
        self.assertTrue((ids, records) == original and bodies == held, "comparator changed private inputs")

    def test_no_filesystem_runtime_or_console_activity(self):
        with mock.patch("builtins.open", side_effect=forbidden), mock.patch.object(os, "open", side_effect=forbidden), \
                contextlib.redirect_stdout(io.StringIO()) as output, contextlib.redirect_stderr(io.StringIO()) as error:
            self.assertIsNone(self.call())
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(error.getvalue(), "")

    def test_exact_role_membership_for_both_mappings(self):
        for role in ROLES:
            ids, records = fixture()
            ids.pop(role)
            self.refusal("role-membership", ids=ids, records=records)
            ids, records = fixture()
            records.pop(role)
            self.refusal("role-membership", ids=ids, records=records)
        ids, records = fixture()
        ids["proxy"] = "f" * 64
        self.refusal("role-membership", ids=ids, records=records)
        ids, records = fixture()
        records["second"] = {"Id": "f" * 64}
        self.refusal("role-membership", ids=ids, records=records)

    def test_unique_canonical_held_ids_and_exact_root_id_binding(self):
        for value in ("bad", "A" * 64, "1" * 63, "sha256:" + "1" * 64, True, None):
            ids, records = fixture()
            ids["db"] = value
            self.refusal("container-identity", ids=ids, records=records)
        ids, records = fixture()
        ids["cache"] = ids["db"]
        self.refusal("container-identity", ids=ids, records=records)
        for role in ROLES:
            for change in ("missing", "wrong", "null", "array"):
                ids, records = fixture()
                if change == "missing": records[role].pop("Id")
                elif change == "wrong": records[role]["Id"] = "f" * 64
                elif change == "null": records[role]["Id"] = None
                else: records[role] = []
                self.refusal("container-identity", ids=ids, records=records)

    def test_closed_mapping_types_and_byte_only_bodies(self):
        ids, records = fixture()
        for invalid in (None, [], tuple(ids.items())):
            with self.assertRaises(MODULE.NextcloudContractError) as refused:
                MODULE.validate_authored_overrides(role_ids=invalid, container_json={}, redis_password=PASSWORD)
            self.assertEqual(refused.exception.code, "role-membership")
        for invalid in (None, [], tuple(records.items())):
            with self.assertRaises(MODULE.NextcloudContractError) as refused:
                MODULE.validate_authored_overrides(role_ids=ids, container_json=invalid, redis_password=PASSWORD)
            self.assertEqual(refused.exception.code, "role-membership")
        for body in (None, "{}", bytearray(b"{}"), memoryview(b"{}")):
            bodies = {role: raw(value) for role, value in records.items()}
            bodies["frontend"] = body
            self.refusal("invalid-input", ids=ids, records=records, bodies=bodies)

    def test_authored_command_missing_null_empty_malformed_shell_and_order(self):
        for role in ("cache", "init", "cron"):
            for value in (None, [], {}, True, "shell-string", ["changed"], [None]):
                ids, records = fixture()
                records[role]["Config"]["Cmd"] = value
                self.refusal("authored-command", ids=ids, records=records)
            ids, records = fixture()
            records[role]["Config"].pop("Cmd")
            self.refusal("authored-command", ids=ids, records=records)
            ids, records = fixture()
            records[role]["Config"]["Cmd"] = " ".join(records[role]["Config"]["Cmd"])
            self.refusal("authored-command", ids=ids, records=records)
        ids, records = fixture()
        records["cache"]["Config"]["Cmd"] = ["redis-server", PASSWORD, "--requirepass"]
        self.refusal("authored-command", ids=ids, records=records)
        ids, records = fixture()
        records["init"]["Config"]["Cmd"] = ["php", "status", "/var/www/html/occ"]
        self.refusal("authored-command", ids=ids, records=records)

    def test_checked_config_parents_must_be_objects_not_missing_null_or_empty(self):
        for role in ("db", "cache", "app", "init", "cron"):
            for value in (None, [], "", True, {}):
                ids, records = fixture()
                records[role]["Config"] = value
                self.refusal("authored-command" if role in {"cache", "init", "cron"} else "authored-health",
                             ids=ids, records=records)
            ids, records = fixture()
            records[role].pop("Config")
            self.refusal("authored-command" if role in {"cache", "init", "cron"} else "authored-health",
                         ids=ids, records=records)

    def test_init_user_must_be_exact_authored_nonempty_scalar(self):
        for value in (None, "", "0", "root", "www-data:root", ["www-data"], {}, True):
            ids, records = fixture()
            records["init"]["Config"]["User"] = value
            self.refusal("authored-user", ids=ids, records=records)
        ids, records = fixture()
        records["init"]["Config"].pop("User")
        self.refusal("authored-user", ids=ids, records=records)

    def test_health_parent_test_missing_null_empty_none_and_shell_are_refused(self):
        for role in ("db", "cache", "app"):
            for value in (None, {}, [], "health", False):
                ids, records = fixture()
                records[role]["Config"]["Healthcheck"] = value
                self.refusal("authored-health", ids=ids, records=records)
            for value in (None, [], {}, "CMD-SHELL check", ["NONE"], ["CMD", "changed"], [None]):
                ids, records = fixture()
                records[role]["Config"]["Healthcheck"]["Test"] = value
                self.refusal("authored-health", ids=ids, records=records)
            for field in ("Healthcheck", "Test"):
                ids, records = fixture()
                if field == "Healthcheck": records[role]["Config"].pop(field)
                else: records[role]["Config"]["Healthcheck"].pop(field)
                self.refusal("authored-health", ids=ids, records=records)
            ids, records = fixture()
            health = records[role]["Config"]["Healthcheck"]
            health["Test"] = " ".join(health["Test"])
            self.refusal("authored-health", ids=ids, records=records)

    def test_cache_health_credential_order_and_php_literal_dollars_are_exact(self):
        ids, records = fixture()
        records["cache"]["Config"]["Healthcheck"]["Test"] = ["CMD", "redis-cli", "-a", "ping", PASSWORD]
        self.refusal("authored-health", ids=ids, records=records)
        ids, records = fixture()
        test = records["app"]["Config"]["Healthcheck"]["Test"]
        test[1] = test[1].replace("$s", "$$s")
        self.refusal("authored-health", ids=ids, records=records)
        ids, records = fixture()
        test = records["app"]["Config"]["Healthcheck"]["Test"]
        records["app"]["Config"]["Healthcheck"]["Test"] = ["CMD-SHELL", *test[1].split()]
        self.refusal("authored-health", ids=ids, records=records)

    def test_health_numbers_require_exact_typed_nanoseconds_and_retries(self):
        for role in ("db", "cache", "app"):
            for field in ("Interval", "Timeout", "Retries"):
                for value in (None, True, False, 0, -1, 2.0, "60", [], {}):
                    ids, records = fixture()
                    records[role]["Config"]["Healthcheck"][field] = value
                    self.refusal("authored-health", ids=ids, records=records)
                ids, records = fixture()
                expected = records[role]["Config"]["Healthcheck"][field]
                records[role]["Config"]["Healthcheck"][field] = float(expected)
                self.refusal("authored-health", ids=ids, records=records)
                ids, records = fixture()
                records[role]["Config"]["Healthcheck"][field] += 1
                self.refusal("authored-health", ids=ids, records=records)
                ids, records = fixture()
                records[role]["Config"]["Healthcheck"].pop(field)
                self.refusal("authored-health", ids=ids, records=records)

    def test_only_explicit_overrides_are_checked_no_default_or_inherited_claims(self):
        ids, records = fixture()
        for role in ROLES:
            config = records[role].setdefault("Config", {})
            config["Entrypoint"] = ["deliberately-unmeasured-entrypoint"]
            config["WorkingDir"] = "unmeasured"
            if role not in {"cache", "init", "cron"}: config["Cmd"] = "unmeasured-shell"
            if role != "init": config["User"] = None
            if role in {"db", "cache", "app"}:
                config["Healthcheck"]["StartPeriod"] = 0
                config["Healthcheck"]["StartInterval"] = None
            else: config["Healthcheck"] = {"Test": ["NONE"], "StartPeriod": 17.5}
        self.assertIsNone(self.call(ids=ids, records=records))
        records["frontend"]["Config"] = None
        self.assertIsNone(self.call(ids=ids, records=records))

    def test_password_is_bounded_nonempty_unicode_and_private(self):
        for value in (None, b"secret", [], {}, True, "", "x" * 4097, "é" * 2049, "\ud800"):
            self.refusal("invalid-input", password=value)
        for value in ("z" * 4096, "é" * 2048, " spaces $ ' and = remain bytes "):
            self.assertIsNone(self.call(password=value))

    def test_strict_json_duplicates_nonfinite_utf8_and_root_shape(self):
        ids, records = fixture()
        samples = (b"", b"{", b"\xff", b'{"Id":"held","Id":"held"}',
                   b'{"nested":{"x":1,"x":2}}', b'{"x":NaN}', b'{"x":Infinity}', b'{"x":-Infinity}',
                   b'{"x":1e999}', b'{"x":"\\ud800"}')
        for value in samples:
            bodies = {role: raw(record) for role, record in records.items()}
            bodies["frontend"] = value
            self.refusal("budget-exceeded" if value == b"" else "invalid-json", ids=ids, records=records, bodies=bodies)

    def test_response_byte_boundary_is_256k_not_request_metadata_limit(self):
        ids, records = fixture()
        bodies = {role: raw(record) for role, record in records.items()}
        body = bodies["frontend"]
        bodies["frontend"] = body + b" " * (262144 - len(body))
        self.assertIsNone(self.call(ids=ids, records=records, bodies=bodies))
        bodies["frontend"] += b" "
        self.refusal("budget-exceeded", ids=ids, records=records, bodies=bodies)

    def test_depth_collection_string_node_and_integer_budgets(self):
        ids, original = fixture()
        variants = []
        nested = None
        for _ in range(17): nested = [nested]
        variants.extend([nested, list(range(257)), {str(index): None for index in range(257)}, "x" * 65537,
                         ["x" * 32768, "y" * 32768], [list(range(256)) for _ in range(17)], 2**256])
        for value in variants:
            records = copy.deepcopy(original)
            records["frontend"]["ExtraBudget"] = value
            self.refusal("budget-exceeded", ids=ids, records=records)

    def test_native_budget_positive_boundaries_and_unrelated_finite_numbers(self):
        ids, original = fixture()
        nested = None
        for _ in range(15): nested = [nested]  # Root field depth1 through null depth16.
        for value in (nested, list(range(256)), 2**256 - 1, -1e200, "é" * 16000):
            records = copy.deepcopy(original)
            records["frontend"]["ExtraBudget"] = value
            self.assertIsNone(self.call(ids=ids, records=records))
        records = copy.deepcopy(original)
        records["frontend"]["ExtraBudget"] = [nested]
        self.refusal("budget-exceeded", ids=ids, records=records)

    def test_console_and_normal_error_rendering_never_retain_private_json(self):
        ids, records = fixture()
        bodies = {role: raw(value) for role, value in records.items()}
        bodies["frontend"] = b'{"private":"' + UNRELATED.encode() + b'",broken:' + PASSWORD.encode() + b'}'
        error = self.refusal("invalid-json", ids=ids, records=records, bodies=bodies)
        self.assertEqual(error.args, ("invalid-json",))
        self.assertEqual(str(MODULE.NextcloudContractError(PASSWORD)), "invalid-input")


if __name__ == "__main__":
    unittest.main()
