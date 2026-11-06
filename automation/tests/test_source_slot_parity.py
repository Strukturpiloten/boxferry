"""Independent physical source slots and adversarial local-action ownership."""

from __future__ import annotations

import hashlib
import json
import unittest
from collections import Counter
from collections.abc import Callable
from pathlib import Path
from typing import cast

from boxferry_ci import eligibility
from boxferry_ci.eligibility import expected_ci_upgrade

FIXTURES = Path(__file__).parent / "fixtures/source-slot-parity"
OLD, NEW = "a" * 40, "b" * 40  # Synthetic identities; neither is an admitted source.
ACTIONS = ("shared-node", "shared-crate-publish")


def legacy_transform() -> Callable[[bytes, str, str], bytes | None]:
    item = json.loads((FIXTURES / "index.json").read_text())["legacy_parser"]
    source = (FIXTURES / item["fixture"]).read_bytes()
    assert hashlib.sha256(source).hexdigest() == item["sha256"] == item["authority_function_sha256"]
    assert item["sha256"] == "03a7c2acabc304e1dcdbf79bf0ba0896efdf3c834efbc1710ef87e1ccc554df6"
    assert item["revision"] == "c9c6446d0934b7abb01798470f2782059e8890f7"
    assert item["unchanged_at_authority"] == "1cf96f5a667d96e66c119753cf7814e22e0dee7e"
    namespace = dict(vars(eligibility))
    # The exact first-party pure function; its parser support globals are unchanged.
    exec(compile(source, str(FIXTURES / item["fixture"]), "exec"), namespace)
    return cast(Callable[[bytes, str, str], bytes | None], namespace["expected_ci_upgrade"])


def workflow(action: str, pin: str, *, other: str = OLD) -> bytes:
    return f"""---
jobs:
  quality:
    steps:
      - uses: ./_automation/.github/actions/{action}
        with:
          automation-sha: {pin}
          candidate-sha: {other}
          validation-sha: {other}
          cache: {other}
          node-version: {other}
      - run: echo {other}
""".encode()


def replace_slots(data: bytes, slots: list[dict], before: str, after: str) -> bytes:
    """Use recorded physical positions, never discover ownership from candidate text."""
    lines = data.splitlines(keepends=True)
    for slot in slots:
        row, column = slot["line"] - 1, slot["column"]
        assert lines[row][column : column + 40] == before.encode()
        lines[row] = lines[row][:column] + after.encode() + lines[row][column + 40 :]
    return b"".join(lines)


