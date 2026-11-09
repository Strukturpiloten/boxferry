"""Independent examples for bounded dependency evidence and whole-update scope."""

from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import unittest
import urllib.error
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from email.message import Message
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from boxferry_ci.dependencies import (
    CRATES_SOURCE,
    MAX_METADATA_BYTES,
    DependencyContext,
    DependencyError,
    check_dependencies,
)

NOW = datetime(2026, 11, 3, 12, tzinfo=UTC)
OLD = "2026-10-01T00:00:00Z"
YOUNG = "2026-11-03T11:00:00Z"
HASH = "a" * 64
OTHER_HASH = "b" * 64
SRI = "sha512-" + base64.b64encode(bytes(64)).decode()


def cargo_record(
    name: str, version: str, digest: str = HASH, dependencies: tuple[str, ...] = ()
) -> str:
    text = (
        f'[[package]]\nname = "{name}"\nversion = "{version}"\n'
        f'source = "{CRATES_SOURCE}"\nchecksum = "{digest}"\n'
    )
    if dependencies:
        text += "dependencies = " + json.dumps(dependencies) + "\n"
    return text


def cargo_metadata(
    name: str, version: str, timestamp: str | None = OLD, digest: str = HASH
) -> bytes:
    return json.dumps(
        {
            "version": {
                "crate": name,
                "num": version,
                "checksum": digest,
                "created_at": timestamp,
            }
        }
    ).encode()


