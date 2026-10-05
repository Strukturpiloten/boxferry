#!/usr/bin/env python3
"""Offline failure contracts; mocked Engine calls are not live evidence."""

from __future__ import annotations

import copy
import errno
import hashlib
import importlib.util
import json
import os
import pathlib
import shlex
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest import mock


SOURCE = pathlib.Path(__file__).parent / "lib/docker-application-contract.py"
SPEC = importlib.util.spec_from_file_location("docker_application_contract", SOURCE)
assert SPEC is not None and SPEC.loader is not None
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)


def authored_reacquired_compose(image: str) -> bytes:
    command = "printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600"
    path = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    return ("---\nname: bf-docker-core\nservices:\n  docker-service-1:\n"
            f"    image: {image}\n    container_name: bf-docker-core\n"
            "    command:\n      - sh\n      - \"-c\"\n      - " + json.dumps(command) + "\n"
            "    environment:\n      - " + path + "\n"
            "    networks:\n      docker-network-1: {}\n"
            "networks:\n  docker-network-1:\n    name: bridge\n    external: true\n").encode()


class CoreJourneyBoundaryTests(unittest.TestCase):
    def test_private_source_substitution_is_literal_and_allowlisted(self) -> None:
        template = pathlib.Path(__file__).parent.parent / "fixtures/conformance/docker-application/core.compose.yaml"
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            os.chmod(root, 0o700)
            destination = root / "source.yaml"
            image = "registry.invalid/boxferry-core/busybox:1.2.3"
            contract.render_core_source(template, destination, image)
            self.assertEqual(destination.stat().st_mode & 0o777, 0o600)
            self.assertEqual(destination.read_bytes(), template.read_bytes().replace(
                b"BOXFERRY_CORE_IMAGE_LITERAL", image.encode()))
            source = destination.read_text()
            self.assertIn(f"image: {image}", source)
            self.assertNotIn("${", source)
            with self.assertRaises(contract.ContractError):
                contract.render_core_source(template, root / "bad.yaml", "busybox:latest")
            widened = root / "widened.yaml"
            widened.write_text(template.read_text() + "    privileged: true\n")
            with self.assertRaises(contract.ContractError):
                contract.render_core_source(widened, root / "unexpected.yaml", image)

    def test_source_binding_reuses_canonical_prerequisite_and_rejects_expectation_drift(self) -> None:
        template = contract.REPOSITORY_ROOT / "fixtures/conformance/docker-application/core.compose.yaml"
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            os.chmod(root, 0o700)
            with mock.patch.object(contract.core_artifact, "check_source",
                                   wraps=contract.core_artifact.check_source) as check:
                contract.render_core_source(template, root / "source.yaml",
                                            "registry.invalid/boxferry-core/busybox:independent-test")
            self.assertEqual(check.call_count, 1)
            self.assertEqual(check.call_args.args[0], template.read_bytes())
            fixture = root / "fixtures/conformance/docker-application"
            fixture.mkdir(parents=True)
            expectation = json.loads((template.parent / "core-expectations.json").read_bytes())
            expectation["source"]["sha256"] = "0" * 64
            (fixture / "core-expectations.json").write_text(json.dumps(expectation))
            with mock.patch.object(contract, "REPOSITORY_ROOT", root):
                with self.assertRaisesRegex(contract.ContractError, "authored source bytes differ"):
                    contract.render_core_source(template, root / "changed.yaml",
                                                "registry.invalid/boxferry-core/busybox:independent-test")
            self.assertFalse((root / "changed.yaml").exists())

    def test_candidate_receipt_binds_binary_revision_and_dirty_source(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            os.chmod(root, 0o700)
            lens = root / "lens"
            lens.mkdir()
            (root / "Cargo.lock").write_bytes(b"box lock")
            (lens / "Cargo.lock").write_bytes(b"lens lock")
            target = root / "target"
            target.mkdir()
            binary = target / "boxferry"
            binary.write_bytes(b"worktree-local candidate")
            os.chmod(binary, 0o700)
            receipt = root / "receipt.json"
            record = {"schema_version": 2, "boxferry_revision": "a" * 40,
                      "boxferry_source_sha256": "b" * 64,
                      "boxferry_lock_sha256": hashlib.sha256(b"box lock").hexdigest(),
                      "docker_lens_revision": "c" * 40,
                      "docker_lens_source_sha256": "d" * 64,
                      "docker_lens_lock_sha256": hashlib.sha256(b"lens lock").hexdigest(),
                      "build_command": contract.expected_build_command(lens),
                      "override_identity": {"kind": "cargo-crates-io-patch", "package": "docker-lens",
                                            "path": str(lens), "revision": "c" * 40,
                                            "source_sha256": "d" * 64},
                      "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
            receipt.write_text(json.dumps(record))
            os.chmod(receipt, 0o600)
            def git_value(checkout: pathlib.Path, *arguments: str) -> str:
                if arguments[0] == "status":
                    return ""
                return "c" * 40 if checkout == lens else "a" * 40
            def source_value(checkout: pathlib.Path) -> str:
                return "d" * 64 if checkout == lens else "b" * 64
            with mock.patch.object(contract, "git", side_effect=git_value), \
                 mock.patch.object(contract, "candidate_source_digest", side_effect=source_value):
                self.assertEqual(contract.verify_candidate(root, binary, receipt, lens, "c" * 40), record)
                snapshot = root / "boxferry-candidate"
                self.assertEqual(contract.capture_candidate(root, binary, receipt, lens, "c" * 40,
                                                            snapshot), record)
                self.assertEqual(snapshot.read_bytes(), binary.read_bytes())
                self.assertEqual(snapshot.stat().st_mode & 0o777, 0o500)
                binary.write_bytes(b"other binary")
                with self.assertRaises(contract.ContractError):
                    contract.verify_candidate(root, binary, receipt, lens, "c" * 40)
                binary.write_bytes(b"worktree-local candidate")
                with mock.patch.object(contract, "candidate_source_digest", return_value="e" * 64):
                    with self.assertRaises(contract.ContractError):
                        contract.verify_candidate(root, binary, receipt, lens, "c" * 40)
                outside = root / "outside"
                outside.write_bytes(binary.read_bytes())
                with self.assertRaises(contract.ContractError):
                    contract.verify_candidate(root, root / "target" / ".." / "outside",
                                              receipt, lens, "c" * 40)
                record["build_command"] = ["sh", "-c", "cargo build"]
                receipt.write_text(json.dumps(record))
                with self.assertRaises(contract.ContractError):
                    contract.verify_candidate(root, binary, receipt, lens, "c" * 40)
                record["build_command"] = contract.expected_build_command(lens)
                receipt.write_text(json.dumps(record))
                literal_volume_recipe = ["cargo", "build", "--locked", "--package", "boxferry", "--example",
                                         "docker-volume-fixture-rehearsal", "--no-default-features", "--features",
                                         "compose,docker", "--jobs", "2", "--config",
                                         f'patch.crates-io.docker-lens.path="{lens}"']
                with self.assertRaises(contract.ContractError):
                    contract.verify_candidate(root, binary, receipt, lens, "c" * 40, profile="volume-fixtures")
                record["build_command"] = literal_volume_recipe
                receipt.write_text(json.dumps(record))
                self.assertEqual(contract.verify_candidate(root, binary, receipt, lens, "c" * 40,
                                                          profile="volume-fixtures"), record)
                with self.assertRaises(contract.ContractError):
                    contract.verify_candidate(root, binary, receipt, lens, "c" * 40)
                with self.assertRaises(contract.ContractError):
                    contract.expected_build_command(lens, "arbitrary")
                record["build_command"] = contract.expected_build_command(lens)
                receipt.write_text(json.dumps(record))
                (lens / "Cargo.lock").write_bytes(b"changed lock")
                with self.assertRaises(contract.ContractError):
                    contract.verify_candidate(root, binary, receipt, lens, "c" * 40)

    def test_source_digest_changes_with_uncommitted_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "candidate.rs"
            source.write_bytes(b"before")
            inventory = subprocess.CompletedProcess([], 0, b"candidate.rs\0", b"")
            with mock.patch.object(contract, "git", return_value=str(root)), \
                 mock.patch.object(contract.subprocess, "run", return_value=inventory):
                before = contract.candidate_source_digest(root)
                source.write_bytes(b"after")
                self.assertNotEqual(before, contract.candidate_source_digest(root))

    def test_generated_compose_requires_exact_core_semantics(self) -> None:
        image = "registry.invalid/boxferry-core/busybox:1.37.0"
        document = authored_reacquired_compose(image)
        contract.core_generated_compose(document, image)
        for changed in (document.replace(b"docker-service-1:", b"other:"),
                        document.replace(b"name: bf-docker-core\n", b"name: other\n"),
                        document + b"name: second\n",
                        document.replace(b"    container_name: bf-docker-core\n", b""),
                        document.replace(b"container_name: bf-docker-core", b"container_name: other"),
                        document.replace(b"exec sleep 3600", b"exec sleep 9999"),
                        document.replace(image.encode(), b"busybox:latest"),
                        document.replace(contract.CORE_BUSYBOX_ENV[0].encode(), b"PATH=/bin"),
                        document.replace(b"    environment:\n", b""),
                        document.replace(b"    networks:\n      docker-network-1: {}\n", b""),
                        document.replace(b"      docker-network-1: {}", b"      other: {}"),
                        document.replace(b"    name: bridge", b"    name: other"),
                        document.replace(b"    external: true", b"    external: false"),
                        document.replace(b"    external: true", b"    external: \"true\""),
                        document + b"  extra:\n    image: busybox:latest\n",
                        document + b"    privileged: true\n",
                        document + b"    ports:\n      - 8080:80\n",
                        document + b"    volumes:\n      - /host:/data\n",
                        document + b"    environment:\n      - TOKEN=secret\n",
                        document + b"    command:\n      - true\n",
                        document + b"    image: busybox:latest\n",
                        document + b"networks:\n  extra: {}\n"):
            with self.subTest(changed=changed[-35:]), self.assertRaises(contract.ContractError):
                contract.core_generated_compose(changed, image)

    def test_closed_reports_and_native_inspect(self) -> None:
        image = "registry.invalid/boxferry-core/busybox:1.2.3"
        container_id = "f" * 64
        profile = {"kind": "target", "build": {"kind": "upstream"},
                   "engine_release": "29.8.1", "advertised_api_version": "1.56",
                   "acquisition_api_version": "1.49", "rendering_api_version": "1.56",
                   "daemon_mode": "rootful", "evidence_sha256": "b" * 64}
        profiles = {"upstream-rootful": profile}
        report = {"schema_version": 1, "status": "success", "exit_category": "success",
                  "failed_stage": None, "failure_summary": None, "fix_first": None,
                  "source_type": "docker", "target_type": "compose",
                  "application": "bf-docker-core", "truncations": [],
                  "invocation": {"command_kind": "convert"},
                  "primary_diagnostic_code": None,
                  "choices": [{"name": key, "value": value} for key, value in {
                      "loss_policy": "partial", "environment_values": "include",
                      "docker_acquisition": "read-only-local-unix", "docker_import_policy": "portable",
                      "promote_docker_protected_environment_values": "true",
                      "promote_docker_same_host_bind_mounts": "false"}.items()],
                  "fidelity": {"exact": 3, "approximate": 1, "unsupported": 0,
                               "invalid": 0, "other": 0},
                  "diagnostics": [{"code": "BFD0004", "severity": "warning",
                                   "fields": [{"name": "subject", "value": "services[0].runtime_name"}]}],
                  "requested_versions": {"minimum": "rolling", "maximum": "rolling"},
                  "resolved_versions": {"minimum": "rolling", "maximum": "rolling"},
                  "output_artifacts": [{"name": "compose.yaml", "size": 100}]}
        with self.assertRaisesRegex(contract.ContractError, "pending independent native review"):
            contract.core_report(json.dumps(report).encode(), "docker", "compose",
                                 "upstream-rootful", profiles)
        plan = copy.deepcopy(report)
        plan["source_type"], plan["target_type"] = "compose", "docker"
        plan["choices"] = [{"name": key, "value": value} for key, value in {
            "loss_policy": "exact", "environment_values": "withhold",
            "docker_target": "upstream-29.8.1-rootful", "docker_target_build": "upstream",
            "docker_target_engine_release": "29.8.1", "docker_target_advertised_api": "1.56",
            "docker_target_acquisition_api": "1.49", "docker_target_rendering_api": "1.56",
            "docker_target_mode": "rootful", "docker_target_evidence_sha256": "b" * 64}.items()]
        plan["fidelity"]["approximate"] = 0
        plan["diagnostics"] = []
        plan["requested_versions"] = {"minimum": "upstream-29.8.1-rootful",
                                      "maximum": "upstream-29.8.1-rootful"}
        plan["resolved_versions"] = {"minimum": "29.8.1", "maximum": "29.8.1"}
        plan["output_artifacts"] = [{"name": "docker-plan.json", "size": 100}]
        contract.core_report(json.dumps(plan).encode(), "compose", "docker",
                             "upstream-rootful", profiles)
        plan["choices"][-1]["value"] = "f" * 64
        with self.assertRaises(contract.ContractError):
            contract.core_report(json.dumps(plan).encode(), "compose", "docker",
                                 "upstream-rootful", profiles)
        report["truncations"] = [{"field": "events", "original": 2, "retained": 1}]
        with self.assertRaises(contract.ContractError):
            contract.core_report(json.dumps(report).encode(), "docker", "compose",
                                 "upstream-rootful", profiles)
        report["truncations"] = []
        report["status"] = "partial"
        with self.assertRaises(contract.ContractError):
            contract.core_report(json.dumps(report).encode(), "docker", "compose",
                                 "upstream-rootful", profiles)
        report["status"] = "success"
        report["choices"][1]["value"] = "withhold"
        with self.assertRaises(contract.ContractError):
            contract.core_report(json.dumps(report).encode(), "docker", "compose",
                                 "upstream-rootful", profiles)
        report["choices"][1]["value"] = "include"
        report["choices"][4]["value"] = "false"
        with self.assertRaises(contract.ContractError):
            contract.core_report(json.dumps(report).encode(), "docker", "compose",
                                 "upstream-rootful", profiles)
        report["choices"][4]["value"] = "true"
        report["events"] = [contract.CORE_BUSYBOX_ENV[0]]
        with self.assertRaises(contract.ContractError):
            contract.core_report(json.dumps(report).encode(), "docker", "compose",
                                 "upstream-rootful", profiles)

    def test_direct_native_container_and_bridge_contract(self) -> None:
        container_id, network_id, endpoint_id = "f" * 64, "a" * 64, "b" * 64
        image = "registry.invalid/boxferry-core/busybox:1.37.0"
        inspected = {
            "Id": container_id, "Name": "/bf-docker-core",
            "Config": {"Image": image, "Cmd": contract.CORE_COMMAND,
                       "Env": contract.CORE_BUSYBOX_ENV,
                       "ExposedPorts": None, "Volumes": None},
            "HostConfig": {"NetworkMode": "default", "Privileged": False,
                           "ReadonlyRootfs": False, "Binds": None, "Mounts": None,
                           "VolumesFrom": None, "Devices": [], "DeviceRequests": None,
                           "SecurityOpt": None, "CapAdd": None, "CapDrop": None,
                           "Tmpfs": None, "PortBindings": {}},
            "Mounts": [],
            "NetworkSettings": {"Ports": {}, "Networks": {
                "bridge": {"NetworkID": network_id, "EndpointID": endpoint_id}}},
        }
        network = {"Id": network_id, "Name": "bridge", "Driver": "bridge",
                   "Internal": False,
                   "Containers": {container_id: {"Name": "bf-docker-core",
                                                 "EndpointID": endpoint_id}}}
        self.assertEqual(contract.core_inspect(json.dumps(inspected).encode(), container_id,
                                               image, contract.CORE_BUSYBOX_FIXTURE),
                         (network_id, endpoint_id))
        contract.core_network_inspect(json.dumps(network).encode(), network_id,
                                      container_id, endpoint_id)
        wrong_network = copy.deepcopy(inspected)
        wrong_network["NetworkSettings"]["Networks"]["bridge"]["NetworkID"] = "c" * 64
        selected_id, selected_endpoint = contract.core_inspect(
            json.dumps(wrong_network).encode(), container_id, image, contract.CORE_BUSYBOX_FIXTURE)
        with self.assertRaises(contract.ContractError):
            contract.core_network_inspect(json.dumps(network).encode(), selected_id,
                                          container_id, selected_endpoint)

        changes = {
            "command": lambda value: value["Config"].update(Cmd=["sh", "-c", "other"]),
            "unexpected environment": lambda value: value["Config"].update(
                Env=contract.CORE_BUSYBOX_ENV + ["UNREVIEWED=1"]),
            "changed inherited path": lambda value: value["Config"].update(
                Env=["PATH=/usr/bin:/bin"]),
            "missing environment": lambda value: value["Config"].pop("Env"),
            "extra network": lambda value: value["NetworkSettings"]["Networks"].update(
                other={"NetworkID": "c" * 64, "EndpointID": "d" * 64}),
            "missing network": lambda value: value["NetworkSettings"].update(Networks={}),
            "wrong network id": lambda value: value["NetworkSettings"]["Networks"]["bridge"].update(
                NetworkID="z" * 64),
            "missing network id": lambda value: value["NetworkSettings"]["Networks"]["bridge"].pop(
                "NetworkID"),
            "wrong endpoint id": lambda value: value["NetworkSettings"]["Networks"]["bridge"].update(
                EndpointID="z" * 64),
            "missing endpoint id": lambda value: value["NetworkSettings"]["Networks"]["bridge"].pop(
                "EndpointID"),
            "unreviewed alias": lambda value: value["NetworkSettings"]["Networks"]["bridge"].update(
                Aliases=["bf-docker-core"]),
            "requested static address": lambda value: value["NetworkSettings"]["Networks"]["bridge"].update(
                IPAMConfig={"IPv4Address": "172.17.0.2"}),
            "linked peer": lambda value: value["NetworkSettings"]["Networks"]["bridge"].update(
                Links=["peer"]),
            "network none": lambda value: value["HostConfig"].update(NetworkMode="none"),
            "missing network mode": lambda value: value["HostConfig"].pop("NetworkMode"),
            "published port": lambda value: value["HostConfig"].update(
                PortBindings={"80/tcp": [{"HostPort": "8080"}]}),
            "exposed port": lambda value: value["Config"].update(ExposedPorts={"80/tcp": {}}),
            "mount": lambda value: value.update(Mounts=[{"Destination": "/data"}]),
            "bind": lambda value: value["HostConfig"].update(Binds=["/tmp:/data"]),
            "privileged": lambda value: value["HostConfig"].update(Privileged=True),
            "security option": lambda value: value["HostConfig"].update(SecurityOpt=["seccomp=unconfined"]),
            "capability": lambda value: value["HostConfig"].update(CapAdd=["SYS_ADMIN"]),
        }
        for label, mutate in changes.items():
            with self.subTest(label=label), self.assertRaises(contract.ContractError):
                changed = copy.deepcopy(inspected)
                mutate(changed)
                contract.core_inspect(json.dumps(changed).encode(), container_id,
                                      image, contract.CORE_BUSYBOX_FIXTURE)
        with self.assertRaises(contract.ContractError):
            contract.core_inspect(json.dumps(inspected).encode(), container_id,
                                  image, "docker.io/library/busybox:1.38.0@sha256:" + "c" * 64)
        with self.assertRaises(contract.ContractError):
            contract.core_inspect(json.dumps(inspected).encode(), container_id,
                                  "registry.invalid/boxferry-core/busybox:1.38.0",
                                  contract.CORE_BUSYBOX_FIXTURE)

        network_changes = {
            "wrong id": lambda value: value.update(Id="c" * 64),
            "driver": lambda value: value.update(Driver="overlay"),
            "missing driver": lambda value: value.pop("Driver"),
            "internal": lambda value: value.update(Internal=True),
            "missing internal": lambda value: value.pop("Internal"),
            "wrong name": lambda value: value.update(Name="other"),
            "missing member": lambda value: value.update(Containers={}),
            "extra member": lambda value: value["Containers"].update(
                {"c" * 64: {"Name": "other", "EndpointID": "d" * 64}}),
            "wrong member name": lambda value: value["Containers"][container_id].update(
                Name="other"),
            "wrong endpoint": lambda value: value["Containers"][container_id].update(
                EndpointID="d" * 64),
        }
        for label, mutate in network_changes.items():
            with self.subTest(label=label), self.assertRaises(contract.ContractError):
                changed = copy.deepcopy(network)
                mutate(changed)
                contract.core_network_inspect(json.dumps(changed).encode(), network_id,
                                              container_id, endpoint_id)

        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            os.chmod(root, 0o700)
            container_file = root / "native-inspect.json"
            network_file = root / "native-network-inspect.json"
            for path, value in ((container_file, inspected), (network_file, network)):
                path.write_text(json.dumps(value))
                os.chmod(path, 0o600)
            checked = subprocess.run(
                [sys.executable, str(SOURCE), "check-core-output", "--kind", "inspect",
                 "--file", str(container_file), "--container-id", container_id,
                 "--image", image, "--fixture-image", contract.CORE_BUSYBOX_FIXTURE],
                capture_output=True, text=True, check=False)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            self.assertEqual(checked.stdout.strip(), network_id)
            command = [sys.executable, str(SOURCE), "check-core-output", "--kind", "network-inspect",
                       "--file", str(network_file), "--container-file", str(container_file),
                       "--container-id", container_id, "--network-id", network_id,
                       "--image", image, "--fixture-image", contract.CORE_BUSYBOX_FIXTURE]
            checked = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            command[command.index(network_id)] = "c" * 64
            checked = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertNotEqual(checked.returncode, 0)

            output = root / "generated"
            output.mkdir(mode=0o700)
            compose_file = output / "compose.yaml"
            compose_file.write_bytes(authored_reacquired_compose(image))
            os.chmod(compose_file, 0o600)
            compose_command = [
                sys.executable, str(SOURCE), "check-core-output", "--kind", "compose",
                "--file", str(compose_file), "--container-file", str(container_file),
                "--network-file", str(network_file), "--container-id", container_id,
                "--network-id", network_id, "--image", image,
                "--fixture-image", contract.CORE_BUSYBOX_FIXTURE,
            ]
            checked = subprocess.run(compose_command, capture_output=True, text=True, check=False)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            for label, changed in (
                ("missing container evidence", "--container-file"),
                ("missing network evidence", "--network-file"),
                ("wrong network selector", "--network-id"),
            ):
                with self.subTest(label=label):
                    command = compose_command.copy()
                    index = command.index(changed)
                    if changed == "--network-id":
                        command[index + 1] = "c" * 64
                    else:
                        del command[index:index + 2]
                    checked = subprocess.run(command, capture_output=True, text=True, check=False)
                    self.assertNotEqual(checked.returncode, 0)
            wrong_network_file = root / "wrong-network-inspect.json"
            wrong_network = copy.deepcopy(network)
            wrong_network["Containers"][container_id]["EndpointID"] = "d" * 64
            wrong_network_file.write_text(json.dumps(wrong_network))
            os.chmod(wrong_network_file, 0o600)
            changed = compose_command.copy()
            changed[changed.index(str(network_file))] = str(wrong_network_file)
            checked = subprocess.run(changed, capture_output=True, text=True, check=False)
            self.assertNotEqual(checked.returncode, 0)

            socket_path = str(root / "socket/docker.sock")
            privacy_args = ["--container-file", str(container_file),
                            "--network-file", str(network_file),
                            "--container-id", container_id, "--network-id", network_id,
                            "--image", image, "--fixture-image", contract.CORE_BUSYBOX_FIXTURE,
                            "--socket-path", socket_path]
            console_file = root / "compose-console.txt"
            console_file.write_bytes(b"")
            os.chmod(console_file, 0o600)
            console_command = [sys.executable, str(SOURCE), "check-core-output",
                               "--kind", "compose-console", "--file", str(console_file),
                               *privacy_args]
            checked = subprocess.run(console_command, capture_output=True, text=True, check=False)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            for flag, replacement in (("--network-id", "c" * 64),
                                      ("--socket-path", str(root / "other/docker.sock"))):
                changed = console_command.copy()
                changed[changed.index(flag) + 1] = replacement
                checked = subprocess.run(changed, capture_output=True, text=True, check=False)
                self.assertNotEqual(checked.returncode, 0)
            report_file = root / "compose-report.json"
            report_command = [sys.executable, str(SOURCE), "check-core-output",
                              "--kind", "compose-report", "--file", str(report_file),
                              *privacy_args]
            protected = (contract.CORE_BUSYBOX_ENV[0],
                         contract.CORE_BUSYBOX_ENV[0].partition("=")[2],
                         container_id, network_id, endpoint_id, socket_path)
            for kind, path, command in (("console", console_file, console_command),
                                        ("report", report_file, report_command)):
                for label, value in enumerate(protected):
                    with self.subTest(kind=kind, protected=label):
                        path.write_bytes((json.dumps({"leak": value}).encode()
                                          if kind == "report" else value.encode()))
                        os.chmod(path, 0o600)
                        checked = subprocess.run(command, capture_output=True, text=True, check=False)
                        self.assertNotEqual(checked.returncode, 0)
                        self.assertIn("protected native value", checked.stderr)
                        self.assertNotIn(value, checked.stderr)
            for value in (protected[1], container_id, socket_path):
                escaped = json.dumps({"leak": value})
                escaped = escaped.replace(value[0], f"\\u{ord(value[0]):04x}", 1)
                escaped = escaped.replace("/", "\\/")
                report_file.write_text(escaped)
                checked = subprocess.run(report_command, capture_output=True, text=True, check=False)
                self.assertNotEqual(checked.returncode, 0)
                self.assertIn("protected native value", checked.stderr)
            for label, value in enumerate(protected):
                literal = json.dumps(value).replace(value[0], f"\\u{ord(value[0]):04x}", 1)
                literal = literal.replace("/", "\\/")
                self.assertEqual(json.loads(literal), value)
                with self.subTest(prefixed_console=label):
                    console_file.write_text("diagnostic field: " + literal + "\n")
                    checked = subprocess.run(console_command, capture_output=True, text=True, check=False)
                    self.assertNotEqual(checked.returncode, 0)
                    self.assertIn("protected native value", checked.stderr)
                    self.assertNotIn(value, checked.stderr)
                    console_file.write_text("diagnostic field: " + literal[:-1])
                    checked = subprocess.run(console_command, capture_output=True, text=True, check=False)
                    self.assertNotEqual(checked.returncode, 0)
                    self.assertIn("malformed quoted fragment", checked.stderr)
            console_file.write_bytes(b'benign diagnostic: "safe literal"\n')
            checked = subprocess.run(console_command, capture_output=True, text=True, check=False)
            self.assertEqual(checked.returncode, 0, checked.stderr)

    def test_private_output_rejects_extra_files_and_open_modes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            os.chmod(root, 0o700)
            plan = root / "docker-plan.json"
            plan.write_bytes(b"{}")
            os.chmod(plan, 0o600)
            self.assertEqual(contract.core_output_file(plan, 16_384, {"docker-plan.json"}), b"{}")
            extra = root / "unexpected.txt"
            extra.write_text("extra")
            with self.assertRaises(contract.ContractError):
                contract.core_output_file(plan, 16_384, {"docker-plan.json"})
            extra.unlink()
            os.chmod(plan, 0o644)
            with self.assertRaises(contract.ContractError):
                contract.core_output_file(plan, 16_384, {"docker-plan.json"})

    def test_storage_sample_retries_only_run_owned_descendant_enoent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            volume, run, graph = (root / name for name in ("volume", "run", "graph"))
            for path in (volume, run, graph):
                path.mkdir()
            baseline = 9 * 1024 * 1024
            def df(_command: list[str], _temporary_root: pathlib.Path) -> tuple[int, bytes, bytes]:
                return 0, f"Filesystem 1024-blocks Used Available Capacity Mounted\nfs 100 1 {baseline} 1% /\n".encode(), b""
            responses = [df(None, run), df(None, run),
                         (1, b"", f"du: cannot access '{volume}/gone': No such file or directory\n".encode()),
                         df(None, run), df(None, run),
                         (0, f"10\t{volume}\n".encode(), b""),
                         (0, f"20\t{run}\n".encode(), b"")]
            with mock.patch.object(contract, "bounded_measurement", side_effect=responses) as measurement:
                sample = contract.sample_owned_storage(volume, run, graph, baseline)
            self.assertEqual((sample["used"], sample["temporary_used"]), (10, 20))
            self.assertEqual(measurement.call_count, 7)
            self.assertFalse(contract.transient_owned_du_error(volume,
                f"du: cannot access '{run}/gone': No such file or directory\n".encode()))
            self.assertFalse(contract.transient_owned_du_error(volume,
                f"du: cannot access '{volume}/gone': Permission denied\n".encode()))

    def test_storage_sample_fails_persistent_churn_and_low_disk(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            volume, run, graph = (root / name for name in ("volume", "run", "graph"))
            for path in (volume, run, graph):
                path.mkdir()
            baseline = 9 * 1024 * 1024
            sufficient = (0, f"fs 100 1 {baseline} 1% /\n".encode(), b"")
            missing = (1, b"", f"du: cannot access '{volume}/gone': No such file or directory\n".encode())
            with mock.patch.object(contract, "bounded_measurement",
                                   side_effect=[sufficient, sufficient, missing] * 3), \
                 mock.patch.object(contract.time, "sleep"):
                with self.assertRaises(contract.ContractError):
                    contract.sample_owned_storage(volume, run, graph, baseline)
            low = (0, b"fs 100 1 100 1% /\n", b"")
            with mock.patch.object(contract, "bounded_measurement", return_value=low):
                with self.assertRaises(contract.ContractError):
                    contract.sample_owned_storage(volume, run, graph, baseline)


class DependencyScheduleTests(unittest.TestCase):
    """Independent expected edges; no fixture or native request is the oracle."""

    def setUp(self) -> None:
        self.profile = {
            "kind": "target", "build": {"kind": "upstream"},
            "engine_release": "29.8.1", "advertised_api_version": "1.56",
            "acquisition_api_version": "1.49", "rendering_api_version": "1.56",
            "daemon_mode": "rootless", "evidence_sha256": "b" * 64,
        }
        self.services = {"frontend": "web-live", "database": "db-live", "migration": "init-live"}
        self.expected = [("frontend", "database", "healthy", True, False),
                         ("frontend", "migration", "completed_successfully", True, False),
                         ("migration", "database", "started", True, False)]
        self.plan = {
            "schema_version": 1, "context": copy.deepcopy(self.profile), "prerequisites": [],
            "requests": [{"method": "POST", "path": f"/v1.56/containers/create?name={name}",
                          "body": {"Image": "registry.invalid/test:fixture"}}
                         for name in ("web-live", "db-live", "init-live")],
        }
        self.sidecar = {
            "schema_version": 1, "kind": "boxferry-docker-dependency-decisions",
            "docker_plan_sha256": hashlib.sha256(self.plan_bytes()).hexdigest(),
            "native_execution": False,
            "decisions": [self.decision(*edge) for edge in self.expected],
        }

    def plan_bytes(self) -> bytes:
        return json.dumps(self.plan, separators=(",", ":")).encode()

    def decision(self, service: str, dependency: str, condition: str,
                 required: bool, restart: bool) -> dict[str, object]:
        return {
            "service": service, "service_runtime_name": self.services[service],
            "dependency": dependency, "dependency_runtime_name": self.services[dependency],
            "condition": condition, "condition_explicit": True,
            "required": required, "required_explicit": True,
            "restart": restart, "restart_explicit": True,
            "fidelity": "approximate", "native_engine_field": False,
            "provenance": {key: ["source_document"] for key in ("reference", "condition", "required", "restart")},
        }

    def check(self) -> tuple[list[list[str]], list[dict[str, object]]]:
        return contract.dependency_schedule(
            self.plan_bytes(), json.dumps(self.sidecar).encode(), profile=self.profile,
            services=self.services, expected_edges=self.expected,
        )

    def test_actual_sidecar_drives_layers_with_independent_identity_and_edges(self) -> None:
        layers, decisions = self.check()
        self.assertEqual(layers, [["database"], ["migration"], ["frontend"]])
        self.assertEqual(decisions, self.sidecar["decisions"])
        self.assertEqual(decisions[0]["service_runtime_name"], "web-live")

    def test_reordered_decisions_keep_same_schedule(self) -> None:
        self.sidecar["decisions"].reverse()
        self.assertEqual(self.check()[0], [["database"], ["migration"], ["frontend"]])

    def test_swapped_and_whitespace_modified_plan_pairs_fail_binding(self) -> None:
        for plan_bytes in (self.plan_bytes() + b"\n", self.plan_bytes().replace(b"web-live", b"other-web")):
            with self.subTest(plan=plan_bytes), self.assertRaises(contract.ContractError):
                contract.dependency_schedule(plan_bytes, json.dumps(self.sidecar).encode(),
                                             profile=self.profile, services=self.services, expected_edges=self.expected)

    def test_missing_sidecar_cannot_silently_remove_dependencies(self) -> None:
        with self.assertRaisesRegex(contract.ContractError, "missing"):
            contract.dependency_schedule(self.plan_bytes(), None, profile=self.profile,
                                         services=self.services, expected_edges=self.expected)
        self.assertEqual(contract.dependency_schedule(self.plan_bytes(), None, profile=self.profile,
                                                     services=self.services, expected_edges=[]),
                         ([["database", "frontend", "migration"]], []))

    def test_hash_alone_cannot_authorize_a_different_profile(self) -> None:
        self.plan["context"]["daemon_mode"] = "rootful"
        self.sidecar["docker_plan_sha256"] = hashlib.sha256(self.plan_bytes()).hexdigest()
        with self.assertRaisesRegex(contract.ContractError, "reviewed profile"):
            self.check()

    def test_unknown_duplicate_and_self_edges_fail(self) -> None:
        for mutation in ("unknown", "self", "duplicate"):
            with self.subTest(mutation=mutation):
                original = copy.deepcopy(self.sidecar)
                if mutation == "unknown":
                    self.sidecar["decisions"][0]["dependency"] = "absent"
                elif mutation == "self":
                    self.sidecar["decisions"][0]["dependency"] = "frontend"
                else:
                    self.sidecar["decisions"].append(copy.deepcopy(self.sidecar["decisions"][0]))
                with self.assertRaises(contract.ContractError):
                    self.check()
                self.sidecar = original

    def test_native_target_mismatch_rejected_despite_correct_digest(self) -> None:
        self.plan["requests"][0]["path"] = "/v1.56/containers/create?name=not-the-web"
        self.sidecar["docker_plan_sha256"] = hashlib.sha256(self.plan_bytes()).hexdigest()
        with self.assertRaisesRegex(contract.ContractError, "service inventory"):
            self.check()

    def test_duplicate_and_unexpected_api_create_requests_rejected(self) -> None:
        for path in ("/v1.56/containers/create?name=db-live", "/v1.41/containers/create?name=web-live"):
            self.plan["requests"][0]["path"] = path
            self.sidecar["docker_plan_sha256"] = hashlib.sha256(self.plan_bytes()).hexdigest()
            with self.subTest(path=path), self.assertRaises(contract.ContractError):
                self.check()

    def test_sidecar_runtime_name_cannot_diverge_from_native_requests(self) -> None:
        self.sidecar["decisions"][0]["service_runtime_name"] = "another-web"
        with self.assertRaisesRegex(contract.ContractError, "runtime reference"):
            self.check()

    def test_digest_cannot_authorize_unexpected_native_operations_or_prerequisites(self) -> None:
        for path in ("/v1.56/containers/web-live/start", "/v1.56/containers/web-live/rename",
                     "/v1.56/networks/create", "/v1.56/volumes/create"):
            original = copy.deepcopy(self.plan)
            self.plan["requests"].append({"method": "POST", "path": path, "body": {}})
            self.sidecar["docker_plan_sha256"] = hashlib.sha256(self.plan_bytes()).hexdigest()
            with self.subTest(path=path), self.assertRaisesRegex(contract.ContractError, "create path"):
                self.check()
            self.plan = original
        self.plan["prerequisites"] = [{"kind": "volume", "identity": "external-data"}]
        self.sidecar["docker_plan_sha256"] = hashlib.sha256(self.plan_bytes()).hexdigest()
        with self.assertRaisesRegex(contract.ContractError, "external prerequisites"):
            self.check()

    def test_nonfinite_json_is_not_accepted_even_with_a_correct_digest(self) -> None:
        for value in (float("nan"), float("inf"), float("-inf")):
            self.plan["requests"][0]["body"]["unreviewed"] = value
            self.sidecar["docker_plan_sha256"] = hashlib.sha256(self.plan_bytes()).hexdigest()
            with self.subTest(value=value), self.assertRaisesRegex(contract.ContractError, "non-JSON"):
                self.check()

    def test_cycle_fails_even_if_expected_edges_also_contain_it(self) -> None:
        edge = ("database", "frontend", "started", True, False)
        self.expected.append(edge)
        self.sidecar["decisions"].append(self.decision(*edge))
        with self.assertRaisesRegex(contract.ContractError, "cycle"):
            self.check()

    def test_exact_independent_options_are_required(self) -> None:
        for field, value in (("condition", "started"), ("required", False), ("restart", True)):
            original = copy.deepcopy(self.sidecar)
            self.sidecar["decisions"][0][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(contract.ContractError, "independent application"):
                self.check()
            self.sidecar = original

    def test_false_required_and_restart_are_retained_but_not_execution_evidence(self) -> None:
        self.expected[0] = ("frontend", "database", "healthy", False, True)
        self.sidecar["decisions"][0] = self.decision(*self.expected[0])
        decisions = self.check()[1]
        self.assertFalse(decisions[0]["required"])
        self.assertTrue(decisions[0]["restart"])

    def test_implicit_option_values_must_match_defaults(self) -> None:
        for field in ("condition_explicit", "required_explicit", "restart_explicit"):
            original = copy.deepcopy(self.sidecar)
            first = self.sidecar["decisions"][0]
            first[field] = False
            if field == "required_explicit":
                first["required"] = False
            if field == "restart_explicit":
                first["restart"] = True
            with self.subTest(field=field), self.assertRaisesRegex(contract.ContractError, "documented default"):
                self.check()
            self.sidecar = original

    def test_closed_schema_rejects_raw_provenance_and_numeric_flags(self) -> None:
        for field, value in (("provenance", {"path": "/private/source.yaml"}),
                             ("required", 1), ("native_engine_field", True), ("fidelity", "exact")):
            original = copy.deepcopy(self.sidecar)
            self.sidecar["decisions"][0][field] = value
            with self.subTest(field=field), self.assertRaises(contract.ContractError):
                self.check()
            self.sidecar = original

    def test_duplicate_json_and_oversized_input_fail_closed(self) -> None:
        for sidecar in (json.dumps(self.sidecar).replace('"schema_version": 1',
                         '"schema_version": 1, "schema_version": 1').encode(), b" " * 1_048_577):
            with self.assertRaises(contract.ContractError):
                contract.dependency_schedule(self.plan_bytes(), sidecar, profile=self.profile,
                                             services=self.services, expected_edges=self.expected)


class CatalogueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="boxferry-docker-contract-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        self.revision = "a" * 40
        self.assignments = {
            key: f"ghcr.io/example/{key.lower()}:v1@sha256:{index:064x}"
            for index, key in enumerate(contract.IMAGE_KEYS.values(), start=1)
        }
        self.script = self.root / "scripts/native-conformance.sh"
        self.write_assignments()

    def write_assignments(self, extra: str = "") -> str:
        self.script.write_text(
            "\n".join(f"{key}='{value}'" for key, value in self.assignments.items()) + "\n" + extra,
            encoding="utf-8",
        )
        return hashlib.sha256(self.script.read_bytes()).hexdigest()

    def check(self, digest: str) -> dict[str, str]:
        def fake_git(_root: pathlib.Path, *args: str) -> str:
            if args == ("rev-parse", "--show-toplevel"):
                return str(self.root)
            if args == ("rev-parse", "HEAD"):
                return self.revision
            if args == ("status", "--porcelain", "--untracked-files=all"):
                return ""
            raise AssertionError(args)

        with mock.patch.object(contract, "git", side_effect=fake_git):
            return contract.canonical_images(str(self.root), self.revision, digest)

    def test_exact_five_assignments_are_data_not_executed(self) -> None:
        digest = self.write_assignments("echo this-command-must-not-run\n")
        selected = self.check(digest)
        self.assertEqual(set(selected), set(contract.IMAGE_KEYS))
        self.assertEqual(selected["fixture"], self.assignments["FIXTURE_IMAGE"])

    def test_untrusted_producer_checker_is_never_executed(self) -> None:
        marker = self.root / "producer-executed"
        checker = self.root / "scripts/native-storage-options.py"
        checker.write_text(f"import pathlib\npathlib.Path({str(marker)!r}).touch()\n", encoding="utf-8")
        self.check(self.write_assignments())
        self.assertFalse(marker.exists())
        runner = (pathlib.Path(__file__).parent / "docker-application-conformance.sh").read_text(encoding="utf-8")
        self.assertNotIn("native-storage-options.py", runner)

    def test_read_only_git_queries_disable_checkout_hooks_and_fsmonitor(self) -> None:
        completed = subprocess.CompletedProcess(["git"], 0, stdout="clean\n", stderr="")
        with mock.patch.object(contract.subprocess, "run", return_value=completed) as run:
            self.assertEqual(contract.git(self.root, "status", "--porcelain"), "clean")
        command = run.call_args.args[0]
        self.assertIn("core.fsmonitor=false", command)
        self.assertIn(f"core.hooksPath={os.devnull}", command)
        self.assertIn("protocol.file.allow=never", command)
        self.assertEqual(run.call_args.kwargs["env"]["GIT_CONFIG_GLOBAL"], os.devnull)

    def test_script_digest_drift_fails(self) -> None:
        with self.assertRaisesRegex(contract.ContractError, "digest differs"):
            self.check("0" * 64)

    def test_missing_assignment_fails(self) -> None:
        self.assignments.pop("FIXTURE_IMAGE")
        with self.assertRaisesRegex(contract.ContractError, "missing"):
            self.check(self.write_assignments())

    def test_repeated_assignment_fails(self) -> None:
        repeated = f"FIXTURE_IMAGE='{self.assignments['FIXTURE_IMAGE']}'\n"
        with self.assertRaisesRegex(contract.ContractError, "duplicate"):
            self.check(self.write_assignments(repeated))

    def test_mutable_reference_fails(self) -> None:
        self.assignments["FIXTURE_IMAGE"] = "docker.io/library/busybox:latest"
        with self.assertRaisesRegex(contract.ContractError, "immutable"):
            self.check(self.write_assignments())

    def test_symlink_script_fails(self) -> None:
        original = self.root / "original.sh"
        self.script.rename(original)
        self.script.symlink_to(original)
        with self.assertRaisesRegex(contract.ContractError, "regular file"):
            self.check(hashlib.sha256(original.read_bytes()).hexdigest())

    def test_dirty_checkout_fails(self) -> None:
        with mock.patch.object(contract, "git", side_effect=[str(self.root), self.revision, " M script"]):
            with self.assertRaisesRegex(contract.ContractError, "dirty"):
                contract.canonical_images(str(self.root), self.revision, self.write_assignments())


class ProfileTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="boxferry-docker-profiles-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        (self.root / "src").mkdir()
        self.records = self.root / "docs/evidence/reviewed/sha256"
        self.records.mkdir(parents=True)
        self.paths = []
        self.entries = []
        for symbol, lane in (("Debian11Rootful", "debian11-rootful"),
                             ("Debian11Rootless", "debian11-rootless"),
                             ("UpstreamRootful", "upstream-rootful"),
                             ("UpstreamRootless", "upstream-rootless")):
            debian = lane.startswith("debian11-")
            record = {"schema_version": 1, "lane": lane, "identity": {
                "build": {"kind": "debian-package", "distribution": "debian11",
                          "package_name": "docker.io", "package_revision": "20.10.5+dfsg1-1+deb11u2"}
                         if debian else {"kind": "upstream"},
                "engine_release": "20.10.5+dfsg1" if debian else "29.8.1",
                "advertised_api": "1.41" if debian else "1.56",
                "acquisition_api": "1.41" if debian else "1.49",
                "rendering_api": "1.41" if debian else "1.56",
                "mode": lane.rsplit("-", 1)[1],
            }}
            raw = json.dumps(record).encode()
            digest = hashlib.sha256(raw).hexdigest()
            path = self.records / f"{digest}.json"
            path.write_bytes(raw)
            self.paths.append(path)
            self.entries.append(f'(NativeEvidenceLane::{symbol}, "{digest}", '
                                f'include_str!("../docs/evidence/reviewed/sha256/{digest}.json"),),')
        self.write_catalogue(self.entries)

    def write_catalogue(self, entries: list[str]) -> None:
        (self.root / "src/reviewed_catalog.rs").write_text(
            "const RECORDS: [(NativeEvidenceLane, &str, &str); 4] = [\n"
            + "\n".join(entries) + "\n];\n", encoding="utf-8",
        )

    def test_four_exact_digest_bound_profiles(self) -> None:
        profiles = contract.reviewed_profiles(str(self.root))
        self.assertEqual(set(profiles), set(contract.IMAGE_KEYS) - {"fixture"})
        self.assertEqual(profiles["debian11-rootless"]["build"],
                         {"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u2"})
        self.assertEqual(profiles["upstream-rootful"]["acquisition_api_version"], "1.49")
        self.assertEqual(profiles["upstream-rootful"]["rendering_api_version"], "1.56")
        self.assertEqual(profiles["debian11-rootful"]["evidence_sha256"], self.paths[0].stem)

    def test_tampered_record_fails_digest(self) -> None:
        self.paths[0].write_bytes(self.paths[0].read_bytes() + b" ")
        with self.assertRaisesRegex(contract.ContractError, "digest differs"):
            contract.reviewed_profiles(str(self.root))

    def test_missing_duplicate_and_commented_entries_fail_closed(self) -> None:
        for entries in (self.entries[:-1], self.entries + [self.entries[0]],
                        ["// " + self.entries[0], *self.entries[1:]]):
            with self.subTest(entries=entries):
                self.write_catalogue(entries)
                with self.assertRaises(contract.ContractError):
                    contract.reviewed_profiles(str(self.root))

    def test_symlink_record_rejected(self) -> None:
        original = self.root / "original.json"
        self.paths[0].rename(original)
        self.paths[0].symlink_to(original)
        with self.assertRaises(OSError):
            contract.reviewed_profiles(str(self.root))


class DaemonModeTests(unittest.TestCase):
    def test_both_modes_require_exactly_one_process_and_matching_marker(self) -> None:
        for family in ("debian11", "upstream"):
            contract.verify_daemon_mode(f"{family}-rootful", "false", "1:0\n")
            contract.verify_daemon_mode(f"{family}-rootless", "true", "1:1000\n")
            for mode, marker, report in (
                ("rootful", "false", "0:"), ("rootless", "true", "2:1000"),
                ("rootful", "false", "1:1000"), ("rootless", "true", "1:0"),
                ("rootful", "true", "1:0"), ("rootless", "false", "1:1000"),
                ("rootful", "false", "1:0\n1:0"), ("rootless", "yes", "1:1000"),
            ):
                with self.subTest(family=family, mode=mode, marker=marker, report=report):
                    with self.assertRaises(contract.ContractError):
                        contract.verify_daemon_mode(f"{family}-{mode}", marker, report)


class ArtifactTests(unittest.TestCase):
    def test_artifact_identity_is_bounded_private_and_not_semantic_approval(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            os.chmod(root, 0o700)
            artifact = root / "artifact.json"
            raw = b'{"private-canary":"not-a-valid-create-request"}\n'
            artifact.write_bytes(raw)
            os.chmod(artifact, 0o600)
            command = [sys.executable, str(SOURCE), "artifact-identity", "--artifact", str(artifact)]
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, hashlib.sha256(raw).hexdigest() + "\n")
            self.assertNotIn("private-canary", result.stdout + result.stderr)
            for malformed in (b"", b"x" * 16_385, b"\xff"):
                with self.subTest(malformed_length=len(malformed)):
                    artifact.write_bytes(malformed)
                    result = subprocess.run(command, capture_output=True, text=True, check=False)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stdout, "")
                    self.assertNotIn("private-canary", result.stderr)

    def test_runner_reuses_selected_artifact_identity_at_every_validation(self) -> None:
        runner = (SOURCE.parent.parent / "docker-application-conformance.sh").read_text()
        binding = 'artifact_sha256=$(python3 "$contract" artifact-identity --artifact "$artifact")'
        self.assertEqual(runner.count(binding), 2)
        self.assertEqual(runner.count('--artifact-sha256 "$artifact_sha256"'), 4)

    image = "registry.invalid/boxferry-core/busybox:1.37.0"
    api = "1.41"
    lane = "debian11-rootful"

    # Authored separately from the artifact; production reads the digest-bound
    # catalogue. Neither the artifact under test nor the decoder is the oracle.
    profiles = {
        "debian11-rootful": {
            "kind": "target",
            "build": {"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u2"},
            "engine_release": "20.10.5+dfsg1",
            "advertised_api_version": "1.41",
            "acquisition_api_version": "1.41",
            "rendering_api_version": "1.41",
            "daemon_mode": "rootful",
            "evidence_sha256": "c" * 64,
        },
    }

    def check(self, *args: object, **kwargs: object) -> dict[str, object]:
        kwargs.setdefault("expected_plan_sha256", hashlib.sha256(args[0].encode()).hexdigest())
        return contract.core_request(*args, profiles=self.profiles, **kwargs)

    @classmethod
    def record(cls) -> dict[str, object]:
        return {
            "schema_version": 1,
            "context": {
                "kind": "target",
                "build": {"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u2"},
                "engine_release": "20.10.5+dfsg1",
                "advertised_api_version": "1.41",
                "acquisition_api_version": "1.41",
                "rendering_api_version": "1.41",
                "daemon_mode": "rootful",
                "evidence_sha256": "c" * 64,
            },
            "requests": [{
                "method": "POST",
                "path": "/v1.41/containers/create?name=bf-docker-core",
                "body": {"Image": cls.image,
                         "Cmd": ["sh", "-c", "printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600"],
                         "HostConfig": {}},
            }],
            "prerequisites": [],
        }

    def test_independent_core_shape_is_allowed(self) -> None:
        self.assertEqual(
            self.check(json.dumps(self.record()) + "\n", self.image, self.api, self.lane, "20.10.5", "1.41"),
            self.record()["requests"][0],
        )

    def test_artifact_reuses_canonical_source_and_shape_prerequisite(self) -> None:
        raw = json.dumps(self.record())
        digest = hashlib.sha256(raw.encode()).hexdigest()
        with mock.patch.object(contract.core_artifact, "validate_core",
                               wraps=contract.core_artifact.validate_core) as validate:
            self.check(raw, self.image, self.api, self.lane, expected_plan_sha256=digest)
        self.assertEqual(validate.call_count, 1)
        self.assertEqual(validate.call_args.args[0], raw.encode())
        self.assertEqual(validate.call_args.kwargs["expected_plan_sha256"], digest)
        self.assertEqual(validate.call_args.kwargs["profile"], self.profiles[self.lane])
        self.assertEqual(validate.call_args.kwargs["source_bytes"],
                         (contract.REPOSITORY_ROOT / "fixtures/conformance/docker-application/core.compose.yaml").read_bytes())

    def test_snapshot_digest_does_not_authorize_changed_or_unsafe_bytes(self) -> None:
        raw = json.dumps(self.record())
        digest = hashlib.sha256(raw.encode()).hexdigest()
        for selected_digest in (None, True, "", "a" * 63, "A" * 64, "0" * 64):
            with self.subTest(digest=selected_digest), self.assertRaisesRegex(
                    contract.ContractError, "exact plan byte binding differs"):
                self.check(raw, self.image, self.api, self.lane, expected_plan_sha256=selected_digest)
        with self.assertRaisesRegex(contract.ContractError, "exact plan byte binding differs"):
            self.check(raw + "\n", self.image, self.api, self.lane, expected_plan_sha256=digest)
        changed = self.record()
        changed["requests"][0]["body"]["HostConfig"] = {"Privileged": True}
        changed_raw = json.dumps(changed)
        with self.assertRaisesRegex(contract.ContractError, "literal create request differs"):
            self.check(changed_raw, self.image, self.api, self.lane,
                       expected_plan_sha256=hashlib.sha256(changed_raw.encode()).hexdigest())

    def test_source_drift_blocks_artifact_even_with_matching_snapshot(self) -> None:
        with mock.patch.object(contract, "bounded_regular_bytes", side_effect=(
                b"---\nname: unreviewed\n", (contract.REPOSITORY_ROOT /
                    "fixtures/conformance/docker-application/core-expectations.json").read_bytes())):
            with self.assertRaisesRegex(contract.ContractError, "authored source bytes differ"):
                self.check(json.dumps(self.record()), self.image, self.api, self.lane)

    def test_selected_rendering_api_and_all_four_native_lane_identities(self) -> None:
        for lane in ("debian11-rootful", "debian11-rootless", "upstream-rootful", "upstream-rootless"):
            with self.subTest(lane=lane):
                debian = lane.startswith("debian11-")
                api = "1.41" if debian else "1.56"
                profile = {"kind": "target", "build": {"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u2"}
                           if debian else {"kind": "upstream"}, "engine_release": "20.10.5+dfsg1" if debian else "29.8.1",
                           "advertised_api_version": api, "acquisition_api_version": api, "rendering_api_version": api,
                           "daemon_mode": lane.rsplit("-", 1)[1], "evidence_sha256": "e" * 64}
                record = self.record()
                record["context"] = profile
                record["requests"][0]["path"] = f"/v{api}/containers/create?name=bf-docker-core"
                raw = json.dumps(record)
                kwargs = {"profiles": {lane: profile}, "expected_plan_sha256": hashlib.sha256(raw.encode()).hexdigest(),
                          "observed_package": "20.10.5+dfsg1-1+deb11u2" if debian else ""}
                self.assertEqual(contract.core_request(raw, self.image, api, lane,
                                 "20.10.5" if debian else "29.8.1", api, **kwargs), record["requests"][0])
                with self.assertRaisesRegex(contract.ContractError, "requested rendering API"):
                    contract.core_request(raw, self.image, "1.40", lane, **kwargs)

    def test_cli_requires_snapshot_digest_and_rejects_mismatch_without_replay(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            artifact = root / "docker-plan.json"
            artifact.write_text(json.dumps(self.record()))
            artifact.chmod(0o600)
            common = ["--artifact", str(artifact), "--image", self.image, "--api-version", self.api,
                      "--lane", self.lane, "--catalogue-json", json.dumps({"profiles": self.profiles})]
            for command in ("validate-artifact", "replay-test-only"):
                options = [] if command == "validate-artifact" else [
                    "--observed-release", "20.10.5", "--observed-api", "1.41", "--observed-package",
                    "20.10.5+dfsg1-1+deb11u2", "--socket", str(root / "absent.sock"),
                    "--state", str(root / "state.json"), "--allow-isolated-apply"]
                missing = subprocess.run([sys.executable, str(SOURCE), command, *common, *options],
                                         capture_output=True, text=True, timeout=5, check=False)
                self.assertEqual(missing.returncode, 2)
                self.assertIn("--artifact-sha256", missing.stderr)
                mismatch = subprocess.run([sys.executable, str(SOURCE), command, *common, *options,
                                           "--artifact-sha256", "0" * 64],
                                          capture_output=True, text=True, timeout=5, check=False)
                self.assertEqual(mismatch.returncode, 1)
                self.assertIn("exact plan byte binding differs", mismatch.stderr)
                self.assertNotIn(str(root), mismatch.stderr)
                self.assertFalse((root / "state.json").exists())

    def test_replay_snapshot_failure_precedes_any_engine_request(self) -> None:
        with mock.patch.object(contract, "engine_request") as request:
            with self.assertRaisesRegex(contract.ContractError, "exact plan byte binding differs"):
                contract.replay("/unavailable/docker.sock", json.dumps(self.record()), self.image,
                                self.api, self.lane, "20.10.5", "1.41", "/unavailable/state.json",
                                profiles=self.profiles, observed_package="20.10.5+dfsg1-1+deb11u2",
                                expected_plan_sha256="0" * 64)
        request.assert_not_called()

    def test_rejects_second_operation(self) -> None:
        value = self.record()
        value["requests"].append(value["requests"][0])
        with self.assertRaisesRegex(contract.ContractError, "literal create request differs"):
            self.check(json.dumps(value), self.image, self.api, self.lane)

    def test_rejects_ambiguous_duplicate_json_key(self) -> None:
        text = json.dumps(self.record()).replace('"method": "POST"', '"method": "POST", "method": "GET"')
        with self.assertRaisesRegex(contract.ContractError, "invalid unambiguous JSON document"):
            self.check(text, self.image, self.api, self.lane)

    def test_rejects_unreviewed_target_operation(self) -> None:
        value = self.record()
        value["requests"][0]["path"] = "/v1.41/images/create"
        with self.assertRaisesRegex(contract.ContractError, "literal create request differs"):
            self.check(json.dumps(value), self.image, self.api, self.lane)

    def test_rejects_privileged_or_omitted_command(self) -> None:
        for bad_body in [
            {"Image": self.image, "Cmd": contract.CORE_COMMAND, "HostConfig": {"Privileged": True}},
            {"Image": self.image, "HostConfig": {}},
        ]:
            value = self.record()
            value["requests"][0]["body"] = bad_body
            with self.assertRaisesRegex(contract.ContractError, "literal create request differs"):
                self.check(json.dumps(value), self.image, self.api, self.lane)

    def test_rejects_missing_prerequisite_or_observed_profile_mismatch(self) -> None:
        value = self.record()
        value["prerequisites"] = [{"kind": "network", "identity": "external"}]
        with self.assertRaisesRegex(contract.ContractError, "prerequisites"):
            self.check(json.dumps(value), self.image, self.api, self.lane)
        value = self.record()
        with self.assertRaisesRegex(contract.ContractError, "observed daemon"):
            self.check(json.dumps(value), self.image, self.api, self.lane, "29.8.1")
        with self.assertRaisesRegex(contract.ContractError, "active reviewed profile"):
            self.check(json.dumps(value), self.image, self.api, "debian11-rootless")

    def test_rejects_observed_api_and_unreviewed_context(self) -> None:
        value = self.record()
        with self.assertRaisesRegex(contract.ContractError, "advertised API"):
            self.check(json.dumps(value), self.image, self.api, self.lane, "20.10.5", "1.49")
        value["context"]["kind"] = "observed"
        with self.assertRaisesRegex(contract.ContractError, "complete target context differs"):
            self.check(json.dumps(value), self.image, self.api, self.lane)

    def test_rejects_noncanonical_image_alias(self) -> None:
        with self.assertRaisesRegex(contract.ContractError, "preloaded image alias"):
            self.check(json.dumps(self.record()), "docker.io/library/busybox:latest", self.api, self.lane)

    def test_artifact_requires_private_cli_style_permissions(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-contract-") as directory:
            root = pathlib.Path(directory)
            artifact = root / "docker-plan.json"
            artifact.write_text(json.dumps(self.record()), encoding="utf-8")
            artifact.chmod(0o644)
            with self.assertRaisesRegex(contract.ContractError, "0600"):
                contract.read_private_artifact(artifact)
            artifact.chmod(0o600)
            self.assertEqual(contract.read_private_artifact(artifact), artifact.read_text())

    def test_valid_looking_profile_changes_are_not_admitted(self) -> None:
        changes = {
            "engine_release": "20.10.5+dfsg1.changed",
            "acquisition_api_version": "1.42",
            "advertised_api_version": "1.49",
            "evidence_sha256": "d" * 64,
            "build": {"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u3"},
        }
        for field, replacement in changes.items():
            with self.subTest(field=field):
                value = self.record()
                value["context"][field] = replacement
                with self.assertRaisesRegex(contract.ContractError, "complete target context differs"):
                    self.check(json.dumps(value), self.image, self.api, self.lane)

    def test_only_exact_reviewed_debian_suffix_and_package_are_allowed(self) -> None:
        for release in ("20.10.5", "20.10.5+dfsg1"):
            self.check(json.dumps(self.record()), self.image, self.api, self.lane, release,
                       observed_package="20.10.5+dfsg1-1+deb11u2")
        for release in ("20.10.5+unknown", "20.10.50", "20.10.5+dfsg1.extra"):
            with self.subTest(release=release), self.assertRaisesRegex(contract.ContractError, "observed daemon"):
                self.check(json.dumps(self.record()), self.image, self.api, self.lane, release)
        for package in ("", "20.10.5", "20.10.5+dfsg1-1+deb11u3"):
            with self.subTest(package=package), self.assertRaisesRegex(contract.ContractError, "installed Docker package"):
                self.check(json.dumps(self.record()), self.image, self.api, self.lane, observed_package=package)

    def test_artifact_is_bounded_before_read_and_fifo_never_blocks(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-bounds-") as directory:
            root = pathlib.Path(directory)
            artifact = root / "docker-plan.json"
            artifact.write_bytes(b"x" * 16_385)
            artifact.chmod(0o600)
            with self.assertRaisesRegex(contract.ContractError, "bounded regular file"):
                contract.read_private_artifact(artifact)
            fifo = root / "fifo"
            os.mkfifo(fifo)
            with self.assertRaisesRegex(contract.ContractError, "bounded regular file"):
                contract.bounded_regular_bytes(fifo, 16_384)

    def test_mocked_replay_records_exact_owned_container_before_start(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-contract-") as directory:
            root = pathlib.Path(directory)
            parent = root / "socket"
            parent.mkdir(mode=0o700)
            socket_path = parent / "docker.sock"
            socket_path.touch()
            replies = [(404, b""), (201, json.dumps({"Id": "b" * 64}).encode()), (204, b"")]
            with mock.patch.object(contract.stat, "S_ISSOCK", return_value=True), mock.patch.object(
                contract, "engine_request", side_effect=replies
            ) as request:
                state = contract.replay(
                    str(socket_path), json.dumps(self.record()), self.image, self.api, self.lane,
                    "20.10.5", "1.41", str(root / "state.json"),
                    profiles=self.profiles, observed_package="20.10.5+dfsg1-1+deb11u2",
                    expected_plan_sha256=hashlib.sha256(json.dumps(self.record()).encode()).hexdigest(),
                )
            self.assertEqual(state["container_id"], "b" * 64)
            self.assertEqual(json.loads((root / "state.json").read_text()), state)
            self.assertEqual(request.call_count, 3)
            self.assertEqual(request.call_args_list[0].args[1:3], ("GET", "/v1.41/containers/bf-docker-core/json"))
            self.assertEqual(request.call_args_list[2].args[1:3], ("POST", f"/v1.41/containers/{'b' * 64}/start"))


class VolumeFixtureTests(unittest.TestCase):
    # Independent literals, never inferred from the producer or tested inventory.
    fixtures = (
        ("forgejo", "forgejo", ("forge-db", "forge-data")),
        ("nextcloud", "cloud", ("cloud-database", "cloud-redis", "cloud-nextcloud")),
        ("paperless-ngx", "paperless", ("paper-data", "paper-media", "paper-consume", "paper-export",
                                     "paper-pgdata", "paper-redisdata")),
        ("immich", "immich", ("immich-library", "immich-model-cache", "immich-pgdata", "immich-redisdata")),
        ("observability", "observability", ("observability-alloy-data", "observability-grafana-data",
                                         "observability-loki-data", "observability-prometheus-data",
                                         "observability-telemetry-logs")),
        ("supabase", "supabase", ("supabase-deno-cache", "supabase-pgdata", "supabase-storage")),
    )
    run_id = "test-run"
    prefix = "test-prefix"
    receipt = "a" * 64
    lane = "upstream-rootless"
    api = "1.56"

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.output = self.root / "output"
        self.output.mkdir(mode=0o700)
        self.profile = {"kind": "target", "build": {"kind": "upstream"}, "engine_release": "29.8.1",
                        "advertised_api_version": self.api, "acquisition_api_version": self.api,
                        "rendering_api_version": self.api, "daemon_mode": "rootless", "evidence_sha256": "b" * 64}
        self.profiles = {self.lane: self.profile}
        self.manifest = {"schema": 1, "scope": "volume-only", "lane": self.lane, "run": self.run_id,
                         "prefix": self.prefix, "candidate_receipt_sha256": self.receipt, "fixtures": []}
        self.requests = []
        for identity, owner, suffixes in self.fixtures:
            source_path = pathlib.Path(f"fixtures/conformance/{identity}-application/compose.yaml")
            source = (contract.REPOSITORY_ROOT / source_path).read_bytes()
            target = self.root / source_path
            target.parent.mkdir(parents=True)
            target.write_bytes(source)
            requests = [{"method": "POST", "path": "/v1.56/volumes/create",
                         "body": {"Name": f"{self.prefix}-{suffix}", "Labels": {
                             "io.boxferry.live-run": self.run_id, "io.boxferry.application": f"{self.prefix}-{owner}"}}}
                        for suffix in suffixes]
            artifact = {"schema_version": 1, "context": copy.deepcopy(self.profile),
                        "requests": requests, "prerequisites": []}
            filename = f"{identity}-volumes.json"
            raw = json.dumps(artifact).encode()
            (self.output / filename).write_bytes(raw)
            os.chmod(self.output / filename, 0o600)
            self.manifest["fixtures"].append({"id": identity, "source_sha256": hashlib.sha256(source).hexdigest(),
                                             "artifact": filename, "artifact_sha256": hashlib.sha256(raw).hexdigest(),
                                             "volume_count": len(suffixes)})
            self.requests.extend(requests)
        self.save_manifest()

    def save_manifest(self) -> None:
        path = self.output / "manifest.json"
        path.write_text(json.dumps(self.manifest))
        os.chmod(path, 0o600)

    def mutate_artifact(self, mutation, index: int = 0) -> None:
        row = self.manifest["fixtures"][index]
        path = self.output / row["artifact"]
        artifact = json.loads(path.read_bytes())
        mutation(artifact)
        raw = json.dumps(artifact).encode()
        path.write_bytes(raw)
        row["artifact_sha256"] = hashlib.sha256(raw).hexdigest()
        self.save_manifest()

    def validate(self) -> list[dict]:
        return contract.validate_volume_fixtures(self.output, self.root, lane=self.lane, run=self.run_id,
                                                prefix=self.prefix, receipt_sha256=self.receipt,
                                                api_version=self.api, profiles=self.profiles)

    def test_independent_six_fixture_23_request_context_and_source_inventory(self) -> None:
        self.assertEqual(self.validate(), self.requests)
        self.assertEqual([row["volume_count"] for row in self.manifest["fixtures"]], [2, 3, 6, 4, 5, 3])
        self.assertEqual(len(self.requests), 23)
        source = self.root / "fixtures/conformance/forgejo-application/compose.yaml"
        source.write_bytes(source.read_bytes() + b"\n# changed source\n")
        with self.assertRaisesRegex(contract.ContractError, "original fixture source differs"):
            self.validate()

    def test_manifest_sha_receipt_field_and_token_confusion_refused(self) -> None:
        original = copy.deepcopy(self.manifest)
        for change in ({"candidate_receipt_sha256": "c" * 64}, {"lane": "upstream-rootful"},
                       {"run": "other-run"}, {"prefix": "other-prefix"}, {"scope": "application"},
                       {"schema": True}, {"raw_private": "not-authorized"}):
            with self.subTest(change=change):
                self.manifest = {**copy.deepcopy(original), **change}
                self.save_manifest()
                with self.assertRaises(contract.ContractError):
                    self.validate()
        for key, value in (("artifact_sha256", "0" * 64), ("source_sha256", "0" * 64),
                           ("volume_count", True), ("artifact", "../escape"), ("id", "other")):
            self.manifest = copy.deepcopy(original)
            self.manifest["fixtures"][0][key] = value
            self.save_manifest()
            with self.subTest(key=key), self.assertRaises(contract.ContractError):
                self.validate()
        for token in ("Bad", "-bad", "bad/name", "a" * 65):
            with self.subTest(token=token), self.assertRaises(contract.ContractError):
                contract.expected_volumes(token, self.prefix)

    def test_complete_artifact_request_labels_context_and_prerequisites_closed(self) -> None:
        original = (self.output / "forgejo-volumes.json").read_bytes()
        mutations = (
            lambda a: a.update(extra="private"),
            lambda a: a.update(schema_version=True),
            lambda a: a.update(context={**a["context"], "evidence_sha256": "f" * 64}),
            lambda a: a.update(prerequisites=[{"kind": "image"}]),
            lambda a: a["requests"].append(copy.deepcopy(a["requests"][0])),
            lambda a: a["requests"][0].update(method="GET"),
            lambda a: a["requests"][0].update(path="/v1.56/containers/create"),
            lambda a: a["requests"][0]["body"].update(Driver="local"),
            lambda a: a["requests"][0]["body"].update(Name="test-prefix-wrong"),
            lambda a: a["requests"][0]["body"]["Labels"].update(extra="private"),
            lambda a: a["requests"][0]["body"]["Labels"].update({"io.boxferry.live-run": "other-run"}),
        )
        for index, mutation in enumerate(mutations):
            (self.output / "forgejo-volumes.json").write_bytes(original)
            self.mutate_artifact(mutation)
            with self.subTest(index=index), self.assertRaises(contract.ContractError):
                self.validate()

    def test_all_23_requests_checked_before_engine_mutation(self) -> None:
        requests = copy.deepcopy(self.requests)
        requests[-1]["path"] = "/v1.56/containers/create"
        with mock.patch.object(contract, "engine_request") as native, self.assertRaises(contract.ContractError):
            contract.apply_volume_fixtures(self.root / "socket", self.root / "ledger", requests,
                                          run=self.run_id, prefix=self.prefix, lane=self.lane, api_version=self.api)
        native.assert_not_called()

    def apply(self, native) -> pathlib.Path:
        state = self.root / "ledger.json"
        with mock.patch.object(contract, "volume_socket"), mock.patch.object(contract, "engine_request", side_effect=native):
            contract.apply_volume_fixtures(self.root / "socket", state, self.validate(),
                                          run=self.run_id, prefix=self.prefix, lane=self.lane, api_version=self.api)
        return state

    def cleanup(self, state: pathlib.Path, native) -> None:
        with mock.patch.object(contract, "volume_socket"), mock.patch.object(contract, "engine_request", side_effect=native):
            contract.cleanup_volume_fixtures(self.root / "socket", state,
                                            run=self.run_id, prefix=self.prefix, lane=self.lane, api_version=self.api)

    def test_existing_volume_cannot_be_reused_or_registered(self) -> None:
        calls = []
        def native(_socket, method, path, body=None, **_kwargs):
            calls.append(method)
            return 200, json.dumps(self.requests[0]["body"]).encode()
        with self.assertRaisesRegex(contract.ContractError, "occupied"):
            self.apply(native)
        self.assertEqual(calls, ["GET"])
        ledger = contract.volume_ledger(self.root / "ledger.json", run=self.run_id, prefix=self.prefix,
                                        lane=self.lane, api_version=self.api)
        self.assertEqual(ledger["volumes"], [])

    def test_partial_post_failure_timeout_and_cancellation_keep_cleanup_ledger(self) -> None:
        for failure in (RuntimeError("private-failure"), TimeoutError("private-timeout"), KeyboardInterrupt()):
            state = self.root / "ledger.json"
            if state.exists():
                state.unlink()
            created = {}
            def native(_socket, method, path, body=None, **_kwargs):
                if method == "POST":
                    record = json.loads(body)
                    ledger = contract.volume_ledger(state, run=self.run_id, prefix=self.prefix,
                                                    lane=self.lane, api_version=self.api)
                    self.assertEqual(ledger["volumes"][-1], record)
                    created[record["Name"]] = record
                    raise failure
                name = path.rsplit("/", 1)[-1]
                if method == "DELETE":
                    created.pop(name)
                    return 204, b""
                return (200, json.dumps(created[name]).encode()) if name in created else (404, b"")
            with self.subTest(failure=type(failure)), self.assertRaises(type(failure)) as raised:
                self.apply(native)
            self.assertIs(raised.exception, failure)
            self.cleanup(state, native)
            self.assertEqual(created, {})

    def test_successful_exact_inspection_and_cleanup_absence_for_all23(self) -> None:
        created = {}
        def native(_socket, method, path, body=None, **_kwargs):
            if method == "POST":
                value = json.loads(body)
                created[value["Name"]] = value
                return 201, json.dumps(value).encode()
            name = path.rsplit("/", 1)[-1]
            if method == "DELETE":
                created.pop(name)
                return 204, b""
            return (200, json.dumps(created[name]).encode()) if name in created else (404, b"")
        state = self.apply(native)
        self.assertEqual(len(created), 23)
        self.cleanup(state, native)
        self.assertEqual(created, {})

    def test_failed_inspect_matching_stdout_wrong_labels_and_failed_absence_cannot_authorize_cleanup(self) -> None:
        state = self.root / "ledger.json"
        ledger = {"schema": 1, "scope": "volume-only", "lane": self.lane, "run": self.run_id,
                  "prefix": self.prefix, "api_version": self.api, "volumes": [self.requests[0]["body"]]}
        state.write_text(json.dumps(ledger))
        os.chmod(state, 0o600)
        for status, value in ((500, self.requests[0]["body"]),
                              (200, {**self.requests[0]["body"], "Labels": {"io.boxferry.live-run": self.run_id}})):
            calls = []
            def native(_socket, method, path, body=None, **_kwargs):
                calls.append(method)
                return status, json.dumps(value).encode()
            with self.subTest(status=status), self.assertRaises(contract.ContractError):
                self.cleanup(state, native)
            self.assertEqual(calls, ["GET"])
        with self.assertRaises(contract.ContractError):
            self.cleanup(state, mock.Mock(side_effect=[(200, json.dumps(self.requests[0]["body"]).encode()),
                                                       (204, b""), (500, b"")]))

    def test_volume_deadline_refusal_precedes_request(self) -> None:
        with mock.patch.object(contract.time, "monotonic", return_value=10), \
             mock.patch.object(contract, "engine_request") as native, self.assertRaises(contract.ContractError):
            contract.volume_call(self.root / "socket", "POST", "/v1.56/volumes/create", b"{}", 10)
        native.assert_not_called()

    def test_last_fixture_refusal_and_private_output_schema_do_not_reach_apply(self) -> None:
        self.mutate_artifact(lambda a: a["requests"][-1]["body"].update(Driver="unreviewed"), index=5)
        with mock.patch.object(contract, "engine_request") as native, self.assertRaises(contract.ContractError):
            self.validate()
        native.assert_not_called()
        raw = (self.output / "manifest.json").read_bytes()
        (self.output / "manifest.json").write_bytes(raw.replace(b'"schema": 1', b'"schema": 1, "schema": 1'))
        with self.assertRaises(contract.ContractError):
            self.validate()
        (self.output / "manifest.json").write_bytes(raw)
        os.chmod(self.output / "manifest.json", 0o644)
        with self.assertRaises(contract.ContractError):
            self.validate()

    def test_volume_runner_preserves_core_oracle_and_excludes_core_workload(self) -> None:
        runner = (SOURCE.parent.parent / "docker-application-conformance.sh").read_text()
        self.assertIn('--profile volume-fixtures --destination "$boxferry_snapshot"', runner)
        self.assertIn('--lane "$lane" --run "$volume_run" --prefix "$volume_prefix"', runner)
        self.assertRegex(runner, r'if \[\[ \$profile != volume-fixtures \]\]; then\n\s+fixture_pull_attempted=true')
        begin = runner.index('if [[ $profile == volume-fixtures ]]; then\n  bounded 30s python3 "$contract" verify-candidate')
        end = runner.index('\nelse\n', begin)
        volume_branch = runner[begin:end]
        self.assertIn("apply-volume-fixtures --allow-isolated-apply", volume_branch)
        self.assertNotIn("containers/create", volume_branch)
        self.assertNotIn("load --input", volume_branch)
        self.assertNotIn("convert docker compose", volume_branch)
        self.assertIn("cleanup-volume-fixtures --allow-isolated-apply", runner)
        self.assertIn("write-volume-evidence", runner)
        self.assertIn("volume_candidate_proof(args.boxferry_root", SOURCE.read_text())
        self.assertLess(runner.index("apply-volume-fixtures --allow-isolated-apply"), runner.index("capture-volume-evidence"))
        self.assertLess(runner.rindex("cleanup_owned ||"), runner.rindex("if ! finish_volume_evidence"))
        self.assertEqual(contract.CORE_REACQUIRE_REPORT_EXPECTATIONS, {})
        self.assertIn("no container/image workload or six-application acceptance", runner)

    def test_cleanup_refuses_modified_ledger_and_timeout_and_creation_inspect_failure(self) -> None:
        created = {}
        state = self.root / "ledger.json"
        def native(_socket, method, path, body=None, **_kwargs):
            if method == "POST":
                record = json.loads(body)
                created[record["Name"]] = record
                return 201, json.dumps(record).encode()
            return (500, json.dumps(next(iter(created.values()))).encode()) if created else (404, b"")
        with self.assertRaisesRegex(contract.ContractError, "created volume inspection failed"):
            self.apply(native)
        ledger = json.loads(state.read_bytes())
        ledger["volumes"][0]["Name"] = "ambient-volume"
        state.write_text(json.dumps(ledger))
        with mock.patch.object(contract, "engine_request") as calls, self.assertRaises(contract.ContractError):
            self.cleanup(state, calls)
        calls.assert_not_called()
        ledger["volumes"][0]["Name"] = self.requests[0]["body"]["Name"]
        state.write_text(json.dumps(ledger))
        with mock.patch.object(contract.os, "geteuid", return_value=state.stat().st_uid + 1), \
                mock.patch.object(contract, "engine_request") as calls, self.assertRaises(contract.ContractError):
            self.cleanup(state, calls)
        calls.assert_not_called()
        with self.assertRaises(contract.ContractError):
            self.cleanup(state, mock.Mock(side_effect=TimeoutError("private-body")))


class StorageMountTests(unittest.TestCase):
    destination = "/home/docker/.local/share/docker"

    def row(self, options: str = "rw,relatime", destination: str | None = None) -> bytes:
        selected = destination or self.destination
        return f"42 31 0:99 / {selected} {options} - ext4 /dev/sda rw\n".encode()

    def test_exact_historical_mount_and_docker_root(self) -> None:
        contract.verify_storage_mount(self.row(), self.destination)
        contract.verify_docker_root("debian11-rootless", self.destination)
        contract.verify_docker_root("upstream-rootful", "/var/lib/docker")

    def test_bad_mount_flags_duplicate_and_structure_fail(self) -> None:
        cases = [
            self.row("ro,relatime"),
            self.row("rw,nosuid"),
            self.row("rw,nodev"),
            self.row() + self.row(),
            b"42 31 0:99 / /home/docker/.local/share/docker rw ext4 /dev/sda rw\n",
            self.row(destination="/var/lib/docker"),
            b"x" * 1_048_577,
        ]
        for value in cases:
            with self.subTest(value=value[:80]), self.assertRaises(contract.ContractError):
                contract.verify_storage_mount(value, self.destination)

    def test_wrong_docker_root_is_rejected_for_each_mode(self) -> None:
        for lane, path in (("debian11-rootless", "/var/lib/docker"),
                           ("upstream-rootful", self.destination)):
            with self.subTest(lane=lane), self.assertRaisesRegex(contract.ContractError, "DockerRootDir"):
                contract.verify_docker_root(lane, path)

    def test_mountinfo_cli_reads_only_bounded_data(self) -> None:
        command = [sys.executable, str(SOURCE), "verify-storage-mount", "--destination", self.destination]
        accepted = subprocess.run(command, input=self.row(), capture_output=True, check=False)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        rejected = subprocess.run(command, input=self.row("rw,nodev"), capture_output=True, check=False)
        self.assertEqual(rejected.returncode, 1)
        self.assertNotIn(self.destination.encode(), rejected.stderr)

    def test_outer_mounts_require_exact_identities_destinations_and_count(self) -> None:
        volume = "bf-docker-core-data-abcdefgh"
        socket_dir = "/tmp/boxferry-docker-core.abcdefgh/socket"
        mounts = [
            {"Type": "volume", "Name": volume, "Destination": self.destination},
            {"Type": "bind", "Source": socket_dir, "Destination": "/boxferry-core"},
        ]
        contract.verify_outer_mounts(json.dumps(mounts).encode(), volume, self.destination, socket_dir)
        variants = [
            [{**mounts[0], "Destination": "/var/lib/docker"}, mounts[1]],
            [mounts[0], {**mounts[1], "Destination": "/wrong"}],
            [mounts[0], {**mounts[1], "Source": "/other/socket"}],
            [mounts[0], mounts[0]],
            mounts + [mounts[1]],
        ]
        for variant in variants:
            with self.subTest(variant=variant), self.assertRaises(contract.ContractError):
                contract.verify_outer_mounts(json.dumps(variant).encode(), volume, self.destination, socket_dir)
        with self.assertRaises(contract.ContractError):
            contract.verify_outer_mounts(b" " * 16_385, volume, self.destination, socket_dir)


class ParentLifetimeTests(unittest.TestCase):
    def test_parent_alive_cli_rejects_missing_and_recycled_pid_with_status_three(self) -> None:
        pid = os.getpid()
        start = contract.process_start(pid)
        for candidate_pid, candidate_start in ((pid, start + 1), (2_147_483_647, start)):
            with self.subTest(pid=candidate_pid):
                result = subprocess.run(
                    [sys.executable, str(SOURCE), "parent-alive", "--parent-pid", str(candidate_pid),
                     "--parent-start", str(candidate_start)],
                    capture_output=True, text=True, timeout=3, check=False,
                )
                self.assertEqual(result.returncode, 3, result.stderr)

    def test_recycled_pid_identity_cannot_be_signaled(self) -> None:
        pid = os.getpid()
        original_start = contract.process_start(pid)
        with mock.patch.object(contract, "process_start", return_value=original_start + 1), \
             mock.patch.object(contract.signal, "pidfd_send_signal") as send:
            with self.assertRaises(contract.ParentGone):
                contract.signal_parent(pid, original_start)
            with self.assertRaises(contract.ParentGone):
                contract.guard_parent(pid, original_start, 1, 1)
            send.assert_not_called()

    def test_exited_parent_stops_guard_without_signal(self) -> None:
        parent = subprocess.Popen(["sleep", "0.2"])
        self.addCleanup(lambda: parent.poll() is None and parent.kill())
        start = contract.process_start(parent.pid)
        begun = time.monotonic()
        with mock.patch.object(contract.signal, "pidfd_send_signal") as send:
            contract.guard_parent(parent.pid, start, 1, 1)
            send.assert_not_called()
        self.assertLess(time.monotonic() - begun, 0.8)
        parent.wait(timeout=2)
        with self.assertRaises(contract.ParentGone):
            contract.signal_parent(parent.pid, start)

    def test_guard_acknowledges_only_after_verifying_live_parent(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-guard-") as name:
            marker = pathlib.Path(name) / "ready"
            parent = subprocess.Popen(["sleep", "0.2"])
            self.addCleanup(lambda: parent.poll() is None and parent.kill())
            start = contract.process_start(parent.pid)
            contract.guard_parent(parent.pid, start, 1, 1, str(marker))
            self.assertEqual(marker.read_text(encoding="ascii"),
                             f"{os.getpid()}:{contract.process_start(os.getpid())}")
            rejected = pathlib.Path(name) / "rejected"
            with self.assertRaises(contract.ParentGone):
                contract.guard_parent(os.getpid(), contract.process_start(os.getpid()) + 1,
                                      1, 1, str(rejected))
            self.assertFalse(rejected.exists())


class PresenceTests(unittest.TestCase):
    def test_native_empty_status_only_and_complete_combined_stream_are_authoritative(self) -> None:
        cases = [("import sys; sys.exit(0)", "present"),
                 ("import sys; sys.exit(1)", "absent"),
                 ("import sys; sys.stderr.write('DO-NOT-PRINT/config-error'); sys.exit(1)", "unknown"),
                 ("import sys; sys.stderr.write('DO-NOT-PRINT/warning'); sys.exit(0)", "unknown"),
                 ("print(' ')", "unknown"),
                 ("import sys; sys.exit(2)", "unknown"),
                 ("import sys; sys.exit(125)", "unknown"),
                 ("import os, signal; os.kill(os.getpid(), signal.SIGTERM)", "unknown"),
                 ("import sys; sys.stdout.write('x'*16385)", "oversized"),
                 ("import sys; sys.stderr.write('x'*16385)", "oversized")]
        for source, expected in cases:
            with self.subTest(source=source):
                self.assertEqual(contract.readiness_read([sys.executable, "-c", source],
                                                        time.monotonic() + 2, presence=True), (expected, b""))

    def test_presence_helper_closes_failures_cancellation_and_restores_handlers(self) -> None:
        previous = {signum: contract.signal.getsignal(signum) for signum in
                    (contract.signal.SIGTERM, contract.signal.SIGINT, contract.signal.SIGHUP)}
        for kind, prefix in (("container", "bf-docker-core-"), ("volume", "bf-docker-core-data-")):
            for outcome in ("present", "absent", "unknown", "read-failed", "oversized", "timed-out",
                            "termination-unverified", KeyboardInterrupt, OSError):
                with self.subTest(kind=kind, outcome=outcome):
                    with mock.patch.object(contract, "readiness_read") as read:
                        if isinstance(outcome, str):
                            read.return_value = outcome, b""
                        else:
                            read.side_effect = outcome
                        self.assertEqual(contract.podman_presence(kind, prefix + "test", "test"),
                                         outcome if outcome in ("present", "absent") else "unknown")
                        self.assertEqual(read.call_args.args[0], ["podman", kind, "exists", prefix + "test"])
                        self.assertEqual(read.call_args.kwargs, {"presence": True})
                    self.assertEqual(previous, {signum: contract.signal.getsignal(signum) for signum in previous})
        with mock.patch.object(contract, "readiness_read") as read:
            for kind, name, run in (("network", "bf-docker-core-test", "test"),
                                    ("container", "ambient", "test"), ("volume", "bf-docker-core-test", "test"),
                                    ("container", "bf-docker-core-../other", "../other")):
                self.assertEqual(contract.podman_presence(kind, name, run), "unknown")
            read.assert_not_called()

    def test_presence_deadline_cancellation_and_unverified_teardown_cannot_report_absence(self) -> None:
        started = time.monotonic()
        result = contract.readiness_read([sys.executable, "-c", "import time; time.sleep(60)"],
                                         started + 0.1, presence=True)
        # Scheduling may exhaust the same deadline before teardown is verified;
        # neither failure may supply presence/absence or retain native output.
        self.assertIn(result, (("timed-out", b""), ("termination-unverified", b"")))
        self.assertLess(time.monotonic() - started, 2)
        with mock.patch.object(contract.subprocess, "Popen") as launched:
            self.assertEqual(contract.readiness_read(["fake-read"], time.monotonic() - 1, presence=True),
                             ("timed-out", b""))
        launched.assert_not_called()
        for action in ("timeout", "cancel", "kill-failed", "reap-failed"):
            with self.subTest(action=action), tempfile.TemporaryFile() as stream:
                child = mock.Mock(pid=12345, stdout=stream)
                child.wait.return_value = 1
                if action == "reap-failed":
                    child.wait.side_effect = subprocess.TimeoutExpired("private-command", 1)
                def kill_group(_pid, signum):
                    if action == "kill-failed":
                        raise PermissionError
                    if signum == 0:
                        raise ProcessLookupError
                with mock.patch.object(contract.subprocess, "Popen", return_value=child), \
                        mock.patch.object(contract.select, "select", side_effect=KeyboardInterrupt if action == "cancel" else None,
                                          return_value=([] if action == "timeout" else [stream.fileno()], [], [])), \
                        mock.patch.object(contract.os, "waitid", return_value=mock.Mock()), \
                        mock.patch.object(contract.os, "killpg", side_effect=kill_group):
                    if action == "cancel":
                        with self.assertRaises(KeyboardInterrupt):
                            contract.readiness_read(["fake-read"], time.monotonic() + 2, presence=True)
                    else:
                        self.assertEqual(contract.readiness_read(["fake-read"], time.monotonic() + 2, presence=True),
                                         ("timed-out" if action == "timeout" else "termination-unverified", b""))
                self.assertTrue(stream.closed)
                child.wait.assert_called_once()

    def test_reaped_empty_native_one_requires_bounded_positive_group_disappearance(self) -> None:
        for readback in ("persists", "permission-error", "disappears"):
            with self.subTest(readback=readback), tempfile.TemporaryFile() as stream:
                child = mock.Mock(pid=12345, stdout=stream)
                events = []
                def reap(**_kwargs):
                    events.append("reap")
                    return 1
                def kill_group(pid, signum):
                    self.assertEqual(pid, 12345)
                    if signum == contract.signal.SIGKILL:
                        self.assertNotIn("reap", events)
                        events.append("signal-owned-group")
                    else:
                        self.assertEqual(signum, 0)
                        self.assertIn("reap", events)
                        events.append("read-group")
                        if readback == "permission-error":
                            raise PermissionError("DO-NOT-PRINT")
                        if readback == "disappears":
                            raise ProcessLookupError
                child.wait.side_effect = reap
                started = time.monotonic()
                with mock.patch.object(contract.subprocess, "Popen", return_value=child), \
                        mock.patch.object(contract.os, "waitid", return_value=mock.Mock()), \
                        mock.patch.object(contract.os, "killpg", side_effect=kill_group):
                    result = contract.readiness_read(["fake-read"], time.monotonic() + 2, presence=True)
                self.assertEqual(result, ("absent" if readback == "disappears" else "termination-unverified", b""))
                self.assertLess(time.monotonic() - started, 0.75)
                self.assertEqual(events[:2], ["signal-owned-group", "reap"])
                self.assertIn("read-group", events)
                self.assertLessEqual(events.count("read-group"), 26)
                self.assertTrue(stream.closed)
                child.wait.assert_called_once()

    def test_cli_closed_markers_with_fake_native_tool_never_print_native_diagnostics(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            native = pathlib.Path(name) / "podman"
            environment = dict(os.environ, PATH=name + os.pathsep + os.environ["PATH"])
            for source, status, marker in (("exit 0", 0, "present"), ("exit 1", 1, "absent"),
                                           ("printf 'DO-NOT-PRINT/config-error' >&2; exit 1", 2, "unknown"),
                                           ("printf 'DO-NOT-PRINT/warning' >&2; exit 0", 2, "unknown")):
                with self.subTest(source=source):
                    native.write_text("#!/bin/sh\n" + source + "\n", encoding="ascii")
                    native.chmod(0o700)
                    result = subprocess.run([sys.executable, str(SOURCE), "podman-presence", "--kind", "container",
                                             "--name", "bf-docker-core-test", "--run", "test"],
                                            env=environment, capture_output=True, text=True, timeout=6, check=False)
                    self.assertEqual((result.returncode, result.stdout, result.stderr), (status, marker + "\n", ""))


class PingObservationTests(unittest.TestCase):
    def test_literal_envelopes_and_native_status_are_independent_of_phrase_like_paths(self) -> None:
        cases = [(7, "000", b"curl: (7) Couldn't connect to server\n", "connect-error-observed"),
                 (7, "000", b"curl: (7) Could not connect to server\n", "connect-error-observed"),
                 (7, "000", b"curl: (7) Failed to connect to localhost port 80 after 0 ms: Could not connect to server\n",
                  "connect-error-observed"),
                 (7, "000", b"curl: (7) Failed to connect to localhost:80 after 0 ms: Could not connect to server\n",
                  "connect-error-observed"),
                 (7, "000", b"curl: (7) Failed to connect to localhost port 80 after 12 ms: Connection refused\n",
                  "connect-error-observed"),
                 (28, "000", b"curl: (28) Operation timed out after 5000 milliseconds with 0 bytes received\n",
                  "timeout-error-observed"),
                 (22, "503", b"curl: (22) The requested URL returned error: 503\n", "http-error-observed"),
                 (5, "000", b"curl: (5) Could not resolve proxy: private.invalid\n", "proxy-resolution-error-observed"),
                 (6, "000", b"curl: (6) Could not resolve host: localhost\n", "host-resolution-error-observed")]
        for status, http, raw, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(contract.readiness_ping_error(raw, status, http), expected)
                self.assertEqual(contract.readiness_ping_error(raw, 0, http), "unknown")
                for altered in (b"/private/" + raw, raw + b"DO-NOT-PRINT", raw + raw, raw + b"\x00",
                                raw.replace(b"curl:", b"path-curl:"), b"DO-NOT-PRINT " + raw):
                    self.assertEqual(contract.readiness_ping_error(altered, status, http), "unknown")
        for raw in (b"", b"\n", "curl: (7) Verbindung verweigert".encode(),
                    b"curl: (7) Failed to connect to /private/Connection refused\n", b"x" * 16_385):
            self.assertEqual(contract.readiness_ping_error(raw, 7, "000"), "unknown")
        self.assertEqual(contract.readiness_ping_error(b"curl: (22) The requested URL returned error: 503\n",
                                                     22, "404"), "unknown")
        self.assertEqual(contract.readiness_ping_error(cases[0][2], 7, "\u2603"), "unknown")

    def collect(self, source: str, *, seconds: float = 2):
        return contract.native_read.native_poll_read([sys.executable, "-c", source], time.monotonic() + seconds)

    def test_separate_streams_preserve_completed_nonzero_native_status(self) -> None:
        self.assertEqual(self.collect("import sys; sys.stdout.write('000'); sys.stderr.write('private-error'); sys.exit(7)"),
                         ("completed", 7, b"000", b"private-error"))
        self.assertEqual(self.collect("import sys; sys.stdout.write('000'); sys.exit(28)")[:2], ("completed", 28))
        self.assertEqual(self.collect("import sys; sys.stdout.write('200')"), ("completed", 0, b"200", b""))

    def test_readiness_curl_has_exact_isolated_argv_and_one_original_invocation(self) -> None:
        socket_path = pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock")
        expected = ["curl", "-q", "--noproxy", "*", "--fail", "--silent", "--show-error", "--max-time", "5",
                    "--max-filesize", "65536", "--output", "/dev/null", "--write-out", "%{http_code}",
                    "--unix-socket", str(socket_path), "http://localhost/_ping"]
        for native_status, http in ((0, "200"), (7, "000")):
            with self.subTest(native_status=native_status):
                def collect(arguments, deadline, *, teardown_observations):
                    self.assertEqual(arguments, expected)
                    self.assertEqual(deadline, 103)
                    teardown_observations["signal"] = "denied"
                    return ("completed", native_status, http.encode(),
                            b"" if native_status == 0 else b"curl: (7) Couldn't connect to server\n")
                with mock.patch.object(contract.time, "clock_gettime", return_value=150), \
                        mock.patch.object(contract.time, "monotonic", return_value=100), \
                        mock.patch.object(contract.native_read, "native_poll_read", side_effect=collect) as call:
                    result = contract.readiness_ping(socket_path, "153.00")
                call.assert_called_once()
                self.assertEqual(set(call.call_args.kwargs), {"teardown_observations"})
                self.assertEqual(result["ping-curl-exit"], str(native_status))
                self.assertEqual(result["ping-http-status"], http)
                self.assertEqual(result["ping-teardown-signal"], "denied")
                self.assertEqual(result["ping-collector"], "completed")

    def test_overflow_continues_draining_both_streams_and_preserves_native_completion(self) -> None:
        for stream in ("stdout", "stderr", "both"):
            with self.subTest(stream=stream):
                writes = "; ".join(f"sys.{target}.write('x' * 200000); sys.{target}.flush()"
                                   for target in (("stdout", "stderr") if stream == "both" else (stream,)))
                source = f"import sys; {writes}; sys.stdout.write('000'); sys.stderr.write('private-tail'); sys.exit(7)"
                self.assertEqual(self.collect(source), (f"{stream}-oversized", 7,
                    b"000" if stream == "stderr" else b"", b"private-tail" if stream == "stdout" else b""))

    def test_eof_before_exit_and_cutoff_do_not_manufacture_curl_timeout_status(self) -> None:
        started = time.monotonic()
        result = self.collect("import os,time; os.close(1); os.close(2); time.sleep(60)", seconds=0.4)
        self.assertEqual(result, ("timed-out", None, b"", b""))
        self.assertLess(time.monotonic() - started, 1)
        with mock.patch.object(contract.native_read.subprocess, "Popen") as launched:
            self.assertEqual(self.collect("unused", seconds=0.2), ("timed-out", None, b"", b""))
        launched.assert_not_called()

    def test_descendant_pipe_does_not_erase_already_observed_native_exit(self) -> None:
        source = ("import os,time; child=os.fork(); "
                  "time.sleep(60) if child == 0 else os._exit(7)")
        result = self.collect(source, seconds=0.45)
        self.assertIn(result[0], ("timed-out", "termination-unverified"))
        self.assertEqual(result[1:], (7, b"", b""))

    def test_waitnowait_identity_precedes_group_signal_and_teardown_uses_remaining_budget(self) -> None:
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            out.write(b"000"); out.seek(0)
            events = []
            child = mock.Mock(pid=12345, stdout=out, stderr=err)
            def observe(*args):
                self.assertEqual(args, (os.P_PID, 12345, os.WEXITED | os.WNOHANG | os.WNOWAIT))
                events.append("observe")
                return SimpleNamespace(si_code=os.CLD_EXITED, si_status=7)
            def signal_group(_pid, sig):
                if sig == 0:
                    raise ProcessLookupError
                events.append("signal")
            def reap(**kwargs):
                self.assertGreaterEqual(kwargs["timeout"], 0)
                self.assertLessEqual(kwargs["timeout"], 0.25)
                events.append("reap")
                return 7
            child.wait.side_effect = reap
            with mock.patch.object(contract.native_read.subprocess, "Popen", return_value=child), \
                    mock.patch.object(contract.native_read.os, "waitid", side_effect=observe), \
                    mock.patch.object(contract.native_read.os, "killpg", side_effect=signal_group):
                result = self.collect("fake")
            self.assertEqual(result, ("completed", 7, b"000", b""))
            self.assertLess(events.index("observe"), events.index("signal"))
            self.assertLess(events.index("signal"), events.index("reap"))
            child.poll.assert_not_called()
            self.assertTrue(out.closed and err.closed)

    def test_cancellation_and_termination_uncertainty_never_supply_induced_native_exit(self) -> None:
        for failure in (KeyboardInterrupt(), PermissionError("DO-NOT-PRINT")):
            with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
                child = mock.Mock(pid=12345, stdout=out, stderr=err)
                child.wait.return_value = -9
                with mock.patch.object(contract.native_read.subprocess, "Popen", return_value=child), \
                        mock.patch.object(contract.native_read.os, "waitid", side_effect=failure), \
                        mock.patch.object(contract.native_read.os, "killpg", side_effect=ProcessLookupError):
                    result = self.collect("fake")
                self.assertEqual(result, ("cancelled" if isinstance(failure, KeyboardInterrupt) else "read-failed",
                                          None, b"", b""))
                self.assertTrue(out.closed and err.closed)
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            child = mock.Mock(pid=12345, stdout=out, stderr=err)
            child.wait.side_effect = subprocess.TimeoutExpired("DO-NOT-PRINT", 0.25)
            with mock.patch.object(contract.native_read.subprocess, "Popen", return_value=child), \
                    mock.patch.object(contract.native_read.os, "waitid", side_effect=KeyboardInterrupt), \
                    mock.patch.object(contract.native_read.os, "killpg", side_effect=PermissionError):
                self.assertEqual(self.collect("fake"), ("termination-unverified", None, b"", b""))

    def test_absolute_readiness_deadline_accounts_startup_without_six_second_window(self) -> None:
        now = [100.0]
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            child = mock.Mock(pid=12345, stdout=out, stderr=err)
            def startup(*_args, **_kwargs):
                now[0] = 101.8
                return child
            def elapsed(*_args):
                now[0] = 102
                return ([], [], [])
            with mock.patch.object(contract.native_read.subprocess, "Popen", side_effect=startup), \
                    mock.patch.object(contract.native_read.time, "monotonic", side_effect=lambda: now[0]), \
                    mock.patch.object(contract.native_read.select, "select", side_effect=elapsed), \
                    mock.patch.object(contract.native_read.os, "waitid", return_value=None), \
                    mock.patch.object(contract.native_read.os, "killpg", side_effect=ProcessLookupError):
                result = contract.native_read.native_poll_read(["fake"], 102)
            self.assertEqual(result, ("timed-out", None, b"", b""))
            self.assertAlmostEqual(child.wait.call_args.kwargs["timeout"], 0.2)
        with mock.patch.object(contract.time, "clock_gettime", return_value=150), \
                mock.patch.object(contract.time, "monotonic", return_value=100), \
                mock.patch.object(contract.native_read, "native_poll_read", return_value=("completed", 7, b"000",
                    b"curl: (7) Couldn't connect to server\n")) as collect:
            result = contract.readiness_ping(pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock"), "153.00")
        self.assertEqual(collect.call_args.args[1], 103)
        self.assertEqual(collect.call_args.args[0], ["curl", "-q", "--noproxy", "*", "--fail", "--silent", "--show-error", "--max-time", "5",
                         "--max-filesize", "65536", "--output", "/dev/null", "--write-out", "%{http_code}",
                         "--unix-socket", "/tmp/boxferry-docker-core.test/socket/docker.sock", "http://localhost/_ping"])
        self.assertEqual(result["ping-curl-error"], "connect-error-observed")

    def test_atomic_poll_fields_do_not_survive_incomplete_oversized_or_malformed_streams(self) -> None:
        path = pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock")
        for outcome, status, stdout in (("completed", 7, b"000\n"), ("stdout-oversized", 7, b""),
                                       ("timed-out", None, b"000"), ("termination-unverified", 7, b"000")):
            with mock.patch.object(contract.native_read, "native_poll_read", return_value=(outcome, status, stdout,
                    b"curl: (7) Couldn't connect to server\n")):
                result = contract.readiness_ping(path, "999999999.00")
            self.assertEqual(result["ping-http-status"], "unknown")
            self.assertEqual(result["ping-curl-error"], "unknown")
            self.assertEqual(result["ping-curl-exit"], "7" if status == 7 else "unknown")
        with mock.patch.object(contract.native_read, "native_poll_read") as collect:
            result = contract.readiness_ping(pathlib.Path("/ambient/DO-NOT-PRINT"), "DO-NOT-PRINT")
        collect.assert_not_called()
        self.assertNotIn("DO-NOT-PRINT", str(result))

    def test_stderr_overflow_keeps_http_but_stdout_overflow_and_malformed_output_fail_closed(self) -> None:
        path = pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock")
        cases = [("stderr-oversized", b"200", b"", "200", "oversized"),
                 ("stdout-oversized", b"", b"private-tail", "unknown", "oversized"),
                 ("both-oversized", b"", b"", "unknown", "oversized"),
                 ("completed", b"200\n", b"", "unknown", "invalid-output"),
                 ("completed", b"DO-NOT-PRINT/private", b"", "unknown", "invalid-output")]
        for outcome, stdout, stderr, http, collector in cases:
            with self.subTest(outcome=outcome, stdout=stdout), \
                    mock.patch.object(contract.native_read, "native_poll_read", return_value=(
                        outcome, 0, stdout, stderr)):
                result = contract.readiness_ping(path, "999999999.00")
            self.assertEqual(result, {"ping-curl-exit": "0", "ping-http-status": http,
                                     "ping-curl-error": "unknown", "ping-collector": collector,
                                     "ping-teardown-signal": "unknown"})
            self.assertNotIn("DO-NOT-PRINT", str(result))
        self.assertEqual(self.collect("import sys; sys.stderr.write('x' * 200000); sys.stdout.write('200')"),
                         ("stderr-oversized", 0, b"200", b""))
        self.assertEqual(self.collect("import sys; sys.stdout.write('x' * 200000); sys.stderr.write('private')"),
                         ("stdout-oversized", 0, b"", b"private"))

    def test_boottime_only_expiry_or_unavailable_completion_clock_keeps_native_fields_not_readiness(self) -> None:
        path = pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock")
        for status in (0, 7):
            for final_clock in (153.0, 154.0, OSError("DO-NOT-PRINT")):
                with self.subTest(status=status, final_clock=final_clock), \
                        mock.patch.object(contract.time, "monotonic", return_value=100), \
                        mock.patch.object(contract.time, "clock_gettime", side_effect=[150, final_clock]), \
                        mock.patch.object(contract.native_read, "native_poll_read", return_value=(
                            "completed", status, b"200" if status == 0 else b"000",
                            b"" if status == 0 else b"curl: (7) Couldn't connect to server\n")) as collect:
                    result = contract.readiness_ping(path, "153.00")
                self.assertEqual(collect.call_args.args[1], 103)
                self.assertEqual(result["ping-curl-exit"], str(status))
                self.assertEqual(result["ping-http-status"], "200" if status == 0 else "000")
                self.assertEqual(result["ping-curl-error"], "unknown" if status == 0 else "connect-error-observed")
                self.assertEqual(result["ping-collector"], "read-failed" if isinstance(final_clock, OSError)
                                 else "timed-out")
                self.assertNotIn("DO-NOT-PRINT", str(result))

    def test_failed_collection_keeps_teardown_uncertainty_instead_of_later_clock_outcome(self) -> None:
        path = pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock")
        for outcome in ("termination-unverified", "cancelled", "read-failed", "timed-out", "launch-failed"):
            for final_clock in (153.0, OSError("DO-NOT-PRINT"), KeyboardInterrupt()):
                with self.subTest(outcome=outcome, final_clock=final_clock), \
                        mock.patch.object(contract.time, "clock_gettime", side_effect=[150, final_clock]) as clock, \
                        mock.patch.object(contract.native_read, "native_poll_read", return_value=(
                            outcome, 7, b"", b"")):
                    result = contract.readiness_ping(path, "153.00")
                self.assertEqual(result, {"ping-curl-exit": "7", "ping-http-status": "unknown",
                                         "ping-curl-error": "unknown", "ping-collector": outcome,
                                         "ping-teardown-signal": "unknown"})
                self.assertEqual(clock.call_count, 1)
                self.assertNotIn("DO-NOT-PRINT", str(result))

    def test_collection_stops_at_five_seconds_even_with_a_long_absolute_budget(self) -> None:
        now = [100.0]
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            child = mock.Mock(pid=12345, stdout=out, stderr=err)
            def elapsed(_read, _write, _error, timeout):
                self.assertLessEqual(timeout, 0.05)
                now[0] = 105.0
                return ([], [], [])
            with mock.patch.object(contract.native_read.subprocess, "Popen", return_value=child), \
                    mock.patch.object(contract.native_read.time, "monotonic", side_effect=lambda: now[0]), \
                    mock.patch.object(contract.native_read.select, "select", side_effect=elapsed), \
                    mock.patch.object(contract.native_read.os, "waitid", return_value=None), \
                    mock.patch.object(contract.native_read.os, "killpg", side_effect=ProcessLookupError):
                result = contract.native_read.native_poll_read(["fake"], 280)
            self.assertEqual(result, ("timed-out", None, b"", b""))
            child.wait.assert_called_once_with(timeout=0.25)

    def test_completed_native_status_survives_absolute_deadline_or_teardown_cancellation(self) -> None:
        for boundary in ("deadline", "cancelled"):
            with self.subTest(boundary=boundary), tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
                now = [100.0]
                out.write(b"000"); out.seek(0)
                child = mock.Mock(pid=12345, stdout=out, stderr=err)
                def signal_group(_pid, sig):
                    if sig == 0 and boundary == "deadline":
                        now[0] = 102.0
                        raise ProcessLookupError
                with mock.patch.object(contract.native_read.subprocess, "Popen", return_value=child), \
                        mock.patch.object(contract.native_read.time, "monotonic", side_effect=lambda: now[0]), \
                        mock.patch.object(contract.native_read.time, "sleep", side_effect=KeyboardInterrupt), \
                        mock.patch.object(contract.native_read.os, "waitid", return_value=SimpleNamespace(
                            si_code=os.CLD_EXITED, si_status=7)), \
                        mock.patch.object(contract.native_read.os, "killpg", side_effect=signal_group):
                    result = contract.native_read.native_poll_read(["fake"], 102)
                self.assertEqual(result, ("termination-unverified", 7, b"", b""))
                self.assertTrue(out.closed and err.closed)


class PostReapTeardownTests(unittest.TestCase):
    def collect(self, *, native_status=7, pre_signal="denied", lookup="absent", reap_failure=None,
                close_failure=None, close_failure_stream="stdout", collection="completed", fallback_cancelled=False,
                clock_failure=None, clock_failure_at=None):
        now = [100.0]
        metadata = {"signal": "stale-private-value"}
        events = []
        reaped = [False]
        post_close_clock_calls = [0]
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            out.write(b"x" * 20_000 if collection == "stdout-oversized" else b"200" if native_status == 0 else b"000")
            out.seek(0)
            if collection == "stderr-oversized":
                err.write(b"x" * 20_000); err.seek(0)
            stdout = mock.Mock(wraps=out)
            stderr = mock.Mock(wraps=err)
            child = mock.Mock(pid=12345, stdout=stdout, stderr=stderr)
            def close(stream, name):
                events.append(name)
                if name == f"{close_failure_stream}-close" and close_failure is not None:
                    raise close_failure
                stream.close()
            stdout.close.side_effect = lambda: close(out, "stdout-close")
            stderr.close.side_effect = lambda: close(err, "stderr-close")
            def kill_leader():
                self.assertFalse(reaped[0])
                events.append("leader-signal")
                if fallback_cancelled:
                    raise KeyboardInterrupt
            child.kill.side_effect = kill_leader
            def reap(**kwargs):
                self.assertFalse(reaped[0])
                self.assertGreaterEqual(kwargs["timeout"], 0)
                self.assertLessEqual(kwargs["timeout"], 0.25)
                events.append("reap")
                if reap_failure is not None:
                    raise reap_failure
                reaped[0] = True
                if lookup == "already-expired":
                    now[0] = 102.0
                return native_status
            child.wait.side_effect = reap
            def signal_group(pid, sig):
                self.assertEqual(pid, 12345)
                if sig != 0:
                    self.assertEqual(sig, contract.native_read.signal.SIGKILL)
                    self.assertFalse(reaped[0])
                    events.append("group-signal")
                    failure = {"denied": PermissionError("DO-NOT-PRINT"), "failed": OSError("DO-NOT-PRINT"),
                               "absent": ProcessLookupError(), "cancelled": KeyboardInterrupt()}.get(pre_signal)
                    if failure is not None:
                        raise failure
                    return
                self.assertTrue(reaped[0] and out.closed and err.closed)
                self.assertLess(now[0], 102.0)
                events.append("lookup")
                if lookup in ("absent", "late-absence"):
                    if lookup == "late-absence":
                        now[0] = 102.0
                    raise ProcessLookupError
                failure = {"denied": PermissionError("DO-NOT-PRINT"), "failed": OSError("DO-NOT-PRINT"),
                           "cancelled": KeyboardInterrupt()}.get(lookup)
                if failure is not None:
                    raise failure
            real_select = contract.native_read.select.select
            def select_streams(*args):
                if collection == "timed-out":
                    now[0] = 101.8
                    return ([], [], [])
                if collection == "cancelled":
                    raise KeyboardInterrupt
                if collection == "read-failed":
                    raise OSError("DO-NOT-PRINT")
                return real_select(*args)
            def clock():
                if reaped[0] and out.closed and err.closed:
                    post_close_clock_calls[0] += 1
                    if post_close_clock_calls[0] == clock_failure_at:
                        events.append(f"clock-failure-{clock_failure_at}")
                        raise clock_failure
                return now[0]
            with mock.patch.object(contract.native_read.subprocess, "Popen", return_value=child), \
                    mock.patch.object(contract.native_read.time, "monotonic", side_effect=clock), \
                    mock.patch.object(contract.native_read.time, "sleep", side_effect=lambda delay: now.__setitem__(0, now[0] + delay)), \
                    mock.patch.object(contract.native_read.select, "select", side_effect=select_streams), \
                    mock.patch.object(contract.native_read.os, "waitid", return_value=SimpleNamespace(
                        si_code=os.CLD_EXITED, si_status=native_status)), \
                    mock.patch.object(contract.native_read.os, "killpg", side_effect=signal_group):
                result = contract.native_read.native_poll_read(["fake"], 102, teardown_observations=metadata)
            child.wait.assert_called_once()
            child.poll.assert_not_called()
            stdout.close.assert_called_once()
            stderr.close.assert_called_once()
        self.assertNotIn("DO-NOT-PRINT", repr((result, metadata)))
        return result, metadata, events

    def test_denied_signal_then_verified_post_reap_absence_keeps_native_zero_and_seven(self) -> None:
        for status, expected_stdout in ((0, b"200"), (7, b"000")):
            with self.subTest(status=status):
                result, metadata, events = self.collect(native_status=status)
                self.assertEqual(result, ("completed", status, expected_stdout, b""))
                self.assertEqual(metadata, {"signal": "denied"})
                self.assertEqual(events, ["group-signal", "leader-signal", "reap", "stdout-close", "stderr-close", "lookup"])

    def test_present_denied_failed_cancelled_or_late_lookup_never_verifies_absence(self) -> None:
        for status in (0, 7):
            for lookup in ("present", "denied", "failed", "cancelled", "late-absence", "already-expired"):
                with self.subTest(status=status, lookup=lookup):
                    result, metadata, events = self.collect(native_status=status, lookup=lookup)
                    self.assertEqual(result, ("termination-unverified", status, b"", b""))
                    self.assertEqual(metadata, {"signal": "denied"})
                    self.assertEqual("lookup" in events, lookup != "already-expired")

    def test_reap_or_either_stream_close_uncertainty_cannot_be_cleared(self) -> None:
        for failure in (OSError("DO-NOT-PRINT"), KeyboardInterrupt(), subprocess.TimeoutExpired("fake", 0.25)):
            with self.subTest(reap_failure=type(failure)):
                result, metadata, events = self.collect(reap_failure=failure)
                self.assertEqual(result, ("termination-unverified", 7, b"", b""))
                self.assertNotIn("lookup", events)
                self.assertEqual(metadata["signal"], "denied")
        for stream in ("stdout", "stderr"):
            for failure in (OSError("DO-NOT-PRINT"), KeyboardInterrupt()):
                with self.subTest(stream=stream, close_failure=type(failure)):
                    result, _, events = self.collect(close_failure=failure, close_failure_stream=stream)
                    self.assertEqual(result, ("termination-unverified", 7, b"", b""))
                    self.assertNotIn("lookup", events)
                    self.assertIn("stdout-close", events)
                    self.assertIn("stderr-close", events)

    def test_signal_metadata_and_teardown_cancellation_stay_distinct_from_positive_absence(self) -> None:
        for signal_observation in ("sent", "absent", "denied", "failed", "cancelled"):
            with self.subTest(signal_observation=signal_observation):
                result, metadata, events = self.collect(pre_signal=signal_observation)
                self.assertEqual(metadata["signal"], signal_observation)
                self.assertEqual(result, (("termination-unverified", 7, b"", b"") if signal_observation == "cancelled"
                                          else ("completed", 7, b"000", b"")))
                self.assertIn("lookup", events)
        result, metadata, _ = self.collect(fallback_cancelled=True)
        self.assertEqual(result, ("termination-unverified", 7, b"", b""))
        self.assertEqual(metadata["signal"], "denied")

    def test_positive_absence_preserves_underlying_collection_failures_and_resets_metadata(self) -> None:
        for collection in ("timed-out", "cancelled", "read-failed", "stdout-oversized", "stderr-oversized"):
            with self.subTest(collection=collection):
                result, metadata, events = self.collect(collection=collection)
                self.assertEqual(result[:2], (collection, 7))
                self.assertEqual(metadata["signal"], "denied")
                self.assertIn("lookup", events)
        metadata = {"signal": "denied"}
        with mock.patch.object(contract.native_read.subprocess, "Popen") as launch:
            self.assertEqual(contract.native_read.native_poll_read(["fake"], time.monotonic() - 1,
                teardown_observations=metadata), ("timed-out", None, b"", b""))
        launch.assert_not_called()
        self.assertEqual(metadata, {"signal": "not-run"})

    def test_helper_and_diagnostic_metadata_is_closed_independent_and_private(self) -> None:
        path = pathlib.Path("/tmp/boxferry-docker-core.test/socket/docker.sock")
        for value in ("not-run", "sent", "absent", "denied", "failed", "cancelled", "unknown",
                      "DO-NOT-PRINT/private", "denied\n", None, ["DO-NOT-PRINT"]):
            for status in (0, 7):
                with self.subTest(value=value, status=status):
                    def collect(_arguments, _deadline, *, teardown_observations):
                        teardown_observations["signal"] = value
                        return ("completed", status, b"200" if status == 0 else b"000",
                                b"" if status == 0 else b"curl: (7) Couldn't connect to server\n")
                    with mock.patch.object(contract.native_read, "native_poll_read", side_effect=collect):
                        result = contract.readiness_ping(path, "999999999.00")
                    expected = value if isinstance(value, str) and value in {
                        "not-run", "sent", "absent", "denied", "failed", "cancelled", "unknown"} else "unknown"
                    self.assertEqual(result["ping-teardown-signal"], expected)
                    self.assertEqual(result["ping-curl-exit"], str(status))
                    self.assertEqual(result["ping-http-status"], "200" if status == 0 else "000")
                    self.assertEqual(result["ping-collector"], "completed")
                    validated = contract.readiness_ping_observations("7", "000", "unknown", "completed", value)
                    self.assertEqual(validated["ping-teardown-signal"], expected)
                    self.assertEqual(validated["ping-curl-exit"], "7")
                    self.assertNotIn("DO-NOT-PRINT", repr((result, validated)))

    def test_each_teardown_clock_boundary_retains_native_exit_and_signal_on_interruption_or_error(self) -> None:
        # Clock calls are counted only after positive reap and BOTH closes:
        # deadline creation=1, loop condition=2, lookup postcheck/remaining=3,
        # final expiry after timely absence=4. No process mutation is permitted.
        boundaries = [("group-deadline", 1, "absent"), ("while-condition", 2, "absent"),
                      ("ESRCH-postcheck", 3, "absent"), ("late-ESRCH-postcheck", 3, "late-absence"),
                      ("post-lookup-remaining", 3, "present"), ("final-expiry", 4, "absent")]
        for boundary, clock_call, lookup in boundaries:
            for native_status in (0, 7):
                for failure in (KeyboardInterrupt(), OSError("DO-NOT-PRINT/private-clock")):
                    with self.subTest(boundary=boundary, status=native_status, failure=type(failure)):
                        result, metadata, events = self.collect(native_status=native_status, lookup=lookup,
                            clock_failure=failure, clock_failure_at=clock_call)
                        self.assertEqual(result, ("termination-unverified", native_status, b"", b""))
                        self.assertEqual(metadata, {"signal": "denied"})
                        self.assertEqual(events[-1], f"clock-failure-{clock_call}")
                        self.assertEqual(events.count("reap"), 1)
                        self.assertEqual(events.count("group-signal"), 1)
                        self.assertEqual(events.count("leader-signal"), 1)
                        self.assertEqual("lookup" in events, clock_call >= 3)


class ReadinessDiagnosticTests(unittest.TestCase):
    def test_inspect_uses_native_go_id_field_not_plain_format_compatibility_alias(self) -> None:
        # Podman 6.0.2 accepts {{.Id}} but not {{json .Id}}. The native Go
        # field is ID; the controlled JSON key exposed to our parser stays id.
        self.assertEqual(contract.READINESS_INSPECT_FORMAT,
                         '{"id":{{json .ID}},"name":{{json .Name}},"labels":{{json .Config.Labels}},'
                         '"state":{{json .State.Status}},"running":{{json .State.Running}}}')

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp")
        self.addCleanup(self.temporary.cleanup)
        self.run_id = pathlib.Path(self.temporary.name).name.removeprefix("boxferry-docker-core.")
        self.outer = f"bf-docker-core-{self.run_id}"
        self.socket_path = pathlib.Path(self.temporary.name) / "socket/docker.sock"
        self.socket_path.parent.mkdir()
        self.record = {"id": "a" * 64, "name": self.outer, "labels": {
            "io.boxferry.docker-core-run": self.run_id, "private-label": "DO-NOT-PRINT/private/path"},
            "state": "exited", "running": False}

    def diagnose(self, responses, *, registered: bool = True):
        responses = list(responses)
        if len(responses) == 2:
            responses.append(("read", json.dumps(self.record).encode()))
        with mock.patch.object(contract, "readiness_read", side_effect=responses) as calls:
            result = contract.readiness_diagnostics(self.outer, self.run_id, self.socket_path,
                                                   registered=registered)
        return result, calls

    def test_successful_owned_inspect_uses_only_immutable_id_and_closed_log_observation(self) -> None:
        result, calls = self.diagnose([("read", json.dumps(self.record).encode()),
                                      ("read", b"permission denied DO-NOT-PRINT /private/path")])
        self.assertEqual(result, {"socket": "absent", "outer": "verified", "state": "exited",
                                  "logs": "permission-error-observed", "socket-owner": "unknown",
                                  "socket-mode": "unknown", "socket-lifetime": "stable",
                                  "socket-connect": "missing", "outer-recheck": "stable"})
        self.assertEqual(calls.call_args_list[0].args[0], ["podman", "inspect", "--format",
                                                        contract.READINESS_INSPECT_FORMAT, self.outer])
        self.assertEqual(calls.call_args_list[1].args[0], ["podman", "logs", "--tail", "80", "a" * 64])
        self.assertEqual(calls.call_args_list[1].kwargs, {"merge_output": True})
        self.assertEqual(calls.call_args_list[2].args[0], ["podman", "inspect", "--format",
                                                        contract.READINESS_INSPECT_FORMAT, "a" * 64])
        self.assertNotIn("DO-NOT-PRINT", json.dumps(result))
        self.assertEqual(calls.call_args_list[0].args[1], calls.call_args_list[1].args[1])

    def test_unregistered_wrong_owner_and_failed_status_with_matching_stdout_never_read_logs(self) -> None:
        result, calls = self.diagnose([], registered=False)
        self.assertEqual(result["outer"], "not-registered")
        calls.assert_not_called()
        for outcome in ("read-failed", "oversized", "timed-out"):
            with self.subTest(outcome=outcome):
                result, calls = self.diagnose([(outcome, json.dumps(self.record).encode())])
                self.assertEqual(result["outer"], outcome)
                self.assertEqual(result["logs"], "not-read")
                self.assertEqual(calls.call_count, 1)
        self.record["labels"]["io.boxferry.docker-core-run"] = "another-run"
        result, calls = self.diagnose([("read", json.dumps(self.record).encode())])
        self.assertEqual(result["outer"], "wrong-owner")
        self.assertEqual(calls.call_count, 1)

    def test_closed_inspect_parsing_rejects_injected_fields_types_names_and_duplicate_json(self) -> None:
        for mutation in (lambda row: row.update(extra="private-value"),
                         lambda row: row.update(name="ambient-other-container"),
                         lambda row: row.update(id="a" * 63), lambda row: row.update(running=1),
                         lambda row: row.update(labels=[])):
            row = copy.deepcopy(self.record)
            mutation(row)
            result, calls = self.diagnose([("read", json.dumps(row).encode())])
            self.assertEqual(result["outer"], "malformed")
            self.assertEqual(calls.call_count, 1)
        for raw in (b'{"id":1,"id":2}', b"private non-JSON error", b'{"id":NaN}'):
            result, calls = self.diagnose([("read", raw)])
            self.assertEqual(result["outer"], "malformed")
            self.assertEqual(calls.call_count, 1)

    def test_socket_and_unknown_state_categories_never_include_native_values_or_paths(self) -> None:
        self.socket_path.write_bytes(b"private-value")
        self.record["state"] = "/private-path/DO-NOT-PRINT"
        result, _ = self.diagnose([("read", json.dumps(self.record).encode()), ("read", b"DO-NOT-PRINT")])
        self.assertEqual(result, {"socket": "not-socket", "outer": "verified", "state": "unknown",
                                  "logs": "content-present", "socket-owner": "unknown", "socket-mode": "unknown",
                                  "socket-lifetime": "stable", "socket-connect": "not-socket",
                                  "outer-recheck": "stable"})
        self.record["state"] = "running"
        result, _ = self.diagnose([("read", json.dumps(self.record).encode()), ("read-failed", b"DO-NOT-PRINT")])
        self.assertEqual(result["state"], "inconsistent")
        self.assertEqual(result["logs"], "read-failed")
        with mock.patch.object(pathlib.Path, "lstat", side_effect=PermissionError("private-path")):
            result, _ = self.diagnose([], registered=False)
        self.assertEqual(result["socket"], "unavailable")
        self.assertNotIn("private", json.dumps(result))

    def test_log_observations_are_finite_and_not_causality_or_raw_content(self) -> None:
        cases = [(b"", "empty"), (b"   \n", "empty"), (b"private-value", "content-present"),
                 (b"failed to mount overlay /private", "storage-error-observed"),
                 (b"iptables failed private-key", "network-error-observed"),
                 (b"address already in use /private", "socket-error-observed"),
                 (b"failed to start daemon private", "startup-error-observed"),
                 (b"permission denied; failed to start daemon private", "multiple-errors-observed")]
        for raw, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(contract.readiness_log_category(raw), expected)

    def observe_socket(self):
        return contract.readiness_socket_observations(self.socket_path, time.monotonic() + 2)

    def test_real_pinned_socket_connects_without_sending_bytes_and_closed_metadata(self) -> None:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(self.socket_path))
            os.chmod(self.socket_path, 0o600)
            listener.listen(1)
            result = self.observe_socket()
            listener.settimeout(1)
            accepted, _ = listener.accept()
            with accepted:
                accepted.settimeout(1)
                self.assertEqual(accepted.recv(1), b"")
        self.assertEqual(result, {"socket": "socket", "socket-owner": "self", "socket-mode": "owner-only",
                                  "socket-lifetime": "stable", "socket-connect": "connected"})
        self.assertNotIn(str(self.socket_path), json.dumps(result))

    def test_real_pinned_socket_distinguishes_refused_missing_and_non_socket(self) -> None:
        result = self.observe_socket()
        self.assertEqual((result["socket"], result["socket-connect"], result["socket-lifetime"]),
                         ("absent", "missing", "stable"))
        self.socket_path.write_text("DO-NOT-PRINT/private-data")
        result = self.observe_socket()
        self.assertEqual((result["socket"], result["socket-connect"]), ("not-socket", "not-socket"))
        self.socket_path.unlink()
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(self.socket_path))
            os.chmod(self.socket_path, 0o666)
        result = self.observe_socket()
        self.assertEqual((result["socket-connect"], result["socket-mode"], result["socket-lifetime"]),
                         ("refused", "shared", "stable"))
        self.assertNotIn("DO-NOT-PRINT", json.dumps(result))

    def test_pinned_socket_replacement_never_contacts_replacement_and_invalidates_observation(self) -> None:
        real_connect = socket.socket.connect
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as original, \
                socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as replacement:
            original.bind(str(self.socket_path))
            original.listen(1)
            def replace_then_connect(probe, endpoint):
                self.assertRegex(endpoint, r"^/proc/self/fd/[0-9]+$")
                self.socket_path.unlink()
                replacement.bind(str(self.socket_path))
                replacement.listen(1)
                return real_connect(probe, endpoint)
            with mock.patch.object(socket.socket, "connect", replace_then_connect):
                result = self.observe_socket()
            original.settimeout(1)
            accepted, _ = original.accept()
            accepted.close()
            replacement.settimeout(0.02)
            with self.assertRaises(TimeoutError):
                replacement.accept()
        self.assertEqual(result["socket-lifetime"], "changed")
        self.assertEqual(result["socket-connect"], "unknown")
        self.assertEqual(result["socket-owner"], "unknown")
        self.assertEqual(result["socket-mode"], "unknown")

    def test_pinned_socket_permission_timeout_unknown_and_shared_deadline_are_closed(self) -> None:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(self.socket_path))
            for failure, expected in ((PermissionError(errno.EACCES, "DO-NOT-PRINT/private-path"), "permission-denied"),
                                      (OSError(errno.EPERM, "DO-NOT-PRINT/private-path"), "permission-denied"),
                                      (TimeoutError("DO-NOT-PRINT/private-path"), "timed-out"),
                                      (OSError(errno.EIO, "DO-NOT-PRINT/private-path"), "unknown")):
                with self.subTest(expected=expected), \
                        mock.patch.object(socket.socket, "connect", side_effect=failure), \
                        mock.patch.object(socket.socket, "settimeout") as timeout:
                    result = self.observe_socket()
                self.assertEqual(result["socket-connect"], expected)
                self.assertEqual(result["socket-lifetime"], "stable")
                self.assertGreater(timeout.call_args.args[0], 0)
                self.assertLessEqual(timeout.call_args.args[0], 1)
                self.assertNotIn("DO-NOT-PRINT", json.dumps(result))
            with mock.patch.object(contract.time, "monotonic", side_effect=[100, 100.1, 100.2]), \
                    mock.patch.object(socket.socket, "connect", side_effect=TimeoutError), \
                    mock.patch.object(socket.socket, "settimeout") as timeout:
                result = contract.readiness_socket_observations(self.socket_path, 100.25)
            self.assertAlmostEqual(timeout.call_args.args[0], 0.15)
            self.assertEqual(result["socket-connect"], "timed-out")
            with mock.patch.object(contract.os, "open") as opened:
                result = contract.readiness_socket_observations(self.socket_path, time.monotonic() - 1)
            opened.assert_not_called()
            self.assertEqual(result["socket-connect"], "timed-out")

    def test_boundary_and_ownership_failure_never_authorize_socket_connect(self) -> None:
        cases = [(self.outer, self.run_id, pathlib.Path("/ambient/socket"), True, []),
                 (self.outer, self.run_id, self.socket_path, False, []),
                 (self.outer, self.run_id, self.socket_path, True, [("read-failed", b"private")]),
                 (self.outer, self.run_id, self.socket_path, True, [("read", b"private")])]
        foreign = copy.deepcopy(self.record)
        foreign["labels"]["io.boxferry.docker-core-run"] = "other"
        cases.append((self.outer, self.run_id, self.socket_path, True, [("read", json.dumps(foreign).encode())]))
        for outer, run, path, registered, responses in cases:
            with self.subTest(registered=registered, responses=responses), \
                    mock.patch.object(contract, "readiness_read", side_effect=responses), \
                    mock.patch.object(contract, "readiness_socket_observations") as observe:
                result = contract.readiness_diagnostics(outer, run, path, registered=registered)
            observe.assert_not_called()
            self.assertEqual(result["socket-connect"], "not-checked")
            self.assertEqual(result["socket-owner"], "unknown")

    def test_outer_identity_recheck_invalidates_socket_observations_without_discarding_logs(self) -> None:
        for change, expected in ((lambda row: row.update(id="b" * 64), "changed"),
                                 (lambda row: row["labels"].update({"io.boxferry.docker-core-run": "other"}), "changed"),
                                 (lambda row: row.update(name="DO-NOT-PRINT"), "malformed")):
            current = copy.deepcopy(self.record)
            change(current)
            with self.subTest(expected=expected):
                result, _ = self.diagnose([("read", json.dumps(self.record).encode()), ("read", b""),
                                          ("read", json.dumps(current).encode())])
            self.assertEqual(result["outer-recheck"], expected)
            self.assertEqual(result["logs"], "empty")
            self.assertEqual(result["socket-connect"], "unknown")
            self.assertEqual(result["socket-lifetime"], "unknown")
            self.assertNotIn("DO-NOT-PRINT", json.dumps(result))
        for outcome in ("timed-out", "read-failed", "oversized", "termination-unverified"):
            result, _ = self.diagnose([("read", json.dumps(self.record).encode()), ("read", b""), (outcome, b"private")])
            self.assertEqual(result["outer-recheck"], outcome)
            self.assertEqual(result["socket-connect"], "unknown")

    def test_socket_unsupported_platform_and_symlink_boundary_have_no_path_fallback(self) -> None:
        with mock.patch.object(contract.sys, "platform", "not-linux"), \
                mock.patch.object(contract.os, "open") as opened:
            result = self.observe_socket()
        opened.assert_not_called()
        self.assertEqual(result["socket-connect"], "not-checked")
        self.socket_path.symlink_to("/ambient/DO-NOT-PRINT")
        with mock.patch.object(socket.socket, "connect") as connect:
            result = self.observe_socket()
        connect.assert_not_called()
        self.assertEqual(result["socket-connect"], "not-socket")
        self.assertNotIn("DO-NOT-PRINT", json.dumps(result))

    def test_directory_lifetime_changes_and_node_disappearance_invalidate_connected_result(self) -> None:
        real_connect = socket.socket.connect
        for change in ("root-mode", "directory-mode", "unlink"):
            with self.subTest(change=change), socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                self.socket_path.unlink(missing_ok=True)
                listener.bind(str(self.socket_path))
                listener.listen(1)
                def connect_then_change(probe, endpoint):
                    real_connect(probe, endpoint)
                    if change == "unlink":
                        self.socket_path.unlink()
                    else:
                        os.chmod(self.socket_path.parent.parent if change == "root-mode"
                                 else self.socket_path.parent, 0o755 if change == "root-mode" else 0o700)
                with mock.patch.object(socket.socket, "connect", connect_then_change):
                    result = self.observe_socket()
                os.chmod(self.socket_path.parent.parent, 0o700)
                os.chmod(self.socket_path.parent, 0o755)
                self.assertEqual(result["socket-lifetime"], "changed")
                self.assertEqual(result["socket-connect"], "unknown")

    def test_socket_descriptors_close_on_success_permission_and_connect_exception(self) -> None:
        real_open = os.open
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(self.socket_path))
            listener.listen(1)
            for failure in (None, PermissionError(errno.EACCES, "DO-NOT-PRINT"), KeyboardInterrupt()):
                opened = []
                def remember_open(*args, **kwargs):
                    descriptor = real_open(*args, **kwargs)
                    opened.append(descriptor)
                    return descriptor
                with self.subTest(failure=type(failure)), mock.patch.object(contract.os, "open", remember_open):
                    if failure is None:
                        self.observe_socket()
                        listener.settimeout(1)
                        accepted, _ = listener.accept()
                        accepted.close()
                    else:
                        with mock.patch.object(socket.socket, "connect", side_effect=failure):
                            if isinstance(failure, KeyboardInterrupt):
                                with self.assertRaises(KeyboardInterrupt):
                                    self.observe_socket()
                            else:
                                self.observe_socket()
                self.assertEqual(len(opened), 3)
                for descriptor in opened:
                    with self.assertRaises(OSError) as closed:
                        os.fstat(descriptor)
                    self.assertEqual(closed.exception.errno, errno.EBADF)

    def test_invalid_private_directory_and_socket_directory_symlink_prevent_inspection(self) -> None:
        os.chmod(self.socket_path.parent.parent, 0o755)
        with mock.patch.object(contract, "readiness_read") as read:
            result = contract.readiness_diagnostics(self.outer, self.run_id, self.socket_path, registered=True)
        read.assert_not_called()
        self.assertEqual(result["outer"], "invalid-boundary")
        os.chmod(self.socket_path.parent.parent, 0o700)
        self.socket_path.parent.rmdir()
        self.socket_path.parent.symlink_to("/tmp")
        with mock.patch.object(contract, "readiness_read") as read:
            result = contract.readiness_diagnostics(self.outer, self.run_id, self.socket_path, registered=True)
        read.assert_not_called()
        self.assertEqual(result["outer"], "invalid-boundary")
        self.assertEqual(result["socket-connect"], "not-checked")

    def test_registered_cli_socket_observation_keeps_native_values_and_paths_private(self) -> None:
        native = pathlib.Path(self.temporary.name) / "podman"
        native.write_text("#!/bin/sh\nif [ \"$1\" = inspect ]; then\n"
                          "printf '%s\\n' " + shlex.quote(json.dumps(self.record)) + "\n"
                          "else printf 'permission denied DO-NOT-PRINT/private-log' >&2; fi\n")
        native.chmod(0o700)
        environment = os.environ.copy()
        environment["PATH"] = str(native.parent) + os.pathsep + environment["PATH"]
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(self.socket_path))
            listener.listen(1)
            result = subprocess.run([sys.executable, str(SOURCE), "readiness-diagnostics", "--registered",
                                     "--outer", self.outer, "--run", self.run_id, "--socket", str(self.socket_path)],
                                    env=environment, capture_output=True, text=True, timeout=10, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertIn("socket-connect=connected outer-recheck=stable", result.stdout)
        self.assertIn("startup-cause=unestablished", result.stdout)
        for private in ("DO-NOT-PRINT", "a" * 64, self.outer, self.run_id, str(self.socket_path)):
            self.assertNotIn(private, result.stdout)

    def test_bounded_subprocess_output_status_combined_streams_and_timeout(self) -> None:
        cases = [("print('safe')", False, "read", b"safe\n"),
                 ("import sys; print('matching-inspect'); sys.exit(1)", False, "read-failed", b""),
                 ("import sys; sys.stdout.write('x'*16385)", False, "oversized", b""),
                 ("import sys; sys.stderr.write('private-value')", False, "read", b""),
                 ("import sys; sys.stderr.write('x'*16385)", True, "oversized", b"")]
        for source, merge, expected, raw in cases:
            with self.subTest(expected=expected, merge=merge):
                self.assertEqual(contract.readiness_read([sys.executable, "-c", source], time.monotonic() + 2,
                                                        merge_output=merge), (expected, raw))
        children = []
        original_popen = subprocess.Popen
        def launch(*args, **kwargs):
            child = original_popen(*args, **kwargs)
            children.append(child)
            self.assertTrue(kwargs["start_new_session"])
            return child
        started = time.monotonic()
        with mock.patch.object(contract.subprocess, "Popen", side_effect=launch):
            result = contract.readiness_read([sys.executable, "-c", "import time; time.sleep(60)"], started + 0.1)
        self.assertEqual(result, ("timed-out", b""))
        self.assertLess(time.monotonic() - started, 2)
        self.assertIsNotNone(children[0].poll())
        self.assertTrue(children[0].stdout.closed)

    def test_cancellation_kills_owned_reader_and_restores_diagnostic_signal_handlers(self) -> None:
        children = []
        original_popen = subprocess.Popen
        def launch(*args, **kwargs):
            child = original_popen(*args, **kwargs)
            children.append(child)
            return child
        with mock.patch.object(contract.subprocess, "Popen", side_effect=launch), \
                mock.patch.object(contract.select, "select", side_effect=KeyboardInterrupt), \
                self.assertRaises(KeyboardInterrupt):
            contract.readiness_read([sys.executable, "-c", "import time; time.sleep(60)"], time.monotonic() + 2)
        self.assertIsNotNone(children[0].poll())
        self.assertTrue(children[0].stdout.closed)
        previous = {signum: contract.signal.getsignal(signum) for signum in
                    (contract.signal.SIGTERM, contract.signal.SIGINT, contract.signal.SIGHUP)}
        result, _ = self.diagnose([KeyboardInterrupt])
        self.assertEqual(result["outer"], "cancelled")
        result, _ = self.diagnose([("read", json.dumps(self.record).encode()), KeyboardInterrupt])
        self.assertEqual(result["outer"], "verified")
        self.assertEqual(result["logs"], "cancelled")
        self.assertEqual(previous, {signum: contract.signal.getsignal(signum) for signum in previous})

    def test_kill_and_reap_uncertainty_is_closed_and_always_closes_output(self) -> None:
        for kill_error, wait_error in ((PermissionError("private-error"), None),
                                       (KeyboardInterrupt(), None),
                                       (None, subprocess.TimeoutExpired("private-command", 1)),
                                       (None, KeyboardInterrupt())):
            with self.subTest(kill_error=type(kill_error), wait_error=type(wait_error)), tempfile.TemporaryFile() as stream:
                child = mock.Mock(pid=12345, stdout=stream)
                if wait_error is not None:
                    child.wait.side_effect = wait_error
                with mock.patch.object(contract.subprocess, "Popen", return_value=child), \
                        mock.patch.object(contract.select, "select", return_value=([], [], [])), \
                        mock.patch.object(contract.os, "killpg", side_effect=kill_error), \
                        mock.patch.object(contract.time, "monotonic", return_value=100):
                    result = contract.readiness_read(["fake-read"], 100.25)
                self.assertEqual(result, ("termination-unverified", b""))
                self.assertTrue(stream.closed)
                child.wait.assert_called_once_with(timeout=0.25)
                if kill_error is not None:
                    child.kill.assert_called_once_with()

    def test_eof_success_and_failed_status_terminate_owned_group_before_first_reap(self) -> None:
        for status in (0, 1, -15):
            with self.subTest(status=status), tempfile.TemporaryFile() as stream:
                stream.write(b"bounded-private-output")
                stream.seek(0)
                events = []
                child = mock.Mock(pid=12345, stdout=stream)
                def observe(*args):
                    self.assertEqual(args, (contract.os.P_PID, 12345,
                                            contract.os.WEXITED | contract.os.WNOHANG | contract.os.WNOWAIT))
                    events.append("observe-unreaped")
                    return mock.Mock()
                def reap(**_kwargs):
                    events.append("reap")
                    return status
                child.wait.side_effect = reap
                with mock.patch.object(contract.subprocess, "Popen", return_value=child), \
                        mock.patch.object(contract.os, "waitid", side_effect=observe), \
                        mock.patch.object(contract.os, "killpg", side_effect=lambda *_args: events.append("terminate-group")):
                    result = contract.readiness_read(["fake-read"], time.monotonic() + 2)
                self.assertEqual(events, ["observe-unreaped", "terminate-group", "reap"])
                self.assertEqual(result, ("read", b"bounded-private-output") if status == 0 else ("read-failed", b""))
                self.assertTrue(stream.closed)
                child.poll.assert_not_called()

    def test_total_eight_second_budget_includes_final_read_and_reaping(self) -> None:
        now = [100.0]
        original_read = contract.readiness_read
        def first_read_then_bounded_logs(arguments, deadline, **kwargs):
            if arguments[1] == "inspect":
                self.assertEqual(deadline, 108)
                now[0] = 107.75
                return "read", json.dumps(self.record).encode()
            return original_read(arguments, deadline, **kwargs)
        with tempfile.TemporaryFile() as stream:
            child = mock.Mock(pid=12345, stdout=stream)
            with mock.patch.object(contract.time, "monotonic", side_effect=lambda: now[0]), \
                    mock.patch.object(contract, "readiness_read", side_effect=first_read_then_bounded_logs), \
                    mock.patch.object(contract.subprocess, "Popen", return_value=child), \
                    mock.patch.object(contract.select, "select", return_value=([], [], [])) as selected, \
                    mock.patch.object(contract.os, "killpg"):
                result = contract.readiness_diagnostics(self.outer, self.run_id, self.socket_path, registered=True)
            self.assertEqual(result["logs"], "timed-out")
            self.assertEqual(selected.call_args.args[3], 0.25)
            child.wait.assert_called_once_with(timeout=0.25)
            self.assertTrue(stream.closed)
        with mock.patch.object(contract.subprocess, "Popen") as launched:
            self.assertEqual(contract.readiness_read(["fake-read"], time.monotonic() - 1), ("timed-out", b""))
        launched.assert_not_called()

    def test_failed_log_status_with_recognized_stdout_and_invalid_socket_boundary_are_not_observations(self) -> None:
        result, calls = self.diagnose([("read", json.dumps(self.record).encode()),
                                      ("read-failed", b"permission denied /private")])
        self.assertEqual(result["logs"], "read-failed")
        self.assertEqual(calls.call_count, 3)
        with mock.patch.object(contract, "readiness_read") as calls, mock.patch.object(pathlib.Path, "lstat") as lstat:
            result = contract.readiness_diagnostics(self.outer, self.run_id, pathlib.Path("/ambient/socket"), registered=True)
        self.assertEqual(result["outer"], "invalid-boundary")
        calls.assert_not_called()
        lstat.assert_not_called()
        self.record["labels"]["io.boxferry.docker-core-run"] = "another-run"
        for outcome in ("read", "read-failed"):
            result, calls = self.diagnose([(outcome, json.dumps(self.record).encode())])
            self.assertEqual(result["outer"], "wrong-owner" if outcome == "read" else "read-failed")
            self.assertEqual(calls.call_count, 1)

    def test_cli_unregistered_diagnostic_is_sanitized_without_native_execution(self) -> None:
        result = subprocess.run([sys.executable, str(SOURCE), "readiness-diagnostics", "--outer", "bf-docker-core-test",
                                 "--run", "test", "--socket", str(self.socket_path)], capture_output=True,
                                text=True, timeout=3, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "readiness observations: ping-curl-exit=not-run ping-http-status=unknown "
                                       "ping-curl-error=unknown ping-collector=not-run ping-teardown-signal=not-run "
                                       "socket=unavailable outer=not-registered state=unverified "
                                       "logs=not-read socket-owner=unknown socket-mode=unknown socket-lifetime=unknown "
                                       "socket-connect=not-checked outer-recheck=not-checked; startup-cause=unestablished\n")
        self.assertEqual(result.stderr, "")

    def test_ping_observations_accept_only_closed_numeric_ranges_and_markers(self) -> None:
        for status in [str(value) for value in range(100)] + ["not-run", "unknown"]:
            self.assertEqual(contract.readiness_ping_observations(status, "000")["ping-curl-exit"], status)
        for status in [f"{value:03d}" for value in range(600)] + ["unknown"]:
            self.assertEqual(contract.readiness_ping_observations("7", status)["ping-http-status"], status)
        for invalid in ("", "-1", "100", "600", "999", "00", "7\n", " 7", "7 ", "０", "DO-NOT-PRINT"):
            with self.subTest(invalid=invalid):
                self.assertEqual(contract.readiness_ping_observations(invalid, "000")["ping-curl-exit"], "unknown")
        for invalid in ("", "-1", "0", "99", "600", "999", "000\n", " 000", "000 ", "０００", "DO-NOT-PRINT"):
            with self.subTest(invalid=invalid):
                self.assertEqual(contract.readiness_ping_observations("7", invalid)["ping-http-status"], "unknown")

    def test_cli_malformed_ping_fields_are_unknown_and_never_echoed(self) -> None:
        result = subprocess.run([sys.executable, str(SOURCE), "readiness-diagnostics", "--outer", "bf-docker-core-test",
                                 "--run", "test", "--socket", str(self.socket_path),
                                 "--ping-curl-exit=DO-NOT-PRINT/private-exit",
                                 "--ping-http-status=DO-NOT-PRINT/private-body",
                                 "--ping-teardown-signal=DO-NOT-PRINT/private-signal"],
                                capture_output=True, text=True, timeout=3, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ping-curl-exit=unknown ping-http-status=unknown", result.stdout)
        self.assertIn("ping-teardown-signal=unknown", result.stdout)
        self.assertIn("startup-cause=unestablished", result.stdout)
        self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
        self.assertEqual(result.stderr, "")


class VolumeEvidenceTests(unittest.TestCase):
    fixtures = VolumeFixtureTests.fixtures
    lane = "upstream-rootless"
    api = "1.56"
    receipt = "a" * 64
    save_manifest = VolumeFixtureTests.save_manifest
    validate = VolumeFixtureTests.validate
    apply = VolumeFixtureTests.apply
    run_id = "bf-a1b2c3d4"
    prefix = "bf-volume-a1b2c3d4"
    outer_run = "A1b2C3d4"
    source_hashes = (
        "2d7e81eeefc7060812900791db0a3a9fef08b748779f8697fef12f0cced4d5be",
        "ca714ddfe9b64620faf47c714db6be2907f7ec6428529c3e041bfd123bdd4761",
        "f22f0e4194db3b907cdb0845045339f8561ad830407acf66f6f1063fcca6f762",
        "a1ca57ad8d5342aafa2e947c9ed657b6006e89288c97d1d12166ddc86b55755d",
        "d66a018d9c3cfe804ab594ad6a1bcbe89e366ac1780422efa90ef3fca3143cbc",
        "9dd2e33343e8cf00836d783fb25d6488dcc0b958f68472dafa4b6d7b24c8a42b",
    )

    def setUp(self) -> None:
        VolumeFixtureTests.setUp(self)
        self.record = {"schema_version": 2, "boxferry_revision": "1" * 40,
                       "boxferry_source_sha256": "2" * 64, "boxferry_lock_sha256": "3" * 64,
                       "docker_lens_revision": "4" * 40, "docker_lens_source_sha256": "5" * 64,
                       "docker_lens_lock_sha256": "6" * 64, "binary_sha256": "7" * 64,
                       "override_identity": {"path": "protected-private-source-canary"}}
        self.receipt_path = self.root / "candidate-receipt.json"
        self.receipt_path.write_text(json.dumps(self.record))
        self.receipt_path.chmod(0o600)
        self.receipt = hashlib.sha256(self.receipt_path.read_bytes()).hexdigest()
        self.manifest["candidate_receipt_sha256"] = self.receipt
        self.save_manifest()
        self.created = {}
        self.state = self.apply(self.native)
        self.disposable = pathlib.Path("/tmp/boxferry-docker-core.A1b2C3d4")
        self.native_proof = {"native_script_sha256": "8" * 64,
                             "catalogue_sha256": "9" * 64, "target": self.profile}

    def native(self, _socket, method, path, body=None, **_kwargs):
        if method == "POST":
            value = json.loads(body)
            self.created[value["Name"]] = value
            return 201, json.dumps({**value, "Private": "protected-runtime-body-canary"}).encode()
        name = path.rsplit("/", 1)[-1]
        if method == "DELETE":
            self.created.pop(name)
            return 204, b"protected-delete-body-canary"
        return ((200, json.dumps({**self.created[name], "Private": "protected-runtime-body-canary"}).encode())
                if name in self.created else (404, b"protected-absent-body-canary"))

    def capture(self):
        with mock.patch.object(contract, "verify_candidate", return_value=self.record), \
             mock.patch.object(contract, "volume_native_proof", return_value=({"profiles": self.profiles}, self.native_proof)):
            return contract.capture_volume_evidence(self.output, self.root, self.root / "binary", self.receipt_path,
                self.root / "lens", "4" * 40, "8" * 64, self.state, lane=self.lane, run=self.run_id,
                prefix=self.prefix, outer_run=self.outer_run, receipt_sha256=self.receipt, api_version=self.api,
                observed_release="29.8.1", observed_api=self.api, observed_package="")

    def cleanup_proof(self):
        with mock.patch.object(contract, "volume_socket"), \
             mock.patch.object(contract, "engine_request", side_effect=self.native):
            return contract.cleanup_volume_fixtures(self.root / "socket", self.state, run=self.run_id,
                                                    prefix=self.prefix, lane=self.lane, api_version=self.api)

    def complete(self, stage, outcomes, **changes):
        options = {"candidate": stage["candidate"], "native": self.native_proof,
                   "disposable": self.disposable, "disposable_identity": {"device": 11, "inode": 12},
                   "interrupted": False, "outer_absent": True, "storage_absent": True, **changes}
        return contract.completed_volume_evidence(stage, outcomes, **options)

    def test_literal_proof_bindings_and_privacy_survive_disposable_deletion(self) -> None:
        stage = self.capture()
        self.assertNotIn("result", stage)
        self.assertEqual(set(stage), {"schema", "scope", "lane", "outer_run", "volume_run", "volume_prefix",
                                     "api_version", "candidate", "native", "observed", "manifest_sha256", "fixtures"})
        self.assertEqual((stage["schema"], stage["scope"], stage["outer_run"], stage["volume_run"], stage["volume_prefix"]),
                         (1, "volume-only", "A1b2C3d4", "bf-a1b2c3d4", "bf-volume-a1b2c3d4"))
        self.assertEqual(stage["candidate"], {"receipt_schema": 2, "receipt_sha256": self.receipt,
            "build_profile": "volume-fixtures", "boxferry_revision": "1" * 40,
            "boxferry_source_sha256": "2" * 64, "boxferry_lock_sha256": "3" * 64, "binary_sha256": "7" * 64,
            "docker_lens_revision": "4" * 40, "docker_lens_source_sha256": "5" * 64, "docker_lens_lock_sha256": "6" * 64})
        self.assertEqual([row["source_sha256"] for row in stage["fixtures"]], list(self.source_hashes))
        self.assertEqual([row["volume_count"] for row in stage["fixtures"]], [2, 3, 6, 4, 5, 3])
        for row, (identity, _owner, _suffixes) in zip(stage["fixtures"], self.fixtures, strict=True):
            self.assertEqual(row["id"], identity)
            self.assertEqual(row["artifact_sha256"], hashlib.sha256((self.output / f"{identity}-volumes.json").read_bytes()).hexdigest())
        self.assertEqual(stage["manifest_sha256"], hashlib.sha256((self.output / "manifest.json").read_bytes()).hexdigest())
        outcomes = self.cleanup_proof()
        self.assertFalse(self.created)
        # Only sanitized shell-held input/cleanup records survive producer teardown.
        for path in self.output.iterdir():
            path.unlink()
        self.output.rmdir()
        self.state.unlink()
        proof = self.complete(stage, outcomes)
        self.assertEqual(proof["result"], "checks-passed")
        expected_first = hashlib.sha256(b'{"Labels":{"io.boxferry.application":"bf-volume-a1b2c3d4-forgejo",'
            b'"io.boxferry.live-run":"bf-a1b2c3d4"},"Name":"bf-volume-a1b2c3d4-forge-db"}').hexdigest()
        self.assertEqual(proof["volumes"][0], {"identity_sha256": expected_first, "cleanup": "removed", "absence": "verified"})
        self.assertEqual(len(proof["volumes"]), 23)
        self.assertEqual(proof["closure"]["outer"], {"ownership_sha256": hashlib.sha256(
            b'{"kind":"container","name":"bf-docker-core-A1b2C3d4","run":"A1b2C3d4"}').hexdigest(), "absence": "verified"})
        encoded = json.dumps(proof)
        for value in ("protected-", "Name", "Labels", "io.boxferry", str(self.root)):
            self.assertNotIn(value, encoded)

    def test_capture_rejects_receipt_artifact_source_and_complete_ledger_tampering(self) -> None:
        original = self.state.read_bytes()
        for mutation in (lambda value: value["volumes"].pop(),
                         lambda value: value["volumes"][0]["Labels"].update(extra="protected-label-canary")):
            value = json.loads(original)
            mutation(value)
            self.state.write_text(json.dumps(value))
            with self.assertRaises(contract.ContractError):
                self.capture()
        self.state.write_bytes(original)
        artifact = self.output / "forgejo-volumes.json"
        old_artifact = artifact.read_bytes()
        artifact.write_bytes(old_artifact + b" ")
        with self.assertRaisesRegex(contract.ContractError, "artifact hash differs"):
            self.capture()
        artifact.write_bytes(old_artifact)
        source = self.root / "fixtures/conformance/forgejo-application/compose.yaml"
        old_source = source.read_bytes()
        source.write_bytes(old_source + b"\n# source drift\n")
        with self.assertRaisesRegex(contract.ContractError, "original fixture source differs"):
            self.capture()
        source.write_bytes(old_source)
        self.receipt_path.write_text("{}")
        with self.assertRaisesRegex(contract.ContractError, "receipt bytes differ"):
            self.capture()

    def test_cleanup_distinguishes_preexisting_absence_and_failure_cannot_emit_outcomes(self) -> None:
        stage = self.capture()
        self.created.pop(f"{self.prefix}-forge-db")
        proof = self.complete(stage, self.cleanup_proof())
        self.assertEqual(proof["volumes"][0]["cleanup"], "already-absent")
        with mock.patch.object(contract, "volume_socket"), \
             mock.patch.object(contract, "engine_request", return_value=(500, b"protected-native-canary")), \
             self.assertRaises(contract.ContractError):
            contract.cleanup_volume_fixtures(self.root / "socket", self.state, run=self.run_id,
                                            prefix=self.prefix, lane=self.lane, api_version=self.api)

    def test_negative_closure_interruption_or_tampering_cannot_complete(self) -> None:
        stage = self.capture()
        outcomes = self.cleanup_proof()
        for changes in ({"interrupted": True}, {"outer_absent": False}, {"storage_absent": False},
                        {"candidate": {}}, {"native": {}}, {"disposable": self.root}):
            with self.subTest(changes=changes), self.assertRaises(contract.ContractError):
                self.complete(stage, outcomes, **changes)
        for mutate in (lambda value: value[0].update(absence="unknown"), lambda value: value.pop(),
                       lambda value: value.append(value[0]), lambda value: value[0].update(identity_sha256="a" * 64),
                       lambda value: value[0].update(raw_body="protected-cleanup-canary")):
            value = copy.deepcopy(outcomes)
            mutate(value)
            with self.assertRaises(contract.ContractError):
                self.complete(stage, value)
        changed = copy.deepcopy(stage)
        changed["fixtures"][0]["source_sha256"] = "0" * 64
        with self.assertRaises(contract.ContractError):
            self.complete(changed, outcomes)

    def evidence(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        directory = pathlib.Path(temporary.name)
        # Portable offline positive fixtures model the live harness's root owner.
        real_fstat = os.fstat
        def metadata(fd):
            value = real_fstat(fd)
            return SimpleNamespace(st_dev=value.st_dev, st_ino=value.st_ino, st_mode=value.st_mode, st_uid=0,
                                   st_size=value.st_size, st_nlink=value.st_nlink)
        patcher = mock.patch.object(contract.os, "fstat", side_effect=metadata)
        patcher.start()
        self.addCleanup(patcher.stop)
        identity = contract.volume_evidence_directory(directory, self.disposable, (self.root,))
        return directory, identity

    def test_private_destination_no_overwrite_and_failure_revoke_only_original_inode(self) -> None:
        proof = self.complete(self.capture(), self.cleanup_proof())
        directory, identity = self.evidence()
        contract.write_volume_evidence(directory, self.disposable, (self.root,), identity, proof)
        output = directory / "volume-proof.json"
        self.assertEqual(json.loads(output.read_bytes()), proof)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        with self.assertRaises(contract.ContractError):
            contract.write_volume_evidence(directory, self.disposable, (self.root,), identity, proof)
        output.unlink()
        with mock.patch.object(contract.os, "fsync", side_effect=OSError("protected-write-error")), self.assertRaises(OSError):
            contract.write_volume_evidence(directory, self.disposable, (self.root,), identity, proof)
        self.assertEqual(list(directory.iterdir()), [])
        replacement = b"pre-existing-substituted-file"
        def replaced(_fd):
            output.unlink()
            output.write_bytes(replacement)
            raise OSError("protected-write-error")
        with mock.patch.object(contract.os, "fsync", side_effect=replaced), self.assertRaises(OSError):
            contract.write_volume_evidence(directory, self.disposable, (self.root,), identity, proof)
        self.assertEqual(output.read_bytes(), replacement)

    def test_destination_path_owner_mode_symlink_staleness_and_changed_identity_refused(self) -> None:
        directory, identity = self.evidence()
        for path in (directory / "missing", pathlib.Path("relative"), self.root, self.output):
            with self.subTest(path=path), self.assertRaises((contract.ContractError, OSError)):
                contract.volume_evidence_directory(path, self.disposable, (self.root,))
        linked = directory.parent / (directory.name + "-link")
        linked.symlink_to(directory)
        self.addCleanup(linked.unlink)
        with self.assertRaises(contract.ContractError):
            contract.volume_evidence_directory(linked, self.disposable, (self.root,))
        directory.chmod(0o755)
        with self.assertRaises(contract.ContractError):
            contract.volume_evidence_directory(directory, self.disposable, (self.root,))

    def test_catchable_writer_interruption_revokes_new_proof_and_preserves_replacement(self) -> None:
        proof = self.complete(self.capture(), self.cleanup_proof())
        directory, identity = self.evidence()
        handlers = {}
        def installed(signum, handler):
            previous = handlers.get(signum)
            handlers[signum] = handler
            return previous
        def interrupt(_identity):
            handlers[contract.signal.SIGTERM](contract.signal.SIGTERM, None)
        with mock.patch.object(contract.signal, "signal", side_effect=installed), self.assertRaises(contract.ContractError):
            contract.write_volume_evidence(directory, self.disposable, (self.root,), identity, proof, announce=interrupt)
        self.assertEqual(list(directory.iterdir()), [])
        token = {}
        contract.write_volume_evidence(directory, self.disposable, (self.root,), identity, proof, announce=token.update)
        original = directory / "volume-proof.json"
        # Retain the original inode so a replacement cannot reuse its number.
        held = os.open(original, os.O_RDONLY)
        try:
            original.unlink()
            original.write_text("protected-replacement-canary")
            contract.remove_volume_evidence(directory, identity, token)
            self.assertEqual(original.read_text(), "protected-replacement-canary")
        finally:
            os.close(held)
        directory.chmod(0o700)
        with self.assertRaises(contract.ContractError):
            contract.volume_evidence_directory(directory, self.disposable, (self.root,), {"device": 0, "inode": 0})
        with mock.patch.object(contract.os, "fstat", return_value=SimpleNamespace(st_uid=123, st_mode=0o40700)):
            with self.assertRaises(contract.ContractError):
                contract.volume_evidence_directory(directory, self.disposable, (self.root,))
        (directory / "stale").write_text("protected-stale-canary")
        with self.assertRaises(contract.ContractError):
            contract.volume_evidence_directory(directory, self.disposable, (self.root,))


class RunnerSafetyTests(unittest.TestCase):
    runner = (pathlib.Path(__file__).parent / "docker-application-conformance.sh").read_text(encoding="utf-8")

    def curl_statements(self) -> list[str]:
        """Collect literal call sites independently of their option spelling."""
        statements = []
        lines = self.runner.splitlines()
        for index, line in enumerate(lines):
            if "curl " not in line or line.strip().startswith("for tool in "):
                continue
            statement = line.strip()
            while statement.endswith("\\"):
                index += 1
                statement = statement[:-1] + lines[index].strip()
            statements.append(statement)
        return statements

    def function(self, name: str) -> str:
        start = self.runner.index(f"{name}() {{")
        end = self.runner.index("\n}\n", start) + 3
        return self.runner[start:end]

    def bash(self, body: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(["bash", "-c", "set -Eeuo pipefail\ncontract=/unused\nprofile=replay-probe\n" +
                               self.function("podman_presence") + body],
                              capture_output=True, text=True, timeout=8, check=False)

    def test_timeout_wrapper_ends_a_stalled_command_and_has_kill_after(self) -> None:
        wrapper = self.function("bounded")
        self.assertIn("--kill-after=5s", wrapper)
        started = time.monotonic()
        result = self.bash(wrapper + "\nbounded 0.1s sleep 3\n")
        self.assertEqual(result.returncode, 124)
        self.assertLess(time.monotonic() - started, 2)

    def test_volume_evidence_handoff_requires_all_gates_and_revokes_failed_or_interrupted_write(self) -> None:
        for status, interrupted in ((0, False), (1, False), (0, True)):
            body = """cleanup_interrupted=false
registered=true
volume_input_proof='input-proof'
volume_cleanup_proof='cleanup-proof'
evidence_directory=/unused
evidence_identity='private-directory-token'
run_dir=/unused
boxferry_root=/unused
boxferry_binary=/unused
boxferry_receipt=/unused
lens_root=/unused
lens_revision=unused
script_sha=unused
trap 'cleanup_interrupted=true' HUP INT TERM
""" + self.function("finish_volume_evidence") + f"""
bounded() {{
  [[ $1 == 30s && $4 == write-volume-evidence ]] || return 99
  printf '{{"device":1,"inode":2}}'
  {('kill -HUP "$$"' if interrupted else ':')}
  return {status}
}}
cleanup_bounded() {{
  [[ $1 == 5s && $4 == remove-volume-evidence ]] || return 99
  printf 'revoked\\n' >&2
}}
finish_volume_evidence
"""
            result = self.bash(body)
            with self.subTest(status=status, interrupted=interrupted):
                self.assertEqual(result.returncode, 0 if status == 0 and not interrupted else 1, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual("revoked" in result.stderr, status != 0 or interrupted)
        body = """cleanup_interrupted=true
registered=true
volume_input_proof='input'
volume_cleanup_proof='cleanup'
bounded() { printf 'must-not-run'; return 99; }
""" + self.function("finish_volume_evidence") + "\nfinish_volume_evidence\n"
        result = self.bash(body)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")

    def test_evidence_option_is_required_only_for_volume_profile_before_any_tool_call(self) -> None:
        script = SOURCE.parent.parent / "docker-application-conformance.sh"
        for profile in ("catalogue", "replay-probe", "core-journey", "volume-fixtures"):
            command = ["bash", str(script), "--profile", profile, "--docker-lens-root", "/protected-private-canary",
                       "--docker-lens-revision", "1" * 40, "--native-script-sha256", "2" * 64]
            if profile != "volume-fixtures":
                command.extend(["--evidence-directory", "/protected-destination-canary"])
            result = subprocess.run(command, capture_output=True, text=True, timeout=3, check=False)
            with self.subTest(profile=profile):
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertNotIn("protected-", result.stderr)

    def test_interrupt_at_final_trap_reset_after_successful_finalizer_return_revokes(self) -> None:
        start = self.runner.rindex("if [[ $profile == volume-fixtures ]]; then")
        stop = self.runner.index("\nfi\n", start) + len("\nfi\n")
        final_block = self.runner[start:stop]
        for interrupted in (False, True):
            with tempfile.TemporaryDirectory() as name:
                proof = pathlib.Path(name) / "proof-created"
                body = f"""profile=volume-fixtures
lane=upstream-rootless
registered=true
cleanup_interrupted=false
volume_input_proof=input-proof
volume_cleanup_proof=cleanup-proof
evidence_directory=unused
evidence_identity=directory-token
run_dir=unused
boxferry_root=unused
boxferry_binary=unused
boxferry_receipt=unused
lens_root=unused
lens_revision=unused
script_sha=unused
report_host_cache() {{ :; }}
trap 'cleanup_interrupted=true' HUP INT TERM
bounded() {{
  [[ $1 == 30s && $4 == write-volume-evidence ]] || return 99
  printf 'created' > {shlex.quote(str(proof))}
  printf '{{"device":1,"inode":2}}'
}}
cleanup_bounded() {{
  [[ $1 == 5s && $4 == remove-volume-evidence ]] || return 99
  [[ ${{*: -1}} == '{{"device":1,"inode":2}}' ]] || return 98
  rm -- {shlex.quote(str(proof))}
  printf 'revoked\\n' >&2
}}
""" + self.function("finish_volume_evidence")
                if interrupted:
                    # DEBUG is not inherited by the function. This hook is reached
                    # after successful finalizer return but BEFORE the caller's
                    # handler reset, closing the last catchable-command boundary.
                    body += """
injected=false
late_interrupt() {
  if [[ $injected == false && $1 == 'trap - EXIT HUP INT TERM' ]]; then
    injected=true
    kill -HUP "$$"
  fi
}
trap 'late_interrupt "$BASH_COMMAND"' DEBUG
"""
                result = self.bash(body + "\n" + final_block)
                with self.subTest(interrupted=interrupted):
                    self.assertEqual(result.returncode, 1 if interrupted else 0, result.stderr)
                    self.assertEqual(proof.exists(), not interrupted)
                    self.assertEqual("CHECKS-PASSED" in result.stdout, not interrupted)
                    self.assertEqual("revoked" in result.stderr, interrupted)

    def test_presence_protocol_requires_exact_marker_and_matching_wrapper_status(self) -> None:
        cases = [(0, "present\n", 0), (1, "absent\n", 1), (2, "unknown\n", 2),
                 (1, "", 2), (0, "absent\n", 2), (1, "present\n", 2),
                 (124, "absent\n", 2), (125, "present\n", 2), (130, "absent\n", 2),
                 (1, "absent\n\n", 2), (1, "absent", 2), (0, "present\nDO-NOT-PRINT", 2)]
        for status, marker, expected in cases:
            with self.subTest(status=status, marker=marker):
                body = f"""run_id=test
presence_unverified=false
bounded() {{ printf '%s' {shlex.quote(marker)}; return {status}; }}
state=0
podman_presence bounded container bf-docker-core-test || state=$?
printf 'state=%s unverified=%s\\n' "$state" "$presence_unverified"
"""
                result = self.bash(body)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, f"state={expected} unverified={'true' if expected == 2 else 'false'}\n")
                self.assertNotIn("DO-NOT-PRINT", result.stderr)

    def test_unknown_preflight_retains_evidence_without_registering_or_mutating_resources(self) -> None:
        start = self.runner.index("container_state=0\n")
        preflight = self.runner[start:self.runner.index("registered=true\n", start)]
        for kind in ("container", "volume"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                directory = pathlib.Path(name)
                evidence = directory / "private-evidence"
                evidence.write_bytes(b"DO-NOT-PRINT")
                body = f"""run_id=test
run_dir={shlex.quote(name)}
registered=false
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
watchdog_pid=
contract=/unused
cleanup_now() {{ printf '1\\n'; }}
bounded() {{
  if [[ $6 == {kind} ]]; then printf 'unknown\\n'; return 2; fi
  printf 'absent\\n'; return 1
}}
cleanup_bounded() {{ shift; "$@"; }}
report_host_cache() {{ :; }}
""" + self.function("cleanup_owned") + self.function("on_exit") + "trap on_exit EXIT\n" + preflight
                result = self.bash(body)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(evidence.read_bytes(), b"DO-NOT-PRINT")
                self.assertIn(f"private-directory={name}", result.stderr)
                self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)

    def test_initial_and_post_removal_unknown_retain_evidence_for_each_resource(self) -> None:
        for kind in ("container", "volume"):
            for phase in ("initial", "post-removal"):
                with self.subTest(kind=kind, phase=phase), \
                        tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                    directory = pathlib.Path(name)
                    evidence = directory / "private-evidence"
                    evidence.write_bytes(b"DO-NOT-PRINT")
                    (directory / "busybox.tar").write_bytes(b"owned archive")
                    body = f"""registered=true
run_id=test
run_dir={shlex.quote(name)}
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
watchdog_pid=
socket_path=
removed=false
cleanup_now() {{ printf '1\\n'; }}
cleanup_bounded() {{
  if [[ $2 == python3 && $4 == podman-presence ]]; then
    if [[ $6 != {kind} ]]; then printf 'absent\\n'; return 1; fi
    if [[ {phase} == initial || $removed == true ]]; then printf 'unknown\\n'; return 2; fi
    printf 'present\\n'; return 0
  fi
  if [[ $2 == podman && ( $3 == inspect || $4 == inspect ) ]]; then printf 'test\\n'; return 0; fi
  if [[ $2 == podman ]]; then removed=true; return 0; fi
  shift; "$@"
}}
""" + self.function("cleanup_owned") + "state=0\ncleanup_owned || state=$?\nprintf 'state=%s removed=%s\\n' \"$state\" \"$removed\"\n"
                    result = self.bash(body)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, f"state=1 removed={'true' if phase == 'post-removal' else 'false'}\n")
                    self.assertEqual(evidence.read_bytes(), b"DO-NOT-PRINT")
                    self.assertFalse((directory / "busybox.tar").exists())
                    self.assertIn(f"private-directory={name}", result.stderr)
                    self.assertIn("presence unverified", result.stderr)
                    self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)

    def test_cleanup_success_preserves_original_failure_status(self) -> None:
        for cleanup_status, expected in ((0, 42), (1, 1)):
            body = f"""run_dir=
cleanup_owned() {{ return {cleanup_status}; }}
report_host_cache() {{ :; }}
""" + self.function("on_exit") + "trap on_exit EXIT\nexit 42\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, expected, result.stderr)
            self.assertIn("Docker core harness failed", result.stderr)

    def test_real_presence_helper_rejects_native_diagnostics_through_cleanup(self) -> None:
        for kind in ("container", "volume"):
            for phase in ("initial", "post-removal"):
                for native_status in (0, 1):
                    with self.subTest(kind=kind, phase=phase, native_status=native_status), \
                            tempfile.TemporaryDirectory(prefix="boxferry-presence-fake-") as fake_name, \
                            tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                        directory = pathlib.Path(name)
                        evidence = directory / "private-evidence"
                        evidence.write_bytes(b"DO-NOT-PRINT/evidence")
                        fake = pathlib.Path(fake_name)
                        removed = fake / "removed"
                        native = fake / "podman"
                        native.write_text(f"""#!/bin/sh
if [ "$2" = exists ]; then
  if [ "$1" != {kind} ]; then exit 1; fi
  if [ {phase} = post-removal ] && [ ! -e {shlex.quote(str(removed))} ]; then exit 0; fi
  printf 'DO-NOT-PRINT/native-diagnostic' >&2
  exit {native_status}
fi
if [ "$1" = inspect ] || [ "$2" = inspect ]; then printf 'test\\n'; exit 0; fi
if [ "$1" = rm ] || [ "$2" = rm ]; then touch {shlex.quote(str(removed))}; exit 0; fi
exit 125
""", encoding="ascii")
                        native.chmod(0o700)
                        body = f"""export PATH={shlex.quote(fake_name)}:"$PATH"
contract={shlex.quote(str(SOURCE))}
registered=true
run_id=test
run_dir={shlex.quote(name)}
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
watchdog_pid=
socket_path=
cleanup_now() {{ printf '1\\n'; }}
""" + self.function("cleanup_bounded") + self.function("cleanup_owned") + "cleanup_owned\n"
                        result = self.bash(body)
                        self.assertEqual(result.returncode, 1, result.stderr)
                        self.assertEqual(evidence.read_bytes(), b"DO-NOT-PRINT/evidence")
                        self.assertEqual(removed.exists(), phase == "post-removal", result.stderr)
                        self.assertIn("presence unverified", result.stderr)
                        self.assertIn(f"private-directory={name}", result.stderr)
                        self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
    def test_readiness_failure_diagnostics_precede_teardown_for_core_and_volume_without_repair(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        self.assertEqual(self.runner.count("readiness_failure 'nested Docker daemon did not become ready'"), 1)
        for profile in ("core-journey", "volume-fixtures"):
            for reason in ("timeout", "exited"):
                for diagnostic_status in (0, 124, 130):
                    with self.subTest(profile=profile, reason=reason, diagnostic_status=diagnostic_status), \
                            tempfile.TemporaryDirectory(prefix="boxferry-readiness-fake-") as name:
                        sentinel = pathlib.Path(name) / "fake-socket"
                        sentinel.write_bytes(b"")
                        body = f"""profile={profile}
registered=true
outer=bf-docker-core-test
run_id=test
socket_path={shlex.quote(str(sentinel))}
contract=unused
SECONDS=0
cleanup_owned() {{ printf 'exact-owned-teardown\\n' >&2; }}
trap cleanup_owned EXIT
curl() {{
  return 1
}}
bounded() {{
  if [[ $2 == podman ]]; then printf 'false\\n'; return 0; fi
  if [[ $4 == readiness-ping ]]; then printf '7 000 unknown completed unknown\\n'; return 0; fi
  [[ $1 == 12s && $2 == python3 && $4 == readiness-diagnostics && $5 == --registered ]] || return 99
  if (({diagnostic_status} == 0)); then
    printf 'readiness observations: outer=verified state=exited logs=empty; startup-cause=unestablished\\n'
  else
    printf 'DO-NOT-PRINT /private-error-path\\n' >&2
  fi
  return {diagnostic_status}
}}
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path").replace(
                            "deadline=$((SECONDS + 180))", "deadline=$((SECONDS - 1))" if reason == "timeout" else
                            "deadline=$((SECONDS + 180))")
                        result = self.bash(body)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    message = "nested Docker daemon did not become ready" if reason == "timeout" else \
                              "outer Docker daemon exited before readiness"
                    self.assertIn(message, result.stderr)
                    self.assertLess(result.stderr.index(message), result.stderr.index("readiness observations:"))
                    self.assertLess(result.stderr.index("readiness observations:"), result.stderr.index("exact-owned-teardown"))
                    self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
                    self.assertNotIn("private-error-path", result.stdout + result.stderr)
                    self.assertNotIn("CHECKS-PASSED", result.stdout + result.stderr)
                    self.assertEqual(result.stderr.count("readiness observations:"), 1)
    def test_failed_ping_poll_records_only_existing_curl_exit_and_fixed_http_status(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        cases = (("connect-failure", 7, "000", True, "7", "000"),
                 ("curl-timeout", 28, "000", True, "28", "000"),
                 ("http-error", 22, "503", True, "22", "503"),
                 ("unexpected-exit", 99, "000", True, "99", "000"),
                 ("out-of-range-exit", 100, "000", True, "unknown", "000"),
                 ("malformed-http", 7, "DO-NOT-PRINT/private-http", True, "7", "unknown"),
                 ("http-newline", 7, "000\n", True, "7", "unknown"),
                 ("missing-socket", 7, "000", False, "not-run", "unknown"))
        for profile in ("core-journey", "volume-fixtures"):
            for reason, status, http, socket_exists, expected_exit, expected_http in cases:
                with self.subTest(profile=profile, reason=reason), \
                        tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                    directory = pathlib.Path(name)
                    socket_path = directory / "socket/docker.sock"
                    socket_path.parent.mkdir()
                    if socket_exists:
                        socket_path.write_bytes(b"")
                    count_path = directory / "curl-count"
                    curl_path = directory / "curl"
                    expected_args = ["-q", "--noproxy", "*", "--fail", "--silent", "--show-error", "--max-time", "5", "--max-filesize", "65536",
                                     "--output", "/dev/null", "--write-out", "%{http_code}", "--unix-socket",
                                     str(socket_path), "http://localhost/_ping"]
                    curl_path.write_text(f"""#!{sys.executable}
import pathlib, sys
if sys.argv[1:] != {expected_args!r}:
    sys.exit(98)
with pathlib.Path({str(count_path)!r}).open('a') as output:
    output.write('poll\\n')
with open(sys.argv[sys.argv.index('--output') + 1], 'w') as output:
    output.write('DO-NOT-PRINT/private-response-body')
sys.stderr.write('DO-NOT-PRINT/private-curl-error')
sys.stdout.write({http!r})
sys.exit({status})
""", encoding="utf-8")
                    curl_path.chmod(0o700)
                    native = directory / "podman"
                    native.write_text("#!/bin/sh\nprintf 'DO-NOT-PRINT/private-native-error' >&2\nexit 1\n",
                                      encoding="utf-8")
                    native.chmod(0o700)
                    body = f"""PATH={shlex.quote(name)}:"$PATH"
profile={profile}
registered=true
outer=bf-docker-core-{directory.name.removeprefix('boxferry-docker-core.')}
run_id={shlex.quote(directory.name.removeprefix('boxferry-docker-core.'))}
socket_path={shlex.quote(str(socket_path))}
contract={shlex.quote(str(SOURCE))}
cleanup_owned() {{ printf 'exact-owned-teardown\\n' >&2; }}
trap cleanup_owned EXIT
bounded() {{
  if [[ $2 == podman ]]; then printf 'false\\n'; return 0; fi
  shift; "$@"
}}
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path") + \
                        "\nprintf 'DO-NOT-APPLY\\n'\n"
                    result = self.bash(body)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertEqual(count_path.read_text() if count_path.exists() else "",
                                     "poll\n" if socket_exists else "")
                    self.assertIn(f"ping-curl-exit={expected_exit} ping-http-status={expected_http}", result.stderr)
                    self.assertIn("startup-cause=unestablished", result.stderr)
                    self.assertLess(result.stderr.index("readiness observations:"), result.stderr.index("exact-owned-teardown"))
                    self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
                    self.assertNotIn("DO-NOT-APPLY", result.stdout + result.stderr)
                    self.assertNotIn(str(socket_path), result.stdout + result.stderr)

    def test_readiness_failure_sanitizes_fields_even_when_diagnostics_fail(self) -> None:
        for registered in ("true", "false"):
            with self.subTest(registered=registered):
                body = f"""registered={registered}
outer=bf-docker-core-test
run_id=test
socket_path=/unused
ping_curl_exit=DO-NOT-PRINT/private-exit
ping_http_status=DO-NOT-PRINT/private-http
bounded() {{
  [[ ${{12}} == --ping-curl-exit=unknown && ${{13}} == --ping-http-status=unknown ]] || {{ printf 'BAD-ARGS\\n'; return 99; }}
  return 124
}}
""" + self.function("readiness_failure") + "\nreadiness_failure 'readiness failed'\n"
                result = self.bash(body)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("ping-curl-exit=unknown ping-http-status=unknown", result.stderr)
                self.assertIn("startup-cause=unestablished", result.stderr)
                self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
                self.assertNotIn("BAD-ARGS", result.stdout + result.stderr)

    def test_readiness_keeps_final_poll_and_cadence_without_extra_request(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        for final_exit in (0, 22):
            with self.subTest(final_exit=final_exit), tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                count_path = pathlib.Path(name) / "count"
                socket_path = pathlib.Path(name) / "socket/docker.sock"
                socket_path.parent.mkdir()
                socket_path.touch()
                curl_path = pathlib.Path(name) / "curl"
                curl_path.write_text(f"""#!/bin/sh
if [ ! -e {shlex.quote(str(count_path))} ]; then
  printf 'first\\n' > {shlex.quote(str(count_path))}; printf '000'; exit 7
fi
printf 'second\\n' >> {shlex.quote(str(count_path))}
printf '503'; exit {final_exit}
""")
                curl_path.chmod(0o700)
                body = f"""PATH={shlex.quote(name)}:"$PATH"
registered=false
outer=bf-docker-core-test
socket_path={shlex.quote(str(socket_path))}
contract={shlex.quote(str(SOURCE))}
count_path={shlex.quote(str(count_path))}
bounded() {{
  if [[ $2 == podman ]]; then
    [[ $(wc -l < "$count_path") == 1 ]] && printf 'true\\n' || printf 'false\\n'
  else shift; "$@"; fi
}}
sleep() {{ [[ $1 == 2 ]] || return 99; printf 'cadence=2\\n'; }}
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path") + \
                    "\nprintf 'ready\\n'\n"
                result = self.bash(body)
                self.assertEqual(result.returncode, 0 if final_exit == 0 else 1, result.stderr)
                self.assertEqual(count_path.read_text(), "first\nsecond\n", result.stderr)
                self.assertEqual(result.stdout, "cadence=2\nready\n" if final_exit == 0 else "cadence=2\n")
                if final_exit == 0:
                    self.assertEqual(result.stderr, "")
                else:
                    self.assertIn("ping-curl-exit=22 ping-http-status=503", result.stderr)

    def test_readiness_exhausted_wrapper_reserve_preserves_last_actual_poll_atomically(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        for missing in (False, True):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as name:
                socket_path = pathlib.Path(name) / "socket"
                socket_path.touch()
                body = f"""registered=false
socket_path={shlex.quote(str(socket_path))}
outer=bf-docker-core-test
contract=unused
polls=0
read() {{
  case "$2" in
    ping_readiness_uptime) ping_readiness_uptime=100.50 ;;
    ping_poll_uptime) ping_poll_uptime=100.50; ((polls == 0)) || ping_poll_uptime=275.50 ;;
    ping_handoff_uptime) ping_handoff_uptime=100.50 ;;
    *) builtin read "$@" ;;
  esac
}}
bounded() {{
  if [[ $2 == podman ]]; then printf 'true\\n'; return 0; fi
  [[ $1 == 5s && $4 == readiness-ping && $7 == --deadline-boottime && $8 == 280.50 ]] || return 99
  printf '7 000 connect-error-observed completed denied\\n'
}}
sleep() {{
  [[ $1 == 2 ]] || return 99
  polls=1
  if [[ {str(missing).lower()} == true ]]; then rm -f "$socket_path"; SECONDS=$deadline; fi
}}
cleanup_owned() {{ printf 'exact-owned-teardown\\n' >&2; }}
trap cleanup_owned EXIT
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path")
                result = self.bash(body)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("ping-curl-exit=7 ping-http-status=000 ping-curl-error=connect-error-observed ping-collector=completed",
                          result.stderr)
            self.assertLess(result.stderr.index("readiness observations:"), result.stderr.index("exact-owned-teardown"))

    def test_readiness_poll_wrapper_clips_native_wait_and_kill_reserve_inside_absolute_deadline(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        for now, expected in (("100.50", "5s"), ("271.90", "3s"), ("274.90", None)):
            with self.subTest(now=now), tempfile.TemporaryDirectory() as name:
                socket_path = pathlib.Path(name) / "socket"
                socket_path.touch()
                body = f"""registered=false
socket_path={shlex.quote(str(socket_path))}
outer=bf-docker-core-test
contract=unused
read() {{
  case "$2" in
    ping_readiness_uptime) ping_readiness_uptime=100.50 ;;
    ping_poll_uptime) ping_poll_uptime={now} ;;
    ping_handoff_uptime) ping_handoff_uptime={now} ;;
    *) builtin read "$@" ;;
  esac
}}
bounded() {{
  [[ $1 == {expected or 'unused'} ]] || return 99
  printf '0 200 unknown completed unknown\\n'
}}
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path") + \
                    '\nprintf "accepted=%s\\n" "$ping_collector"\n'
                result = self.bash(body)
            if expected is None:
                self.assertEqual(result.returncode, 1)
                self.assertIn("ping-curl-exit=not-run", result.stderr)
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")
                self.assertEqual(result.stdout, "accepted=completed\n")

    def test_readiness_accepts_native_zero_only_after_verified_collection(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        cases = [("0 200 unknown completed", True), ("0 200 unknown oversized", True),
                 ("0 unknown unknown oversized", False), ("0 unknown unknown completed", False),
                 ("0 unknown unknown invalid-output", False)]
        cases.extend((f"0 unknown unknown {outcome}", False) for outcome in
                     ("timed-out", "cancelled", "termination-unverified", "wrapper-failed", "unknown"))
        cases.extend((("0 200 unknown completed\nDO-NOT-PRINT/private", False),
                      ("0 200 DO-NOT-PRINT/private completed", False)))
        for record, accepted in cases:
            record += " unknown"
            with self.subTest(record=record), tempfile.TemporaryDirectory() as name:
                socket_path = pathlib.Path(name) / "socket"
                socket_path.touch()
                body = f"""registered=false
socket_path={shlex.quote(str(socket_path))}
outer=bf-docker-core-test
contract=unused
bounded() {{
  if [[ $2 == podman ]]; then printf 'false\\n'; else printf '%s\\n' {shlex.quote(record)}; fi
}}
sleep() {{ return 99; }}
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path") + \
                    '\nprintf "accepted\\n"\n'
                result = self.bash(body)
                self.assertEqual(result.returncode, 0 if accepted else 1, result.stderr)
                self.assertEqual(result.stdout, "accepted\n" if accepted else "")
                self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
                if not accepted:
                    self.assertIn("startup-cause=unestablished", result.stderr)

    def test_readiness_rechecks_absolute_handoff_and_keeps_wrapper_uncertain_native_fields(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        cases = [("280.49", "0 200 unknown completed", 0, True, "completed"),
                 ("280.50", "0 200 unknown completed", 0, False, "timed-out"),
                 ("280.51", "0 200 unknown completed", 0, False, "timed-out"),
                 ("invalid", "0 200 unknown completed", 0, False, "wrapper-failed"),
                 ("100.50", "7 000 connect-error-observed completed", 124, False, "wrapper-failed"),
                 ("100.50", "0 200 unknown completed", 124, False, "wrapper-failed")]
        for handoff, record, wrapper_status, accepted, collector in cases:
            record += " denied"
            with self.subTest(handoff=handoff, wrapper_status=wrapper_status, record=record), \
                    tempfile.TemporaryDirectory() as name:
                socket_path = pathlib.Path(name) / "socket"
                socket_path.touch()
                count_path = pathlib.Path(name) / "polls"
                body = f"""registered=false
socket_path={shlex.quote(str(socket_path))}
outer=bf-docker-core-test
contract=unused
read() {{
  case "$2" in
    ping_readiness_uptime) ping_readiness_uptime=100.50 ;;
    ping_poll_uptime) ping_poll_uptime=100.50 ;;
    ping_handoff_uptime) ping_handoff_uptime={handoff} ;;
    *) builtin read "$@" ;;
  esac
}}
bounded() {{
  if [[ $2 == podman ]]; then printf 'false\\n'; return 0; fi
  printf 'original-poll\\n' >> {shlex.quote(str(count_path))}
  printf '%s\\n' {shlex.quote(record)}
  return {wrapper_status}
}}
sleep() {{ return 99; }}
cleanup_owned() {{ printf 'exact-owned-teardown\\n' >&2; }}
trap cleanup_owned EXIT
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path") + \
                    '\nprintf "accepted\\n"\n'
                result = self.bash(body)
                self.assertEqual(result.returncode, 0 if accepted else 1, result.stderr)
                self.assertEqual(result.stdout, "accepted\n" if accepted else "")
                self.assertEqual(count_path.read_text(), "original-poll\n")
                if not accepted:
                    native_exit, http_status, error, _, teardown_signal = record.split()
                    self.assertIn(f"ping-curl-exit={native_exit} ping-http-status={http_status} "
                                  f"ping-curl-error={error} ping-collector={collector} "
                                  f"ping-teardown-signal={teardown_signal}", result.stderr)
                    self.assertLess(result.stderr.index("readiness observations:"),
                                    result.stderr.index("exact-owned-teardown"))

    def test_readiness_signal_field_is_independent_private_and_preserved_in_failure_fallback(self) -> None:
        start = self.runner.index("read -r ping_readiness_uptime _ < /proc/uptime")
        readiness = self.runner[start:self.runner.index('\nchmod 0666 "$socket_path"', start)]
        cases = [("7 000 connect-error-observed completed denied", 124, "7", "wrapper-failed", "denied"),
                 ("0 200 unknown completed DO-NOT-PRINT/private", 0, "0", "unknown", "unknown"),
                 ("DO-NOT-PRINT/private unknown unknown unknown denied", 0, "unknown", "unknown", "denied"),
                 ("7 000 unknown completed", 0, "unknown", "unknown", "unknown"),
                 ("7 000 unknown completed denied extra", 0, "unknown", "unknown", "unknown")]
        for registered in (False, True):
            for record, wrapper_status, native_exit, collector, signal_observation in cases:
                with self.subTest(registered=registered, record=record), tempfile.TemporaryDirectory() as name:
                    socket_path = pathlib.Path(name) / "socket"
                    socket_path.touch()
                    body = f"""registered={str(registered).lower()}
socket_path={shlex.quote(str(socket_path))}
outer=bf-docker-core-test
run_id=test
contract=unused
bounded() {{
  if [[ $2 == podman ]]; then printf 'false\\n'; return 0; fi
  if [[ $4 == readiness-diagnostics ]]; then return 124; fi
  printf '%s\\n' {shlex.quote(record)}
  return {wrapper_status}
}}
sleep() {{ return 99; }}
""" + self.function("readiness_failure") + readiness.replace("-S $socket_path", "-f $socket_path")
                    result = self.bash(body)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertIn(f"ping-curl-exit={native_exit}", result.stderr)
                    self.assertIn(f"ping-collector={collector} ping-teardown-signal={signal_observation}", result.stderr)
                    self.assertIn("startup-cause=unestablished", result.stderr)
                    self.assertNotIn("DO-NOT-PRINT", result.stdout + result.stderr)
                    if registered:
                        self.assertIn("diagnostic=unavailable", result.stderr)

    def test_core_journey_uses_exact_id_and_preserves_transport_only_result(self) -> None:
        self.assertIn('verify-candidate --boxferry-root "$boxferry_root"', self.runner)
        self.assertIn('render-core-source', self.runner)
        self.assertIn('capture-candidate', self.runner)
        self.assertIn('boxferry_snapshot="$run_dir/boxferry-candidate"', self.runner)
        self.assertNotIn('"$boxferry_binary" convert', self.runner)
        self.assertIn('"$boxferry_snapshot" convert compose docker', self.runner)
        self.assertIn('"$boxferry_snapshot" convert docker compose', self.runner)
        self.assertIn('--docker-import-policy portable', self.runner)
        self.assertIn('--promote-docker-protected-environment-values', self.runner)
        self.assertIn('--environment-values include', self.runner)
        self.assertIn('--docker-resource-decision "network:${network_id}=external:bridge"', self.runner)
        self.assertRegex(self.runner, r'2>[ \t]*"\$run_dir/compose-console\.txt"')
        self.assertIn('check-core-output --kind compose-console', self.runner)
        self.assertIn('check-core-output --kind compose-report', self.runner)
        self.assertIn('--socket-path "$socket_path"', self.runner)
        self.assertIn('convert compose docker', self.runner)
        self.assertIn('--docker-target "$docker_preset"', self.runner)
        self.assertIn('validate-artifact --artifact "$artifact"', self.runner)
        self.assertIn('replay-test-only --allow-isolated-apply', self.runner)
        self.assertIn('inspect --format \'{{json .}}\' "$container_id"', self.runner)
        self.assertIn('--docker-container-id "$container_id"', self.runner)
        self.assertIn('--network-file "$run_dir/native-network-inspect.json"', self.runner)
        self.assertIn('check-core-output --kind compose', self.runner)
        self.assertIn('CORE-JOURNEY-SCAFFOLD CHECKS-PASSED:', self.runner)
        self.assertIn('TRANSPORT-ONLY PASS:', self.runner)

    def test_native_baseline_keeps_platform_and_id_selected_network_reads(self) -> None:
        self.assertIn(
            "podman image inspect --format '{{.Os}}/{{.Architecture}}' \"$fixture_image\") == linux/amd64",
            self.runner,
        )
        self.assertIn(
            "image inspect --format '{{.Os}}/{{.Architecture}}' \"$fixture_tag\") == linux/amd64",
            self.runner,
        )
        self.assertIn('network inspect --format \'{{json .}}\' "$network_id"', self.runner)
        self.assertIn('check-core-output --kind network-inspect', self.runner)
        self.assertIn('--container-file "$run_dir/native-inspect.json"', self.runner)
        self.assertIn('--network-id "$network_id"', self.runner)

    def test_watchdog_scan_timeout_signals_main_even_if_du_stalls(self) -> None:
        body = """main_pid=424242
main_start=123
contract=/unused
baseline_free=9999999
volume_path=/unused
run_dir=/unused
sleep() { :; }
bounded() { return 124; }
python3() { if [[ $2 == signal-parent ]]; then printf 'signaled original parent\\n'; fi; }
signal_main() { printf 'signaled original parent\\n'; }
watchdog_parent_alive() { return 0; }
watchdog_abort() { signal_main; return 1; }
""" + self.function("watchdog") + "\nwatchdog\n"
        result = self.bash(body)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("signaled original parent", result.stdout)
        self.assertIn('bounded 50s python3 "$contract" sample-owned-storage', self.function("watchdog"))
        self.assertIn('timeout=10', SOURCE.read_text())
        self.assertIn('signal-parent --parent-pid "$main_pid"', self.function("signal_main"))
        self.assertIn('deadline-guard --parent-pid "$main_pid"', self.runner)

    def test_orphan_watchdog_stops_without_scanning_or_signaling(self) -> None:
        body = """main_pid=424242
main_start=123
contract=/unused
run_dir=/unused
baseline_free=9999999
python3() { return 3; }
bounded() { printf 'unexpected scan\\n'; }
signal_main() { printf 'unexpected signal\\n'; }
""" + self.function("watchdog_parent_alive") + self.function("watchdog") + "\nwatchdog\n"
        result = self.bash(body)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_parent_monitor_error_records_failure_and_keeps_watchdog_running(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
            body = f"""main_pid=424242
main_start=123
contract=/unused
run_dir={shlex.quote(name)}
python3() {{ return 1; }}
""" + self.function("watchdog_parent_alive") + "\nwatchdog_parent_alive\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((pathlib.Path(name) / "watchdog-failure").exists())
            self.assertIn("parent monitor failed", result.stderr)

    def test_watchdog_retries_pidfd_signal_after_helper_error(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
            body = f"""run_dir={shlex.quote(name)}
attempts=0
sleep() {{ :; }}
signal_main() {{ attempts=$((attempts + 1)); ((attempts >= 2)); }}
""" + self.function("watchdog_abort") + "\nwatchdog_abort\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertTrue((pathlib.Path(name) / "watchdog-failure").exists())

    def test_independent_deadline_uses_pidfd_and_fixed_cleanup_grace(self) -> None:
        self.assertIn('--execution-seconds 780 --cleanup-seconds 120 --ready-file "$run_dir/deadline-guard-ready" &', self.runner)
        self.assertIn("runtime_deadline=$((runtime_deadline + 750))", self.runner)
        self.assertNotIn('kill -TERM "$main_pid"', self.runner)
        self.assertNotIn('kill -KILL "$main_pid"', self.runner)
        self.assertIn("signal.pidfd_send_signal(fd, signal.SIGTERM)", SOURCE.read_text())
        self.assertIn("signal.pidfd_send_signal(fd, signal.SIGKILL)", SOURCE.read_text())

    def test_late_long_runtime_command_is_clipped_before_guard_term(self) -> None:
        body = """guard_ready=true
cleanup_active=false
runtime_deadline=750
guard_alive() { :; }
cleanup_now() { printf '730\\n'; }
timeout() { printf 'duration=%s\\n' "$3"; return 124; }
""" + self.function("bounded") + "\nbounded 180s sleep 999\n"
        result = self.bash(body)
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(result.stdout.strip(), "duration=12s")

    def test_dead_guard_prevents_foreground_runtime_operation(self) -> None:
        body = """guard_ready=true
cleanup_active=false
guard_alive() { return 1; }
timeout() { printf 'unsafe command ran\\n'; }
""" + self.function("bounded") + "\nbounded 180s sleep 999\n"
        result = self.bash(body)
        self.assertEqual(result.returncode, 125, result.stderr)
        self.assertNotIn("unsafe command ran", result.stdout)

    def test_slow_cleanup_exhausts_aggregate_budget_and_reports_exact_residuals(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
            directory = pathlib.Path(name)
            clock = directory / "fake-clock"
            clock.write_text("0\n", encoding="ascii")
            body = f"""registered=true
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
run_id=test
run_dir={shlex.quote(name)}
watchdog_pid=
socket_path=
cleanup_now() {{ read -r tick < {shlex.quote(str(clock))}; printf '%s\\n' "$tick"; }}
timeout() {{ printf '100\\n' > {shlex.quote(str(clock))}; return 124; }}
""" + self.function("cleanup_bounded") + self.function("cleanup_owned") + "\ncleanup_owned\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(
                f"cleanup residuals: outer-container=bf-docker-core-test "
                f"storage-volume=bf-docker-core-data-test private-directory={name}", result.stderr,
            )
            self.assertTrue(directory.exists())

    def test_unjoined_watchdog_keeps_scanned_resources_and_reports_exact_names(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
            directory = pathlib.Path(name)
            archive = directory / "busybox.tar"
            archive.write_bytes(b"owned archive")
            body = f"""registered=true
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
run_dir={shlex.quote(name)}
watchdog_pid=123
cleanup_now() {{ printf '0\\n'; }}
sleep() {{ :; }}
kill() {{ return 0; }}
cleanup_bounded() {{ printf 'unsafe deletion\\n'; return 1; }}
""" + self.function("cleanup_owned") + "\ncleanup_owned\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertNotIn("unsafe deletion", result.stdout)
            self.assertTrue(archive.exists())
            self.assertTrue((directory / "stop-watchdog").exists())
            self.assertIn(
                f"cleanup residuals: outer-container=bf-docker-core-test "
                f"storage-volume=bf-docker-core-data-test private-directory={name}", result.stderr,
            )

    def test_failed_volume_check_retains_private_evidence_and_reports_name(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
            directory = pathlib.Path(name)
            (directory / "busybox.tar").write_bytes(b"owned archive")
            body = f"""registered=true
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
run_id=test
run_dir={shlex.quote(name)}
watchdog_pid=
cleanup_now() {{ printf '1\\n'; }}
socket_path=
cleanup_bounded() {{
  if [[ $2 == python3 && $4 == podman-presence && $6 == container ]]; then printf 'absent\n'; return 1; fi
  if [[ $2 == python3 && $4 == podman-presence && $6 == volume ]]; then return 124; fi
  shift
  "$@"
}}
""" + self.function("cleanup_owned") + "\ncleanup_owned\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, 1)
            self.assertTrue(directory.exists())
            self.assertIn("storage-volume=bf-docker-core-data-test", result.stderr)
            self.assertIn(f"private-directory={name}", result.stderr)

    def test_unowned_outer_is_retained_but_its_private_archive_is_removed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
            directory = pathlib.Path(name)
            archive = directory / "busybox.tar"
            archive.write_bytes(b"owned archive")
            body = f"""registered=true
outer=bf-docker-core-test
storage_volume=
run_id=test
run_dir={shlex.quote(name)}
watchdog_pid=
cleanup_now() {{ printf '1\\n'; }}
socket_path=
cleanup_bounded() {{
  if [[ $2 == python3 && $4 == podman-presence && $6 == container ]]; then printf 'present\n'; return 0; fi
  if [[ $2 == podman && $3 == inspect ]]; then printf '%s\\n' other-run; return 0; fi
  shift
  "$@"
}}
""" + self.function("cleanup_owned") + "\ncleanup_owned\n"
            result = self.bash(body)
            self.assertEqual(result.returncode, 1)
            self.assertTrue(directory.exists())
            self.assertFalse(archive.exists())
            self.assertIn("outer-container=bf-docker-core-test", result.stderr)
            self.assertIn(f"private-directory={name}", result.stderr)

    def failed_ownership_cleanup(self, resource: str) -> None:
        for status in (1, 124):
            with self.subTest(resource=resource, status=status), \
                    tempfile.TemporaryDirectory(prefix="boxferry-cleanup-proof-") as proof_name, \
                    tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                directory = pathlib.Path(name)
                archive = directory / "busybox.tar"
                archive.write_bytes(b"owned archive")
                attempted_removal = pathlib.Path(proof_name) / "attempted-removal"
                outer = "bf-docker-core-test" if resource == "container" else ""
                volume = "bf-docker-core-data-test" if resource == "volume" else ""
                inspect_prefix = "$3 == inspect" if resource == "container" else "$3 == volume && $4 == inspect"
                body = f"""registered=true
outer={shlex.quote(outer)}
storage_volume={shlex.quote(volume)}
run_id=test
run_dir={shlex.quote(name)}
watchdog_pid=
socket_path=
cleanup_now() {{ printf '1\\n'; }}
cleanup_bounded() {{
  if [[ $2 == python3 && $4 == podman-presence && $6 == {resource} ]]; then printf 'present\n'; return 0; fi
  if [[ $2 == podman && {inspect_prefix} ]]; then printf 'test\\n'; return {status}; fi
  if [[ $2 == podman ]]; then printf 'unsafe removal\\n' > {shlex.quote(str(attempted_removal))}; return 0; fi
  shift
  "$@"
}}
""" + self.function("cleanup_owned") + "\ncleanup_owned\n"
                result = self.bash(body)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertFalse(attempted_removal.exists(), "failed inspect authorized native removal")
                self.assertIn("ownership unverified; refusing removal", result.stderr)
                if resource == "container":
                    self.assertTrue(directory.exists())
                    self.assertFalse(archive.exists())
                    expected = f"outer-container={outer} storage-volume=none private-directory={name}"
                else:
                    self.assertTrue(directory.exists())
                    self.assertFalse(archive.exists())
                    expected = f"outer-container=none storage-volume={volume} private-directory={name}"
                self.assertIn("cleanup residuals: " + expected, result.stderr)

    def test_volume_profile_requires_inner_absence_proof_despite_successful_outer_teardown(self) -> None:
        closure = self.runner[self.runner.index("cleanup_interrupted=false\ntrap"):
                              self.runner.index("if [[ $profile == core-journey ]]; then\n  printf 'CORE-JOURNEY-SCAFFOLD")]
        for scenario in ("outer-absent", "inner-failed", "verified", "ledger-missing"):
            with self.subTest(scenario=scenario), \
                    tempfile.TemporaryDirectory(prefix="boxferry-cleanup-proof-") as proof_name, \
                    tempfile.TemporaryDirectory(prefix="boxferry-docker-core.", dir="/tmp") as name:
                directory = pathlib.Path(name)
                proof = pathlib.Path(proof_name)
                ledger = directory / "volume-ledger.json"
                ledger_bytes = json.dumps({"volumes": [{"Name": "test-prefix-forge-db", "Labels": {
                    "io.boxferry.live-run": "test-run", "io.boxferry.application": "test-prefix-forgejo"}}]}).encode()
                if scenario != "ledger-missing":
                    ledger.write_bytes(ledger_bytes)
                    ledger.chmod(0o600)
                socket_path = directory / "docker.sock"
                # Native transport is fake: only the Bash socket predicate uses
                # a regular sentinel, avoiding an actual socket/runtime service.
                with socket_path.open("wb"):
                    body = f"""profile=volume-fixtures
registered=true
volume_apply_attempted=true
outer=bf-docker-core-test
storage_volume=bf-docker-core-data-test
run_id=test
volume_run=test-run
volume_prefix=test-prefix
lane=upstream-rootful
api_version=1.56
run_dir={shlex.quote(name)}
socket_path={shlex.quote(str(socket_path))}
watchdog_pid=
contract=unused
boxferry_root=unused
boxferry_binary=unused
boxferry_receipt=unused
lens_root=unused
lens_revision=unused
receipt_sha256=candidate
outer_removed=false
storage_removed=false
finish_volume_evidence() {{ :; }}  # This control isolates inner-cleanup admission; handoff has separate controls.
cleanup_now() {{ printf '1\\n'; }}
report_host_cache() {{ :; }}
bounded() {{ :; }}
python3() {{ printf 'candidate\\n'; }}
cleanup_bounded() {{
  if [[ $2 == python3 && $4 == podman-presence && $6 == container ]]; then
    if [[ {shlex.quote(scenario)} != outer-absent && $outer_removed == false ]]; then printf 'present\n'; return 0; fi
    printf 'absent\n'; return 1
  fi
  if [[ $2 == podman && $3 == inspect ]]; then printf 'test\\n'; return 0; fi
  if [[ $2 == python3 && $4 == cleanup-volume-fixtures ]]; then
    printf 'attempted\\n' > {shlex.quote(str(proof / 'inner-attempted'))}
    [[ {shlex.quote(scenario)} == verified ]]; return;
  fi
  if [[ $2 == podman && $3 == rm ]]; then outer_removed=true; return 0; fi
  if [[ $2 == python3 && $4 == podman-presence && $6 == volume ]]; then
    if [[ $storage_removed == false ]]; then printf 'present\n'; return 0; fi
    printf 'absent\n'; return 1
  fi
  if [[ $2 == podman && $3 == volume && $4 == inspect ]]; then printf 'test\\n'; return 0; fi
  if [[ $2 == podman && $3 == volume && $4 == rm ]]; then
    storage_removed=true
    printf 'removed\\n' > {shlex.quote(str(proof / 'storage-removed'))}
    return 0
  fi
  shift
  "$@"
}}
""" + self.function("cleanup_owned").replace("-S $socket_path", "-f $socket_path") + closure
                    result = self.bash(body)
                self.assertTrue((proof / "storage-removed").exists(), result.stderr)
                if scenario == "verified":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("VOLUME-ONLY REHEARSAL CHECKS-PASSED", result.stdout)
                    self.assertFalse(directory.exists())
                else:
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertNotIn("CHECKS-PASSED", result.stdout)
                    self.assertIn("inner-volume cleanup/absence unverified", result.stderr)
                    self.assertIn(f"private-directory={name}", result.stderr)
                    self.assertTrue(directory.exists())
                    if scenario != "ledger-missing":
                        self.assertEqual(ledger.read_bytes(), ledger_bytes)
                self.assertEqual((proof / "inner-attempted").exists(), scenario != "outer-absent")

    def test_failed_outer_inspection_cannot_authorize_removal_with_matching_stdout(self) -> None:
        self.failed_ownership_cleanup("container")

    def test_failed_volume_inspection_cannot_authorize_removal_with_matching_stdout(self) -> None:
        self.failed_ownership_cleanup("volume")

    def test_native_curl_responses_and_pre_watchdog_podman_calls_are_bounded(self) -> None:
        curl_lines = self.curl_statements()
        self.assertEqual(len(curl_lines), 4)
        for line in curl_lines:
            self.assertIn("curl -q --noproxy '*' ", line)
            self.assertIn("--max-filesize 65536", line)
            self.assertIn("--max-time 10", line)
        self.assertIn("bounded 15s podman info --format '{{.Host.Security.Rootless}}'", self.runner)
        self.assertIn('podman_presence cleanup_bounded container "$outer"', self.runner)
        self.assertIn('podman_presence cleanup_bounded volume "$storage_volume"', self.runner)

    def test_all_four_shell_curl_sites_have_exact_isolated_get_argv_and_single_invocations(self) -> None:
        statements = self.curl_statements()
        self.assertEqual(len(statements), 4)
        self.assertEqual(sum(statement.startswith("inner_status=") for statement in statements), 2)
        self.assertEqual(sum(statement.startswith("version_json=") for statement in statements), 1)
        self.assertEqual(sum(statement.startswith("info_json=") for statement in statements), 1)
        with tempfile.TemporaryDirectory() as name:
            directory = pathlib.Path(name)
            socket_path = directory / "DO-NOT-CONNECT.sock"
            calls = directory / "calls.jsonl"
            wrapper_calls = directory / "wrappers"
            # Entire argv literals assert default GET, URLs, counts and limits;
            # each source call site is evaluated once without native resources.
            cleanup = ["-q", "--noproxy", "*", "--silent", "--max-time", "10", "--max-filesize", "65536",
                       "--output", "/dev/null", "--write-out", "%{http_code}", "--unix-socket", str(socket_path),
                       "http://localhost/v1.41/containers/bf-docker-core/json"]
            version = ["-q", "--noproxy", "*", "--fail", "--silent", "--max-time", "10", "--max-filesize", "65536",
                       "--unix-socket", str(socket_path), "http://localhost/version"]
            info = ["-q", "--noproxy", "*", "--fail", "--silent", "--max-time", "10", "--max-filesize", "65536",
                    "--unix-socket", str(socket_path), "http://localhost/info"]
            expected = [cleanup, cleanup, version, info]
            curl = directory / "curl"
            curl.write_text(f"""#!{sys.executable}
import json, pathlib, sys
calls = pathlib.Path({str(calls)!r})
observed = [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []
expected = {expected!r}
with calls.open('a') as stream:
    stream.write(json.dumps(sys.argv[1:]) + '\\n')
if len(observed) >= len(expected) or sys.argv[1:] != expected[len(observed)]:
    sys.exit(98)
sys.stdout.write('200' if len(observed) == 0 else '404' if len(observed) == 1 else '{{}}')
""", encoding="utf-8")
            curl.chmod(0o700)
            body = f"""PATH={shlex.quote(name)}:"$PATH"
socket_path={shlex.quote(str(socket_path))}
api_version=1.41
failure=0
cleanup_bounded() {{
  printf '%s\\n' "$1" >> {shlex.quote(str(wrapper_calls))}
  [[ $1 == 10s ]] || return 99
  shift
  "$@"
}}
""" + "\n".join(statements) + '\n[[ $failure == 0 && $inner_status == 404 && $version_json == "{}" && $info_json == "{}" ]]\n'
            result = self.bash(body)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout + result.stderr, "")
            self.assertEqual([json.loads(line) for line in calls.read_text().splitlines()], expected)
            self.assertEqual(wrapper_calls.read_text(), "10s\n10s\n")

    def test_retained_host_cache_report_names_exact_pinned_inputs(self) -> None:
        body = """outer_image=ghcr.io/example/engine:v1@sha256:aaaa
fixture_image=docker.io/library/busybox:1@sha256:bbbb
outer_pull_attempted=true
fixture_pull_attempted=false
""" + self.function("report_host_cache") + "\nreport_host_cache\n"
        result = self.bash(body)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("policy=retained-shared", result.stderr)
        self.assertIn("outer-pin=ghcr.io/example/engine:v1@sha256:aaaa", result.stderr)
        self.assertIn("fixture-pull-attempted=false", result.stderr)


if __name__ == "__main__":
    unittest.main()
