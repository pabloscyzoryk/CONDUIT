import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
function bodyOf(name){
  const start=source.search(new RegExp(`(?:void|bool|int) ${name}\\(`));
  assert.ok(start>=0,name);let begin=source.indexOf('{',start)+1,end=begin,depth=1;
  while(depth){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;end++;}
  return source.slice(begin,end-1);
}
const changed=new Function('previous','current',bodyOf('NativeEntryPlanChanged').replace(/\bint\s+/g,'let '));
const reset=new Function('g_b','bi','MAXTP',bodyOf('NativeResetEntryProgress').replace(/\bint\s+/g,'let '));
const basket=()=>({zone_lo:100,zone_hi:101,has_sl:true,sl:90,ntp:2,tps:[110,120],
  tp_stage:2,plan_observed_stage:3,zone_touched:true,drop_armed:true,drop_po_ts:12,last_tp_ts:34,
  tp_touch_ts:[56,78],tphit_sig_ts:[90,91],secured:true,fast_addons:1,realized:5});

test('only changed effective entry geometry triggers reset, independently of source metadata',()=>{
  const previous=basket();
  for(const mutate of [b=>b.zone_lo--,b=>b.zone_hi++,b=>b.sl--,b=>b.has_sl=false,
    b=>b.tps[0]++,b=>{b.ntp=1;}]){
    const current=structuredClone(previous);mutate(current);assert.equal(changed(previous,current),true);
  }
  const same=structuredClone(previous);same.comment='synthetic revision';same.entry_lo=98;
  same.tp_stage=0;same.created_ts=999;same.tps.push(999);
  assert.equal(changed(previous,same),false);
  const absent=structuredClone(previous);absent.has_sl=false;
  const absentAgain=structuredClone(absent);absentAgain.sl=999;
  assert.equal(changed(absent,absentAgain),false);
});

test('empty SPP wire field is an empty target list, while explicit numeric targets are retained',()=>{
  const read=new Function('fields','count','targets','MAXTP','StringLen','StringToDouble',
    bodyOf('NativeReadSppTargets').replace(/\bint\s+/g,'let '));
  const targets=[99,98,97];
  assert.equal(read(['key','nan','nan',''],4,targets,32,x=>x.length,Number),0);
  assert.deepEqual(targets,[99,98,97]);
  assert.equal(read(['key','nan','nan','110','120',''],6,targets,32,x=>x.length,Number),2);
  assert.deepEqual(targets,[110,120,97]);
  assert.equal(read(['key','nan','nan','0'],4,targets,32,x=>x.length,Number),1);
  assert.equal(targets[0],0);
  assert.match(bodyOf('WykonajWiadomosc'),/ntp=NativeReadSppTargets\(f,nf,tt\)/);
});

test('pending TP correction follows current stage rather than per-grid final assignment',()=>{
  const target=new Function('g_b','bi','result',bodyOf('NativeCorrectionPendingTarget')
    .replace(/\bint\s+/g,'let ').replace('target=g_b[bi].tps[index]','result.target=g_b[bi].tps[index]'));
  const state={ntp:3,tps:[110,120,130],tp_stage:1},result={};
  assert.equal(target([state],0,result),true);assert.equal(result.target,120);
  state.tp_stage=9;target([state],0,result);assert.equal(result.target,130);
  state.ntp=0;assert.equal(target([state],0,result),false);
  const body=bodyOf('WykonajWiadomosc');
  assert.match(body,/if\(idx==0\)idx=1/);
  const correction=body.slice(body.indexOf('else if(kind == "TPCORR")'),body.indexOf('else if(kind == "SPP")'));
  assert.match(correction,/NativeCorrectionPendingTarget/);assert.equal(correction.includes('TargetForEx'),false);
});

test('entry progress clears observations while retaining risk state, source delivery and economics',()=>{
  const current=basket();reset([current],0,2);
  assert.deepEqual([current.tp_stage,current.plan_observed_stage,current.zone_touched,current.drop_armed,
    current.drop_po_ts,current.last_tp_ts,current.tp_touch_ts],[0,0,false,false,0,0,[0,0]]);
  assert.deepEqual([current.secured,current.fast_addons,current.realized,current.tphit_sig_ts],[true,1,5,[90,91]]);
});

