import { ENGINE_TEMPLATES } from "./engineTemplates";
import { EN } from "./en";
import { PL } from "./pl";

type Language = "en" | "pl";

const token = /\{([^{}]*)\}/g;
const escapeRegex = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const identity = (value: string, next: () => number) => value.split(":", 1)[0] || String(next());

function compile(source: string, target: string) {
  const names: string[] = [];
  let end = 0, auto = 0, pattern = "^";
  for (const match of source.matchAll(token)) {
    pattern += escapeRegex(source.slice(end, match.index)).replace(/\s+/g, "\\s+") + "([\\s\\S]*?)";
    names.push(identity(match[1], () => auto++));
    end = match.index! + match[0].length;
  }
  pattern += escapeRegex(source.slice(end)).replace(/\s+/g, "\\s+") + "$";
  return { re: new RegExp(pattern), names, target, literal: source.replace(token, "").length };
}

function complete(rule: ReturnType<typeof compile>): boolean {
  let auto = 0;
  return [...rule.target.matchAll(token)].every(match => rule.names.includes(identity(match[1], () => auto++)));
}

// Longer fixed phrases first: a broad diagnostic container must not consume a
// more specific message. Templates never modify source objects or machine codes.
const uiTemplates = (Object.keys(EN) as (keyof typeof EN)[])
  .filter(key => /^(toast\.|eng\.|potw\.)/.test(key) && EN[key].includes("{"))
  .map(key => [PL[key], EN[key]] as const);
const templates = [...ENGINE_TEMPLATES, ...uiTemplates];
const rules = Object.fromEntries((["en", "pl"] as const).map(language => [language,
  templates.map(([pl, en]) => compile(language === "en" ? pl : en, language === "en" ? en : pl))
    .filter(rule => rule.literal >= 5 && complete(rule)).sort((a, b) => b.literal - a.literal),
])) as Record<Language, ReturnType<typeof compile>[]>;
const cache = new Map<string, string>();
const diagnosticVariables = new Set(["e", "powod", "opis", "zostaje", "category", "r", "reason", "p", "przyczyna", "b", "opis_stopu", "naglowek"]);

function renderEngineText(text: string, language: Language, depth = 0): string {
  if (depth > 4 || !text) return text;
  for (const [pl, en] of [["\nTREŚĆ:\n", "\nSOURCE MESSAGE:\n"], ["\nTreść wiadomości:\n", "\nMessage text:\n"]]) {
    for (const marker of [pl, en]) {
      const at = text.indexOf(marker);
      if (at >= 0) return renderEngineText(text.slice(0, at), language, depth + 1) + (language === "en" ? en : pl) + text.slice(at + marker.length);
    }
  }
  for (const rule of rules[language]) {
    const match = rule.re.exec(text);
    if (!match) continue;
    const values = new Map(rule.names.map((name, i) => [name, match[i + 1]]));
    let auto = 0;
    return rule.target.replace(token, (original, spec: string) => {
      const name = identity(spec, () => auto++);
      const value = values.get(name);
      // Keep account identities, prices, symbols, paths and arbitrary user text
      // byte-for-byte. Nested diagnostics are translated at their own boundary.
      return value === undefined ? original : diagnosticVariables.has(name) ? renderEngineText(value, language, depth + 1) : value;
    });
  }
  if (text.startsWith("Error: ")) return "Error: " + renderEngineText(text.slice(7), language, depth + 1);
  if (text.includes("\n")) return text.split("\n").map(line => renderEngineText(line, language, depth + 1)).join("\n");
  return text;
}

export function presentEngineText(text: string, language: Language = "en"): string {
  const key = language + "\0" + text;
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  const rendered = renderEngineText(text, language);
  // Repeated live snapshots should not run hundreds of regexes for every row.
  // Bounded storage also prevents unique diagnostics accumulating indefinitely.
  if (text.length <= 8000) {
    if (cache.size >= 1024) cache.delete(cache.keys().next().value!);
    cache.set(key, rendered);
  }
  return rendered;
}
