#!/usr/bin/env python3
"""Network-free controls for the pure authored Nextcloud input consistency seam."""
from __future__ import annotations

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

MODULE_PATH = pathlib.Path(__file__).resolve().parent / "lib" / "docker-nextcloud-input-contract.py"
SPEC = importlib.util.spec_from_file_location("nextcloud_input_contract", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)


def fixture(phase: str = "source", provisioner: str = "native", lane: str = "upstream-rootless") -> tuple[dict, dict]:
    prefix = "bf-nextcloud-a1b2c3d4-" + provisioner
    source_ids = {role: digit * 64 for role, digit in
                  (("db", "1"), ("cache", "2"), ("app", "3"), ("init", "4"), ("cron", "5"), ("frontend", "6"))}
    current_ids = source_ids if phase == "source" else {role: digit * 64 for role, digit in
                  (("db", "a"), ("cache", "b"), ("app", "c"), ("init", "d"), ("cron", "e"), ("frontend", "f"))}
    source_generation = "cloud" if phase == "source" else "restore"
    destination_generation = "restore" if phase == "source" else "reexport"
    bindings = {
        "lane": lane, "provisioner": provisioner, "phase": phase, "run_id": "bf-a1b2c3d4",
        "prefix": prefix, "application_name": prefix + "-cloud", "socket": "/private/owned/docker.sock",
        # Synthetic independently supplied profile objects, not software pins/native admissions.
        # Their differing modes deliberately prevent inference from target or lane.
        "source_profile": {"engine_release": "71.3.2", "daemon_mode": "rootful"},
        "targets": {
            "compose": {"implementation": "compose-specification", "version": "91.4.2"},
            "quadlet": {"implementation": "podman", "version": "92.5.3"},
            "podman": {"implementation": "podman", "version": "93.6.4", "daemon_mode": "rootless"},
            "docker": {"implementation": "docker", "version": "94.7.5", "engine_build": "upstream",
                       "daemon_mode": "rootless"},
        },
        "role_ids": copy.deepcopy(current_ids), "proxy_id": "9" * 64, "peer_id": "0" * 64,
        "resources": {
            "backend_network_id": "7" * 64, "edge_network_id": "8" * 64,
            "database_volume": f"{prefix}-{source_generation}-database",
            "redis_volume": f"{prefix}-{source_generation}-redis",
            "nextcloud_volume": f"{prefix}-{source_generation}-nextcloud",
        },
    }
    if phase == "reacquired":
        bindings["prior_source_role_ids"] = source_ids
    request = {
        "schema_version": 1, "application_name": prefix + "-cloud", "socket": "/private/owned/docker.sock",
        "container_ids": list(reversed(list(current_ids.values()))),
        "resource_decisions": [
            {"kind": "network", "source": "7" * 64, "action": "create",
             "target": f"{prefix}-{destination_generation}-backend"},
            {"kind": "network", "source": "8" * 64, "action": "external", "target": f"{prefix}-shared-edge"},
            *({"kind": "volume", "source": f"{prefix}-{source_generation}-{name}", "action": "create",
               "target": f"{prefix}-{destination_generation}-{name}"} for name in ("database", "redis", "nextcloud")),
        ],
        "public_labels": [],
        "promote_protected_environment_values": True, "include_environment_values": True,
        "promote_same_host_bind_mounts": True, "source_profile": copy.deepcopy(bindings["source_profile"]),
        "loss_policy": "allow-partial", "targets": copy.deepcopy(bindings["targets"]),
    }
    subjects = [("container", identity) for identity in current_ids.values()]
    subjects += [("network", "7" * 64)]
    subjects += [("volume", f"{prefix}-{source_generation}-{name}") for name in ("database", "redis", "nextcloud")]
    for kind, source in subjects:
        for key in ("io.boxferry.live-run", "io.boxferry.application"):
            request["public_labels"].append({"kind": kind, "source": source, "key": key})
    return request, bindings


def selection(bindings: dict, kind: str = "label") -> dict:
    ids = list(bindings["role_ids"].values())
    selector = {"kind": "label", "key": "io.boxferry.application", "value": bindings["application_name"]}
    if kind == "all":
        ids += [bindings["proxy_id"], bindings["peer_id"]]
        selector = {"kind": "all"}
    return {"schema_version": 1, "application_name": bindings["application_name"], "socket": bindings["socket"],
            "source_profile": copy.deepcopy(bindings["source_profile"]), "expected_container_ids": ids[::-1],
            "selector": selector}


