#!/usr/bin/env bash
set -euo pipefail
python3 - << 'PY'
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import tempfile
import unittest

ROOT = Path.cwd()
IDENTIFIER = "a" * 64

class EmulatorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="boxferry-emulator-test-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.bin = self.directory / "bin"
        self.bin.mkdir()
        self.log = self.directory / "calls"
        self.state = self.directory / "container.json"
        self.output = self.directory / "emulator"
        self.env = {**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}", "CALLS": str(self.log), "STATE": str(self.state)}
        runtime = '''#!/usr/bin/env python3
import json,os,pathlib,shutil,sys
args=sys.argv[1:]
with open(os.environ["CALLS"], "a") as stream: stream.write(json.dumps([pathlib.Path(sys.argv[0]).name, *args]) + "\\n")
state=pathlib.Path(os.environ["STATE"])
if args[0] == "create":
 name=args[args.index("--name")+1]
 label,owner=args[args.index("--label")+1].split("=",1)
 assert args[args.index("--platform")+1] == "linux/amd64"
 item={"Id":"a"*64,"Name":"/"+name,"Config":{"Image":args[-1],"Labels":{label:owner}},"State":{"Running":False}}
 if pathlib.Path(sys.argv[0]).name == "podman":
  tag_reference,digest=args[-1].rsplit("@",1)
  normalized=tag_reference.rsplit(":",1)[0]+"@"+digest
  item["Config"]["Image"]=normalized
  item["ImageName"]=normalized
  item["ImageDigest"]=digest
 state.write_text(json.dumps(item))
 print(os.environ.get("CREATE_STDOUT", item["Id"]))
 if os.environ.get("CREATE_TIMEOUT"): raise SystemExit(124)
elif args[:2] == ["container", "inspect"]:
 if os.environ.get("INSPECT_MISSING"): raise SystemExit(1)
 item=json.loads(state.read_text())
 assert args[2] == item["Name"][1:]
 if os.environ.get("BAD_OWNER"): item["Config"]["Labels"]={}
 if os.environ.get("BAD_IMAGE"): item["Config"]["Image"]="unowned-image"
 if os.environ.get("FOREIGN_REPOSITORY"):
  reference="docker.io/foreign/binfmt@"+item["ImageDigest"]
  item["Config"]["Image"]=reference
  item["ImageName"]=reference
 if os.environ.get("WRONG_DIGEST"):
  reference="docker.io/tonistiigi/binfmt@sha256:"+"b"*64
  item["Config"]["Image"]=reference
  item["ImageName"]=reference
  item["ImageDigest"]="sha256:"+"b"*64
 if os.environ.get("CONFLICTING_REFERENCE"): item["ImageName"]="docker.io/foreign/binfmt@"+item["ImageDigest"]
 if os.environ.get("CONFLICTING_DIGEST"): item["ImageDigest"]="sha256:"+"b"*64
 if os.environ.get("BARE_TAG"):
  item["Config"]["Image"]="docker.io/tonistiigi/binfmt:qemu-v10.2.3-68"
  item["ImageName"]=item["Config"]["Image"]
 if os.environ.get("REFERENCE_SUFFIX"):
  item["Config"]["Image"] += "/suffix"
  item["ImageName"]=item["Config"]["Image"]
 if os.environ.get("BAD_NAME"): item["Name"]="/unowned-container"
 if os.environ.get("BAD_INSPECT_ID"): item["Id"]="a"*12
 if os.environ.get("RUNNING"): item["State"]["Running"]=True
 print(json.dumps([item]))
elif args[0] == "cp":
 assert args[1].startswith("a"*64+":/usr/bin/qemu-")
 if os.environ.get("BAD_COPY"): raise SystemExit(1)
 shutil.copyfile("/bin/true", args[2])
 if os.environ.get("BAD_ELF"): pathlib.Path(args[2]).write_bytes(b"bad emulator")
elif args[0] == "rm":
 assert args == ["rm", "--", "a"*64], "attempted arbitrary or forced deletion"
 if os.environ.get("BAD_REMOVE"): raise SystemExit(1)
 state.unlink()
else: raise SystemExit(99)
'''
        for name in ["docker", "podman"]:
            self.write(name, runtime)
        self.write("timeout", '''#!/usr/bin/env python3
import os,subprocess,sys
args=sys.argv[1:]
assert args[:2] == ["--signal=TERM", "--kill-after=5s"]
assert args[2] in ["10s", "30s", "60s", "180s"]
if args[3].endswith(("/qemu-i386", "/qemu-arm")):
 print("qemu-" + os.environ.get("EXPECTED_MACHINE", "i386") + " version " + ("0.0.0" if os.environ.get("BAD_VERSION") else "10.2.3") + " (reviewed tool)")
else: raise SystemExit(subprocess.run(args[3:], check=False).returncode)
''')
    def write(self, name, text):
        path = self.bin / name
        path.write_text(text)
        path.chmod(0o755)
    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
    def run_helper(self, backend="docker", target="i686-unknown-linux-musl", **changes):
        return subprocess.run(["bash", "scripts/lib/release-emulator.sh", backend, target, str(self.output)], cwd=ROOT, env={**self.env, **changes}, capture_output=True, text=True, timeout=15, check=False)
    def test_explicit_backends_extract_without_running_or_privilege(self):
        for backend in ["docker", "podman"]:
            with self.subTest(backend=backend):
                self.output = self.directory / backend
                result = self.run_helper(backend)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue((self.output / "qemu-i386").is_file())
                self.assertFalse((self.output / "container-recovery.json").exists())
        calls = self.calls()
        self.assertEqual(sum(call[1] == "create" for call in calls), 2)
        self.assertEqual(sum(call[1:] == ["rm", "--", IDENTIFIER] for call in calls), 2)
        self.assertFalse(any("--privileged" in call or "run" in call or "--install" in call or "-f" in call for call in calls))
        self.assertTrue(any(":/usr/bin/qemu-i386" in " ".join(call) for call in calls))
    def test_podman_normalized_reference_preserves_exact_repository_and_digest(self):
        result = self.run_helper(backend="podman", BAD_REMOVE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Failed to remove the verified stopped extraction container", result.stderr)
        item = json.loads(self.state.read_text())
        record = json.loads((self.output / "container-recovery.json").read_text())
        tagged_repository, digest = record["image"].rsplit("@", 1)
        normalized = tagged_repository.rsplit(":", 1)[0] + "@" + digest
        self.assertEqual(item["Config"]["Image"], normalized)
        self.assertEqual(item["ImageName"], normalized)
        self.assertEqual(item["ImageDigest"], digest)
        self.assertEqual([call[1:] for call in self.calls() if call[1] == "rm"], [["rm", "--", IDENTIFIER]])
    def test_podman_foreign_conflicting_or_unpinned_references_never_delete(self):
        for flag in ["FOREIGN_REPOSITORY", "WRONG_DIGEST", "CONFLICTING_REFERENCE", "CONFLICTING_DIGEST", "BARE_TAG", "REFERENCE_SUFFIX"]:
            with self.subTest(flag=flag):
                self.output = self.directory / flag
                before = len(self.calls())
                result = self.run_helper(backend="podman", **{flag:"1"})
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue((self.output / "container-recovery.json").is_file())
                self.assertFalse(any(call[1] in ["rm", "cp"] for call in self.calls()[before:]))
                self.assertIn("no container was removed", result.stderr)
    def test_arm_uses_the_selected_direct_emulator(self):
        result = self.run_helper(target="armv7-unknown-linux-musleabihf", EXPECTED_MACHINE="arm")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(":/usr/bin/qemu-arm" in " ".join(call) for call in self.calls()))
    def test_installed_emulator_is_accepted_by_the_artifact_driver(self):
        driver = runpy.run_path(str(ROOT / "scripts/release-binaries.py"))
        for target, machine in [("i686-unknown-linux-musl", "i386"), ("armv7-unknown-linux-musleabihf", "arm")]:
            with self.subTest(target=target):
                self.output = self.directory / machine
                result = self.run_helper(target=target, EXPECTED_MACHINE=machine)
                self.assertEqual(result.returncode, 0, result.stderr)
                path = self.output / ("qemu-" + machine)
                self.assertEqual(driver["execution"](target, path), {"mode": "qemu-user", "emulator": path.name})
                with self.assertRaises(driver["ArtifactError"]):
                    driver["execution"](target, self.output / "qemu")
                other = "armv7-unknown-linux-musleabihf" if machine == "i386" else "i686-unknown-linux-musl"
                with self.assertRaises(driver["ArtifactError"]):
                    driver["execution"](other, path)
    def test_unsupported_backend_target_and_collision_do_not_touch_runtime(self):
        for backend,target in [("auto", "i686-unknown-linux-musl"), ("docker", "x86_64-unknown-linux-musl")]:
            self.assertNotEqual(self.run_helper(backend,target).returncode, 0)
        self.output.mkdir()
        self.assertNotEqual(self.run_helper().returncode, 0)
        self.assertFalse(self.log.exists())
    def test_bad_copy_architecture_and_version_propagate_failure_and_cleanup(self):
        for variable in ["BAD_COPY", "BAD_ELF", "BAD_VERSION"]:
            with self.subTest(variable=variable):
                self.output = self.directory / variable
                self.assertNotEqual(self.run_helper(**{variable:"1"}).returncode, 0)
                self.assertFalse(self.output.exists())
                self.assertFalse(self.state.exists())
        self.assertEqual(sum(call[1:] == ["rm", "--", IDENTIFIER] for call in self.calls()), 3)
    def test_malformed_stdout_recovers_only_a_proven_owned_full_identifier(self):
        for stdout in ["--all", "a"*12, "a"*63, "a"*65, "", IDENTIFIER+"\\n--all"]:
            with self.subTest(stdout=stdout):
                self.output = self.directory / ("bad-"+str(len(stdout)))
                before=len(self.calls())
                result=self.run_helper(CREATE_STDOUT=stdout)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.output.exists())
                calls=self.calls()[before:]
                self.assertFalse(any(call[1] == "cp" for call in calls))
                self.assertEqual([call[1:] for call in calls if call[1] == "rm"], [["rm", "--", IDENTIFIER]])
    def test_ambiguous_create_recovers_owned_stopped_container(self):
        result=self.run_helper(CREATE_TIMEOUT="1", CREATE_STDOUT="--all")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.output.exists())
        self.assertFalse(self.state.exists())
        self.assertEqual([call[1:] for call in self.calls() if call[1] == "rm"], [["rm", "--", IDENTIFIER]])
    def test_unverified_identity_preserves_recovery_and_never_deletes(self):
        for flag in ["INSPECT_MISSING", "BAD_OWNER", "BAD_IMAGE", "BAD_NAME", "BAD_INSPECT_ID", "RUNNING"]:
            with self.subTest(flag=flag):
                self.output=self.directory / flag
                before=len(self.calls())
                result=self.run_helper(**{flag:"1", "CREATE_TIMEOUT":"1"})
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue((self.output / "container-recovery.json").is_file())
                self.assertTrue(self.state.is_file())
                self.assertFalse(any(call[1] in ["rm", "cp"] for call in self.calls()[before:]))
                self.assertIn("no container was removed", result.stderr)
                self.assertIn("Recovery record retained", result.stderr)
    def test_cleanup_failure_retains_actionable_recovery_and_propagates(self):
        result=self.run_helper(BAD_REMOVE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.state.is_file())
        self.assertFalse((self.output / "qemu-i386").exists())
        record=json.loads((self.output / "container-recovery.json").read_text())
        state=json.loads(self.state.read_text())
        self.assertEqual(state["Name"], "/"+record["name"])
        self.assertEqual(state["Config"]["Labels"][record["label"]], record["owner"])
        self.assertIn("Recovery record retained", result.stderr)

