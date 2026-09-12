import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import ts from 'typescript';

const src = resolve(import.meta.dirname, '../src');
const read = p => readFileSync(resolve(src, p), 'utf8');
function harness(language = 'en') {
  const cache = new Map(), states = [];
  let cursor = 0;
  const jsx = (type, props, key) => ({ type, props: { ...props, key } });
  const react = {
    useId: () => 't100-test', useEffect: () => {},
    useState(initial) {
      const index = cursor++;
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial;
      return [states[index], next => { states[index] = typeof next === 'function' ? next(states[index]) : next; }];
    },
  };
  function load(name, parent = src + '/root.ts') {
    if (name === 'react') return react;
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx, Fragment: 'Fragment' };
    if (name === '@/components/ui') return Object.fromEntries(['Card', 'Badge', 'Button', 'Checkbox', 'Field'].map(k => [k, k]));
    if (name === '@/i18n') return { useT: () => (key, args = {}) => {
      const dict = load(`@/i18n/${language}`)[language.toUpperCase()];
      assert.equal(typeof dict[key], 'string', key);
      return dict[key].replace(/\{(\w+)\}/g, (_, k) => args[k] ?? `{${k}}`);
    } };
    let path = name.startsWith('@/') ? resolve(src, name.slice(2)) : resolve(dirname(parent), name);
    if (!existsSync(path)) path = [path + '.ts', path + '.tsx'].find(existsSync);
    assert.ok(path, name);
    if (cache.has(path)) return cache.get(path);
    const exports = {}; cache.set(path, exports);
    const js = ts.transpileModule(readFileSync(path, 'utf8'), { compilerOptions: {
      module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
    } }).outputText;
    new Function('exports', 'require', js)(exports, child => load(child, path));
    return exports;
  }
  return { load, render(props) { cursor = 0; return load('@/components/panels/T100Panel').T100Panel({ mode: 'AUTO-EA', ...props }); } };
}
const flatten = n => Array.isArray(n) ? n.flatMap(flatten) : n && typeof n === 'object' ? [n, ...flatten(n.props?.children)] : [];
const field = (tree, key) => flatten(tree).find(n => n.props?.['data-t100-key'] === key);
const input = (tree, key) => flatten(field(tree, key)).find(n => ['input', 'select'].includes(n.type));
const button = (tree, text) => flatten(tree).find(n => n.type === 'Button' && n.props.children === text);

test('all 31 typed/default UI parameters agree with the actual production Config', () => {
  const { load } = harness();
  const defaults = load('@/data/defaultSettings').DEFAULT_SETTINGS.t100;
  const source = read('../rust/crates/core/src/t100/mod.rs');
  const body = source.split('pub struct Config {')[1].split('\n}')[0];
  const fields = [...body.matchAll(/pub\s+(\w+):\s*(bool|u8|u32|usize|f64),/g)].map(m => m[1]);
  const init = source.split('impl Default for Config')[1].split('impl Config')[0];
  const values = Object.fromEntries([...init.matchAll(/(\w+)\s*:\s*(false|true|[0-9.]+)/g)].map(m => [m[1], JSON.parse(m[2])]));
  assert.equal(fields.length, 31);
  assert.deepEqual(Object.keys(defaults).sort(), fields.sort());
  assert.deepEqual(defaults, values);
  assert.equal(defaults.enabled, false);
  assert.deepEqual(load('@/store/t100Settings').t100Errors(defaults), []);
});

test('wrong types, non-finite numbers, coupled bounds and blank input fail without normalization', () => {
  const { load } = harness();
  const defaults = load('@/data/defaultSettings').DEFAULT_SETTINGS.t100;
  const { t100Errors, t100Number, t100Document } = load('@/store/t100Settings');
  for (const key of Object.keys(defaults)) {
    assert.ok(t100Errors({ ...defaults, [key]: typeof defaults[key] === 'boolean' ? 'true' : true }).length, key);
  }
  for (const patch of [{ risk_pct: 0 }, { risk_pct: 21 }, { risk_pct: 5, portfolio_risk_pct: 4 },
    { max_positions: 1.5 }, { experts: 0 }, { experts: 16 }, { cooldown_bars: 4294967296 },
    { session_start_utc: 21, session_end_utc: 5 }, { session_end_utc: 25 },
    { adaptation: 1.01 }, { signal_half_life_min: 0 }, { stop_atr: 0.29 },
    { score_threshold: Infinity }, { min_atr: NaN }, { daily_loss_pct: 51 }]) {
    assert.ok(t100Errors({ ...defaults, ...patch }).length, JSON.stringify(patch));
  }
  assert.ok(Number.isNaN(t100Number('')));
  assert.equal(t100Number('0,25'), 0.25);
  assert.equal(t100Document(null), null);
  const malformed = t100Document({ enabled: 'true', typo: 1 });
  assert.equal(malformed.enabled, 'true');
  assert.equal(malformed.typo, 1);
  assert.ok(t100Errors(malformed).length);
  assert.deepEqual(t100Errors({ ...defaults, score_threshold: 1.5, signal_weight: 0, break_even_r: 0,
    daily_profit_lock_pct: 0, daily_giveback_pct: 0, cooldown_bars: 0, session_start_utc: 0, session_end_utc: 24 }), []);
});

