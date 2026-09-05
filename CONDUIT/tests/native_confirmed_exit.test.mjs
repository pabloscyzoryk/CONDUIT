import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// ACTUAL MQL state-machine bodies, primitive syntax translation only.
// The broker is synthetic; this is NOT execution of MetaTrader or its MQL VM.
const source = readFileSync(new URL('../mql5/CONDUIT_XT.mq5', import.meta.url), 'utf8');
function extract(name) {
  const m = new RegExp('\\b(?:void|bool|int)\\s+' + name + '\\s*\\(([^)]*)\\)\\s*\\{').exec(source);
  assert.ok(m, `actual production function ${name}`);
  const start = m.index+m[0].length;
  let depth=1,end=start;
  for(;depth&&end<source.length;end++){if(source[end]==='{')depth++;else if(source[end]==='}')depth--;}
  assert.equal(depth,0);
  return {args:m[1],body:source.slice(start,end-1)};
}
function rig(enabled=true) {
  const env = {In_ConfirmedExitRetry:enabled, g_now:10000, g_nb:2,
    g_b:[{state:1,exit_pending:false,exit_last_attempt:0,exit_reason:''},
         {state:1,exit_pending:false,exit_last_attempt:0,exit_reason:''}],
    ST_WORKING:1,ST_DONE:3,g_halted:'',ArraySize:a=>a.length,
    positions:[],orders:[],calls:[],complete:true,cancelReject:0,closeReject:0,
    fillOnCancel:false,partialClose:0,
  };
  env.ExitOwnedSnapshot=(bi,p,o)=>{
    p.splice(0,p.length,...env.positions.filter(x=>x.bi===bi).map(x=>x.id));
    o.splice(0,o.length,...env.orders.filter(x=>x.bi===bi).map(x=>x.id));
    return env.complete;
  };
  env.ExitRememberLivePositions=()=>{};
  env.ExitCancelOwned=(bi,id)=>{
    env.calls.push(['cancel',id,env.g_now]);
    if(env.cancelReject-->0)return false;
    env.orders=env.orders.filter(x=>x.id!==id);
    if(env.fillOnCancel){env.fillOnCancel=false;env.positions.push({bi,id:id+100,vol:.04});return false;}
    return true;
  };
  env.ExitCloseOwned=(bi,id,reason)=>{
    const p=env.positions.find(x=>x.id===id&&x.bi===bi);
    if(!p)return true;
    env.calls.push(['close',id,env.g_now,reason,p.vol]);
    if(env.closeReject-->0)return false;
    if(env.partialClose-->0){p.vol/=2;return false;}
    env.positions=env.positions.filter(x=>x!==p);return true;
  };
  for(const name of ['ConfirmedExitPending','ExitRiskAllowed','AttemptConfirmedExit','RequestConfirmedExit','RetryConfirmedExits']) {
    const f=extract(name);
    const args=f.args.split(',').filter(x=>x.trim()).map(x=>x.trim().replace(/^(?:int|bool|string)\s+/,''));
    const body=f.body.replace(/\bulong\s+(\w+)\[\];/g,'let $1=[];')
      .replace(/\b(?:int|bool|long|string)\s+/g,'let ');
    env[name]=new Function('env',`with(env){return function(${args.join(',')}){${body}}}`)(env);
  }
  return env;
}

test('rejected close retains intent and original reason; retry cadence independent of TP/retrace/halt',()=>{
  const r=rig();r.positions=[{bi:0,id:7,vol:.08},{bi:1,id:8,vol:.09}];r.closeReject=1;
  r.RequestConfirmedExit(0,'BANK_ALL');
  assert.equal(r.g_b[0].state,1);assert.equal(r.g_b[0].exit_pending,true);
  assert.equal(r.ExitRiskAllowed(0),false);assert.equal(r.ExitRiskAllowed(1),true);
  r.g_now=10500;r.RequestConfirmedExit(0,'DIFFERENT_REASON');
  assert.equal(r.calls.length,1);assert.equal(r.g_b[0].exit_reason,'BANK_ALL');
  r.g_halted='HALT';r.g_now=10999;r.RetryConfirmedExits();assert.equal(r.calls.length,1);
  r.g_now=11000;r.RetryConfirmedExits();
  assert.equal(r.g_b[0].state,3);assert.equal(r.g_b[0].exit_pending,false);
  assert.deepEqual(r.positions,[{bi:1,id:8,vol:.09}]);
  assert.equal(r.calls[1][3],'BANK_ALL');
  r.g_now=12000;r.RetryConfirmedExits();assert.equal(r.calls.length,2);
});

test('cancel rejection followed by fill closes new ownership and retries remaining partial volume',()=>{
  const r=rig();r.orders=[{bi:0,id:17}];r.cancelReject=1;
  r.RequestConfirmedExit(0,'EXPIRED');
  assert.equal(r.g_b[0].exit_pending,true);assert.equal(r.g_b[0].state,1);
  r.fillOnCancel=true;r.partialClose=1;r.g_now=11000;r.RetryConfirmedExits();
  assert.equal(r.orders.length,0);assert.equal(r.positions[0].id,117);
  assert.equal(r.positions[0].vol,.02);assert.equal(r.g_b[0].exit_pending,true);
  r.g_now=12000;r.RetryConfirmedExits();
  assert.equal(r.positions.length,0);assert.equal(r.g_b[0].state,3);
  assert.deepEqual(r.calls.filter(x=>x[0]==='close').map(x=>x[4]),[.04,.02]);
});

