import test from 'node:test';
import assert from 'node:assert/strict';
import { load } from './ui_alias_harness.mjs';
const { matchesSearch, normalizeSearch, searchScore } = load('lib/search');

test('operator can find Polish axes without accents, underscores or exact word order', () => {
  assert.equal(normalizeSearch('ŻÓŁĆ__Ładuj'), 'zolc laduj');
  assert.equal(matchesSearch('max lot', 'lot_max'), true);
  assert.equal(matchesSearch('koszyka zamkniecie', 'Zamknięcie całego koszyka'), true);
  assert.equal(matchesSearch('lot trailing', 'lot_max', 'Trailing stop'), true);
  assert.equal(matchesSearch('lot trailing', 'lot_max', 'Entry settings'), false);
});

test('actual configuration keys rank above incidental prose', () => {
  assert.equal(searchScore('lot max', 'lot_max', 'Maximum lot'), 100);
  assert.ok(searchScore('lot max', 'lot_max', 'Maximum lot') > searchScore('lot max', 'order_volume_contract_v2', 'STRICT BROKER VOLUME CONTRACT'));
});

test('every schema and raw axis remains discoverable by its actual key', () => {
  const { SETTINGS_SCHEMA, COVERED_KEYS } = load('data/settingsSchema');
  const { DEFAULT_SETTINGS } = load('data/defaultSettings');
  const keys = new Set(SETTINGS_SCHEMA.flatMap(group => group.fields.map(field => field.key)));
  for (const key of Object.keys(DEFAULT_SETTINGS)) {
    if (key === 'merge_config') continue;
    assert.ok(keys.has(key) || !COVERED_KEYS.has(key), key);
    assert.equal(matchesSearch(key, key), true, key);
  }
});
