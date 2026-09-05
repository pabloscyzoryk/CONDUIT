import test from 'node:test';
import assert from 'node:assert/strict';
import { load } from './ui_alias_harness.mjs';
const { PresetWriteQueue } = load('store/presetWriteQueue');
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };

test('rapid changes to one preset keep submission order; another preset is independent', async () => {
  const first = deferred(), calls = [];
  const queue = new PresetWriteQueue((name, patch) => { calls.push([name, patch]); return calls.length === 1 ? first.promise : Promise.resolve(); });
  const a = queue.write('A', { lot_max: 1 });
  const b = queue.write('A', { lot_max: 5 });
  const c = queue.write('B', { lot_max: 10 });
  assert.deepEqual(calls, [['A', { lot_max: 1 }], ['B', { lot_max: 10 }]]);
  first.resolve(); await Promise.all([a, b, c]);
  assert.deepEqual(calls.at(-1), ['A', { lot_max: 5 }]);
});

test('rejected writes remain observable and do not poison subsequent edits or reload', async () => {
  const first = deferred(), calls = [];
  const queue = new PresetWriteQueue((name, patch) => { calls.push([name, patch]); return calls.length === 1 ? first.promise : Promise.resolve(); });
  const a = queue.write('A', { lot_max: 1 });
  const rejected = assert.rejects(a, /offline/);
  const b = queue.write('A', { lot_max: 5 });
  first.reject(new Error('offline'));
  await Promise.all([rejected, b, queue.idle('A')]);
  assert.equal(calls.length, 2);
  await queue.write('A', { lot_max: 3 });
  assert.equal(calls.length, 3);
});
