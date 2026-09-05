#!/usr/bin/env node


import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(ROOT, "src");

const bledy = [];

/* ---------- pliki src/ rekursywnie ---------- */
function pliki(dir, out = []) {
  for (const e of readdirSync(dir)) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) pliki(p, out);
    else if (/\.(tsx?|mts)$/.test(e)) out.push(p);
  }
  return out;
}

/* ---------- klucze słowników ---------- */
function kluczeSlownika(plik) {
  const src = readFileSync(join(SRC, "i18n", plik), "utf8");
  const out = new Set();
  // Dokładnie 2 spacje wcięcia = klucz obiektu; wartości wieloliniowe
  // mają 4 i potrafią zawierać `":` w środku polskich cudzysłowów.
  for (const m of src.matchAll(/^ {2}"([^"]+)":/gm)) out.add(m[1]);
  return out;
}

const EN = kluczeSlownika("en.ts");
const PL = kluczeSlownika("pl.ts");

/* ---------- 1. użycia t()/tt()/RichT w src/ ---------- */
const uzyte = new Map(); // klucz -> pierwszy plik
for (const p of pliki(SRC)) {
  if (p.includes(`${join("src", "i18n")}`) && /(?:en|pl)\.ts$/.test(p)) continue;
  const tekst = readFileSync(p, "utf8");
  for (const m of tekst.matchAll(/\b(?:tt|tSlownik|t)\(\s*"([^"]+)"/g)) {
    if (!uzyte.has(m[1])) uzyte.set(m[1], p);
  }
  for (const m of tekst.matchAll(/<RichT\s+k="([^"]+)"/g)) {
    if (!uzyte.has(m[1])) uzyte.set(m[1], p);
  }
}
for (const [klucz, plik] of uzyte) {
  if (!EN.has(klucz)) bledy.push(`brak w EN: „${klucz}” (użyty w ${plik.replace(ROOT, "")})`);
}

/* ---------- 2. EN ⇔ PL ---------- */
for (const k of EN) if (!PL.has(k)) bledy.push(`brak w PL: „${k}”`);
for (const k of PL) if (!EN.has(k)) bledy.push(`sierota w PL (nie ma w EN): „${k}”`);

/* ---------- 3. schemat vs nakładka EN ---------- */
/** Grupy schematu: [{id, pola: Map<klucz, Set<wartości opcji>>}] w kolejności pliku. */
function grupySchematu() {
  const src = readFileSync(join(SRC, "data", "settingsSchema.ts"), "utf8");
  const grupy = [];
  let biezaca = null;
  let biezacePole = null;
  for (const linia of src.split("\n")) {
    const id = linia.match(/^\s{4}id: "([\w-]+)",/);
    if (id) {
      biezaca = { id: id[1], pola: new Map() };
      grupy.push(biezaca);
      continue;
    }
    if (!biezaca) continue;
    const klucz = linia.match(/\bkey: "([\w]+)"/);
    if (klucz) {
      biezacePole = new Set();
      biezaca.pola.set(klucz[1], biezacePole);
    }
    if (biezacePole) {
      for (const m of linia.matchAll(/\bvalue: "([^"]*)"/g)) biezacePole.add(m[1]);
    }
  }
  return grupy;
}

/** Nakładka EN: Map<idGrupy, Map<kluczPola, Set<opcje>>> */
function grupyNakladki() {
  const src = readFileSync(join(SRC, "i18n", "settingsSchema.en.ts"), "utf8");
  const grupy = new Map();
  let biezaca = null;
  let wOptions = false;
  let biezaceOpcje = null;
  let biezacaMapa = null;
  for (const linia of src.split("\n")) {
    const g = linia.match(/^\s{2}([\w]+): \{/);
    if (g) {
      biezacaMapa = new Map();
      grupy.set(g[1], biezacaMapa);
      biezaca = g[1];
      continue;
    }
    if (!biezaca) continue;
    const pole = linia.match(/^\s{6}([\w]+): \{/);
    if (pole && biezacaMapa) {
      biezaceOpcje = new Set();
      biezacaMapa.set(pole[1], biezaceOpcje);
      wOptions = false;
      continue;
    }
    if (/^\s{8}options: \{/.test(linia)) wOptions = true;
    if (wOptions && biezaceOpcje) {
      for (const m of linia.matchAll(/^\s{10}(?:([\w]+)|"([^"]+)"):/g)) {
        biezaceOpcje.add(m[1] ?? m[2]);
      }
      // options w jednej linii: { both: "...", buy: "...", sell: "..." }
    }
    const inline = linia.match(/options: \{ (.+) \}/);
    if (inline && biezaceOpcje) {
      for (const m of inline[1].matchAll(/(?:([\w]+)|"([^"]+)"):/g)) biezaceOpcje.add(m[1] ?? m[2]);
    }
    if (/^\s{8}\},?$/.test(linia)) wOptions = false;
  }
  return grupy;
}

const schemat = grupySchematu();
const nakladka = grupyNakladki();

for (const g of schemat) {
  const n = nakladka.get(g.id);
  if (!n) {
    bledy.push(`nakładka EN: brak CAŁEJ grupy „${g.id}” (${g.pola.size} pól)`);
    continue;
  }
  for (const [klucz, opcje] of g.pola) {
    const pole = n.get(klucz);
    if (!pole) {
      bledy.push(`nakładka EN: grupa „${g.id}” bez pola „${klucz}”`);
      continue;
    }
    for (const v of opcje) {
      if (v !== "" && !pole.has(v)) {
        bledy.push(`nakładka EN: „${g.id}.${klucz}” bez etykiety opcji „${v}”`);
      }
    }
  }
}

/* ---------- werdykt ---------- */
if (bledy.length) {
  console.error(`\n[i18n kanarek] CZERWONY — ${bledy.length} braków:\n`);
  for (const b of bledy) console.error("  ✗ " + b);
  console.error("");
  process.exit(1);
}
console.log(
  `[i18n kanarek] zielony: ${uzyte.size} kluczy użytych w src/ · EN=${EN.size} · PL=${PL.size} · ` +
    `schemat ${schemat.length} grup / ${schemat.reduce((a, g) => a + g.pola.size, 0)} pól pokrytych`,
);
