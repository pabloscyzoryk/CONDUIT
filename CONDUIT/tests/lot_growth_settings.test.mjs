import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import ts from 'typescript';

const root = resolve(import.meta.dirname, '..');
const cache = new Map();
function load(name, parent = resolve(root, 'src/root.ts')) {
  let path = name.startsWith('@/') ? resolve(root, 'src', name.slice(2)) : resolve(dirname(parent), name);
  if (!existsSync(path)) path += '.ts';
  if (cache.has(path)) return cache.get(path);
  const exports = {}; cache.set(path, exports);
  const js = ts.transpileModule(readFileSync(path, 'utf8'), { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
  } }).outputText;
  new Function('exports', 'require', js)(exports, child => load(child, path));
  return exports;
}
const { DEFAULT_SETTINGS: defaults } = load('@/data/defaultSettings');
const { SETTINGS_SCHEMA, ZNACZENIE_ZERA, lotGrowthIssue } = load('@/data/settingsSchema');
const { przetlumaczGrupy } = load('@/i18n/schema');
const { presetFromDisk } = load('@/data/presets');
const { powiazPoleUstawien } = load('@/store/warstwaPola');
const groups = SETTINGS_SCHEMA.filter(g => ['lotgrowth', 'lotcontext'].includes(g.id));
const fields = groups.flatMap(g => g.fields);
const keys = fields.map(f => f.key);
const enabled = { ...defaults, lot_growth_mode: 'Power' };

