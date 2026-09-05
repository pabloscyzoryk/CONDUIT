import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = path => readFileSync(new URL('../src/' + path, import.meta.url), 'utf8');
function compile(code, imports={}) {
  const exports = {};
  new Function('exports','require', ts.transpileModule(code,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText)(exports,name=>{
    if (Object.hasOwn(imports,name)) return imports[name];
    throw new Error(name);
  });
  return exports;
}
function format() { return compile(source('lib/format.ts'), {'@/data/defaultSettings':{FX_RATES:{}}, '@/i18n':{getLanguage:()=> 'en',t:(k,a)=>a ? `${k}:${a.n}` : k}}); }
const clock=compile(source('lib/clock.ts'));
function chartLabels() {
  const text=source('components/chart/chartRender.ts');
  const ast=ts.createSourceFile('chartRender.ts',text,ts.ScriptTarget.Latest,true);
  const keep=ast.statements.filter(n=>ts.isVariableStatement(n)&&n.declarationList.declarations.some(d=>['labelDate','TF_LABEL'].includes(d.name.getText(ast))));
  assert.equal(keep.length,2);
  return compile(keep.map(n=>n.getText(ast)).join('\n')+'\nexport { TF_LABEL };');
}
for (const zone of ['UTC','Europe/Warsaw','America/New_York']) {
  test(`broker labels and chart history stay exact across DST in ${zone}`,()=>{
    const previous=process.env.TZ; process.env.TZ=zone;
    try {
      const f=format(), c=chartLabels();
      for(const day of ['2026-01-15','2026-07-15','2026-10-24','2026-10-26','2026-11-15']) {
        const t=Date.parse(`${day}T12:34:56Z`);
        assert.equal(f.brokerTime(t),'12:34:56');
        assert.equal(c.TF_LABEL['1m'](c.labelDate(t,0)),'12:34');
        assert.equal(c.TF_LABEL['1d'](c.labelDate(t,0)), `${Number(day.slice(8))}.${day.slice(5,7)}`);
      }
      assert.equal(f.brokerTime(0),'—'); assert.equal(f.brokerDateTime(NaN),'—');
      if(zone==='Europe/Warsaw') {
        const utc=Date.parse('2026-07-15T09:34:56Z');
        assert.equal(f.time(utc),'11:34:56','actual UTC log timestamps retain local display');
        assert.equal(f.brokerTime(utc),'09:34:56','explicit UTC event display is independent');
      }
    } finally { if(previous===undefined) delete process.env.TZ;else process.env.TZ=previous; }
  });
}
test('broker timestamp cannot masquerade as a fresh quote, including old or future records',()=>{
  const now=Date.parse('2026-09-04T12:00:00Z');
  const q={time:now+3*3600000-10*60000,timeBasis:'broker_wall',timeUtc:null};
  assert.equal(clock.quoteUtcTime(q,true),null);
  assert.equal(clock.quoteState(clock.quoteUtcTime(q,true),now),'nieznane');
  assert.equal(clock.quoteUtcTime({time:now},true),null,'unqualified legacy is unknown');
  assert.equal(clock.quoteState(now+3*3600000,now),'nieznane','bad future UTC does not mean fresh');
  assert.equal(clock.quoteState(now-90001,now),'martwe');
  assert.equal(clock.quoteState(now-20001,now),'stare');
  assert.equal(clock.quoteState(now-1000,now),'swieze');
  assert.equal(clock.quoteState(clock.quoteUtcTime({...q,timeUtc:now-600000},true),now),'martwe');
  assert.equal(clock.quoteUtcTime({time:now-1000,timeBasis:'utc'},true),now-1000);
  assert.equal(clock.quoteUtcTime({time:now-1000},false),now-1000,'local demo has a known UTC clock');
});
test('short history window compares broker values to its source clock, never workstation UTC',()=>{
  const utc=Date.parse('2026-09-04T12:00:00Z'), broker=utc+3*3600000;
  const from=clock.historyFrom('10',utc-3600000,utc,true,broker);
  const rows=[{closeTime:broker-5*60000},{closeTime:broker-60*60000}];
  assert.deepEqual(rows.filter(r=>r.closeTime>=from),[rows[0]],'60-minute old deal must not pass last10-minute filter');
  assert.equal(clock.historyFrom('session',utc,utc,true,broker),null,'UTC session anchor cannot be used as broker time');
  assert.equal(clock.historyFrom('10',utc,utc,true,0),null);
  assert.equal(clock.historyFrom('all',utc,utc,true,0),0);
  assert.equal(clock.historyFrom('10',utc,utc,false,broker),utc-600000);
});
test('production presentation uses explicit clocks at the previously affected call sites',()=>{
  for(const f of ['PositionsPanel','PendingsPanel']) assert.match(source(`components/panels/${f}.tsx`),/app\.live \? brokerTime/);
  assert.match(source('components/panels/BasketsPanel.tsx'),/app\.live \? `\$\{brokerDateTime\(b.createdAt\)/);
  assert.match(source('components/panels/ChatPanel.tsx'),/m\.timeBasis === "utc" \? "UTC"/);
  assert.match(source('views/HistoryView.tsx'),/historyFrom\(period/);
  assert.doesNotMatch(source('components/chart/useCandles.ts'),/getTimezoneOffset\(/);
  assert.doesNotMatch(source('components/chart/chartRender.ts'),/\.get(Hours|Minutes|Date|Month)\(/);
});
