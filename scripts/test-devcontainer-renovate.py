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
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCKERFILE_PATH = ".devcontainer/Dockerfile"
EXPECTED_PINS = {
    ("docker", "docker.io/library/node"),
    ("docker", "mcr.microsoft.com/devcontainers/rust"),
    ("crate", "cargo-deny"),
    ("crate", "cargo-llvm-cov"),
    ("crate", "cargo-semver-checks"),
    ("crate", "zizmor"),
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
        if not any(re.search(pattern[1:-1], path) for pattern in manager["managerFilePatterns"]):
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
        key: configuration.get(key, False) for key in ("automerge", "dependencyDashboardApproval")
    }
    policy["minimumReleaseAge"] = configuration["minimumReleaseAge"]
    for rule in configuration["packageRules"]:
        selectors = {key for key in rule if key.startswith("match")}
        if selectors - dependency.keys():
            raise ValueError("policy regression must cover every production selector")

        def matches(value: str | None, patterns: list[str]) -> bool:
            if value is None:
                return False
            excluded = [pattern[1:] for pattern in patterns if pattern.startswith("!")]
            included = [pattern for pattern in patterns if not pattern.startswith("!")]
            return not any(fnmatch.fnmatchcase(value, pattern) for pattern in excluded) and (
                not included or any(fnmatch.fnmatchcase(value, pattern) for pattern in included)
            )

        if any(not matches(dependency[key], rule[key]) for key in selectors):
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


def inherited_configuration(local: dict) -> dict:
    common = json.loads((ROOT / "renovate-presets/common.json").read_text())
    rust = json.loads((ROOT / "renovate-presets/rust.json").read_text())
    common["packageRules"] += rust["packageRules"] + local["packageRules"]
    common["customManagers"] += local["customManagers"]
    common.update(
        {
            key: value
            for key, value in local.items()
            if key not in {"packageRules", "customManagers"}
        }
    )
    return common


class DevContainerRenovateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.configuration = json.loads(
            (ROOT / ".github/renovate.json").read_text(encoding="utf-8")
        )
        self.configuration = inherited_configuration(self.configuration)
        self.dockerfile = (ROOT / DOCKERFILE_PATH).read_text(encoding="utf-8")

    def test_all_operational_pins_have_one_real_extraction_owner(self) -> None:
        pins = validate_ownership(self.configuration, self.dockerfile)
        self.assertEqual(len(pins), len(EXPECTED_PINS))
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
                            self.configuration,
                            self.dockerfile.replace(reference, replacement, 1),
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
                    re.findall(r" AS [a-z_]+", source),
                    re.findall(r" AS [a-z_]+", self.dockerfile),
                )
                legacy = self.dockerfile.replace(
                    f"{match['depName']}:{match['currentValue']}@",
                    f"{match['depName']}@",
                    1,
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

    def test_devcontainer_image_updates_keep_their_group_and_age(self) -> None:
        rules = [
            rule
            for rule in self.configuration["packageRules"]
            if rule.get("description") == "Keep non-major Dev Container image updates together"
        ]
        self.assertEqual(len(rules), 1)
        self.assertEqual(rules[0]["matchManagers"], ["custom.regex"])
        self.assertEqual(rules[0]["matchFileNames"], [DOCKERFILE_PATH])
        self.assertEqual(
            set(rules[0]["matchUpdateTypes"]),
            {"minor", "patch", "pin", "digest", "pinDigest"},
        )
        self.assertNotIn("minimumReleaseAge", rules[0])
        self.assertEqual(self.configuration["minimumReleaseAge"], "3 days")
        self.assertNotIn("scripts/install-kubernetes-tools.sh", rules[0]["matchFileNames"])

    def test_all_image_context_pins_are_held_without_manual_approval(self) -> None:
        for _, match in validate_ownership(self.configuration, self.dockerfile):
            for update_type in (
                "major",
                "minor",
                "patch",
                "pin",
                "digest",
                "pinDigest",
            ):
                with self.subTest(dependency=match["depName"], update=update_type):
                    policy = pin_policy(self.configuration, match, update_type)
                    self.assertIs(policy["automerge"], True)
                    self.assertIs(policy["dependencyDashboardApproval"], False)
                    self.assertEqual(policy["minimumReleaseAge"], "3 days")
                    if update_type != "major":
                        expected = (
                            "Shared BoxFerry automation"
                            if match["depName"] == "docker.io/library/node"
                            else "Dev Container toolchain"
                        )
                        self.assertEqual(policy["groupName"], expected)

    def test_unwanted_manual_override_is_observable(self) -> None:
        changed = copy.deepcopy(self.configuration)
        changed["packageRules"].append({"matchFileNames": [DOCKERFILE_PATH], "automerge": False})
        for _, match in validate_ownership(changed, self.dockerfile):
            self.assertIs(pin_policy(changed, match, "patch")["automerge"], False)

    def test_manual_dockerfile_review_does_not_capture_normal_cargo_dependencies(
        self,
    ) -> None:
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
                    re.findall(r" AS [a-z_]+", source),
                    re.findall(r" AS [a-z_]+", self.dockerfile),
                )


