#!/usr/bin/env python3
"""Independent native projection and application export privacy regressions."""

import copy
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "projection", ROOT / "fixtures/conformance/podman-live/canonicalize_compose.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

SOURCE = """---
services:
  live-large-api:
    image: example.invalid/api:1
    networks:
      live-large-edge:
        aliases:
          - public-api
      live-large-private:
        aliases:
          - api
  unrelated:
    networks:
      other:
        aliases:
          - retained
"""
EXPECTED = """---
services:
  live-large-api:
    image: example.invalid/api:1
    networks:
      live-large-edge: {}
      live-large-private: {}
  unrelated:
    networks:
      other:
        aliases:
          - retained
"""
REPORT = {
    "status": "success",
    "diagnostics": [{
        "code": "BFQ0003",
        "severity": "warning",
        "fields": [
            {"name": "subject", "value": "services.live-large-api.networks"},
            {"name": "reason", "value": (
                "IP, IP6, and NetworkAlias require exactly one compatible network attachment"
            )},
        ],
    }],
}


class ReviewedAliasLossTests(unittest.TestCase):
    def test_only_two_reviewed_aliases_are_removed(self):
        self.assertEqual(MODULE.reviewed_quadlet_alias_loss(SOURCE, "live", REPORT), EXPECTED)

    def test_missing_wrong_or_duplicate_diagnostic_is_rejected(self):
        mutations = [
            {"status": "success", "diagnostics": []},
            {**REPORT, "status": "failure"},
            {**REPORT, "diagnostics": REPORT["diagnostics"] * 2},
        ]
        for key, value in [("code", "BFQ0002"), ("severity", "info")]:
            changed = copy.deepcopy(REPORT)
            changed["diagnostics"][0][key] = value
            mutations.append(changed)
        for index in (0, 1):
            changed = copy.deepcopy(REPORT)
            changed["diagnostics"][0]["fields"][index]["value"] = "unreviewed"
            mutations.append(changed)
        for report in mutations:
            with self.subTest(report=report), self.assertRaises(ValueError):
                MODULE.reviewed_quadlet_alias_loss(SOURCE, "live", report)

    def test_missing_or_changed_alias_or_service_is_rejected(self):
        for source in [
            SOURCE.replace("- api\n", "- changed\n"),
            SOURCE.replace("          - public-api\n", ""),
            SOURCE.replace("live-large-api:", "other-api:"),
            SOURCE.replace("live-large-private:", "other-private:"),
        ]:
            with self.subTest(source=source), self.assertRaises(ValueError):
                MODULE.reviewed_quadlet_alias_loss(source, "live", REPORT)

    def test_unreviewed_semantic_changes_are_not_normalized(self):
        for before, after in [
            ("api:1", "api:2"),
            ("- retained", "- unexpected"),
            ("          - api\n", "          - api\n          - additional\n"),
        ]:
            changed = SOURCE.replace(before, after)
            self.assertNotEqual(MODULE.reviewed_quadlet_alias_loss(changed, "live", REPORT), EXPECTED)


class ReviewedRestartLossTests(unittest.TestCase):
    def setUp(self):
        self.source = "---\nservices:\n  live-options:\n    restart: on-failure:3\n    image: example.invalid/options:1\n"
        self.report = copy.deepcopy(REPORT)
        self.report["diagnostics"][0]["fields"] = [
            {"name": "subject", "value": "services.live-options.restart_policy"},
            {"name": "reason", "value": (
                "a finite container restart count has no equivalent in Restart=; "
                "systemd start-rate limits use different time-window semantics"
            )},
        ]

    def test_exact_diagnosed_restart_loss_only(self):
        expected = "---\nservices:\n  live-options:\n    image: example.invalid/options:1\n"
        self.assertEqual(MODULE.reviewed_quadlet_restart_loss(self.source, "live", self.report), expected)
        self.assertEqual(MODULE.reviewed_quadlet_restart_loss(SOURCE, "live", {}), SOURCE)

    def test_missing_diagnostic_or_changed_value_is_rejected(self):
        with self.assertRaises(ValueError):
            MODULE.reviewed_quadlet_restart_loss(self.source, "live", REPORT)
        with self.assertRaises(ValueError):
            MODULE.reviewed_quadlet_restart_loss(self.source.replace("failure:3", "failure:4"), "live", self.report)

    def test_other_service_restart_is_unchanged(self):
        source = self.source.replace("live-options:", "other-options:")
        self.assertEqual(MODULE.reviewed_quadlet_restart_loss(source, "live", self.report), source)