class SourceSlotParityTests(unittest.TestCase):
    def test_recorded_seven_consumer_bytes_and_independent_expected_slots(self) -> None:
        index = json.loads((FIXTURES / "index.json").read_text())
        counts: Counter[str] = Counter()
        for consumer in index["consumers"]:
            before = consumer["before"]
            for item in consumer["files"]:
                data = (FIXTURES / item["fixture"]).read_bytes()
                self.assertEqual(hashlib.sha256(data).hexdigest(), item["sha256"])
            for item in consumer["workflows"]:
                with self.subTest(repository=consumer["repository"], path=item["path"]):
                    data = (FIXTURES / item["fixture"]).read_bytes()
                    expected = replace_slots(data, item["slots"], before, NEW)
                    self.assertEqual(expected_ci_upgrade(data, before, NEW), expected)
                    counts.update(slot["kind"] for slot in item["slots"])
                    for slot in item["unowned"]:
                        tampered = replace_slots(expected, [slot], before, NEW)
                        self.assertNotEqual(expected_ci_upgrade(data, before, NEW), tampered)
        self.assertEqual(len(index["consumers"]), 7)
        self.assertEqual(counts["shared-node"], 4)
        self.assertEqual(counts["shared-crate-publish"], 18)
        self.assertEqual(counts["shared-source"], 17)
        for item in index["presets"]:
            self.assertEqual(
                hashlib.sha256((FIXTURES / item["fixture"]).read_bytes()).hexdigest(),
                item["sha256"],
            )

    def test_exact_steps_preserve_quotes_comments_spacing_endings_and_other_values(self) -> None:
        for action in ACTIONS:
            for quote in ("", "'", '"'):
                for ending in (b"\n", b"\r\n"):
                    for final_newline in (True, False):
                        with self.subTest(
                            action=action, quote=quote, ending=ending, final=final_newline
                        ):

                            def source(
                                pin: str,
                                action: str = action,
                                quote: str = quote,
                                final_newline: bool = final_newline,
                                ending: bytes = ending,
                            ) -> bytes:
                                data = workflow(action, pin).replace(
                                    f"automation-sha: {pin}".encode(),
                                    f"automation-sha:  {quote}{pin}{quote}  # source".encode(),
                                )
                                if not final_newline:
                                    data = data.removesuffix(b"\n")
                                return data.replace(b"\n", ending)

                            self.assertEqual(
                                expected_ci_upgrade(source(OLD), OLD, NEW), source(NEW)
                            )

    def test_multiple_new_action_steps_are_replaced_without_rewriting_data(self) -> None:
        before = workflow(ACTIONS[0], OLD)
        after = workflow(ACTIONS[0], NEW)
        for action in (ACTIONS[1], ACTIONS[1]):
            before += workflow(action, OLD).split(b"    steps:\n", 1)[1]
            after += workflow(action, NEW).split(b"    steps:\n", 1)[1]
        self.assertEqual(expected_ci_upgrade(before, OLD, NEW), after)

    def test_foreign_similar_and_alternate_local_paths_have_no_authority(self) -> None:
        for action in ACTIONS:
            exact = f"./_automation/.github/actions/{action}".encode()
            for unowned in (
                f"./other/.github/actions/{action}",
                f"../_automation/.github/actions/{action}",
                f"./_automation/.github/actions/{action}-extra",
                f"./_automation/.github/actions/{action}/",
                f"attacker/boxferry/.github/actions/{action}@{OLD}",
                "./_automation/.github/actions/shared-python",
            ):
                with self.subTest(action=action, unowned=unowned):
                    self.assertIsNone(
                        expected_ci_upgrade(
                            workflow(action, OLD).replace(exact, unowned.encode()), OLD, NEW
                        )
                    )

    def test_new_actions_require_real_step_direct_mapping_and_declared_key(self) -> None:
        for action in ACTIONS:
            with self.subTest(action=action):
                valid = workflow(action, OLD)
                uses = f"uses: ./_automation/.github/actions/{action}"
                for data in (
                    (
                        f"---\njobs:\n  quality:\n    {uses}\n"
                        f"    with:\n      automation-sha: {OLD}\n"
                    ).encode(),
                    (
                        f"---\nunknown:\n  steps:\n    - {uses}\n"
                        f"      with:\n        automation-sha: {OLD}\n"
                    ).encode(),
                    valid.replace(b"automation-sha:", b"automation_sha:"),
                    valid.replace(b"        with:\n", b"        unknown:\n"),
                    valid.replace(
                        b"        with:\n", b"        with:\n          nested:\n"
                    ).replace(b"          automation-sha:", b"            automation-sha:"),
                    valid.replace(
                        f"automation-sha: {OLD}".encode(), b"automation-sha: ${{ github.sha }}"
                    ),
                    valid.replace(
                        f"automation-sha: {OLD}".encode(),
                        b"automation-sha: ${{ inputs.automation_sha }}",
                    ),
                    (
                        "---\njobs:\n  quality:\n    steps:\n      - with:\n"
                        f"          automation-sha: {OLD}\n        {uses}\n"
                    ).encode(),
                    valid.replace(
                        b"with:\n          automation-sha: " + OLD.encode(),
                        b"with: {automation-sha: " + OLD.encode() + b"}",
                    ),
                    valid.replace(
                        uses.encode(), ("uses: &action " + uses.removeprefix("uses: ")).encode()
                    ),
                    valid.replace(uses.encode(), b"uses: *action"),
                ):
                    self.assertIsNone(expected_ci_upgrade(data, OLD, NEW))

    def test_scalar_fake_steps_cannot_authorize_new_actions(self) -> None:
        for action in ACTIONS:
            for style in ("|", ">", "|-", ">+2", "!!str |", "&script |-"):
                fake = b"".join(
                    b"          " + line for line in workflow(action, OLD).splitlines(keepends=True)
                )
                data = f"---\njobs:\n  script:\n    steps:\n      - run: {style}\n".encode() + fake
                with self.subTest(action=action, style=style):
                    self.assertIsNone(expected_ci_upgrade(data, OLD, NEW))

    def test_ambiguous_or_invalid_input_fails_closed(self) -> None:
        for action in ACTIONS:
            valid = workflow(action, OLD)
            for data in (
                valid + b"jobs: {}\n",
                valid.replace(b"    steps:\n", b"    steps: []\n    steps:\n"),
                valid.replace(b"        with:\n", b"        with: {}\n        with:\n"),
                valid.replace(
                    b"        with:\n", b"        uses: attacker/action@v1\n        with:\n"
                ),
                valid.replace(
                    b"          candidate-sha:",
                    f"          automation-sha: {OLD}\n          candidate-sha:".encode(),
                ),
                valid + b'unknown: "multiline\n  data\n"\n',
                valid + b"\tunknown: data\n",
                valid + b"\0",
                valid + b"\xff",
            ):
                with self.subTest(action=action, data=data):
                    self.assertIsNone(expected_ci_upgrade(data, OLD, NEW))
            for before, after in (
                (OLD.upper(), NEW),
                (OLD, NEW.upper()),
                (OLD, OLD),
                (OLD[:-1], NEW),
            ):
                self.assertIsNone(expected_ci_upgrade(valid, before, after))

    def test_existing_shared_source_alias_and_job_context_are_preserved(self) -> None:
        for key in ("automation-sha", "automation_sha"):
            for source in (
                workflow("shared-source", OLD),
                (
                    "---\njobs:\n  quality:\n"
                    "    uses: ./_automation/.github/actions/shared-source\n"
                    f"    with:\n      automation-sha: {OLD}\n"
                ).encode(),
            ):
                data = source.replace(b"automation-sha:", f"{key}:".encode())
                expected = data.replace(f"{key}: {OLD}".encode(), f"{key}: {NEW}".encode())
                self.assertEqual(expected_ci_upgrade(data, OLD, NEW), expected)
