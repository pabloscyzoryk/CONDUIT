import test from 'node:test';
import assert from 'node:assert/strict';
import { defaults, fields, load, renderField } from './ui_alias_harness.mjs';

const aliases = load('store/settingAliases');
const { opisWarstwyPola } = load('store/warstwaPola');
const schema = load('data/settingsSchema');

test('deferred ENTRY and target-relot are opt-in strategy controls, never account overrides', () => {
  assert.equal(defaults.defer_entry_until_receipts, false);
  assert.equal(defaults.deferred_entry_max_age_s, 300);
  assert.equal(defaults.pending_relot_reconcile_target, false);
  for (const [key, value] of [['defer_entry_until_receipts', true], ['deferred_entry_max_age_s', 120], ['pending_relot_reconcile_target', true]]) {
    assert.equal(opisWarstwyPola(key).zakres, 'preset');
    const actual = renderField(key, { ...defaults }, { aliases });
    assert.deepEqual(actual.calls, []);
    actual.control.props.onChange(value);
    assert.deepEqual(actual.calls, [{ owner: 'SELECTED-PRESET', patch: { [key]: value } }]);
  }
});

test('deferred age is positive seconds, not a zero/unlimited switch; prerequisites are stated without reading a fake global value from a preset', () => {
  const age = fields.get('deferred_entry_max_age_s').field;
  assert.equal(age.type, 'num');
  assert.equal(age.unit, 's');
  assert.equal(age.min, 1);
  assert.match(age.hint, /pierwszego odbioru/);
  assert.match(age.hint, /Edycja nie przedłuża/);
  assert.equal(schema.ZNACZENIE_ZERA.deferred_entry_max_age_s, undefined);
  assert.equal(age.when({ ...defaults, defer_entry_until_receipts: false }), false);
  assert.equal(age.when({ ...defaults, defer_entry_until_receipts: true }), true);
  const feature = fields.get('defer_entry_until_receipts').field;
  assert.match(feature.hint, /globalnego close_receipt_reconcile/);
  assert.match(feature.hint, /nie komendy NOW/);
  assert.match(feature.hint, /RAM/);
  assert.equal(feature.warn, undefined, 'a strategy document alone cannot decide a global account prerequisite');
});

test('target relot states legacy-plan override and preserves the legacy choice unchanged', () => {
  const old = fields.get('pending_relot_wg_planu').field;
  const feature = fields.get('pending_relot_reconcile_target').field;
  for (const legacy of [false, true]) {
    const state = { ...defaults, pending_relot_on_balance: true, pending_relot_wg_planu: legacy };
    assert.equal(old.warn(state), null);
    const actual = renderField('pending_relot_reconcile_target', state, { aliases });
    actual.control.props.onChange(true);
    const changed = { ...state, ...actual.calls[0].patch };
    assert.equal(changed.pending_relot_wg_planu, legacy);
    assert.match(old.warn(changed), /zastąpiony/);
  }
  assert.equal(feature.when(defaults), false);
  assert.equal(feature.when({ ...defaults, pending_relot_reconcile_target: true }), true, 'enabled-but-inactive option stays reachable');
});
