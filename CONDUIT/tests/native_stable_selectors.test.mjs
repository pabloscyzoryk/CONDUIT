import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
function bodyOf(name){
  const start=source.search(new RegExp(`(?:void|int|bool) ${name}\\(`));
  assert.ok(start>=0,name);let begin=source.indexOf('{',start)+1,end=begin,depth=1;
  while(depth){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;end++;}
  return source.slice(begin,end-1);
}
const sort=new Function('tickets','keys','count','descending',bodyOf('NativeStableSortTickets').replace(/\b(?:ulong|double|int)\s+/g,'let '));

test('native stable ascending and descending selectors preserve ties after a better key moves forward',()=>{
  for(const descending of [false,true]){
    const tickets=[11,12,13,14,15];const keys=descending?[1,1,2,1,2]:[2,2,1,2,1];
    sort(tickets,keys,tickets.length,descending);
    assert.deepEqual(tickets,[13,15,11,12,14]);
    assert.deepEqual(keys,descending?[2,2,1,1,1]:[1,1,2,2,2]);
  }
});
test('selector snapshot prefix leaves unused capacity untouched and supports already-sorted equal keys',()=>{
  const tickets=[7,5,9,99],keys=[1,1,1,-100];sort(tickets,keys,3,false);
  assert.deepEqual(tickets,[7,5,9,99]);assert.deepEqual(keys,[1,1,1,-100]);
});
test('all position strategy selectors use stable keys, while explicit exposure ticket tie-breaks remain separate',()=>{
  assert.match(bodyOf('HandleRiskFree'),/NativeStableSortTickets\(order,distance,n,false\)/);
  assert.match(bodyOf('HandleRiskFree'),/NativeSortProfit\(order,n,true\)/);
  assert.match(bodyOf('RiskfreePass'),/NativeSortProfit\(zywe,n,true\)/);
  const bank=source.slice(source.indexOf('// Stable bank order'),source.indexOf('// Stable bank order')+170);
  assert.match(bank,/NativeSortProfit\(live,n,In_BankFrom!=0\)/);
  assert.match(bodyOf('JestRunneremWgGlebokosci'),/NativeStableSortTickets\(tk,op,m,g_b\[bi\]\.side!=0\)/);
  assert.ok(source.includes('NativeStableSortTickets(tk,op,n,side!=0);'));
});
test('shallow pending ranking preserves equal-price source order before deleting unique indices',()=>{
  const body=bodyOf('CancelPendingsKeep');
  const start=body.indexOf('// Stable insertion order'),end=body.indexOf('// kasujemy',start);
  const actual=body.slice(start,end).replace(/\b(?:int|double)\s+/g,'let ');
  const rank=new Function('g_b','bi','idx','px','n',actual);
  const idx=[0,1,2,3],px=[100,100,101,100];rank([{side:0}],0,idx,px,4);
  assert.deepEqual(idx,[2,0,1,3]);
});
