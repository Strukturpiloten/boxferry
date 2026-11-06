"""Shared-policy composition examples; native extraction is validated separately.

Consumers can call ``consumer_config`` and ``effective`` for their own local
rules. This intentionally models only the matchers listed below, and rejects
unknown matchers instead of claiming complete Renovate emulation.
"""

from __future__ import annotations

import argparse
import copy
import fnmatch
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from boxferry_ci.devcontainer_updates import limited_config

ROOT = Path(__file__).resolve().parents[2]
PRESETS = ROOT / "renovate-presets"
SHA = "a" * 40


def _merge(before: dict[str, Any], after: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(before)
    for key, value in after.items():
        if key == "extends":
            continue
        if key in {"packageRules", "customManagers"}:
            result[key] = [*result.get(key, []), *copy.deepcopy(value)]
        elif isinstance(value, dict) and isinstance(result.get(key), dict):
            result[key] = _merge(result[key], value)
        else:
            result[key] = copy.deepcopy(value)
    return result


def consumer_config(path: Path) -> dict[str, Any]:
    """Compose shared files before consumer additions, preserving rule order."""
    local = json.loads(path.read_text())
    result: dict[str, Any] = {}
    for reference in local.get("extends", []):
        match = re.fullmatch(
            r"github>Strukturpiloten/boxferry//renovate-presets/(common|rust)#([a-f0-9]{40})",
            reference,
        )
        if not match:
            if reference == "config:best-practices":
                continue  # Native Renovate checks this external preset.
            raise AssertionError(f"unsupported or mutable preset reference: {reference}")
        result = _merge(result, json.loads((PRESETS / f"{match[1]}.json").read_text()))
    return _merge(result, local)


def _matches(value: str, patterns: list[str], *, file: bool = False) -> bool:
    def one(pattern: str) -> bool:
        if pattern.startswith("/") and pattern.endswith("/"):
            return bool(re.search(pattern[1:-1], value))
        return fnmatch.fnmatchcase(value, pattern) if file else value == pattern

    negative = [p[1:] for p in patterns if p.startswith("!")]
    positive = [p for p in patterns if not p.startswith("!")]
    return not any(one(p) for p in negative) and (not positive or any(one(p) for p in positive))


def effective(config: dict[str, Any], dependency: dict[str, str]) -> dict[str, Any]:
    """Evaluate the supported policy matchers for an independent dependency case."""
    result = {
        key: copy.deepcopy(value)
        for key, value in config.items()
        if key not in {"packageRules", "customManagers"}
    }
    fields = {
        "matchManagers": "manager",
        "matchDatasources": "datasource",
        "matchPackageNames": "packageName",
        "matchDepNames": "depName",
        "matchDepTypes": "depType",
        "matchUpdateTypes": "updateType",
        "matchFileNames": "fileName",
    }
    for rule in config.get("packageRules", []):
        unknown = {key for key in rule if key.startswith("match")} - fields.keys()
        if unknown:
            raise AssertionError(
                f"unsupported Renovate matcher in static examples: {sorted(unknown)}"
            )
        if all(
            _matches(dependency.get(fields[key], ""), value, file=key == "matchFileNames")
            for key, value in rule.items()
            if key in fields
        ):
            result = _merge(
                result, {key: value for key, value in rule.items() if key not in fields}
            )
    return result


def assert_automatic_policy(config: dict[str, Any]) -> None:
    """Regression contract for a consumer's effective local/shared rule order."""
    examples = [
        {
            "manager": "cargo",
            "datasource": "crate",
            "packageName": "serde",
            "updateType": "major",
        },
        {
            "manager": "rust-toolchain",
            "datasource": "rust-version",
            "packageName": "rust",
            "updateType": "major",
        },
        {
            "manager": "pep621",
            "datasource": "pypi",
            "packageName": "ruff",
            "fileName": "automation/pyproject.toml",
            "updateType": "major",
        },
        {
            "manager": "pyenv",
            "datasource": "github-tags",
            "packageName": "python/cpython",
            "fileName": "automation/.python-version",
            "updateType": "major",
        },
        {
            "manager": "custom.regex",
            "datasource": "github-runners",
            "packageName": "ubuntu",
            "fileName": ".github/workflows/ci.yml",
            "updateType": "major",
        },
        {
            "manager": "custom.regex",
            "datasource": "docker",
            "packageName": "example/image",
            "fileName": ".devcontainer/Dockerfile",
            "updateType": "major",
        },
        {
            "manager": "custom.regex",
            "datasource": "github-release-attachments",
            "packageName": "docker/compose",
            "fileName": "scripts/lib/compose-provider.sh",
            "updateType": "major",
        },
    ]
    for dependency in examples:
        result = effective(config, dependency)
        if (
            result.get("automerge") is not True
            or result.get("dependencyDashboardApproval") is not False
        ):
            raise AssertionError(f"routine human approval overrides shared policy: {dependency}")
    first_party = effective(
        config,
        {
            "manager": "cargo",
            "datasource": "crate",
            "packageName": "compose-lens",
            "updateType": "minor",
        },
    )
    has_rust = any(
        rule.get("groupName") == "First-party Rust releases"
        for rule in config.get("packageRules", [])
    )
    if has_rust and (
        first_party.get("groupName") != "First-party Rust releases"
        or first_party.get("minimumReleaseAge") != "0 days"
    ):
        raise AssertionError("late consumer rule breaks isolated first-party release policy")

    for dependency in (
        {
            "manager": "devcontainer",
            "depType": "feature",
            "fileName": ".devcontainer/devcontainer.json",
        },
        {
            "manager": "custom.regex",
            "datasource": "github-releases",
            "packageName": "cli/cli",
            "fileName": ".devcontainer/devcontainer.json",
        },
    ):
        if effective(config, dependency).get("enabled") is not False:
            raise AssertionError("hosted App must yield feature ownership to limited updater")


def _shared() -> dict[str, Any]:
    return _merge(
        json.loads((PRESETS / "common.json").read_text()),
        json.loads((PRESETS / "rust.json").read_text()),
    )


class PresetTests(unittest.TestCase):
    def test_automatic_policy_including_major_compiler_image_checksum(self) -> None:
        assert_automatic_policy(_shared())

    def test_common_queue_auto_and_pre_adoption_local_policy_remain_distinct(self) -> None:
        local = json.loads((ROOT / ".github/renovate.json").read_text())
        for config, expected in ((_shared(), "auto"), (local, "behind-base-branch")):
            result = effective(
                config,
                {
                    "manager": "cargo",
                    "datasource": "crate",
                    "packageName": "serde",
                    "updateType": "minor",
                },
            )
            self.assertEqual(result["rebaseWhen"], expected)
            self.assertEqual(result["minimumReleaseAge"], "3 days")

    def test_devcontainer_node_image_couples_source_without_changing_age_or_other_owners(
        self,
    ) -> None:
        config = _shared()
        rule = next(
            rule
            for rule in config["packageRules"]
            if rule.get("matchPackageNames") == ["docker.io/library/node"]
        )
        self.assertEqual(rule["matchManagers"], ["custom.regex"])
        self.assertEqual(rule["matchDatasources"], ["docker"])
        self.assertEqual(rule["matchFileNames"], [".devcontainer/Dockerfile"])
        self.assertNotIn("minimumReleaseAge", rule)
        self.assertFalse(rule["separateMajorMinor"])
        node = {
            "manager": "custom.regex",
            "datasource": "docker",
            "packageName": "docker.io/library/node",
            "fileName": ".devcontainer/Dockerfile",
        }
        for update in ("major", "minor", "patch", "digest"):
            result = effective(config, {**node, "updateType": update})
            self.assertEqual(result["groupName"], "Shared BoxFerry automation")
            self.assertEqual(result["minimumReleaseAge"], "3 days")
            self.assertEqual(result["minimumReleaseAgeBehaviour"], "timestamp-required")
            self.assertFalse(result["separateMajorMinor"])
        before = copy.deepcopy(config)
        before["packageRules"].remove(rule)
        counterexamples = (
            {**node, "packageName": "docker.io/library/node-extra"},
            {**node, "fileName": "Dockerfile"},
            {**node, "manager": "dockerfile"},
            {**node, "datasource": "node-version"},
            {
                "manager": "custom.regex",
                "datasource": "node-version",
                "packageName": "node",
                "fileName": ".github/workflows/ci.yml",
                "updateType": "major",
            },
            {"manager": "cargo", "datasource": "crate", "packageName": "compose-lens"},
            {
                "manager": "github-actions",
                "datasource": "github-digest",
                "packageName": "Strukturpiloten/boxferry",
            },
        )
        for dependency in counterexamples:
            with self.subTest(dependency=dependency):
                self.assertEqual(effective(config, dependency), effective(before, dependency))
        source = effective(config, counterexamples[-1])
        self.assertEqual(source["minimumReleaseAge"], "0 days")

    def test_ordinary_age_and_missing_timestamp_policy_are_explicit(self) -> None:
        config = _shared()
        result = effective(
            config,
            {
                "manager": "cargo",
                "datasource": "crate",
                "packageName": "serde",
                "updateType": "minor",
            },
        )
        self.assertEqual(result["minimumReleaseAge"], "3 days")
        self.assertEqual(result["minimumReleaseAgeBehaviour"], "timestamp-required")
        self.assertEqual(result["internalChecksFilter"], "strict")

    def test_exact_first_party_names_do_not_match_prefix_or_other_ecosystems(
        self,
    ) -> None:
        config = _shared()
        for dependency in (
            {
                "manager": "cargo",
                "datasource": "crate",
                "packageName": "compose-lens-malicious",
            },
            {"manager": "npm", "datasource": "npm", "packageName": "compose-lens"},
            {
                "manager": "cargo",
                "datasource": "github-tags",
                "packageName": "compose-lens",
            },
        ):
            self.assertEqual(effective(config, dependency)["minimumReleaseAge"], "3 days")

    def test_first_party_group_wins_ordinary_rust_group(self) -> None:
        result = effective(
            _shared(),
            {
                "manager": "cargo",
                "datasource": "crate",
                "packageName": "podman-lens",
                "updateType": "minor",
            },
        )
        self.assertEqual(result["groupName"], "First-party Rust releases")
        self.assertEqual(result["minimumReleaseAge"], "0 days")

    def test_lock_maintenance_uses_required_transitive_age_gate(self) -> None:
        result = effective(
            _shared(),
            {
                "manager": "cargo",
                "datasource": "crate",
                "packageName": "serde",
                "updateType": "lockFileMaintenance",
            },
        )
        self.assertEqual(result["minimumReleaseAge"], "0 days")
        self.assertNotEqual(result.get("groupName"), "First-party Rust releases")
        self.assertTrue(result["automerge"])

    def test_shared_preset_sha_has_one_active_updater_and_round_trips(self) -> None:
        config = _shared()
        reference = f"github>Strukturpiloten/boxferry//renovate-presets/common#{SHA}"
        managers = config["customManagers"]
        owners = []
        for manager in managers:
            for expression in manager["matchStrings"]:
                expression = re.sub(r"\(\?<([A-Za-z]+)>", r"(?P<\1>", expression)
                match = re.search(expression, reference)
                if match:
                    owners.append((manager, match.groupdict()))
        self.assertEqual(len(owners), 1)
        manager, groups = owners[0]
        self.assertEqual(manager["datasourceTemplate"], "github-digest")
        self.assertEqual(manager["currentValueTemplate"], "automation-v1")
        replacement = reference.replace(groups["currentDigest"], "b" * 40)
        self.assertEqual(
            replacement,
            "github>Strukturpiloten/boxferry//renovate-presets/common#" + "b" * 40,
        )
        native = effective(
            config,
            {
                "manager": "renovate-config",
                "datasource": "github-tags",
                "packageName": "Strukturpiloten/boxferry",
            },
        )
        self.assertFalse(native["enabled"])
        updater = effective(
            config,
            {
                "manager": "custom.regex",
                "datasource": "github-digest",
                "packageName": "Strukturpiloten/boxferry",
            },
        )
        self.assertEqual(updater["minimumReleaseAge"], "0 days")
        self.assertTrue(updater["automerge"])

    def test_all_shared_adoption_pins_extract_and_replace_once(self) -> None:
        config = _shared()
        examples = [
            (".github/automation.json", '{"revision": "' + SHA + '"}'),
            (".github/workflows/ci.yml", "    automation_sha: " + SHA + "\n"),
            (".github/workflows/ci.yml", '    automation-sha: "' + SHA + '"\n'),
            (".github/workflows/ci.yml", "    AUTOMATION_SHA: '" + SHA + "'\n"),
            (
                ".github/workflows/ci.yml",
                "    # renovate: datasource=github-digest "
                "depName=Strukturpiloten/boxferry currentValue=automation-v1\n    ref: "
                + SHA
                + "\n",
            ),
        ]
        for path, content in examples:
            with self.subTest(path=path, content=content):
                owners = []
                for manager in config["customManagers"]:
                    if not any(
                        re.search(pattern[1:-1], path) for pattern in manager["managerFilePatterns"]
                    ):
                        continue
                    for pattern in manager["matchStrings"]:
                        pattern = re.sub(r"\(\?<([A-Za-z]+)>", r"(?P<\1>", pattern)
                        for match in re.finditer(pattern, content):
                            owners.append((manager, match))
                self.assertEqual(len(owners), 1)
                manager, match = owners[0]
                replacement = match.group().replace(match["currentDigest"], "b" * 40)
                updated = content[: match.start()] + replacement + content[match.end() :]
                self.assertEqual(updated, content.replace(SHA, "b" * 40))

    def test_shared_native_workflow_refs_keep_atomic_group_after_actions_group(self) -> None:
        result = effective(
            _shared(),
            {
                "manager": "github-actions",
                "datasource": "github-digest",
                "packageName": "Strukturpiloten/boxferry",
                "depType": "action",
                "updateType": "digest",
            },
        )
        self.assertEqual(result["groupName"], "Shared BoxFerry automation")
        self.assertEqual(result["minimumReleaseAge"], "0 days")
        self.assertTrue(result["pinDigests"])

    def test_first_party_source_identities_are_exact(self) -> None:
        config = _shared()
        for name, age in (
            ("Strukturpiloten/compose-lens", "0 days"),
            ("Strukturpiloten/compose-lens-extra", "3 days"),
            ("someone/compose-lens", "3 days"),
        ):
            result = effective(
                config,
                {"manager": "custom.regex", "datasource": "github-tags", "packageName": name},
            )
            self.assertEqual(result["minimumReleaseAge"], age)

    def test_both_presets_share_the_same_distribution_group(self) -> None:
        manager = _shared()["customManagers"][0]
        pattern = re.sub(r"\(\?<([A-Za-z]+)>", r"(?P<\1>", manager["matchStrings"][0])
        for preset in ("common", "rust"):
            self.assertIsNotNone(
                re.search(
                    pattern,
                    f"github>Strukturpiloten/boxferry//renovate-presets/{preset}#{SHA}",
                )
            )

    def test_late_manual_or_first_party_override_is_detected(self) -> None:
        for rule in (
            {"matchManagers": ["cargo"], "automerge": False},
            {"matchManagers": ["cargo"], "dependencyDashboardApproval": True},
            {"matchManagers": ["cargo"], "minimumReleaseAge": "3 days"},
        ):
            config = _merge(_shared(), {"packageRules": [rule]})
            with self.assertRaises(AssertionError):
                assert_automatic_policy(config)

    def test_foundation_bootstrap_uv_owners_equal_shared_definitions(self) -> None:
        local = json.loads((ROOT / ".github/renovate.json").read_text())
        if any(
            ref.startswith("github>Strukturpiloten/boxferry//renovate-presets/common#")
            for ref in local.get("extends", [])
        ):
            return
        shared = _shared()
        for manager in shared["customManagers"]:
            if manager.get("depNameTemplate") in {"uv", "astral-sh/uv"}:
                self.assertEqual(local["customManagers"].count(manager), 1)
        for dependency in (
            {"manager": "pep621", "datasource": "pypi", "packageName": "ruff"},
            {"manager": "custom.regex", "datasource": "pypi", "packageName": "uv"},
            {
                "manager": "custom.regex",
                "datasource": "github-release-attachments",
                "packageName": "astral-sh/uv",
            },
        ):
            result = effective(local, dependency)
            self.assertEqual(result["groupName"], "Python automation")
            self.assertTrue(result["automerge"])
            self.assertFalse(result["dependencyDashboardApproval"])

    def test_timestamp_optional_requires_exact_operational_source_manager_and_path(self) -> None:
        cases = [
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "ghcr.io/example/tool",
                    "fileName": ".devcontainer/Dockerfile",
                },
                True,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "ghcr.io/renovatebot/renovate",
                    "fileName": "automation/renovate-runtime.json",
                },
                True,
            ),
            (
                {
                    "manager": "devcontainer",
                    "datasource": "docker",
                    "depType": "feature",
                    "packageName": "ghcr.io/devcontainers/features/github-cli",
                    "fileName": ".devcontainer/devcontainer.json",
                },
                False,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "quay.io/libpod/alpine",
                    "fileName": "scripts/podman-live-conformance.sh",
                },
                True,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "github-runners",
                    "packageName": "ubuntu",
                    "fileName": ".github/workflows/ci.yml",
                },
                True,
            ),
            (
                {
                    "manager": "github-actions",
                    "datasource": "github-runners",
                    "packageName": "ubuntu",
                    "fileName": ".github/workflows/ci.yml",
                },
                True,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "mcr.microsoft.com/devcontainers/rust",
                    "fileName": ".devcontainer/Dockerfile",
                },
                True,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "code.forgejo.org/forgejo/forgejo",
                    "fileName": "fixtures/conformance/forgejo-application/images.tsv",
                },
                False,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "quay.io/podman/stable",
                    "fileName": "scripts/native-runtime-pins.sh",
                },
                False,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "custom.fcos-qemu-x86_64-stable",
                    "packageName": "fcos",
                    "fileName": "scripts/kind-vm-image.json",
                },
                False,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "ghcr.io/example/tool",
                    "fileName": ".github/workflows/ci.yml",
                },
                False,
            ),
            (
                {
                    "manager": "devcontainer",
                    "datasource": "docker",
                    "depType": "image",
                    "packageName": "ghcr.io/example/tool",
                    "fileName": ".devcontainer/devcontainer.json",
                },
                False,
            ),
            (
                {
                    "manager": "github-actions",
                    "datasource": "docker",
                    "packageName": "ghcr.io/example/tool",
                    "fileName": ".github/workflows/ci.yml",
                },
                False,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "codeberg.org/forgejo/forgejo",
                    "fileName": "fixtures/conformance/forgejo-application/images.tsv",
                },
                True,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "mcr.microsoft.com/other/tool",
                    "fileName": ".devcontainer/Dockerfile",
                },
                False,
            ),
            (
                {
                    "manager": "custom.regex",
                    "datasource": "docker",
                    "packageName": "ghcr.io/devcontainers/features/github-cli",
                    "fileName": ".devcontainer/devcontainer-lock.json",
                },
                False,
            ),
            (
                {
                    "manager": "devcontainer",
                    "datasource": "docker",
                    "depType": "feature",
                    "packageName": "ghcr.io/other/feature",
                    "fileName": ".devcontainer/devcontainer.json",
                },
                False,
            ),
        ]
        for dependency, optional in cases:
            with self.subTest(dependency=dependency):
                actual = effective(_shared(), dependency)
                self.assertEqual(
                    actual["minimumReleaseAgeBehaviour"],
                    "timestamp-optional" if optional else "timestamp-required",
                )

    def test_json_files_have_interface_contract(self) -> None:
        for name in ("common", "rust"):
            config = json.loads((PRESETS / f"{name}.json").read_text())
            self.assertIn("interface v1", config["description"])
            self.assertIn("$schema", config)


