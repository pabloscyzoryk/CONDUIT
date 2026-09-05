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
  const saveOld = new Function('useCallback', 'setPresetDoc', 'api', 'presetKonfig', `${compiled};return zapiszLatkePresetu;`)(
    f => f, f => { state = f(state); }, { savePresetSettings: (name, patch) => calls.push({ name, patch }) }, 'OLD-PRESET');
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
