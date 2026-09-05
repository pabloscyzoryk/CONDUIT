import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
function actualBody(name){
  const hit=new RegExp(`bool\\s+${name}\\s*\\([^)]*\\)\\s*\\{`).exec(source);
  if(!hit){
    // RED uses the actual pre-fix Phase-A occupancy body, not an invented
    // surrogate. A broker-confirmed missing ticket still returns occupied.
    const old=/bool live_pos = false;([\s\S]*?)if\(live_pos\) continue;/.exec(source);
    assert.ok(old,'actual legacy PlaceGrid body');
    return `let i=lv;let live_pos=false;${old[1].replace(/\bint\s+/g,'let ')}return live_pos;`;
  }
  assert.ok(hit,`actual MQL helper ${name}`);
  const start=hit.index+hit[0].length;let end=start,depth=1;
  while(depth&&end<source.length){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;end++;}
  return source.slice(start,end-1).replace(/\bulong\s+(\w+)\[\];/g,'let $1=[];').replace(/\b(?:int|bool)\s+/g,'let ');
}
function rig(on=true){
  const e={In_ConfirmedExitRetry:on,g_b:[{npos:2,pos:[11,12],pos_lv:[0,1],npend:1,pend:[21],pend_lv:[2]}],
    positions:[12],orders:[21],complete:true,ArraySize:x=>x.length,snapshotCalls:0};
  e.ExitOwnedSnapshot=(_bi,p,o)=>{e.snapshotCalls++;p.push(...e.positions);o.push(...e.orders);return e.complete;};
  e.GridLevelHasLivePosition=new Function('env',`with(env){return function(bi,lv){${actualBody('GridLevelHasLivePosition')}}}`)(e);
  return e;
}
test('actual native grid frees a broker-confirmed absent cached ticket in the same tick, OFF retains old occupancy',()=>{
  const on=rig();assert.equal(on.GridLevelHasLivePosition(0,0),false);assert.equal(on.GridLevelHasLivePosition(0,1),true);
  const off=rig(false);assert.equal(off.GridLevelHasLivePosition(0,0),true);assert.equal(off.snapshotCalls,0);
});
test('incomplete ownership and unknown position/order level cannot become a free slot',()=>{
  const e=rig();e.complete=false;assert.equal(e.GridLevelHasLivePosition(0,0),true);
  e.complete=true;e.positions.push(99);assert.equal(e.GridLevelHasLivePosition(0,0),true);
  e.positions=[12];e.g_b[0].pos_lv[1]=-1;assert.equal(e.GridLevelHasLivePosition(0,0),true);
  e.g_b[0].pos_lv[1]=1;e.orders.push(22);assert.equal(e.GridLevelHasLivePosition(0,0),true);
});
test('both native market-budget and placement phases use the same occupancy proof',()=>{
  const start=source.indexOf('int PlaceGrid(');const end=source.indexOf('\nvoid ',start);
  const body=source.slice(start,end);
  assert.equal((body.match(/GridLevelHasLivePosition\(bi, i\)/g)||[]).length,2);
});