def _validate_native_runtime(runtime: Path) -> None:
    """Require the already provisioned SDK; this test never acquires or skips it."""
    pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
    try:
        assert (runtime / "source-image.txt").read_text().strip() == pin
        package = json.loads((runtime / "package.json").read_text())
        assert package["version"] == pin.split(":", 1)[1].split("@", 1)[0]
        assert (runtime / "dist/modules/manager/github-actions/index.js").is_file()
    except (OSError, ValueError, KeyError, AssertionError) as error:
        raise AssertionError(
            "missing or corrupt pinned native Renovate SDK; provision before the complete gate"
        ) from error


class NativePresetTests(unittest.TestCase):
    _cached_native_report: dict[str, Any] | None = None

    def _native_report(self) -> dict[str, Any]:
        if self._cached_native_report is not None:
            return self._cached_native_report
        runtime = Path(
            os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate")
        ).resolve()
        _validate_native_runtime(runtime)
        script = r"""
(async () => {
  const fs = require("fs"),
    os = require("os"),
    path = require("path");
  const assert = require("assert/strict"),
    cp = require("child_process");
  const packageRoot = process.argv[1],
    root = process.argv[2];
  const base = packageRoot + "/dist/";
  assert.equal(require(base + "util/regex.js").regexEngineStatus.type, "available");
  await require(base + "logger/index.js").init();
  const { GlobalConfig } = require(base + "config/global.js");
  const { extractPackageFile: extract } = require(
    base + "modules/manager/custom/regex/index.js",
  );
  const { doAutoReplace } = require(
    base + "workers/repository/update/branch/auto-replace.js",
  );
  const { applyPackageRules } = require(base + "util/package-rules/index.js");
  const common = JSON.parse(fs.readFileSync(root + "/renovate-presets/common.json"));
  const rust = JSON.parse(fs.readFileSync(root + "/renovate-presets/rust.json"));
  const { resolveConfigPresets } = require(base + "config/presets/index.js");
  const { config: resolvedCommon } = await resolveConfigPresets(common);
  assert.equal(resolvedCommon.rebaseWhen, "auto");
  const shared = {
    ...resolvedCommon,
    packageRules: [...resolvedCommon.packageRules, ...rust.packageRules],
  };
  const old = "a".repeat(40),
    next = "b".repeat(40),
    digest = "a".repeat(64);
  const preset = '"github>Strukturpiloten/boxferry//renovate-presets/';
  const marker =
    "# renovate: datasource=github-digest " +
    "depName=Strukturpiloten/boxferry currentValue=automation-v1\n    ref: ";
  const uvMetadata = (arch) =>
    JSON.stringify(
      {
        filename: "uv-" + arch + "-unknown-linux-gnu.tar.gz",
        currentVersion: "1.2.3",
        sha256: digest,
      },
      null,
      2,
    );
  const temporary = fs.mkdtempSync(os.tmpdir() + "/boxferry-native-regression-");
  GlobalConfig.set({ localDir: temporary });
  let replacements = 0;
  try {
    const fixtures = [
      [".github/renovate.json", preset + "common#" + old + '"', 0, old, next],
      [".github/renovate.json", preset + "rust#" + old + '"', 0, old, next],
      [".github/automation.json", '{"revision":"' + old + '"}', 1, old, next],
      [".github/workflows/ci.yml", "    automation_sha: " + old, 2, old, next],
      [".github/workflows/ci.yml", marker + old, 3, old, next],
      ["automation/uv-artifacts.json", uvMetadata("x86_64"), 4, digest, "b".repeat(64)],
      [
        "automation/uv-artifacts.json",
        uvMetadata("aarch64"),
        4,
        digest,
        "b".repeat(64),
      ],
      [
        "automation/pyproject.toml",
        '[tool.uv]\nrequired-version = "==1.2.3"\n',
        5,
        "1.2.3",
        "1.2.4",
      ],
      [
        "automation/renovate-runtime.json",
        '{"image":"ghcr.io/renovatebot/renovate:1.2.3@sha256:' + digest + '"}',
        6,
        "sha256:" + digest,
        "sha256:" + "b".repeat(64),
      ],
    ];
    for (const [file, content, index, current, replacement] of fixtures) {
      const manager = common.customManagers[index],
        result = extract(content, file, manager);
      assert.equal(result.deps.length, 1, file);
      if (index === 4)
        assert.equal(result.deps[0].depType, JSON.parse(content).filename);
      const dep = result.deps[0],
        isDigest = Boolean(dep.currentDigest);
      const upgrade = {
        ...manager,
        ...dep,
        manager: "regex",
        depIndex: 0,
        packageFile: file,
        newValue: [4, 6].includes(index)
          ? "1.2.4"
          : isDigest
            ? dep.currentValue
            : replacement,
        ...(isDigest ? { newDigest: replacement } : {}),
      };
      const updated = await doAutoReplace(upgrade, content, false);
      const expected = content.replace(current, replacement);
      assert.equal(
        updated,
        index === 4
          ? expected.replace('"currentVersion": "1.2.3"', '"currentVersion": "1.2.4"')
          : index === 6
            ? expected.replace(":1.2.3@", ":1.2.4@")
            : expected,
      );
      replacements++;
    }
    const actions = require(base + "modules/manager/github-actions/index.js");
    // Node's producer literal has one native manager, even when YAML aliases reuse it.
    const producerConfig = JSON.parse(fs.readFileSync(root + "/.github/renovate.json"));
    const nodeCustom = (file, content) => (producerConfig.customManagers || []).flatMap(manager =>
      manager.managerFilePatterns.some(pattern => new RegExp(pattern.slice(1,-1)).test(file))
        ? (extract(content,file,manager)?.deps || []).filter(dep => dep.depName === "node") : []);
    const producerFile = ".github/workflows/ci.yml";
    const producerText = fs.readFileSync(root + "/" + producerFile,"utf8");
    const literalVersions = [...producerText.matchAll(
      /^\s+node-version: ([0-9]+\.[0-9]+\.[0-9]+)$/gm)];
    assert.equal(literalVersions.length,1,"one physical independent producer authority");
    const nodeVersion = literalVersions[0][1];
    const producerDiscovery = await actions.extractPackageFile(producerText,producerFile,{});
    const producerNodes = producerDiscovery.deps.filter(dep => dep.depName === "node");
    assert.ok(producerNodes.length >= 1,"native Node owner must remain");
    assert.ok(producerNodes.every(dep => dep.currentValue === nodeVersion &&
      dep.datasource === "github-releases" &&
      dep.packageName === "actions/node-versions" && !dep.skipReason));
    assert.equal(nodeCustom(producerFile,producerText).length,0,"no duplicate custom Node owner");
    const replacedNode = await doAutoReplace({...producerNodes[0],manager:"github-actions",
      packageFile:producerFile,depIndex:producerDiscovery.deps.indexOf(producerNodes[0]),
      newValue:"99.7.3"},producerText,false);
    assert.equal(replacedNode,producerText.replace("node-version: "+nodeVersion,
      "node-version: 99.7.3"),"native replacement changes only the physical authority");
    const legacyNode = "---\njobs:\n  fixture:\n    steps:\n      - uses: " +
      "actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0\n" +
      "        with:\n          # renovate: datasource=node-version depName=node\n" +
      "          node-version: " + nodeVersion + "\n";
    for(const oldFile of [producerFile,".github/workflows/shared-devcontainer-updates.yml"]) {
      const legacyNative = await actions.extractPackageFile(legacyNode,oldFile,{});
      assert.equal(legacyNative.deps.filter(
        dep => dep.depName === "node" && !dep.skipReason).length,1);
      assert.equal(nodeCustom(oldFile,legacyNode).length,1,
        "both former annotations duplicated native ownership");
    }
        // Independent decoded YAML-node inventory prevents native extraction from silently
    // omitting an orphan input. The SDK must discover each physical value independently.
    const packageRequire = require("module").createRequire(
      packageRoot + "/package.json",
    );
    const YAML = packageRequire("yaml");
    const { config: effectiveProducer } =
      await resolveConfigPresets(producerConfig);
    const matrix = {
      positive: [],
      partial: [],
      competing: [],
      scalar: [],
      production: [],
    };
    const matchFiles = require(base + "util/string-match.js").matchRegexOrGlobList;
    const nodeRecord = (dep) =>
      dep.depName === "node" &&
      dep.datasource === "github-releases" &&
      dep.packageName === "actions/node-versions" &&
      !dep.skipReason;
    async function validateNodeOwners(
      text,
      file,
      {
        required = false,
        config = effectiveProducer,
        sdk = actions.extractPackageFile,
      } = {},
    ) {
      const document = YAML.parseDocument(text);
      assert.equal(document.errors.length, 0, "YAML parser errors");
      assert.equal(document.warnings.length, 0, "unsupported YAML parser warnings");
      const fields = [];
      function inventory(node, keys = []) {
        if (YAML.isMap(node))
          for (const pair of node.items) {
            assert.ok(
              YAML.isScalar(pair.key),
              "unsupported non-scalar mapping key",
            );
            const key = pair.key.value;
            if (key === "with" && keys.at(-2) === "steps")
              assert.ok(YAML.isMap(pair.value), "step inputs must be a mapping");
            const interfaceDeclaration =
              keys.length === 1 &&
              ["inputs", "outputs"].includes(keys[0]) &&
              /\/action\.ya?ml$/.test(file);
            if (key === "node-version" && !interfaceDeclaration)
              fields.push({ keys: [...keys, key], value: pair.value });
            inventory(pair.value, [...keys, key]);
          }
        else if (YAML.isSeq(node))
          node.items.forEach((item, index) => inventory(item, [...keys, index]));
        // Aliases refer to their original physical declaration; scalars are never mappings.
      }
      inventory(document.contents);
      assert.ok(
        !required || fields.length > 0,
        "missing physical Node input owner",
      );
      const result = await sdk(text, file, {});
      const deps = result?.deps || [];
      const nativeRows = deps.filter(nodeRecord);
      const configuredManagers = config.enabledManagers;
      assert.ok(
        !fields.length ||
          !configuredManagers ||
          configuredManagers.includes("github-actions"),
        "native manager disabled",
      );
      const nativeManager = config["github-actions"] || {};
      assert.ok(!fields.length || nativeManager.enabled !== false,
        "native manager config disabled");
      assert.ok(!fields.length || matchFiles(file, nativeManager.managerFilePatterns ||
        actions.defaultConfig.managerFilePatterns), "native manager file discovery excluded owner");
      const customRows = [];
      for (const manager of config.customManagers || []) {
        if (!matchFiles(file, manager.managerFilePatterns)) continue;
        for (const dep of extract(text, file, manager)?.deps || []) {
          if (!(dep.replaceString || "").includes("node-version")) continue;
          const effective = await applyPackageRules(
            { ...config, ...manager, ...dep, manager: "regex", packageFile: file },
            config.packageRules || [],
          );
          if (effective.enabled !== false) customRows.push(dep);
        }
      }
      assert.equal(
        customRows.length,
        0,
        "competing effective regex owner for Node input",
      );
      // Validate every physical owner/SDK record before any replacement side effect.
      for (const field of fields) {
        assert.ok(YAML.isScalar(field.value), "Node input must be scalar");
        const keys = field.keys;
        assert.equal(keys.at(-2), "with", "detached Node input");
        assert.equal(keys.at(-4), "steps", "Node input must belong to a real step");
        assert.equal(
          typeof keys.at(-3),
          "number",
          "Node input must belong to a step sequence",
        );
        const step = document.getIn(keys.slice(0, -2), true);
        assert.ok(YAML.isMap(step), "missing setup step");
        const use = step.get("uses", true);
        assert.ok(YAML.isScalar(use), "missing setup-node SDK action");
        assert.match(
          use.value,
          /^actions\/setup-node@[a-f0-9]{40}$/,
          "immutable setup-node reference",
        );
        assert.match(
          (use.comment || "").trim(),
          /^v\d+\.\d+(?:\.\d+)?$/,
          "exact setup-node tag",
        );
        assert.ok(
          deps.some(
            (dep) =>
              dep.depName === "actions/setup-node" &&
              dep.currentDigest === use.value.split("@")[1] &&
              dep.currentValue === use.comment.trim() &&
              !dep.skipReason,
          ),
          "missing setup-node SDK record",
        );
        const value = String(field.value.value);
        assert.ok(
          nativeRows.some((dep) => dep.currentValue === value),
          "missing native Node record",
        );
        for (const dep of nativeRows.filter((dep) => dep.currentValue === value)) {
          const effective = await applyPackageRules(
            { ...config, ...nativeManager, ...dep, manager: "github-actions", packageFile: file },
            config.packageRules || [],
          );
          assert.notEqual(
            effective.enabled,
            false,
            "native Node owner disabled by effective config",
          );
        }
      }
      for (const [index, field] of fields.entries()) {
        const changedVersion = "98.7." + (index + 1);
        const [start, end] = field.value.range;
        const challenge = text.slice(0, start) + changedVersion + text.slice(end);
        const discovery = await sdk(challenge, file, {});
        const challenged = discovery?.deps.find(
          (dep) => nodeRecord(dep) && dep.currentValue === changedVersion,
        );
        assert.ok(
          challenged,
          "SDK omitted independently changed physical Node input",
        );
        const replacement = await doAutoReplace(
          {
            ...challenged,
            manager: "github-actions",
            depIndex: discovery.deps.indexOf(challenged),
            packageFile: file,
            newValue: "99.7.3",
          },
          challenge,
          false,
        );
        assert.equal(
          replacement,
          text.slice(0, start) + "99.7.3" + text.slice(end),
          "native replacement must affect only independently changed physical input",
        );
      }
      assert.ok(!required || nativeRows.length > 0, "missing native Node owner");
      return { physical: fields.length, nativeRows: nativeRows.length };
    }
    const setup =
      "actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0";
    const wrapSteps = (steps) => "---\njobs:\n  fixture:\n    steps:\n" + steps;
    const validStep =
      "      - uses: " +
      setup +
      "\n        with:\n          node-version: 25.0.0\n";
    const valid = wrapSteps(validStep);
    const block = await validateNodeOwners(valid, producerFile, { required: true });
    assert.equal(block.physical, 1);
    matrix.positive.push({ name: "block", accepted: true, ...block });
    const positiveForms = [
      [
        "anchor alias",
        wrapSteps(
          validStep.replace("- uses:", "- &node\n        uses:") +
            "      - *node\n",
        ),
      ],
      ["quoted key", valid.replace("node-version:", '"node-version":')],
      ["escaped key", valid.replace("node-version:", '"node\\u002dversion":')],
      ["tagged key", valid.replace("node-version:", '!!str "node\\u002dversion":')],
      [
        "anchored key",
        valid.replace("node-version:", '&key "node\\u002dversion":'),
      ],
      [
        "flow input",
        valid.replace(
          "with:\n          node-version: 25.0.0",
          'with: {"node\\u002dversion": "25.0.0"}',
        ),
      ],
    ];
    for (const [name, text] of positiveForms) {
      // These representations are independently observed supported by the pinned SDK.
      matrix.positive.push({
        name,
        accepted: true,
        ...(await validateNodeOwners(text, producerFile, { required: true })),
      });
    }
    assert.ok(
      matrix.positive.find((row) => row.name === "anchor alias").accepted,
      "current alias form must work",
    );
    const orphanSteps = [
      ["bad SDK ref", validStep.replace(setup, "actions/setup-node@example")],
      [
        "bad SHA",
        validStep.replace(
          "949feb2413d6458794dcd2491c4babbbce0c15c1",
          "a".repeat(39),
        ),
      ],
      ["missing exact tag", validStep.replace(" # v7.1.0", "")],
      [
        "foreign action",
        validStep.replace("actions/setup-node@", "foreign/setup-node@"),
      ],
      [
        "similar action",
        validStep.replace("actions/setup-node@", "actions/setup-node-extra@"),
      ],
      ["detached field", validStep.replace("with:", "env:")],
      [
        "missing action",
        validStep.replace("      - uses: " + setup, "      - name: orphan"),
      ],
      [
        "escaped detached key",
        validStep
          .replace("with:", "env:")
          .replace("node-version:", '!!str "node\\u002dversion":'),
      ],
      [
        "anchored detached key",
        validStep
          .replace("with:", "env:")
          .replace("node-version:", '&key "node\\u002dversion":'),
      ],
      [
        "quoted detached key",
        validStep
          .replace("with:", "env:")
          .replace("node-version:", '"node-version":'),
      ],
      [
        "input scalar",
        validStep.replace("node-version: 25.0.0", "node-version:25.0.0"),
      ],
    ];
    const partialCategories = {
      "missing owner": "missing physical Node input owner",
      "bad SDK ref": "immutable setup-node reference",
      "bad SHA": "immutable setup-node reference",
      "missing exact tag": "exact setup-node tag",
      "foreign action": "immutable setup-node reference",
      "similar action": "immutable setup-node reference",
      "detached field": "detached Node input",
      "missing action": "missing setup-node SDK action",
      "escaped detached key": "detached Node input",
      "anchored detached key": "detached Node input",
      "quoted detached key": "detached Node input",
      "input scalar": "step inputs must be a mapping",
      "missing SDK node": "missing native Node record",
      "missing SDK actions/setup-node": "missing setup-node SDK record",
    };
    const competingCategories = {
      "node-version/node": "competing effective regex owner for Node input",
      "github-tags/nodejs/node": "competing effective regex owner for Node input",
      "npm/other-package blank line": "competing effective regex owner for Node input",
      "node-version/node blank line": "competing effective regex owner for Node input",
      "disabled native manager": "native manager disabled",
      "disabled native manager config": "native manager config disabled",
      "excluded native manager file": "native manager file discovery excluded owner",
      "disabled effective native rule": "native Node owner disabled by effective config",
    };
    async function refused(group, name, text, options = {}) {
      const expectedCategory = group === "partial" ?
        partialCategories[name.replace(/ (alone|alongside valid owner)$/, "")] :
        group === "competing" ? competingCategories[name] : "missing physical Node input owner";
      assert.ok(expectedCategory, "negative case lacks an intended refusal category: " + name);
      let refusal;
      try {
        await validateNodeOwners(text, producerFile, {
          required: true,
          ...options,
        });
      } catch (error) {
        refusal = error.message;
      }
      assert.ok(refusal, "ownership unexpectedly accepted: " + name);
      const category = refusal.split("\n")[0];
      assert.equal(category, expectedCategory, "wrong refusal category for " + name);
      matrix[group].push({ name, expectedCategory, category, refusal });
    }
    await refused(
      "partial",
      "missing owner",
      wrapSteps("      - run: echo no-node\n"),
    );
    for (const [name, step] of orphanSteps)
      for (const alongside of [false, true])
        await refused(
          "partial",
          name + (alongside ? " alongside valid owner" : " alone"),
          wrapSteps((alongside ? validStep : "") + step),
        );
    for (const missing of ["node", "actions/setup-node"])
      for (const alongside of [false, true]) {
        const second = validStep
          .replace("25.0.0", "26.0.0")
          .replace("949feb2413d6458794dcd2491c4babbbce0c15c1", "b".repeat(40));
        await refused(
          "partial",
          "missing SDK " +
            missing +
            (alongside ? " alongside valid owner" : " alone"),
          wrapSteps((alongside ? validStep : "") + second),
          {
            sdk: async (...args) => {
              const result = await actions.extractPackageFile(...args);
              return {
                ...result,
                deps: result.deps.filter(
                  (dep) =>
                    !(
                      dep.depName === missing &&
                      (missing === "node"
                        ? dep.currentValue === "26.0.0"
                        : dep.currentDigest === "b".repeat(40))
                    ),
                ),
              };
            },
          },
        );
      }
    await assert.rejects(
      refused("partial", "missing SDK node alone", valid, {
        sdk: async () => { throw new Error("update-failure"); },
      }),
      /wrong refusal category for missing SDK node alone/,
      "an unrelated replacement/SDK failure must not satisfy an ownership refusal",
    );
    matrix.categoryGuardControl = {unrelatedFailureRejected: true};
    for (const [datasource, packageName, blank] of [
      ["node-version", "node", ""],
      ["github-tags", "nodejs/node", ""],
      ["npm", "other-package", "\n"],
      ["node-version", "node", "\n"],
    ])
      await refused(
        "competing",
        datasource + "/" + packageName + (blank ? " blank line" : ""),
        valid.replace(
          "          node-version:",
          "          # renovate: datasource=" +
            datasource +
            " depName=" +
            packageName +
            "\n" +
            blank +
            "          node-version:",
        ),
      );
    await refused("competing", "disabled native manager", valid, {
      config: { ...effectiveProducer, enabledManagers: ["custom.regex"] },
    });
    await refused("competing", "disabled native manager config", valid, {
      config: {...effectiveProducer, "github-actions": {enabled: false}},
    });
    await refused("competing", "excluded native manager file", valid, {
      config: {...effectiveProducer,
        "github-actions": {managerFilePatterns: ["never-matches.yml"]}},
    });
    await refused("competing", "disabled effective native rule", valid, {
      config: {
        ...effectiveProducer,
        packageRules: [
          ...(effectiveProducer.packageRules || []),
          {
            matchManagers: ["github-actions"],
            matchPackageNames: ["actions/node-versions"],
            enabled: false,
          },
        ],
      },
    });
    const fake =
      "          steps:\n            - uses: " +
      setup +
      "\n              with:\n                node-version: 25.0.0\n";
    for (const header of [
      "|",
      "&payload |",
      "!!str |",
      '"script',
      '&payload "script',
      '!!str "script',
      "'script",
    ]) {
      const quoted = header.includes("script");
      const text = wrapSteps(
        "      - run: " +
          header +
          "\n" +
          fake +
          (quoted
            ? header.includes('"')
              ? '          "\n'
              : "          '\n"
            : ""),
      );
      await refused("scalar", header + " scalar spoof", text);
      // Alongside a genuine owner, scalar text contributes no independent authority.
      const alongside = await validateNodeOwners(
        text.replace("    steps:\n", "    steps:\n" + validStep),
        producerFile,
        { required: true },
      );
      assert.equal(
        alongside.physical,
        1,
        "scalar text must not count as a second owner",
      );
      matrix.scalar.push({ name: header + " alongside valid owner", ...alongside });
    }
    for (const file of fs
      .readdirSync(root + "/.github/workflows")
      .filter((file) => /\.ya?ml$/.test(file))) {
      const name = ".github/workflows/" + file;
      matrix.production.push({
        file: name,
        ...(await validateNodeOwners(
          fs.readFileSync(root + "/" + name, "utf8"),
          name,
          { required: name === producerFile },
        )),
      });
    }
    const sharedNodeFile = ".github/actions/shared-node/action.yml";
    matrix.production.push({
      file: sharedNodeFile,
      ...(await validateNodeOwners(
        fs.readFileSync(root + "/" + sharedNodeFile, "utf8"),
        sharedNodeFile,
        { required: true },
      )),
    });
    matrix.oracle = {
      image: fs.readFileSync(packageRoot + "/source-image.txt", "utf8").trim(),
      version: JSON.parse(fs.readFileSync(packageRoot + "/package.json")).version,
      yamlVersion: packageRequire("yaml/package.json").version,
      provenance:
        "existing integrity-pinned cached Renovate SDK; public APIs only; " +
        "no oracle source copied",
      license: {
        renovate: JSON.parse(fs.readFileSync(packageRoot + "/package.json"))
          .license,
        yaml: packageRequire("yaml/package.json").license,
      },
      command:
        "node --experimental-vm-modules -e <repository-owned matrix> " +
        "<cached-sdk> <repository> <limited-config>",
      redistribution: "no oracle source redistributed",
    };
    const nodeActionFile = ".github/actions/shared-node/action.yml";
    const nodeAction = fs.readFileSync(root + "/" + nodeActionFile,"utf8");
    const nodeFileMatches = require(base + "util/string-match.js").matchRegexOrGlobList;
    assert.ok(nodeFileMatches(nodeActionFile,actions.defaultConfig.managerFilePatterns),
      "shared composite must remain in native manager file discovery");
    assert.equal((nodeAction.match(/uses: actions\/setup-node@/g)||[]).length,1);
    const sharedDiscovery = await actions.extractPackageFile(nodeAction,nodeActionFile,{});
    const dynamicNodes = sharedDiscovery.deps.filter(dep => dep.depName === "node");
    assert.ok(dynamicNodes.length >= 1,"dynamic Node input must be discovered");
    // github-actions preserves this expression; native lookup rejects its value before I/O.
    assert.ok(dynamicNodes.every(dep =>
      dep.currentValue === "${{ steps.version.outputs.node_version }}" && !dep.skipReason));
    const nodeVm = require("vm");
    const lookupFile = base + "workers/repository/process/lookup/index.js";
    const lookupRequire = require("module").createRequire(lookupFile);
    const datasourceIndex = lookupRequire.resolve("../../../../modules/datasource/index.js");
    const ioAttempts = [];
    const datasourceIo = ["getDigest","getPkgReleases","getRawPkgReleases"];
    const denied = label => (...args) => {
      ioAttempts.push(label);
      throw new Error("unexpected shared Node discovery I/O: " + label);
    };
    const guardedLookup = new nodeVm.SourceTextModule(fs.readFileSync(lookupFile,"utf8"),
      {identifier:lookupFile});
    const restored = [];
    for(const [module,names] of [
      [require("http"),["request","get"]], [require("https"),["request","get"]],
      [require("net"),["connect","createConnection"]],
      [require("net").Socket.prototype,["connect"]], [require("tls"),["connect"]],
      [require("dns"),["lookup","resolve","resolve4","resolve6"]],
      [require("dns").promises,["lookup","resolve","resolve4","resolve6"]],
      [require("dgram").Socket.prototype,["send","connect"]],
      [globalThis,["fetch","WebSocket"]],
    ]) for(const name of names) {
      if(typeof module[name] !== "function") continue;
      const original = module[name];
      module[name] = denied("transport."+name);
      restored.push(()=>{module[name]=original;});
    }
    require("module").syncBuiltinESMExports();
    let nodeLookupProof;
    try {
      await guardedLookup.link(specifier => {
        const resolved = lookupRequire.resolve(specifier);
        const actual = lookupRequire(specifier);
        const names = Object.keys(actual);
        return new nodeVm.SyntheticModule(names,function() {
          for(const name of names) this.setExport(name,
            resolved === datasourceIndex && datasourceIo.includes(name)
              ? denied("datasource."+name) : actual[name]);
        });
      });
      await guardedLookup.evaluate();
      const results = [];
      for(const row of dynamicNodes) {
        const result = await guardedLookup.namespace.lookupUpdates({...row,
          manager:"github-actions",packageFile:nodeActionFile});
        const unwrapped = await result.unwrap();
        assert.equal(unwrapped.ok,true);
        assert.equal(unwrapped.val.skipReason,"invalid-value");
        assert.deepEqual(unwrapped.val.updates,[]);
        assert.deepEqual(ioAttempts,[],"dynamic Node must cause no datasource/network I/O");
        results.push(unwrapped.val);
      }
      // A real valid-version lookup must reach the denied datasource: the guard is live.
      await guardedLookup.namespace.lookupUpdates({...dynamicNodes[0],currentValue:nodeVersion,
        manager:"github-actions",packageFile:nodeActionFile,updatePinnedDependencies:true});
      assert.ok(ioAttempts.some(value=>value.startsWith("datasource.")),
        "valid native lookup must exercise the datasource guard");
      nodeLookupProof = {extracted:dynamicNodes,results,blockedDatasourceMethods:datasourceIo,
        transportGuardControl:ioAttempts.slice()};
    } finally {
      for(const restore of restored.reverse()) restore();
      require("module").syncBuiltinESMExports();
    }
    const setupOwners = sharedDiscovery.deps.filter(dep => dep.depName === "actions/setup-node");
    assert.equal(setupOwners.length,1,"one native Action owner for this literal occurrence");
    assert.equal(setupOwners[0].currentDigest,"949feb2413d6458794dcd2491c4babbbce0c15c1");
    assert.equal(setupOwners[0].currentValue,"v7.1.0");
    assert.equal(nodeCustom(nodeActionFile,nodeAction).length,0);
    const updaterFile = ".github/workflows/shared-devcontainer-updates.yml";
    const updaterText = fs.readFileSync(root + "/" + updaterFile,"utf8");
    const updaterDiscovery = await actions.extractPackageFile(updaterText,updaterFile,{});
    assert.equal(updaterDiscovery.deps.filter(
      dep => dep.depName === "node" && !dep.skipReason).length,0);
    assert.equal(nodeCustom(updaterFile,updaterText).length,0);
    const workflow = await actions.extractPackageFile(
      "---\njobs:\n  shared:\n    uses: Strukturpiloten/boxferry/.github/workflows/rust-ci.yml@" +
        old +
        " # automation-v1\n",
      ".github/workflows/ci.yml",
      {},
    );
    assert.equal(workflow.deps.length, 1);
    assert.equal(workflow.deps[0].datasource, "github-digest");
    assert.equal(workflow.deps[0].currentDigest, old);
    const extraComment =
      "---\njobs:\n  shared:\n    uses: " +
      "Strukturpiloten/boxferry/.github/workflows/shared-dependency-eligibility.yml@" +
      old +
      " # automation-v1 interface v1\n";
    const nativeRef = await actions.extractPackageFile(
      extraComment,
      ".github/workflows/ci.yml",
      {},
    );
    assert.equal(nativeRef.deps[0].datasource, "github-digest");
    assert.equal(nativeRef.deps[0].currentValue, "automation-v1");
    const refUpdate = await doAutoReplace(
      {
        ...nativeRef.deps[0],
        manager: "github-actions",
        packageFile: ".github/workflows/ci.yml",
        depIndex: 0,
        newValue: "automation-v1",
        newDigest: next,
      },
      extraComment,
      false,
    );
    assert.equal(refUpdate, extraComment.replace(old, next));
    replacements++;
    const cases = [
      [
        { manager: "regex", datasource: "pypi", packageName: "uv" },
        "Python automation",
      ],
      [
        {
          manager: "regex",
          datasource: "github-release-attachments",
          packageName: "astral-sh/uv",
        },
        "Python automation",
      ],
      [
        { manager: "pep621", datasource: "pypi", packageName: "ruff" },
        "Python automation",
      ],
      [
        {
          manager: "cargo",
          datasource: "crate",
          packageName: "compose-lens",
          updateType: "minor",
        },
        "First-party Rust releases",
      ],
      [
        {
          manager: "github-actions",
          datasource: "github-digest",
          packageName: "Strukturpiloten/boxferry",
          updateType: "digest",
        },
        "Shared BoxFerry automation",
      ],
      [
        {
          manager: "regex",
          datasource: "docker",
          packageName: "ghcr.io/renovatebot/renovate",
          updateType: "major",
        },
        "Renovate native test runtime",
      ],
    ];
    for (const [dep, group] of cases) {
      const result = await applyPackageRules({
        ...shared,
        depName: dep.packageName,
        ...dep,
      });
      assert.equal(result.groupName, group);
      assert.equal(result.automerge, true);
      assert.equal(result.dependencyDashboardApproval, false);
      assert.equal(result.rebaseWhen, "auto");
    }
    const defaults = require(base+"config/defaults.js").getConfig();
    const { generateBranchName } = require(base+"workers/repository/updates/branch-name.js");
    const { generateBranchConfig } = require(base+"workers/repository/updates/generate.js");
    const sourceAdoption = await applyPackageRules({...defaults,...shared,
      manager:"regex",datasource:"github-digest",depName:"Strukturpiloten/boxferry",
      packageName:"Strukturpiloten/boxferry",packageFile:".github/automation.json",
      updateType:"digest",currentValue:"automation-v1",newValue:"automation-v1",
      currentDigest:old,newDigest:next,isDigest:true});
    const nodeImage = await applyPackageRules({...defaults,...shared,
      manager:"regex",datasource:"docker",depName:"docker.io/library/node",
      packageName:"docker.io/library/node",packageFile:".devcontainer/Dockerfile",
      updateType:"major",currentValue:"24.0.0",newValue:"25.0.0",newVersion:"25.0.0",newMajor:25,
      currentDigest:"sha256:"+digest,newDigest:"sha256:"+"b".repeat(64)});
    assert.equal(nodeImage.minimumReleaseAge,"3 days");
    assert.equal(nodeImage.minimumReleaseAgeBehaviour,"timestamp-required");
    assert.equal(nodeImage.separateMajorMinor,false);
    assert.equal(sourceAdoption.minimumReleaseAge,"0 days");
    generateBranchName(sourceAdoption);
    generateBranchName(nodeImage);
    assert.equal(nodeImage.branchName,sourceAdoption.branchName);
    const coupledBranch=generateBranchConfig([sourceAdoption,nodeImage]);
    assert.equal(coupledBranch.upgrades.length,2);
    assert.equal(coupledBranch.groupName,"Shared BoxFerry automation");
    const waitingBranch=generateBranchConfig([{...sourceAdoption,pendingChecks:false},
      {...nodeImage,pendingChecks:true}]);
    assert.equal(waitingBranch.upgrades.length,1);
    assert.equal(waitingBranch.upgrades[0].depName,"Strukturpiloten/boxferry");
    assert.equal(nodeImage.minimumReleaseAge,"3 days");
    const agedNode={...nodeImage,pendingChecks:false};
    generateBranchName(agedNode);
    assert.equal(agedNode.branchName,sourceAdoption.branchName);
    const readyBranch=generateBranchConfig([{...sourceAdoption,pendingChecks:false},agedNode]);
    assert.equal(readyBranch.upgrades.length,2);
    const ordinaryCrate=await applyPackageRules({...defaults,...shared,manager:"cargo",
      datasource:"crate",depName:"compose-lens",packageName:"compose-lens",updateType:"minor"});
    assert.equal(ordinaryCrate.groupName,"First-party Rust releases");
    assert.equal(ordinaryCrate.minimumReleaseAge,"0 days");
    for(const change of [{packageName:"docker.io/library/alpine"},
      {packageFile:"other/Dockerfile"},{manager:"dockerfile"},
      {datasource:"node-version",packageName:"node",packageFile:".github/workflows/ci.yml"}]) {
      const unrelated=await applyPackageRules({...defaults,...shared,manager:"regex",
        datasource:"docker",packageName:"docker.io/library/node",depName:"docker.io/library/node",
        packageFile:".devcontainer/Dockerfile",updateType:"major",...change});
      assert.notEqual(unrelated.groupName,"Shared BoxFerry automation");
      assert.equal(unrelated.minimumReleaseAge,"3 days");
    }
    const featureName = "ghcr.io/devcontainers/features/github-cli";
    const featurePath = ".devcontainer/devcontainer.json";
    const featureLockPath = ".devcontainer/devcontainer-lock.json";
    assert.equal(common.customManagers.some(manager =>
      manager.managerFilePatterns.some(pattern => pattern.includes("devcontainer-lock"))), false);
    const devcontainer = require(base + "modules/manager/devcontainer/index.js");
    const { matchRegexOrGlobList } = require(base + "util/string-match.js");
    assert.equal(
      matchRegexOrGlobList(
        featureLockPath,
        devcontainer.defaultConfig.managerFilePatterns,
      ),
      false,
    );
    const declaration = JSON.stringify({
      features: { [featureName + ":1.1.0"]: { version: "2.103.2" } },
    });
    const declared = devcontainer.extractPackageFile(declaration, featurePath, {});
    assert.equal(declared.deps.length, 1);
    const declarationEffective = await applyPackageRules({
      ...shared,
      ...declared.deps[0],
      manager: "devcontainer",
      packageName: featureName,
      packageFile: featurePath,
    });
    assert.equal(declarationEffective.enabled, false);
    const declarationUpdated = await doAutoReplace(
      {
        ...declarationEffective,
        manager: "devcontainer",
        packageFile: featurePath,
        depIndex: 0,
        newValue: "1.2.0",
      },
      declaration,
      false,
    );
    assert.equal(
      JSON.parse(declarationUpdated).features[featureName + ":1.2.0"].version,
      "2.103.2",
    );
    const cliOption = await applyPackageRules({
      ...shared,
      manager: "regex",
      datasource: "github-releases",
      packageName: "cli/cli",
      depName: "cli/cli",
      packageFile: featurePath,
    });
    assert.equal(cliOption.enabled, false);
    assert.equal(cliOption.minimumReleaseAgeBehaviour, "timestamp-required");
    replacements += 2;
    // Execute the unmodified pinned ESM prune algorithm with no real platform writes.
    const vm = require("node:vm");
    const context = vm.createContext({});
    const removed = [], visited = [];
    const prefix = "renovate-devcontainer/";
    const stubs = {
      "../../../constants/error-messages.js": {},
      "../../../util/regex.js": {regEx: value => new RegExp(value)},
      "../../../config/global.js": {GlobalConfig: {get: () => undefined}},
      "../../../util/string.js": {uniqueStrings:
        (value,index,array) => array.indexOf(value)===index},
      "../../../logger/index.js": {logger: new Proxy({}, {get:()=>()=>{}})},
      "../../../util/git/index.js": {getBranchList:()=>
        [prefix+"features",prefix+"lock-file-maintenance","unrelated"],
        setUserRepoConfig:()=>{}},
      "../../../modules/platform/scm.js": {scm:{isBranchModified:async branch=>{
        visited.push(branch);return false;},
        deleteBranch:async branch=>removed.push(branch)}},
      "../../../modules/platform/index.js": {platform:{findPr:async()=>null}},
      "../../../modules/platform/comment.js": {ensureComment:()=>{
        throw Error("unexpected comment");}},
      "../process/index.js": {isMultiBaseBranch:()=>false},
      "../reconfigure/utils.js": {getReconfigureBranchName: value=>value+"reconfigure"},
      "@sindresorhus/is": {isNonEmptyStringAndNotWhitespace:
        value=>typeof value==="string"&&!!value.trim()},
    };
    const pruneModule = new vm.SourceTextModule(
      fs.readFileSync(base+"workers/repository/finalize/prune.js","utf8"),{context});
    await pruneModule.link(specifier=>{
      assert.ok(Object.hasOwn(stubs,specifier),"unreviewed native prune import "+specifier);
      const values=stubs[specifier];
      return new vm.SyntheticModule(Object.keys(values),function(){
        for(const [name,value] of Object.entries(values))this.setExport(name,value);
      },{context});
    });
    await pruneModule.evaluate();
    const pruneConfig={repoIsOnboarded:true,defaultBranch:"main",
      branchPrefix:prefix,pruneStaleBranches:true};
    await pruneModule.namespace.pruneStaleBranches(pruneConfig,[]);
    assert.deepEqual(removed,[prefix+"features"]);
    assert.deepEqual(visited,[prefix+"features"]);
    removed.length=0;visited.length=0;
    await pruneModule.namespace.pruneStaleBranches(pruneConfig,[prefix+"features"]);
    assert.deepEqual(removed,[]);
    assert.deepEqual(visited,[]);
    GlobalConfig.set({localDir:root});
    const npmExtractor = require(base+"modules/manager/npm/extract/index.js");
    const cliManifest = "automation/devcontainer/package.json";
    const cliPackage = await npmExtractor.extractPackageFile(
      fs.readFileSync(root+"/"+cliManifest,"utf8"),cliManifest,{});
    assert.equal(cliPackage.managerData.npmLock,"automation/devcontainer/package-lock.json");
    assert.equal(cliPackage.deps.filter(dep=>dep.depName==="@devcontainers/cli").length,1);
    for(const manager of common.customManagers) {
      assert.equal(extract(fs.readFileSync(root+"/"+cliManifest,"utf8"),cliManifest,manager),null);
    }
    GlobalConfig.set({localDir:temporary});
    const limited = JSON.parse(process.argv[3]);
    const optionManager = limited.customManagers[0];
    const option = extract(declaration,featurePath,optionManager);
    assert.equal(option.deps.length,1);
    assert.equal(option.deps[0].depName,"cli/cli");
    const optionUpdated = await doAutoReplace({...optionManager,...option.deps[0],
      manager:"regex",depIndex:0,packageFile:featurePath,newValue:"2.104.0"},declaration,false);
    assert.equal(JSON.parse(optionUpdated).features[featureName+":1.1.0"].version,"2.104.0");
    assert.equal(extract('{"version":"2.103.2"}',featurePath,optionManager),null);
    for(const [depType,enabled] of [["feature",true],["image",false],["node",false],
      ["go",false],["python",false],["ruby",false]]) {
      const actual = await applyPackageRules({...limited,manager:"devcontainer",depType,
        packageName:featureName,packageFile:featurePath});
      assert.equal(actual.enabled,enabled);
      assert.equal(actual.minimumReleaseAge,"3 days");
      assert.equal(actual.groupName,"Dev Container features");
    }
    const limitedOption = await applyPackageRules({...limited,...option.deps[0],manager:"regex",
      packageName:"cli/cli",packageFile:featurePath});
    assert.equal(limitedOption.minimumReleaseAge,"3 days");
    assert.equal(limitedOption.dependencyDashboardApproval,false);
    assert.equal(limitedOption.automerge,true);
    assert.equal(limitedOption.groupName,"Dev Container features");
    replacements++;
    // Runner policy uses the real native extractor; matrix-only discovery remains caller-owned.
    const runnerSemantics = {
      native: [],
      matrix: [],
      exclusions: [],
      scopeControls: [],
    };
    const runnerWorkflow = [
      "---",
      "jobs:",
      "  linux:",
      "    runs-on: ubuntu-24.04",
      "    steps:",
      "      - uses: " + setup,
      "        with:",
      "          node-version: " + nodeVersion,
      // Synthetic immutable Action identity: extraction fixture only, never downloaded/executed.
      "      - uses: actions/setup-python@" + old + " # v1.2.3",
      "        with:",
      "          python-version: 3.13.14",
      "  apple:",
      "    runs-on: macos-15",
      "    steps:",
      "      - run: true",
      "  microsoft:",
      "    runs-on: windows-2025",
      "    steps:",
      "      - run: true",
      "",
    ].join("\n");
    const runnerFile = ".github/workflows/runner-policy.yml";
    const runnerDiscovery = await actions.extractPackageFile(
      runnerWorkflow,
      runnerFile,
      {},
    );
    const nativeRunners = runnerDiscovery.deps.filter(
      (dep) => dep.datasource === "github-runners",
    );
    assert.deepEqual(nativeRunners.map((dep) => dep.depName).sort(), [
      "macos",
      "ubuntu",
      "windows",
    ]);
    assert.ok(
      nativeRunners.every(
        (dep) => dep.depType === "github-runner" && !dep.skipReason,
      ),
    );
    const runnerRule = shared.packageRules.filter((rule) =>
      rule.matchDatasources?.includes("github-runners"),
    );
    assert.equal(
      runnerRule.length,
      1,
      "one existing runner rule, not a duplicate policy",
    );
    assert.deepEqual(runnerRule[0].matchManagers, [
      "custom.regex",
      "github-actions",
    ]);
    const priorRunnerConfig = structuredClone(shared);
    priorRunnerConfig.packageRules.find((rule) =>
      rule.matchDatasources?.includes("github-runners"),
    ).matchManagers = ["custom.regex"];
    async function runnerPolicy(dep, manager, file, config = shared) {
      const result = await applyPackageRules({
        ...config,
        ...dep,
        manager,
        packageFile: file,
        packageName: dep.packageName || dep.depName,
      });
      return {
        enabled: result.enabled !== false,
        minimumReleaseAge: result.minimumReleaseAge,
        behaviour: result.minimumReleaseAgeBehaviour,
        approval: result.dependencyDashboardApproval,
        automerge: result.automerge,
      };
    }
    function requireRunnerPolicy(result) {
      assert.deepEqual(
        result,
        {
          enabled: true,
          minimumReleaseAge: "3 days",
          behaviour: "timestamp-optional",
          approval: false,
          automerge: true,
        },
        "native/custom runner selector retains operational age and quarantine policy",
      );
    }
    for (const file of [runnerFile, ".github/workflows/runner-policy.yaml"]) {
      for (const dep of nativeRunners) {
        const result = await runnerPolicy(dep, "github-actions", file);
        requireRunnerPolicy(result);
        runnerSemantics.native.push({
          file,
          depName: dep.depName,
          depType: dep.depType,
          datasource: dep.datasource,
          currentValue: dep.currentValue,
          result,
        });
      }
    }
    const actualProducerRunners = producerDiscovery.deps.filter(
      (dep) => dep.datasource === "github-runners",
    );
    assert.ok(
      actualProducerRunners.length > 0,
      "real producer CI native runner rows remain discoverable",
    );
    for (const dep of actualProducerRunners)
      requireRunnerPolicy(await runnerPolicy(dep, "github-actions", producerFile));
    runnerSemantics.actualProducerNativeRows = actualProducerRunners.length;
    const oldNative = await runnerPolicy(
      nativeRunners[0],
      "github-actions",
      runnerFile,
      priorRunnerConfig,
    );
    assert.equal(oldNative.behaviour, "timestamp-required");
    assert.throws(
      () => requireRunnerPolicy(oldNative),
      /runner selector retains operational age/,
    );
    runnerSemantics.preFixNativeControl = oldNative;
    const matrixWorkflow = [
      "---",
      "jobs:",
      "  fixture:",
      "    runs-on: ${{ matrix.runner }}",
      "    strategy:",
      "      matrix:",
      "        # renovate: datasource=github-runners",
      "        runner:",
      "          - ubuntu-24.04",
      "    steps:",
      "      - run: true",
      "",
    ].join("\n");
    const matrixNative = await actions.extractPackageFile(
      matrixWorkflow,
      runnerFile,
      {},
    );
    assert.equal(
      (matrixNative?.deps || []).filter(
        (dep) => dep.datasource === "github-runners",
      ).length,
      0,
      "pinned SDK leaves annotated matrix discovery to the caller custom extractor",
    );
    const matrixManager = {
      customType: "regex",
      datasourceTemplate: "github-runners",
      matchStrings: [
        "# renovate: datasource=github-runners\\n\\s+runner:\\n\\s+-\\s+" +
          "(?<depName>ubuntu)-(?<currentValue>[0-9]+\\.[0-9]+)",
      ],
    };
    const matrixRows =
      extract(matrixWorkflow, runnerFile, matrixManager)?.deps || [];
    assert.equal(matrixRows.length, 1);
    for (const dep of matrixRows) {
      const result = await runnerPolicy(dep, "regex", runnerFile);
      requireRunnerPolicy(result);
      runnerSemantics.matrix.push({
        depName: dep.depName,
        datasource: dep.datasource,
        currentValue: dep.currentValue,
        result,
      });
    }
    const canonicalRunnerRows = (shared.customManagers || [])
      .flatMap(
        (manager) =>
          extract(runnerWorkflow + matrixWorkflow, runnerFile, manager)?.deps || [],
      )
      .filter((dep) => dep.datasource === "github-runners");
    assert.equal(
      canonicalRunnerRows.length,
      0,
      "common must not add a runner extractor/inventory",
    );
    runnerSemantics.canonicalCustomRunnerRows = canonicalRunnerRows.length;
    const nativeControls = runnerDiscovery.deps.filter(
      (dep) =>
        dep.packageName === "actions/python-versions" ||
        dep.packageName === "actions/node-versions" ||
        (dep.datasource === "github-tags" && dep.currentDigest),
    );
    assert.equal(
      nativeControls.length,
      4,
      "native Action SHA, Python and Node controls must be observed",
    );
    for (const dep of nativeControls) {
      const result = await runnerPolicy(dep, "github-actions", runnerFile);
      assert.deepEqual(
        result,
        await runnerPolicy(dep, "github-actions", runnerFile, priorRunnerConfig),
      );
      assert.deepEqual(result, {
        enabled: true,
        minimumReleaseAge: "3 days",
        behaviour: "timestamp-required",
        approval: false,
        automerge: true,
      });
      runnerSemantics.exclusions.push({
        depName: dep.depName,
        packageName: dep.packageName,
        depType: dep.depType,
        datasource: dep.datasource,
        result,
      });
    }
    for (const [manager, dep, expectedAge] of [
      [
        "cargo",
        { depName: "podman-lens", packageName: "podman-lens", datasource: "crate" },
        "0 days",
      ],
      [
        "pep621",
        { depName: "ruff", packageName: "ruff", datasource: "pypi" },
        "3 days",
      ],
      [
        "regex",
        {
          depName: "astral-sh/uv",
          packageName: "astral-sh/uv",
          datasource: "github-release-attachments",
        },
        "3 days",
      ],
    ]) {
      const result = await runnerPolicy(dep, manager, runnerFile);
      assert.deepEqual(
        result,
        await runnerPolicy(dep, manager, runnerFile, priorRunnerConfig),
      );
      assert.equal(result.enabled, true);
      assert.equal(result.approval, false);
      assert.equal(result.minimumReleaseAge, expectedAge);
      runnerSemantics.exclusions.push({
        depName: dep.depName,
        datasource: dep.datasource,
        result,
      });
    }
    for (const [label, manager, dep, file] of [
      [
        "outside workflow path",
        "github-actions",
        nativeRunners[0],
        "fixtures/runner.yml",
      ],
      [
        "unrelated package",
        "github-actions",
        { ...nativeRunners[0], packageName: "self-hosted" },
        runnerFile,
      ],
      [
        "unrelated datasource",
        "github-actions",
        { ...nativeRunners[0], datasource: "github-tags" },
        runnerFile,
      ],
      ["unrelated manager", "npm", nativeRunners[0], runnerFile],
    ]) {
      const result = await runnerPolicy(dep, manager, file);
      assert.equal(result.behaviour, "timestamp-required", label);
      assert.equal(result.minimumReleaseAge, "3 days", label);
      runnerSemantics.scopeControls.push({ label, result });
    }
    const timestampCases = [
      [
        "regex",
        "docker",
        "ghcr.io/example/tool",
        ".devcontainer/Dockerfile",
        null,
        true,
      ],
      [
        "devcontainer",
        "docker",
        "ghcr.io/devcontainers/features/github-cli",
        ".devcontainer/devcontainer.json",
        "feature",
        false,
      ],
      [
        "regex",
        "docker",
        "quay.io/libpod/alpine",
        "scripts/podman-live-conformance.sh",
        null,
        true,
      ],
      ["regex", "github-runners", "ubuntu", ".github/workflows/ci.yml", null, true],
      [
        "regex",
        "docker",
        "mcr.microsoft.com/devcontainers/rust",
        ".devcontainer/Dockerfile",
        null,
        true,
      ],
      [
        "regex",
        "docker",
        "code.forgejo.org/forgejo/forgejo",
        "fixtures/conformance/forgejo-application/images.tsv",
        null,
        false,
      ],
      [
        "regex",
        "docker",
        "quay.io/podman/stable",
        "scripts/native-runtime-pins.sh",
        null,
        false,
      ],
      [
        "regex",
        "custom.fcos-qemu-x86_64-stable",
        "fcos",
        "scripts/kind-vm-image.json",
        null,
        false,
      ],
      [
        "regex",
        "docker",
        "ghcr.io/example/tool",
        ".github/workflows/ci.yml",
        null,
        false,
      ],
      [
        "devcontainer",
        "docker",
        "ghcr.io/example/tool",
        ".devcontainer/devcontainer.json",
        "image",
        false,
      ],
      [
        "github-actions",
        "docker",
        "ghcr.io/example/tool",
        ".github/workflows/ci.yml",
        null,
        false,
      ],
    ];
    timestampCases.push(
      [
        "regex",
        "docker",
        "codeberg.org/forgejo/forgejo",
        "fixtures/conformance/forgejo-application/images.tsv",
        null,
        true,
      ],
      [
        "regex",
        "docker",
        "mcr.microsoft.com/other/tool",
        ".devcontainer/Dockerfile",
        null,
        false,
      ],
      [
        "regex",
        "docker",
        "ghcr.io/devcontainers/features/github-cli",
        ".devcontainer/devcontainer-lock.json",
        null,
        false,
      ],
      [
        "devcontainer",
        "docker",
        "ghcr.io/other/feature",
        ".devcontainer/devcontainer.json",
        "feature",
        false,
      ],
    );
    for (const [
      manager,
      datasource,
      packageName,
      packageFile,
      depType,
      optional,
    ] of timestampCases) {
      const actual = await applyPackageRules({
        ...shared,
        manager,
        datasource,
        packageName,
        depName: packageName,
        packageFile,
        depType,
      });
      assert.equal(
        actual.minimumReleaseAgeBehaviour,
        optional ? "timestamp-optional" : "timestamp-required",
      );
    }
    GlobalConfig.set({ localDir: root });
    const local = JSON.parse(fs.readFileSync(root + "/.github/renovate.json"));
    const { config: resolvedLocal } = await resolveConfigPresets(local);
    const localEffective = await applyPackageRules({
      ...resolvedLocal,
      manager: "cargo",
      datasource: "crate",
      packageName: "serde",
      depName: "serde",
    });
    assert.equal(localEffective.rebaseWhen, "behind-base-branch");
    const adopted = (local.extends || []).some((ref) =>
      ref.startsWith("github>Strukturpiloten/boxferry//renovate-presets/common#"),
    );
    const managers = [
      ...(local.customManagers || []),
      ...(adopted ? common.customManagers : []),
    ];
    const files = cp
      .execFileSync(
        "git",
        ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        { cwd: root, encoding: "utf8" },
      )
      .split("\0")
      .filter(Boolean);
    const owners = new Map();
    let extracted = 0;
    for (let index = 0; index < managers.length; index++) {
      const manager = managers[index];
      for (const file of files) {
        if (
          !manager.managerFilePatterns.some((pattern) =>
            new RegExp(pattern.slice(1, -1)).test(file),
          )
        )
          continue;
        const result = extract(
          fs.readFileSync(path.join(root, file), "utf8"),
          file,
          manager,
        );
        for (const dep of result?.deps || []) {
          const key = JSON.stringify([
            file,
            dep.replaceString,
            dep.currentValue,
            dep.currentDigest,
          ]);
          if (!owners.has(key)) owners.set(key, new Set());
          owners.get(key).add(index);
          extracted++;
        }
      }
    }
    assert.equal(
      [...owners.values()].filter((value) => value.size > 1).length,
      0,
      "duplicate active custom extraction owners",
    );
    console.log(
      JSON.stringify({
        version: JSON.parse(fs.readFileSync(packageRoot + "/package.json")).version,
        replacements,
        ruleCases: cases.length,
        operationalExtractions: extracted,
        runnerSemantics,
        nodeOwnership: {physicalProducerLiterals:literalVersions.length,
          nativeProducerRows:producerNodes, lookup:nodeLookupProof,
          actionOwners:setupOwners, matrix},
validationWiring: (() => {
  const ci=YAML.parse(producerText);
  const release=YAML.parse(fs.readFileSync(root+'/.github/workflows/release.yml','utf8'));
  return {automation:ci.jobs.automation, required:ci.jobs["pr-gate"],
    validationPlan:ci.jobs["validation-plan"],
    events:Object.keys(ci.on), releaseUses:Object.values(release.jobs).map(job => job.uses)};
})(),
      }),
    );
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
"""
        with patch.dict(os.environ, {"BOXFERRY_NODE": "/trusted/node"}):
            limited = limited_config(
                "Strukturpiloten/boxferry", Path("/trusted/source"), Path("/trusted/python")
            )
        completed = subprocess.run(
            [
                "node",
                "--experimental-vm-modules",
                "-e",
                script,
                str(runtime),
                str(ROOT),
                json.dumps(limited),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        if evidence := os.environ.get("BOXFERRY_NODE_OWNERSHIP_EVIDENCE"):
            output = Path(evidence)
            output.write_text(
                json.dumps(json.loads(completed.stdout.splitlines()[-1]), indent=2) + "\n"
            )
            output.with_suffix(".raw.log").write_text(completed.stdout + completed.stderr)
        report = json.loads(completed.stdout.splitlines()[-1])
        type(self)._cached_native_report = report
        return report

    def test_real_extraction_replacement_rule_order_and_unique_local_owners(self) -> None:
        report = self._native_report()
        self.assertGreater(report["replacements"], 0)
        self.assertGreater(report["operationalExtractions"], 0)

    def test_native_and_custom_runner_selectors_preserve_age_and_unrelated_updates(self) -> None:
        proof = self._native_report()["runnerSemantics"]
        self.assertEqual(len(proof["native"]), 6)
        self.assertGreater(proof["actualProducerNativeRows"], 0)
        self.assertEqual(len(proof["matrix"]), 1)
        self.assertEqual(proof["canonicalCustomRunnerRows"], 0)
        self.assertEqual(proof["preFixNativeControl"]["behaviour"], "timestamp-required")
        self.assertEqual(len(proof["exclusions"]), 7)
        self.assertEqual(len(proof["scopeControls"]), 4)
        self.assertTrue(all(row["result"]["enabled"] for row in proof["exclusions"]))

    def test_native_node_owner_accepts_real_block_anchor_alias_and_decoded_yaml(self) -> None:
        matrix = self._native_report()["nodeOwnership"]["matrix"]
        self.assertEqual(len(matrix["positive"]), 7)
        for observed in matrix["positive"]:
            with self.subTest(observed=observed["name"]):
                self.assertTrue(observed["accepted"])
                self.assertEqual(observed["physical"], 1)
                self.assertGreaterEqual(observed["nativeRows"], 1)
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual(matrix["oracle"]["version"], pin.split(":", 1)[1].split("@", 1)[0])
        production = {item["file"]: item for item in matrix["production"]}
        self.assertEqual(production[".github/workflows/ci.yml"]["physical"], 1)
        self.assertEqual(production[".github/actions/shared-node/action.yml"]["physical"], 1)

    def test_native_node_owner_rejects_partial_or_missing_ownership(self) -> None:
        cases = self._native_report()["nodeOwnership"]["matrix"]["partial"]
        self.assertEqual(len(cases), 27)
        for case in cases:
            self.assertEqual(case["category"], case["expectedCategory"])
            self.assertEqual(case["refusal"].splitlines()[0], case["expectedCategory"])
        self.assertEqual(sum("alongside valid owner" in case["name"] for case in cases), 13)
        self.assertTrue(
            self._native_report()["nodeOwnership"]["matrix"]["categoryGuardControl"][
                "unrelatedFailureRejected"
            ]
        )

    def test_native_node_owner_rejects_competing_effective_managers(self) -> None:
        cases = self._native_report()["nodeOwnership"]["matrix"]["competing"]
        self.assertEqual(len(cases), 8)
        for case in cases:
            self.assertEqual(case["category"], case["expectedCategory"])
            self.assertEqual(case["refusal"].splitlines()[0], case["expectedCategory"])

    def test_native_node_scalar_text_is_not_an_owner(self) -> None:
        cases = self._native_report()["nodeOwnership"]["matrix"]["scalar"]
        self.assertEqual(len(cases), 14)
        for observed in cases:
            if "refusal" in observed:
                self.assertEqual(observed["category"], "missing physical Node input owner")
                self.assertEqual(observed["refusal"].splitlines()[0], observed["expectedCategory"])
            else:
                self.assertEqual(observed["physical"], 1)

    def test_native_node_dynamic_lookup_stays_inactive(self) -> None:
        proof = self._native_report()["nodeOwnership"]["lookup"]
        self.assertTrue(proof["transportGuardControl"])
        self.assertTrue(
            all(value.startswith("datasource.") for value in proof["transportGuardControl"])
        )
        for result in proof["results"]:
            self.assertEqual(result["skipReason"], "invalid-value")
            self.assertEqual(result["updates"], [])

    def test_native_node_runtime_missing_or_corrupt_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-native-cache-control-") as directory:
            runtime = Path(directory)
            for corrupt in (False, True):
                with self.subTest(corrupt=corrupt):
                    if corrupt:
                        (runtime / "source-image.txt").write_text("wrong-image")
                        (runtime / "package.json").write_text("{}")
                    with self.assertRaisesRegex(AssertionError, "missing or corrupt pinned native"):
                        _validate_native_runtime(runtime)
            pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
            (runtime / "source-image.txt").write_text(pin)
            (runtime / "package.json").write_text(
                json.dumps({"version": pin.split(":", 1)[1].split("@", 1)[0]})
            )
            module = runtime / "dist/modules/manager/github-actions/index.js"
            module.parent.mkdir(parents=True)
            module.write_text("throw new Error('synthetic corrupt SDK module');")
            _validate_native_runtime(runtime)
            failed = subprocess.run(
                ["node", "-e", "require(process.argv[1])", str(module)],
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn("synthetic corrupt SDK module", failed.stderr)

    def _trusted_shared_selection_cases(self) -> dict[str, Any]:
        """Exercise real schema-3 planning/gating; only the read-only Git transport is synthetic."""
        from boxferry_ci import gate as shared_gate
        from boxferry_ci import planner as shared_planner
        from boxferry_ci.contracts import ContractError, object_value
        from boxferry_ci.policy import load_policy

        current = load_policy(ROOT / "scripts/shared-validation-policy.json")
        self.assertIn("automation", current.selected_jobs(("prose",)))
        observed: dict[str, Any] = {
            "current_policy_prose_selects_automation": True,
            "controlled_trusted_prose_skip_fixture": True,
            "source_cases": [],
            "refusals": [],
        }
        with tempfile.TemporaryDirectory(prefix="boxferry-trusted-node-plan-") as directory:
            fixture = Path(directory)
            raw = json.loads(current.path.read_text())
            # This fixture models the approved future choice; live policy remains unchanged.
            raw["profile_jobs"]["prose"] = [
                job for job in raw["profile_jobs"]["prose"] if job != "automation"
            ]
            config = fixture / "scripts/shared-validation-policy.json"
            config.parent.mkdir(parents=True)
            config.write_text(json.dumps(raw, sort_keys=True) + "\n")
            trusted_bytes = config.read_bytes()
            fixture_policy = load_policy(config)
            base, head, tested = "a" * 40, "b" * 40, "c" * 40
            revisions = {base, head, tested}
            contents: dict[str, bytes] = {"scripts/shared-validation-policy.json": trusted_bytes}
            if current.documentation_examples_manifest is not None:
                manifest = current.documentation_examples_manifest
                contents[manifest] = (ROOT / manifest).read_bytes()
                target = fixture / manifest
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(contents[manifest])
            paths = (
                ".github/workflows/ci.yml",
                ".github/workflows/shared-devcontainer-updates.yml",
                ".github/workflows/shared-devcontainer-validation.yml",
                ".github/workflows/shared-release-validation.yml",
                ".github/actions/shared-node/action.yml",
                ".github/renovate.json",
                "renovate-presets/common.json",
                "renovate-presets/rust.json",
                "automation/tests/test_presets.py",
                "automation/src/boxferry_ci/node_setup.py",
            )
            contents.update({path: (ROOT / path).read_bytes() for path in paths})
            prose = "docs/public/guide/controlled-native-prose-fixture.md"
            contents[prose] = b"Plain documentation without executable content.\n"
            selected_path = [paths[0]]

            def transport(_root: Path, *args: str, optional: bool = False) -> bytes | None:
                if args == ("rev-parse", "--show-toplevel"):
                    return str(fixture).encode()
                if args == ("rev-parse", "HEAD"):
                    return tested.encode()
                if args[:2] == ("merge-base", "--is-ancestor"):
                    self.assertEqual(args[2:], (head, tested))
                    return b""
                if args == ("merge-base", "HEAD", "origin/main"):
                    return base.encode()
                if args[0] == "diff":
                    return (
                        b"M\0" + selected_path[0].encode() + b"\0"
                        if "--find-renames" in args
                        else b""
                    )
                if args == ("ls-files", "--others", "--exclude-standard", "-z"):
                    return b""
                if args[:2] == ("ls-tree", "-z"):
                    revision, separator, path = args[2], args[3], args[4]
                    self.assertIn(revision, revisions)
                    self.assertEqual(separator, "--")
                    if path not in contents:
                        return b""
                    blob = contents[path]
                    oid = hashlib.sha1(
                        b"blob " + str(len(blob)).encode() + b"\0" + blob, usedforsecurity=False
                    ).hexdigest()
                    return f"100644 blob {oid}\t{path}\0".encode()
                if args[0] == "show":
                    revision, path = args[1].split(":", 1)
                    self.assertIn(revision, revisions)
                    return contents.get(path)
                raise AssertionError(f"unexpected fixture Git read: {args}, optional={optional}")

            def recompute() -> dict[str, Any]:
                return shared_planner.make_plan(
                    fixture,
                    fixture_policy,
                    event="pull_request",
                    base_sha=base,
                    head_sha=head,
                    tested_sha=tested,
                    policy_sha=base,
                )

            def needs_for(plan: dict[str, Any]) -> dict[str, Any]:
                jobs = object_value(plan["jobs"], "fixture plan jobs")
                return {
                    "validation-plan": {"result": "success"},
                    **{
                        job: {"result": "success" if jobs[job] else "skipped"}
                        for job in fixture_policy.jobs
                    },
                }

            comparison = shared_planner.Comparison("pull_request", base, head, tested)
            with patch.object(shared_planner, "git", side_effect=transport):
                # All four policy/planning/gate functions stay live; nothing returns a canned plan.
                shared_planner.verify_policy_revision(fixture_policy, base, comparison)
                for path in paths:
                    selected_path[0] = path
                    self.assertEqual(
                        shared_planner.classify(fixture, comparison, fixture_policy)[0], ("full",)
                    )
                    plan = recompute()
                    self.assertIs(object_value(plan["jobs"], "fixture jobs")["automation"], True)
                    expected = recompute()
                    shared_gate.verify_gate(
                        plan, needs_for(plan), fixture_policy, comparison, expected_plan=expected
                    )
                    observed["source_cases"].append({"path": path, "automation_selected": True})
                    for result in ("skipped", "failure", "cancelled", "missing"):
                        needs = needs_for(plan)
                        if result == "missing":
                            del needs["automation"]
                        else:
                            needs["automation"]["result"] = result
                        with self.assertRaisesRegex(
                            ContractError, "selected automation|dependencies do not match"
                        ):
                            shared_gate.verify_gate(
                                plan, needs, fixture_policy, comparison, expected_plan=recompute()
                            )
                        observed["refusals"].append({"path": path, "result": result})
                selected_path[0] = prose
                prose_plan = recompute()
                self.assertIs(object_value(prose_plan["jobs"], "fixture jobs")["automation"], False)
                shared_gate.verify_gate(
                    prose_plan,
                    needs_for(prose_plan),
                    fixture_policy,
                    comparison,
                    expected_plan=recompute(),
                )
                observed["controlled_prose_planned_skip_accepted"] = True
                selected_path[0] = paths[0]
                with self.assertRaisesRegex(
                    ContractError, "independent exact-candidate comparison"
                ):
                    shared_gate.verify_gate(
                        prose_plan,
                        needs_for(prose_plan),
                        fixture_policy,
                        comparison,
                        expected_plan=recompute(),
                    )
                observed["refusals"].append({"case": "prose plan forged for Node source change"})
                with self.assertRaisesRegex(ContractError, "exact comparison base"):
                    shared_planner.make_plan(
                        fixture,
                        fixture_policy,
                        event="pull_request",
                        base_sha=base,
                        head_sha=head,
                        tested_sha=tested,
                        policy_sha=head,
                    )
                observed["refusals"].append({"case": "untrusted policy SHA"})
                config.write_bytes(trusted_bytes + b" ")
                with self.assertRaisesRegex(ContractError, "contents differ from the trusted base"):
                    recompute()
                config.write_bytes(trusted_bytes)
                observed["refusals"].append({"case": "untrusted policy bytes"})
                plan = recompute()
                changed = shared_planner.Comparison("pull_request", base, head, "d" * 40)
                with self.assertRaisesRegex(ContractError, "exact tested_sha"):
                    shared_gate.verify_gate(
                        plan, needs_for(plan), fixture_policy, changed, expected_plan=recompute()
                    )
                observed["refusals"].append({"case": "untrusted tested identity"})
                for event in ("push", "workflow_dispatch", "workflow_call", "local"):
                    plan = shared_planner.make_plan(
                        fixture, fixture_policy, event=event, tested_sha=tested
                    )
                    self.assertIs(object_value(plan["jobs"], "fixture jobs")["automation"], True)
            observed["trusted_fixture_policy_sha256"] = hashlib.sha256(trusted_bytes).hexdigest()
            observed["trusted_blob_sha256"] = {
                path: hashlib.sha256(value).hexdigest() for path, value in contents.items()
            }
        return observed

    @staticmethod
    def _shared_route_command(command: str) -> str:
        mode = "plan" if command == "plan" else "gate --devcontainer-validation"
        suffix = (
            '--github-output "${GITHUB_OUTPUT}"'
            if command == "plan"
            else ('--plan-json "${PLAN_JSON}" --needs-json "${NEEDS_JSON}"')
        )
        return (
            "set -euo pipefail\nuv run --locked --project _automation/automation "
            f"python -m boxferry_ci {mode} "
            '--repository-root "${GITHUB_WORKSPACE}" '
            "--config .validation-base/scripts/shared-validation-policy.json "
            '--event "${EVENT_NAME}" --base-sha "${BASE_SHA}" --head-sha "${HEAD_SHA}" '
            '--tested-sha "${GITHUB_SHA}" --policy-sha "${BASE_SHA}" '
            f'--automation-sha "${{AUTOMATION_SHA}}" {suffix}'
        )

    def _assert_trusted_shared_node_route(self, wiring: dict[str, Any]) -> None:
        from boxferry_ci.policy import load_policy

        automation = wiring["automation"]
        self.assertEqual(
            automation.get("if"), "needs.validation-plan.outputs.select_automation == 'true'"
        )
        dependencies = automation.get("needs")
        self.assertEqual(
            [dependencies] if isinstance(dependencies, str) else dependencies, ["validation-plan"]
        )
        planner = wiring["validationPlan"]
        self.assertNotIn("if", planner)
        self.assertIs(planner.get("continue-on-error", False), False)
        self.assertEqual(
            planner["outputs"]["select_automation"], "${{ steps.plan.outputs.select_automation }}"
        )
        self.assertEqual(planner["outputs"]["plan"], "${{ steps.plan.outputs.plan }}")
        policy = load_policy(ROOT / "scripts/shared-validation-policy.json")
        self.assertEqual(set(wiring["required"]["needs"]), {"validation-plan", *policy.jobs})
        base_ref = (
            "${{ github.event.pull_request.base.sha || "
            "github.event.merge_group.base_sha || github.sha }}"
        )
        head_ref = "${{ github.event.pull_request.head.sha || github.sha }}"
        bound_automation = []
        for command, job in (("plan", planner), ("gate", wiring["required"])):
            self.assertIs(job.get("continue-on-error", False), False)
            checkout = [
                step
                for step in job["steps"]
                if step.get("uses", "").startswith("actions/checkout@")
            ]
            self.assertEqual(len(checkout), 3)
            by_path = {step.get("with", {}).get("path", "."): step for step in checkout}
            self.assertEqual(set(by_path), {".", "_automation", ".validation-base"})
            self.assertEqual(by_path["."]["with"]["ref"], "${{ github.sha }}")
            self.assertEqual(by_path["."]["with"]["fetch-depth"], 0)
            shared = by_path["_automation"]["with"]
            self.assertEqual(shared["repository"], "Strukturpiloten/boxferry")
            self.assertRegex(shared["ref"], r"^[a-f0-9]{40}$")
            bound_automation.append(shared["ref"])
            self.assertEqual(by_path[".validation-base"]["with"]["ref"], base_ref)
            self.assertTrue(
                all(step["with"].get("persist-credentials") is False for step in checkout)
            )
            self.assertEqual(
                sum(
                    step.get("uses") == "./_automation/.github/actions/shared-python"
                    for step in job["steps"]
                ),
                1,
            )
            calls = [
                step
                for step in job["steps"]
                if f"python -m boxferry_ci {command}" in step.get("run", "")
            ]
            self.assertEqual(len(calls), 1)
            call = calls[0]
            self.assertEqual(call.get("shell"), "bash")
            for step in [
                *checkout,
                call,
                *[
                    step
                    for step in job["steps"]
                    if step.get("uses") == "./_automation/.github/actions/shared-python"
                ],
            ]:
                self.assertNotIn("if", step)
                self.assertIs(step.get("continue-on-error", False), False)
            env = call["env"]
            self.assertEqual(env["EVENT_NAME"], "${{ github.event_name }}")
            self.assertEqual(env["BASE_SHA"], base_ref)
            self.assertEqual(env["HEAD_SHA"], head_ref)
            self.assertEqual(env["AUTOMATION_SHA"], shared["ref"])
            self.assertTrue(call["run"].startswith("set -euo pipefail\n"))
            normalized = " ".join(call["run"].replace("\\\n", " ").split())
            self.assertEqual(normalized, " ".join(self._shared_route_command(command).split()))
            if command == "plan":
                self.assertEqual(call["id"], "plan")
            else:
                self.assertEqual(env["PLAN_JSON"], "${{ needs.validation-plan.outputs.plan }}")
                self.assertEqual(env["NEEDS_JSON"], "${{ toJSON(needs) }}")
        self.assertEqual(bound_automation[0], bound_automation[1])
        # CLI recomputes from independently bound source/policy rather than supplied selections.
        cli = (ROOT / "automation/src/boxferry_ci/__main__.py").read_text()
        self.assertIn("expected_plan=_plan(args, policy)", cli)

    def _controlled_conditional_wiring(self, original: dict[str, Any]) -> dict[str, Any]:
        """Authored contract fixture: candidate T differs from immutable planner/gate A."""
        fixture = copy.deepcopy(original)
        fixture["automation"]["needs"] = "validation-plan"
        fixture["automation"]["if"] = "needs.validation-plan.outputs.select_automation == 'true'"
        pin = next(
            step["uses"]
            for step in original["automation"]["steps"]
            if step.get("uses", "").startswith("actions/checkout@")
        )
        base = (
            "${{ github.event.pull_request.base.sha || "
            "github.event.merge_group.base_sha || github.sha }}"
        )
        automation_sha = "e" * 40  # Explicit synthetic identity, not an admitted pin.

        def job(command: str) -> dict[str, Any]:
            env = {
                "EVENT_NAME": "${{ github.event_name }}",
                "BASE_SHA": base,
                "HEAD_SHA": "${{ github.event.pull_request.head.sha || github.sha }}",
                "AUTOMATION_SHA": automation_sha,
            }
            if command == "gate":
                env.update(
                    PLAN_JSON="${{ needs.validation-plan.outputs.plan }}",
                    NEEDS_JSON="${{ toJSON(needs) }}",
                )
            call = {"shell": "bash", "env": env, "run": self._shared_route_command(command)}
            if command == "plan":
                call["id"] = "plan"
            return {
                "steps": [
                    {
                        "uses": pin,
                        "with": {
                            "ref": "${{ github.sha }}",
                            "fetch-depth": 0,
                            "persist-credentials": False,
                        },
                    },
                    {
                        "uses": pin,
                        "with": {
                            "repository": "Strukturpiloten/boxferry",
                            "ref": automation_sha,
                            "path": "_automation",
                            "persist-credentials": False,
                        },
                    },
                    {
                        "uses": pin,
                        "with": {
                            "ref": base,
                            "path": ".validation-base",
                            "persist-credentials": False,
                        },
                    },
                    {"uses": "./_automation/.github/actions/shared-python"},
                    call,
                ]
            }

        fixture["validationPlan"] = job("plan")
        fixture["validationPlan"]["outputs"] = {
            "plan": "${{ steps.plan.outputs.plan }}",
            "select_automation": "${{ steps.plan.outputs.select_automation }}",
        }
        fixture["required"]["steps"] = job("gate")["steps"]
        return fixture

    def test_native_node_ownership_validation_cannot_be_skipped(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "boxferry_node_selector", ROOT / "scripts/validation-plan.py"
        )
        assert spec is not None and spec.loader is not None
        selector = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(selector)
        policy = selector.policy_at(ROOT)
        wiring = self._native_report()["validationWiring"]
        local = (ROOT / "scripts/check-all.sh").read_text()
        suite = unittest.TestLoader().discover(
            str(ROOT / "automation/tests"), pattern="test_presets.py"
        )

        def discovered_names(tests: unittest.TestSuite) -> set[str]:
            names = set()
            for test in tests:
                if isinstance(test, unittest.TestSuite):
                    names.update(discovered_names(test))
                elif isinstance(test, unittest.TestCase):
                    names.add(test.id().rsplit(".", 1)[-1])
            return names

        discovered = discovered_names(suite)
        required = {
            "test_native_node_owner_accepts_real_block_anchor_alias_and_decoded_yaml",
            "test_native_node_owner_rejects_partial_or_missing_ownership",
            "test_native_node_owner_rejects_competing_effective_managers",
            "test_native_node_scalar_text_is_not_an_owner",
            "test_native_node_dynamic_lookup_stays_inactive",
            "test_native_node_runtime_missing_or_corrupt_fails_closed",
            "test_native_node_ownership_validation_cannot_be_skipped",
        }

        def check(current: dict[str, Any], local_gate: str, names: set[str]) -> None:
            automation = current["automation"]
            conditional = "if" in automation
            if conditional:
                self._assert_trusted_shared_node_route(current)
            self.assertIs(automation.get("continue-on-error", False), False)
            steps = automation["steps"]
            candidate = [
                step
                for step in steps
                if step.get("uses", "").startswith("actions/checkout@")
                and step.get("with", {}).get("path") == "_automation"
            ]
            self.assertEqual(len(candidate), 1)
            self.assertEqual(candidate[0]["with"]["ref"], "${{ github.sha }}")
            self.assertIs(candidate[0]["with"].get("persist-credentials"), False)
            self.assertNotIn("if", candidate[0])
            self.assertIs(candidate[0].get("continue-on-error", False), False)
            runs = [step.get("run", "") for step in steps]
            provision = next(
                i for i, run in enumerate(runs) if "boxferry_ci.renovate_runtime" in run
            )
            discover = next(i for i, run in enumerate(runs) if "boxferry_ci.test_reporting" in run)
            self.assertLess(provision, discover)
            for index in (provision, discover):
                self.assertNotIn("if", steps[index])
                self.assertIs(steps[index].get("continue-on-error", False), False)
            self.assertIn("--start-directory _automation/automation/tests", runs[discover])
            self.assertIn("BOXFERRY_RENOVATE_PACKAGE", steps[discover]["env"])
            self.assertIn(
                "boxferry_ci.test_reporting --repository-root . --start-directory automation/tests",
                local_gate,
            )
            self.assertIn(
                'unittest.TestLoader().discover(str(args.start_directory), pattern="test*.py")',
                (ROOT / "automation/src/boxferry_ci/test_reporting_worker.py").read_text(),
            )
            self.assertTrue(required.issubset(names))
            self.assertEqual(
                policy["local_commands"]["full"], [["bash", "scripts/check-all.sh", "--check"]]
            )
            self.assertTrue(
                {"push", "workflow_dispatch", "workflow_call"}.issubset(current["events"])
            )
            self.assertIn("./.github/workflows/ci.yml", current["releaseUses"])
            for path in (
                ".github/workflows/ci.yml",
                ".github/workflows/shared-devcontainer-updates.yml",
                ".github/workflows/shared-devcontainer-validation.yml",
                ".github/workflows/shared-release-validation.yml",
                ".github/actions/shared-node/action.yml",
                "renovate-presets/common.json",
                "renovate-presets/rust.json",
                ".github/renovate.json",
                "automation/tests/test_presets.py",
                "automation/src/boxferry_ci/node_setup.py",
            ):
                self.assertEqual(selector.path_profile(path, [], policy, set()), "full", path)
                profile, _ = selector.profile_for_changes(
                    ROOT, SHA, SHA, [("M", (path,))], policy, ROOT, local=False
                )
                self.assertEqual(profile, "full", path)
            for event in ("push", "workflow_dispatch", "workflow_call"):
                args = argparse.Namespace(
                    repository_root=ROOT,
                    policy_root=ROOT,
                    event=event,
                    tested_sha=SHA,
                    force_full=False,
                )
                self.assertEqual(selector.make_plan(args, policy)["profile"], "full")
            local_args = argparse.Namespace(
                repository_root=ROOT,
                policy_root=ROOT,
                event="local",
                tested_sha=SHA,
                force_full=False,
                base_ref="origin/main",
            )
            with (
                patch.object(selector, "git", return_value=SHA.encode()),
                patch.object(
                    selector,
                    "local_changes",
                    return_value=([("M", ("automation/tests/test_presets.py",))], "f" * 64),
                ),
            ):
                self.assertEqual(selector.make_plan(local_args, policy)["profile"], "full")
            aggregate = current["required"]
            self.assertIn("automation", aggregate["needs"])
            self.assertEqual(aggregate["if"], "always()")
            if conditional:
                return
            run = next(
                step["run"]
                for step in aggregate["steps"]
                if "BINARY_NEEDS_PY" in step.get("run", "")
            )
            payload = textwrap.dedent(
                run.split("<<'BINARY_NEEDS_PY'\n", 1)[1].split("\nBINARY_NEEDS_PY", 1)[0]
            )
            for result in ("success", "skipped", "failure", "cancelled", "missing"):
                needs = {job: {"result": "success"} for job in aggregate["needs"]}
                if result == "missing":
                    del needs["automation"]
                else:
                    needs["automation"]["result"] = result
                completed = subprocess.run(
                    [sys.executable, "-c", payload],
                    capture_output=True,
                    text=True,
                    timeout=10,
                    env={**os.environ, "NEEDS_JSON": json.dumps(needs)},
                )
                self.assertEqual(completed.returncode == 0, result == "success", result)

        check(wiring, local, discovered)
        controlled = self._controlled_conditional_wiring(wiring)
        check(controlled, local, discovered)
        selection_evidence = self._trusted_shared_selection_cases()
        selection_evidence["supported_routes"] = [
            "current unconditional legacy",
            "exact trusted shared schema-3",
        ]
        for mutation in (
            "missing dependency",
            "wrong selector",
            "wrong output",
            "candidate policy",
            "wrong A",
            "wrong candidate",
            "wrong policy argument",
            "wrong policy identity",
            "wrong plan binding",
            "wrong candidate package",
            "plan step condition",
            "plan step continue-on-error",
            "gate step condition",
            "gate step continue-on-error",
            "plan job continue-on-error",
            "gate job continue-on-error",
        ):
            changed = copy.deepcopy(controlled)
            if mutation == "missing dependency":
                changed["automation"]["needs"] = []
            elif mutation == "wrong selector":
                changed["automation"]["if"] = "always()"
            elif mutation == "wrong candidate package":
                changed["automation"]["steps"][0]["with"]["ref"] = "main"
            elif mutation.startswith(("plan step", "gate step", "plan job", "gate job")):
                job = (
                    changed["validationPlan"]
                    if mutation.startswith("plan")
                    else changed["required"]
                )
                target = job if " job " in mutation else job["steps"][-1]
                target["if" if mutation.endswith("condition") else "continue-on-error"] = (
                    False if mutation.endswith("condition") else True
                )
            elif mutation == "wrong output":
                changed["validationPlan"]["outputs"]["select_automation"] = (
                    "${{ steps.untrusted.outputs.select_automation }}"
                )
            else:
                job = (
                    changed["validationPlan"]
                    if mutation in {"candidate policy", "wrong A", "wrong candidate"}
                    else changed["required"]
                )
                if mutation == "candidate policy":
                    job["steps"][2]["with"]["ref"] = "${{ github.sha }}"
                elif mutation == "wrong A":
                    job["steps"][1]["with"]["ref"] = "f" * 40
                elif mutation == "wrong candidate":
                    job["steps"][0]["with"]["ref"] = "main"
                else:
                    call = job["steps"][-1]
                    if mutation == "wrong plan binding":
                        call["env"]["PLAN_JSON"] = "${{ needs.untrusted.outputs.plan }}"
                    elif mutation == "wrong policy identity":
                        call["run"] = call["run"].replace(
                            '--policy-sha "${BASE_SHA}"', '--policy-sha "${GITHUB_SHA}"'
                        )
                    else:
                        call["run"] = call["run"].replace(
                            ".validation-base/scripts/shared-validation-policy.json",
                            "scripts/shared-validation-policy.json",
                        )
            with self.subTest(trusted_route_mutation=mutation), self.assertRaises(AssertionError):
                check(changed, local, discovered)
            selection_evidence.setdefault("binding_refusals", []).append(mutation)
        if evidence := os.environ.get("BOXFERRY_NODE_SELECTION_EVIDENCE"):
            Path(evidence).write_text(json.dumps(selection_evidence, indent=2) + "\n")
        changed = copy.deepcopy(wiring)
        changed["automation"]["if"] = "false"
        with self.assertRaises(AssertionError):
            check(changed, local, discovered)
        changed = copy.deepcopy(wiring)
        changed["automation"]["continue-on-error"] = True
        with self.assertRaises(AssertionError):
            check(changed, local, discovered)
        for command in ("boxferry_ci.renovate_runtime", "boxferry_ci.test_reporting"):
            for flag, value in (("if", False), ("continue-on-error", True)):
                with self.subTest(command=command, flag=flag):
                    changed = copy.deepcopy(wiring)
                    step = next(
                        step
                        for step in changed["automation"]["steps"]
                        if command in step.get("run", "")
                    )
                    step[flag] = value
                    with self.assertRaises(AssertionError):
                        check(changed, local, discovered)
        changed = copy.deepcopy(wiring)
        for step in changed["required"]["steps"]:
            if "BINARY_NEEDS_PY" in step.get("run", ""):
                step["run"] = step["run"].replace(
                    'prerequisite.get("result") != "success"', "False"
                )
        with self.assertRaises(AssertionError):
            check(changed, local, discovered)
        with self.assertRaises(AssertionError):
            check(wiring, local, discovered - {next(iter(required))})
        with patch.object(selector, "path_profile", return_value="prose"):
            with self.assertRaises(AssertionError):
                check(wiring, local, discovered)
        with self.assertRaises(AssertionError):
            check(wiring, local.replace("boxferry_ci.test_reporting", "omitted"), discovered)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--consumer":
        assert_automatic_policy(consumer_config(Path(sys.argv[2])))
    else:
        unittest.main()
