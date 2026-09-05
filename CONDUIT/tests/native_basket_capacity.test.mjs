import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
function bodyOf(name){
  const start=source.search(new RegExp(`(?:bool|int|void) ${name}\\(`));
  assert.ok(start>=0,name);let begin=source.indexOf('{',start),end=begin+1,depth=1;
  while(depth){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;end++;}
  return source.slice(begin+1,end-1);
}
function js(body){return body.replace('bool retain[MAXB];','let retain=Array(MAXB).fill(false);')
  .replace('ulong positions[],orders[];','let positions=[],orders=[];')
  .replace(/\b(?:bool|int|long|ulong|double|string)\s+/g,'let ')
  .replace(/\((?:string|long|ulong)\)/g,'')
  .replace('g_b[write]=g_b[i]','g_b[write]=structuredClone(g_b[i])');}
function rig(){
  const e={MAXB:600,ST_DONE:3,g_nb:600,g_cnt_reject:0,g_now:1,In_Diag:true,g_handle_diag:1,
    INVALID_HANDLE:-1,g_b:Array.from({length:600},(_,id)=>({id,state:3,exit_pending:false,entry_review:false})),
    owned:new Set(),unknown:false,checks:[],logs:[],ArraySize:x=>x.length};
  e.Alive=i=>e.g_b[i].state!==3;
  e.ExitOwnedSnapshot=(i,p,_o)=>{e.checks.push(e.g_b[0].id);if(e.owned.has(e.g_b[i].id))p.push(1);return !e.unknown;};
  e.PrintFormat=(...v)=>e.logs.push(v);e.FileWrite=()=>{};
  e.run=new Function('env',`with(env){return function(entry_kind){${js(bodyOf('NativeEnsureBasketCapacity'))}}}`)(e);
  return e;
}
test('compaction resolves all ownership before moving and preserves live state and identities',()=>{
  const e=rig();e.g_b[590].state=1;e.g_b[590].nested={ticket:123};
  e.owned.add(595);e.g_b[597].exit_pending=true;e.g_b[599].entry_review=true;
  assert.equal(e.run('Entry'),true);assert.equal(e.g_nb,4);
  assert.deepEqual(e.g_b.slice(0,4).map(x=>x.id),[590,595,597,599]);
  assert.equal(e.g_b[0].nested.ticket,123);assert.ok(e.checks.every(x=>x===0));
  assert.equal(e.g_cnt_reject,0);
});
test('600 live baskets and uncertain ownership refuse explicitly without losing exposure',()=>{
  const e=rig();for(const b of e.g_b)b.state=0;
  assert.equal(e.run('MarketOpen'),false);assert.equal(e.g_nb,600);assert.equal(e.g_cnt_reject,1);
  assert.equal(e.logs[0][1],'MarketOpen');assert.deepEqual(e.g_b.map(x=>x.id),Array.from({length:600},(_,i)=>i));
  const unknown=rig();unknown.unknown=true;assert.equal(unknown.run('Entry'),false);
  assert.equal(unknown.g_nb,600);assert.equal(unknown.g_cnt_reject,1);
});
test('both actual entry families use capacity proof and clear the newly allocated slot',()=>{
  for(const name of ['HandleEntry','HandleMkt']){
    const b=bodyOf(name);
    assert.match(b,/if\(!NativeEnsureBasketCapacity\("(?:Entry|MarketOpen)"\)\)return;/);
    assert.match(b,/int bi = g_nb; g_nb\+\+;\s+ZeroMemory\(g_b\[bi\]\);/);
  }
});

test('actual result aggregation includes more than 600 sparse lifetime IDs and counts every close once',()=>{
  const ids=Array.from({length:601},(_,i)=>i===600?9000000:i+1);
  const rows=ids.map((id,i)=>({magic:7,entry:1,position:i+1,profit:id===9000000?2:1,swap:-.1,commission:-.2}));
  rows.push({magic:99,entry:1,position:1,profit:999,swap:0,commission:0});
  const e={g_nrej:ids.length,g_rej_bid:ids,g_rej_tk:ids.map((_,i)=>i+1),g_rej_msg:ids.map(x=>x+100),
    In_Magic:7,DEAL_MAGIC:'magic',DEAL_ENTRY:'entry',DEAL_ENTRY_OUT:1,DEAL_POSITION_ID:'position',
    DEAL_PROFIT:'profit',DEAL_SWAP:'swap',DEAL_COMMISSION:'commission',ArraySize:a=>a.length,
    ArrayResize:(a,n)=>{while(a.length<n)a.push({});a.length=n;},TimeCurrent:()=>1,HistorySelect:()=>true,
    HistoryDealsTotal:()=>rows.length,HistoryDealGetTicket:i=>i+1,
    HistoryDealGetInteger:(d,k)=>rows[d-1][k],HistoryDealGetDouble:(d,k)=>rows[d-1][k]};
  e.NativeBasketResultIndex=new Function('env',`with(env){return function(results,id,source_id){${js(bodyOf('NativeBasketResultIndex'))}}}`)(e);
  const collect=new Function('env',`with(env){return function(results){${js(bodyOf('CollectNativeBasketResults'))}}}`)(e);
  const results=[];assert.equal(collect(results),true);assert.equal(results.length,601);
  assert.ok(results.every(x=>x.closes===1));assert.equal(results.at(-1).id,9000000);
  assert.ok(Math.abs(results.at(-1).profit-1.7)<1e-9);
  rows.push({magic:7,entry:1,position:999999,profit:1,swap:0,commission:0});
  assert.equal(collect(results),false);assert.equal(results.length,601);
});
