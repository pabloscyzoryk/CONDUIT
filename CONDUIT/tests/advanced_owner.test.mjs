import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const read = p => readFileSync(new URL(p, import.meta.url), 'utf8');
const modules = new Map();
function load(path) {
  if (modules.has(path)) return modules.get(path);
  const source = read('../src/' + path + '.ts');
  const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  const exports = {};
  modules.set(path, exports);
  new Function('exports', 'require', js)(exports, name => {
    const mapped = name.startsWith('@/') ? name.slice(2) : name === './defaultSettings' ? 'data/defaultSettings' : null;
    assert.ok(mapped && ['data/defaultSettings', 'data/settingsSchema', 'store/polaRachunku.generated', 'store/warstwaPola'].includes(mapped), name);
    return load(mapped);
  });
  return exports;
}
const { DEFAULT_SETTINGS } = load('data/defaultSettings');
const { COVERED_KEYS, SETTINGS_SCHEMA } = load('data/settingsSchema');
const { POLA_RACHUNKU } = load('store/polaRachunku.generated');
const { powiazPoleUstawien, opisWarstwyPola } = load('store/warstwaPola');
const source = read('../src/views/SettingsView.tsx');
const tree = ts.createSourceFile('SettingsView.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const fn = tree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'AdvancedSection');
const js = ts.transpileModule(fn.getText(tree), { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;
const createElement = (type, props, ...children) => ({ type, props: { ...props, children } });
const render = (app, editor = null, blocked = false) => new Function(
  'React', 'useApp', 'useT', 'useState', 'useMemo', 'useContext', 'EdycjaPresetuCtx', 'useEffect', 'matchesSearch',
  'DEFAULT_SETTINGS', 'COVERED_KEYS', 'powiazPoleUstawien', 'Card', 'TextInput', 'Empty', 'Checkbox', 'NumberInput',
  `${js};return AdvancedSection;`,
)(
  { createElement }, () => app, () => (k, args) => k + (args ? JSON.stringify(args) : ''),
  initial => [initial, () => {}], f => f(), () => editor, {}, () => {}, load('lib/search').matchesSearch, DEFAULT_SETTINGS,
  COVERED_KEYS, powiazPoleUstawien, 'Card', 'TextInput', 'Empty', 'Checkbox', 'NumberInput',
)({ zablokowane: blocked });
function flatten(node) {
  if (Array.isArray(node)) return node.flatMap(flatten);
  return node && typeof node === 'object' ? [node, ...flatten(node.props?.children)] : [];
}
const row = (rendered, key) => flatten(rendered).find(n => n.type === 'label' && n.props.key === key);
const control = (rendered, key) => flatten(row(rendered, key)).find(n => ['NumberInput', 'TextInput', 'Checkbox'].includes(n.type));
function documents() {
  const calls = [];
  const app = { settings: { ...DEFAULT_SETTINGS, day_target_pct: 0, price_log_interval_s: 10 }, setSetting(k, v) { calls.push(['global', k, v]); this.settings[k] = v; } };
  const editor = (name, value) => ({ nazwa: name, doc: { ...DEFAULT_SETTINGS, day_target_pct: value, price_log_interval_s: 99 }, set(k, v) { calls.push([name, k, v]); this.doc[k] = v; } });
  return { app, a: editor('PRESET-A', 1), b: editor('PRESET-B', 2), calls };
}

test('actual Advanced renders the selected preset and writes only its strategy field', () => {
  const { app, a, b, calls } = documents();
  const rendered = render(app, b);
  assert.equal(control(rendered, 'day_target_pct').props.value, 2);
  assert.equal(calls.length, 0, 'render/bind has no mutation');
  control(rendered, 'day_target_pct').props.onChange(3);
  assert.deepEqual(calls, [['PRESET-B', 'day_target_pct', 3]]);
  assert.equal(app.settings.day_target_pct, 0);
  assert.equal(a.doc.day_target_pct, 1);
  assert.match(row(rendered, 'day_target_pct').props['data-setting-owner'], /PRESET-B/);
});

test('actual Advanced account/runtime field stays global even inside preset B', () => {
  const { app, b, calls } = documents();
  const rendered = render(app, b);
  assert.equal(control(rendered, 'price_log_interval_s').props.value, 10);
  control(rendered, 'price_log_interval_s').props.onChange(30);
  assert.deepEqual(calls, [['global', 'price_log_interval_s', 30]]);
  assert.equal(b.doc.price_log_interval_s, 99);
  control(rendered, 'ai_mode').props.onChange(true);
  assert.deepEqual(calls[1], ['global', 'ai_mode', true]);
});

test('no preset selected explicitly edits the global document; no broadcasting', () => {
  const { app, a, b, calls } = documents();
  const rendered = render(app);
  control(rendered, 'day_target_pct').props.onChange(4);
  assert.deepEqual(calls, [['global', 'day_target_pct', 4]]);
  assert.equal(a.doc.day_target_pct, 1);
  assert.equal(b.doc.day_target_pct, 2);
});

test('owner is captured by the original rendered callback, never rebased at commit', () => {
  const { app, a, b, calls } = documents();
  const old = control(render(app, b), 'day_target_pct').props.onChange;
  const fresh = control(render(app, a), 'day_target_pct').props.onChange;
  old(7);
  fresh(8);
  assert.deepEqual(calls, [['PRESET-B', 'day_target_pct', 7], ['PRESET-A', 'day_target_pct', 8]]);
});

test('loading/error never exposes a fallback editing control, even with a stale preset document', () => {
  const { app, b, calls } = documents();
  for (const editor of [null, b]) {
    const rendered = render(app, editor, true);
    assert.equal(control(rendered, 'day_target_pct'), undefined);
    assert.equal(control(rendered, 'price_log_interval_s'), undefined);
    powiazPoleUstawien('day_target_pct', app, editor, true).set(99);
  }
  assert.deepEqual(calls, []);
});

test('all actual UI fields have source-attributed scope; all canonical account fields remain global', () => {
  for (const key of Object.keys(DEFAULT_SETTINGS)) {
    const scope = opisWarstwyPola(key);
    assert.ok(scope?.dowod, `missing scope ${key}`);
    assert.ok(['preset', 'rachunek'].includes(scope.zakres));
  }
  for (const key of POLA_RACHUNKU) assert.equal(opisWarstwyPola(key).zakres, 'rachunek', key);
  assert.equal(opisWarstwyPola('lot_max').zakres, 'preset');
  assert.equal(opisWarstwyPola('price_tol').zakres, 'preset');
  assert.equal(opisWarstwyPola('konto_dzwignia').zakres, 'rachunek');
  assert.equal(opisWarstwyPola('new_unclassified_field'), null);
});

test('new volume contract is global; independent BE and retarget flags belong to the selected strategy', () => {
  const { app, b, calls } = documents();
  for (const key of ['order_volume_contract_v2', 'be_never_loosen', 'retarget_respects_final_target']) {
    assert.equal(DEFAULT_SETTINGS[key], false, key);
    powiazPoleUstawien(key, app, b).set(true);
  }
  assert.deepEqual(calls, [
    ['global', 'order_volume_contract_v2', true],
    ['PRESET-B', 'be_never_loosen', true],
    ['PRESET-B', 'retarget_respects_final_target', true],
  ]);
});

test('archived no-op controls remain visible read-only, cannot mutate either owner', () => {
  const { app, b, calls } = documents();
  const rendered = render(app, b);
  for (const key of ['allow_mt5_modify', 'spp_refresh_after_modify', 'grid_fallback_best_edge', 'sim_clock_strict']) {
    assert.ok(row(rendered, key), key);
    assert.equal(control(rendered, key), undefined, key);
    assert.equal(opisWarstwyPola(key).nieobslugiwane, true);
    powiazPoleUstawien(key, app, b).set(true);
  }
  assert.deepEqual(calls, []);
});

test('actual owner-tagged preset state rejects stale documents and preserves an old save owner', () => {
  const view = tree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'SettingsView');
  const variable = name => view.body.statements.find(n => ts.isVariableStatement(n) && n.declarationList.declarations.some(d => d.name.getText(tree) === name));
  const compile = text => ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const contextJs = compile(variable('edycjaCtx').getText(tree));
  const makeContext = new Function('wielosilnik', 'presetDoc', 'presetKonfig', 'zapiszPolePresetu', 'zapiszLatkePresetu', `${contextJs};return edycjaCtx;`);
  const { b } = documents();
  assert.equal(makeContext(true, { nazwa: 'PRESET-B', doc: b.doc }, 'PRESET-A', () => {}), null);
  assert.equal(makeContext(true, null, 'PRESET-A', () => {}), null);
  assert.equal(makeContext(true, { nazwa: 'PRESET-B', doc: b.doc }, 'PRESET-B', () => {}).doc, b.doc);
  let state = { nazwa: 'PRESET-A', doc: { ...DEFAULT_SETTINGS, day_target_pct: 1 } };
  const calls = [];
  const callbackJs = compile(variable('zapiszPolePresetu').getText(tree));
  const oldSave = new Function('useCallback', 'setPresetDoc', 'saveQueue', 'presetKonfig', 'reportPresetError', `${callbackJs};return zapiszPolePresetu;`)(
    f => f, f => { state = f(state); }, { current: new (load('store/presetWriteQueue').PresetWriteQueue)((name, patch) => { calls.push([name, patch]); return Promise.resolve({ ok: true }); }) }, 'PRESET-B', () => {});
  oldSave('day_target_pct', 9);
  assert.equal(state.doc.day_target_pct, 1, 'late B save cannot overwrite A document');
  assert.deepEqual(calls, [['PRESET-B', { day_target_pct: 9 }]]);
  assert.match(source, /<EdycjaPresetuCtx.Provider key=\{presetKonfig \|\| "global"\} value=\{edycjaCtx\}>\s*<AdvancedSection/);
  assert.match(source, /wielosilnik && !edycjaCtx \? \(\s*<fieldset[^>]*disabled/);
});

test('actual numeric input commits draft only on blur; discarding/unmounting a draft makes no write', () => {
  const primitive = read('../src/components/ui/index.tsx');
  const primitiveTree = ts.createSourceFile('ui.tsx', primitive, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const number = primitiveTree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'NumberInput');
  const compiled = ts.transpileModule(number.getText(primitiveTree).replace(/^export /, ''), { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;
  const numberComponent = new Function('React', 'useState', 'useRef', 'useEffect', `${compiled};return NumberInput;`)(
    { createElement }, init => [typeof init === 'function' ? init() : init, () => {}], value => ({ current: value }), () => {});
  const { app, b, calls } = documents();
  const props = control(render(app, b), 'day_target_pct').props;
  const input = flatten(numberComponent(props)).find(n => n.type === 'input');
  input.props.onChange({ target: { value: '7' } });
  assert.deepEqual(calls, [], 'editing a draft is not an apply');
  // Discarded component has no effect; there is no form-wide Cancel in UI.
  const next = flatten(numberComponent(props)).find(n => n.type === 'input');
  next.props.onBlur({ target: { value: '3.5' } });
  assert.deepEqual(calls, [['PRESET-B', 'day_target_pct', 3.5]]);
});