class FileToolWrapperTests(unittest.TestCase):
    def test_devcontainer_copies_the_complete_canonical_tool_reader_and_callers(
        self,
    ) -> None:
        dockerfile = (ROOT / DOCKERFILE_PATH).read_text(encoding="utf-8")
        for source in (
            "scripts/install-file-tools.sh",
            "automation/src/boxferry_ci/tooling.py",
            "automation/src/boxferry_ci/git_source.py",
            "automation/tool-artifacts.json",
            "scripts/shared-ci.py",
            "scripts/install-git.sh",
        ):
            with self.subTest(source=source):
                declarations = [
                    line.split()[1:-1]
                    for line in dockerfile.splitlines()
                    if line.startswith("COPY ")
                ]
                self.assertEqual(sum(source in inputs for inputs in declarations), 1)
                self.assertTrue((ROOT / source).is_file())
        self.assertIn(
            "bash /opt/boxferry-tooling/scripts/install-file-tools.sh /usr/local/bin",
            dockerfile,
        )
        inventory = json.loads(
            (ROOT / "automation/tool-artifacts.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            {row["tool"] for row in inventory["assets"]},
            {"tombi", "shfmt", "shellcheck", "hadolint", "actionlint", "lychee"},
        )
        self.assertNotIn("depName=rhysd/actionlint", dockerfile)
        self.assertNotIn("depName=lychee", dockerfile)

    def test_wrapper_delegates_destination_profile_and_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "scripts"
            tooling = root / "automation/src/boxferry_ci"
            shim = root / "bin"
            for path in (scripts, tooling, shim):
                path.mkdir(parents=True)
            wrapper = scripts / "install-file-tools.sh"
            wrapper.write_bytes((ROOT / "scripts/install-file-tools.sh").read_bytes())
            (tooling / "tooling.py").write_text("# selected canonical implementation\n")
            (root / "automation/tool-artifacts.json").write_bytes(
                (ROOT / "automation/tool-artifacts.json").read_bytes()
            )
            python = shim / "python3"
            python.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$@" > "$TEST_LOG"\nexit "${TEST_EXIT:-0}"\n'
            )
            python.chmod(0o755)
            log = root / "arguments"
            environment = {
                **os.environ,
                "PATH": f"{shim}{os.pathsep}{os.environ['PATH']}",
                "TEST_LOG": str(log),
            }
            for arguments, destination in (
                ([], "/usr/local/bin"),
                ([str(root / "tools spaces")], str(root / "tools spaces")),
            ):
                with self.subTest(arguments=arguments):
                    result = subprocess.run(
                        ["bash", str(wrapper), *arguments],
                        env=environment,
                        capture_output=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(
                        log.read_text().splitlines(),
                        [
                            str(tooling / "tooling.py"),
                            "--destination",
                            destination,
                            "--profile",
                            "core",
                        ],
                    )
            log.unlink()
            result = subprocess.run(
                ["bash", str(wrapper), "first", "second"],
                env=environment,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 2)
            self.assertFalse(log.exists())
            result = subprocess.run(
                ["bash", str(wrapper), str(root / "tools")],
                env={**environment, "TEST_EXIT": "17"},
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 17)
            self.assertTrue(log.exists())

    def test_git_wrapper_uses_verified_source_with_explicit_candidate_inventory(
        self,
    ) -> None:
        for case in (
            "success",
            "bootstrap-refused",
            "install-refused",
            "wrong-profile",
        ):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "scripts").mkdir()
                (root / ".github").mkdir()
                (root / "automation").mkdir()
                binary = root / "bin"
                binary.mkdir()
                wrapper = root / "scripts/install-git.sh"
                wrapper.write_bytes((ROOT / "scripts/install-git.sh").read_bytes())
                (root / "scripts/shared-ci.py").write_text("# verified bootstrap fixture\n")
                manifest = json.loads((ROOT / ".github/automation.json").read_text())
                (root / ".github/automation.json").write_text(json.dumps(manifest))
                inventory = root / "automation/tool-artifacts.json"
                inventory.write_bytes((ROOT / "automation/tool-artifacts.json").read_bytes())
                selected = root / "target/boxferry-automation" / manifest["revision"] / "automation"
                selected.mkdir(parents=True)
                # The admitted producer deliberately has no operational Git row.
                (selected / "tool-artifacts.json").write_text('{"schema":2,"sources":[]}')
                log = root / "calls.jsonl"
                prefix = root / "complete prefix"
                python = binary / "python3"
                python.write_text(
                    f"#!{sys.executable}\nimport json,os,sys\n"
                    f"with open({str(log)!r},'a') as stream: stream.write(json.dumps(['python3',*sys.argv[1:]])+'\\n')\n"
                    "if sys.argv[1].endswith('shared-ci.py'):\n"
                    f" raise SystemExit({19 if case == 'bootstrap-refused' else 0})\n"
                    f"os.execv({sys.executable!r},[{sys.executable!r},*sys.argv[1:]])\n"
                )
                uv = binary / "uv"
                git_payload = (
                    "#!/bin/sh\nprintf '%s\\n' 'rust: "
                    + ("enabled" if case == "wrong-profile" else "disabled")
                    + "'\n"
                )
                uv.write_text(
                    f"#!{sys.executable}\n"
                    "import json,os,sys,types\nfrom pathlib import Path\n"
                    f"with open({str(log)!r},'a') as stream: stream.write(json.dumps(['uv',*sys.argv[1:]])+'\\n')\n"
                    "class ToolError(ValueError): pass\n"
                    "package=types.ModuleType('boxferry_ci'); package.__path__=[]\n"
                    "installer=types.ModuleType('boxferry_ci.git_install')\n"
                    "def load_source(path):\n"
                    f" assert path == Path({str(inventory)!r}), 'must use candidate inventory'\n"
                    " return json.loads(path.read_text())['sources'][0]\n"
                    "def install(prefix, source, *, execute):\n"
                    f" if {case == 'install-refused'!r}: raise ToolError('controlled source integrity refusal')\n"
                    " assert source == load_source(Path(sys.argv[-1]))\n"
                    " prefix.joinpath('bin').mkdir(parents=True)\n"
                    " git=prefix/'bin/git'\n"
                    f" git.write_text({git_payload!r})\n"
                    " git.chmod(0o755)\n"
                    " result=execute(('controlled-prerequisite',),cwd=prefix,stage='fixture',timeout_seconds=17)\n"
                    " assert result.status == 'success'\n"
                    " return {'complete_prefix':str(prefix)}\n"
                    "def run(*args,**kwargs):\n"
                    " assert args == (('controlled-prerequisite',),) and kwargs['timeout_seconds']==17\n"
                    " return types.SimpleNamespace(status='success',stdout='bounded prerequisite output\\n',stderr='')\n"
                    "installer.install=install;installer.load_source=load_source\n"
                    "package.runner=types.SimpleNamespace(run=run);package.tooling=types.SimpleNamespace(ToolError=ToolError)\n"
                    "sys.modules['boxferry_ci']=package;sys.modules['boxferry_ci.git_install']=installer\n"
                    "program=sys.stdin.read();sys.argv=['-',*sys.argv[-2:]]\n"
                    "exec(compile(program,'canonical-wrapper-input','exec'))\n"
                )
                for executable in (python, uv):
                    executable.chmod(0o755)
                result = subprocess.run(
                    ["bash", str(wrapper), str(prefix)],
                    env={**os.environ, "PATH": str(binary) + ":" + os.environ["PATH"]},
                    capture_output=True,
                    text=True,
                    timeout=15,
                    check=False,
                )
                calls = [json.loads(line) for line in log.read_text().splitlines()]
                if case == "bootstrap-refused":
                    self.assertEqual(result.returncode, 19)
                    self.assertEqual(len(calls), 1)
                    self.assertFalse(prefix.exists())
                    continue
                self.assertEqual(
                    calls[-1],
                    [
                        "uv",
                        "run",
                        "--locked",
                        "--project",
                        str(selected),
                        "python",
                        "-I",
                        "-",
                        str(prefix),
                        str(inventory),
                    ],
                )
                if case == "success":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("bounded prerequisite output", result.stdout)
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("Canonical Git image installation failed", result.stderr)
                    self.assertIn(
                        "integrity refusal" if case == "install-refused" else "C build profile",
                        result.stderr,
                    )


class KubernetesToolRenovateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.path = "scripts/install-kubernetes-tools.sh"
        self.source = (ROOT / self.path).read_text(encoding="utf-8")
        self.configuration = json.loads(
            (ROOT / ".github/renovate.json").read_text(encoding="utf-8")
        )
        self.configuration = inherited_configuration(self.configuration)

    def validate(self, configuration: dict, source: str) -> list[tuple[dict, re.Match]]:
        pins = extracted_pins(configuration, source, self.path)
        expected = Counter(
            [
                ("github-release-attachments", "kubernetes-sigs/kind"),
                ("github-release-attachments", "kubernetes-sigs/kind"),
                ("github-releases", "kubernetes/kubernetes"),
                ("github-releases", "helm/helm"),
                ("github-releases", "kubernetes-sigs/kustomize"),
            ]
        )
        identities = Counter(
            (
                match.groupdict().get("datasource", manager.get("datasourceTemplate")),
                match["depName"],
            )
            for manager, match in pins
        )
        if identities != expected:
            raise ValueError("kind needs two architecture owners and other tools one each")
        kind_pins = [match for _, match in pins if match["depName"] == "kubernetes-sigs/kind"]
        if {match["depType"] for match in kind_pins} != {
            "kind-linux-amd64",
            "kind-linux-arm64",
        }:
            raise ValueError("kind asset owners must cover both Linux architectures")
        if len({match["currentValue"] for match in kind_pins}) != 1:
            raise ValueError("kind architecture owners must share one release tag")
        return pins

    def test_each_tool_has_one_owner_and_two_reviewed_checksums(self) -> None:
        pins = self.validate(self.configuration, self.source)
        self.assertEqual(
            len(pins),
            len(re.findall(r"^readonly \w+_version=", self.source, re.MULTILINE)) + 2,
        )
        for tool in ("kubectl", "helm", "kustomize"):
            self.assertEqual(
                len(re.findall(rf'readonly {tool}_checksum="[a-f0-9]{{64}}"', self.source)),
                2,
            )
        kind_records = re.findall(
            r'(?m)^readonly kind_(amd64|arm64)_release="(v[0-9.]+)" '
            r'kind_\1_checksum="([a-f0-9]{64})"$',
            self.source,
        )
        self.assertEqual({record[0] for record in kind_records}, {"amd64", "arm64"})
        self.assertEqual(len({record[1] for record in kind_records}), 1)
        self.assertEqual(len(kind_records), 2)

    def test_missing_and_duplicate_extraction_are_rejected(self) -> None:
        for manager, match in self.validate(self.configuration, self.source):
            with self.subTest(
                dependency=match["depName"], dep_type=match.groupdict().get("depType")
            ):
                datasource = match.groupdict().get("datasource") or manager.get(
                    "datasourceTemplate"
                )
                marker = f"# renovate: datasource={datasource} depName={match['depName']}"
                with self.assertRaises(ValueError):
                    self.validate(self.configuration, self.source.replace(marker, "", 1))
                duplicate = copy.deepcopy(self.configuration)
                duplicate["customManagers"].append(copy.deepcopy(manager))
                with self.assertRaises(ValueError):
                    self.validate(duplicate, self.source)

    def test_kustomize_nested_release_tag_is_normalized(self) -> None:
        manager, _ = next(
            item
            for item in self.validate(self.configuration, self.source)
            if item[1]["depName"] == "kubernetes-sigs/kustomize"
        )
        pattern = python_pattern(manager["extractVersionTemplate"])
        self.assertEqual(re.fullmatch(pattern, "kustomize/v99.2.3")["version"], "99.2.3")
        self.assertIsNone(re.fullmatch(pattern, "kyaml/v99.2.3"))

    def test_scoped_kubernetes_updates_auto_merge_without_dashboard_approval(
        self,
    ) -> None:
        for manager, match in self.validate(self.configuration, self.source):
            for update_type in (
                "major",
                "minor",
                "patch",
                "pin",
                "digest",
                "pinDigest",
            ):
                with self.subTest(dependency=match["depName"], update=update_type):
                    datasource = match.groupdict().get("datasource") or manager.get(
                        "datasourceTemplate"
                    )
                    policy = effective_policy(
                        self.configuration,
                        {
                            "matchManagers": "custom.regex",
                            "matchDatasources": datasource,
                            "matchPackageNames": match["depName"],
                            "matchFileNames": self.path,
                            "matchDepTypes": match.groupdict().get("depType"),
                            "matchUpdateTypes": update_type,
                        },
                    )
                    self.assertIs(policy["automerge"], True)
                    self.assertIs(policy["dependencyDashboardApproval"], False)
                    self.assertEqual(policy["minimumReleaseAge"], "3 days")
                    self.assertEqual(policy["groupName"], "Kubernetes development tools")


