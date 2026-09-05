import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
const start=source.indexOf('double PozZysk('),begin=source.indexOf('{',start),end=source.indexOf('\n  }',begin);
const body=source.slice(begin+1,end).replace(/\b(?:int|double|long)\s+/g,'let ');
function gross(position,bid,ask){
  const env={POSITION_TYPE:'side',POSITION_TYPE_BUY:0,POSITION_PRICE_OPEN:'open',POSITION_VOLUME:'volume',XAU_CONTRACT:100,
    PositionSelectByTicket:()=>!!position,PositionGetInteger:k=>position[k],PositionGetDouble:k=>position[k],
    ExitPx:s=>s===0?bid:ask,SideSign:s=>s===0?1:-1};
  return new Function('env',`with(env){return function(t){${body}}}`)(env)(1);
}
test('overnight bank ranking uses gross like core, while the receipt remains net',()=>{
  const old={side:0,open:4033.14,volume:.01,swap:-2.42};
  const newer={side:0,open:4034.29,volume:.01,swap:0};
  const oldGross=gross(old,4041.31,4041.51),newGross=gross(newer,4041.31,4041.51);
  assert.ok(oldGross>newGross);assert.ok(oldGross+old.swap<newGross+newer.swap);
  assert.ok(Math.abs(oldGross-8.17)<1e-8);assert.ok(Math.abs(newGross-7.02)<1e-8);
});
test('gross selector preserves sub-cent prices and uses ask for shorts',()=>{
  const p={side:1,open:4000,volume:.01,swap:100};
  assert.ok(Math.abs(gross(p,3998.999,3999.003)-.997)<1e-8);
  p.swap=-100;assert.ok(Math.abs(gross(p,3998.999,3999.003)-.997)<1e-8);
  assert.equal(gross(null,1,2),0);
});
test('position swap is reserved for final accounting, not strategy selectors',()=>{
  assert.equal((source.match(/PositionGetDouble\(POSITION_SWAP\)/g)||[]).length,1);
  assert.match(source,/g_final_positions\[n\]\.swap = PositionGetDouble\(POSITION_SWAP\)/);
});
