import test from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';
import { load, read, defaults } from './ui_alias_harness.mjs';

test('fresh public install initializes notifications without assuming six sample channels', () => {
  const source = read('../src/store/AppStore.tsx');
  const tree = ts.createSourceFile('AppStore.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let init;
  function visit(node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(tree) === '[notify, setNotifyState]') init = node.initializer.getText(tree);
    ts.forEachChild(node, visit);
  }
  visit(tree);
  assert.ok(init);
  const js = ts.transpileModule(`const initial = ${init};`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const evaluate = new Function('useState', 'storage', 'CHANNELS', `${js}; return initial;`);
  for (const channels of [[], load('data/telegram').CHANNELS]) {
    const initial = evaluate(fn => fn(), { load: (_key, value) => value }, channels);
    assert.deepEqual(initial.channels, []);
    assert.equal(initial.summaryIntervalMin, 240);
  }
});

test('empty public AI catalogue never crashes or fabricates a model policy', () => {
  assert.deepEqual(load('data/telegram').AI_MODELS, []);
  const { aiEffectiveSettings } = load('store/aiPolicy');
  const base = { ...defaults, ai_mode: true, ai_model: 'loaded-at-runtime', runner_trail_start: 42 };
  const out = aiEffectiveSettings(base, base.ai_model);
  assert.deepEqual(out, base);
  assert.notEqual(out, base);
});
