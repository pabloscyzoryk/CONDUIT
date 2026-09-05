import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
const start=source.indexOf('bool NativeStrategyRealized('), begin=source.indexOf('{',start),end=source.indexOf('\n  }',begin);
let body=source.slice(begin+1,end)
  .replaceAll('return false;','return {valid:false,value};')
  .replaceAll('return true;','return {valid:true,value};')
  .replaceAll('return MathIsValidNumber(value);','return {valid:MathIsValidNumber(value),value};');
const strategy=new Function('side','open_price','close_price','volume','swap','broker_net','net_mode',
  `let value=0;const MathIsValidNumber=Number.isFinite,XAU_CONTRACT=100,SideSign=s=>s===0?1:-1;${body}`);
test('confirmed raw geometry keeps the rearm zero boundary independently of cash cents',()=>{
  const opens=[4595.80,4594.43,4593.44,4592.19,4591.13],cash=[1.77,.40,-.59,-1.84,-2.90];
  let raw=0,actual=0;
  for(let i=0;i<opens.length;i++){
    const result=strategy(1,opens[i],4594.03,.01,0,cash[i],false);
    assert.equal(result.valid,true);raw+=result.value;actual+=cash[i];
  }
  const floating=(4593.57-4596.73)*-1*100*.01;
  assert.ok(raw+floating>=0);assert.ok(actual+floating<0);
  assert.ok(raw+floating<1e-10);
});
test('strategy basis includes allocated swap and the actual partial volume once',()=>{
  assert.deepEqual(strategy(0,4000,4001,.04,-.75,999,false),{valid:true,value:3.25});
  assert.deepEqual(strategy(0,4000,4001,.02,-.25,999,false),{valid:true,value:1.75});
});
test('canonical net basis preserves the complete broker result without price derivation',()=>{
  assert.deepEqual(strategy(-1,NaN,NaN,NaN,NaN,12.345,true),{valid:true,value:12.345});
  assert.equal(strategy(0,4000,4001,.01,0,Infinity,true).valid,false);
});
test('missing geometry cannot silently become a strategy receipt',()=>{
  for(const args of [[0,0,4001,.01,0],[1,4000,NaN,.01,0],[0,4000,4001,0,0],[-1,4000,4001,.01,0],[0,4000,4001,.01,NaN]])
    assert.equal(strategy(...args,0,false).valid,false);
  assert.match(source,/g_rej_realized_review\[ri\]=false/);
  assert.match(source,/g_rej_bid\[i\]==g_b\[bi\]\.id && g_rej_realized_review\[i\]/);
});