class DependencyTests(unittest.TestCase):
    def test_nested_dependency_action_contract_and_native_pin_ownership(self) -> None:
        root = Path(__file__).resolve().parents[2]
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", root / "target/tools/renovate"))
        expected = json.loads((root / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual((runtime / "source-image.txt").read_text().strip(), expected)
        result = subprocess.run(
            [
                str(runtime / "node"),
                str(root / "scripts/test-cargo-deny-action.cjs"),
                str(runtime),
                str(root),
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.base, self.head = self.root / "base", self.root / "head"
        self.base.mkdir()
        self.head.mkdir()

    def check(
        self,
        fetch: Callable[[str], bytes],
        *,
        state: dict[str, Any] | None = None,
        context: DependencyContext | None = None,
    ) -> dict[str, Any]:
        return check_dependencies(
            self.base, self.head, now=NOW, fetch=fetch, state=state, context=context
        )

    def cargo(self, before: str = "", after: str | None = None) -> None:
        (self.base / "Cargo.lock").write_text("version = 4\n" + before)
        (self.head / "Cargo.lock").write_text(
            "version = 4\n" + (after if after is not None else cargo_record("serde", "1.0.228"))
        )

    def context(
        self,
        paths: tuple[str, ...] = ("Cargo.toml", "Cargo.lock"),
        actor: str = "renovate[bot]",
    ) -> DependencyContext:
        return DependencyContext(
            actor, "Bot", "Strukturpiloten/boxferry", "a" * 40, "b" * 40, paths
        )

    def first_party(self, extra_before: str = "", extra_after: str = "") -> None:
        before = cargo_record(
            "compose-lens", "1.0.0", dependencies=("smallvec 1.0.0",)
        ) + cargo_record("smallvec", "1.0.0")
        after = cargo_record(
            "compose-lens", "1.1.0", dependencies=("smallvec 1.1.0",)
        ) + cargo_record("smallvec", "1.1.0")
        self.cargo(before + extra_before, after + extra_after)
        (self.base / "Cargo.toml").write_text('[workspace.dependencies]\ncompose-lens = "1.0.0"\n')
        (self.head / "Cargo.toml").write_text('[workspace.dependencies]\ncompose-lens = "1.1.0"\n')

    @staticmethod
    def any_cargo(url: str) -> bytes:
        name, version = url.rsplit("/", 2)[1:]
        return cargo_metadata(name, version, YOUNG)

    def standalone(self, root: Path, version: str) -> None:
        directory = root / ".devcontainer"
        directory.mkdir(exist_ok=True)
        (directory / "Dockerfile").write_text(
            "# renovate: datasource=crate depName=cargo-semver-checks\n"
            f"ARG CARGO_SEMVER_CHECKS_VERSION={version}\n"
        )

    def test_standalone_tool_uses_its_own_publication_age_and_integrity(self) -> None:
        self.standalone(self.base, "0.50.0")
        self.standalone(self.head, "0.51.0")
        seen = []

        def fetch(url: str) -> bytes:
            seen.append(url)
            return cargo_metadata("cargo-semver-checks", "0.51.0", YOUNG)

        report = self.check(fetch)
        self.assertEqual(seen, ["https://crates.io/api/v1/crates/cargo-semver-checks/0.51.0"])
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["results"][0]["integrity"], [HASH])
        self.assertFalse(report["first_party_exempt"])
        boundary = NOW + timedelta(hours=71)  # YOUNG was published one hour before NOW.
        waiting = check_dependencies(
            self.base, self.head, now=boundary - timedelta(seconds=1), fetch=fetch
        )
        self.assertEqual(waiting["status"], "waiting")
        mature = check_dependencies(self.base, self.head, now=boundary, fetch=fetch)
        self.assertEqual(mature["status"], "pass")

    def test_unchanged_standalone_tool_does_not_reset_age_or_fetch(self) -> None:
        self.standalone(self.base, "0.50.0")
        self.standalone(self.head, "0.50.0")
        report = self.check(lambda _: self.fail("unchanged standalone version fetched"))
        self.assertEqual(report["status"], "pass")
        self.assertEqual(report["results"], [])

    def test_standalone_tool_cannot_inherit_first_party_update_exemption(self) -> None:
        self.first_party()
        self.standalone(self.base, "0.50.0")
        self.standalone(self.head, "0.51.0")
        report = self.check(self.any_cargo, context=self.context())
        self.assertFalse(report["first_party_exempt"])
        self.assertEqual(report["status"], "waiting")
        # Existing whole-first-party/transitive exemption remains when tools do not change.
        self.standalone(self.head, "0.50.0")
        report = self.check(self.any_cargo, context=self.context())
        self.assertTrue(report["first_party_exempt"])
        self.assertEqual(report["status"], "pass")

    def test_standalone_tool_requires_exact_metadata_without_quarantine_fallback(self) -> None:
        self.standalone(self.head, "0.51.0")
        cases = [
            cargo_metadata("other", "0.51.0"),
            cargo_metadata("cargo-semver-checks", "0.50.0"),
            cargo_metadata("cargo-semver-checks", "0.51.0", None),
            cargo_metadata("cargo-semver-checks", "0.51.0", "future"),
            cargo_metadata("cargo-semver-checks", "0.51.0", "2026-11-03T12:00:01Z"),
            cargo_metadata("cargo-semver-checks", "0.51.0", digest="not-checksum"),
        ]
        for metadata in cases:
            with self.subTest(metadata=metadata):
                report = self.check(lambda _, data=metadata: data)
                self.assertEqual(report["status"], "failure")
                self.assertEqual(report["state"]["first_seen"], {})

    def test_malformed_owned_standalone_declaration_fails_closed(self) -> None:
        self.standalone(self.head, "0.51.0")
        path = self.head / ".devcontainer/Dockerfile"
        original = path.read_text()
        for text in (
            original + original,
            original.replace("0.51.0", "latest"),
            original.replace("depName=cargo-semver-checks", "depName=unrelated"),
        ):
            path.write_text(text)
            report = self.check(lambda _: self.fail("malformed declaration reached registry"))
            self.assertEqual(report["status"], "failure")

    def test_mature_version_passes_and_uses_exact_endpoint(self) -> None:
        self.cargo()
        seen = []

        def fetch(url: str) -> bytes:
            seen.append(url)
            return cargo_metadata("serde", "1.0.228")

        report = self.check(fetch)
        self.assertEqual(report["status"], "pass")
        self.assertEqual(seen, ["https://crates.io/api/v1/crates/serde/1.0.228"])
        self.assertEqual(report["results"][0]["age_evidence"], "registry-publication")

    def test_young_waits_until_exact_72_hour_boundary(self) -> None:
        self.cargo()

        def fetch(_: str) -> bytes:
            return cargo_metadata("serde", "1.0.228", YOUNG)

        report = self.check(fetch)
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["retry_at"], "2026-11-06T11:00:00Z")
        self.assertTrue(report["retry_eligible"])
        later = check_dependencies(self.base, self.head, now=NOW + timedelta(days=3), fetch=fetch)
        self.assertEqual(later["status"], "pass")

    def test_unchanged_lock_version_does_not_trigger_network(self) -> None:
        self.cargo(cargo_record("serde", "1.0.228"), cargo_record("serde", "1.0.228"))
        report = self.check(lambda _: self.fail("unchanged version fetched"))
        self.assertEqual(report["results"], [])

    def test_missing_timestamp_is_observation_quarantine_and_survives_recheck(
        self,
    ) -> None:
        self.cargo()

        def fetch(_: str) -> bytes:
            return cargo_metadata("serde", "1.0.228", None)

        report = self.check(fetch)
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["results"][0]["age_evidence"], "artifact-first-seen")
        self.assertEqual(report["retry_at"], "2026-11-06T12:00:00Z")
        later = check_dependencies(
            self.base,
            self.head,
            now=NOW + timedelta(hours=72),
            fetch=fetch,
            state=report["state"],
        )
        self.assertEqual(later["status"], "pass")

    def test_simple_rebase_preserves_immutable_artifact_first_seen(self) -> None:
        self.cargo()

        def fetch(_: str) -> bytes:
            return cargo_metadata("serde", "1.0.228", None)

        initial = self.check(fetch)
        unrelated = cargo_record("unrelated-base-update", "1.0.0")
        for root in (self.base, self.head):
            with (root / "Cargo.lock").open("a") as lock:
                lock.write(unrelated)
        rebased = check_dependencies(
            self.base, self.head, now=NOW + timedelta(hours=1), state=initial["state"], fetch=fetch
        )
        self.assertEqual(rebased["status"], "waiting")
        self.assertEqual(rebased["retry_at"], initial["retry_at"])
        self.assertEqual(rebased["state"], initial["state"])

    def test_first_seen_cannot_transfer_to_different_artifact(self) -> None:
        self.cargo()
        first = self.check(lambda _: cargo_metadata("serde", "1.0.228", None))
        (self.head / "Cargo.lock").write_text(cargo_record("serde", "1.0.228", OTHER_HASH))
        later = check_dependencies(
            self.base,
            self.head,
            now=NOW + timedelta(days=4),
            state=first["state"],
            fetch=lambda _: cargo_metadata("serde", "1.0.228", None, OTHER_HASH),
        )
        self.assertEqual(later["status"], "waiting")
        self.assertEqual(len(later["state"]["first_seen"]), 2)

    def test_malformed_and_future_timestamps_fail_without_fallback(self) -> None:
        self.cargo()
        for timestamp in (
            "yesterday",
            "2026-11-03",
            "2026-11-03T12:00:01Z",
            "2026-02-30T00:00:00Z",
        ):
            with self.subTest(timestamp=timestamp):
                report = self.check(
                    lambda _, value=timestamp: cargo_metadata("serde", "1.0.228", value)
                )
                self.assertEqual(report["status"], "failure")
                self.assertEqual(report["state"]["first_seen"], {})
                self.assertFalse(report["retry_eligible"])

    def test_metadata_identity_and_integrity_must_match(self) -> None:
        self.cargo()
        for payload in (
            cargo_metadata("impostor", "1.0.228"),
            cargo_metadata("serde", "1.0.228", digest=OTHER_HASH),
        ):
            with self.subTest(payload=payload):
                self.assertEqual(self.check(lambda _, value=payload: value)["status"], "failure")

    def test_version_metadata_oversize_is_structured_failure(self) -> None:
        self.cargo()
        report = self.check(lambda _: b" " * (MAX_METADATA_BYTES + 1))
        self.assertEqual(report["status"], "failure")
        self.assertIn("byte bound", report["results"][0]["reason"])

    def test_transport_failure_has_bounded_automatic_retry(self) -> None:
        self.cargo()

        def fetch(_: str) -> bytes:
            raise urllib.error.URLError("temporary registry failure")

        report = self.check(fetch)
        self.assertEqual(report["status"], "failure")
        self.assertTrue(report["retry_eligible"])
        self.assertEqual(report["retry_at"], "2026-11-03T13:00:00Z")

    def test_bad_state_is_failure_not_silent_quarantine_reset(self) -> None:
        self.cargo()
        for state in (
            {},
            {"schema_version": 1, "first_seen": {"a" * 64: "2027-01-01T00:00:00Z"}},
            {"schema_version": 2, "first_seen": {}},
        ):
            with self.subTest(state=state):
                report = self.check(lambda _: self.fail("bad state fetched"), state=state)
                self.assertEqual(report["status"], "failure")

    def test_first_party_whole_update_exempts_young_transitives(self) -> None:
        self.first_party()
        report = self.check(self.any_cargo, context=self.context())
        self.assertEqual(report["status"], "pass")
        self.assertTrue(report["first_party_exempt"])
        self.assertEqual({r["name"] for r in report["results"]}, {"compose-lens", "smallvec"})
        self.assertTrue(
            all(r["age_evidence"] == "first-party-update-exemption" for r in report["results"])
        )

    def test_first_party_still_requires_authoritative_integrity(self) -> None:
        self.first_party()
        report = self.check(
            lambda url: cargo_metadata(*url.rsplit("/", 2)[1:], digest=OTHER_HASH),
            context=self.context(),
        )
        self.assertEqual(report["status"], "failure")

    def test_actor_name_or_package_prefix_cannot_grant_exemption(self) -> None:
        self.first_party()
        self.assertFalse(
            self.check(self.any_cargo, context=self.context(actor="renovate"))["first_party_exempt"]
        )
        self.assertFalse(self.check(self.any_cargo)["first_party_exempt"])
        (self.head / "Cargo.toml").write_text(
            '[workspace.dependencies]\ncompose-lens-malicious = "1.1.0"\n'
        )
        self.assertFalse(self.check(self.any_cargo, context=self.context())["first_party_exempt"])

    def test_unrelated_lock_maintenance_cannot_inherit_exemption(self) -> None:
        self.first_party(cargo_record("unrelated", "1.0.0"), cargo_record("unrelated", "2.0.0"))
        report = self.check(self.any_cargo, context=self.context())
        self.assertFalse(report["first_party_exempt"])
        self.assertEqual(report["status"], "waiting")
        self.assertIn("outside", report["first_party_reason"])

    def test_extra_paths_or_manifest_metadata_cannot_inherit_exemption(self) -> None:
        self.first_party()
        report = self.check(
            self.any_cargo,
            context=self.context(("Cargo.toml", "Cargo.lock", ".github/renovate.json")),
        )
        self.assertFalse(report["first_party_exempt"])
        with (self.head / "Cargo.toml").open("a") as file:
            file.write('\n[workspace.package]\nrust-version = "1.98.1"\n')
        self.assertFalse(self.check(self.any_cargo, context=self.context())["first_party_exempt"])

    def test_lock_maintenance_without_first_party_manifest_transition_is_ordinary(
        self,
    ) -> None:
        self.first_party()
        (self.head / "Cargo.toml").write_bytes((self.base / "Cargo.toml").read_bytes())
        report = self.check(self.any_cargo, context=self.context(("Cargo.lock",)))
        self.assertFalse(report["first_party_exempt"])
        self.assertEqual(report["status"], "waiting")

    def test_npm_exact_version_uses_integrity_observation_when_time_absent(
        self,
    ) -> None:
        (self.base / "package-lock.json").write_text(
            json.dumps({"lockfileVersion": 3, "packages": {}})
        )
        (self.head / "package-lock.json").write_text(
            json.dumps(
                {
                    "lockfileVersion": 3,
                    "packages": {
                        "node_modules/@scope/pkg": {
                            "version": "1.2.3",
                            "resolved": "https://registry.npmjs.org/@scope/pkg/-/pkg-1.2.3.tgz",
                            "integrity": SRI,
                        }
                    },
                }
            )
        )
        urls = []

        def fetch(url: str) -> bytes:
            urls.append(url)
            return json.dumps(
                {"name": "@scope/pkg", "version": "1.2.3", "dist": {"integrity": SRI}}
            ).encode()

        report = self.check(fetch)
        self.assertEqual(urls, ["https://registry.npmjs.org/%40scope%2Fpkg/1.2.3"])
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["results"][0]["age_evidence"], "artifact-first-seen")

    def test_isolated_devcontainer_cli_integrity_keeps_full_72_hour_hold(self) -> None:
        lock = self.head / "automation/devcontainer/package-lock.json"
        lock.parent.mkdir(parents=True)
        lock.write_text(
            json.dumps(
                {
                    "lockfileVersion": 3,
                    "packages": {
                        "node_modules/@devcontainers/cli": {
                            "version": "0.89.0",
                            "resolved": "https://registry.npmjs.org/@devcontainers/cli/-/cli-0.89.0.tgz",
                            "integrity": SRI,
                        }
                    },
                }
            )
        )
        urls = []

        def fetch(url: str) -> bytes:
            urls.append(url)
            return json.dumps(
                {"name": "@devcontainers/cli", "version": "0.89.0", "dist": {"integrity": SRI}}
            ).encode()

        context = DependencyContext(
            "strukturpiloten-release-plz[bot]",
            "Bot",
            "Strukturpiloten/boxferry",
            "a" * 40,
            "b" * 40,
            ("automation/devcontainer/package-lock.json",),
        )
        initial = self.check(fetch, context=context)
        self.assertEqual(initial["status"], "waiting")
        self.assertFalse(initial["first_party_exempt"])
        self.assertEqual(urls, ["https://registry.npmjs.org/%40devcontainers%2Fcli/0.89.0"])
        before = check_dependencies(
            self.base,
            self.head,
            fetch=fetch,
            context=context,
            state=initial["state"],
            now=NOW + timedelta(hours=72) - timedelta(seconds=1),
        )
        boundary = check_dependencies(
            self.base,
            self.head,
            fetch=fetch,
            context=context,
            state=initial["state"],
            now=NOW + timedelta(hours=72),
        )
        self.assertEqual(before["status"], "waiting")
        self.assertEqual(boundary["status"], "pass")

    def test_python_checks_all_locked_artifact_uploads(self) -> None:
        (self.head / "automation").mkdir()
        (self.head / "automation/uv.lock").write_text(f"""version = 1
[[package]]
name = "ruff"
version = "0.17.3"
source = {{ registry = "https://pypi.org/simple" }}
sdist = {{ url = "https://files.pythonhosted.org/ruff.tar.gz", hash = "sha256:{HASH}" }}
wheels = [
  {{ url = "https://files.pythonhosted.org/ruff.whl", hash = "sha256:{OTHER_HASH}" }}
]
""")
        metadata = {
            "info": {"name": "ruff", "version": "0.17.3"},
            "urls": [
                {"digests": {"sha256": HASH}, "upload_time_iso_8601": OLD},
                {"digests": {"sha256": OTHER_HASH}, "upload_time_iso_8601": YOUNG},
            ],
        }
        report = self.check(lambda _: json.dumps(metadata).encode())
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["retry_at"], "2026-11-06T11:00:00Z")
        metadata["urls"].pop()
        self.assertEqual(self.check(lambda _: json.dumps(metadata).encode())["status"], "failure")

    def test_unknown_registry_and_bad_lockfile_fail(self) -> None:
        self.cargo(
            after=cargo_record("serde", "1.0.228").replace(
                CRATES_SOURCE, "registry+https://example.test"
            )
        )
        self.assertEqual(
            self.check(lambda _: self.fail("unsupported source fetched"))["status"],
            "failure",
        )
        (self.head / "Cargo.lock").write_text("not = [toml")
        self.assertEqual(self.check(lambda _: self.fail("bad TOML fetched"))["status"], "failure")

    def test_malformed_metadata_shapes_are_structured_failure(self) -> None:
        self.cargo()
        for data in (b"null", b"[]", b'{"version": []}', b'{"version": null}'):
            with self.subTest(data=data):
                self.assertEqual(self.check(lambda _, data=data: data)["status"], "failure")

    def test_http_retry_only_for_rate_limit_and_server_errors(self) -> None:
        self.cargo()
        for status, retry in ((404, False), (429, True), (503, True)):

            def fetch(_: str, status: int = status) -> bytes:
                raise urllib.error.HTTPError(
                    "https://crates.io", status, "failure", Message(), None
                )

            report = self.check(fetch)
            self.assertEqual(report["status"], "failure")
            self.assertEqual(report["retry_eligible"], retry)

    def test_context_schema_and_naive_clock_rejected(self) -> None:
        with self.assertRaises(DependencyError):
            DependencyContext.from_mapping({"actor_login": "renovate[bot]"})
        with self.assertRaises(DependencyError):
            check_dependencies(self.base, self.head, now=datetime(2026, 11, 3))


