import test from 'node:test';
import assert from 'node:assert/strict';
import { defaults, fields, load, renderField } from './ui_alias_harness.mjs';

const aliases = load('store/settingAliases');
const { opisWarstwyPola } = load('store/warstwaPola');

test('closed-net accounting is an opt-in global contract even inside a strategy editor', () => {
  assert.equal(defaults.closed_profit_net_costs, false);
  assert.equal(opisWarstwyPola('closed_profit_net_costs').zakres, 'rachunek');
  for (const preset of [false, true]) {
    for (const value of [false, true]) {
      const actual = renderField('closed_profit_net_costs', { ...defaults }, { preset, aliases });
      assert.deepEqual(actual.calls, [], 'render/load does not write or enable prerequisites');
      actual.control.props.onChange(value);
      assert.deepEqual(actual.calls, [{ owner: 'global', patch: { closed_profit_net_costs: value } }]);
    }
  }
});

test('cost toggle visibly states the unresolved live/restart gate without treating a preset as account truth', () => {
  const { field } = fields.get('closed_profit_net_costs');
  assert.equal(field.when, undefined, 'the global contract stays reachable even when dependencies are disabled');
  assert.match(field.hint, /basket_realized_broker_only=true/);
  assert.match(field.hint, /close_receipt_reconcile=true/);
  assert.match(field.hint, /nie pobiera kosztów z salda ponownie/);
  assert.match(field.hint, /Nie włączaj na VPS/);
  assert.equal(field.warn(defaults), null);
  assert.match(field.warn({ ...defaults, closed_profit_net_costs: true }), /Sim/);
  assert.match(field.warn({ ...defaults, closed_profit_net_costs: true, close_receipt_reconcile: true }), /nie jest jeszcze potwierdzona/);
});
