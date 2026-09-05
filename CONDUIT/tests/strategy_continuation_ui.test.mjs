import test from 'node:test';
import assert from 'node:assert/strict';
import { defaults, fields, load, renderField } from './ui_alias_harness.mjs';

const aliases = load('store/settingAliases');
const { opisWarstwyPola } = load('store/warstwaPola');
const { POLA_RACHUNKU } = load('store/polaRachunku.generated');
const key = 'restore_strategy_continuation';

test('continuation is default OFF and belongs to account, never selected preset', () => {
  assert.equal(defaults[key], false);
  assert.ok(POLA_RACHUNKU.includes(key), 'generated core account contract must contain the key; an account UI group alone is insufficient');
  assert.equal(opisWarstwyPola(key).zakres, 'rachunek');
  for (const preset of [true, false]) {
    for (const value of [true, false]) {
      const actual = renderField(key, { ...defaults }, { preset, aliases });
      assert.deepEqual(actual.calls, []);
      actual.control.props.onChange(value);
      assert.deepEqual(actual.calls, [{ owner: 'global', patch: { [key]: value } }]);
    }
  }
});

test('continuation UI explicitly limits stage A and does not claim complete crash recovery', () => {
  const field = fields.get(key).field;
  assert.equal(field.type, 'bool');
  assert.equal(field.warn(defaults), null);
  assert.match(field.warn({ ...defaults, [key]: true }), /W WALIDACJI/);
  assert.match(field.hint, /nie zapewnia atomowego zapisu/);
  assert.match(field.hint, /nie naprawia wstecz/);
  assert.match(field.hint, /ochronne zamknięcia/);
});
