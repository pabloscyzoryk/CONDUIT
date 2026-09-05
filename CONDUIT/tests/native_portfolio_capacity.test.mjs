import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
const start=source.indexOf('int ProfitBudgetAvailable(');
let begin=source.indexOf('{',start),end=begin+1,depth=1;
while(depth){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;end++;}
const body=source.slice(begin+1,end-1).replace(/\b(?:int|double|long|ulong|bool)\s+/g,'let ');
function rig(){
  const e={In_MaxPortfolioRisk:5,In_ProfitBudgetArmPct:0,In_ProfitBudgetKeepPct:50,
    In_ProfitBudgetDeployPct:100,In_Magic:123,_Symbol:'XAUUSD',g_day:0,g_now:1000,
    g_day_start_eq:0,g_day_peak_eq:0,g_bid:4000,g_ask:4000.2,equity:700,
    positions:[],orders:[],g_ps_vsl:[],XAU_CONTRACT:100,
    MathMax:Math.max,MathMin:Math.min,MathIsValidNumber:Number.isFinite,
    DayOf:()=>1,ACCOUNT_EQUITY:'equity',POSITION_MAGIC:'magic',POSITION_SYMBOL:'symbol',
    POSITION_SL:'sl',POSITION_VOLUME:'volume',POSITION_TYPE:'side',POSITION_TYPE_BUY:0,
    ORDER_MAGIC:'magic',ORDER_SYMBOL:'symbol',ORDER_SL:'sl',ORDER_PRICE_OPEN:'entry',
    ORDER_VOLUME_CURRENT:'volume',ORDER_TYPE:'side',ORDER_TYPE_BUY_LIMIT:0,
    ORDER_TYPE_BUY_STOP:2,ORDER_TYPE_BUY_STOP_LIMIT:4,ORDER_TYPE_SELL_LIMIT:1,
    ORDER_TYPE_SELL_STOP:3,ORDER_TYPE_SELL_STOP_LIMIT:5,PsIdx:()=>-1};
  e.PbPositive=x=>Number.isFinite(x)&&x>0;
  e.AccountInfoDouble=()=>e.equity;e.PositionsTotal=()=>e.positions.length;e.OrdersTotal=()=>e.orders.length;
  e.PositionGetTicket=i=>{e.position=e.positions[i];return i+1;};
  e.OrderGetTicket=i=>{e.order=e.orders[i];return i+1;};
  e.PositionGetDouble=e.PositionGetInteger=e.PositionGetString=k=>e.position[k];
  e.OrderGetDouble=e.OrderGetInteger=e.OrderGetString=k=>e.order[k];
  e.SideSign=side=>side===0?1:-1;e.ExitPx=side=>side===0?e.g_bid:e.g_ask;
  e.run=new Function('env',`with(env){return function(){let remaining,error;const state=(()=>{${body}})();return {state,remaining,error};}}`)(e);
  e.pos=(side,sl,volume=.01)=>({magic:123,symbol:'XAUUSD',side,sl,volume});
  e.pending=(side,entry,sl,volume=.01)=>({...e.pos(side,sl,volume),entry});
  return e;
}
const near=(a,b)=>assert.ok(Math.abs(a-b)<1e-8,`${a} != ${b}`);

test('portfolio alone needs no daily anchor and ignores unused reserve fields',()=>{
  const e=rig();e.In_ProfitBudgetKeepPct=NaN;e.In_ProfitBudgetDeployPct=Infinity;
  assert.deepEqual(e.run(),{state:1,remaining:35,error:''});
  e.In_MaxPortfolioRisk=0;e.g_bid=NaN;assert.equal(e.run().state,0);
});
test('portfolio remains active before profit reserve arm and validates the requested anchor',()=>{
  const e=rig();e.In_ProfitBudgetArmPct=20;e.g_day=1;e.g_day_start_eq=600;e.g_day_peak_eq=700;
  near(e.run().remaining,35);e.g_day=0;assert.equal(e.run().error,'UnknownDayAnchor');
});
test('positions are marked from current exit and one pool is subtracted from both caps',()=>{
  const e=rig();e.In_ProfitBudgetArmPct=10;e.g_day=1;e.g_day_start_eq=600;e.g_day_peak_eq=700;
  e.positions.push({...e.pos(0,3999,.1),old_entry:3000},e.pos(1,4001,.1));
  e.orders.push(e.pending(0,3990,3980));
  near(e.run().remaining,7); // min(portfolio35, reserve50) - (10 + 8 + 10)
  e.positions[0].old_entry=9000;near(e.run().remaining,7);
  e.In_ProfitBudgetDeployPct=50;near(e.run().remaining,0);
});
test('missing protection and invalid quote fail closed while confirmed cancellation releases risk',()=>{
  const e=rig();e.orders.push(e.pending(0,3990,3980));near(e.run().remaining,25);
  e.orders=[];near(e.run().remaining,35);
  e.positions.push(e.pos(0,0));assert.equal(e.run().error,'MissingStop');
  e.positions=[];e.g_ask=3999;assert.equal(e.run().error,'InvalidQuote');
});
test('nonfinite settings or capacity overflow cannot be hidden by a finite second cap',()=>{
  const e=rig();e.In_MaxPortfolioRisk=Infinity;assert.equal(e.run().error,'InvalidSettings');
  e.In_MaxPortfolioRisk=1e308;e.In_ProfitBudgetArmPct=10;e.g_day=1;e.g_day_start_eq=600;e.g_day_peak_eq=700;
  assert.equal(e.run().error,'InvalidAccount');
  e.g_day=0;assert.equal(e.run().error,'UnknownDayAnchor');
});
