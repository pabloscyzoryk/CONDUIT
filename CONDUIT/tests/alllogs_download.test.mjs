import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const compiled = ts.transpileModule(readFileSync(new URL('../src/store/transport.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
function api(fetch) {
  const exports = {};
  new Function('exports', 'require', 'fetch', 'window', compiled)(exports,
    name => { assert.equal(name, '@/i18n'); return { t: key => key }; }, fetch,
    { location: { search: '', protocol: 'http:', host: '127.0.0.1:8787', origin: 'http://127.0.0.1:8787' } });
  return exports.api;
}

test('allLogs download preserves invalid UTF8 and full integer text as raw bytes', async () => {
  const bytes = Uint8Array.from([...Buffer.from('{"ticket":18446744073709551615}\r\n'), 0xff, 0xfe, 0, 128]);
  const calls = [];
  const client = api(async (url, request) => {
    calls.push({ url, request });
    const response = new Response(bytes);
    response.json = () => { throw new Error('raw download must not parse JSON'); };
    response.text = () => { throw new Error('raw download must not decode text'); };
    return response;
  });
  const result = await client.pobierzAllLogs('synthetic/alllogs_2026-09-08.txt');
  assert.deepEqual(new Uint8Array(await result.arrayBuffer()), bytes);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, 'http://127.0.0.1:8787/api/fs/download');
  assert.equal(JSON.parse(calls[0].request.body).path, 'synthetic/alllogs_2026-09-08.txt');
});

test('refused download never turns an error document into a saved diagnostic file', async () => {
  const client = api(async () => new Response('{"error":"outside export scope"}', { status: 403 }));
  await assert.rejects(client.pobierzAllLogs('synthetic/other.txt'), /outside export scope/);
});
