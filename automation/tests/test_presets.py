"""Shared-policy composition examples; native extraction is validated separately.

Consumers can call ``consumer_config`` and ``effective`` for their own local
rules. This intentionally models only the matchers listed below, and rejects
unknown matchers instead of claiming complete Renovate emulation.
"""

from __future__ import annotations

import copy
import fnmatch
import json
import os
import re
import subprocess
import sys
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


class NativePresetTests(unittest.TestCase):
    def test_real_extraction_replacement_rule_order_and_unique_local_owners(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        self.assertTrue(
            runtime.is_dir(), "provision the pinned Renovate runtime before the complete gate"
        )
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual((runtime / "source-image.txt").read_text().strip(), pin)
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


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--consumer":
        assert_automatic_policy(consumer_config(Path(sys.argv[2])))
    else:
        unittest.main()
