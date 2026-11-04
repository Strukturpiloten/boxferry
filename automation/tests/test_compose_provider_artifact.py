"""Candidate source binding and artifact checks independent of production pin versions."""

from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from boxferry_ci import compose_provider as provider

ARTIFACT = b"authored fixture release asset, never executable"
DIGEST = hashlib.sha256(ARTIFACT).hexdigest()
PIN = provider.Pin("v8.7.6", DIGEST)


def sources(pin: provider.Pin) -> dict[str, str]:
    result = {
        provider.PROVIDER: (
            f'  local -r expected_release="{pin.release}"\n'
            f'  local -r expected_sha256="{pin.digest}"\n'
            '  local -r expected_version="${expected_release#v}"\n'
            '  local -r expected_url="https://github.com/docker/compose/releases/download/'
            'v${expected_version}/docker-compose-linux-x86_64"\n'
        )
    }
    for path in provider.CATALOGUES:
        columns = (
            ["docker-compose", pin.version, "linux-amd64", pin.digest, pin.url]
            if "supabase-application" in path
            else ["docker-compose", pin.version, pin.url, pin.digest]
        )
        result[path] = "\t".join([*columns, "Apache-2.0"]) + "\n"
    return result


class CandidateArtifactIntegrity(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.write_sources(PIN)

    def write_sources(self, pin: provider.Pin) -> None:
        for path, content in sources(pin).items():
            destination = self.root / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(content)

    @staticmethod
    def fetch(url: str, *, allowed_hosts: set[str], max_bytes: int) -> str:
        if url != PIN.url or "github.com" not in allowed_hosts or max_bytes < len(ARTIFACT):
            raise AssertionError("incorrect artifact origin or bound")
        return hashlib.sha256(ARTIFACT).hexdigest()

    def test_explicit_local_verification_deduplicates_all_seven_pins(self) -> None:
        with patch.object(provider, "fetch_artifact_sha256", side_effect=self.fetch) as fetch:
            pin = provider.verify(self.root, verify_working_tree=True, artifact_fetcher=fetch)
        self.assertEqual(pin, PIN)
        fetch.assert_called_once_with(
            PIN.url, allowed_hosts=set(provider.HOSTS), max_bytes=provider.MAX_ARTIFACT
        )
        self.assertEqual(
            sorted(
                p.relative_to(self.root).as_posix() for p in self.root.rglob("*") if p.is_file()
            ),
            sorted(sources(PIN)),
        )

    def test_consistent_new_version_url_with_stale_digest_fails(self) -> None:
        # Every text-only consistency assertion passes; the actual bytes still reject it.
        stale = provider.Pin(PIN.release, hashlib.sha256(b"previous release bytes").hexdigest())
        self.write_sources(stale)
        for path in provider.CATALOGUES:
            provider.validate_catalogue(sources(stale)[path], path, stale)
        with self.assertRaisesRegex(provider.ProviderError, "downloaded artifact SHA-256 differs"):
            provider.verify(self.root, verify_working_tree=True, artifact_fetcher=self.fetch)

    def test_real_stream_hash_rejects_stale_digest_without_executing_source(self) -> None:
        stale = provider.Pin(PIN.release, hashlib.sha256(b"previous artifact").hexdigest())
        self.write_sources(stale)
        marker = self.root / "must-not-exist"
        canonical = self.root / provider.PROVIDER
        canonical.write_text(canonical.read_text() + f"touch '{marker}'\n")
        with patch(
            "boxferry_ci.kubernetes_tool_checksum_refresh.urllib.request.build_opener"
        ) as build:
            response = build.return_value.open.return_value.__enter__.return_value
            response.headers = {"Content-Length": str(len(ARTIFACT))}
            response.geturl.return_value = PIN.url
            response.read.side_effect = [ARTIFACT, b""]
            with self.assertRaisesRegex(
                provider.ProviderError, "downloaded artifact SHA-256 differs"
            ):
                provider.verify(self.root, verify_working_tree=True)
            self.assertEqual(response.read.call_count, 2)
            self.assertEqual(build.return_value.open.call_count, 1)
        self.assertFalse(marker.exists())
        self.assertEqual(
            sorted(
                p.relative_to(self.root).as_posix() for p in self.root.rglob("*") if p.is_file()
            ),
            sorted(sources(stale)),
        )

    def test_partial_catalogue_update_fails_before_any_download(self) -> None:
        (self.root / provider.CATALOGUES[0]).write_text(
            sources(provider.Pin("v1.0.0", DIGEST))[provider.CATALOGUES[0]]
        )
        with patch.object(provider, "fetch_artifact_sha256") as fetch:
            with self.assertRaisesRegex(provider.ProviderError, "differ from the canonical pin"):
                provider.verify(self.root, verify_working_tree=True, artifact_fetcher=fetch)
            fetch.assert_not_called()

    def test_requires_explicit_source_mode_and_exact_commit(self) -> None:
        for candidate_sha, working_tree in ((None, False), ("main", False), ("a" * 40, True)):
            with (
                self.subTest(candidate_sha=candidate_sha, working_tree=working_tree),
                self.assertRaises(provider.ProviderError),
            ):
                provider.verify(
                    self.root, candidate_sha=candidate_sha, verify_working_tree=working_tree
                )

    def test_immutable_source_ignores_mutated_worktree(self) -> None:
        self.write_sources(provider.Pin("v99.0.0", "a" * 64))
        sha = "b" * 40
        expected = sources(PIN)

        def git(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[bytes]:
            self.assertEqual(command[:2], ["git", "--no-replace-objects"])
            self.assertEqual(kwargs["cwd"], self.root)
            if command[2:4] == ["cat-file", "-t"]:
                self.assertEqual(command[4], sha)
                output = b"commit\n"
            elif command[2] == "ls-tree":
                self.assertEqual(command[4], sha)
                path = command[-1]
                output = b"100644 blob " + b"c" * 40 + b"\t" + path.encode() + b"\0"
            else:
                self.assertTrue(command[-1].startswith(sha + ":"))
                content = expected[command[-1].split(":", 1)[1]].encode()
                output = str(len(content)).encode() if command[2] == "cat-file" else content
            return subprocess.CompletedProcess(command, 0, output, b"")

        with patch.object(provider.subprocess, "run", side_effect=git):
            self.assertEqual(
                provider.verify(self.root, candidate_sha=sha, artifact_fetcher=self.fetch),
                PIN,
            )

    def test_alternate_or_prior_metadata_declarations_rejected(self) -> None:
        canonical = sources(PIN)[provider.PROVIDER]
        for name in ("release", "sha256", "version", "url"):
            for declaration in (
                f"  local -r expected_{name}='prior-value'\n",
                f'    local  -r  expected_{name}="prior-value"\n',
                f"  readonly expected_{name}\n",
                f"  local -r expected_{name}\n",
                f"  local -r expected_{name}+='prior-value'\n",
                f"  local -r expected_{name}[0]='prior-value'\n",
                f"expected_{name}=prior-value\n",
                f"expected_{name}+=prior-value\n",
            ):
                with (
                    self.subTest(declaration=declaration),
                    self.assertRaises(provider.ProviderError),
                ):
                    provider.parse_provider(declaration + canonical)

    def test_immutable_source_rejects_nonregular_git_modes(self) -> None:
        sha = "b" * 40
        for mode, kind in (("120000", "blob"), ("160000", "commit"), ("040000", "tree"), ("", "")):
            entry = f"{mode} {kind} {'c' * 40}\t{provider.PROVIDER}\0".encode() if mode else b""
            responses = [
                subprocess.CompletedProcess([], 0, b"commit\n"),
                subprocess.CompletedProcess([], 0, entry),
            ]
            with (
                self.subTest(mode=mode),
                patch.object(provider.subprocess, "run", side_effect=responses),
            ):
                with self.assertRaisesRegex(provider.ProviderError, "regular Git blob"):
                    provider.verify(self.root, candidate_sha=sha, artifact_fetcher=self.fetch)

    def test_noncommit_object_rejected(self) -> None:
        with (
            patch.object(
                provider.subprocess,
                "run",
                return_value=subprocess.CompletedProcess([], 0, b"tree\n"),
            ),
            self.assertRaisesRegex(provider.ProviderError, "commit object"),
        ):
            provider.verify(self.root, candidate_sha="b" * 40, artifact_fetcher=self.fetch)

    def test_local_nonregular_source_fails_without_blocking(self) -> None:
        canonical = self.root / provider.PROVIDER
        canonical.unlink()
        os.mkfifo(canonical)
        with self.assertRaisesRegex(provider.ProviderError, "regular file"):
            provider.verify(self.root, verify_working_tree=True, artifact_fetcher=self.fetch)

    def test_duplicate_pin_changed_asset_and_linked_source_fail(self) -> None:
        canonical = sources(PIN)[provider.PROVIDER]
        for content in (
            canonical + canonical,
            canonical.replace("docker-compose-linux-x86_64", "other"),
        ):
            with (
                self.subTest(content=content),
                self.assertRaises(provider.ProviderError),
            ):
                provider.parse_provider(content)
        path = self.root / provider.PROVIDER
        path.unlink()
        path.symlink_to(self.root / provider.CATALOGUES[0])
        with self.assertRaisesRegex(provider.ProviderError, "symbolic links"):
            provider.verify(self.root, verify_working_tree=True, artifact_fetcher=self.fetch)


if __name__ == "__main__":
    unittest.main()
