import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import ts from 'typescript';

const root = resolve(import.meta.dirname, '../src');
const modules = new Map(), saved = [], patches = [];
const react = {
  createContext: () => ({}), useCallback: f => f, useContext: () => null,
  useEffect: () => {}, useSyncExternalStore: (_, get) => get(),
};
const jsx = { jsx: (type, props) => ({ type, props }), jsxs: (type, props) => ({ type, props }) };
function load(name, parent = root + '/index.ts') {
  if (name === 'react') return react;
  if (name === 'react/jsx-runtime') return jsx;
  if (name === '@/store/storage') return { load: (_, value) => value, save: (...args) => saved.push(args) };
  if (name === '@/store/transport') return { api: { patchSettings: async value => patches.push(value) } };
  let file = name.startsWith('@/') ? resolve(root, name.slice(2)) : resolve(dirname(parent), name);
  if (!existsSync(file) || !/\.tsx?$/.test(file)) file = [file + '.ts', file + '.tsx', file + '/index.tsx'].find(existsSync);
  assert.ok(file, name);
  file = resolve(file); // Windows aliases and relative imports share one module.
  if (modules.has(file)) return modules.get(file);
  const exports = {}; modules.set(file, exports);
  const source = readFileSync(file, 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } }).outputText;
  new Function('exports', 'require', compiled)(exports, child => load(child, file));
  return exports;
}
const i18n = load('@/i18n');
const { EN } = load('@/i18n/en'), { PL } = load('@/i18n/pl');
const { SETTINGS_SCHEMA } = load('@/data/settingsSchema');
const { SCHEMA_EN } = load('@/i18n/settingsSchema.en');
const { przetlumaczGrupy } = load('@/i18n/schema');
const { ENGINE_TEMPLATES } = load('@/i18n/engineTemplates');
const { presentEngineText } = load('@/i18n/enginePresentation');
const { tSilnik } = load('@/i18n/silnik');

test('PL ↔ EN changes every dictionary entry without exposing a raw key', () => {
  for (const language of ['pl', 'en', 'pl', 'en']) {
    i18n.setLanguage(language);
    for (const [key, value] of Object.entries(language === 'en' ? EN : PL)) {
      assert.ok(value.trim(), key);
      assert.equal(i18n.t(key), value, key);
      assert.notEqual(i18n.t(key), key, key);
    }
  }
  assert.ok(saved.some(([, lang]) => lang === 'pl'));
  assert.ok(patches.some(value => value.language === 'en'));
  const count = patches.length;
  i18n.applyServerLanguage('pl');
  assert.equal(patches.length, count, 'server language update must not send a write back');
  i18n.cycleLanguage(); assert.equal(i18n.getLanguage(), 'en');
});

test('every schema label, hint, enum and warning has an English presentation with unchanged machine values', () => {
  const translated = przetlumaczGrupy(SETTINGS_SCHEMA, 'en');
  for (let i = 0; i < SETTINGS_SCHEMA.length; i++) {
    const group = SETTINGS_SCHEMA[i], english = SCHEMA_EN[group.id];
    assert.ok(english?.title, group.id);
    if (group.desc) assert.notEqual(english.desc, undefined, group.id + '.desc');
    for (let j = 0; j < group.fields.length; j++) {
      const field = group.fields[j], layer = english.fields?.[field.key];
      assert.ok(layer?.label, field.key);
      if (field.hint) assert.notEqual(layer.hint, undefined, field.key + '.hint');
      if (field.warn) assert.equal(typeof layer.warn, 'function', field.key + '.warn');
      for (const option of field.options ?? []) {
        // ISO currency codes and their native symbols are language-neutral.
        if (field.key === 'display_currency' && !['OFF', 'MT5', 'PLN'].includes(option.value)) continue;
        assert.ok(layer.options?.[option.value], `${field.key}:${option.value}`);
      }
      assert.equal(translated[i].fields[j].key, field.key);
      assert.ok(!['pkt', 'dni'].includes(translated[i].fields[j].unit), field.key + '.unit');
      assert.deepEqual(translated[i].fields[j].options?.map(o => o.value), field.options?.map(o => o.value));
    }
  }
});

test('visible static JSX copy and accessibility labels do not regress to untranslated Polish', () => {
  const files = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory() ? files(resolve(directory, entry.name)) : entry.name.endsWith('.tsx') ? [resolve(directory, entry.name)] : []);
  const polish = /[ąćęłńóśźż]|\b(?:Anuluj|Wykresy|ulubiony|nowy poziom|instrument bota|Reset widoku|Szukaj instrumentu|Wybierz ten katalog|pusto)\b/i;
  for (const file of files(root)) {
    const tree = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const visit = node => {
      if (ts.isJsxText(node)) assert.ok(!polish.test(node.text), `${file}: ${node.text.trim()}`);
      if (ts.isJsxAttribute(node) && ['title', 'placeholder', 'aria-label'].includes(node.name.text) && node.initializer && ts.isStringLiteral(node.initializer))
        assert.ok(!polish.test(node.initializer.text), `${file}: ${node.initializer.text}`);
      ts.forEachChild(node, visit);
    };
    visit(tree);
  }
});