class InputContractTests(unittest.TestCase):
    def failure(self, function, request, bindings, code=None):
        with self.assertRaises(contract.InputContractError) as caught:
            function(request, bindings)
        error = caught.exception
        if code is not None:
            self.assertEqual(error.code, code)
        self.assertEqual(error.args, (error.code,))
        self.assertIn(error.code, contract._CODES)
        return error

    def test_all_eight_contexts_and_permutations_are_consistent_not_admission(self):
        for lane in ("upstream-rootful", "upstream-rootless"):
            for provisioner in ("native", "compose"):
                for phase in ("source", "reacquired"):
                    with self.subTest(lane=lane, provisioner=provisioner, phase=phase):
                        request, bindings = fixture(phase, provisioner, lane)
                        before = copy.deepcopy((request, bindings))
                        self.assertIsNone(contract.validate_outcome_request(request, bindings))
                        request["resource_decisions"].reverse()
                        request["public_labels"].reverse()
                        self.assertIsNone(contract.validate_outcome_request(request, bindings))
                        request["resource_decisions"].reverse()
                        request["public_labels"].reverse()
                        self.assertEqual((request, bindings), before)
                        for kind in ("label", "all"):
                            self.assertIsNone(contract.validate_selection_request(selection(bindings, kind), bindings))

    def test_ids_must_be_exact_distinct_held_members(self):
        for replacement in [
            ["1" * 64] * 6, ["A" * 64] + ["2" * 64] * 5, ["1" * 63] + ["2" * 64] * 5,
            [], ["1" * 64] * 7, ["9" * 64, "2" * 64, "3" * 64, "4" * 64, "5" * 64, "6" * 64],
        ]:
            request, bindings = fixture()
            request["container_ids"] = replacement
            self.failure(contract.validate_outcome_request, request, bindings, "role-membership")

    def test_held_bindings_require_six_exact_unique_roles_and_distinct_excluded_ids(self):
        for role in ("db", "cache", "app", "init", "cron", "frontend"):
            for defect in ("missing", "renamed", "duplicate", "proxy", "peer"):
                with self.subTest(role=role, defect=defect):
                    request, bindings = fixture()
                    roles = bindings["role_ids"]
                    if defect == "missing":
                        roles.pop(role)
                    elif defect == "renamed":
                        roles["worker"] = roles.pop(role)
                    elif defect == "duplicate":
                        roles[role] = next(value for name, value in roles.items() if name != role)
                    else:
                        bindings[defect + "_id"] = roles[role]
                    self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
                    self.failure(contract.validate_selection_request, selection(bindings), bindings,
                                 "invalid-bindings")
        request, bindings = fixture()
        bindings["role_ids"]["worker"] = "a" * 64
        self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")

    def test_reacquired_requires_prior_ids_and_refuses_any_source_role_overlap(self):
        request, bindings = fixture("reacquired")
        bindings.pop("prior_source_role_ids")
        self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
        for source_role in ("db", "cache", "app", "init", "cron", "frontend"):
            request, bindings = fixture("reacquired")
            bindings["role_ids"]["frontend"] = bindings["prior_source_role_ids"][source_role]
            self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
        request, bindings = fixture()
        bindings["prior_source_role_ids"] = copy.deepcopy(bindings["role_ids"])
        self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")

    def test_reacquired_proxy_and_peer_cannot_reuse_any_prior_source_role(self):
        for excluded_role in ("proxy_id", "peer_id"):
            for prior_role in ("db", "cache", "app", "init", "cron", "frontend"):
                with self.subTest(excluded_role=excluded_role, prior_role=prior_role):
                    request, bindings = fixture("reacquired")
                    prior_ids = set(bindings["prior_source_role_ids"].values())
                    self.assertTrue(set(bindings["role_ids"].values()).isdisjoint(prior_ids))
                    bindings[excluded_role] = bindings["prior_source_role_ids"][prior_role]
                    self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
                    for kind in ("all", "label"):
                        self.failure(contract.validate_selection_request, selection(bindings, kind), bindings,
                                     "invalid-bindings")

    def test_closed_binding_identity_and_phase_schemas(self):
        for key, wrong in [("phase", "reexport"), ("provisioner", "podman"), ("lane", "unreviewed"),
                           ("run_id", "bf-not-a-token"), ("prefix", "other"),
                           ("application_name", "other"), ("socket", "relative.sock")]:
            request, bindings = fixture()
            bindings[key] = wrong
            self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
        for key in ("role_ids", "resources", "source_profile", "targets"):
            request, bindings = fixture()
            bindings[key]["unknown-private-key"] = "private-value"
            self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
        request, bindings = fixture()
        bindings["unexpected"] = True
        self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")
        request, bindings = fixture()
        bindings["peer_id"] = bindings["proxy_id"]
        self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")

    def test_source_and_target_profiles_are_explicit_equal_objects(self):
        request, bindings = fixture()
        self.assertIsNone(contract.validate_outcome_request(request, bindings))
        for profile in ("source_profile", "targets"):
            altered = copy.deepcopy(request)
            if profile == "source_profile":
                altered[profile]["engine_release"] = "different"
            else:
                altered[profile]["docker"]["version"] = "different"
            self.failure(contract.validate_outcome_request, altered, bindings,
                         "source-profile" if profile == "source_profile" else "target-profile")
        altered = copy.deepcopy(request)
        altered["source_profile"] = {"engine_release": request["targets"]["docker"]["version"],
                                     "daemon_mode": request["targets"]["docker"]["daemon_mode"]}
        self.failure(contract.validate_outcome_request, altered, bindings, "source-profile")
        for value in [None, True, [], "unknown"]:
            altered = copy.deepcopy(request)
            altered["source_profile"]["daemon_mode"] = value
            self.failure(contract.validate_outcome_request, altered, bindings, "source-profile")
            altered = copy.deepcopy(request)
            altered["targets"]["docker"]["daemon_mode"] = value
            self.failure(contract.validate_outcome_request, altered, bindings, "target-profile")

    def test_request_identity_and_version_are_exact(self):
        for key, wrong, code in [
            ("application_name", "other", "request-identity"), ("socket", "/other/private.sock", "request-identity"),
            ("schema_version", True, "request-shape"), ("schema_version", 2, "request-shape"),
        ]:
            request, bindings = fixture()
            request[key] = wrong
            self.failure(contract.validate_outcome_request, request, bindings, code)

    def test_provisioning_origin_and_each_profile_remain_bound_to_held_context(self):
        request, bindings = fixture()
        other_request, other_bindings = fixture(provisioner="compose")
        self.failure(contract.validate_outcome_request, other_request, bindings, "request-identity")
        self.failure(contract.validate_selection_request, selection(other_bindings), bindings,
                     "request-identity")
        for key in ("engine_release", "daemon_mode"):
            for function, candidate in (
                (contract.validate_outcome_request, copy.deepcopy(request)),
                (contract.validate_selection_request, selection(bindings)),
            ):
                candidate["source_profile"][key] = "rootless" if key == "daemon_mode" else "71.3.3"
                self.failure(function, candidate, bindings, "source-profile")
        for target in ("compose", "quadlet", "podman", "docker"):
            candidate = copy.deepcopy(request)
            candidate["targets"][target]["version"] = "95.0.0"
            self.failure(contract.validate_outcome_request, candidate, bindings, "target-profile")

    def test_resource_decisions_are_phase_source_and_destination_bound(self):
        for phase in ("source", "reacquired"):
            for index, key, wrong in [(0, "source", "e" * 64), (1, "action", "create"),
                                      (1, "target", "wrong-edge"), (2, "action", "external"),
                                      (2, "target", "wrong-generation"), (3, "source", "unselected-volume"),
                                      (4, "kind", "network")]:
                request, bindings = fixture(phase)
                request["resource_decisions"][index][key] = wrong
                self.failure(contract.validate_outcome_request, request, bindings, "resource-decisions")
        for count in (4, 6):
            request, bindings = fixture()
            rows = request["resource_decisions"]
            request["resource_decisions"] = rows[:count] if count == 4 else rows + [copy.deepcopy(rows[0])]
            self.failure(contract.validate_outcome_request, request, bindings, "resource-decisions")
        request, bindings = fixture()
        request["resource_decisions"][4] = copy.deepcopy(request["resource_decisions"][0])
        self.failure(contract.validate_outcome_request, request, bindings, "resource-decisions")
        request, bindings = fixture("reacquired")
        bindings["resources"]["database_volume"] = bindings["prefix"] + "-cloud-database"
        self.failure(contract.validate_outcome_request, request, bindings, "invalid-bindings")

    def test_labels_require_twenty_exact_scopes_without_external_or_provider_leakage(self):
        for replacement in [
            {"kind": "network", "source": "8" * 64, "key": "io.boxferry.application"},
            {"kind": "container", "source": "9" * 64, "key": "io.boxferry.application"},
            {"kind": "container", "source": "0" * 64, "key": "io.boxferry.live-run"},
            {"kind": "container", "source": "1" * 64, "key": "com.docker.compose.project"},
            {"kind": "container", "source": "1" * 64, "key": "io.boxferry.*"},
            {"kind": "container", "source": "1" * 64, "key": "io.boxferry.application", "value": "not-authorized"},
        ]:
            request, bindings = fixture()
            request["public_labels"][0] = replacement
            self.failure(contract.validate_outcome_request, request, bindings, "public-labels")
        request, bindings = fixture()
        request["public_labels"][0] = copy.deepcopy(request["public_labels"][1])
        self.failure(contract.validate_outcome_request, request, bindings, "public-labels")
        for count in (0, 19, 21):
            request, bindings = fixture()
            labels = request["public_labels"]
            request["public_labels"] = labels[:count] if count < 20 else labels + [copy.deepcopy(labels[0])]
            self.failure(contract.validate_outcome_request, request, bindings, "public-labels")

    def test_each_consent_is_explicit_true_and_not_implied_by_the_other_two(self):
        for key in ("promote_protected_environment_values", "include_environment_values", "promote_same_host_bind_mounts"):
            for wrong in (False, None, 1, "true"):
                request, bindings = fixture()
                request[key] = wrong
                self.failure(contract.validate_outcome_request, request, bindings, "recipe-consents")
            request, bindings = fixture()
            request.pop(key)
            self.failure(contract.validate_outcome_request, request, bindings, "request-shape")
        request, bindings = fixture()
        request["loss_policy"] = "allow-approximate"
        self.failure(contract.validate_outcome_request, request, bindings, "recipe-policy")

    def test_selection_schema_and_membership_parity(self):
        for phase in ("source", "reacquired"):
            _, bindings = fixture(phase)
            for kind in ("label", "all"):
                request = selection(bindings, kind)
                self.assertIsNone(contract.validate_selection_request(request, bindings))
                for extra in ("container_ids", "targets", "resource_decisions", "public_labels", "loss_policy",
                              "promote_protected_environment_values", "include_environment_values",
                              "promote_same_host_bind_mounts"):
                    altered = copy.deepcopy(request)
                    altered[extra] = True
                    self.failure(contract.validate_selection_request, altered, bindings, "selection-shape")
                altered = copy.deepcopy(request)
                altered["expected_container_ids"].pop()
                self.failure(contract.validate_selection_request, altered, bindings, "selection-membership")
            request = selection(bindings)
            request["selector"] = {"kind": "label", "key": "io.boxferry.live-run", "value": bindings["run_id"]}
            self.failure(contract.validate_selection_request, request, bindings, "selection-selector")
            request = selection(bindings, "all")
            request["selector"]["key"] = "not-accepted"
            self.failure(contract.validate_selection_request, request, bindings, "selection-selector")
            request = selection(bindings, "all")
            request["expected_container_ids"][0] = request["expected_container_ids"][1]
            self.failure(contract.validate_selection_request, request, bindings, "selection-membership")

    def test_strict_loader_then_closed_schemas_reject_duplicates_unknowns_and_nonfinite(self):
        request, bindings = fixture()
        raw = json.dumps(request).encode()
        loaded = contract.load_json_document(raw)
        self.assertIsNone(contract.validate_outcome_request(loaded, bindings))
        for raw in [b'{"schema_version":1,"schema_version":1}', b'{"nested":{"x":1,"x":2}}',
                    b'{"x":NaN}', b'{"x":Infinity}', b'{"x":-Infinity}', b'{"x":', b"\xff", b"[]"]:
            with self.subTest(raw=raw), self.assertRaises(contract.InputContractError):
                contract.load_json_document(raw)
        request["unknown-private-field"] = "private-value"
        self.failure(contract.validate_outcome_request, contract.load_json_document(json.dumps(request).encode()),
                     bindings, "request-shape")

    def test_malformed_private_data_and_budgets_fail_with_contract_errors(self):
        request, bindings = fixture()
        for wrong in (None, [], True, 1, "private-data"):
            self.failure(contract.validate_outcome_request, wrong, bindings)
            self.failure(contract.validate_outcome_request, request, wrong)
        for mutate in (
            lambda value: value.update({"socket": "\ud800"}),
            lambda value: value.update({"socket": "/private/\x00sock"}),
            lambda value: value.update({"schema_version": 1.0}),
            lambda value: value.update({"container_ids": {"not": "a-list"}}),
            lambda value: value["resource_decisions"][0].update({"source": []}),
            lambda value: value["public_labels"][0].update({"key": {}}),
        ):
            altered = copy.deepcopy(request)
            mutate(altered)
            self.failure(contract.validate_outcome_request, altered, bindings)
        for raw in [b" " * 65537, b"[" * 1000 + b"]" * 1000, b'{"x":"' + b"x" * 65536 + b'"}']:
            with self.assertRaises(contract.InputContractError):
                contract.load_json_document(raw)

    def test_wire_budget_boundary_and_cyclic_or_oversized_objects(self):
        request, bindings = fixture()
        raw = json.dumps(request).encode()
        exact = raw + b" " * (65536 - len(raw))
        self.assertIsNone(contract.validate_outcome_request(contract.load_json_document(exact), bindings))
        with self.assertRaises(contract.InputContractError) as caught:
            contract.load_json_document(exact + b" ")
        self.assertEqual(caught.exception.code, "budget-exceeded")
        cyclic = {}
        cyclic["cycle"] = cyclic
        self.failure(contract.validate_outcome_request, cyclic, bindings, "budget-exceeded")
        oversized = {f"key-{index}": None for index in range(257)}
        self.failure(contract.validate_outcome_request, oversized, bindings, "budget-exceeded")

    def test_integer_budget_is_bounded_before_and_after_json_decoding(self):
        for integer in (0, (1 << 256) - 1, -((1 << 256) - 1)):
            self.assertEqual(contract.load_json_document(json.dumps({"n": integer}).encode()),
                             {"n": integer})
        request, bindings = fixture()
        for integer in (1 << 256, -(1 << 256)):
            with self.assertRaises(contract.InputContractError) as caught:
                contract.load_json_document(json.dumps({"n": integer}).encode())
            self.assertEqual(caught.exception.code, "budget-exceeded")
            candidate = copy.deepcopy(request)
            candidate["schema_version"] = integer
            self.failure(contract.validate_outcome_request, candidate, bindings, "budget-exceeded")
        for digits in (79, 5000, 65530):
            with self.assertRaises(contract.InputContractError) as caught:
                contract.load_json_document(b'{"n":' + b"9" * digits + b"}")
            self.assertEqual(caught.exception.code, "budget-exceeded")

    def test_direct_objects_obey_aggregate_utf8_string_budget(self):
        request, bindings = fixture()
        for private in ("x" * 65537, "\U0001f512" * 16385, ["\u00e9" * 20000] * 2):
            candidate = copy.deepcopy(request)
            candidate["private"] = private
            self.failure(contract.validate_outcome_request, candidate, bindings, "budget-exceeded")

    def test_depth_collection_and_node_budgets_have_independent_boundaries(self):
        nested = 0
        for _ in range(16):
            nested = {"n": nested}
        self.assertEqual(contract.load_json_document(json.dumps(nested).encode()), nested)
        with self.assertRaises(contract.InputContractError) as caught:
            contract.load_json_document(json.dumps({"n": nested}).encode())
        self.assertEqual(caught.exception.code, "budget-exceeded")
        self.assertEqual(len(contract.load_json_document(json.dumps({"n": [0] * 256}).encode())["n"]), 256)
        for value in ({"n": [0] * 257}, {str(index): 0 for index in range(257)},
                      {"n": [[0] * 256 for _ in range(16)]}):
            with self.assertRaises(contract.InputContractError) as caught:
                contract.load_json_document(json.dumps(value).encode())
            self.assertEqual(caught.exception.code, "budget-exceeded")
        # Root, key, outer list, 16 inner lists and 4077 scalar nodes total 4096.
        exact_nodes = {"n": [[0] * 256 for _ in range(15)] + [[0] * 237]}
        self.assertEqual(contract.load_json_document(json.dumps(exact_nodes).encode()), exact_nodes)
        exact_nodes["n"][-1].append(0)
        with self.assertRaises(contract.InputContractError) as caught:
            contract.load_json_document(json.dumps(exact_nodes).encode())
        self.assertEqual(caught.exception.code, "budget-exceeded")

    def test_failures_never_expose_private_values_in_str_repr_or_traceback(self):
        canary = "private-path-label-key-credential-canary"
        request, bindings = fixture()
        mutations = [
            lambda value: value.update({"socket": "/" + canary}),
            lambda value: value["public_labels"][0].update({"key": canary}),
            lambda value: value["resource_decisions"][0].update({"source": canary}),
            lambda value: value.update({canary: canary}),
        ]
        for mutate in mutations:
            altered = copy.deepcopy(request)
            mutate(altered)
            try:
                contract.validate_outcome_request(altered, bindings)
            except contract.InputContractError as error:
                self.assertNotIn(canary, str(error))
                self.assertNotIn(canary, repr(error))
                self.assertNotIn(canary, "".join(traceback.format_exception(error)))
            else:
                self.fail("expected value-free failure")
        self.assertEqual(str(contract.InputContractError(canary)), "invalid-input")
        for raw in [('{"' + canary + '":1,"' + canary + '":2}').encode(),
                    ('{"' + canary + '":NaN}').encode(), ('{"' + canary + '":').encode()]:
            try:
                contract.load_json_document(raw)
            except contract.InputContractError as error:
                self.assertNotIn(canary, str(error))
                self.assertNotIn(canary, repr(error))
                self.assertNotIn(canary, "".join(traceback.format_exception(error)))
            else:
                self.fail("expected private JSON failure")

    def test_import_is_inert_and_has_no_cli_receipt_or_mutation_path(self):
        source = MODULE_PATH.read_text(encoding="utf-8")
        code = compile(source, str(MODULE_PATH), "exec")
        namespace = {"__name__": "nextcloud_input_contract_inert"}
        with mock.patch("builtins.open", side_effect=AssertionError("unexpected file IO")), \
             mock.patch.object(pathlib.Path, "read_bytes", side_effect=AssertionError("unexpected read")), \
             mock.patch.object(pathlib.Path, "read_text", side_effect=AssertionError("unexpected read")), \
             mock.patch.object(pathlib.Path, "write_bytes", side_effect=AssertionError("unexpected write")), \
             mock.patch.object(pathlib.Path, "write_text", side_effect=AssertionError("unexpected write")), \
             mock.patch.object(os, "open", side_effect=AssertionError("unexpected file IO")), \
             mock.patch.object(os, "system", side_effect=AssertionError("unexpected execution")), \
             mock.patch.object(subprocess, "Popen", side_effect=AssertionError("unexpected subprocess")), \
             mock.patch.object(socket, "socket", side_effect=AssertionError("unexpected socket")), \
             mock.patch("sys.stdout", new_callable=io.StringIO) as output, \
             mock.patch("sys.stderr", new_callable=io.StringIO) as errors:
            for name in ("nextcloud_input_contract_inert", "__main__"):
                namespace = {"__name__": name}
                exec(code, namespace)
                request, bindings = fixture()
                loaded = namespace["load_json_document"](json.dumps(request).encode())
                self.assertIsNone(namespace["validate_outcome_request"](loaded, bindings))
                self.assertIsNone(namespace["validate_outcome_request"](request, bindings))
                self.assertIsNone(namespace["validate_selection_request"](selection(bindings), bindings))
                with self.assertRaises(namespace["InputContractError"]):
                    namespace["load_json_document"](b'{"private":NaN}')
                request["include_environment_values"] = False
                with self.assertRaises(namespace["InputContractError"]):
                    namespace["validate_outcome_request"](request, bindings)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(errors.getvalue(), "")
        self.assertNotIn("main", namespace)
        self.assertNotIn("receipt", namespace)


if __name__ == "__main__":
    unittest.main()
