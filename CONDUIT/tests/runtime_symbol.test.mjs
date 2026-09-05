import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const source = read('../src/store/runtimeSymbol.ts');
const js = ts.transpileModule(source, { compilerOptions: {
  target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS,
} }).outputText;
const exports = {};
new Function('exports', js)(exports);
const { resolveBotSymbol, runtimeQuote } = exports;
const quote = symbol => ({ symbol, bid: 4448.53, ask: 4448.89, spread: .36,
  time: 1788219381939, change: 0, changePct: 0, dayHigh: 4450, dayLow: 4400 });
const allQuotes = { XAUUSD: quote('XAUUSD'), 'XAUUSD.s': quote('XAUUSD.s'), BTCUSD: quote('BTCUSD') };

test('PUPrime AUTO uses bound .s even when bare gold and manually searched quotes exist', () => {
  const config = Object.freeze({ mt5_symbol: '' });
  const symbol = resolveBotSymbol(true, { mt5: 'connected', resolvedSymbol: 'XAUUSD.s' }, config.mt5_symbol, 'XAUUSD');
  assert.equal(symbol, 'XAUUSD.s');
  assert.equal(runtimeQuote(symbol, allQuotes), allQuotes['XAUUSD.s']);
  assert.equal(config.mt5_symbol, '', 'runtime binding must never pin persisted AUTO');
});

test('Vantage -> PUPrime -> Vantage follows the current atomic binding', () => {
  for (const symbol of ['XAUUSD', 'XAUUSD.s', 'XAUUSD']) {
    assert.equal(resolveBotSymbol(true, { mt5: 'connected', resolvedSymbol: symbol }, 'XAUUSD.s', 'XAUUSD'), symbol);
  }
});

test('disconnect, reconnect and missing binding never borrow stale/watched prices', () => {
  for (const connection of [
    { mt5: 'disconnected', resolvedSymbol: 'XAUUSD.s' },
    { mt5: 'connecting', resolvedSymbol: 'XAUUSD' },
    { mt5: 'connected', resolvedSymbol: '' },
    { mt5: 'connected', resolvedSymbol: '  ' },
    { mt5: 'connected' },
  ]) {
    const symbol = resolveBotSymbol(true, connection, '', 'XAUUSD');
    assert.equal(symbol, '');
    const q = runtimeQuote(symbol, allQuotes);
    assert.ok(Number.isNaN(q.bid));
    assert.equal(q.time, 0);
  }
});

test('legacy explicit setting remains supported but new unbound runtime never falls back', () => {
  assert.equal(resolveBotSymbol(true, { mt5: 'connected' }, ' EURUSD ', 'XAUUSD'), 'EURUSD');
  assert.equal(resolveBotSymbol(true, { mt5: 'connected', resolvedSymbol: '' }, 'EURUSD', 'XAUUSD'), '');
  assert.equal(resolveBotSymbol(false, { mt5: 'disconnected' }, '', 'XAUUSD'), 'XAUUSD');
});

test('missing, mismatched and invalid quote cannot activate manual ticket', () => {
  for (const quotes of [{}, { 'XAUUSD.s': quote('XAUUSD') },
    { 'XAUUSD.s': { ...quote('XAUUSD.s'), bid: 0 } },
    { 'XAUUSD.s': { ...quote('XAUUSD.s'), ask: NaN } },
    { 'XAUUSD.s': { ...quote('XAUUSD.s'), ask: 1 } }]) {
    assert.ok(Number.isNaN(runtimeQuote('XAUUSD.s', quotes).bid));
  }
});

test('actual AppStore primary declarations use the runtime quote without generator in LIVE', () => {
  const app = read('../src/store/AppStore.tsx');
  const tree = ts.createSourceFile('AppStore.tsx', app, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const declarations = [];
  const visit = n => {
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && ['symbolBota', 'primary'].includes(n.name.text)) declarations.push(n.getText(tree));
    ts.forEachChild(n, visit);
  };
  visit(tree);
  assert.equal(declarations.length, 2);
  const compiled = ts.transpileModule(declarations.map(d => `const ${d};`).join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const render = new Function('live', 'connection', 'settingsView', 'quotesView', 'quotes',
    'PRIMARY_SYMBOL', 'getQuote', 'resolveBotSymbol', 'runtimeQuote', `${compiled}; return primary;`);
  const noGenerator = () => { throw new Error('LIVE tried synthetic price'); };
  for (const symbol of ['XAUUSD.s', 'XAUUSD']) {
    const actual = render(true, { mt5: 'connected', resolvedSymbol: symbol }, { mt5_symbol: '' },
      allQuotes, {}, 'XAUUSD', noGenerator, resolveBotSymbol, runtimeQuote);
    assert.equal(actual, allQuotes[symbol]);
  }
  const missing = render(true, { mt5: 'connected', resolvedSymbol: '' }, { mt5_symbol: '' },
    allQuotes, {}, 'XAUUSD', noGenerator, resolveBotSymbol, runtimeQuote);
  assert.ok(Number.isNaN(missing.bid));
});

test('all bot-instrument consumers share primary and do not interpret AUTO again', () => {
  for (const path of ['chart/ChartsSection.tsx', 'chart/TradingChart.tsx', 'panels/TicketPanel.tsx', 'panels/LotAuto.tsx']) {
    const code = read('../src/components/' + path);
    assert.match(code, /app\.primary\.symbol/, path);
    assert.doesNotMatch(code, /app\.settings\.mt5_symbol\s*\|\|/, path);
  }
  assert.doesNotMatch(read('../src/components/layout/Shell.tsx'), /q\.symbol\s*\|\|\s*PRIMARY_SYMBOL/);
});
