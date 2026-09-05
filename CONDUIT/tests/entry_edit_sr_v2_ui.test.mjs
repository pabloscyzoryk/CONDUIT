import test from 'node:test';
import assert from 'node:assert/strict';
import { defaults, fields, load, renderField } from './ui_alias_harness.mjs';

const aliases = load('store/settingAliases');
const { opisWarstwyPola } = load('store/warstwaPola');

test('entry edit and exact SR warmup default OFF and edit only the selected strategy', () => {
  for (const key of ['entry_edit_geometry_v2', 'sr_warmup_exact_ticks']) {
    assert.equal(defaults[key], false);
    assert.equal(opisWarstwyPola(key).zakres, 'preset');
    for (const value of [true, false]) {
      const actual = renderField(key, { ...defaults }, { aliases });
      assert.deepEqual(actual.calls, []);
      actual.control.props.onChange(value);
      assert.deepEqual(actual.calls, [{ owner: 'SELECTED-PRESET', patch: { [key]: value } }]);
    }
  }
});

test('experimental controls state qualification limits and do not enable parents', () => {
  for (const key of ['entry_edit_geometry_v2', 'sr_warmup_exact_ticks']) {
    const field = fields.get(key).field;
    assert.equal(field.warn(defaults), null);
    assert.match(field.warn({ ...defaults, [key]: true }), /LIVE|parytet/);
  }
  const sr = fields.get('sr_warmup_exact_ticks').field;
  assert.equal(sr.when(defaults), false);
  assert.equal(sr.when({ ...defaults, trail_sr_enabled: true }), true);
  assert.equal(sr.when({ ...defaults, sr_warmup_exact_ticks: true }), true,
    'enabled but dormant controls remain reachable for disabling');
});
