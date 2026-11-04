from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from boxferry_ci.contracts import ContractError, object_value
from boxferry_ci.markdown import MAX_MARKDOWN_BYTES, code_projection, unchanged_code
from boxferry_ci.planner import make_plan, parse_diff
from boxferry_ci.policy import Policy, load_policy


class Repository(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.command("init", "--initial-branch=main")
        self.command("config", "user.email", "fixture@example.invalid")
        self.command("config", "user.name", "Fixture")
        self.config = self.root / "scripts/validation-policy.json"
        self.config.parent.mkdir()
        self.config.write_text(
            json.dumps(
                {
                    "schema": 2,
                    "repository": "Strukturpiloten/example",
                    "jobs": ["rust", "documentation", "python"],
                    "profile_jobs": {
                        "prose": ["documentation"],
                        "executable-docs": ["documentation"],
                        "python": ["python"],
                    },
                    "public_documentation_root": "docs/public/",
                    "full_documentation_roots": ["docs/public/development/"],
                    "executable_documentation_roots": ["docs/public/tutorials/"],
                    "executable_documentation_paths": ["fixtures/documentation.yaml"],
                    "documentation_examples_manifest": None,
                    "local_commands": {
                        profile: [["true"]]
                        for profile in (
                            "prose",
                            "executable-docs",
                            "python",
                            "full",
                        )
                    },
                    "path_profiles": {"python": ["automation/*.py"]},
                }
            )
        )
        self.write("docs/public/guide.md", "A plain paragraph.\n")
        self.write("src/lib.rs", "fn initial() {}\n")
        self.base = self.commit()
        self.policy = load_policy(self.config)

    def command(self, *args: str) -> str:
        result = subprocess.run(
            ("git", *args),
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout.strip()

    def write(self, path: str, content: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)

    def commit(self) -> str:
        self.command("add", "--all")
        self.command("commit", "--quiet", "--allow-empty", "-m", "fixture")
        return self.command("rev-parse", "HEAD")

    def plan(
        self, *, event: str = "pull_request", policy: Policy | None = None
    ) -> dict[str, object]:
        head = self.command("rev-parse", "HEAD")
        return make_plan(
            self.root,
            policy or self.policy,
            event=event,
            base_sha=self.base,
            head_sha=head,
            tested_sha=head,
            policy_sha=self.base,
        )

    def maintainer_policy(
        self,
        content: str | None,
        *,
        paths: tuple[str, ...] = ("README.md",),
        extra: dict[str, object] | None = None,
    ) -> None:
        raw = json.loads(self.config.read_text())
        raw["path_profiles"]["prose"] = list(paths)
        if extra is not None:
            raw.update(extra)
        self.config.write_text(json.dumps(raw))
        if content is not None:
            self.write("README.md", content)
        self.base = self.commit()
        self.policy = load_policy(self.config)

    def test_allowlisted_readme_prose_around_unchanged_command_skips_rust(self) -> None:
        before = (
            "# Project\n\nOld explanation.\n\n- A prose list item.\n\n"
            "```sh\ncargo test --locked\n```\n\n[Guide](docs/guide.md).\n"
        )
        self.maintainer_policy(before)
        self.write(
            "README.md",
            before.replace("Old explanation.", "A clearer explanation.").replace(
                "A prose list item.", "A clearer prose list item."
            ),
        )
        self.commit()
        plan = self.plan()
        self.assertEqual(plan["profiles"], ["prose"])
        self.assertFalse(object_value(plan["jobs"], "jobs")["rust"])

    def test_actual_root_readme_prose_change_preserves_all_existing_code(self) -> None:
        source = (Path(__file__).resolve().parents[2] / "README.md").read_bytes()
        prose = next(
            line
            for line in source.splitlines(keepends=True)
            if line.startswith(b"BoxFerry ") and b"`" not in line and b"<" not in line
        )
        modified = source.replace(prose, b"The BoxFerry project " + prose[len(b"BoxFerry ") :], 1)
        self.assertNotEqual(source, modified)
        self.assertIsNotNone(code_projection(source))
        self.maintainer_policy(source.decode("utf-8"))
        self.write("README.md", modified.decode("utf-8"))
        self.commit()
        plan = self.plan()
        self.assertEqual(plan["profiles"], ["prose"])
        self.assertFalse(object_value(plan["jobs"], "jobs")["rust"])

    def test_allowlisted_readme_command_body_language_and_delimiter_changes_are_full(self) -> None:
        before = "Explanation.\n\n```sh\necho original\n```\n"
        self.maintainer_policy(before)
        for after in (
            before.replace("echo original", "echo changed"),
            before.replace("```sh", "```bash"),
            before.replace("```sh", "```sh,no_run"),
            before.replace("```\n", "````\n"),
            "Explanation only.\n",
            before + "\n`new executable example`\n",
        ):
            self.command("reset", "--hard", self.base)
            self.write("README.md", after)
            self.commit()
            with self.subTest(after=after):
                self.assertEqual(self.plan()["profiles"], ["full"])

    def test_public_executable_docs_keep_focused_examples_even_with_prose_override(self) -> None:
        self.maintainer_policy(None, paths=("docs/public/guide.md",))
        self.write("docs/public/guide.md", "```sh\ncargo test\n```\n")
        self.base = self.commit()
        self.write("docs/public/guide.md", "New explanation.\n\n```sh\ncargo test\n```\n")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["executable-docs"])

    def test_full_roots_and_executable_roots_paths_and_pages_precede_prose(self) -> None:
        cases: tuple[tuple[str, dict[str, object], str], ...] = (
            ("docs/public/development/guide.md", {}, "full"),
            ("docs/public/tutorials/guide.md", {}, "executable-docs"),
            ("fixtures/documentation.yaml", {}, "executable-docs"),
            (
                "docs/owned-example.md",
                {"executable_documentation_paths": ["docs/owned-example.md"]},
                "executable-docs",
            ),
            (
                "docs/manifest-example.md",
                {"documentation_examples_manifest": "docs/examples.toml"},
                "executable-docs",
            ),
        )
        for path, overrides, expected in cases:
            self.command("reset", "--hard", self.base)
            if overrides.get("documentation_examples_manifest"):
                self.write("docs/examples.toml", f'schema = 1\n[[examples]]\npages = ["{path}"]\n')
            self.write(path, "Old prose.\n")
            self.maintainer_policy(None, paths=(path,), extra=overrides)
            self.write(path, "New prose.\n")
            self.commit()
            with self.subTest(path=path):
                self.assertEqual(self.plan()["profiles"], [expected])

    def test_unknown_markdown_and_broad_prose_globs_do_not_gain_projection_exception(self) -> None:
        self.maintainer_policy("Old prose.\n", paths=("*.md",))
        self.write("README.md", "New prose.\n")
        self.write("UNREVIEWED.md", "Another prose file.\n")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["full"])

    def test_source_policy_and_root_or_nested_uv_lock_never_use_configured_prose(self) -> None:
        for path in (
            "src/lib.rs",
            "uv.lock",
            "automation/uv.lock",
            "scripts/validation-policy.json",
        ):
            self.command("reset", "--hard", self.base)
            raw = json.loads(self.config.read_text())
            raw["path_profiles"] = {"prose": [path], "python": [path]}
            self.config.write_text(json.dumps(raw))
            self.base = self.commit()
            self.policy = load_policy(self.config)
            if path == "scripts/validation-policy.json":
                self.config.write_text(self.config.read_text() + "\n")
                self.commit()
                self.command("checkout", self.base, "--", path)
            else:
                self.write(path, "changed\n")
                self.commit()
            with self.subTest(path=path):
                self.assertEqual(self.plan()["profiles"], ["full"])

    def test_added_deleted_and_renamed_maintainer_code_remain_full(self) -> None:
        content = "```sh\ncargo test\n```\n"
        self.maintainer_policy(None, paths=("README.md", "MAINTAINERS.md"))
        self.write("README.md", content)
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["full"])
        self.base = self.command("rev-parse", "HEAD")
        self.command("rm", "README.md")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["full"])
        self.command("reset", "--hard", self.base)
        self.command("mv", "README.md", "MAINTAINERS.md")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["full"])

    def test_local_new_staged_code_has_no_historical_pair(self) -> None:
        self.maintainer_policy(None)
        self.write("README.md", "```sh\ncargo test\n```\n")
        self.command("add", "README.md")
        self.assertEqual(
            make_plan(self.root, self.policy, event="local", base_ref="main")["profiles"], ["full"]
        )

    def test_local_staged_code_change_cannot_be_hidden_by_restored_working_copy(self) -> None:
        before = "Use `cargo test` here.\n"
        self.maintainer_policy(before)
        self.write("README.md", "Use `cargo test --all` here.\n")
        self.command("add", "README.md")
        self.write("README.md", "A clearer explanation of `cargo test` here.\n")
        self.assertEqual(
            make_plan(self.root, self.policy, event="local", base_ref="main")["profiles"], ["full"]
        )

    def test_maintainer_prose_and_python_changes_keep_known_union(self) -> None:
        self.maintainer_policy("Use `cargo test`.\n")
        self.write("README.md", "Run `cargo test` during development.\n")
        self.write("automation/check.py", "print('fixture')\n")
        self.commit()
        plan = self.plan()
        self.assertEqual(plan["profiles"], ["prose", "python"])
        self.assertFalse(object_value(plan["jobs"], "jobs")["rust"])

    def test_prose_never_selects_rust(self) -> None:
        self.write("docs/public/guide.md", "A revised plain paragraph.\n")
        self.commit()
        plan = self.plan()
        self.assertEqual(plan["profiles"], ["prose"])
        self.assertEqual(plan["jobs"], {"rust": False, "documentation": True, "python": False})

    def test_all_executable_markdown_forms_select_focused_examples(self) -> None:
        for content in (
            "```sh\necho hello\n```",
            "    echo hello",
            " \techo hello",
            "`cargo test`",
            "<pre>run</pre>",
        ):
            with self.subTest(content=content):
                self.command("reset", "--hard", self.base)
                self.write("docs/public/guide.md", content)
                self.commit()
                self.assertEqual(self.plan()["profiles"], ["executable-docs"])
                self.assertFalse(object_value(self.plan()["jobs"], "jobs")["rust"])

    def test_deleted_executable_content_remains_executable(self) -> None:
        self.write("docs/public/guide.md", "```sh\nrun\n```\n")
        self.base = self.commit()
        self.write("docs/public/guide.md", "Only prose now.\n")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["executable-docs"])

    def test_mixed_known_scopes_take_union(self) -> None:
        self.write("docs/public/guide.md", "A revised paragraph.\n")
        self.write("automation/check.py", "print('fixture')\n")
        self.commit()
        plan = self.plan()
        self.assertEqual(plan["profile"], "mixed")
        self.assertEqual(plan["profiles"], ["prose", "python"])
        self.assertEqual(plan["jobs"], {"rust": False, "documentation": True, "python": True})

    def test_source_unknown_and_dependency_changes_require_full(self) -> None:
        for path in ("src/lib.rs", "Cargo.lock", "unknown.txt", "automation/uv.lock"):
            with self.subTest(path=path):
                self.command("reset", "--hard", self.base)
                self.write(path, "changed\n")
                self.commit()
                self.assertEqual(self.plan()["profiles"], ["full"])

    def test_renaming_source_to_documentation_does_not_hide_source(self) -> None:
        self.command("mv", "src/lib.rs", "docs/public/source.md")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["full"])

    def test_queue_classifies_all_commits_in_candidate(self) -> None:
        self.write("src/lib.rs", "fn queue_member_one() {}\n")
        self.commit()
        self.write("docs/public/guide.md", "Queue member two, prose only.\n")
        self.commit()
        self.assertEqual(self.plan(event="merge_group")["profiles"], ["full"])

    def test_tested_merge_delta_includes_changes_beyond_pr_head(self) -> None:
        self.write("docs/public/guide.md", "PR prose change.\n")
        head = self.commit()
        self.write("src/lib.rs", "fn extra_tested_change() {}\n")
        tested = self.commit()
        plan = make_plan(
            self.root,
            self.policy,
            event="pull_request",
            base_sha=self.base,
            head_sha=head,
            tested_sha=tested,
            policy_sha=self.base,
        )
        self.assertEqual(plan["profiles"], ["full"])

    def test_candidate_cannot_weaken_its_policy(self) -> None:
        raw = json.loads(self.config.read_text())
        raw["profile_jobs"]["prose"] = ["python"]
        raw["profile_jobs"]["executable-docs"] = ["python"]
        self.config.write_text(json.dumps(raw))
        self.commit()
        with self.assertRaisesRegex(ContractError, "trusted base"):
            self.plan(policy=load_policy(self.config))

    def test_trusted_policy_change_itself_requires_full(self) -> None:
        trusted = self.root / "trusted-policy.json"
        trusted.write_bytes(self.config.read_bytes())
        # The trusted policy is loaded from an independent base worktree in production.
        raw = json.loads(self.config.read_text())
        raw["path_profiles"] = {"prose": ["scripts/validation-policy.json"]}
        self.config.write_text(json.dumps(raw))
        self.commit()
        self.command("checkout", self.base, "--", "scripts/validation-policy.json")
        plan = self.plan()
        self.assertEqual(plan["profiles"], ["full"])

    def test_missing_commit_and_symlinks_fail_closed(self) -> None:
        os.symlink("../../src/lib.rs", self.root / "docs/public/link.md")
        self.commit()
        self.assertEqual(self.plan()["profiles"], ["full"])
        plan = make_plan(
            self.root,
            self.policy,
            event="pull_request",
            base_sha=self.base,
            head_sha="0" * 40,
            tested_sha="0" * 40,
            policy_sha=self.base,
        )
        self.assertEqual(plan["profiles"], ["full"])

    def test_local_untracked_source_prevents_docs_only(self) -> None:
        self.write("docs/public/guide.md", "Local prose.\n")
        self.write("unknown.py", "print('untracked')\n")
        plan = make_plan(self.root, self.policy, event="local", base_ref="main")
        self.assertEqual(plan["profiles"], ["full"])

    def test_non_pr_events_remain_complete(self) -> None:
        for event in ("push", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                self.assertEqual(self.plan(event=event)["profiles"], ["full"])


class MarkdownProjection(unittest.TestCase):
    def test_unchanged_supported_regions_allow_surrounding_prose_changes(self) -> None:
        for body in (
            b"```sh\ncargo test --locked\n```\n",
            b"~~~rust,no_run\nfn main() {}\n~~~\n",
            b"    first command\n\n    second command\n",
            b"Run `cargo test` here.\n",
            b"Use ``command `literal` text`` safely.\n",
            b'<pre class="shell">\ncargo test\n</pre>\n',
            b"Use <code>cargo test</code> safely.\n",
        ):
            before = b"Old explanation.\n\n" + body + b"\nOld conclusion.\n"
            after = b"A clearer explanation.\n\n" + body + b"\nA clearer conclusion.\n"
            with self.subTest(body=body):
                self.assertTrue(unchanged_code((before, after)))

    def test_every_code_byte_and_delimiter_is_opaque(self) -> None:
        for before, after in (
            (b"```sh\necho one\n```\n", b"```sh\necho two\n```\n"),
            (b"```rust\nrun\n```\n", b"```rust,no_run\nrun\n```\n"),
            (b"```sh\nrun\n```\n", b"```bash\nrun\n```\n"),
            (b"```sh\nrun\n```\n", b"```sh\nrun\n````\n"),
            (b"    run\n\n    next\n", b"    run\n\n\n    next\n"),
            (b"    run\n", b"        run\n"),
            (b"\trun\n", b"    run\n"),
            (b"Use `cargo test`.\n", b"Use `cargo check`.\n"),
            (b"Use `cargo test`.\n", b"Use ``cargo test``.\n"),
            (b"Use ``command `one` text``.\n", b"Use ``command `two` text``.\n"),
            (b"<pre>run</pre>\n", b"<pre>changed</pre>\n"),
            (b'<code class="one">run</code>\n', b'<code class="two">run</code>\n'),
        ):
            with self.subTest(before=before, after=after):
                self.assertFalse(unchanged_code((before, after)))

    def test_added_or_removed_regions_and_incomplete_evidence_are_full(self) -> None:
        code = b"```sh\nrun\n```\n"
        self.assertFalse(unchanged_code((code,)))
        self.assertFalse(unchanged_code(()))
        self.assertFalse(unchanged_code((b"Prose.\n", code)))
        self.assertFalse(unchanged_code((code, b"Prose.\n")))
        self.assertFalse(unchanged_code((code, code.replace(b"run", b"changed"), code)))

    def test_unsupported_or_ambiguous_shapes_fail_closed(self) -> None:
        for value in (
            b"```sh\nunclosed\n",
            b"Use `an unclosed span.\n",
            b"Use ``a span with a different closing run`.\n",
            b"Use `multiline\nspan`.\n",
            b"  ```sh\nnested or ambiguous\n  ```\n",
            b"> ```sh\n> nested\n> ```\n",
            b"- An item.\n\n    nested indented code\n",
            b"<!--\n```sh\nhidden\n```\n-->\n",
            b"<div>\n```sh\nhidden\n```\n</div>\n",
            b"<pre>unclosed\n",
            b"<pre><code>nested</code></pre>\n",
            b"<code><span>nested</span></code>\n",
            b"<code>wrong closing</pre>\n",
            b"<pre>malformed attributes</pre extra>\n",
            b"Prose before.\n<code>ambiguous raw block</code>\n",
            b"<code>ambiguous raw block</code>\nFollowing prose without a boundary.\n",
            b"Use \\`escaped delimiters\\`.\n",
            b"Use \\<code>escaped HTML</code>.\n",
            b"[destination](`cargo test`)\n",
            b"![`hidden alt command`](image.png)\n",
            b"[multiline destination](\n`cargo test`\n)\n",
            b"[^note]: nested container\n\n    command\n",
            b'---\ntitle: "Use `cargo test`."\n---\n',
            b'+++\ntitle = "Use `cargo test`."\n+++\n',
            b"::: hidden\n```sh\nrun\n```\n:::\n",
            b"!!! note\n    nested command\n",
            b"[reference]: target\n    `hidden title`\n",
        ):
            with self.subTest(value=value):
                self.assertIsNone(code_projection(value))
                self.assertFalse(unchanged_code((value, value)))

    def test_context_changes_cannot_hide_unchanged_code(self) -> None:
        command = b"```sh\nrun\n```\n"
        for changed in (
            b"<!--\n" + command + b"-->\n",
            b"<div>\n" + command + b"</div>\n",
            command.replace(b"```", b"> ```"),
        ):
            self.assertFalse(unchanged_code((command, changed)))
        self.assertFalse(unchanged_code((b"Use `run`.\n", b"Use \\`run`.\n")))

    def test_projection_limits_invalid_text_and_unknown_controls_fail_closed(self) -> None:
        for value in (b"x" * (MAX_MARKDOWN_BYTES + 1), b"\xff", b"bad\0text", b"bad\vtext"):
            self.assertIsNone(code_projection(value))


class DiffParsing(unittest.TestCase):
    def test_add_delete_and_rename_paths(self) -> None:
        self.assertEqual(
            parse_diff(b"A\0new.md\0D\0old.md\0R100\0a.md\0b.md\0"),
            [
                ("A", ("new.md",)),
                ("D", ("old.md",)),
                ("R", ("a.md", "b.md")),
            ],
        )

    def test_malformed_paths_and_records_fail_closed(self) -> None:
        for raw in (b"M\0file", b"R100\0one\0", b"A\0../escape\0", b"X\0x\0", b"A\0\xff\0"):
            with self.subTest(raw=raw), self.assertRaises(ContractError):
                parse_diff(raw)
