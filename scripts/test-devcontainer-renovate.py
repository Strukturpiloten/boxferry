#!/usr/bin/env python3
"""Check actual Dev Container pin extraction and image replacement without network access."""

from __future__ import annotations

import copy
import fnmatch
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCKERFILE_PATH = ".devcontainer/Dockerfile"
EXPECTED_PINS = {
    ("docker", "docker.io/library/node"),
    ("docker", "ghcr.io/astral-sh/uv"),
    ("docker", "mcr.microsoft.com/devcontainers/rust"),
    ("crate", "cargo-deny"),
    ("crate", "cargo-llvm-cov"),
    ("crate", "cargo-semver-checks"),
    ("crate", "lychee"),
    ("crate", "zizmor"),
    ("github-releases", "rhysd/actionlint"),
}


def python_pattern(pattern: str) -> str:
    """Translate only named-group spelling; production patterns remain Renovate's patterns."""
    return re.sub(r"\(\?<([A-Za-z][A-Za-z0-9_]*)>", r"(?P<\1>", pattern)


def extracted_pins(configuration: dict, dockerfile: str) -> list[tuple[dict, re.Match]]:
    pins = []
    for manager in configuration["customManagers"]:
        if manager["customType"] != "regex":
            continue
        if not any(
            re.search(pattern[1:-1], DOCKERFILE_PATH) for pattern in manager["managerFilePatterns"]
        ):
            continue
        for pattern in manager["matchStrings"]:
            pins.extend(
                (manager, match) for match in re.finditer(python_pattern(pattern), dockerfile)
            )
    return pins


def validate_ownership(configuration: dict, dockerfile: str) -> list[tuple[dict, re.Match]]:
    if configuration["dockerfile"]["enabled"] is not False:
        raise ValueError("native Dockerfile extraction would duplicate regex ownership")
    pins = extracted_pins(configuration, dockerfile)
    identities = [(match["datasource"], match["depName"]) for _, match in pins]
    if len(identities) != len(EXPECTED_PINS) or set(identities) != EXPECTED_PINS:
        raise ValueError("each operational pin needs exactly one owner")
    image_pins = [(manager, match) for manager, match in pins if match["datasource"] == "docker"]
    if len(image_pins) != len(re.findall(r"(?m)^FROM ", dockerfile)):
        raise ValueError("every external image needs version and digest extraction")
    if len(pins) - len(image_pins) != len(re.findall(r"(?m)^ARG [A-Z0-9_]+_VERSION=", dockerfile)):
        raise ValueError("every tool argument needs version extraction")
    for manager, match in image_pins:
        expected = f"FROM {match['depName']}:{match['currentValue']}@{match['currentDigest']}"
        if not match[0].endswith(expected) or manager.get("versioningTemplate") != "docker":
            raise ValueError("image release annotation and immutable tagged reference must agree")
    return pins


def effective_policy(configuration: dict, dependency: dict) -> dict:
    """Evaluate the current policy's selectors in order; reject unmodelled selectors."""
    policy = {
        key: configuration.get(key, False)
        for key in ("automerge", "dependencyDashboardApproval")
    }
    policy["minimumReleaseAge"] = configuration["minimumReleaseAge"]
    for rule in configuration["packageRules"]:
        selectors = {key for key in rule if key.startswith("match")}
        if selectors - dependency.keys():
            raise ValueError("policy regression must cover every production selector")
        if any(
            dependency[key] is None
            or not any(fnmatch.fnmatchcase(dependency[key], pattern) for pattern in rule[key])
            for key in selectors
        ):
            continue
        policy.update({key: value for key, value in rule.items() if key not in selectors})
    return policy


def pin_policy(configuration: dict, match: re.Match, update_type: str) -> dict:
    return effective_policy(
        configuration,
        {
            "matchManagers": "custom.regex",
            "matchDatasources": match["datasource"],
            "matchPackageNames": match["depName"],
            "matchFileNames": DOCKERFILE_PATH,
            "matchDepTypes": None,
            "matchUpdateTypes": update_type,
        },
    )


class DevContainerRenovateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.configuration = json.loads(
            (ROOT / ".github/renovate.json").read_text(encoding="utf-8")
        )
        self.dockerfile = (ROOT / DOCKERFILE_PATH).read_text(encoding="utf-8")

    def test_all_nine_operational_pins_have_one_real_extraction_owner(self) -> None:
        pins = validate_ownership(self.configuration, self.dockerfile)
        self.assertEqual(len(pins), 9)
        self.assertTrue(all(match["currentValue"] for _, match in pins))

    def test_missing_tool_marker_cannot_silently_drop_a_dependency(self) -> None:
        for datasource, name in EXPECTED_PINS:
            with self.subTest(dependency=name):
                source = self.dockerfile.replace(
                    f"# renovate: datasource={datasource} depName={name}\n", "", 1
                )
                with self.assertRaises(ValueError):
                    validate_ownership(self.configuration, source)

    def test_duplicate_or_native_manager_is_rejected(self) -> None:
        owners = {
            id(manager): manager
            for manager, _ in extracted_pins(self.configuration, self.dockerfile)
        }
        for owner in owners.values():
            configuration = copy.deepcopy(self.configuration)
            configuration["customManagers"].append(copy.deepcopy(owner))
            with self.assertRaises(ValueError):
                validate_ownership(configuration, self.dockerfile)
        configuration = copy.deepcopy(self.configuration)
        configuration["dockerfile"]["enabled"] = True
        with self.assertRaises(ValueError):
            validate_ownership(configuration, self.dockerfile)

    def test_mismatched_tag_or_missing_digest_is_rejected(self) -> None:
        for _, match in validate_ownership(self.configuration, self.dockerfile):
            if match["datasource"] != "docker":
                continue
            with self.subTest(image=match["depName"]):
                reference = f"{match['depName']}:{match['currentValue']}@{match['currentDigest']}"
                for replacement in (
                    f"{match['depName']}:incorrect@{match['currentDigest']}",
                    f"{match['depName']}:{match['currentValue']}",
                ):
                    with self.assertRaises(ValueError):
                        validate_ownership(
                            self.configuration, self.dockerfile.replace(reference, replacement, 1)
                        )

    def test_image_replacement_updates_comment_tag_and_digest_without_touching_aliases(
        self,
    ) -> None:
        for manager, match in validate_ownership(self.configuration, self.dockerfile):
            if match["datasource"] != "docker":
                continue
            with self.subTest(image=match["depName"]):
                values = {
                    **match.groupdict(),
                    "newValue": "99.1.2-test",
                    "newDigest": "sha256:" + "a" * 64,
                }
                replacement = re.sub(
                    r"\{\{\{([A-Za-z]+)\}\}\}",
                    lambda field, substitutions=values: substitutions[field[1]],
                    manager["autoReplaceStringTemplate"],
                )
                expected = (
                    f"# renovate: datasource=docker depName={match['depName']}\n"
                    f"# release: 99.1.2-test\nFROM {match['depName']}:99.1.2-test@sha256:"
                    + "a"
                    * 64
                )
                self.assertEqual(replacement, expected)
                source = (
                    self.dockerfile[: match.start()] + replacement + self.dockerfile[match.end() :]
                )
                validate_ownership(self.configuration, source)
                self.assertEqual(
                    re.findall(r" AS [a-z_]+", source), re.findall(r" AS [a-z_]+", self.dockerfile)
                )
                legacy = self.dockerfile.replace(
                    f"{match['depName']}:{match['currentValue']}@", f"{match['depName']}@", 1
                )
                legacy_match = next(
                    candidate
                    for _, candidate in extracted_pins(self.configuration, legacy)
                    if candidate["depName"] == match["depName"]
                )
                migrated = (
                    legacy[: legacy_match.start()] + replacement + legacy[legacy_match.end() :]
                )
                validate_ownership(self.configuration, migrated)

    def test_non_major_updates_share_one_group_without_waiving_age_or_integrity(self) -> None:
        rules = [
            rule
            for rule in self.configuration["packageRules"]
            if rule.get("description")
            == "Keep non-major Dev Container image and tool updates together"
        ]
        self.assertEqual(len(rules), 1)
        self.assertEqual(rules[0]["matchManagers"], ["custom.regex"])
        self.assertEqual(rules[0]["matchFileNames"], [DOCKERFILE_PATH])
        self.assertEqual(
            set(rules[0]["matchUpdateTypes"]), {"minor", "patch", "pin", "digest", "pinDigest"}
        )
        self.assertNotIn("minimumReleaseAge", rules[0])
        self.assertEqual(self.configuration["minimumReleaseAge"], "3 days")

    def test_all_nine_pins_require_effective_manual_review_after_generic_rules(self) -> None:
        for _, match in validate_ownership(self.configuration, self.dockerfile):
            for update_type in ("major", "minor", "patch", "pin", "digest", "pinDigest"):
                with self.subTest(dependency=match["depName"], update=update_type):
                    policy = pin_policy(self.configuration, match, update_type)
                    self.assertIs(policy["automerge"], False)
                    self.assertIs(policy["dependencyDashboardApproval"], True)
                    self.assertEqual(policy["minimumReleaseAge"], "3 days")
                    if update_type != "major":
                        self.assertEqual(policy["groupName"], "Dev Container toolchain")

    def test_missing_or_misordered_manual_rule_exposes_generic_automerge(self) -> None:
        manual = next(
            rule
            for rule in self.configuration["packageRules"]
            if rule.get("description")
            == "Require manual review of Dev Container image and tool updates"
        )
        for place_first in (False, True):
            changed = copy.deepcopy(self.configuration)
            changed["packageRules"].remove(manual)
            if place_first:
                changed["packageRules"].insert(0, copy.deepcopy(manual))
            for _, match in validate_ownership(changed, self.dockerfile):
                with self.subTest(dependency=match["depName"], misplaced=place_first):
                    self.assertIs(pin_policy(changed, match, "patch")["automerge"], True)

    def test_manual_dockerfile_review_does_not_capture_normal_cargo_dependencies(self) -> None:
        policy = effective_policy(
            self.configuration,
            {
                "matchManagers": "cargo",
                "matchDatasources": "crate",
                "matchPackageNames": "serde",
                "matchFileNames": "Cargo.toml",
                "matchDepTypes": "dependencies",
                "matchUpdateTypes": "patch",
            },
        )
        self.assertIs(policy["automerge"], True)
        self.assertIs(policy["dependencyDashboardApproval"], False)
        self.assertEqual(policy["minimumReleaseAge"], "3 days")

    def test_digest_only_update_keeps_the_exact_release_and_stage_aliases(self) -> None:
        for manager, match in validate_ownership(self.configuration, self.dockerfile):
            if match["datasource"] != "docker":
                continue
            with self.subTest(image=match["depName"]):
                values = {
                    **match.groupdict(),
                    "newValue": match["currentValue"],
                    "newDigest": "sha256:" + "b" * 64,
                }
                replacement = re.sub(
                    r"\{\{\{([A-Za-z]+)\}\}\}",
                    lambda field, substitutions=values: substitutions[field[1]],
                    manager["autoReplaceStringTemplate"],
                )
                self.assertIn(f"# release: {match['currentValue']}\n", replacement)
                self.assertTrue(
                    replacement.endswith(f":{match['currentValue']}@{values['newDigest']}")
                )
                source = (
                    self.dockerfile[: match.start()] + replacement + self.dockerfile[match.end() :]
                )
                validate_ownership(self.configuration, source)
                self.assertEqual(
                    re.findall(r" AS [a-z_]+", source), re.findall(r" AS [a-z_]+", self.dockerfile)
                )


if __name__ == "__main__":
    unittest.main()
