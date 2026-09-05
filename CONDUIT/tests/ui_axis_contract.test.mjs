import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
function data(path, imports = {}) {
  const exports = {};
  const js = ts.transpileModule(read(path), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  new Function('exports', 'require', js)(exports, name => {
    assert.ok(Object.hasOwn(imports, name), name);
    return imports[name];
  });
  return exports;
}
const defaults = data('../src/data/defaultSettings.ts');
const { DEFAULT_SETTINGS } = defaults;
const { SETTINGS_SCHEMA, ZNACZENIE_ZERA } = data('../src/data/settingsSchema.ts', { './defaultSettings': defaults });
const fields = new Map(SETTINGS_SCHEMA.flatMap(g => g.fields.map(f => [f.key, f])));

test('independent BreakevenOnly remains visible without smart_sl or scale_out', () => {
  const field = fields.get('breakeven_protection');
  for (const breakeven_protection of [false, true]) {
    const state = { ...DEFAULT_SETTINGS, smart_sl: false, scale_out: false, breakeven_protection };
    assert.equal(field.when?.(state) ?? true, true);
  }
  assert.match(read('../rust/crates/server/src/settings_map.rs'), /\(false, true\) => SmartSlMode::BreakevenOnly/);
});

test('manual bonus amount is visible for separate-credit simulation even with deduction OFF', () => {
  const field = fields.get('kredyt_reczny');
  assert.equal(field.when({ ...DEFAULT_SETTINGS, odlicz_kredyt: false, credit_balance_separate: true }), true);
  assert.equal(field.when({ ...DEFAULT_SETTINGS, odlicz_kredyt: false, credit_balance_separate: false }), false);
});

test('retarget option exposes its dependent and inactive-enabled states without changing defaults', () => {
  const field = fields.get('retarget_respects_final_target');
  assert.equal(DEFAULT_SETTINGS.retarget_respects_final_target, false);
  assert.equal(field.when({ ...DEFAULT_SETTINGS, cele_na_ostatnim: true }), true);
  assert.equal(field.when({ ...DEFAULT_SETTINGS, cele_na_ostatnim: false, retarget_respects_final_target: true }), true);
  assert.match(field.hint, /TP=None/);
  assert.match(field.hint, /Nie zmienia samego inkasa/);
});

test('strict SL verification tolerance zero is not labeled OFF and requires the verify mode', () => {
  assert.equal(ZNACZENIE_ZERA.sl_hit_verify_tol, undefined);
  const field = fields.get('sl_hit_verify_tol');
  assert.equal(field.when({ ...DEFAULT_SETTINGS, sl_hit_mode: 'verify', sl_hit_verify_tol: 0 }), true);
  for (const mode of ['cancel_pendings', 'close_all', 'ignore']) {
    assert.equal(field.when({ ...DEFAULT_SETTINGS, sl_hit_mode: mode }), false);
  }
});

test('exit spread / post-TP exit pause labels agree with the audited core direction', () => {
  assert.equal(ZNACZENIE_ZERA.exit_spread_mult, 'wylaczone');
  assert.equal(ZNACZENIE_ZERA.hold_after_tp_hit_min, 'wylaczone');
  assert.match(fields.get('exit_spread_mult').hint, /NIE blokadę/);
  assert.match(fields.get('hold_after_tp_hit_min').hint, /NIE blokuje nowych wejść/);
  const core = read('../rust/crates/core/src/engine.rs');
  assert.match(core, /if self\.cfg\.exit_spread_mult > 0\.0 && self\.spread_med > 0\.0/);
  assert.match(core, /let reguly_wolne = !za_swieza && !za_maly_zysk && !po_tp_hit/);
});

test('actual NumberInput allows explicit/cleared unlimited lot_max without clamping to .01', () => {
  const source = read('../src/components/ui/index.tsx');
  const tree = ts.createSourceFile('ui.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const fn = tree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'NumberInput');
  const js = ts.transpileModule(fn.getText(tree).replace(/^export /, ''), { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;
  const component = new Function('React', 'useState', 'useRef', 'useEffect', `${js};return NumberInput;`)(
    { createElement: (type, props, ...children) => ({ type, props: { ...props, children } }) },
    init => [typeof init === 'function' ? init() : init, () => {}], current => ({ current }), () => {});
  const field = fields.get('lot_max');
  assert.equal(field.min, 0);
  assert.equal(ZNACZENIE_ZERA.lot_max, 'bez-limitu');
  const writes = [];
  const treeNode = component({ value: 10, onChange: v => writes.push(v), min: field.min, zeroLabel: 'BEZ LIMITU' });
  const input = treeNode.props.children.find(n => n?.type === 'input');
  for (const raw of ['0', '', '0.01', '-1']) input.props.onBlur({ target: { value: raw } });
  assert.deepEqual(writes, [0, 0, 0.01, 0]);
  // Reproduction of the old schema constraint, against the same primitive.
  const old = component({ value: 10, onChange: v => writes.push(v), min: 0.01, zeroLabel: 'BEZ LIMITU' });
  old.props.children.find(n => n?.type === 'input').props.onBlur({ target: { value: '0' } });
  assert.equal(writes.at(-1), 0.01);
});

test('no declared zero-special numeric control has a positive minimum that makes zero unreachable', () => {
  for (const [key, field] of fields) {
    if (field.type === 'num' && ZNACZENIE_ZERA[key]) assert.ok((field.min ?? 0) <= 0, key);
  }
});
