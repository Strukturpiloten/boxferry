#!/usr/bin/env python3
"""Finite owner recovery through real guard processes; no container runtime."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "scripts/lib/docker-application-contract.py"
SPEC = importlib.util.spec_from_file_location("owner_contract", SOURCE)
assert SPEC is not None and SPEC.loader is not None
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)
CID = "a" * 64
IMAGE = "registry.invalid/native/engine:29.8.1@sha256:" + "b" * 64

# This is a native command fixture, not a fake owner/guard/capability. The real
# owner validates source, writes custody, observes, executes, kills and recovers.
BOUNDARY = r'''import json,os,pathlib,signal,sys,time
configuration=pathlib.Path(sys.argv[1]); args=sys.argv[2:]
state=json.loads(configuration.read_text())
def save():
    temporary=configuration.with_suffix('.new')
    temporary.write_text(json.dumps(state)); temporary.replace(configuration)
if args[0]=='simulate-create':
    if args[1]=='late':
        pathlib.Path(state['side_effect']).write_text(str(os.getpid()))
        time.sleep(1)
    state['container']=True; state['volume']=True; save()
    pathlib.Path(state['side_effect']).write_text(str(os.getpid()))
    if args[1] in ('valid','partial'):
        path=pathlib.Path(state['cid_file'])
        path.write_text('a'*64 if args[1]=='valid' else 'a'*12); path.chmod(0o600)
    if len(args)>2:
        while True: time.sleep(.05)
    sys.exit(0)
assert args[:4]==['--engine',state['engine'],'--directory',state['directory']]
op,*tail=args[4:]
with open(state['events'],'a') as log: log.write(json.dumps(args)+'\n')
if state.get('unavailable'): sys.exit(2)
if op=='probe-session':
    child=os.fork()
    if child==0:
        pathlib.Path(state['session_child']).write_text(str(os.getpid()))
        os.close(1); os.close(2)
        while True: time.sleep(.1)
    while not pathlib.Path(state['session_child']).exists(): time.sleep(.001)
    sys.exit(7)
if op=='presence':
    kind,identity=tail
    if kind=='container' and identity in (state['outer_name'],'a'*64):
        if state.get('unknown_presence'): sys.exit(2)
        sys.exit(0 if state['container'] else 1)
    if kind=='volume' and identity==state['volume_name']: sys.exit(0 if state['volume'] else 1)
    sys.exit(2)
if op=='daemon-binding':
    assert tail==['a'*64,state['outer_name'],'io.boxferry.docker-core-run',state['outer_run'],state['image']]
    sys.exit(0 if state['container'] and not state.get('wrong_identity') else 2)
if op=='check-mounts':
    assert tail==['a'*64,state['volume_name'],state['mode'],state['directory']+'/socket','/boxferry-core','/private/graph']
    expected=[{'Type':'volume','Name':state['volume_name'],'Driver':'local','RW':True,
               'Source':'/private/graph/volumes/'+state['volume_name']+'/_data',
               'Destination':'/var/lib/docker' if state['mode']=='rootful' else '/home/docker/.local/share/docker'},
              {'Type':'bind','RW':True,'Source':state['directory']+'/socket','Destination':'/boxferry-core'}]
    sys.exit(0 if state['mounts']==expected else 2)
if op=='observe':
    assert tail==['volume',state['volume_name']]
    result={'Name':state['volume_name'],'Driver':'local',
            'Mountpoint':'/private/graph/volumes/'+state['volume_name']+'/_data',
            'Labels':{'io.boxferry.docker-core-run':state['outer_run']},'Options':None,'Scope':'local'}
    result.update(state.get('volume_observation',{}))
    print(json.dumps(result)); sys.exit(0)
if op=='binding':
    assert tail==['volume',state['volume_name'],state['volume_name'],'io.boxferry.docker-core-run',state['outer_run']]
    sys.exit(0 if state['volume'] and not state.get('wrong_volume') else 2)
if op=='exec':
    if state.get('block_ready'):
        pathlib.Path(state['ready_blocked_pid']).write_text(str(os.getpid()))
        while True: time.sleep(.01)
    assert tail[:3]==['a'*64,'sh','-ec'] and tail[4:] in (
        ['--',state['fixture_root'],'new'],['--',state['fixture_root'],'bound'])
    assert '[ ! -L /tmp ]' in tail[3] and '/proc/self/mountinfo' in tail[3]
    sys.exit(2 if state.get('fixture_escape') else 0)
if op=='remove':
    assert tail==['a'*64,state['outer_name'],'io.boxferry.docker-core-run',state['outer_run']]
    if state.get('cleanup_failure'): sys.exit(2)
    if state.get('block'):
        pathlib.Path(state['blocked_pid']).write_text(str(os.getpid()))
        subprocess=None
        while True: time.sleep(.1)
    state['container']=False; save()
    if state.get('overflow'): os.write(1,b'private-output-canary'*65536)
    sys.exit(0)
if op=='volume-remove':
    assert tail==[state['volume_name'],'io.boxferry.docker-core-run',state['outer_run']]
    if state.get('wrong_volume'): sys.exit(2)
    state['volume']=False; save(); sys.exit(0)
if op=='apparmor':
    assert tail==['cleanup','upstream-rootless','c'*32]
    if not state.get('runner_receipt'): sys.exit(2)
    print(state['runner_receipt']); sys.exit(0)
raise AssertionError('unexpected native command')
'''


# Controlled ACK-write fault only. The real imported guard, its native boundary,
# pidfds, custody validation and teardown all run in the separate child process.
ACK_FAULT_GUARD = r'''import importlib.util,json,pathlib,signal,sys,time
source,custody,digest,event,tamper,marker=sys.argv[1:]
spec=importlib.util.spec_from_file_location('ack_fault_contract',source)
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
path=pathlib.Path(custody);value=json.loads(path.read_bytes())
original=module.socket.socket.sendall
fired=False
def sendall(channel,data,*args,**kwargs):
    global fired
    if not fired and json.loads(data)['phase']=='ready':
        fired=True;pathlib.Path(marker).write_text('entered-ack-write')
        if tamper=='yes':path.write_bytes(path.read_bytes()+b' ')
        if event=='parent-loss':
            descriptor=module.verified_parent_fd(value['parent_pid'],value['parent_start'])
            try:signal.pidfd_send_signal(descriptor,signal.SIGKILL)
            finally:module.os.close(descriptor)
        elif event=='cancel':signal.raise_signal(signal.SIGTERM)
        elif event=='expiry':
            while module.owner_clock()<value['execution_deadline']:time.sleep(.01)
        else:raise AssertionError('unknown injected event')
        raise BrokenPipeError('controlled acknowledgement write failure')
    return original(channel,data,*args,**kwargs)
module.socket.socket.sendall=sendall
sys.argv=module.owner_guard_command(value,path,digest)[1:]
raise SystemExit(module.main())
'''


class OwnerRecovery(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="owner-source-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = pathlib.Path(self.temporary.name)
        token = hashlib.sha256(os.fsencode(self.base)).hexdigest()[:8]
        self.directory = pathlib.Path("/tmp") / ("boxferry-docker-core." + token)
        self.directory.mkdir(mode=0o700)
        self.addCleanup(self.remove_directory)
        self.socket = self.directory / "socket"
        self.socket.mkdir(mode=0o700)
        self.snapshot = self.directory / "outer-runtime-helper"
        self.snapshot.mkdir(mode=0o700)
        self.state_path = self.base / "native-state.json"
        self.boundary = self.base / "native-boundary.py"
        self.boundary.write_text(BOUNDARY)
        for name in contract.OUTER_RUNTIME_FILES:
            path = self.snapshot / pathlib.Path(name).name
            data = ('exec "' + sys.executable + '" "' + str(self.boundary) + '" "' + str(self.state_path) + '" "$@"\n').encode() if name.endswith('native-outer-command.sh') else b'# held native fixture\n'
            path.write_bytes(data)
            path.chmod(0o600)
        self.parent = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], start_new_session=True)
        self.addCleanup(self.stop_parent)
        self.guard = None
        self.value = {
            "parent_pid": self.parent.pid, "parent_start": contract.process_start(self.parent.pid),
            "execution_deadline": int(contract.owner_clock()) + 20, "cleanup_deadline": int(contract.owner_clock()) + 28,
            "guard_source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
            "directory": str(self.directory), "directory_identity": self.identity(self.directory),
            "helper_closure": {name: hashlib.sha256((self.snapshot / pathlib.Path(name).name).read_bytes()).hexdigest()
                               for name in contract.OUTER_RUNTIME_FILES},
            "engine": "podman", "lane": "upstream-rootless", "context": "owned-context", "run": "owned-run",
            "prefix": "owned-prefix", "outer_name": "bf-docker-core-" + token, "outer_run": token,
            "outer_owner": "io.boxferry.docker-core-run", "outer_image": IMAGE,
            "volume_name": "bf-docker-core-data-" + token,
            "volume_path": "/private/graph/volumes/bf-docker-core-data-" + token + "/_data",
            "graph_root": "/private/graph", "socket_directory": str(self.socket),
            "socket_identity": self.identity(self.socket), "guest_directory": "/boxferry-core",
            "fixture_root": "/tmp/boxferry-forgejo-" + token, "runner_profile": None,
            "cid_file": str(self.directory / "outer-create.cid"),
        }
        self.state = {"engine": "podman", "directory": str(self.directory), "container": False, "volume": False,
                      "outer_name": self.value["outer_name"], "outer_run": token, "image": IMAGE,
                      "volume_name": self.value["volume_name"], "mode": "rootless", "fixture_root": self.value["fixture_root"],
                      "cid_file": self.value["cid_file"], "events": str(self.base / "events"),
                      "side_effect": str(self.base / "side-effect"), "blocked_pid": str(self.base / "blocked-pid"),
                      "ready_blocked_pid": str(self.base / "ready-blocked-pid"), "session_child": str(self.base / "session-child")}
        self.mounts()
        self.save()
        self.unrelated = self.base / "unrelated-private-key"
        self.unrelated.write_text("do-not-remove")

    def identity(self, path):
        info = path.stat()
        return f"{info.st_dev}:{info.st_ino}"

    def mounts(self):
        self.state["mounts"] = [
            {"Type": "volume", "Name": self.value["volume_name"], "Driver": "local", "RW": True,
             "Source": self.value["volume_path"],
             "Destination": "/var/lib/docker" if self.state["mode"] == "rootful" else "/home/docker/.local/share/docker"},
            {"Type": "bind", "RW": True, "Source": str(self.socket), "Destination": "/boxferry-core"}]

    def remove_directory(self):
        import shutil
        if self.directory.exists():
            shutil.rmtree(self.directory)

    def stop_parent(self):
        if self.guard is not None and self.guard.poll() is None:
            self.guard.kill()
            self.guard.wait(timeout=3)
        if self.parent.poll() is None:
            self.parent.kill()
        self.parent.wait(timeout=3)

    def save(self):
        self.state_path.write_text(json.dumps(self.state))

    def events(self):
        path = self.base / "events"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def prepare(self):
        self.path, self.digest = contract.prepare_owner_custody(self.value)
        self.assertEqual([row[4:] for row in self.events()], [
            ["presence", "container", self.value["outer_name"]], ["presence", "volume", self.value["volume_name"]]])
        self.assertTrue(self.path.exists())

    def launch(self):
        self.prepare()
        self.guard = subprocess.Popen(contract.owner_guard_command(self.value, self.path, self.digest),
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        cutoff = time.monotonic() + 3
        while not (self.directory / "owner-ready.json").exists():
            self.assertIsNone(self.guard.poll(), self.guard.stderr.read().decode() if self.guard.poll() is not None else "")
            self.assertLess(time.monotonic(), cutoff)
            time.sleep(.01)
        self.capability("pending")

    def capability(self, phase, **changes):
        options = {"phase": phase, "context": "owned-context", "execution_deadline": self.value["execution_deadline"],
                   "cleanup_deadline": self.value["cleanup_deadline"]}
        options.update(changes)
        return contract.owner_capability(self.path, self.digest, **options)

    def create(self, mode="valid"):
        subprocess.run([sys.executable, str(self.boundary), str(self.state_path), "simulate-create", mode], check=True, timeout=2)
        self.state = json.loads(self.state_path.read_text())

    def finish(self, expected=0):
        output, error = self.guard.communicate(timeout=12)
        self.assertEqual(self.guard.returncode, expected, error.decode())
        self.assertNotIn(b"canary", output + error)
        self.assertEqual(self.unrelated.read_text(), "do-not-remove")
        return error

    def test_real_guard_survives_owner_sigkill_and_removes_exact_owner_then_storage(self):
        self.launch()
        self.create()
        self.capability("ready")
        self.parent.kill()
        self.finish()
        self.assertFalse(self.directory.exists())
        operations = [row[4:] for row in self.events()]
        remove = operations.index(["remove", CID, self.value["outer_name"], "io.boxferry.docker-core-run", self.value["outer_run"]])
        volume = operations.index(["volume-remove", self.value["volume_name"], "io.boxferry.docker-core-run", self.value["outer_run"]])
        self.assertLess(remove, volume)
        self.assertFalse(json.loads(self.state_path.read_text())["container"])
        self.assertFalse(json.loads(self.state_path.read_text())["volume"])

    def test_owner_dies_before_any_create_closes_only_proven_absence(self):
        self.launch()
        self.parent.kill()
        self.finish()
        self.assertFalse(self.directory.exists())
        self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))

    def test_valid_cid_published_before_parent_bookkeeping_is_recovered(self):
        self.launch()
        self.create()
        self.assertFalse((self.directory / "owner-bound.json").exists())
        self.parent.kill()
        self.finish()
        self.assertFalse(self.directory.exists())

    def test_missing_cid_with_present_resource_retains_custody(self):
        self.launch(); self.create("missing"); self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))

    def test_partial_cid_with_present_resource_is_not_rediscovered_by_name(self):
        self.launch(); self.create("partial"); self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_normal_close_is_independent_cleanup_not_parent_success_echo(self):
        self.launch(); self.create(); self.capability("ready")
        self.capability("close")
        self.finish()
        self.assertFalse(self.directory.exists())
        self.assertIsNone(self.parent.poll())

    def test_deadline_expiry_uses_original_cutoff_and_kills_parent(self):
        self.value["execution_deadline"] = int(contract.owner_clock()) + 4
        self.value["cleanup_deadline"] = self.value["execution_deadline"] + 6
        cutoff = self.value["cleanup_deadline"]
        self.launch(); self.create()
        self.finish()
        self.assertLess(contract.owner_clock(), cutoff)
        self.assertFalse(self.directory.exists())
        self.assertIsNotNone(self.parent.poll())

    def test_context_deadline_and_copied_marker_do_not_grant_capability(self):
        self.launch()
        before = len(self.events())
        for options in ({"context": "other"}, {"execution_deadline": self.value["execution_deadline"] + 1},
                        {"cleanup_deadline": self.value["cleanup_deadline"] + 1}):
            with self.subTest(options=options), self.assertRaises(contract.ContractError):
                self.capability("pending", **options)
        marker = self.directory / "owner-ready.json"
        marker.write_text(json.dumps({"pid": self.parent.pid, "start": self.value["parent_start"], "custody_sha256": self.digest}))
        with self.assertRaises(contract.ContractError):
            self.capability("pending")
        self.assertEqual(len(self.events()), before)
        self.parent.kill(); self.finish()

    def test_wrong_outer_identity_stays_residual_and_refuses_removal(self):
        self.launch(); self.create(); self.state["wrong_identity"] = True; self.save()
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_mount_or_fixture_escape_cannot_grant_profile_readiness(self):
        self.launch(); self.create()
        self.state["mounts"].append({"Type": "bind", "Source": "/private/host-key", "Destination": "/tmp"}); self.save()
        with self.assertRaises((contract.ContractError, OSError)):
            self.capability("ready")
        self.finish(expected=1)
        self.assertFalse((self.directory / "owner-bound.json").exists())
        self.assertTrue(self.path.exists())

    def test_symlink_mount_escape_refuses_profile_readiness(self):
        self.launch(); self.create(); self.state["fixture_escape"] = True; self.save()
        with self.assertRaises((contract.ContractError, OSError)):
            self.capability("ready")
        self.finish(expected=1)
        self.assertTrue(self.path.exists())

    def test_engine_selection_is_explicit_and_both_existing_engines_work(self):
        self.value["engine"] = "docker"; self.value["lane"] = "upstream-rootful"
        self.state["engine"] = "docker"; self.state["mode"] = "rootful"; self.mounts(); self.save()
        self.launch(); self.create(); self.capability("ready"); self.parent.kill(); self.finish()
        self.assertTrue(all(row[:2] == ["--engine", "docker"] for row in self.events()))

    def test_source_or_custody_drift_refuses_before_any_native_mutation(self):
        self.prepare()
        original = self.path.read_bytes()
        self.path.write_bytes(original + b" ")
        with self.assertRaises(contract.ContractError):
            contract.owner_read(self.path, self.digest)
        self.path.write_bytes(original)
        (self.snapshot / "native-presence.py").write_bytes(b"changed helper")
        with self.assertRaises(contract.ContractError):
            contract.owner_read(self.path, self.digest)
        self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))

    def test_private_state_alias_permissions_types_and_fixture_paths_fail_before_native_calls(self):
        for key, bad in (("fixture_root", "/boxferry-core/probe"), ("fixture_root", "/tmp/../boxferry-core/probe"),
                         ("engine", "remote"), ("outer_name", "unrelated"), ("volume_path", "/private/wrong"),
                         ("execution_deadline", True), ("guard_source_sha256", "00" * 32)):
            value = copy.deepcopy(self.value); value[key] = bad
            with self.subTest(key=key, bad=bad), self.assertRaises(contract.ContractError):
                contract.prepare_owner_custody(value)
        self.socket.chmod(0o755)
        with self.assertRaises(contract.ContractError): contract.prepare_owner_custody(self.value)
        self.assertEqual(self.events(), [])

    def test_collision_and_unknown_presence_refuse_pending_custody(self):
        self.state["container"] = True; self.save()
        with self.assertRaises(contract.ContractError): contract.prepare_owner_custody(self.value)
        self.assertFalse((self.directory / "owner-custody.json").exists())

    def test_cleanup_failure_preserves_liability_and_does_not_remove_storage(self):
        self.launch(); self.create(); self.state["cleanup_failure"] = True; self.save()
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "volume-remove" for row in self.events()))

    def test_output_overflow_after_partial_teardown_retains_custody(self):
        self.launch(); self.create(); self.state["overflow"] = True; self.save()
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertTrue(json.loads(self.state_path.read_text())["volume"])

    def test_guard_cancellation_runs_bounded_recovery_instead_of_abandoning_owner(self):
        self.launch(); self.create(); self.guard.send_signal(signal.SIGTERM); self.finish()
        self.assertFalse(self.directory.exists())

    def test_real_inflight_create_is_stopped_before_pending_identity_observation(self):
        # The native fixture changes state and waits before any CID publication.
        # It is a real original-session child, not a late JSON success flag.
        self.parent.kill(); self.parent.wait(timeout=2)
        trigger = self.base / "begin-create"
        parent_source = (
            "import pathlib,subprocess,sys,time\n"
            "while not pathlib.Path(sys.argv[1]).exists(): time.sleep(.01)\n"
            "subprocess.run(sys.argv[2:],check=False)\n"
        )
        self.parent = subprocess.Popen([sys.executable, "-c", parent_source, str(trigger), sys.executable,
                                       str(self.boundary), str(self.state_path), "simulate-create", "missing", "hold"],
                                      start_new_session=True)
        self.value["parent_pid"] = self.parent.pid
        self.value["parent_start"] = contract.process_start(self.parent.pid)
        self.launch(); trigger.touch()
        end = time.monotonic() + 3
        effect = self.base / "side-effect"
        while not effect.exists():
            self.assertLess(time.monotonic(), end); time.sleep(.01)
        native_pid = int(effect.read_text())
        # Let the independently running guard bind the original-session child.
        time.sleep(.15)
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        try:
            fields = pathlib.Path(f"/proc/{native_pid}/stat").read_text().rsplit(")", 1)[1].split()
            self.assertEqual(fields[0], "Z")
        except FileNotFoundError:
            pass
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_recycled_parent_or_numeric_session_cannot_authorize_signals(self):
        with mock.patch.object(contract, "process_start", return_value=self.value["parent_start"] + 1), \
                mock.patch.object(contract.signal, "pidfd_send_signal") as sent:
            with self.assertRaises(contract.ContractError):
                contract.owner_stop_work(self.value, {})
            sent.assert_not_called()
        self.assertEqual(self.events(), [])

    def test_unknown_presence_and_unavailable_daemon_retain_custody(self):
        self.launch(); self.create(); self.state["unavailable"] = True; self.save()
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_expired_cleanup_terminates_bounded_native_process_and_retains_custody(self):
        self.value["execution_deadline"] = int(contract.owner_clock()) + 4
        self.value["cleanup_deadline"] = self.value["execution_deadline"] + 2
        self.launch(); self.create(); self.state["block"] = True; self.save(); self.parent.kill()
        self.finish(expected=1)
        self.assertTrue(self.path.exists())
        native_pid = int((self.base / "blocked-pid").read_text())
        self.assertFalse(pathlib.Path(f"/proc/{native_pid}").exists())
        self.assertLess(contract.owner_clock(), self.value["cleanup_deadline"] + 2)

    def test_owned_volume_binding_failure_retains_storage_after_exact_outer_removal(self):
        self.launch(); self.create(); self.state["wrong_volume"] = True; self.save(); self.parent.kill()
        self.finish(expected=1)
        self.assertTrue(self.path.exists())
        state = json.loads(self.state_path.read_text())
        self.assertFalse(state["container"]); self.assertTrue(state["volume"])

    def test_pending_unknown_presence_does_not_masquerade_as_absence(self):
        self.launch(); self.state["unknown_presence"] = True; self.save(); self.parent.kill()
        self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))

    def test_cid_symlink_cannot_rebind_to_a_different_private_file(self):
        self.launch(); self.create()
        pathlib.Path(self.value["cid_file"]).unlink()
        pathlib.Path(self.value["cid_file"]).symlink_to(self.unrelated)
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_runner_teardown_receipt_is_between_outer_and_storage_closure(self):
        self.value["engine"] = "docker"
        self.state["engine"] = "docker"
        self.value["runner_profile"] = {"nonce": "c" * 32, "revision": "d" * 40, "workflow_run": "1234", "attempt": "1"}
        receipt = {"schema": 1, "prerequisite": "github_runner_userns_v1", "outcome": "verified",
                   "candidate": "d" * 40, "label": "verified", "profile_sha256": "e" * 64,
                   "parser_sha256": "f" * 64, "abi_sha256": "1" * 64, "parser_version": "4.0.1",
                   "cleanup": "removed", "consumer": "boxferry-reviewed-docker-consumer",
                   "workflow_run": "1234", "attempt": "1"}
        self.state["runner_receipt"] = "DOCKERLENS_NATIVE_APPARMOR_PROFILE: " + json.dumps(receipt, sort_keys=True, separators=(",", ":"))
        self.save(); self.launch(); self.create(); self.capability("ready"); self.parent.kill(); self.finish()
        operations = [row[4] for row in self.events()]
        self.assertLess(operations.index("remove"), operations.index("apparmor"))
        self.assertLess(operations.index("apparmor"), operations.index("volume-remove"))

    def test_completed_cid_tamper_cannot_remove_a_replacement(self):
        self.launch(); self.create(); self.capability("ready")
        pathlib.Path(self.value["cid_file"]).write_text("f" * 64)
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_source_tamper_on_running_guard_revokes_capability_before_mutation(self):
        self.launch()
        (self.snapshot / "native-presence.py").write_bytes(b"drift")
        with self.assertRaises(contract.ContractError): self.capability("pending")
        self.guard.send_signal(signal.SIGTERM); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_before_ack_creation_is_not_accepted_as_guarded(self):
        self.prepare(); self.create()
        self.guard = subprocess.Popen(contract.owner_guard_command(self.value, self.path, self.digest),
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        self.finish(expected=1)
        self.assertFalse((self.directory / "owner-ready.json").exists())
        self.assertTrue(self.path.exists())

    def test_wrong_dedicated_volume_source_or_remote_options_refuse_ready(self):
        self.launch(); self.create()
        self.state["volume_observation"] = {"Options": {"type": "nfs", "device": "unrelated-host:/private"}}
        self.save()
        with self.assertRaises((contract.ContractError, OSError)): self.capability("ready")
        self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))

    def test_inflight_create_is_quiesced_before_missing_cid_and_absence_can_close(self):
        self.parent.kill(); self.parent.wait(timeout=2)
        trigger = self.base / "begin-create"
        parent_source = (
            "import pathlib,subprocess,sys,time\n"
            "while not pathlib.Path(sys.argv[1]).exists(): time.sleep(.01)\n"
            "subprocess.run(sys.argv[2:],check=False)\n"
        )
        self.parent = subprocess.Popen([sys.executable, "-c", parent_source, str(trigger), sys.executable,
                                       str(self.boundary), str(self.state_path), "simulate-create", "late"],
                                      start_new_session=True)
        self.value["parent_pid"] = self.parent.pid
        self.value["parent_start"] = contract.process_start(self.parent.pid)
        self.launch(); trigger.touch()
        end = time.monotonic() + 3
        effect = self.base / "side-effect"
        while not effect.exists():
            self.assertLess(time.monotonic(), end); time.sleep(.01)
        time.sleep(.15)
        self.parent.kill(); self.finish()
        time.sleep(1.05)
        self.assertFalse(self.directory.exists())
        state = json.loads(self.state_path.read_text())
        self.assertFalse(state["container"]); self.assertFalse(state["volume"])

    def test_close_failure_cannot_echo_a_successful_application_result(self):
        self.launch(); self.create(); self.state["cleanup_failure"] = True; self.save()
        with self.assertRaises((contract.ContractError, OSError)):
            self.capability("close")
        self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertTrue(json.loads(self.state_path.read_text())["container"])

    def test_sealed_custody_tamper_on_parent_loss_refuses_native_teardown(self):
        self.launch(); self.create()
        self.path.write_bytes(self.path.read_bytes() + b" ")
        self.parent.kill(); self.finish(expected=1)
        self.assertTrue(self.path.exists())
        self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))

    def test_native_group_is_signaled_before_any_explicit_or_implicit_reap(self):
        import selectors
        from types import SimpleNamespace
        order = []
        process = mock.MagicMock()
        process.pid = 424242
        process.wait.side_effect = lambda **_kwargs: order.append("reap") or 0
        process.poll.side_effect = AssertionError("poll would reap before group cleanup")
        process.communicate.side_effect = AssertionError("communicate would reap before group cleanup")
        selector = mock.MagicMock()
        selector.get_map.return_value = {}
        def exited(kind, pid, flags):
            self.assertEqual((kind, pid), (os.P_PID, 424242))
            self.assertEqual(flags, os.WEXITED | os.WNOHANG | os.WNOWAIT)
            self.assertNotIn("reap", order)
            order.append("observe-unreaped-exit")
            return SimpleNamespace(si_code=os.CLD_EXITED, si_status=7)
        def signal_group(pid, number):
            self.assertEqual((pid, number), (424242, signal.SIGKILL))
            self.assertNotIn("reap", order, "reaped identity could target a recycled unrelated group")
            order.append("signal-reserved-group")
        with mock.patch.object(contract.subprocess, "Popen", return_value=process), \
                mock.patch.object(selectors, "DefaultSelector", return_value=selector), \
                mock.patch.object(contract.os, "set_blocking"), mock.patch.object(contract.os, "waitid", side_effect=exited), \
                mock.patch.object(contract.os, "killpg", side_effect=signal_group):
            result = contract.owner_call(self.value, ["presence", "container", self.value["outer_name"]],
                                         self.value["execution_deadline"], unprepared=True)
        self.assertEqual(result, (7, b""))
        self.assertEqual(order, ["observe-unreaped-exit", "signal-reserved-group", "reap"])
        process.poll.assert_not_called(); process.communicate.assert_not_called()

    def test_exited_native_leader_keeps_descendant_group_owned_until_cleanup(self):
        self.prepare()
        result = contract.owner_call(self.value, ["probe-session"], self.value["execution_deadline"])
        self.assertEqual(result[0], 7)
        child = int((self.base / "session-child").read_text())
        try:
            state = pathlib.Path(f"/proc/{child}/stat").read_text().rsplit(")", 1)[1].split()[0]
            self.assertEqual(state, "Z")
        except FileNotFoundError:
            pass

    def blocked_readiness(self, event, *, tamper=False, fail_cleanup=False):
        if event == "expiry":
            self.value["execution_deadline"] = int(contract.owner_clock()) + 4
            self.value["cleanup_deadline"] = self.value["execution_deadline"] + 6
        original_cutoff = self.value["cleanup_deadline"]
        self.launch(); self.create()
        self.state["block_ready"] = True
        self.state["cleanup_failure"] = fail_cleanup
        self.save()
        outcomes = []
        def request_ready():
            try:
                outcomes.append(self.capability("ready"))
            except Exception as error:
                outcomes.append(error)
        client = threading.Thread(target=request_ready)
        client.start()
        marker = self.base / "ready-blocked-pid"
        cutoff = time.monotonic() + 3
        while not marker.exists():
            self.assertLess(time.monotonic(), cutoff); time.sleep(.01)
        blocked_pid = int(marker.read_text())
        if tamper:
            self.path.write_bytes(self.path.read_bytes() + b" ")
        if event == "parent-loss": self.parent.kill()
        elif event == "cancel":
            self.guard.send_signal(signal.SIGTERM)
            time.sleep(.03)
            if self.guard.poll() is None: self.guard.send_signal(signal.SIGINT)
        elif event != "expiry": raise AssertionError("unknown test event")
        client.join(timeout=12)
        self.assertFalse(client.is_alive())
        self.finish(expected=1)
        self.assertEqual(len(outcomes), 1)
        self.assertIsInstance(outcomes[0], Exception)
        self.assertFalse(pathlib.Path(f"/proc/{blocked_pid}").exists())
        self.assertLess(contract.owner_clock(), original_cutoff)
        if tamper or fail_cleanup:
            self.assertTrue(self.path.exists())
            self.assertFalse(any(row[4] == "volume-remove" for row in self.events()))
        else:
            self.assertFalse(self.directory.exists())
            state = json.loads(self.state_path.read_text())
            self.assertFalse(state["container"]); self.assertFalse(state["volume"])
            operations = [row[4] for row in self.events()]
            self.assertLess(operations.index("remove"), operations.index("volume-remove"))
            events = self.events(); time.sleep(.1)
            self.assertEqual(self.events(), events, "no native work may resume after closure")

    def test_parent_sigkill_during_real_blocked_readiness_runs_recovery(self):
        self.blocked_readiness("parent-loss")

    def test_guard_cancellation_during_real_blocked_readiness_runs_recovery(self):
        self.blocked_readiness("cancel")

    def test_execution_expiry_during_real_blocked_readiness_uses_original_reserve(self):
        self.blocked_readiness("expiry")

    def test_lifecycle_interruption_does_not_authorize_tampered_custody_cleanup(self):
        self.blocked_readiness("parent-loss", tamper=True)
        self.assertFalse(any(row[4] == "remove" for row in self.events()))

    def test_lifecycle_recovery_failure_retains_custody_and_fails(self):
        self.blocked_readiness("cancel", fail_cleanup=True)

    def ack_send_interruption(self, event, *, tamper=False):
        import socket
        import struct
        if event == "expiry":
            self.value["execution_deadline"] = int(contract.owner_clock()) + 5
            self.value["cleanup_deadline"] = self.value["execution_deadline"] + 6
        self.prepare()
        original_cutoff = self.value["cleanup_deadline"]
        hook = self.base / "ack-write-hook"
        self.guard = subprocess.Popen([sys.executable, "-c", ACK_FAULT_GUARD, str(SOURCE), str(self.path), self.digest,
                                       event, "yes" if tamper else "no", str(hook)],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        cutoff = time.monotonic() + 3
        while not (self.directory / "owner-ready.json").exists():
            self.assertIsNone(self.guard.poll())
            self.assertLess(time.monotonic(), cutoff); time.sleep(.01)
        def request(phase):
            # Raw fixed IPC is intentional: only the ACK boundary is injected;
            # production command-line/capability checks have their own controls.
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as channel:
                channel.settimeout(max(.001, original_cutoff - contract.owner_clock()))
                channel.connect(str(self.directory / "owner-guard.sock"))
                pid, uid, _gid = struct.unpack("3i", channel.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                self.assertEqual((pid, uid), (self.guard.pid, os.geteuid()))
                channel.sendall((phase + " " + "f" * 64 + "\n").encode())
                return channel.recv(4096)
        pending = json.loads(request("pending"))
        self.assertEqual(pending, {"challenge": "f" * 64, "custody_sha256": self.digest, "phase": "pending",
                                   "execution_deadline": self.value["execution_deadline"],
                                   "cleanup_deadline": original_cutoff})
        self.create()
        self.assertEqual(request("ready"), b"")
        self.finish(expected=1)
        self.assertEqual(hook.read_text(), "entered-ack-write")
        self.assertLess(contract.owner_clock(), original_cutoff)
        if tamper:
            self.assertTrue(self.path.exists())
            self.assertFalse(any(row[4] in ("remove", "volume-remove") for row in self.events()))
        else:
            self.assertFalse(self.directory.exists())
            state = json.loads(self.state_path.read_text())
            self.assertFalse(state["container"]); self.assertFalse(state["volume"])
            operations = [row[4] for row in self.events()]
            self.assertLess(operations.index("remove"), operations.index("volume-remove"))
            events = self.events(); time.sleep(.1)
            self.assertEqual(self.events(), events)

    def test_parent_loss_inside_ack_send_failure_runs_nonqualifying_recovery(self):
        self.ack_send_interruption("parent-loss")

    def test_guard_cancel_inside_ack_send_failure_runs_nonqualifying_recovery(self):
        self.ack_send_interruption("cancel")

    def test_execution_expiry_inside_ack_send_failure_preserves_cleanup_reserve(self):
        self.ack_send_interruption("expiry")

    def test_ack_send_interruption_cannot_authorize_changed_custody(self):
        self.ack_send_interruption("parent-loss", tamper=True)

    def test_source_import_and_missing_custody_do_no_native_io(self):
        with self.assertRaises(OSError):
            contract.owner_read(self.directory / "missing", "00" * 32)
        self.assertEqual(self.events(), [])


if __name__ == "__main__":
    unittest.main()