class GitSourceAdmission(unittest.TestCase):
    """Synthetic canonical inventory independent of operational binary pins."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base, self.head = Path(temporary.name) / "base", Path(temporary.name) / "head"
        self.base.mkdir()
        self.head.mkdir()
        self.row = {
            "tool": "git",
            "provider": "kernel.org",
            "currentVersion": "2.50.9",
            "filename": "git-2.50.9.tar.xz",
            "url": "https://www.kernel.org/pub/software/scm/git/git-2.50.9.tar.xz",
            "sha256": "9" * 64,
        }
        self.calls: list[str] = []
        self.write(self.head)

    def write(self, root: Path, *, source: bool = True) -> None:
        path = root / "automation/tool-artifacts.json"
        path.parent.mkdir(exist_ok=True)
        path.write_text(
            json.dumps(
                {
                    "schema": 2,
                    "provenance": {},
                    "assets": [],
                    "sources": [self.row] if source else [],
                }
            )
        )

    def metadata(self, url: str) -> bytes:
        self.calls.append(url)
        if url.startswith("https://crates.io/api/v1/crates/"):
            return DependencyTests.any_cargo(url)
        self.assertEqual(url, "https://www.kernel.org/pub/software/scm/git/sha256sums.asc")
        return (
            "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n"
            + self.row["sha256"]
            + "  "
            + self.row["filename"]
            + "\n-----BEGIN PGP SIGNATURE-----\n\nZmFrZQ==\n=abcd\n"
            "-----END PGP SIGNATURE-----\n"
        ).encode()

    def check(
        self,
        now: datetime = NOW,
        state: dict[str, Any] | None = None,
        context: DependencyContext | None = None,
    ) -> dict[str, Any]:
        return check_dependencies(
            self.base, self.head, now=now, state=state, context=context, fetch=self.metadata
        )

    def test_initial_observation_one_instant_before_and_exact_72_hours(self) -> None:
        initial = self.check()
        self.assertEqual(initial["status"], "waiting")
        result = initial["results"][0]
        self.assertEqual(
            (result["ecosystem"], result["age_evidence"], result["integrity"]),
            ("git-source", "artifact-first-seen", ["9" * 64]),
        )
        self.assertEqual(result["evidence_at"], "2026-11-03T12:00:00Z")
        self.assertEqual(result["retry_at"], "2026-11-06T12:00:00Z")
        before = self.check(NOW + timedelta(hours=72, microseconds=-1), initial["state"])
        self.assertEqual(before["status"], "waiting")
        exact = self.check(NOW + timedelta(hours=72), initial["state"])
        self.assertEqual(exact["status"], "pass")
        self.assertEqual(exact["state"], initial["state"])

    def test_retry_and_rebase_preserve_observation_new_digest_or_version_resets(self) -> None:
        initial = self.check()
        self.write(self.base, source=False)
        for head_revision in ("b" * 40, "d" * 40):
            context = DependencyContext(
                "renovate[bot]",
                "Bot",
                "Strukturpiloten/boxferry",
                "c" * 40,
                head_revision,
                ("automation/tool-artifacts.json",),
            )
            retry = self.check(NOW + timedelta(hours=48), initial["state"], context)
            self.assertEqual(retry["state"], initial["state"])
            self.assertFalse(retry["first_party_exempt"])
        old_key = initial["results"][0]["artifact_key"]
        self.row["sha256"] = "8" * 64
        self.write(self.head)
        changed = self.check(NOW + timedelta(hours=72), initial["state"])
        new_key = changed["results"][0]["artifact_key"]
        self.assertNotEqual(new_key, old_key)
        self.assertEqual(changed["status"], "waiting")
        self.assertEqual(changed["state"]["first_seen"][new_key], "2026-11-06T12:00:00Z")
        self.row.update(
            currentVersion="2.50.10",
            filename="git-2.50.10.tar.xz",
            url="https://www.kernel.org/pub/software/scm/git/git-2.50.10.tar.xz",
        )
        self.write(self.head)
        version = self.check(NOW + timedelta(hours=72), initial["state"])
        self.assertNotEqual(version["results"][0]["artifact_key"], new_key)
        self.assertEqual(version["status"], "waiting")

    def test_integrity_failure_and_transient_retry_do_not_create_observation(self) -> None:
        from unittest.mock import patch

        for body in (
            b"missing",
            self.metadata("https://www.kernel.org/pub/software/scm/git/sha256sums.asc").replace(
                b"9" * 64, b"8" * 64
            ),
        ):
            with patch.object(self, "metadata", return_value=body):
                failed = self.check()
            self.assertEqual(failed["status"], "failure")
            self.assertEqual(failed["state"]["first_seen"], {})
            self.assertFalse(failed["retry_eligible"])
        with patch.object(self, "metadata", side_effect=urllib.error.URLError("offline")):
            failed = self.check()
        self.assertEqual(failed["status"], "failure")
        self.assertTrue(failed["retry_eligible"])
        self.assertEqual(failed["retry_at"], "2026-11-03T13:00:00Z")
        self.assertEqual(failed["state"]["first_seen"], {})
        retry = self.check(NOW + timedelta(hours=1), failed["state"])
        self.assertEqual(retry["results"][0]["evidence_at"], "2026-11-03T13:00:00Z")
        mature = self.check(NOW + timedelta(hours=73), retry["state"])
        self.assertEqual(mature["status"], "pass")
        with patch.object(self, "metadata", return_value=b"missing"):
            failed_again = self.check(NOW + timedelta(hours=73), mature["state"])
        self.assertEqual(failed_again["status"], "failure")
        self.assertEqual(failed_again["state"], mature["state"])

    def test_future_forged_state_and_noncanonical_source_fail_before_fetch(self) -> None:
        initial = self.check()
        self.calls.clear()
        key = initial["results"][0]["artifact_key"]
        for state in (
            {"schema_version": 1, "first_seen": {key: "2026-11-04T12:00:00Z"}},
            {"schema_version": 1, "first_seen": {"forged": "2026-10-01T00:00:00Z"}},
            {"schema_version": 1, "first_seen": {}, "candidate_date": OLD},
        ):
            with self.subTest(state=state):
                self.assertEqual(self.check(state=state)["status"], "failure")
        self.assertEqual(self.calls, [])
        self.row["url"] = "https://evil.invalid/git-2.50.9.tar.xz"
        self.write(self.head)
        self.assertEqual(self.check()["status"], "failure")
        self.assertEqual(self.calls, [])

    def test_direct_checker_rejects_complete_inventory_removal_and_retains_trusted_state(
        self,
    ) -> None:
        initial = self.check()
        self.write(self.base)
        (self.head / "automation/tool-artifacts.json").unlink()
        self.calls.clear()
        removed = self.check(state=initial["state"])
        self.assertEqual(removed["status"], "failure")
        self.assertIn("canonical tool inventory is missing", removed["error"])
        self.assertEqual(removed["results"], [])
        self.assertEqual(removed["state"], initial["state"])
        self.assertEqual(self.calls, [])
        fresh = self.check()
        self.assertEqual(fresh["status"], "failure")
        self.assertEqual(fresh["state"]["first_seen"], {})
        self.assertEqual(self.calls, [])

    def test_direct_checker_pair_boundary_preserves_supported_absence_and_row_removal(self) -> None:
        (self.head / "automation/tool-artifacts.json").unlink()
        absent = self.check()
        self.assertEqual(absent["status"], "pass")
        self.assertEqual(absent["state"]["first_seen"], {})
        self.assertEqual(self.calls, [])
        self.write(self.head)
        introduced = self.check()
        self.assertEqual(introduced["status"], "waiting")
        self.assertEqual(introduced["results"][0]["version"], "2.50.9")
        self.write(self.base)
        self.write(self.head, source=False)
        self.calls.clear()
        optional_removed = self.check()
        self.assertEqual(optional_removed["status"], "pass")
        self.assertEqual(optional_removed["state"]["first_seen"], {})
        self.assertEqual(self.calls, [])

    def test_git_metadata_uses_existing_shared_request_and_byte_budgets(self) -> None:
        from unittest.mock import patch

        from boxferry_ci import dependencies

        for name, limit, retry in (
            ("MAX_METADATA_REQUESTS", 0, True),
            ("MAX_METADATA_BYTES", 1, False),
            ("MAX_TOTAL_METADATA_BYTES", 1, False),
        ):
            with self.subTest(budget=name), patch.object(dependencies, name, limit):
                result = self.check()
            self.assertEqual(result["status"], "failure")
            self.assertEqual(result["retry_eligible"], retry)
            self.assertEqual(result["state"]["first_seen"], {})
        self.calls.clear()
        for package in (
            dependencies.Package("unsupported", "git", "2.50.9", "kernel.org", ("9" * 64,)),
            dependencies.Package(
                "git-source",
                "git",
                "2.50.9",
                "https://evil.invalid/",
                ("9" * 64, "git-2.50.9.tar.xz"),
            ),
        ):
            with self.assertRaises(ValueError):
                dependencies._metadata(package, self.metadata, NOW)
        self.assertEqual(self.calls, [])

    def test_unchanged_inventory_and_optional_source_removal_make_no_metadata_requests(
        self,
    ) -> None:
        self.write(self.base)
        self.assertEqual(self.check()["status"], "pass")
        self.write(self.head, source=False)
        self.assertEqual(self.check()["status"], "pass")
        self.assertEqual(self.calls, [])

    def test_git_cannot_inherit_valid_whole_closure_first_party_waiver(self) -> None:
        # Reuse the independently authored Cargo closure, then introduce Git while
        # presenting an otherwise eligible isolated Cargo context.
        fixtures = DependencyTests()
        fixtures.base, fixtures.head = self.base, self.head
        fixtures.first_party()
        context = fixtures.context()
        report = self.check(context=context)
        self.assertFalse(report["first_party_exempt"])
        self.assertIn("Git source", report["first_party_reason"])
        git = next(result for result in report["results"] if result["ecosystem"] == "git-source")
        self.assertEqual((git["status"], git["age_evidence"]), ("waiting", "artifact-first-seen"))


if __name__ == "__main__":
    unittest.main()
