"""Exercise the Git identity boundary in the native preset test inputs."""

from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from native_preset_source import PRESETS, PresetSourceError, pinned_preset_input

from boxferry_ci import bootstrap
from boxferry_ci.bootstrap import REPOSITORY, BootstrapError, git_environment


class NativePresetSource(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / ".github").mkdir()
        (self.root / "renovate-presets").mkdir()
        self.git("init", "--quiet", "--template=")
        for name in PRESETS:
            self.preset(name).write_text(json.dumps({"description": f"pinned {name}"}))
        self.pinned = self.commit()
        self.configure(self.pinned)

    def git(self, *arguments: str, root: Path | None = None) -> str:
        return subprocess.run(
            [
                "git",
                "-C",
                str(self.root if root is None else root),
                "-c",
                "user.name=Native preset test",
                "-c",
                "user.email=native-preset@example.invalid",
                *arguments,
            ],
            capture_output=True,
            check=True,
            env=git_environment(),
            text=True,
            timeout=10,
        ).stdout.strip()

    def preset(self, name: str) -> Path:
        return self.root / "renovate-presets" / f"{name}.json"

    def commit(self) -> str:
        self.git("add", "renovate-presets")
        self.git("commit", "--quiet", "-m", "fixture")
        return self.git("rev-parse", "HEAD")

    def configure(self, revision: str) -> None:
        (self.root / ".github/automation.json").write_text(
            json.dumps(
                {
                    "schema": 1,
                    "repository": REPOSITORY,
                    "revision": revision,
                    "policy": "scripts/shared-validation-policy.json",
                    "interface": 1,
                }
            )
        )
        extends = [f"github>{REPOSITORY}//renovate-presets/{name}#{revision}" for name in PRESETS]
        (self.root / ".github/renovate.json").write_text(json.dumps({"extends": extends}))

    def test_reads_pinned_bytes_even_when_candidate_files_and_head_differ(self) -> None:
        self.preset("common").write_text('{"description": "candidate common"}')
        self.assertNotEqual(self.commit(), self.pinned)
        self.preset("rust").write_text('{"description": "working-tree rust"}')
        with patch("native_preset_source.prepare_object_cache") as fetch:
            data = json.loads(pinned_preset_input(self.root))
        fetch.assert_not_called()
        self.assertEqual(
            data,
            {
                f"github>{REPOSITORY}//renovate-presets/{name}#{self.pinned}": json.dumps(
                    {"description": f"pinned {name}"}
                )
                for name in PRESETS
            },
        )

    def test_references_must_match_manifest_and_cover_both_presets(self) -> None:
        config_path = self.root / ".github/renovate.json"
        original = json.loads(config_path.read_text())["extends"]
        for references in (
            [original[0], original[0]],
            original[:1],
            [original[0].replace(REPOSITORY, "elsewhere/boxferry"), original[1]],
            [original[0].split("#")[0], original[1]],
            [original[0].replace(self.pinned, "a" * 40), original[1]],
        ):
            with self.subTest(references=references):
                config_path.write_text(json.dumps({"extends": references}))
                with self.assertRaisesRegex(
                    PresetSourceError, "expected exactly the common and rust"
                ):
                    pinned_preset_input(self.root)

    def test_rejects_missing_or_nonregular_preset_at_the_commit(self) -> None:
        self.preset("common").unlink()
        self.configure(self.commit())
        with self.assertRaisesRegex(
            PresetSourceError, "common.json: missing regular tracked preset"
        ):
            pinned_preset_input(self.root)
        self.preset("common").symlink_to("rust.json")
        self.configure(self.commit())
        with self.assertRaisesRegex(
            PresetSourceError, "common.json: missing regular tracked preset"
        ):
            pinned_preset_input(self.root)

    def test_rejects_malformed_preset_or_non_commit_revision(self) -> None:
        for raw, diagnostic in (
            ("{bad json", "not valid UTF-8 JSON"),
            ('{"value": NaN}', "not valid UTF-8 JSON"),
            ('{"value": Infinity}', "not valid UTF-8 JSON"),
            ('{"value": -Infinity}', "not valid UTF-8 JSON"),
            ("[]", "must be a JSON object"),
        ):
            with self.subTest(raw=raw):
                self.preset("common").write_text(raw)
                self.configure(self.commit())
                with self.assertRaisesRegex(PresetSourceError, diagnostic):
                    pinned_preset_input(self.root)
        self.configure(self.git("rev-parse", "HEAD:renovate-presets/common.json"))
        with self.assertRaisesRegex(PresetSourceError, "expected a Git commit"):
            pinned_preset_input(self.root)

    def test_shallow_object_cache_reads_raw_blobs_even_with_archive_substitution(self) -> None:
        raw = '{"description": "$Format:%H$"}'
        self.preset("common").write_text(raw)
        (self.root / ".gitattributes").write_text("renovate-presets/common.json export-subst\n")
        self.git("add", ".gitattributes")
        revision = self.commit()
        self.configure(revision)
        expected = pinned_preset_input(self.root)
        archived = subprocess.check_output(
            ["git", "-C", str(self.root), "archive", revision, "renovate-presets/common.json"],
            env=git_environment(),
            timeout=10,
        )
        self.assertNotIn(raw.encode(), archived)
        self.assertIn(revision.encode(), archived)
        self.preset("common").write_text('{"description": "candidate"}')
        self.commit()
        with tempfile.TemporaryDirectory() as directory:
            shallow = Path(directory) / "shallow"
            self.git("clone", "--quiet", "--depth=1", self.root.as_uri(), str(shallow))
            self.assertEqual(self.git("rev-parse", "--is-shallow-repository", root=shallow), "true")
            with self.assertRaises(subprocess.CalledProcessError):
                self.git("cat-file", "-t", revision, root=shallow)
            shutil.copytree(self.root / ".github", shallow / ".github")

            def fetch_fixture(destination: Path, requested: str) -> Path:
                self.assertEqual(requested, revision)
                objects = destination / f"target/boxferry-automation/{requested}.git"
                objects.mkdir(parents=True)
                self.git("init", "--bare", "--quiet", "--template=", root=objects)
                self.git(
                    "fetch",
                    "--quiet",
                    "--depth=1",
                    "--no-tags",
                    self.root.as_uri(),
                    requested,
                    root=objects,
                )
                (objects / "config").write_text(bootstrap.BARE_CONFIG)
                return bootstrap.prepare_object_cache(destination, requested)

            with patch("native_preset_source.prepare_object_cache", side_effect=fetch_fixture):
                self.assertEqual(pinned_preset_input(shallow), expected)

    def test_missing_commit_uses_exact_fetch_and_propagates_unavailable_source(self) -> None:
        revision = "a" * 40
        self.configure(revision)
        with (
            patch(
                "native_preset_source.prepare_object_cache",
                side_effect=BootstrapError("pinned commit unavailable"),
            ) as fetch,
            self.assertRaisesRegex(
                PresetSourceError, "cannot acquire pinned presets.*pinned commit unavailable"
            ),
        ):
            pinned_preset_input(self.root)
        self.assertEqual(fetch.call_args.args[1], revision)
        self.assertFalse(fetch.call_args.args[0].exists())