class ApplicationExportPrivacyTests(unittest.TestCase):
    INCLUDE = '[[ "${output}" != podman ]] && target_arguments+=(--environment-values include)'

    def run_exports(self, application, mutation=None):
        helper = ROOT / "scripts/lib" / f"{application}-application.sh"
        source = helper.read_text(encoding="utf-8")
        function = re.search(
            rf"\n{application}_run_exports\(\) \{{.*?\n\}}", source, re.DOTALL
        ).group(0)
        self.assertIn(self.INCLUDE, function)
        if mutation is not None:
            function = function.replace(self.INCLUDE, mutation)
        shell = r'''
set -euo pipefail
repository_root=$1
current_case=$2
application=$3
source "$4"
eval "$5"
for assertion in assert_output_membership assert_output_semantics assert_success_contract assert_direct_export_environment; do
  eval "${application}_${assertion}() { :; }"
done
assert_successful_conversion() { :; }
boxferry_operation() {
  local output=$4 includes=0
  while (( $# )); do
    if [[ "$1" == --environment-values ]]; then
      [[ "${2:-}" == include ]] || return 95
      includes=$((includes + 1))
    fi
    shift
  done
  if [[ "$output" == podman ]]; then
    [[ "$includes" == 0 ]] || return 96
  else
    [[ "$includes" == 1 ]] || return 97
  fi
  printf '%s\n' "$output" >> "${current_case}/calls"
  printf '%s\n' '{"schema_version":1,"status":"success","exit_category":"success","diagnostics":[],"fidelity":{"invalid":0},"output_artifacts":[{"name":"stub"}]}'
}
"${application}_run_exports" cli /nonexistent-test-socket contract
'''
        with tempfile.TemporaryDirectory() as temporary:
            result = subprocess.run(
                ["bash", "-c", shell, "export-privacy-test", str(ROOT), temporary,
                 application, str(helper), function],
                capture_output=True, text=True, timeout=10, check=False,
            )
            calls = Path(temporary) / "calls"
            return result, calls.read_text().splitlines() if calls.exists() else []

    def test_every_selection_explicitly_includes_only_safe_output_values(self):
        for application in ("paperless", "immich", "supabase"):
            with self.subTest(application=application):
                result, calls = self.run_exports(application)
                self.assertEqual(result.returncode, 0, result.stderr)
                selections = 4 if application == "supabase" else 3
                self.assertEqual(calls, ["compose", "quadlet", "podman"] * selections)

    def test_missing_opt_in_or_including_podman_values_is_rejected(self):
        for application in ("paperless", "immich", "supabase"):
            for mutation in (":", "target_arguments+=(--environment-values include)"):
                with self.subTest(application=application, mutation=mutation):
                    result, _ = self.run_exports(application, mutation)
                    self.assertNotEqual(result.returncode, 0)


class SupabaseEnvironmentArtifactTests(unittest.TestCase):
    PASSWORD = "boxferry-public-supabase-db-password"

    def check_artifact(self, output, text):
        filenames = {"compose": "compose.yaml", "quadlet": "contract-supabase-db.container",
                     "podman": "podman.json"}
        with tempfile.TemporaryDirectory() as temporary:
            Path(temporary, filenames[output]).write_text(text, encoding="utf-8")
            result = subprocess.run([
                "bash", "-c",
                'set -euo pipefail; source "$1"; supabase_assert_direct_export_environment "$3" "$2" contract',
                "artifact-privacy-test", str(ROOT / "scripts/lib/supabase-application.sh"),
                temporary, output,
            ], capture_output=True, text=True, timeout=5, check=False)
            return result.returncode

    def test_compose_value_must_belong_to_database_environment(self):
        source = ("---\nservices:\n  contract-supabase-db:\n    environment:\n"
                  f"      - POSTGRES_PASSWORD={self.PASSWORD}\n")
        self.assertEqual(self.check_artifact("compose", source), 0)
        for before, after in [(self.PASSWORD, "withheld"), ("-db:", "-other:"),
                              ("environment:", "labels:"), ("POSTGRES_PASSWORD", "OTHER")]:
            self.assertNotEqual(self.check_artifact("compose", source.replace(before, after)), 0)

    def test_quadlet_value_must_belong_to_database_container_section(self):
        source = f"[Container]\nEnvironment=POSTGRES_PASSWORD={self.PASSWORD}\n"
        self.assertEqual(self.check_artifact("quadlet", source), 0)
        for before, after in [(self.PASSWORD, "withheld"), ("[Container]", "[Service]"),
                              ("POSTGRES_PASSWORD", "OTHER")]:
            self.assertNotEqual(self.check_artifact("quadlet", source.replace(before, after)), 0)

    def test_podman_plan_must_withhold_both_renderings_and_known_value(self):
        plan = {"operations": [{"action": "create",
                 "resource": {"kind": "container", "name": "contract-supabase-db"},
                 "cli": {"argv": ["create", "database-image"]},
                 "libpod": {"body": {"json": {}}}}]}
        self.assertEqual(self.check_artifact("podman", json.dumps(plan)), 0)
        mutations = []
        for arguments in (["--env", "POSTGRES_PASSWORD=unexpected"],
                          ["--env=POSTGRES_PASSWORD=unexpected"],
                          ["-ePOSTGRES_PASSWORD=unexpected"],
                          ["--env", "POSTGRES_PASSWORD"]):
            cli = copy.deepcopy(plan)
            cli["operations"][0]["cli"]["argv"] += arguments
            mutations.append(cli)
        for value in ("unexpected", "", None):
            api = copy.deepcopy(plan)
            api["operations"][0]["libpod"]["body"]["json"]["env"] = {"POSTGRES_PASSWORD": value}
            mutations.append(api)
        mutations += [{"operations": []}, {**plan, "unexpected": self.PASSWORD}]
        for mutation in mutations:
            self.assertNotEqual(self.check_artifact("podman", json.dumps(mutation)), 0)


if __name__ == "__main__":
    unittest.main()