class RenovateTests(unittest.TestCase):
    def test_build_tag_updates_preserve_the_single_tag_and_digest_owner(self):
        config=json.loads((ROOT / ".github/renovate.json").read_text())
        owners=[manager for manager in config["customManagers"] if manager["managerFilePatterns"] == ["/^scripts/lib/release-emulator\\.sh$/"]]
        self.assertEqual(len(owners), 1)
        owner=owners[0]
        expected=r"regex:^qemu-v(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)-(?<build>\d+)$"
        self.assertEqual(owner["versioningTemplate"], expected)
        expression=re.compile(expected.removeprefix("regex:").replace("(?<", "(?P<"))
        current=expression.fullmatch("qemu-v10.2.3-68")
        self.assertIsNotNone(current)
        assert current is not None
        for tag,components in [("qemu-v10.2.3-69",(10,2,3,69)),("qemu-v10.3.0-70",(10,3,0,70))]:
            match=expression.fullmatch(tag)
            self.assertIsNotNone(match)
            assert match is not None
            parsed=tuple(int(match[group]) for group in ["major","minor","patch","build"])
            self.assertEqual(parsed, components)
            self.assertGreater(parsed, tuple(int(current[group]) for group in ["major","minor","patch","build"]))
            source=(ROOT / "scripts/lib/release-emulator.sh").read_text()
            extraction=re.compile(owner["matchStrings"][0].replace("(?<", "(?P<"))
            self.assertEqual(len(list(extraction.finditer(source))), 1)
            metadata=extraction.search(source).groupdict()
            metadata.update(newValue=tag,newDigest="sha256:"+"b"*64)
            replacement=owner["autoReplaceStringTemplate"]
            for key,value in metadata.items(): replacement=replacement.replace("{{{"+key+"}}}", value)
            updated,count=extraction.subn(lambda match:replacement,source)
            self.assertEqual(count,1)
            self.assertEqual(updated.count(":"+tag+"@sha256:"+"b"*64),1)
        rules=[rule for rule in config["packageRules"] if rule.get("matchPackageNames") == ["docker.io/tonistiigi/binfmt"]]
        self.assertEqual(len(rules),1)
        self.assertIs(rules[0]["automerge"],False)
        self.assertIs(rules[0]["dependencyDashboardApproval"],True)

unittest.main()
PY
