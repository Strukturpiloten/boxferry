"""Synthetic immutable manifests exercise operational age, integrity, and parser boundaries."""

from __future__ import annotations

import hashlib
import io
import json
import sys
import tarfile
import tempfile
import unittest
import urllib.error
import urllib.request
from collections.abc import Mapping
from datetime import UTC, datetime, timedelta
from email.message import Message
from pathlib import Path
from typing import Any
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from boxferry_ci.dependencies import (
    DependencyContext,
    DependencyError,
    OciBlobRedirect,
    _NoMetadataRedirect,
    check_dependencies,
    fetch_metadata,
)
from boxferry_ci.operational import (
    FEATURE,
    FEATURE_LOCK,
    OperationalError,
    Pin,
    snapshot_paths,
    validate_feature_graph,
    verify_manifest,
)

NOW = datetime(2026, 11, 3, 12, tzinfo=UTC)
IMAGE_PATH = "automation/renovate-runtime.json"
MEDIA = "application/vnd.oci.image.manifest.v1+json"


def manifest(seed: str = "one", media: str = MEDIA) -> bytes:
    return json.dumps(
        {
            "schemaVersion": 2,
            "mediaType": media,
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": "sha256:" + hashlib.sha256(seed.encode()).hexdigest(),
                "size": len(seed),
            },
            "layers": [],
        },
        separators=(",", ":"),
    ).encode()


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def image(data: bytes, *, source: str = "ghcr.io/example/tool", tag: str = "1.0.0") -> str:
    return f"{source}:{tag}@{digest(data)}"


def feature_bundle(
    name: str = "github-cli", version: str = "1.1.0", dependencies: dict[str, Any] | None = None
) -> list[bytes]:
    metadata: dict[str, Any] = {"id": name, "version": version}
    if dependencies is not None:
        metadata["dependsOn"] = dependencies
    packaged = json.dumps(metadata).encode()
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as archive:
        member = tarfile.TarInfo("devcontainer-feature.json")
        member.size = len(packaged)
        archive.addfile(member, io.BytesIO(packaged))
    config, layer = b"{}", output.getvalue()
    body = json.dumps(
        {
            "schemaVersion": 2,
            "mediaType": MEDIA,
            "config": {
                "mediaType": "application/vnd.devcontainers",
                "digest": digest(config),
                "size": len(config),
            },
            "layers": [
                {
                    "mediaType": "application/vnd.devcontainers.layer.v1+tar",
                    "digest": digest(layer),
                    "size": len(layer),
                }
            ],
            "annotations": {"dev.containers.metadata": json.dumps(metadata)},
        }
    ).encode()
    return [body, config, layer]


class Registry:
    def __init__(self, blobs: list[bytes], *, authenticate: bool = False) -> None:
        self.blobs = {digest(blob): blob for blob in blobs}
        self.authenticate = authenticate
        self.calls: list[tuple[str, dict[str, str]]] = []

    def __call__(self, url: str, headers: Mapping[str, str]) -> bytes:
        self.calls.append((url, dict(headers)))
        if "/token?" in url or "/v2/auth?" in url:
            return b'{"token":"synthetic-anonymous-pull-token"}'
        if self.authenticate and "Authorization" not in headers:
            message = Message()
            message["WWW-Authenticate"] = 'Bearer realm="https://untrusted.invalid/token"'
            raise urllib.error.HTTPError(url, 401, "authentication", message, None)
        value = self.blobs.get(url.rsplit("/", 1)[1])
        if value is None:
            raise urllib.error.HTTPError(url, 404, "not found", Message(), None)
        return value


class OperationalTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.base, self.head = self.root / "base", self.root / "head"
        self.base.mkdir()
        self.head.mkdir()
        self.blob = manifest()
        self.registry = Registry([self.blob])

    def write(self, root: Path, path: str, text: str) -> None:
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def pin(
        self,
        root: Path,
        path: str = IMAGE_PATH,
        data: bytes | None = None,
        source: str = "ghcr.io/example/tool",
        tag: str = "1.0.0",
    ) -> None:
        self.write(
            root, path, json.dumps({"image": image(data or self.blob, source=source, tag=tag)})
        )

    def context(self, actor: str = "renovate[bot]") -> DependencyContext:
        paths = {
            p.relative_to(root).as_posix()
            for root in (self.base, self.head)
            for p in root.rglob("*")
            if p.is_file()
        }
        return DependencyContext(
            actor, "Bot", "Strukturpiloten/boxferry", "a" * 40, "b" * 40, tuple(sorted(paths))
        )

    def check(
        self,
        *,
        now: datetime = NOW,
        state: dict[str, Any] | None = None,
        context: DependencyContext | None = None,
    ) -> dict[str, Any]:
        return check_dependencies(
            self.base,
            self.head,
            now=now,
            state=state,
            context=context or self.context(),
            fetch_request=self.registry,
        )

    def features(
        self,
        root: Path,
        *,
        locked: bool = True,
        version: str = "1.1.0",
        option: str = "2.103.2",
        data: bytes | None = None,
    ) -> None:
        source = f"ghcr.io/devcontainers/features/github-cli:{version}"
        self.write(root, FEATURE, json.dumps({"features": {source: {"version": option}}}))
        if locked:
            bundle = feature_bundle(version=version)
            self.registry.blobs.update({digest(blob): blob for blob in bundle})
            value = data or bundle[0]
            entry = {
                "version": version,
                "resolved": f"ghcr.io/devcontainers/features/github-cli@{digest(value)}",
                "integrity": digest(value),
            }
            self.write(root, FEATURE_LOCK, json.dumps({"features": {source: entry}}))

    def test_new_manifest_waits_exactly_72_hours_then_reuses_verified_identity(self) -> None:
        self.pin(self.head)
        first = self.check()
        self.assertEqual(first["status"], "waiting")
        self.assertEqual(first["retry_at"], "2026-11-06T12:00:00Z")
        self.assertEqual(len(self.registry.calls), 1)
        self.registry.calls.clear()
        later = self.check(now=NOW + timedelta(hours=72), state=first["state"])
        self.assertEqual(later["status"], "pass")
        self.assertEqual(self.registry.calls, [])
        self.assertTrue(first["results"][0]["artifact_immutable"])

    def test_tag_path_and_base_changes_do_not_reset_observation(self) -> None:
        self.pin(self.head)
        first = self.check()
        (self.head / IMAGE_PATH).unlink()
        self.pin(self.head, "scripts/renovate-tool.json", tag="new-alias")
        self.write(self.base, "Cargo.lock", "version = 4\n")
        self.write(self.head, "Cargo.lock", "version = 4\n")
        second = self.check(now=NOW + timedelta(hours=1), state=first["state"])
        self.assertEqual(second["retry_at"], first["retry_at"])
        self.assertEqual(second["state"], first["state"])
        self.assertEqual(second["results"][0]["artifact_key"], first["results"][0]["artifact_key"])

    def test_same_bytes_retagged_or_moved_from_base_are_not_introduced(self) -> None:
        self.pin(self.base)
        self.pin(self.head, "scripts/renovate-tool.json", tag="alias")
        report = self.check()
        self.assertEqual(report["status"], "pass")
        self.assertEqual(self.registry.calls, [])

    def test_new_digest_for_same_tag_starts_new_observation(self) -> None:
        self.pin(self.head)
        initial = self.check()
        other = manifest("different")
        self.registry = Registry([other])
        self.pin(self.head, data=other)
        later = self.check(now=NOW + timedelta(days=4), state=initial["state"])
        self.assertEqual(later["status"], "waiting")
        self.assertEqual(len(later["state"]["first_seen"]), 2)

    def test_manifest_digest_media_and_descriptor_failures_never_start_clock(self) -> None:
        for body in (
            manifest(media="application/octet-stream"),
            b'{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json"}',
        ):
            with self.subTest(body=body):
                self.registry = Registry([body])
                self.pin(self.head, data=body)
                report = self.check()
                self.assertEqual(report["status"], "failure")
                self.assertEqual(report["state"]["first_seen"], {})
        self.pin(self.head)
        self.registry = Registry([self.blob])
        with patch.object(self.registry, "blobs", {digest(self.blob): manifest("wrong bytes")}):
            self.assertEqual(self.check()["status"], "failure")

    def test_anonymous_auth_uses_only_reviewed_origin_and_exact_manifest(self) -> None:
        self.pin(self.head)
        self.registry.authenticate = True
        report = self.check()
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(len(self.registry.calls), 3)
        self.assertTrue(self.registry.calls[1][0].startswith("https://ghcr.io/token?"))
        for url, headers in self.registry.calls:
            self.assertNotIn("untrusted.invalid", url)
            if "Authorization" in headers:
                self.assertIn("/manifests/sha256:", url)
        self.assertNotIn("synthetic-anonymous-pull-token", json.dumps(report))

    def test_quay_probe_is_a_literal_and_never_executed(self) -> None:
        source = "# renovate: datasource=docker depName=quay.io/libpod/alpine\n"
        self.write(
            self.head,
            "scripts/podman-live-conformance.sh",
            source + f'workload_image="quay.io/libpod/alpine@{digest(self.blob)}"\n',
        )
        self.assertEqual(self.check()["status"], "waiting")
        self.write(
            self.head,
            "scripts/podman-live-conformance.sh",
            source + 'workload_image="$(touch forbidden)"\n',
        )
        self.assertEqual(self.check()["status"], "failure")
        self.assertFalse((self.root / "forbidden").exists())

    def test_dockerfile_annotation_both_from_forms_and_stage_alias(self) -> None:
        for tag in ("", ":1.0.0"):
            text = "# renovate: datasource=docker depName=ghcr.io/example/tool\n# release: 1.0.0\n"
            self.write(
                self.head,
                ".devcontainer/Dockerfile",
                text + f"FROM ghcr.io/example/tool{tag}@{digest(self.blob)} AS toolchain\n",
            )
            self.assertEqual(self.check()["status"], "waiting")
        self.write(
            self.head,
            ".devcontainer/Dockerfile",
            text + f"FROM ghcr.io/example/tool:2.0.0@{digest(self.blob)}\n",
        )
        self.assertEqual(self.check()["status"], "failure")

    def test_catalogue_reader_only_owns_explicit_active_paths(self) -> None:
        path = "fixtures/conformance/immich-application/images.tsv"
        self.write(
            self.head, path, "# role\timage\nserver\t" + image(self.blob) + "\tignored provenance\n"
        )
        self.assertEqual(self.check()["status"], "waiting")
        (self.head / path).unlink()
        self.write(
            self.head, "tools/generator-history.toml", 'image = "' + image(self.blob) + '"\n'
        )
        self.assertEqual(self.check()["results"], [])

    def test_feature_requires_matching_immutable_lock_when_changed(self) -> None:
        self.features(self.head)
        self.assertEqual(self.check()["status"], "waiting")
        document = json.loads((self.head / FEATURE_LOCK).read_text())
        entry = next(iter(document["features"].values()))
        entry["integrity"] = "sha256:" + "b" * 64
        self.write(self.head, FEATURE_LOCK, json.dumps(document))
        self.assertEqual(self.check()["status"], "failure")
        (self.head / FEATURE_LOCK).unlink()
        self.assertEqual(self.check()["status"], "failure")

    def test_unchanged_missing_feature_lock_does_not_block_unrelated_update(self) -> None:
        self.features(self.base, locked=False)
        self.features(self.head, locked=False)
        self.assertEqual(self.check()["status"], "pass")
        self.features(self.head, locked=False, option="2.104.0")
        self.assertEqual(self.check()["status"], "failure")

    def test_feature_option_change_does_not_restart_valid_artifact_age(self) -> None:
        self.features(self.base)
        self.features(self.head, option="2.104.0")
        report = self.check()
        self.assertEqual(report["status"], "pass")
        self.assertEqual(len(self.registry.calls), 3)

    def test_feature_lock_removal_and_duplicate_json_keys_fail(self) -> None:
        self.features(self.base)
        self.features(self.head, locked=False)
        self.assertEqual(self.check()["status"], "failure")
        self.write(self.head, FEATURE, '{"features":{},"features":{}}')
        self.assertEqual(self.check()["status"], "failure")

    def graph(
        self,
        *,
        dependency: str = "ghcr.io/devcontainers/features/common-utils:2",
        child_version: str = "2.4.1",
    ) -> dict[str, Any]:
        root = "ghcr.io/devcontainers/features/github-cli:1.1.0"
        parent = feature_bundle(dependencies={dependency: {"configureZshAsDefaultShell": False}})
        child = feature_bundle(name="common-utils", version=child_version)
        self.registry = Registry([*parent, *child], authenticate=True)
        self.write(self.head, FEATURE, json.dumps({"features": {root: {}}}))
        locked = {
            "features": {
                root: {
                    "version": "1.1.0",
                    "resolved": "ghcr.io/devcontainers/features/github-cli@" + digest(parent[0]),
                    "integrity": digest(parent[0]),
                    "dependsOn": [dependency],
                },
                dependency: {
                    "version": child_version,
                    "resolved": "ghcr.io/devcontainers/features/common-utils@" + digest(child[0]),
                    "integrity": digest(child[0]),
                },
            }
        }
        self.write(self.head, FEATURE_LOCK, json.dumps(locked))
        return locked

    def test_official_transitive_graph_metadata_content_and_all_member_age(self) -> None:
        self.graph()
        report = self.check()
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(len(report["results"]), 2)
        self.assertEqual(len(report["state"]["first_seen"]), 2)
        later = self.check(now=NOW + timedelta(hours=72), state=report["state"])
        self.assertEqual(later["status"], "pass")
        self.assertEqual(self.check(now=NOW + timedelta(hours=72))["status"], "waiting")

    def test_transitive_numeric_minor_and_digest_selectors(self) -> None:
        self.graph(dependency="ghcr.io/devcontainers/features/common-utils:2.4")
        self.assertEqual(len(validate_feature_graph(self.head, self.registry)), 2)
        child = feature_bundle(name="common-utils", version="2.4.1")
        self.graph(dependency="ghcr.io/devcontainers/features/common-utils@" + digest(child[0]))
        self.assertEqual(len(validate_feature_graph(self.head, self.registry)), 2)
        self.graph(child_version="3.0.0")
        self.assertEqual(self.check()["status"], "failure")

    def test_orphan_missing_cycle_unknown_field_and_metadata_edge_mismatch(self) -> None:
        for fault in ("orphan", "missing", "cycle", "field", "metadata"):
            with self.subTest(fault=fault):
                lock = self.graph()
                root, child = list(lock["features"])
                if fault == "orphan":
                    lock["features"][root].pop("dependsOn")
                elif fault == "missing":
                    del lock["features"][child]
                elif fault == "cycle":
                    lock["features"][child]["dependsOn"] = [root]
                elif fault == "field":
                    lock["features"][child]["install"] = "candidate-command"
                else:
                    lock["features"].pop(child)
                    lock["features"][root].pop("dependsOn")
                self.write(self.head, FEATURE_LOCK, json.dumps(lock))
                self.assertEqual(self.check()["status"], "failure")

    def test_feature_layer_integrity_and_packaged_metadata_are_independent(self) -> None:
        lock = self.graph()
        child = next(key for key in lock["features"] if key.endswith(":2"))
        manifest_body = self.registry.blobs[lock["features"][child]["integrity"]]
        layer_digest = json.loads(manifest_body)["layers"][0]["digest"]
        self.registry.blobs[layer_digest] = b"not the held archive"
        report = self.check()
        self.assertEqual(report["status"], "failure")
        self.assertEqual(report["state"]["first_seen"], {})

    def test_transitive_only_republication_starts_its_own_hold(self) -> None:
        self.graph()
        first = self.check()
        for path in (FEATURE, FEATURE_LOCK):
            self.write(self.base, path, (self.head / path).read_text())
        lock = self.graph(child_version="2.4.2")
        self.assertEqual(len(lock["features"]), 2)
        report = self.check(now=NOW + timedelta(days=4), state=first["state"])
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(len(report["results"]), 1)
        self.assertEqual(report["results"][0]["version"], "2.4.2")

    def test_invalid_base_orphan_cannot_launder_a_new_reachable_identity(self) -> None:
        lock = self.graph()
        for path in (FEATURE, FEATURE_LOCK):
            self.write(self.base, path, (self.head / path).read_text())
        source = next(iter(lock["features"]))
        lock["features"][source].pop("dependsOn")
        self.write(self.base, FEATURE_LOCK, json.dumps(lock))
        report = self.check()
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(len(report["results"]), 2)

    def test_missing_optional_annotation_uses_verified_packaged_metadata(self) -> None:
        lock = self.graph()
        source = next(iter(lock["features"]))
        old_digest = lock["features"][source]["integrity"]
        manifest = json.loads(self.registry.blobs.pop(old_digest))
        manifest.pop("annotations")
        body = json.dumps(manifest).encode()
        self.registry.blobs[digest(body)] = body
        lock["features"][source]["integrity"] = digest(body)
        lock["features"][source]["resolved"] = (
            "ghcr.io/devcontainers/features/github-cli@" + digest(body)
        )
        self.write(self.head, FEATURE_LOCK, json.dumps(lock))
        self.assertEqual(len(validate_feature_graph(self.head, self.registry)), 2)

    def test_standalone_graph_rejects_float_schema_and_noncanonical_transitive_semver(self) -> None:
        lock = self.graph()
        source = next(iter(lock["features"]))
        entry = lock["features"][source]
        manifest = json.loads(self.registry.blobs[entry["integrity"]])
        manifest["schemaVersion"] = 2.0
        encoded = json.dumps(manifest).encode()
        self.registry.blobs[digest(encoded)] = encoded
        entry["integrity"] = digest(encoded)
        entry["resolved"] = entry["resolved"].rsplit("@", 1)[0] + "@" + digest(encoded)
        self.write(self.head, FEATURE_LOCK, json.dumps(lock))
        with self.assertRaisesRegex(OperationalError, "supported OCI schema"):
            validate_feature_graph(self.head, self.registry)
        self.graph(child_version="2.04.1")
        with self.assertRaisesRegex(OperationalError, "exact semantic version"):
            validate_feature_graph(self.head, self.registry)

    def test_realistic_config_and_layer_redirects_share_all_request_budgets(self) -> None:
        self.graph()
        underlying = self.registry
        redirects: list[tuple[str, dict[str, str]]] = []

        def request(url: str, headers: Mapping[str, str]) -> bytes:
            if url.startswith("https://pkg-containers.githubusercontent.com/"):
                redirects.append((url, dict(headers)))
                return underlying.blobs[url.split("?", 1)[0].rsplit("/", 1)[1]]
            if "/blobs/sha256:" in url:
                underlying.calls.append((url, dict(headers)))
                raise OciBlobRedirect(
                    "https://pkg-containers.githubusercontent.com/ghcr1/blobs/"
                    + url.rsplit("/", 1)[1]
                    + "?sig=synthetic-private"
                )
            return underlying(url, headers)

        self.assertEqual(len(validate_feature_graph(self.head, request)), 2)
        self.assertEqual(len(redirects), 4)
        self.assertTrue(all(headers == {} for _url, headers in redirects))
        with patch("boxferry_ci.dependencies.MAX_METADATA_REQUESTS", 6):
            report = check_dependencies(
                self.base, self.head, now=NOW, context=self.context(), fetch_request=request
            )
        self.assertEqual(report["status"], "failure")
        self.assertTrue(report["retry_eligible"])
        self.assertEqual(report["state"]["first_seen"], {})
        self.assertNotIn("synthetic-private", json.dumps(report))

    def test_official_selector_alias_collapse_is_explicitly_unsupported(self) -> None:
        lock = self.graph()
        child = next(source for source in lock["features"] if source.endswith(":2"))
        lock["features"][child + ".4"] = lock["features"].pop(child)
        self.write(self.head, FEATURE_LOCK, json.dumps(lock))
        with self.assertRaisesRegex(OperationalError, "selector alias collapse"):
            validate_feature_graph(self.head, self.registry)

    def test_feature_future_and_malformed_publication_times_fail_before_observation(self) -> None:
        for created in ("2030-01-01T00:00:00Z", "not-a-date", 42):
            with self.subTest(created=created):
                lock = self.graph()
                source = next(iter(lock["features"]))
                body = json.loads(self.registry.blobs[lock["features"][source]["integrity"]])
                body["annotations"]["org.opencontainers.image.created"] = created
                encoded = json.dumps(body).encode()
                self.registry.blobs[digest(encoded)] = encoded
                lock["features"][source]["integrity"] = digest(encoded)
                lock["features"][source]["resolved"] = (
                    "ghcr.io/devcontainers/features/github-cli@" + digest(encoded)
                )
                self.write(self.head, FEATURE_LOCK, json.dumps(lock))
                report = self.check()
                self.assertEqual(report["status"], "failure")
                self.assertEqual(report["state"]["first_seen"], {})

    def test_runner_direct_include_and_marked_matrix_selectors(self) -> None:
        text = """---
jobs:
  direct:
    runs-on: ubuntu-26.04
  matrix:
    runs-on: ${{ matrix.runs-on }}
    strategy:
      matrix:
        include:
          - target: linux
            runs-on: ubuntu-26.04-arm
          - target: apple
            runs-on: macos-26-intel
  provider:
    runs-on: ${{ matrix.runner }}
    strategy:
      matrix:
        # renovate: datasource=github-runners
        runner:
          - ubuntu-26.04
"""
        self.write(self.head, ".github/workflows/ci.yml", text)
        report = self.check()
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(
            {item["version"] for item in report["results"]},
            {"ubuntu-26.04", "ubuntu-26.04-arm", "macos-26-intel"},
        )
        self.assertTrue(all(not item["artifact_immutable"] for item in report["results"]))
        self.assertTrue(
            all(item["age_evidence"] == "runner-label-first-seen" for item in report["results"])
        )
        self.assertEqual(self.registry.calls, [])

    def test_runner_block_strings_env_and_self_hosted_are_not_hosted_pins(self) -> None:
        text = """---
jobs:
  test:
    runs-on: [self-hosted, linux]
    env:
      runs-on: ubuntu-99.04
    steps:
      - name: "quoted continuation
          runs-on: ubuntu-99.04"
        run: |
          runs-on: ubuntu-99.04
"""
        self.write(self.head, ".github/workflows/ci.yml", text)
        self.assertEqual(self.check()["results"], [])

    def test_changed_unknown_runner_grammar_fails_instead_of_optional_bypass(self) -> None:
        self.write(
            self.head,
            ".github/workflows/ci.yml",
            "jobs:\n  test:\n    runs-on: ${{ fromJSON(inputs.runner) }}\n",
        )
        self.assertEqual(self.check()["status"], "failure")

    def test_unsupported_flow_runner_and_noncanonical_from_do_not_bypass(self) -> None:
        self.write(self.head, ".github/workflows/ci.yml", "jobs: {test: {runs-on: ubuntu-26.04}}\n")
        self.assertEqual(self.check()["status"], "failure")
        (self.head / ".github/workflows/ci.yml").unlink()
        self.write(
            self.head,
            ".devcontainer/Dockerfile",
            "# renovate: datasource=docker depName=ghcr.io/example/tool\n"
            "# release: 1.0.0\nfrom " + image(self.blob) + "\n",
        )
        self.assertEqual(self.check()["status"], "failure")

    def test_supported_dockerhub_and_unknown_registry_routes_remain_distinct(self) -> None:
        self.pin(self.head, source="docker.io/library/tool")
        self.assertEqual(self.check()["results"], [])
        for source in ("mcr.microsoft.com/other/tool", "code.forgejo.org/forgejo/forgejo"):
            self.pin(self.head, source=source)
            report = self.check()
            self.assertEqual(report["status"], "pass")
            self.assertTrue(report["operational_coverage_gaps"])
            self.assertEqual(report["state"]["first_seen"], {})

    def test_reviewed_mcr_and_actual_codeberg_forgejo_manifests_are_covered(self) -> None:
        for source in ("mcr.microsoft.com/devcontainers/rust", "codeberg.org/forgejo/forgejo"):
            self.pin(self.head, source=source)
            self.assertEqual(self.check()["status"], "waiting")
        self.registry = Registry([self.blob], authenticate=True)
        report = self.check()
        self.assertEqual(report["status"], "waiting")
        self.assertTrue(self.registry.calls[1][0].startswith("https://codeberg.org/v2/token?"))
        self.assertIn("service=container_registry", self.registry.calls[1][0])
        self.pin(self.head, source="mcr.microsoft.com/devcontainers/rust")
        self.assertEqual(self.check()["status"], "failure")

    def test_feature_lock_partial_version_or_source_replacement_is_rejected(self) -> None:
        self.features(self.base)
        self.features(self.head, version="1.2.0")
        lock = json.loads((self.head / FEATURE_LOCK).read_text())
        entry = next(iter(lock["features"].values()))
        entry["version"] = "1.1.0"
        self.write(self.head, FEATURE_LOCK, json.dumps(lock))
        self.assertEqual(self.check()["status"], "failure")
        entry["version"] = "1.2.0"
        entry["resolved"] = "ghcr.io/lookalike/feature@" + digest(self.blob)
        self.write(self.head, FEATURE_LOCK, json.dumps(lock))
        self.assertEqual(self.check()["status"], "failure")

    def test_fixed_qualified_podman_and_fcos_are_not_rediscovered(self) -> None:
        paths = ["scripts/native-runtime-pins.sh", "scripts/kind-vm-image.json"]
        self.assertEqual(snapshot_paths(paths), set())
        for path in paths:
            self.write(self.head, path, json.dumps({"image": image(self.blob)}))
        self.assertEqual(self.check()["results"], [])

    def test_snapshot_counterparts_rename_paths_and_path_boundaries(self) -> None:
        self.assertEqual(snapshot_paths([FEATURE]), {FEATURE, FEATURE_LOCK})
        self.assertEqual(
            snapshot_paths([".github/workflows/old.yml", ".github/workflows/new.yml"]),
            {".github/workflows/old.yml", ".github/workflows/new.yml"},
        )
        self.assertEqual(snapshot_paths([".github/workflows/../secrets.yml", "README.md"]), set())

    def test_missing_snapshot_integration_fails_closed(self) -> None:
        context = DependencyContext(
            "renovate[bot]", "Bot", "Strukturpiloten/boxferry", "a" * 40, "b" * 40, (IMAGE_PATH,)
        )
        report = self.check(context=context)
        self.assertEqual(report["status"], "failure")
        self.assertIn("missing from exact snapshots", report["results"][0]["reason"])

    def test_human_new_operational_pin_receives_the_same_age_hold(self) -> None:
        self.pin(self.head)
        report = self.check(context=self.context(actor="maintainer"))
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(len(self.registry.calls), 1)
        self.pin(self.base)
        self.registry.calls.clear()
        self.assertEqual(self.check(context=self.context(actor="maintainer"))["status"], "pass")
        self.assertEqual(self.registry.calls, [])

    def test_missing_snapshot_roots_are_not_a_false_green(self) -> None:
        for base, head in ((self.root / "missing", self.head), (self.base, self.root / "missing")):
            report = check_dependencies(base, head, now=NOW, fetch_request=self.registry)
            self.assertEqual(report["status"], "failure")
        self.assertEqual(check_dependencies(self.base, self.head, now=NOW)["status"], "pass")

    def test_registry_first_party_flag_never_bypasses_operational_outcome(self) -> None:
        self.pin(self.head)
        with patch(
            "boxferry_ci.dependencies._first_party_scope", return_value=(True, "fixture proof")
        ):
            report = self.check()
        self.assertTrue(report["first_party_exempt"])
        self.assertEqual(report["status"], "waiting")
        self.assertEqual(report["results"][0]["age_evidence"], "manifest-first-seen")

    def test_shared_state_capacity_and_future_observations_fail_closed(self) -> None:
        self.pin(self.head)
        with patch("boxferry_ci.dependencies.MAX_STATE_ENTRIES", 0):
            self.assertEqual(self.check()["status"], "failure")
        key = Pin("oci", "ghcr.io", "example/tool", digest(self.blob), "1", "ignored", 1).key
        self.registry.calls.clear()
        report = self.check(
            state={"schema_version": 1, "first_seen": {key: "2030-01-01T00:00:00Z"}}
        )
        self.assertEqual(report["status"], "failure")
        self.assertEqual(self.registry.calls, [])

    def test_registry_and_operational_responses_share_one_byte_budget(self) -> None:
        self.pin(self.head)
        with patch("boxferry_ci.dependencies.MAX_TOTAL_METADATA_BYTES", len(self.blob) - 1):
            report = self.check()
        self.assertEqual(report["status"], "failure")
        self.assertIn("shared metadata", report["results"][0]["reason"])
        self.assertEqual(report["state"]["first_seen"], {})

    def test_registry_plus_operational_metadata_share_the_same_total_budget(self) -> None:
        self.pin(self.head)
        lock = (
            'version = 4\n[[package]]\nname = "serde"\nversion = "1.0.228"\n'
            'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
            'checksum = "' + "a" * 64 + '"\n'
        )
        self.write(self.head, "Cargo.lock", lock)
        registry_body = json.dumps(
            {
                "version": {
                    "crate": "serde",
                    "num": "1.0.228",
                    "checksum": "a" * 64,
                    "created_at": "2026-01-01T00:00:00Z",
                }
            }
        ).encode()
        limit = max(len(registry_body), len(self.blob)) + 1
        with patch("boxferry_ci.dependencies.MAX_TOTAL_METADATA_BYTES", limit):
            report = check_dependencies(
                self.base,
                self.head,
                now=NOW,
                context=self.context(),
                fetch=lambda _: registry_body,
                fetch_request=self.registry,
            )
        self.assertEqual(report["status"], "failure")
        self.assertEqual(report["results"][0]["status"], "pass")
        self.assertIn("shared metadata", report["results"][1]["reason"])
        self.assertEqual(report["state"]["first_seen"], {})

    def test_auth_attempts_share_request_budget_and_retry_eligibility(self) -> None:
        self.pin(self.head)
        self.registry.authenticate = True
        with patch("boxferry_ci.dependencies.MAX_METADATA_REQUESTS", 1):
            report = self.check()
        self.assertEqual(len(self.registry.calls), 1)
        self.assertEqual(report["status"], "failure")
        self.assertTrue(report["retry_eligible"])
        self.assertEqual(report["retry_at"], "2026-11-03T13:00:00Z")
        self.assertEqual(report["state"]["first_seen"], {})

    def test_manifest_server_failure_retries_without_starting_observation(self) -> None:
        self.pin(self.head)

        def unavailable(url: str, headers: Mapping[str, str]) -> bytes:
            raise urllib.error.HTTPError(url, 503, "unavailable", Message(), None)

        report = check_dependencies(
            self.base, self.head, now=NOW, context=self.context(), fetch_request=unavailable
        )
        self.assertEqual(report["status"], "failure")
        self.assertTrue(report["retry_eligible"])
        self.assertEqual(report["state"]["first_seen"], {})

    def test_manifest_helper_never_calls_unreviewed_origins(self) -> None:
        for source, name in (("untrusted.invalid", "image"), ("quay.io", "other/image")):
            with self.assertRaises(OperationalError):
                verify_manifest(
                    Pin("oci", source, name, digest(self.blob), "1", "fixture", 1), self.registry
                )
        self.assertEqual(self.registry.calls, [])

    def test_exact_metadata_redirects_cannot_forward_credentials(self) -> None:
        request = urllib.request.Request(
            "https://ghcr.io/v2/example/tool/manifests/sha256:" + "a" * 64,
            headers={"Authorization": "Bearer fixture"},
        )
        with self.assertRaises(DependencyError):
            _NoMetadataRedirect().redirect_request(
                request, io.BytesIO(), 302, "redirect", Message(), "https://untrusted.invalid/token"
            )
        with self.assertRaises(DependencyError):
            fetch_metadata("http://ghcr.io/v2/example")


if __name__ == "__main__":
    unittest.main()
