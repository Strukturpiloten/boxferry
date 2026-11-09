"""Execute pinned native Renovate policy with authored effects and no platform writes."""

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

    def test_github_native_distinguishes_direct_merge_enqueue_and_queue_failure(self) -> None:
        runtime = Path(os.environ.get("BOXFERRY_RENOVATE_PACKAGE", ROOT / "target/tools/renovate"))
        pin = json.loads((ROOT / "automation/renovate-runtime.json").read_text())["image"]
        self.assertEqual((runtime / "source-image.txt").read_text().strip(), pin)
        # Load the complete unmodified GitHub module, not a platform.mergePr stub
        # or an extracted/reimplemented merge algorithm. All effects terminate at
        # authored synthetic imports; an unexpected dependency call fails closed.
        script = r"""
(async () => {
  const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
  const base=process.argv[1]+'/dist/';
  const context=vm.createContext({});
  const queries=new vm.SourceTextModule(
    fs.readFileSync(base+'modules/platform/github/graphql.js','utf8'),{context});
  await queries.link(()=>{throw Error('unexpected query import');});
  await queries.evaluate();
  const events=[];
  let directSuccess=false,queueResult='success',queueEnabled=true;
  let pr;
  const logger=new Proxy({}, {get:()=>()=>{}});
  const githubApi={
    putJson:async(path,options)=>{
      assert.equal(path,'repos/Strukturpiloten/compose-lens/pulls/17/merge');
      assert.equal(options.body.merge_method,'squash');
      events.push('PUT direct merge');
      if(!directSuccess)
        throw Object.assign(Error('synthetic protected rejection'),{statusCode:403});
      return {body:{merged:true}};
    },
    requestGraphql:async(query,options)=>{
      if(query===queries.namespace.repoInfoQuery) {
        assert.equal(options.variables.owner,'Strukturpiloten');
        assert.equal(options.variables.name,'compose-lens');
        assert.equal(options.readOnly,true);
        return {data:{repository:{id:'repo-node',nameWithOwner:'Strukturpiloten/compose-lens',
          defaultBranchRef:{name:'main'},squashMergeAllowed:true,
          mergeQueue:queueEnabled?{id:'queue-node'}:null,issues:{nodes:[]}}}};
      }
      assert.equal(query,queries.namespace.enqueuePullRequestMutation);
      assert.equal(options.variables.pullRequestId,'pr-node-17');
      assert.equal(options.count,1);
      events.push('GraphQL enqueue');
      if(queueResult==='throw') throw Error('synthetic queue transport failure');
      if(queueResult==='errors') return {errors:[{message:'synthetic queue rejection'}]};
      return {data:{enqueuePullRequest:{mergeQueueEntry:{id:'entry-node'}}}};
    },
  };
  const overrides={
    '../../../_virtual/_rolldown/runtime.js':{__exportAll:()=>({})},
    '../../../logger/index.js':{logger},
    '../../../util/url.js':{parseUrl:value=>new URL(value)},
    '../../../util/regex.js':{regEx:value=>new RegExp(value)},
    '../../../config/global.js':{GlobalConfig:{get:()=>undefined}},
    '../../../util/host-rules.js':{find:()=>({})},
    '../../../instrumentation/index.js':{instrument:async(_name,operation)=>operation()},
    '../../../util/git/index.js':{initRepo:async(config)=>{
      assert.equal(config.repository,'Strukturpiloten/compose-lens');
    }},
    '../util.js':{repoFingerprint:()=> 'synthetic-fingerprint'},
    './common.js':{githubApi,mapMergeStartegy:strategy=>{
      assert.equal(strategy,undefined);return undefined;
    }},
    './schema.js':{GithubIssue:{array:()=>({catch:()=>({parse:()=>[]})})}},
    './issue.js':{GithubIssueCache:{addIssuesToReconcile:()=>{}}},
    './pr.js':{getPrCache:async()=>({17:pr}),updatePrCache:()=>{}},
    './utils.js':{isGithubEnterpriseServer:()=>false,getRepoUrl:()=> 'synthetic-repository-url'},
    '@sindresorhus/is':{isNonEmptyObject:value=>!!value&&Object.keys(value).length>0,
      isNonEmptyString:value=>typeof value==='string'&&value.length>0},
  };
  const source=fs.readFileSync(base+'modules/platform/github/index.js','utf8');
  // Derive only the ESM linkage names. Unused imports throw if newly exercised;
  // the native source itself is passed verbatim to SourceTextModule.
  const imports=new Map();
  for(const match of source.matchAll(/^import (.+) from ["']([^"']+)["'];$/gm)) {
    const clause=match[1],specifier=match[2];
    const names=clause.startsWith('{')?clause.slice(1,-1).split(',').map(
      name=>name.trim().split(/\s+as\s+/)[0]):['default'];
    const values=Object.fromEntries(names.map(name=>[name,
      overrides[specifier]?.[name]??(()=>{
        throw Error('unexpected native dependency '+specifier+':'+name);
      })]));
    imports.set(specifier,values);
  }
  const module=new vm.SourceTextModule(source,{context});
  await module.link(specifier=>{
    if(specifier==='./graphql.js') return queries;
    assert.ok(imports.has(specifier),'unrecognized native import '+specifier);
    const values=imports.get(specifier);
    return new vm.SyntheticModule(Object.keys(values),function(){
      for(const [name,value] of Object.entries(values))this.setExport(name,value);
    },{context});
  });
  await module.evaluate();
  const native=module.namespace;
  async function scenario(direct,queue,result,expected,expectedEvents) {
    directSuccess=direct;queueEnabled=queue;queueResult=result;
    pr={number:17,node_id:'pr-node-17',targetBranch:'main',state:'open'};
    await native.initRepo({repository:'Strukturpiloten/compose-lens',
      renovateUsername:'renovate[bot]'});
    events.length=0;
    assert.equal(await native.mergePr({branchName:'renovate/retained',id:17}),expected);
    assert.deepEqual(events,expectedEvents);
  }
  // A direct success returns before enqueue: this is not fresh queue evidence.
  await scenario(true,true,'success',true,['PUT direct merge']);
  await scenario(false,true,'success',true,['PUT direct merge','GraphQL enqueue']);
  await scenario(false,true,'errors',false,['PUT direct merge','GraphQL enqueue']);
  await scenario(false,true,'throw',false,['PUT direct merge','GraphQL enqueue']);
  await scenario(false,false,'success',false,['PUT direct merge']);
})().catch(error=>{console.error(error);process.exit(1);});
"""
        result = subprocess.run(
            ["node", "--experimental-vm-modules", "--no-warnings", "-e", script, str(runtime)],
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