test('engine translations preserve variable values, symbols and account identifiers', () => {
  const shared = [...ENGINE_TEMPLATES, ...Object.keys(EN).filter(key => key.startsWith('eng.')).map(key => [PL[key], EN[key]])];
  assert.deepEqual(JSON.parse(readFileSync(resolve(root, '../rust/crates/server/src/engine_translations.json'), 'utf8')), shared, 'email and UI must use the same generated dictionary');
  const interpolate = text => text.replace(/\{([^{}]*)\}/g, () => '7301');
  const mismatches = ENGINE_TEMPLATES.map(([source, english]) => ({ source, actual: presentEngineText(interpolate(source)), expected: interpolate(english) })).filter(row => row.actual !== row.expected);
  assert.deepEqual(mismatches, []);
  assert.equal(presentEngineText('Preset MY-PRESET 42 przeładowany w locie'), 'Preset MY-PRESET 42 reloaded during operation');
  assert.equal(presentEngineText('Sygnał ODROCZONY — nie wykonany'), 'DEFERRED signal — not executed');
  assert.equal(presentEngineText('koszyk B15: broker odrzucił zmianę SL — INVALID_STOPS'), 'basket B15: broker rejected the SL change — INVALID_STOPS');
  assert.equal(presentEngineText('UNKNOWN_BROKER_FACT <value>'), 'UNKNOWN_BROKER_FACT <value>');
  assert.equal(presentEngineText('BUY GOLD 2400\nSL 2390\nTP 2420'), 'BUY GOLD 2400\nSL 2390\nTP 2420');
  assert.equal(presentEngineText('Sygnał odrzucony\nTREŚĆ:\nSygnał odrzucony'), 'Signal rejected\nSOURCE MESSAGE:\nSygnał odrzucony');
});

test('existing dynamic notifications also follow the current display language', () => {
  i18n.setLanguage('en');
  const message = i18n.t('toast.basketCreated', { id: 27 });
  i18n.setLanguage('pl');
  assert.equal(load('./index', resolve(root, 'i18n/silnik.ts')), i18n);
  assert.equal(i18n.getLanguage(), 'pl');
  assert.equal(presentEngineText(message, 'pl'), i18n.t('toast.basketCreated', { id: 27 }));
  assert.equal(tSilnik(message), i18n.t('toast.basketCreated', { id: 27 }));
  i18n.setLanguage('en');
  assert.equal(tSilnik('Brak połączenia z MT5'), 'No connection to MT5');
  i18n.setLanguage('pl');
  assert.equal(tSilnik('No connection to MT5'), 'Brak połączenia z MT5');
  assert.ok(!presentEngineText('RISK FREE @ 2400 · closed 2 positions (5 $, entry VWAP 2400) · runners: 1, SL 2400', 'pl').includes('{'));
});

test('RichT preserves dictionary markup but escapes untrusted interpolation', () => {
  i18n.setLanguage('en');
  const rendered = i18n.RichT({ k: 'path.newName' });
  assert.equal(rendered.props.dangerouslySetInnerHTML.__html, EN['path.newName']);
  const injected = i18n.RichT({ k: 'wiz.models', vars: { n: '<img src=x onerror=alert(1)>', m: 'A&B' } });
  const html = injected.props.dangerouslySetInnerHTML.__html;
  assert.ok(!html.includes('<img'));
  assert.ok(html.includes('&lt;img'));
  assert.ok(html.includes('A&amp;B'));
});


test('entry source telemetry preserves unknown historical counts and is distinct from closed trades', () => {
  const { entrySourceCounts, optionalCount } = load('./lib/labMetrics', resolve(root, 'index.ts'));
  for (const missing of [undefined, null, NaN, Infinity, -1]) assert.equal(optionalCount(missing), '—');
  assert.equal(optionalCount(0), '0');
  const rows = entrySourceCounts({ knownEntrySources: 7, knownFullEntrySources: 4, entrySourcesFirstSeenAsEdit: 3, trades: 31 });
  assert.deepEqual(rows.map(row => row.value), [7, 4, 3]);
  assert.ok(entrySourceCounts({}).every(row => optionalCount(row.value) === '—'));
  for (const language of ['en', 'pl']) {
    i18n.setLanguage(language);
    for (const row of rows) assert.notEqual(i18n.t(row.label), row.label);
  }
});
