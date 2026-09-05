import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import ts from 'typescript';

const root = resolve(import.meta.dirname, '../src');
let app;
const modules = new Map();
const jsx = (type, props) => ({ type, props });
function load(name, parent = root + '/index.ts') {
  if (name === 'react') return { useMemo: fn => fn(), useState: value => [value, () => {}], useEffect: () => {} };
  if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx };
  if (name.endsWith('.css')) return {};
  if (name === '@/store/transport') return { backendBase: () => '' };
  if (name === '@/store/AppStore') return { useApp: () => app };
  if (name === '@/i18n') return { useT: () => key => key };
  if (name === '@/components/ui') return Object.fromEntries(['Badge', 'Button', 'Card', 'Empty', 'Select'].map(key => [key, key]));
  if (name === '@/components/panels/ExportPanel') return { EksportHistorii: 'Export' };
  if (name === '@/lib/format') return { money: x => `$${x.toFixed(2)}`, num: x => String(x), duration: x => String(x), dateTime: x => String(x), toneOf: x => x >= 0 ? 'up' : 'down' };
  let file = name.startsWith('@/') ? resolve(root, name.slice(2)) : resolve(dirname(parent), name);
  if (!existsSync(file) || !/\.tsx?$/.test(file)) file = [file + '.ts', file + '.tsx', file + '/index.ts'].find(existsSync);
  assert.ok(file, name);
  file = resolve(file);
  if (modules.has(file)) return modules.get(file);
  const exports = {}; modules.set(file, exports);
  const compiled = ts.transpileModule(readFileSync(file, 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } }).outputText;
  new Function('exports', 'require', compiled)(exports, child => load(child, file));
  return exports;
}
const { closedNetProfit, closedProfitSummary } = load('@/lib/tradeProfit');
const { zMigawki } = load('@/components/chart/useTrades');
const { HistoryView } = load('@/views/HistoryView');
const record = (overrides = {}) => ({ ticket: 1, symbol: 'TEST', direction: 'BUY', volume: 0.01, openPrice: 100, closePrice: 101, openTime: 1, closeTime: 1000, profit: 20, swap: 0, commission: -2, reason: 'PARTIAL', comment: '', basketId: 1, source: 'BOT', ...overrides });
const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-10, `${a} != ${b}`);

test('gross and simulator price-plus-swap share net exactly once for either swap sign', () => {
  for (const swap of [-7, 3]) {
    const gross = record({ swap, profitBasis: 'PriceOnlyGross' });
    const sim = record({ profit: 20 + swap, swap, profitBasis: 'PricePlusSwap' });
    assert.equal(closedNetProfit(gross), 18 + swap);
    assert.equal(closedNetProfit(sim), closedNetProfit(gross));
    assert.equal(sim.profit, 20 + swap);
  }
});

test('canonical receipt amount retains fees and requires validated matching projection', () => {
  const canonical = record({ profit: 14, swap: -3, commission: -2, profitBasis: 'CanonicalClosedNetV1', netProfit: 14 });
  assert.equal(closedNetProfit(canonical), 14); // Price 20, costs -5 and fee -1 already included.
  assert.equal(closedNetProfit({ ...canonical, netProfit: null }), null);
  assert.equal(closedNetProfit({ ...canonical, netProfit: 15 }), null);
  assert.equal(closedNetProfit({ ...canonical, swap: NaN }), null);
  assert.equal(closedNetProfit({ ...canonical, profitBasis: 'ReportedNet', netProfit: undefined }), 14);
});

test('legacy unknown and nonfinite records do not become fabricated zero or a partial aggregate', () => {
  for (const basis of [undefined, 'Unknown', 'LegacySourceDefined', 'FutureBasis']) {
    assert.equal(closedNetProfit(record({ profitBasis: basis })), null);
  }
  assert.equal(closedNetProfit(record({ profitBasis: 'PriceOnlyGross', profit: Number.MAX_VALUE, swap: Number.MAX_VALUE })), null);
  const summary = closedProfitSummary([record({ profitBasis: 'PriceOnlyGross' }), record()]);
  for (const key of ['total', 'winRate', 'pf', 'best', 'worst']) assert.equal(summary[key], null, key);
  assert.equal(summary.count, 2);
  assert.equal(summary.volume, 0.02);
});

test('every partial tranche has identical net in history summary and chart markers', () => {
  const records = [
    record({ profitBasis: 'PriceOnlyGross', profit: 10, swap: -3, commission: -2 }),
    record({ profitBasis: 'PricePlusSwap', profit: 14, swap: 4, commission: -2, closeTime: 2000 }),
    record({ profitBasis: 'CanonicalClosedNetV1', profit: -4, swap: -1, commission: -1, netProfit: -4, closeTime: 3000 }),
  ];
  const marks = zMigawki(records, 'TEST');
  assert.deepEqual(marks.map(x => x.net), [5, 12, -4]);
  const summary = closedProfitSummary(records);
  assert.equal(summary.total, 13);
  assert.equal(summary.total, marks.reduce((sum, x) => sum + x.net, 0));
  near(summary.winRate, 200 / 3);
  assert.equal(summary.pf, 17 / 4);
  assert.equal(summary.best, 12);
  assert.equal(summary.worst, -4);
  assert.equal(zMigawki([record()], 'TEST')[0].net, null);
});

function nodes(tree, predicate, out = []) {
  if (Array.isArray(tree)) tree.forEach(child => nodes(child, predicate, out));
  else if (tree && typeof tree === 'object') { if (predicate(tree)) out.push(tree); nodes(tree.props?.children, predicate, out); }
  return out;
}
const textOf = tree => Array.isArray(tree) ? tree.map(textOf).join('') : tree && typeof tree === 'object' ? textOf(tree.props?.children) : tree == null ? '' : String(tree);

test('actual HistoryView rows and summary render the same net as chart, including unknowns', () => {
  const closed = [record({ profitBasis: 'PriceOnlyGross', swap: -3 }), record({ profitBasis: 'PricePlusSwap', profit: 23, swap: 3, closeTime: 2000 })];
  app = { settings: { display_currency: 'USD' }, stats: { sessionStart: 0 }, snapshot: { closed, pendingHistory: [] } };
  let tree = HistoryView();
  const rows = nodes(tree, x => x.type === 'td' && x.props.className?.startsWith('num cell-strong')).map(textOf);
  assert.deepEqual(rows, ['+$21.00', '+$15.00']);
  assert.ok(nodes(tree, x => x.type === 'b').map(textOf).includes('+$36.00'));
  near(zMigawki(closed, 'TEST').reduce((sum, x) => sum + x.net, 0), 36);
  app.snapshot.closed.push(record({ closeTime: 3000 }));
  tree = HistoryView();
  assert.equal(nodes(tree, x => x.type === 'b' && x.props.title === 'hist.netUnknown').map(textOf)[0], '—');
  assert.equal(nodes(tree, x => x.type === 'td' && x.props.title === 'hist.netUnknown').map(textOf)[0], '—');
});
