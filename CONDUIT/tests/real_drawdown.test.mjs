import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = file => readFileSync(new URL('../src/' + file, import.meta.url), 'utf8');
function compile(file, imports = {}) {
  const output = ts.transpileModule(source(file), { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText;
  const exports = {};
  new Function('exports', 'require', output)(exports, name => {
    if (Object.hasOwn(imports, name)) return imports[name];
    throw new Error(name);
  });
  return exports;
}
const { realDrawdown, observeDemoEquity } = compile('lib/realDrawdown.ts');
const DAY = 86_400_000, initial = { day: 100, startEquity: 200, minEquity: 200 };

test('RDD retains the day trough through rallies and differs from peak drawdown', () => {
  let day = initial;
  for (const equity of [250, 180, 230]) day = observeDemoEquity(day, equity, 100 * DAY + 1000);
  assert.deepEqual(realDrawdown(day), { amount: 20, percent: 10 });
  assert.equal(day.startEquity, 200);
  assert.equal(day.minEquity, 180);
  day = observeDemoEquity(day, 300, 100 * DAY + 2000);
  assert.deepEqual(realDrawdown(day), { amount: 20, percent: 10 });
  assert.deepEqual(realDrawdown({ ...initial, minEquity: -20 }), { amount: 220, percent: 110.00000000000001 });
});

test('unknown anchors never become a fabricated zero; known unbroken days can be zero', () => {
  for (const value of [undefined, null, {}, { ...initial, startEquity: null },
    { ...initial, minEquity: null }, { ...initial, minEquity: NaN },
    { ...initial, startEquity: Infinity }, { ...initial, day: null },
    { ...initial, startEquity: true }, { ...initial, minEquity: '180' }]) {
    assert.equal(realDrawdown(value), null);
  }
  assert.deepEqual(realDrawdown(initial), { amount: 0, percent: 0 });
  assert.deepEqual(realDrawdown({ ...initial, startEquity: 0, minEquity: -20 }), { amount: 20, percent: null });
  assert.deepEqual(realDrawdown({ ...initial, startEquity: -1, minEquity: -21 }), { amount: 20, percent: null });
  assert.deepEqual(realDrawdown({ ...initial, minEquity: 250 }), { amount: 0, percent: 0 });
});

test('demo day reset is independent of workstation timezone and yesterday trough', () => {
  const previousZone = process.env.TZ;
  try {
    for (const zone of ['UTC', 'Europe/Warsaw', 'America/New_York']) {
      process.env.TZ = zone;
      const yesterday = { ...initial, minEquity: 100 };
      assert.equal(observeDemoEquity(yesterday, 230, 101 * DAY - 1).minEquity, 100);
      const next = observeDemoEquity(yesterday, 230, 101 * DAY);
      assert.deepEqual(next, { day: 101, startEquity: 230, minEquity: 230 });
      assert.deepEqual(realDrawdown(observeDemoEquity(next, 207, 101 * DAY + 1)), { amount: 23, percent: 10 });
    }
  } finally { if (previousZone === undefined) delete process.env.TZ; else process.env.TZ = previousZone; }
});

test('actual RDD component has a PL/EN tooltip, amount, percent and a focusable unknown state', () => {
  const dictionaries = { pl: compile('i18n/pl.ts').PL, en: compile('i18n/en.ts').EN };
  const jsx = { jsx: (type, props) => ({ type, props }), jsxs: (type, props) => ({ type, props }) };
  const flat = node => node == null ? '' : typeof node === 'object'
    ? (Array.isArray(node) ? node.map(flat).join('') : flat(node.props?.children)) : String(node);
  for (const language of ['pl', 'en']) {
    const t = (key, params = {}) => {
      assert.ok(dictionaries[language][key], key);
      return dictionaries[language][key].replace(/\{([^}]+)\}/g, (_, name) => String(params[name] ?? `{${name}}`));
    };
    const format = compile('lib/format.ts', {
      '@/data/defaultSettings': { FX_RATES: { USD: { symbol: '$', rate: 1 } } },
      '@/i18n': { getLanguage: () => language, t },
    });
    const Tooltip = () => {};
    const { RealDrawdown } = compile('components/layout/RealDrawdown.tsx', {
      'react/jsx-runtime': jsx, '@/components/ui': { Tooltip }, '@/i18n': { useT: () => t },
      '@/lib/format': format, '@/lib/realDrawdown': { realDrawdown },
    });
    const tree = RealDrawdown({ day: { ...initial, minEquity: 180 }, currency: 'USD' });
    assert.equal(tree.type, Tooltip);
    assert.equal(flat(tree.props.children), language === 'pl' ? 'RDD: 10,0%' : 'RDD: 10.0%');
    assert.equal(tree.props.children.props.tabIndex, 0);
    const tooltip = flat(tree.props.content);
    assert.ok(tooltip.includes(language === 'pl' ? '$20,00' : '$20.00'), tooltip);
    assert.ok(tooltip.includes(language === 'pl' ? '10,00%' : '10.00%'), tooltip);
    assert.ok(tooltip.includes(language === 'pl' ? 'nie od szczytu' : 'not the peak'));
    const unknown = RealDrawdown({ day: null, currency: 'USD' });
    assert.equal(flat(unknown.props.children), 'RDD: —');
    assert.ok(flat(unknown.props.content).includes(language === 'pl' ? 'nieznane' : 'unknown'));
    assert.ok(!flat(unknown.props.content).includes('$0'));
    const zeroStart = RealDrawdown({ day: { ...initial, startEquity: 0, minEquity: -20 }, currency: 'USD' });
    assert.equal(flat(zeroStart.props.children), 'RDD: —');
    assert.ok(flat(zeroStart.props.content).includes(language === 'pl' ? '$20,00' : '$20.00'));
    assert.ok(flat(zeroStart.props.content).includes('—%'));
  }
});
