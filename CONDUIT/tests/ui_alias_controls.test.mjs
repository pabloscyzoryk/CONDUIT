import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { isAbsolute, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import ts from 'typescript';
import { defaults, fields, renderField, flatten, load, viewSource } from './ui_alias_harness.mjs';

const aliases = load('store/settingAliases');
const preset = JSON.parse(readFileSync(new URL('../config/presets/GOD-X7.json', import.meta.url), 'utf8'));
const repoRoot = fileURLToPath(new URL('..', import.meta.url));
const configuredTarget = process.env.CARGO_TARGET_DIR;
const targetDir = configuredTarget
  ? (isAbsolute(configuredTarget) ? configuredTarget : resolve(repoRoot, 'rust', configuredTarget))
  : join(repoRoot, 'rust', 'target');
const binary = join(targetDir, 'debug', 'examples', 'ui_alias_probe.exe');
function mapper(overrides = {}, patch = {}) {
  const run = spawnSync(binary, [], { input: JSON.stringify({ preset, overrides, patch }), encoding: 'utf8' });
  assert.equal(run.status, 0, `Build the actual offline mapper probe first: ${run.error ?? run.stderr}`);
  return JSON.parse(run.stdout);
}
const doc = overrides => ({ ...defaults, ...mapper(overrides).ui });
function edit(key, value, overrides, owner = true) {
  const before = doc(overrides);
  const actual = renderField(key, before, { preset: owner, aliases });
  assert.deepEqual(actual.calls, [], 'render does not save or migrate anything');
  actual.control.props.onChange(value);
  assert.equal(actual.calls.length, 1, 'siblings go in a single atomic request');
  const patch = actual.calls[0].patch;
  return { actual, patch, result: mapper(overrides, patch), rendered: renderField(key, { ...before, ...patch }, { preset: owner, aliases }) };
}

test('daily trail basis round-trips through the actual preset mapper without changing its thresholds', () => {
  for (const [core, ui] of [['EquityPeak', 'equity_peak'], ['ProfitPeak', 'profit_peak']]) {
    assert.equal(doc({day_trail_basis: core}).day_trail_basis, ui);
    const {patch, result} = edit('day_trail_basis', ui, {day_trail_basis: 'EquityPeak', day_trail_stop_pct: 30, day_trail_arm_pct: 5});
    assert.deepEqual(patch, {day_trail_basis: ui});
    assert.equal(result.after.day_trail_basis, core);
    assert.equal(result.after.day_trail_stop_pct, 30);
    assert.equal(result.after.day_trail_arm_pct, 5);
    assert.ok(result.changed.every(change => change.key === 'day_trail_basis'));
  }
});

test('explicit entry and TP selections clear competing aliases in one owner-bound patch', () => {
  for (const [key, zone_offset_mode] of [['custom_entry', 'Price'], ['entry_offset_dir', 'Directional']]) {
    const result = edit(key, true, {zone_offset_mode: zone_offset_mode === 'Price' ? 'Directional' : 'Price'});
    assert.equal(result.actual.calls[0].owner, 'SELECTED-PRESET');
    assert.equal(result.result.after.zone_offset_mode, zone_offset_mode);
    assert.ok(result.result.changed.every(change => change.key === 'zone_offset_mode'));
  }
  for (const [key, tp_schedule] of [['all_runners', 'AllRunners'], ['official_mode', 'OfficialPct'], ['scale_out', 'ScaleOutPct']]) {
    for (const previous of ['AllRunners', 'OfficialPct', 'ScaleOutPct', 'Ladder']) {
      const result = edit(key, true, {tp_schedule: previous});
      assert.equal(result.result.after.tp_schedule, tp_schedule);
      assert.equal(result.actual.calls[0].owner, 'SELECTED-PRESET');
      assert.equal(Object.keys(result.patch).length, 3);
      assert.ok(result.result.changed.every(change => change.key === 'tp_schedule'));
    }
  }
});

test('legacy conflicting aliases display the engine precedence without rewriting the document', () => {
  const legacy = {...defaults, custom_entry: true, entry_offset_dir: true, all_runners: true, official_mode: true, scale_out: true};
  const saved = JSON.stringify(legacy);
  for (const key of ['custom_entry', 'official_mode', 'scale_out']) {
    const actual = renderField(key, legacy, {aliases});
    assert.equal(actual.control.props.checked, false, key);
    assert.deepEqual(actual.calls, []);
  }
  assert.equal(JSON.stringify(legacy), saved);
});

test('ATR and Chandelier trailing modes are selectable and reach the real engine', () => {
  for (const key of ['trail_mode', 'trail_runner_mode']) {
    for (const [ui, core] of [['atr', 'Atr'], ['chandelier', 'Chandelier']]) {
      assert.ok(fields.get(key).field.options.some(option => option.value === ui));
      const result = edit(key, ui, {trail_mode: 'Gap', trail_runner_mode: 'Gap'});
      assert.equal(result.result.after[key], core);
      assert.ok(result.result.changed.every(change => change.key === key));
    }
  }
});

test('daily profit budget controls affect only the selected preset and remain opt-in', () => {
  assert.equal(defaults.profit_budget_arm_pct, 0);
  assert.equal(defaults.profit_budget_keep_pct, 50);
  assert.equal(defaults.profit_budget_deploy_pct, 100);
  for (const [key, value] of [['profit_budget_arm_pct', 12], ['profit_budget_keep_pct', 65], ['profit_budget_deploy_pct', 80]]) {
    const result = edit(key, value, {profit_budget_arm_pct: 0, profit_budget_keep_pct: 50, profit_budget_deploy_pct: 100});
    assert.deepEqual(result.patch, {[key]: value});
    assert.equal(result.result.after[key], value);
    assert.ok(result.result.changed.every(change => change.key === key));
    assert.equal(result.actual.calls[0].owner, 'SELECTED-PRESET');
  }
});

test('explicit pending validity changes only its source-lifetime axis and preserves protective settings', () => {
  for (const enabled of [true, false]) {
    const result = edit('explicit_pending_until_cancel', enabled, {
      explicit_pending_until_cancel: !enabled,
      pending_ttl_h: 6,
      pending_drop_on_target: true,
      max_dd_pct: 12,
    });
    assert.equal(result.result.after.explicit_pending_until_cancel, enabled);
    assert.equal(result.result.after.pending_ttl_h, 6);
    assert.equal(result.result.after.pending_drop_on_target, true);
    assert.equal(result.result.after.max_dd_pct, 12);
    assert.ok(result.result.changed.every(change => change.key === 'explicit_pending_until_cancel'));
    assert.equal(result.actual.calls[0].owner, 'SELECTED-PRESET');
  }
});

test('threshold aliases reach their core fields when their documented parent is active', () => {
  for (const [key, coreKey, overrides, value] of [
    ['be_lock_points', 'be_lock_pts', {be_lock_pts: 10}, 12],
    ['sl_dist_max', 'entry_sl_dist_limit', {entry_sl_dist_limit: 10}, 12],
    ['harvest_start', 'harvest_start', {harvest_retrace_pct: 10, harvest_start: 2}, 3],
    ['harvest_retrace_pct', 'harvest_retrace_pct', {harvest_retrace_pct: 10}, 15],
  ]) {
    const result = edit(key, value, overrides);
    assert.equal(result.result.after[coreKey], value, key);
    assert.ok(result.result.changed.every(change => change.key === coreKey), key);
  }
});

test('every expanded engine-axis control changes its intended core field and preserves the owner', () => {
  const loaded = doc();
  for (const [key, {field, group}] of fields) {
    if (!group.id.startsWith('engine_')) continue;
    const current = loaded[key];
    const value = field.type === 'select' ? field.options.find(option => option.value !== current).value
      : field.type === 'bool' ? !current
      : field.type === 'text' ? (key === 'entry_uklad' ? '1,2,1' : 'audit-value')
      : current + (field.step ?? 1);
    const rendered = renderField(key, loaded, {aliases});
    assert.deepEqual(rendered.calls, [], `${key}: rendering must not write`);
    const control = flatten(rendered.rendered).find(node => ['Switch','Select','NumberInput','TextInput'].includes(node.type));
    assert.ok(control, `${key}: actual control exists`);
    control.props.onChange(value);
    assert.equal(rendered.calls.length, 1, `${key}: one deliberate write`);
    assert.equal(rendered.calls[0].owner, group.zakres === 'rachunek' ? 'global' : 'SELECTED-PRESET', key);
    const result = mapper({}, rendered.calls[0].patch);
    assert.ok(result.changed.some(change => change.key === key), `${key}: mapper silently ignored control`);
    assert.deepEqual(result.after[key], value, `${key}: core value matches chosen control value`);
    assert.ok(result.changed.every(change => change.key === key), `${key}: edit changed unrelated core fields`);
  }
});

test('actual full-preset display preserves delay above one and every TP-source mode without writes', () => {
  for (const smart_sl_delay of [0, 1, 2, 4]) {
    const actual = renderField('trail_after_tp2', doc({ smart_sl_delay }), { aliases });
    assert.equal(actual.control.props.checked, smart_sl_delay > 0);
    assert.deepEqual(actual.calls, []);
    const output = flatten(actual.rendered).flatMap(n => n.props?.children ?? []);
    assert.ok(output.some(x => typeof x === 'string' && x.includes(`"n":${smart_sl_delay}`)));
    assert.deepEqual(mapper({ smart_sl_delay }).changed, []);
  }
  for (const tp_source of ['Either', 'PriceOnly', 'SignalOnly', 'SignalConfirmedByPrice', 'PriceFirstSignalWindow']) {
    for (const key of ['tp_source', 'tp_detect_price', 'tp_detect_signal']) {
      const actual = renderField(key, doc({ tp_source }), { aliases });
      assert.deepEqual(actual.calls, []);
      if (key === 'tp_source') assert.equal(actual.control.props.value, tp_source);
      else assert.equal(actual.control.props.checked, key === 'tp_detect_price' ? tp_source !== 'SignalOnly' : tp_source !== 'PriceOnly');
    }
    assert.deepEqual(mapper({ tp_source }).changed, []);
  }
});

test('actual trailing switch updates canonical delay both ways, including loaded delay2', () => {
  for (const smart_sl_delay of [0, 1, 2, 4]) {
    for (const value of [false, true]) {
      const { actual, patch, result } = edit('trail_after_tp2', value, { smart_sl_delay, smart_sl_mode: 'Ladder' });
      assert.deepEqual(patch, { trail_after_tp2: value, smart_sl_delay_n: value ? 1 : 0 });
      assert.equal(actual.calls[0].owner, 'SELECTED-PRESET');
      assert.equal(result.after.smart_sl_delay, value ? 1 : 0);
      assert.ok(result.changed.every(x => x.key === 'smart_sl_delay'));
      assert.equal(result.ui.smart_sl_delay_n, value ? 1 : 0);
    }
  }
});

test('trailing visibility follows actual mapper modes, including RF-only and BE-only; scale-out alone is not Smart SL', () => {
  const field = fields.get('trail_after_tp2').field;
  for (const smart_sl_mode of ['Off', 'Ladder', 'LadderWithBe', 'BreakevenOnly']) {
    for (const smart_sl_only_after_rf of [false, true]) {
      const loaded = doc({ smart_sl_mode, smart_sl_only_after_rf });
      assert.equal(field.when(loaded), smart_sl_mode !== 'Off', `${smart_sl_mode}/${smart_sl_only_after_rf}`);
    }
  }
  const onlyScaleOut = { ...doc({ smart_sl_mode: 'Off' }), scale_out: true };
  assert.equal(field.when(onlyScaleOut), false);
});

test('actual simple TP switches replace canonical mode and aliases in one patch', () => {
  for (const tp_source of ['Either', 'SignalConfirmedByPrice', 'PriceFirstSignalWindow']) {
    const priceOnly = edit('tp_detect_signal', false, { tp_source });
    assert.deepEqual(priceOnly.patch, { tp_source: 'PriceOnly', tp_detect_price: true, tp_detect_signal: false });
    assert.equal(priceOnly.result.after.tp_source, 'PriceOnly');
    assert.ok(priceOnly.result.changed.every(x => x.key === 'tp_source'));
    const signalOnly = edit('tp_detect_price', false, { tp_source });
    assert.deepEqual(signalOnly.patch, { tp_source: 'SignalOnly', tp_detect_price: false, tp_detect_signal: true });
    assert.equal(signalOnly.result.after.tp_source, 'SignalOnly');
  }
  assert.equal(edit('tp_detect_signal', true, { tp_source: 'PriceOnly' }).result.after.tp_source, 'Either');
  assert.equal(edit('tp_detect_price', true, { tp_source: 'SignalOnly' }).result.after.tp_source, 'Either');
});

test('actual source selector synchronizes shortcut display without losing advanced modes or unrelated settings', () => {
  for (const tp_source of ['Either', 'PriceOnly', 'SignalOnly', 'SignalConfirmedByPrice', 'PriceFirstSignalWindow']) {
    const { result, patch } = edit('tp_source', tp_source, { tp_source: 'Either', smart_sl_delay: 3, tp_price_tolerance: 0.47 });
    assert.equal(result.after.tp_source, tp_source);
    assert.equal(result.after.smart_sl_delay, 3);
    assert.equal(result.after.tp_price_tolerance, 0.47);
    assert.deepEqual(patch, { tp_source, tp_detect_price: tp_source !== 'SignalOnly', tp_detect_signal: tp_source !== 'PriceOnly' });
    assert.ok(result.changed.every(x => x.key === 'tp_source'));
  }
});

test('turning off the final simple source follows documented PriceOnly fallback and shows it immediately', () => {
  const { result, rendered } = edit('tp_detect_price', false, { tp_source: 'PriceOnly' });
  assert.equal(result.after.tp_source, 'PriceOnly');
  assert.equal(rendered.control.props.checked, true);
  const fromSignal = edit('tp_detect_signal', false, { tp_source: 'SignalOnly' });
  assert.equal(fromSignal.result.after.tp_source, 'PriceOnly');
  assert.equal(fromSignal.rendered.control.props.checked, false);
});

test('original and global owners stay distinct; unrelated controls keep their one-field path', () => {
  const selected = edit('trail_after_tp2', false, { smart_sl_delay: 2 });
  const global = edit('trail_after_tp2', true, { smart_sl_delay: 0 }, false);
  assert.equal(selected.actual.calls[0].owner, 'SELECTED-PRESET');
  assert.equal(global.actual.calls[0].owner, 'global');
  const ordinary = edit('lot_max', 10, { lot_max: 0 });
  assert.deepEqual(ordinary.patch, { lot_max: 10 });
});

test('actual atomic preset callback captures original owner across asynchronous preset selection', () => {
  const tree = ts.createSourceFile('SettingsView.tsx', viewSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const view = tree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'SettingsView');
  const variable = view.body.statements.find(n => ts.isVariableStatement(n) && n.declarationList.declarations.some(d => d.name.getText(tree) === 'zapiszLatkePresetu'));
  const compiled = ts.transpileModule(variable.getText(tree), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  let state = { nazwa: 'NEW-PRESET', doc: { ...defaults, tp_source: 'SignalOnly' } };
  const calls = [];
  const saveOld = new Function('useCallback', 'setPresetDoc', 'saveQueue', 'presetKonfig', 'reportPresetError', `${compiled};return zapiszLatkePresetu;`)(
    f => f, f => { state = f(state); }, { current: new (load('store/presetWriteQueue').PresetWriteQueue)((name, patch) => { calls.push({ name, patch }); return Promise.resolve({ ok: true }); }) }, 'OLD-PRESET', () => {});
  const patch = aliases.settingControlPatch(doc({ tp_source: 'Either' }), 'tp_detect_signal', false);
  saveOld(patch);
  assert.equal(state.doc.tp_source, 'SignalOnly');
  assert.deepEqual(calls, [{ name: 'OLD-PRESET', patch }]);
});

test('legacy documents without canonical aliases retain fallback interpretation; no mutation during projection', () => {
  const old = { trail_after_tp2: true, tp_detect_price: false, tp_detect_signal: true };
  const serialized = JSON.stringify(old);
  assert.equal(aliases.effectiveSmartSlDelay(old), 1);
  assert.equal(aliases.effectiveTpSource(old), 'SignalOnly');
  assert.equal(aliases.effectiveTpSource({ ...old, tp_source: 'UNKNOWN' }), 'Either');
  assert.equal(JSON.stringify(old), serialized);
});
