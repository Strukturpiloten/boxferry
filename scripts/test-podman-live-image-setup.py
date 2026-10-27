#!/usr/bin/env python3
"""Execute the nested image-setup source against a strict legacy Podman fake."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest

IMAGE_ID = "a" * 64
OTHER_ID = "b" * 64
ALIAS = "localhost/boxferry-archive/test/workload:alpine"
WORKLOAD = "localhost/boxferry-workload:alpine"
PORTABLE = "registry.example.invalid/boxferry/test:1"


def fake_podman(arguments: list[str]) -> int:
    """Model tag-only release; reject modern flags and every image-removal path."""
    root = Path(os.environ["FAKE_PODMAN_ROOT"])
    scenario = os.environ["FAKE_SCENARIO"]
    state = json.loads((root / "images.json").read_text())
    with (root / "commands.jsonl").open("a") as log:
        log.write(json.dumps(arguments) + "\n")
    if "--no-prune" in arguments:
        print("Error: unknown flag: --no-prune", file=sys.stderr)
        return 125
    if arguments == ["load", "--input", "/boxferry-workload.tar"]:
        if scenario == "load-failure":
            return 125
        state[ALIAS] = OTHER_ID if scenario == "loaded-id-mismatch" else IMAGE_ID
    elif arguments[:4] == ["image", "inspect", "--format", "{{.Id}}"] and len(arguments) == 5:
        name = arguments[4]
        if scenario == "archive-inspect-failure" and name == ALIAS:
            return 125
        if name not in state:
            return 1
        print("sha256:" + state[name])
    elif arguments[:1] == ["tag"] and len(arguments) == 3:
        source, target = arguments[1:]
        if target == WORKLOAD and scenario == "workload-tag-failure":
            return 125
        if source == IMAGE_ID:
            if IMAGE_ID not in state.values():
                return 1
            state[target] = OTHER_ID if scenario == "initial-retained-id-mismatch" else IMAGE_ID
        elif source in state:
            state[target] = state[source]
        else:
            return 1
    elif arguments == ["untag", "--", IMAGE_ID, ALIAS]:
        if state.get(ALIAS) != IMAGE_ID or state.get(WORKLOAD) != IMAGE_ID:
            return 125
        if scenario == "untag-failure":
            return 125
        if scenario != "alias-survives":
            del state[ALIAS]
        if scenario == "retained-image-missing":
            del state[WORKLOAD]
        if scenario == "retained-id-drift":
            state[WORKLOAD] = OTHER_ID
    elif arguments[:2] == ["image", "exists"] and len(arguments) == 3:
        name = arguments[2]
        if name == ALIAS:
            if scenario == "alias-query-error":
                return 125
            if scenario == "alias-query-diagnostic":
                print("Storage query failed", file=sys.stderr)
                return 1
            if scenario == "alias-query-output":
                print("Unexpected output")
                return 1
            if scenario == "alias-query-false-success":
                return 0
        elif name == WORKLOAD and scenario == "retained-exists-failure":
            return 125
        return 0 if name in state else 1
    elif arguments == ["version", "--format", "{{.Client.Version}}"]:
        print("3.0.1")
    elif arguments == ["info", "--format", "{{.Host.Security.Rootless}}"]:
        print("false")
    else:
        print("Unsupported fake Podman command: " + repr(arguments), file=sys.stderr)
        return 125
    (root / "images.json").write_text(json.dumps(state))
    return 0


class NestedImageSetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        source = (Path(__file__).parent / "podman-live-conformance.sh").read_text()
        start = '    nested_begin "prepare workload image"\n'
        end = '    nested_pass\n'
        if source.count(start) != 1:
            raise AssertionError("Expected one real nested workload image setup")
        cls.snippet = source.split(start, 1)[1].split(end, 1)[0]
        if end not in source.split(start, 1)[1]:
            raise AssertionError("Nested workload image setup has no completion boundary")

    def run_setup(self, scenario: str):
        temporary = tempfile.TemporaryDirectory(prefix="boxferry-image-setup-")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        (root / "images.json").write_text("{}")
        fake = root / "podman"
        command = shlex.join([sys.executable, str(Path(__file__).resolve()), "--fake"])
        fake.write_text('#!/bin/sh\nexec ' + command + ' "$@"\n')
        fake.chmod(0o700)
        environment = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                           FAKE_PODMAN_ROOT=str(root), FAKE_SCENARIO=scenario,
                           BF_ARCHIVE_ALIAS=ALIAS, BF_ARCHIVE_IMAGE_ID=IMAGE_ID,
                           image=WORKLOAD, portable_image=PORTABLE)
        result = subprocess.run(["/bin/sh", "-ceu", self.snippet], env=environment,
                                capture_output=True, text=True, check=False, timeout=10)
        state = json.loads((root / "images.json").read_text())
        commands = [json.loads(line) for line in (root / "commands.jsonl").read_text().splitlines()]
        self.assertFalse(any(command[:2] == ["image", "rm"] for command in commands))
        self.assertFalse(any("--force" in command or "--prune" in command for command in commands))
        return result, state, commands

    def test_legacy_cli_releases_only_archive_name_and_retains_authenticated_image(self) -> None:
        result, state, commands = self.run_setup("success")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state, {WORKLOAD: IMAGE_ID, PORTABLE: IMAGE_ID})
        self.assertEqual(commands.count(["untag", "--", IMAGE_ID, ALIAS]), 1)
        untag_index = commands.index(["untag", "--", IMAGE_ID, ALIAS])
        retained_inspect = ["image", "inspect", "--format", "{{.Id}}", WORKLOAD]
        self.assertIn(retained_inspect, commands[:untag_index])
        self.assertIn(retained_inspect, commands[untag_index + 1:])
        self.assertIn(["image", "exists", ALIAS], commands[untag_index + 1:])

    def test_unauthenticated_image_never_authorizes_untag(self) -> None:
        for scenario in ("load-failure", "archive-inspect-failure", "loaded-id-mismatch",
                         "workload-tag-failure", "initial-retained-id-mismatch"):
            with self.subTest(scenario=scenario):
                result, state, commands = self.run_setup(scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any(command[0] == "untag" for command in commands))
                self.assertNotIn(PORTABLE, state)

    def test_release_or_postcondition_failure_stops_before_portable_tag(self) -> None:
        for scenario in ("untag-failure", "alias-survives", "alias-query-error",
                         "alias-query-diagnostic", "alias-query-output", "alias-query-false-success",
                         "retained-image-missing", "retained-id-drift", "retained-exists-failure"):
            with self.subTest(scenario=scenario):
                result, state, commands = self.run_setup(scenario)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(commands.count(["untag", "--", IMAGE_ID, ALIAS]), 1)
                self.assertNotIn(PORTABLE, state)
                if scenario in ("untag-failure", "alias-survives"):
                    self.assertEqual(state[ALIAS], IMAGE_ID)
                if scenario == "untag-failure":
                    self.assertEqual(commands[-1], ["untag", "--", IMAGE_ID, ALIAS])


if __name__ == "__main__":
    if sys.argv[1:2] == ["--fake"]:
        sys.exit(fake_podman(sys.argv[2:]))
    unittest.main()
