import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

export const read = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const modules = new Map();
export function load(path) {
  if (modules.has(path)) return modules.get(path);
  const exports = {};
  modules.set(path, exports);
  const js = ts.transpileModule(read('../src/' + path + '.ts'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  new Function('exports', 'require', js)(exports, name => {
    const mapped = name.startsWith('@/') ? name.slice(2) : name === './defaultSettings' ? 'data/defaultSettings' : null;
    assert.ok(mapped, name);
    return load(mapped);
  });
  return exports;
}
export const defaults = load('data/defaultSettings').DEFAULT_SETTINGS;
const schema = load('data/settingsSchema');
const ownership = load('store/warstwaPola');
export const fields = new Map(schema.SETTINGS_SCHEMA.flatMap(group => group.fields.map(field => [field.key, { field, group }])));
export const viewSource = read('../src/views/SettingsView.tsx');
const tree = ts.createSourceFile('SettingsView.tsx', viewSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
export const settingSource = tree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'SettingField').getText(tree);
const compiled = ts.transpileModule(settingSource, { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;
export const flatten = node => Array.isArray(node) ? node.flatMap(flatten)
  : node && typeof node === 'object' ? [node, ...flatten(node.props?.children)] : [];

export function renderField(key, doc, { preset = true, aliases = {} } = {}) {
  const calls = [];
  const record = (owner, patch) => { calls.push({ owner, patch }); };
  const app = { settings: doc, setSetting: (k, v) => record('global', { [k]: v }), setSettings: patch => record('global', patch) };
  const editor = { nazwa: 'SELECTED-PRESET', doc, set: (k, v) => record('SELECTED-PRESET', { [k]: v }), patch: patch => record('SELECTED-PRESET', patch) };
  const env = {
    React: { createElement: (type, props, ...children) => ({ type, props: { ...props, children } }) },
    useApp: () => app, useT: () => (key, args) => key + (args ? JSON.stringify(args) : ''),
    useContext: () => preset ? editor : null, EdycjaPresetuCtx: {},
    zakresPola: ownership.zakresPola, warstwaInnaNizGrupa: ownership.warstwaInnaNizGrupa,
    ZNACZENIE_ZERA: schema.ZNACZENIE_ZERA, BRAK_LIMITU_RYZYKOWNY: schema.BRAK_LIMITU_RYZYKOWNY,
    DEFAULT_SETTINGS: defaults,
    Switch: 'Switch', NumberInput: 'NumberInput', TextInput: 'TextInput', Select: 'Select',
    PodgladNog: 'PodgladNog', Icon: 'Icon', Tooltip: 'Tooltip', JsonEditor: 'JsonEditor',
    ...aliases,
  };
  const component = new Function(...Object.keys(env), `${compiled};return SettingField;`)(...Object.values(env));
  const { field, group } = fields.get(key);
  const rendered = component({ field, grupa: group });
  return { calls, rendered, control: flatten(rendered).find(n => ['Switch', 'Select', 'NumberInput'].includes(n.type)) };
}
