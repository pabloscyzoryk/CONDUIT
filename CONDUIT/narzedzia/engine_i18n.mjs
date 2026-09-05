import { readFileSync, writeFileSync } from 'node:fs';
import ts from 'typescript';

// The renderer shared with outgoing email uses the same audited translations
// as the panel. This artifact contains program templates only, never runtime data.
const source = new URL('../src/i18n/engineTemplates.ts', import.meta.url);
const target = new URL('../rust/crates/server/src/engine_translations.json', import.meta.url);
const exports = {};
const js = ts.transpileModule(readFileSync(source, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
new Function('exports', js)(exports);
function dictionary(name) {
  const value = {};
  const compiled = ts.transpileModule(readFileSync(new URL(`../src/i18n/${name}.ts`, import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  new Function('exports', compiled)(value);
  return value[name.toUpperCase()];
}
const EN = dictionary('en'), PL = dictionary('pl');
const templates = [...exports.ENGINE_TEMPLATES, ...Object.keys(EN).filter(key => key.startsWith('eng.')).map(key => [PL[key], EN[key]])];
const content = JSON.stringify(templates, null, 2) + '\n';
let previous = '';
try { previous = readFileSync(target, 'utf8'); } catch { /* first generation */ }
if (previous !== content) writeFileSync(target, content, 'utf8');
console.log(`[engine i18n] ${templates.length} shared presentation templates`);
