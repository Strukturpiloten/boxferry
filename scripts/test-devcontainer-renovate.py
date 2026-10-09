#!/usr/bin/env python3
"""Check actual Dev Container pin extraction and image replacement without network access."""

from __future__ import annotations

import copy
import fnmatch
import hashlib
import io
import json
import os
import re
import subprocess
import tarfile
import tempfile
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


def extracted_pins(
    configuration: dict, dockerfile: str, path: str = DOCKERFILE_PATH
) -> list[tuple[dict, re.Match]]:
    pins = []
    for manager in configuration["customManagers"]:
        if manager["customType"] != "regex":
            continue
        if not any(
            re.search(pattern[1:-1], path) for pattern in manager["managerFilePatterns"]
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
        self.assertEqual(
            rules[0]["matchFileNames"], [DOCKERFILE_PATH, "scripts/install-kubernetes-tools.sh"]
        )
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


class KubernetesToolRenovateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.path = "scripts/install-kubernetes-tools.sh"
        self.source = (ROOT / self.path).read_text(encoding="utf-8")
        self.configuration = json.loads(
            (ROOT / ".github/renovate.json").read_text(encoding="utf-8")
        )

    def validate(self, configuration: dict, source: str) -> list[tuple[dict, re.Match]]:
        pins = extracted_pins(configuration, source, self.path)
        expected = {
            ("github-releases", "kubernetes-sigs/kind"),
            ("github-releases", "kubernetes/kubernetes"),
            ("github-releases", "helm/helm"),
            ("github-releases", "kubernetes-sigs/kustomize"),
        }
        identities = [(match["datasource"], match["depName"]) for _, match in pins]
        if len(identities) != len(expected) or set(identities) != expected:
            raise ValueError("each Kubernetes tool needs exactly one extraction owner")
        return pins

    def test_each_tool_has_one_owner_and_two_reviewed_checksums(self) -> None:
        pins = self.validate(self.configuration, self.source)
        self.assertEqual(len(pins), len(re.findall(r'^readonly \w+_version=', self.source, re.M)))
        for tool in ("kind", "kubectl", "helm", "kustomize"):
            self.assertEqual(
                len(re.findall(rf'readonly {tool}_checksum="[a-f0-9]{{64}}"', self.source)), 2
            )

    def test_missing_and_duplicate_extraction_are_rejected(self) -> None:
        for manager, match in self.validate(self.configuration, self.source):
            with self.subTest(dependency=match["depName"]):
                marker = f'# renovate: datasource={match["datasource"]} depName={match["depName"]}'
                with self.assertRaises(ValueError):
                    self.validate(self.configuration, self.source.replace(marker, "", 1))
                duplicate = copy.deepcopy(self.configuration)
                duplicate["customManagers"].append(copy.deepcopy(manager))
                with self.assertRaises(ValueError):
                    self.validate(duplicate, self.source)

    def test_kustomize_nested_release_tag_is_normalized(self) -> None:
        manager, _ = next(
            item for item in self.validate(self.configuration, self.source)
            if item[1]["depName"] == "kubernetes-sigs/kustomize"
        )
        pattern = python_pattern(manager["extractVersionTemplate"])
        self.assertEqual(re.fullmatch(pattern, "kustomize/v99.2.3")["version"], "99.2.3")
        self.assertIsNone(re.fullmatch(pattern, "kyaml/v99.2.3"))

    def test_checksum_updates_always_require_review_and_nonmajor_grouping(self) -> None:
        for _, match in self.validate(self.configuration, self.source):
            for update_type in ("major", "minor", "patch", "pin", "digest", "pinDigest"):
                with self.subTest(dependency=match["depName"], update=update_type):
                    policy = effective_policy(self.configuration, {
                        "matchManagers": "custom.regex",
                        "matchDatasources": match["datasource"],
                        "matchPackageNames": match["depName"],
                        "matchFileNames": self.path,
                        "matchDepTypes": None,
                        "matchUpdateTypes": update_type,
                    })
                    self.assertIs(policy["automerge"], False)
                    self.assertIs(policy["dependencyDashboardApproval"], True)
                    self.assertEqual(policy["minimumReleaseAge"], "3 days")
                    self.assertTrue(any("checksum" in note for note in policy["prBodyNotes"]))
                    if update_type != "major":
                        self.assertEqual(policy["groupName"], "Dev Container toolchain")


class KubernetesInstallerTests(unittest.TestCase):
    def exercise(self, machine: str, failure: str = "", system: str = "Linux") -> None:
        source = (ROOT / "scripts/install-kubernetes-tools.sh").read_text(encoding="utf-8")
        versions = dict(re.findall(r'readonly (\w+)_version="([^"]+)"', source))
        architecture = "arm64" if machine in ("arm64", "aarch64") else "amd64"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shim = root / "shim"
            shim.mkdir()
            assets = {}
            binaries = {tool: f"#!/bin/sh\necho {tool}\n".encode() for tool in versions}
            urls = {
                "kind": f'https://github.com/kubernetes-sigs/kind/releases/download/v{versions["kind"]}/kind-linux-{architecture}',
                "kubectl": f'https://dl.k8s.io/release/v{versions["kubectl"]}/bin/linux/{architecture}/kubectl',
                "helm": f'https://get.helm.sh/helm-v{versions["helm"]}-linux-{architecture}.tar.gz',
                "kustomize": f'https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v{versions["kustomize"]}/kustomize_v{versions["kustomize"]}_linux_{architecture}.tar.gz',
            }
            for tool, payload in binaries.items():
                asset = root / f"{tool}.asset"
                if tool in ("helm", "kustomize"):
                    with tarfile.open(asset, "w:gz") as archive:
                        member = tarfile.TarInfo(
                            f"linux-{architecture}/helm" if tool == "helm" else tool
                        )
                        member.size = len(payload)
                        member.mode = 0o755
                        archive.addfile(member, io.BytesIO(payload))
                else:
                    asset.write_bytes(payload)
                checksum = hashlib.sha256(asset.read_bytes()).hexdigest()
                source = re.sub(
                    rf'{tool}_checksum="[a-f0-9]+"', f'{tool}_checksum="{checksum}"', source
                )
                assets[urls[tool]] = str(asset)
            if failure == "checksum":
                (root / "helm.asset").write_bytes(b"corrupt release asset")
            installer = root / "install.sh"
            installer.write_text(source, encoding="utf-8")
            (shim / "uname").write_text(
                '#!/bin/sh\ncase "$1" in -s) echo "$TEST_SYSTEM";; -m) echo "$TEST_MACHINE";; esac\n',
                encoding="utf-8",
            )
            (shim / "curl").write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, shutil, sys\n"
                "from pathlib import Path\n"
                "with open(os.environ['TEST_LOG'], 'a') as log: log.write(sys.argv[-1] + '\\n')\n"
                "if os.environ['TEST_FAILURE'] == 'download' and 'get.helm.sh' in sys.argv[-1]: sys.exit(22)\n"
                "assets = json.loads(os.environ['TEST_ASSETS'])\n"
                "shutil.copyfile(assets[sys.argv[-1]], sys.argv[sys.argv.index('--output') + 1])\n",
                encoding="utf-8",
            )
            for executable in shim.iterdir():
                executable.chmod(0o755)
            target = root / "tools with spaces"
            target.mkdir()
            (target / "kind").write_bytes(b"existing installation")
            log = root / "downloads.log"
            result = subprocess.run(
                ["bash", str(installer), str(target)],
                env={
                    **os.environ,
                    "PATH": f"{shim}{os.pathsep}{os.environ['PATH']}",
                    "TEST_SYSTEM": system,
                    "TEST_MACHINE": machine,
                    "TEST_FAILURE": failure,
                    "TEST_ASSETS": json.dumps(assets),
                    "TEST_LOG": str(log),
                },
                capture_output=True,
                text=True,
                check=False,
            )
            unsupported = system != "Linux" or machine not in ("x86_64", "aarch64", "arm64")
            if failure or unsupported:
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(list(target.iterdir()), [target / "kind"])
                self.assertEqual((target / "kind").read_bytes(), b"existing installation")
                if unsupported:
                    self.assertFalse(log.exists(), "unsupported platforms must not download")
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(set(log.read_text().splitlines()), set(urls.values()))
                for tool, payload in binaries.items():
                    self.assertEqual((target / tool).read_bytes(), payload)
                    self.assertEqual((target / tool).stat().st_mode & 0o777, 0o755)

    def test_install_both_architectures_and_arm64_alias(self) -> None:
        for architecture in ("x86_64", "aarch64", "arm64"):
            with self.subTest(architecture=architecture):
                self.exercise(architecture)

    def test_corrupt_and_failed_downloads_preserve_existing_installation(self) -> None:
        for failure in ("checksum", "download"):
            with self.subTest(failure=failure):
                self.exercise("x86_64", failure=failure)

    def test_unsupported_platforms_fail_before_download(self) -> None:
        self.exercise("s390x")
        self.exercise("x86_64", system="Darwin")


if __name__ == "__main__":
    unittest.main()