test('all 19 settings, enum spellings and defaults match the actual core contract', () => {
  const core = readFileSync(resolve(root, 'rust/crates/core/src/settings.rs'), 'utf8');
  const coreKeys = [...core.matchAll(/pub (lot_growth_\w+):/g)].map(m => m[1]);
  assert.equal(keys.length, 19);
  assert.deepEqual([...keys].sort(), coreKeys.sort());
  for (const key of keys) {
    const matches = [...core.matchAll(new RegExp('^\\s+' + key + ': (.+),$', 'gm'))];
    const raw = matches.at(-1)?.[1];
    assert.ok(raw, key);
    const value = raw.includes('::') ? raw.split('::').at(-1) : Number(raw);
    assert.equal(defaults[key], value, key);
  }
  const source = readFileSync(resolve(root, 'rust/crates/core/src/lot_growth.rs'), 'utf8');
  for (const [type, key] of [['LotGrowthMode', 'lot_growth_mode'], ['LotGrowthAllocation', 'lot_growth_allocation']]) {
    const body = source.split(`pub enum ${type} {`)[1].split('}')[0].replace(/#\[default\]/g, '');
    assert.deepEqual(fields.find(f => f.key === key).options.map(o => o.value), body.split(',').map(s => s.trim()).filter(Boolean));
  }
});

test('legacy default leaves curves and every attenuation axis disabled', () => {
  assert.equal(defaults.lot_growth_mode, 'Off');
  assert.equal(defaults.lot_growth_allocation, 'Uniform');
  assert.equal(defaults.lot_growth_basket_risk_pct, 0);
  const axes = fields.filter(f => f.key.endsWith('_strength'));
  assert.equal(axes.length, 10);
  for (const f of axes) {
    assert.equal(defaults[f.key], 0);
    assert.equal(ZNACZENIE_ZERA[f.key], 'wylaczone');
    assert.deepEqual([f.min, f.max, f.step], [0, 2, 0.25]);
  }
  assert.deepEqual(fields.filter(f => !f.when || f.when(defaults)).map(f => f.key), ['lot_growth_mode']);
  assert.equal(lotGrowthIssue(defaults), null);
});

test('each curve shows only its own numeric parameters without resetting hidden values', () => {
  const parameterKeys = ['lot_growth_power', 'lot_growth_rate_pct', 'lot_growth_capital_multiple', 'lot_growth_lot_multiple'];
  for (const [mode, expected] of [['Power', parameterKeys.slice(0, 1)], ['ThresholdLinear', parameterKeys.slice(1, 2)], ['GeometricSteps', parameterKeys.slice(2)]]) {
    const doc = { ...defaults, lot_growth_mode: mode }, before = JSON.stringify(doc);
    assert.deepEqual(fields.filter(f => parameterKeys.includes(f.key) && f.when(doc)).map(f => f.key), expected);
    assert.equal(JSON.stringify(doc), before);
  }
});

test('English overlay translates every label/hint/option and retains all control semantics', () => {
  const english = przetlumaczGrupy(groups, 'en');
  groups.forEach((g, gi) => {
    assert.notEqual(english[gi].title, g.title);
    assert.notEqual(english[gi].desc, g.desc);
    g.fields.forEach((f, i) => {
      const e = english[gi].fields[i];
      assert.notEqual(e.label, f.label, f.key);
      assert.notEqual(e.hint, f.hint, f.key);
      assert.equal(e.key, f.key);
      for (const key of ['type', 'when', 'min', 'max', 'step']) assert.equal(e[key], f[key], `${f.key}.${key}`);
      f.options?.forEach((o, j) => { assert.equal(e.options[j].value, o.value); assert.notEqual(e.options[j].label, o.label); });
    });
  });
  const plWarn = groups[0].fields[0].warn, enWarn = english[0].fields[0].warn;
  for (const patch of [{ lot_growth_mode: 'unknown' }, { lot_growth_allocation: 'unknown' }, { lot_growth_reference_balance: 0 },
    { lot_growth_power: 0 }, { lot_growth_day_dd_strength: 3 }, { t100: { ...defaults.t100, enabled: true } }]) {
    const doc = { ...enabled, ...patch };
    assert.ok(plWarn(doc)); assert.ok(enWarn(doc)); assert.notEqual(plWarn(doc), enWarn(doc));
  }
});

test('invalid imported modes, numbers and strengths stay visible as invalid, not replaced by defaults', () => {
  for (const patch of [{ lot_growth_mode: 'Power2' }, { lot_growth_allocation: 'Equal' }, { lot_growth_reference_lot: 0.001 },
    { lot_growth_reference_balance: 0 }, { lot_growth_basket_risk_pct: 101 }, { lot_growth_power: NaN },
    { lot_growth_day_dd_strength: -1 }, { lot_growth_age_decay_strength: Infinity }, { lot_growth_rearm_decay_strength: '1' }]) {
    const raw = { ...enabled, ...patch };
    const loaded = presetFromDisk({ name: 'IMPORTED', description: '', format: 'Synergy', settings: raw }).values;
    assert.equal(loaded, raw);
    assert.ok(lotGrowthIssue(loaded), JSON.stringify(patch));
    for (const key of Object.keys(patch)) assert.equal(loaded[key], raw[key]);
  }
});

test('every new field writes only the originally selected preset, never the account or another preset', () => {
  const calls = [];
  const global = { settings: { ...defaults }, setSetting: (k, v) => calls.push(['account', k, v]) };
  const make = name => ({ nazwa: name, doc: { ...enabled }, set: (k, v) => calls.push([name, k, v]) });
  const a = make('A'), b = make('B');
  for (const key of keys) {
    const old = powiazPoleUstawien(key, global, a);
    const next = powiazPoleUstawien(key, global, b);
    assert.equal(old.opis.zakres, 'preset'); assert.equal(next.wlasciciel, 'B');
    const value = enabled[key]; old.set(value);
    assert.deepEqual(calls.at(-1), ['A', key, value]);
  }
  assert.equal(calls.length, 19);
});

test('serialized preset roundtrip retains every new field including inactive values', () => {
  const settings = { ...defaults, lot_growth_mode: 'GeometricSteps', lot_growth_allocation: 'ExposureAwareRisk',
    lot_growth_power: 0.55, lot_growth_rate_pct: 0.75, lot_growth_age_decay_strength: 0.5, lot_growth_day_dd_strength: 1 };
  const loaded = presetFromDisk(JSON.parse(JSON.stringify({ name: 'ROUNDTRIP', description: '', format: 'Synergy', settings })));
  for (const key of keys) assert.equal(loaded.values[key], settings[key], key);
  assert.equal(lotGrowthIssue(loaded.values), null);
});

test('boundary validation respects inactive curves and the documented core ranges', () => {
  assert.equal(lotGrowthIssue({ ...defaults, lot_growth_power: -1, lot_growth_day_dd_strength: NaN }), null);
  assert.equal(lotGrowthIssue({ ...enabled, lot_growth_power: 0.0001 }), null);
  assert.equal(lotGrowthIssue({ ...enabled, lot_growth_mode: 'ThresholdLinear', lot_growth_rate_pct: 0, lot_growth_power: -1 }), null);
  assert.equal(lotGrowthIssue({ ...enabled, lot_growth_mode: 'GeometricSteps', lot_growth_capital_multiple: 1 }), 'curve');
  assert.equal(lotGrowthIssue({ ...enabled, lot_growth_day_dd_strength: 2 }), null);
  assert.equal(lotGrowthIssue({ ...enabled, lot_growth_day_dd_strength: 2.01 }), 'strength');
});
