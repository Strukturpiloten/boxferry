"""Execute pinned native Renovate queue/reuse/automerge policy without platform writes."""

from __future__ import annotations

import json
import os
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class NativeRecoveryPolicyTests(unittest.TestCase):
    def test_auto_reuses_queue_branch_and_preserves_automerge_and_active_branch(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual((runtime / "source-image.txt").read_text().strip(), pin)
        script = r"""
(async () => {
  const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
  const base=process.argv[1]+'/dist/',root=process.argv[2];
  const common=JSON.parse(fs.readFileSync(root+'/renovate-presets/common.json'));
  assert.equal(common.rebaseWhen,'auto');
  const defaults=require(base+'config/defaults.js').getConfig();
  assert.equal(defaults.rebaseLabel,'rebase');
  assert.equal(common.keepUpdatedLabel,undefined);
  let queue=true,inQueue=false,conflict=false,behindCalls=0,merged=0,branchStatus="green";
  const removed=[];
  const logger=new Proxy({}, {get:()=>()=>{}});
  const scm={branchExists:async()=>true,isBranchBehindBase:async()=>{behindCalls++;return true;},
    isBranchModified:async()=>false,isBranchConflicted:async()=>conflict,
    deleteBranch:async name=>removed.push(name)};
  const platform={isBranchMergeQueueEnabled:async()=>queue,isPrInMergeQueue:async()=>inQueue,
    getBranchPr:async()=>({labels:[]}),mergePr:async()=>{merged++;return true;}};
  async function native(path,stubs) {
    const context=vm.createContext({});
    const module=new vm.SourceTextModule(fs.readFileSync(base+path,'utf8'),{context});
    await module.link(specifier=>{
      assert.ok(Object.hasOwn(stubs,specifier),'unexpected native import '+specifier);
      const exports=stubs[specifier];
      return new vm.SyntheticModule(Object.keys(exports),function(){
        for(const [name,value] of Object.entries(exports))this.setExport(name,value);
      },{context});
    });
    await module.evaluate();return module.namespace;
  }
  const reuse=await native('workers/repository/update/branch/reuse.js',{
    '../../../../logger/index.js':{logger},
    '../../../../modules/platform/scm.js':{scm},
    '../../../../modules/platform/index.js':{platform},
  });
  const config={...common,baseBranch:'main',branchName:'renovate/retained',upgrades:[]};
  let result=await reuse.shouldReuseExistingBranch(config);
  assert.equal(result.rebaseWhen,'conflicted');assert.equal(result.reuseExistingBranch,true);
  assert.equal(behindCalls,0);
  queue=false;result=await reuse.shouldReuseExistingBranch(config);
  assert.equal(result.rebaseWhen,'behind-base-branch');assert.equal(result.reuseExistingBranch,false);
  assert.equal(behindCalls,1);
  queue=true;conflict=true;result=await reuse.shouldReuseExistingBranch(config);
  assert.equal(result.reuseExistingBranch,false);conflict=false;
  const automerge=await native('workers/repository/update/pr/automerge.js',{
    '../../../../config/global.js':{GlobalConfig:{get:()=>undefined}},
    '../../../../logger/index.js':{logger},'../branch/schedule.js':{isScheduledNow:()=>true},
    '../../../../modules/platform/scm.js':{scm},
    '../../../../modules/platform/index.js':{platform},
    '../../../../modules/platform/comment.js':{ensureComment:()=>{throw Error('comment');},
      ensureCommentRemoval:()=>{throw Error('comment removal');}},
    '../branch/status-checks.js':{resolveBranchStatus:async()=>branchStatus},
  });
  const mergeConfig={...config,automergeType:'pr',pruneBranchAfterAutomerge:true};
  result=await automerge.checkAutoMerge({number:17},mergeConfig);
  assert.equal(result.automerged,false);assert.equal(result.prAutomergeBlockReason,'InMergeQueue');
  assert.equal(merged,1);assert.deepEqual(removed,[]);
  branchStatus='red';result=await automerge.checkAutoMerge({number:17},mergeConfig);
  assert.equal(result.automerged,false);assert.equal(result.prAutomergeBlockReason,'BranchNotGreen');
  assert.equal(merged,1);assert.deepEqual(removed,[]);
  branchStatus='green';
  result=await automerge.checkAutoMerge({number:17,cannotMergeReason:'blocked'},
    mergeConfig);
  assert.equal(result.automerged,false);assert.equal(result.prAutomergeBlockReason,'PlatformNotReady');
  assert.equal(merged,1);assert.deepEqual(removed,[]);
  inQueue=true;await automerge.checkAutoMerge({number:17},mergeConfig);
  assert.equal(merged,1);assert.deepEqual(removed,[]);
  inQueue=false;queue=false;result=await automerge.checkAutoMerge({number:17},mergeConfig);
  assert.equal(result.automerged,true);assert.deepEqual(removed,['renovate/retained']);
  removed.length=0;
  const prune=await native('workers/repository/finalize/prune.js',{
    '../../../constants/error-messages.js':{},
    '../../../util/regex.js':{regEx:value=>new RegExp(value)},
    '../../../config/global.js':{GlobalConfig:{get:()=>undefined}},
    '../../../util/string.js':{uniqueStrings:(value,index,array)=>array.indexOf(value)===index},
    '../../../logger/index.js':{logger},
    '../../../util/git/index.js':{getBranchList:()=>['renovate/retained'],setUserRepoConfig:()=>{}},
    '../../../modules/platform/scm.js':{scm},
    '../../../modules/platform/index.js':{platform:{findPr:async()=>null}},
    '../../../modules/platform/comment.js':{ensureComment:()=>{throw Error('comment');}},
    '../process/index.js':{isMultiBaseBranch:()=>false},
    '../reconfigure/utils.js':{getReconfigureBranchName:value=>value+'reconfigure'},
    '@sindresorhus/is':{isNonEmptyStringAndNotWhitespace:
      value=>typeof value==='string'&&!!value.trim()},
  });
  await prune.pruneStaleBranches({repoIsOnboarded:true,defaultBranch:'main',
    branchPrefix:'renovate/',
    pruneStaleBranches:true},['renovate/retained']);
  assert.deepEqual(removed,[]);
})().catch(error=>{console.error(error);process.exit(1);});
"""
        result = subprocess.run(
            [
                "node",
                "--experimental-vm-modules",
                "--no-warnings",
                "-e",
                script,
                str(runtime),
                str(ROOT),
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