test('working ENTRY reset is outside Armed rebuild, guarded by material plan and rollback restores snapshot',()=>{
  const body=bodyOf('ApplyEntryEdit');
  assert.match(body,/bool plan_changed = NativeEntryPlanChanged\(committed, g_b\[bi\]\)/);
  assert.match(body,/if\(plan_changed\) NativeResetEntryProgress\(bi\)/);
  assert.ok(body.indexOf('NativeResetEntryProgress(bi)')<body.indexOf('if(rebuild && !NativeEntryGridUnchanged'));
  assert.match(body,/if\(!clean\)[\s\S]*g_b\[bi\] = committed;/);
  assert.equal(body.includes('In_ResetTpOnTargetEdit'),false);
  assert.match(bodyOf('ApplySppTargetPlan'),/if\(In_ResetTpOnTargetEdit && changed\)/);
});

const sameSource=new Function('previous','current',bodyOf('NativeSameEntrySource').replace(/\bint\s+/g,'let '));
const raw=()=>({known:true,side:0,is_limit:true,is_stop:false,lo:100,hi:101,has_sl:true,sl:90,
  tp_open:false,has_warstwy_offset:false,warstwy_offset:0,ntp:2,tps:[110,120]});

test('raw source equality retains optional fields and exact values before target expansion',()=>{
  const previous=raw();
  for(const mutate of [s=>s.known=false,s=>s.side=1,s=>s.is_limit=false,s=>s.is_stop=true,
    s=>s.lo--,s=>s.hi++,s=>s.has_sl=false,s=>s.sl--,s=>s.tp_open=true,
    s=>s.has_warstwy_offset=true,s=>s.ntp=1,s=>s.tps[1]++]){
    const next=structuredClone(previous);mutate(next);assert.equal(sameSource(previous,next),false);
  }
  const harmless=structuredClone(previous);harmless.comment='synthetic edit';harmless.warstwy_offset=99;
  assert.equal(sameSource(previous,harmless),true);
  previous.has_warstwy_offset=true;harmless.has_warstwy_offset=true;
  assert.equal(sameSource(previous,harmless),false);
  const body=bodyOf('WykonajWiadomosc');
  assert.ok(body.indexOf('source.tps[i]=tps[i]')<body.indexOf('PrzygotujCele(side, tps, ntp)'));
});

test('actual source wrapper performs no execution after managed SL/TP on equal source; review never commits',()=>{
  const body=bodyOf('ApplySourceEntryEdit').replace(/\bint\s+/g,'let ');
  const wrap=new Function('g_b','bi','source','targets','count','ExitRiskAllowed','SourceWithdrawn',
    'EntryReviewBlocked','NativeSameEntrySource','ApplyEntryEdit',body);
  const initial=raw(), state={...basket(),side:0,entry_source:initial,sl:105,tps:[115,125]};
  let calls=0;
  const execute=()=>wrap([state],0,structuredClone(initial),[116,126,136],3,()=>true,()=>false,
    ()=>Boolean(state.entry_review),sameSource,()=>calls++);
  execute();assert.equal(calls,0);assert.equal(state.tp_stage,2);assert.equal(state.sl,105);
  const changed=raw();changed.tps[0]++;
  wrap([state],0,changed,[111,120],2,()=>true,()=>false,()=>Boolean(state.entry_review),sameSource,
    ()=>{calls++;state.entry_review=true;});
  assert.equal(calls,1);assert.equal(state.entry_source,initial);
  state.entry_review=false;
  wrap([state],0,changed,[111,120],2,()=>true,()=>false,()=>false,sameSource,()=>calls++);
  assert.equal(calls,2);assert.equal(state.entry_source,changed);
});

test('Armed grid tolerance is separate from exact plan progress reset',()=>{
  const actual=bodyOf('NativeEntryGridUnchanged').replace(/\bint\s+/g,'let ')
    .replace(/\(long\)\(([^()]*)\)/g,'Math.trunc($1)');
  const unchanged=new Function('previous','current','MathAbs',actual);
  const previous=basket(), current=structuredClone(previous);current.zone_lo+=2e-10;
  assert.equal(changed(previous,current),true);
  assert.equal(unchanged(previous,current,Math.abs),true);
  current.zone_lo+=1e-8;assert.equal(unchanged(previous,current,Math.abs),false);
  assert.match(bodyOf('ApplyEntryEdit'),/else \{ g_b\[bi\]\.has_sl=false;g_b\[bi\]\.sl=0.0; \}/);
});