test('broker absence is idempotent but unknown ownership/snapshot is not proof of flat',()=>{
  const r=rig();r.complete=false;r.RequestConfirmedExit(0,'CLOSEALL');
  assert.equal(r.g_b[0].state,1);assert.equal(r.g_b[0].exit_pending,true);
  r.complete=true;r.g_now=10001;r.RetryConfirmedExits();
  assert.equal(r.g_b[0].state,3);assert.equal(r.g_b[0].exit_pending,false);
  assert.deepEqual(r.calls,[]);
});

test('OFF leaves all confirmed-intent behavior dormant',()=>{
  const r=rig(false);r.positions=[{bi:0,id:7,vol:.08}];r.RequestConfirmedExit(0,'BANK_ALL');
  r.RetryConfirmedExits();assert.equal(r.ExitRiskAllowed(0),true);
  assert.equal(r.g_b[0].exit_pending,false);assert.deepEqual(r.calls,[]);
  assert.equal(r.positions.length,1);
});

test('new exposure and edit handlers reject the same closing basket before touching broker or plan',()=>{
  for(const name of ['WyslijRynek','WyslijLimit','PlaceGrid','ApplyEntryEdit']) {
    const f=extract(name);
    assert.match(f.body,/if\(!ExitRiskAllowed\(bi\)\) return/);
    assert.ok(f.body.indexOf('ExitRiskAllowed(bi)')<f.body.indexOf(';')+1,`${name}: first statement guard`);
  }
  const tick=extract('OnTick').body;
  assert.ok(tick.indexOf('RetryConfirmedExits();')>tick.indexOf('OdswiezBilety();'));
  assert.ok(tick.indexOf('RetryConfirmedExits();')<tick.indexOf('CheckGuards();'));
  assert.match(extract('CancelPendings').body,/if\(In_ConfirmedExitRetry\)/);
  assert.match(extract('ExitCancelOwned').body,/res\.retcode == TRADE_RETCODE_DONE/);
});

test('all Rust full-basket exit scopes route to the same native session-retained intent',()=>{
  const routes={HandleEntry:'OPPOSITE',HandleTpHit:'BANK_ALL',HandleOutAtEntry:'OAE',
    HandleSlHit:'SLHIT',ExpireStale:'WYGASL',ExpireOld:'WYGASL',FastFillCheck:'FASTFILL',
    ZoneExitAdverseSweep:'ZONEEXIT'};
  for(const [fn,reason] of Object.entries(routes)) {
    assert.match(extract(fn).body,new RegExp('RequestConfirmedExit\\(bi2?, "'+reason+'"\\)'),fn);
  }
  assert.match(extract('CloseEverything').body,/RequestConfirmedExit\(bi, g_powod_zamk\)/);
  const dispatch=extract('WykonajWiadomosc').body;
  assert.ok(dispatch.indexOf('In_ConfirmedExitRetry && kind == "CLOSEALL"')<dispatch.indexOf('int bi = TargetBasket(mi)'));
  assert.match(extract('OdswiezBilety').body,/state == ST_DONE && !In_ConfirmedExitRetry/,'closed cached tickets still reconcile after native Done');
});

test('pending-exit ownership blocks every discretionary family, but not basket counting',()=>{
  for(const fn of ['CoverLateFills','RiskfreePass','ManagePositions','RearmPass','MarketLadderPass',
    'ReentryPass','FastAddonSweep','RelotPendings','RevExitSweep','PendingTtl','DokonczOdroczoneKasowanie',
    'ExpireStale','ExpireOld','FastFillCheck','EnforcePositionLimit','ZoneExitAdverseSweep','WykryjCeleZCeny']) {
    assert.match(extract(fn).body,/ExitRiskAllowed\(bi\)/,fn);
  }
  for(const fn of ['PonowStopy','CloseOrQueue','SweepQueuedExits','RedukujEkspozycje'])
    assert.match(extract(fn).body,/ExitTicketPending\(/,fn);
  assert.equal(extract('Alive').body.trim(),'return g_b[i].state != ST_DONE;','closing basket still occupies its cap');
});

test('native fault harness is default OFF, tester-only before tests/orders, and uses actual order RPCs',()=>{
  assert.match(source,/input int\s+In_TestExitScenario = 0;/);
  const init=extract('OnInit').body;
  assert.ok(init.indexOf('if(!MQLInfoInteger(MQL_TESTER))')<init.indexOf('TestSppTargetPlanReset()'));
  assert.match(extract('ExitFaultScenarioTick').body,/if\(!MQLInfoInteger\(MQL_TESTER\)/);
  assert.match(extract('ExitCancelOwned').body,/In_TestExitScenario > 0 && MQLInfoInteger\(MQL_TESTER\)/);
  assert.match(extract('ZamknijPozycje').body,/actual partial deal/);
  assert.match(extract('ExitFaultScenarioTick').body,/closed realized was lost or duplicated/);
});