function documents(load) {
  const defaults = load('@/data/defaultSettings').DEFAULT_SETTINGS;
  const calls = [];
  const app = { mode: 'AUTO-EA', settings: { ...defaults, ea_enabled: true }, setSetting(k, v) { calls.push(['global', k, v]); this.settings[k] = v; } };
  const editor = name => ({ nazwa: name, doc: { ...defaults, t100: { ...defaults.t100, risk_pct: 2 } },
    set(k, v) { calls.push([name, k, v]); this.doc[k] = v; } });
  return { app, a: editor('A'), b: editor('B'), calls };
}

test('binding fixes one preset owner; invalid and loading saves cannot touch global settings', () => {
  const { load } = harness(); const { bindT100Settings } = load('@/store/t100Settings');
  const { app, a, b, calls } = documents(load);
  const bound = bindT100Settings(app, b);
  bindT100Settings(app, a);
  assert.equal(calls.length, 0, 'binding must not write defaults');
  assert.equal(bound.save({ ...bound.value, risk_pct: 3 }), true);
  assert.equal(calls[0][0], 'B');
  assert.equal(a.doc.t100.risk_pct, 2);
  assert.equal(app.settings.t100.risk_pct, 1);
  assert.equal(app.settings.ea_enabled, true);
  assert.equal(bound.save({ ...bound.value, risk_pct: false }), false);
  assert.equal(bindT100Settings(app, null, true).save(app.settings.t100), false);
  assert.equal(bindT100Settings(app, b, true).save(b.doc.t100), false);
  assert.equal(calls.length, 1);
});

test('actual panel renders every parameter in both languages without duplicate controls or write effects', () => {
  for (const lang of ['pl', 'en']) {
    const h = harness(lang), defaults = h.load('@/data/defaultSettings').DEFAULT_SETTINGS.t100;
    for (const enabled of [false, true]) {
      const h2 = harness(lang); let writes = 0;
      const tree = h2.render({ value: { ...defaults, enabled }, owner: 'PRESET-B', blocked: false, onSave: () => { writes++; } });
      const keys = flatten(tree).filter(n => n.props?.['data-t100-key']).map(n => n.props['data-t100-key']);
      assert.equal(keys.length, 31);
      assert.equal(new Set(keys).size, 31);
      assert.equal(writes, 0);
      for (const key of keys) {
        const dict = h2.load(`@/i18n/${lang}`)[lang.toUpperCase()];
        assert.ok(dict[`t100.field.${key}`]); assert.ok(dict[`t100.hint.${key}`]);
      }
      assert.ok(JSON.stringify(tree).includes('PRESET-B'));
    }
  }
});

test('actual panel edits a local draft and saves only its selected preset after validation', () => {
  const h = harness(); const { bindT100Settings } = h.load('@/store/t100Settings');
  const { app, b, calls } = documents(h.load);
  const binding = bindT100Settings(app, b);
  const props = { value: binding.value, owner: binding.owner, blocked: binding.blocked, onSave: binding.save };
  let tree = h.render(props);
  input(tree, 'enabled').props.onChange({ target: { value: 'true' } });
  tree = h.render(props);
  input(tree, 'risk_pct').props.onChange({ target: { value: '21' } });
  tree = h.render(props);
  assert.equal(button(tree, 'Save T-100').props.disabled, true);
  assert.equal(calls.length, 0);
  input(tree, 'risk_pct').props.onChange({ target: { value: '3' } });
  tree = h.render(props);
  assert.equal(button(tree, 'Save T-100').props.disabled, false);
  button(tree, 'Save T-100').props.onClick();
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], 'B');
  assert.equal(calls[0][2].enabled, true);
  assert.equal(calls[0][2].risk_pct, 3);
  assert.equal(app.settings.t100.enabled, false);
  assert.equal(app.settings.ea_enabled, true);
});

