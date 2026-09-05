// Regenerate UI ownership from the canonical account contract.
import {readFileSync,writeFileSync} from 'node:fs';
import ts from 'typescript';
const read=p=>readFileSync(new URL('../'+p,import.meta.url),'utf8');
const core=read('rust/crates/core/src/wielosilnik.rs');
const block=core.match(/pub const POLA_RACHUNKU: &\[&str\] = &\[([\s\S]*?)\n\];/)[1];
const keys=[...block.matchAll(/^\s*"([a-z0-9_]+)",/gm)].map(m=>m[1]);
const tree=ts.createSourceFile('types.ts',read('src/types/index.ts'),ts.ScriptTarget.Latest,true);
const type=tree.statements.find(n=>ts.isInterfaceDeclaration(n)&&n.name.text==='Settings');
const declared=new Set(type.members.map(n=>n.name.getText(tree)));
const aliases={stops_level:'sim_stops_level',msg_clock_offset_ms:'msg_clock_offset_h',server_tz_offset_ms:'server_tz_offset_h',ai_enabled:'ai_mode'};
const mapped=keys.map(k=>aliases[k]??k).filter(k=>declared.has(k));
const missing=keys.filter(k=>!declared.has(aliases[k]??k));
const output=`/* PLIK GENEROWANY — NIE EDYTOWAĆ RĘCZNIE.
   Źródło: rust/crates/core/src/wielosilnik.rs (POLA_RACHUNKU)
   Generator: narzedzia/pola_rachunku.mjs
   Pól w Ruście: ${keys.length} · zna je panel: ${mapped.length}
   Pola wyłącznie po stronie serwera: ${missing.join(', ')||'brak'}
*/
import type { Settings } from "@/types";

/** Wspólna własność rachunku; preset nogi nie nadpisuje tych pól. */
export const POLA_RACHUNKU = [
${mapped.map(k=>'  '+JSON.stringify(k)+',').join('\n')}
] as const satisfies readonly (keyof Settings)[];

/** Account overlay corresponding to wielosilnik::ustawienia_formatu. */
export function polaRachunkuZ(doc: Settings): Partial<Settings> {
  const out: Record<string, unknown> = {};
  for (const key of POLA_RACHUNKU) out[key] = doc[key];
  return out as Partial<Settings>;
}
`;
writeFileSync(new URL('../src/store/polaRachunku.generated.ts',import.meta.url),output);
console.log(`account ownership: ${mapped.length}/${keys.length} core fields represented`);