class KubernetesInstallerTests(unittest.TestCase):
    def exercise(self, machine: str, failure: str = "", system: str = "Linux") -> None:
        source = (ROOT / "scripts/install-kubernetes-tools.sh").read_text(encoding="utf-8")
        versions = dict(re.findall(r'readonly (\w+)_version="([^"]+)"', source))
        kind_tags = dict(
            re.findall(r'(?m)^readonly kind_(amd64|arm64)_release="(v[0-9.]+)"', source)
        )
        self.assertEqual(set(versions), {"kubectl", "helm", "kustomize"})
        self.assertEqual(set(kind_tags), {"amd64", "arm64"})
        architecture = "arm64" if machine in ("arm64", "aarch64") else "amd64"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shim = root / "shim"
            shim.mkdir()
            assets = {}
            binaries = {tool: f"#!/bin/sh\necho {tool}\n".encode() for tool in ("kind", *versions)}
            urls = {
                "kind": f"https://github.com/kubernetes-sigs/kind/releases/download/{kind_tags[architecture]}/kind-linux-{architecture}",
                "kubectl": f"https://dl.k8s.io/release/v{versions['kubectl']}/bin/linux/{architecture}/kubectl",
                "helm": f"https://get.helm.sh/helm-v{versions['helm']}-linux-{architecture}.tar.gz",
                "kustomize": f"https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v{versions['kustomize']}/kustomize_v{versions['kustomize']}_linux_{architecture}.tar.gz",
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
                if tool == "kind":
                    source, count = re.subn(
                        rf'(?m)^(readonly kind_{architecture}_release="[^"]+" '
                        rf'kind_{architecture}_checksum=")[a-f0-9]+("$)',
                        rf"\g<1>{checksum}\2",
                        source,
                        count=1,
                    )
                    self.assertEqual(count, 1)
                else:
                    source = re.sub(
                        rf'{tool}_checksum="[a-f0-9]+"',
                        f'{tool}_checksum="{checksum}"',
                        source,
                    )
                assets[urls[tool]] = str(asset)
            if failure == "checksum":
                (root / "helm.asset").write_bytes(b"corrupt release asset")
            installer = root / "install.sh"
            installer.write_text(source, encoding="utf-8")
            (shim / "uname").write_text(
                '#!/bin/sh\ncase "$1" in -s) echo "$TEST_SYSTEM";; '
                '-m) echo "$TEST_MACHINE";; esac\n',
                encoding="utf-8",
            )
            (shim / "curl").write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, shutil, sys\n"
                "from pathlib import Path\n"
                "with open(os.environ['TEST_LOG'], 'a') as log: log.write(sys.argv[-1] + '\\n')\n"
                "if os.environ['TEST_FAILURE'] == 'download' and 'get.helm.sh' in sys.argv[-1]:\n"
                " sys.exit(22)\n"
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
            unsupported = system != "Linux" or machine not in (
                "x86_64",
                "aarch64",
                "arm64",
            )
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


class SingleCloneLifecycleTests(unittest.TestCase):
    def test_default_config_uses_vscode_mount_and_only_existing_volumes(self) -> None:
        config = json.loads((ROOT / ".devcontainer/devcontainer.json").read_text())
        self.assertNotIn("workspaceMount", config)
        self.assertNotIn("workspaceFolder", config)
        self.assertEqual(
            config["mounts"],
            [
                "source=boxferry-cargo-${devcontainerId},target=/workspaces/.boxferry-cargo,type=volume",
                "source=boxferry-gh-${devcontainerId},target=/workspaces/.boxferry-gh,type=volume",
            ],
        )
        self.assertEqual(config["postCreateCommand"], "bash .devcontainer/verify-tools.sh")

    def exercise(self, case: str) -> tuple[subprocess.CompletedProcess[str], list[list[str]]]:
        import tomllib

        with tempfile.TemporaryDirectory(prefix="boxferry single clone ") as temporary:
            fixture = Path(temporary)
            binary = fixture / "bin"
            binary.mkdir()
            log = fixture / "calls.jsonl"
            cargo, github = fixture / "cargo", fixture / "gh"
            cargo.mkdir()
            github.mkdir(mode=0o755)
            checkout, tooling = fixture / "checkout", fixture / "image-tooling"
            for name in (
                ".github/automation.json",
                ".devcontainer/Dockerfile",
                "rust-toolchain.toml",
                "automation/tool-artifacts.json",
                "scripts/git-worktree.py",
            ):
                target = checkout / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((ROOT / name).read_bytes())
            shutil.copytree(
                ROOT / "automation/src/boxferry_ci",
                checkout / "automation/src/boxferry_ci",
                ignore=shutil.ignore_patterns("__pycache__"),
            )
            real_git = shutil.which("git")
            assert real_git is not None
            git_environment = {
                key: value for key, value in os.environ.items() if not key.startswith("GIT_")
            }
            git_environment.update(
                GIT_CONFIG_NOSYSTEM="1",
                GIT_CONFIG_GLOBAL=os.devnull,
                GIT_CONFIG_SYSTEM=os.devnull,
                GIT_CONFIG_COUNT="0",
            )
            fixture_git = [
                real_git,
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "core.fsmonitor=false",
                "-C",
                str(checkout),
            ]
            for args in (
                ("init", "--initial-branch=main"),
                ("config", "user.name", "Lifecycle fixture"),
                ("config", "user.email", "fixture@example.invalid"),
            ):
                subprocess.run(
                    [*fixture_git, *args],
                    env=git_environment,
                    check=True,
                    capture_output=True,
                )
            subprocess.run([*fixture_git, "add", "."], env=git_environment, check=True)
            subprocess.run(
                [*fixture_git, "commit", "-m", "fixture"],
                env=git_environment,
                check=True,
                capture_output=True,
            )
            prefix = fixture / "git-prefix"
            for name in (
                "libexec/git-core/git-remote-http",
                "libexec/git-core/git-remote-https",
                "libexec/git-core/git-http-fetch",
                "libexec/git-core/git-http-push",
                "libexec/git-core/git-send-email",
                "share/perl5/Git.pm",
                "share/git-core/templates/info/exclude",
                "share/doc/git/COPYING",
            ):
                path = prefix / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"controlled runtime fixture\n")
            source = json.loads((checkout / "automation/tool-artifacts.json").read_text())[
                "sources"
            ][0]
            provenance = {
                "source": {
                    "version": source["currentVersion"],
                    "url": source["url"],
                    "sha256": source["sha256"],
                },
                "license": {
                    "sha256": hashlib.sha256(
                        (prefix / "share/doc/git/COPYING").read_bytes()
                    ).hexdigest()
                },
            }
            if case == "wrong-git-source":
                provenance["source"]["sha256"] = "0" * 64
            (prefix / "share/doc/git/boxferry-source.json").write_text(json.dumps(provenance))
            if case == "wrong-git-license":
                (prefix / "share/doc/git/COPYING").write_text("changed")
            if case == "missing-git-helper":
                (prefix / "libexec/git-core/git-remote-https").unlink()
            manifest = json.loads((checkout / ".github/automation.json").read_text())
            (tooling / ".github").mkdir(parents=True)
            retained = {**manifest, "revision": "f" * 40} if case == "wrong-authority" else manifest
            (tooling / ".github/automation.json").write_text(json.dumps(retained))
            project = tooling / "target/boxferry-automation" / manifest["revision"] / "automation"
            project.mkdir(parents=True)
            project.joinpath("pyproject.toml").write_bytes(
                (ROOT / "automation/pyproject.toml").read_bytes()
            )
            metadata = tomllib.loads((project / "pyproject.toml").read_text())
            versions = {
                name: next(
                    item.split("==", 1)[1]
                    for item in metadata["dependency-groups"]["dev"]
                    if item.startswith(name + "==")
                )
                for name in ("ruff", "ty")
            }
            versions["git"] = source["currentVersion"]
            versions["uv"] = metadata["tool"]["uv"]["required-version"][2:]
            node = re.search(
                r"FROM docker\.io/library/node:([0-9]+\.[0-9]+\.[0-9]+)-",
                (checkout / ".devcontainer/Dockerfile").read_text(),
            )
            self.assertIsNotNone(node)
            versions["node"] = node[1]
            channel = tomllib.loads((checkout / "rust-toolchain.toml").read_text())["toolchain"][
                "channel"
            ]
            versions["rustc"] = channel
            tools = (
                "actionlint cargo cargo-clippy cargo-deny cargo-llvm-cov cargo-semver-checks "
                "curl gh git git-system perl hadolint helm jq kind kubectl kustomize lychee markdownlint-cli2 "
                "node npm prettier ruff rustc rustfmt rustup shellcheck shfmt "
                "tombi ty uv zizmor sudo podman"
            ).split()
            program = (
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "from pathlib import Path\n"
                f"versions = {versions!r}\n"
                "name, args = Path(sys.argv[0]).name, sys.argv[1:]\n"
                f"case = {case!r}\n"
                f"with open({str(log)!r}, 'a') as stream:\n"
                "    stream.write(json.dumps([name, *args]) + '\\n')\n"
                "if name in ('sudo', 'podman'):\n"
                "    raise SystemExit(99)\n"
                "if name in ('git', 'git-system'):\n"
                "    if args == ['--version', '--build-options']:\n"
                "        print('git version ' + versions['git'])\n"
                "        print('rust: enabled' if case == 'wrong-git-profile' else 'rust: disabled')\n"
                "    elif args == ['--exec-path']:\n"
                f"        print('/unrelated' if case == 'wrong-git-prefix' else {str(prefix / 'libexec/git-core')!r})\n"
                "    elif args == ['--version']:\n"
                "        version = '0.0.0' if case == 'wrong-' + name else versions['git']\n"
                "        print('git version ' + version)\n"
                "    elif args[-2:] == ['rev-parse', '--show-toplevel'] and case in ('missing-checkout', 'wrong-root'):\n"
                "        if case == 'missing-checkout': raise SystemExit(128)\n"
                f"        print(Path({str(checkout)!r}).parent)\n"
                "    else:\n"
                f"        os.execv({real_git!r}, [{real_git!r}, *args])\n"
                "elif name == 'rustup' and args == ['component', 'list', '--installed']:\n"
                "    print('clippy-x86_64-unknown-linux-gnu')\n"
                "    if case != 'missing-llvm':\n"
                "        print('llvm-tools-preview-x86_64-unknown-linux-gnu')\n"
                "elif name in versions and args == ['--version']:\n"
                "    version = '0.0.0' if case == 'wrong-' + name else versions[name]\n"
                "    actual = ('v' + version) if name == 'node' else name + ' ' + version\n"
                "    print(actual + (' (fixture)' if name == 'rustc' else ''))\n"
                "else:\n"
                "    print('fixture client version')\n"
            )
            for name in tools:
                tool = binary / name
                tool.write_text(program)
                tool.chmod(0o755)
            # Keep the fixture's interpreter explicit: hosted /usr/bin/python3 can
            # predate the locked project while this test already runs under it.
            (binary / "python3").symlink_to(sys.executable)
            script = checkout / ".devcontainer/verify-tools.sh"
            script.write_text(
                (ROOT / ".devcontainer/verify-tools.sh")
                .read_text()
                .replace('tooling_root="/opt/boxferry-tooling"', f'tooling_root="{tooling}"')
                .replace(
                    'prefix = Path("/opt/boxferry-git")',
                    f"prefix = Path({str(prefix)!r})",
                )
                .replace('"/usr/bin/git"', repr(str(binary / "git-system")))
            )
            environment = {
                **os.environ,
                "PATH": str(binary) + ":/usr/bin:/bin",
                "CARGO_HOME": str(cargo),
                "GH_CONFIG_DIR": str(github),
                "TOOL_CALLS": str(log),
                "EXPECTED_CHECKOUT": str(checkout),
                "LIFECYCLE_CASE": case,
            }
            environment.pop("GIT_DIR", None)
            environment.pop("GIT_WORK_TREE", None)
            completed = subprocess.run(
                ["bash", str(script)],
                cwd=fixture,
                env=environment,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertFalse(any(call[0] in {"sudo", "podman"} for call in calls))
            self.assertEqual(stat.S_IMODE(github.stat().st_mode), 0o700)
            return completed, calls

    def test_own_worktree_passes_without_sibling_clones_or_runtime_commands(
        self,
    ) -> None:
        completed, calls = self.exercise("valid")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(
            any(
                len(call) >= 5
                and call[0] == "git"
                and call[-4] == "-C"
                and call[-3].endswith("/checkout")
                and call[-2:] == ["rev-parse", "--show-toplevel"]
                for call in calls
            )
        )
        self.assertIn("tooling is ready", completed.stdout)

    def test_fixture_setup_cannot_route_writes_or_hooks_into_ambient_repository(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            outside = Path(temporary)
            marker = outside / "protected"
            marker.write_bytes(b"unrelated")
            configuration = outside / "global-git-config"
            configuration.write_text(
                "[core]\n hooksPath = /unavailable-user-hooks\n[commit]\n gpgSign = true\n"
            )
            from unittest.mock import patch

            with patch.dict(
                os.environ,
                {
                    "GIT_DIR": str(outside / "metadata-that-must-not-be-created"),
                    "GIT_WORK_TREE": str(outside),
                    "GIT_CONFIG_GLOBAL": str(configuration),
                },
            ):
                completed, _calls = self.exercise("valid")
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(marker.read_bytes(), b"unrelated")
            self.assertEqual(
                sorted(path.name for path in outside.iterdir()), ["global-git-config", "protected"]
            )

    def test_missing_or_misbound_own_checkout_fails(self) -> None:
        for case in ("missing-checkout", "wrong-root"):
            with self.subTest(case=case):
                completed, _calls = self.exercise(case)
                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("BoxFerry Git source check failed", completed.stderr)

    def test_existing_missing_component_failure_is_retained(self) -> None:
        completed, _calls = self.exercise("missing-llvm")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("missing Rust component: llvm-tools-preview", completed.stderr)

    def test_wrong_selected_tool_or_retained_authority_requires_rebuild(self) -> None:
        for case in (
            "wrong-uv",
            "wrong-ruff",
            "wrong-ty",
            "wrong-node",
            "wrong-rustc",
            "wrong-authority",
            "wrong-git",
            "wrong-git-system",
            "wrong-git-prefix",
            "wrong-git-profile",
            "wrong-git-source",
            "wrong-git-license",
            "missing-git-helper",
        ):
            with self.subTest(case=case):
                completed, _calls = self.exercise(case)
                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("tool verification failed", completed.stderr)


if __name__ == "__main__":
    unittest.main()