test('actual panel blocks owner loading and preserves malformed input until explicit repair', () => {
  const h = harness(); let writes = 0;
  const blocked = h.render({ value: { enabled: true }, owner: null, blocked: true, onSave: () => { writes++; } });
  assert.equal(flatten(blocked).filter(n => n.props?.['data-t100-key']).length, 0);
  const h2 = harness();
  const invalid = h2.render({ value: 'wrong', owner: 'A', blocked: false, onSave: () => { writes++; } });
  assert.ok(flatten(invalid).some(n => n.props?.role === 'alert'));
  assert.equal(button(invalid, 'Save T-100').props.disabled, true);
  assert.equal(writes, 0);
});

test('actual Settings section shows enabled or malformed policies outside AUTO-EA and keeps old defaults quiet', () => {
  const h = harness(), defaults = h.load('@/data/defaultSettings').DEFAULT_SETTINGS;
  const { bindT100Settings, t100Document, t100Errors } = h.load('@/store/t100Settings');
  const source = read('views/SettingsView.tsx');
  const ast = ts.createSourceFile('SettingsView.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const fn = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'T100Section');
  const js = ts.transpileModule(fn.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;
  function section(mode, t100) {
    const app = { mode, settings: { ...defaults, t100 }, setSetting() { assert.fail('render mutation'); } };
    return new Function('React','useApp','useContext','EdycjaPresetuCtx','bindT100Settings','t100Document','t100Errors','T100Panel', `${js}; return T100Section();`)(
      { createElement: (type, props) => ({ type, props }) }, () => app, () => null, {}, bindT100Settings, t100Document, t100Errors, 'T100Panel');
  }
  assert.equal(section('AUTO', defaults.t100), null);
  assert.equal(section('MANUAL', defaults.t100), null);
  for (const mode of ['AUTO', 'AUTO-EA', 'AI', 'MANUAL']) assert.equal(section(mode, { enabled: true }).type, 'T100Panel');
  assert.equal(section('AUTO-EA', defaults.t100).type, 'T100Panel');
  assert.equal(section('AUTO', { enabled: 'true' }).type, 'T100Panel');
});

test('AUTO cannot enable T100 and a mode switch never erases the saved preset', () => {
  const h = harness(), { bindT100Settings } = h.load('@/store/t100Settings');
  const { app, b, calls } = documents(h.load);
  app.mode = 'AUTO';
  const binding = bindT100Settings(app, b);
  assert.equal(binding.save({ ...binding.value, enabled: true }), false);
  const props = { mode: 'AUTO', value: binding.value, owner: binding.owner, blocked: false, onSave: binding.save };
  let tree = h.render(props);
  assert.equal(flatten(input(tree, 'enabled')).find(n => n.type === 'option' && n.props.value === 'true').props.disabled, true);
  input(tree, 'enabled').props.onChange({ target: { value: 'true' } });
  tree = h.render(props);
  assert.equal(input(tree, 'enabled').props.value, 'false');
  assert.equal(calls.length, 0);

  const h2 = harness(); app.mode = 'AUTO-EA';
  const ea = bindT100Settings(app, b);
  const p2 = { ...props, mode: 'AUTO-EA', onSave: ea.save };
  tree = h2.render(p2);
  input(tree, 'enabled').props.onChange({ target: { value: 'true' } });
  app.mode = 'AUTO';
  tree = h2.render({ ...p2, mode: 'AUTO' });
  assert.equal(button(tree, 'Save T-100').props.disabled, true);
  button(tree, 'Save T-100').props.onClick();
  assert.equal(calls.length, 0, 'stale enabled draft must not be committed after changing mode');

  b.doc.t100.enabled = true;
  const h3 = harness();
  tree = h3.render({ ...props, value: b.doc.t100 });
  assert.equal(input(tree, 'enabled').props.value, 'true');
  assert.ok(JSON.stringify(tree).includes('T-100 requires AUTO-EA'));
  assert.equal(b.doc.t100.enabled, true);
  assert.equal(calls.length, 0, 'mode rendering must not rewrite preset configuration');
});
