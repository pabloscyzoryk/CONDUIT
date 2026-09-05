import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
const start=source.indexOf('void FastAddonSweep()');
let begin=source.indexOf('{',start),end=begin+1,depth=1;
while(depth){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;end++;}
let body=source.slice(begin+1,end-1)
  .replace(/\b(?:int|double|long|ulong|bool)\s+/g,'let ')
  .replace(/\((?:long|int)\)/g,'')
  .replace('let n = 0;', 'let n = 3;'); // supplied physical quote-history fixture
function rig(){
  const basket={had_positions:true,fast_addons:0,tp_stage:1,last_addon_ts:0,side:0,
    ntp:1,tps:[130],id:1,npos:0,pos:[],pos_lv:[],sl:90,has_sl:true};
  const e={In_FastAddonMoveUsd:1,In_FastAddonMax:1,In_FastAddonWindowS:10,
    In_FastAddonCooldownS:10,In_FastAddonMinStage:0,In_MlMinFastAddon:0,
    In_FastAddonLotMult:3,In_LotMin:.01,In_LotMax:5,MAXTK:100,g_now:20000,
    g_nb:1,g_b:[basket],g_open_request_count:0,g_cnt_fastaddon:0,g_cnt_order:0,
    validTarget:true,sendResult:true,transmit:true,volumes:[],MathMax:Math.max,MathMin:Math.min,
    RoundLot:x=>x,LotSize:()=>4,VolOldestInWindow:()=>100,MidPx:()=>110,
    MaxOpenPositionsEff:()=>0,ExitRiskAllowed:()=>true,EntryReviewBlocked:()=>false,
    Alive:()=>true,SideSign:()=>1,LiczPozycje:()=>0,MarginesPozwala:()=>true,
    IntegerToString:String,ZapiszWlasciciela:()=>{}};
  e.TpIsValid=()=>e.validTarget;
  e.WyslijRynek=(_bi,_level,volume)=>{if(e.transmit){e.g_open_request_count++;e.volumes.push(volume);}return e.sendResult;};
  e.run=new Function('env',`with(env){return function(){${body}}}`)(e);
  return e;
}
test('native addon local invalid TP keeps capacity and waits the configured cooldown',()=>{
  const e=rig();e.validTarget=false;e.run();
  assert.equal(e.g_open_request_count,0);assert.equal(e.g_b[0].fast_addons,0);assert.equal(e.g_b[0].last_addon_ts,e.g_now);
  e.validTarget=true;e.g_now+=9999;e.run();assert.equal(e.g_open_request_count,0);
  e.g_now++;e.run();assert.equal(e.g_b[0].fast_addons,1);assert.equal(e.g_open_request_count,1);
});
test('native addon applies the cap after its multiplier to the transmitted request',()=>{
  const e=rig();e.run();assert.deepEqual(e.volumes,[5]);
});
test('transmitted refusal consumes one attempt, a pre-send risk guard does not',()=>{
  const e=rig();e.sendResult=false;e.run();assert.equal(e.g_b[0].fast_addons,1);assert.equal(e.g_b[0].last_addon_ts,e.g_now);
  e.run();assert.equal(e.g_open_request_count,1);
  const local=rig();local.transmit=false;local.sendResult=false;local.run();
  assert.equal(local.g_b[0].fast_addons,0);assert.equal(local.g_b[0].last_addon_ts,0);
});
