#!/usr/bin/env python3
"""Independent, private-value-safe controls for the one controlled network default."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


HELPER = Path(__file__).parent / "lib/apply-reacquire-contract.py"
NETWORK = "fixture-apply-net"
PUBLIC = "---\nname: fixture\nservices:\n  web:\n    image: example.invalid/web:1\n    volumes: [data:/data]\n    networks: [fixture-apply-net]\nnetworks:\n  fixture-apply-net: {}\nvolumes:\n  data: {}\n"
EXPLICIT = PUBLIC.replace("  fixture-apply-net: {}\n", "  fixture-apply-net:\n    internal: false\n")


class ApplyReacquireContractTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / "source.yaml"
        self.target = self.root / "target.yaml"

    def compare(self, source, target, success):
        self.source.write_text(source)
        self.target.write_text(target)
        result = subprocess.run([sys.executable, "-S", str(HELPER), str(self.source), str(self.target), NETWORK],
                                capture_output=True, check=False, timeout=5)
        self.assertEqual(result.returncode == 0, success)
        self.assertEqual(result.stdout, b"")
        self.assertNotIn(b"private-value-never-print", result.stderr)
        self.assertNotIn(b"Traceback", result.stderr)
        # Comparison never rewrites raw evidence.
        self.assertEqual(self.source.read_text(), source)
        self.assertEqual(self.target.read_text(), target)

    def test_only_default_false_representation_varies(self):
        for source, target in ((PUBLIC, PUBLIC), (PUBLIC, EXPLICIT), (EXPLICIT, PUBLIC), (EXPLICIT, EXPLICIT)):
            with self.subTest(source=source == PUBLIC, target=target == PUBLIC):
                self.compare(source, target, True)

    def test_isolation_and_ownership_never_normalize(self):
        for declaration in ("    internal: true\n", "    internal: null\n", '    internal: "false"\n',
                            "    external: true\n", "    external: false\n", "    driver: bridge\n",
                            "    labels: {private: private-value-never-print}\n",
                            "    internal: false\n    internal: false\n"):
            changed = EXPLICIT.replace("    internal: false\n", declaration)
            # Even identical isolated/ownership/unreviewed declarations must fail.
            self.compare(changed, changed, False)
            self.compare(PUBLIC, changed, False)

    def test_wrong_missing_or_extra_resource(self):
        for changed in (PUBLIC.replace(NETWORK, "other-network"),
                        PUBLIC.replace("networks:\n  fixture-apply-net: {}\n", ""),
                        PUBLIC.replace("networks:\n  fixture-apply-net: {}\n", "networks:\n  fixture-apply-net: {}\n  other: {}\n")):
            self.compare(changed, changed, False)

    def test_every_other_byte_remains_strict(self):
        for changed in (EXPLICIT.replace("example.invalid/web:1", "private-value-never-print"),
                        EXPLICIT.replace("data:/data", "data:/other"),
                        EXPLICIT.replace("  data: {}", "  other: {}"),
                        EXPLICIT.replace("  web:", "  other:"),
                        EXPLICIT.replace("    image:", "    labels: {private: private-value-never-print}\n    image:"),
                        EXPLICIT.replace("name: fixture", "name: other"),
                        EXPLICIT.replace("volumes:\n", "# unrelated formatting\nvolumes:\n"),
                        EXPLICIT.replace("    internal: false", "    internal: false # comment")):
            self.compare(PUBLIC, changed, False)

    def test_generated_envelope_rejects_malformed_or_ambiguous_documents(self):
        for changed in (
            PUBLIC + "---\nname: private-value-never-print\n",
            PUBLIC + "...\n",
            "---\n" + PUBLIC,
            PUBLIC + "services:\n  duplicate:\n    image: private-value-never-print\n",
            PUBLIC + "name: duplicate\n",
            PUBLIC + "volumes:\n  duplicate: {}\n",
            PUBLIC + '"networks":\n  fixture-apply-net: {}\n',
            PUBLIC + "'services':\n  duplicate:\n    image: private-value-never-print\n",
            PUBLIC.replace("networks:\n  fixture-apply-net", '"networks":\n  fixture-apply-net'),
            PUBLIC.replace("services:\n", "services: []\n"),
            PUBLIC.replace("services:\n", "services: [private-value-never-print\n"),
            PUBLIC.replace("volumes:\n  data: {}\n", "volumes: {}\n"),
            PUBLIC.replace("name: fixture", "name: [fixture]"),
            PUBLIC.replace("name: fixture", "name:"),
            PUBLIC.replace("  web:\n", "  - web\n"),
            PUBLIC.replace("  web:\n", "  web: {}\n"),
            PUBLIC.replace("  web:\n", "   web:\n"),
            PUBLIC.replace("    image:", "\timage:"),
            PUBLIC.replace("    image:", "    missing-colon\n    image:"),
            PUBLIC.replace("name: fixture", "name: fixture\x00"),
            PUBLIC.replace("name: fixture\n", ""),
            PUBLIC.replace("  data: {}", "  data: {}\n  other: {}"),
        ):
            with self.subTest(case=changed[:40]):
                self.compare(changed, changed, False)

    def test_dependency_free_without_site_packages_and_optional_leading_marker(self):
        # Every helper invocation uses -S, so an undeclared PyYAML import fails here.
        self.compare(PUBLIC.removeprefix("---\n"), EXPLICIT.removeprefix("---\n"), True)
        # The optional document marker is still an otherwise significant byte difference.
        self.compare(PUBLIC, PUBLIC.removeprefix("---\n"), False)
        self.compare(PUBLIC.replace("web:1", "public-\u00e4:1"),
                     EXPLICIT.replace("web:1", "public-\u00e4:1"), True)

    def test_nonregular_and_bounded_sources(self):
        self.target.write_text(PUBLIC)
        for kind in ("fifo", "symlink", "directory", "empty", "oversized", "invalid-utf8"):
            with self.subTest(kind=kind):
                if kind == "fifo": os.mkfifo(self.source)
                elif kind == "symlink": self.source.symlink_to(self.target)
                elif kind == "directory": self.source.mkdir()
                elif kind == "empty": self.source.write_bytes(b"")
                elif kind == "oversized": self.source.write_bytes(b"x" * (8 * 1024 * 1024 + 1))
                else: self.source.write_bytes(b"private-value-never-print\xff")
                result = subprocess.run([sys.executable, "-S", str(HELPER), str(self.source), str(self.target), NETWORK],
                                        capture_output=True, check=False, timeout=5)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")
                self.assertNotIn(b"private-value-never-print", result.stderr)
                self.assertNotIn(b"Traceback", result.stderr)
                if kind == "directory": self.source.rmdir()
                else: self.source.unlink()

    def test_live_route_keeps_exact_podman_comparison(self):
        # Execute the real route with inert callbacks: a Podman byte mismatch must
        # stop before Compose comparison/cleanup and produce only a static diagnostic.
        live = (HELPER.parent.parent / "podman-live-conformance.sh").read_text()
        function = live.split("run_external_apply_reacquire() {", 1)[1].split("\n}\n", 1)[0]
        plan = '{"external_preconditions":[],"operations":[{"action":"create","resource":{"kind":"network","name":"fixture-apply-net"},"cli":{"external_sensitive_input_required":false}},{"action":"create","resource":{"kind":"volume","name":"fixture-apply-data"},"cli":{"external_sensitive_input_required":false}},{"action":"create","resource":{"kind":"container","name":"fixture-apply-web"},"cli":{"external_sensitive_input_required":false}}]}'
        for route in ("apply-source-podman", "apply-target-podman", "apply-source-compose", "apply-target-compose"):
            directory = self.root / "outputs" / route
            directory.mkdir(parents=True)
            (directory / "podman.json").write_text(plan)
            (directory / "podman-commands.sh").write_text(":\n")
            (directory / "compose.yaml").write_text(PUBLIC)
        target = self.root / "outputs/apply-target-podman/podman.json"
        target.write_text(plan + " \n")
        script = '''set -Eeuo pipefail
require_scenario() { :; }; run_convert() { :; }; start_apply_target() { :; }
timed_operation() { :; }; engine_operation() { :; }; activate_outer_runtime() { :; }
current_case=$1; current_prefix=fixture; runtime_root=$1
apply_target_outer=inert; apply_target_socket=inert; engine=inert; script_directory=$2
run_external_apply_reacquire() {\n''' + function + '\n}\nrun_external_apply_reacquire inert'
        result = subprocess.run(["bash", "-c", script, "route-control", str(self.root), str(HELPER.parent.parent)],
                                capture_output=True, check=False, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"Podman-to-Podman source/target podman.json bytes", result.stderr)
        self.assertNotIn(b"private-value-never-print", result.stderr)


if __name__ == "__main__":
    unittest.main()
