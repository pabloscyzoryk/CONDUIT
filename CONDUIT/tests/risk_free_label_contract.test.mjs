import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const read = p => readFileSync(new URL(p, import.meta.url), 'utf8');
const jsx = (text, names, values) => new Function('React', ...names,
  ts.transpileModule('const node = (' + text + ');', { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText + ';return node;')(
    { createElement: (type, props, ...children) => ({ type, props: { ...props, children } }) }, ...values);
function find(source, predicate) {
  const tree = ts.createSourceFile('actual.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let result;
  function visit(n) { if (!result && predicate(n, tree)) result = n; ts.forEachChild(n, visit); }
  visit(tree); assert.ok(result); return result.getText(tree);
}

test('actual history reason tooltip explains RF without changing the profit cell', () => {
  const source = read('../src/views/HistoryView.tsx');
  const fragment = find(source, n => ts.isJsxElement(n) && n.openingElement.tagName.getText() === 'span' && n.openingElement.attributes.getText().includes('hist.reason.riskFree.hint'));
  const c = { reason: 'RISK_FREE', profit: -16.08 };
  const result = jsx(fragment, ['c', 't', 'Badge', 'REASON_TONE', 'REASON_LABEL'], [c, k => k, 'Badge', { RISK_FREE: 'info' }, { RISK_FREE: 'hist.reason.riskFree' }]);
  assert.equal(result.props.title, 'hist.reason.riskFree.hint');
  assert.equal(c.profit, -16.08);
  assert.match(source, /toneOf\(c\.profit\)/);
  assert.match(source, /money\(Math\.abs\(c\.profit\), cur\)/);
});

test('actual basket RF badge follows current riskFree state, not a fabricated secured guarantee', () => {
  const source = read('../src/components/panels/BasketsPanel.tsx');
  const fragment = find(source, n => ts.isJsxExpression(n) && n.expression && n.expression.getText().startsWith('b.riskFree &&'));
  const expression = fragment.slice(1, -1);
  for (const riskFree of [false, true]) {
    const result = jsx(expression, ['b', 'tt', 'Badge'], [{ riskFree, secured: true }, k => k, 'Badge']);
    if (riskFree) assert.equal(result.props.title, 'bask.riskFreeState.hint');
    else assert.equal(result, false, 'secured alone must not invent this state');
  }
  assert.match(read('../rust/crates/server/src/ui.rs'), /risk_free: matches!\(b\.state, conduit_core::BasketState::RiskFree\)/);
});
