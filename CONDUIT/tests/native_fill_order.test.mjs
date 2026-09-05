import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const source=readFileSync(new URL('../mql5/CONDUIT_XT.mq5',import.meta.url),'utf8');
const start=source.indexOf('void OdswiezBilety()');
const begin=source.indexOf('{',start)+1;
const end=source.indexOf('// pozycje zamknięte przez brokera',begin);
assert.ok(start>=0 && end>begin,'actual pending-receipt registration body');
const body=(source.slice(begin,end)+'}').replace(/\b(?:int|long|ulong|bool|double)\s+/g,'let ');

function rig(){
  const basket={state:1,npend:5,pend:[11,12,13,14,15],pend_lv:[0,1,2,3,4],pend_top:[false,false,false,false,false],
    npos:1,pos:[5],pos_lv:[-2],nlv:5,lv_price:[110,109,108,107,106],lv_fill_ts:[0,0,0,0,0],
    lv_filled:[false,false,false,false,false],side:0,has_sl:false};
  const e={g_nb:1,g_b:[basket],ST_DONE:3,ST_RISKFREE:2,ST_WORKING:1,In_ConfirmedExitRetry:true,
    MAXTK:100,g_now:1000,In_VirtualSl:false,In_VirtualSlAll:false,g_ps_vsl:[],
    g_premia_n:0,g_premia_lepiej:0,g_premia_usd:0,XAU_CONTRACT:100,
    POSITION_PRICE_OPEN:'price',POSITION_VOLUME:'volume',positions:new Set([5,11,14,15]),
    orders:new Set([12]),receipts:[],SideSign:()=>1,PsEnsure:()=>0};
  e.OrderSelect=t=>e.orders.has(t);e.PositionSelectByTicket=t=>e.positions.has(t);
  e.PositionGetDouble=k=>k==='price'?100:0.01;
  e.ZapiszWlasciciela=t=>e.receipts.push(t);
  e.refresh=new Function('env',`with(env){return function(){${body}}}`)(e);
  return e;
}

test('simultaneous native receipts preserve submission order across still-pending and canceled legs',()=>{
  const e=rig();e.refresh();const b=e.g_b[0];
  assert.deepEqual(b.pos.slice(0,b.npos),[5,11,14,15]);
  assert.deepEqual(b.pos_lv.slice(0,b.npos),[-2,0,3,4]);
  assert.deepEqual(b.pend.slice(0,b.npend),[12]);
  assert.deepEqual(e.receipts,[11,14,15]);
  e.refresh();assert.deepEqual(b.pos.slice(0,b.npos),[5,11,14,15]);
  assert.deepEqual(e.receipts,[11,14,15]);
});

test('later receipts append after existing positions and duplicate confirmations retain their original place',()=>{
  const e=rig();e.g_b[0].pos=[5,11];e.g_b[0].pos_lv=[-2,-1];e.g_b[0].npos=2;
  e.refresh();e.orders.clear();e.positions.add(12);e.refresh();
  const b=e.g_b[0];assert.deepEqual(b.pos.slice(0,b.npos),[5,11,14,15,12]);
  assert.deepEqual(b.pos_lv.slice(0,b.npos),[-2,0,3,4,1]);
  assert.equal(b.npend,0);
});
