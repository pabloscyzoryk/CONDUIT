

import { EN } from "./en";
import { PL } from "./pl";
import { getLanguage, t } from "./index";
import { presentEngineText } from "./enginePresentation";

// Exact known interface copy can also arrive as a log/toast title. Resolving
// both languages on render keeps these labels responsive to PL ↔ EN changes.
const KNOWN_COPY = new Map<string, string>();
for (const key of Object.keys(EN) as (keyof typeof EN)[]) {
  for (const value of [EN[key], PL[key]]) {
    if (value.length >= 8 && !value.includes("{")) KNOWN_COPY.set(value, key);
  }
}

/** Czy klucz w ogóle istnieje w słowniku (EN = źródło prawdy).
 *  Potrzebne, bo `t()` dla nieznanego klucza zwraca sam klucz — a tu
 *  chcemy w takiej sytuacji ORYGINALNY tekst z silnika. */
export function maKlucz(klucz: string): boolean {
  return klucz in EN;
}

/** Tłumaczy, jeśli klucz istnieje; w przeciwnym razie oddaje tekst wejściowy. */
function tlumaczLubZostaw(klucz: string, oryginal: string): string {
  return maKlucz(klucz) ? t(klucz) : oryginal;
}

/* ------------------------------------------------------------------
   1. KATALOGI WBUDOWANE (formaty, łańcuchy)

   Klucz robimy z NAZWY, nie z treści opisu: nazwa jest identyfikatorem
   i nie zmienia się przy redakcji zdania, a opis owszem. Format/łańcuch
   dodany przez użytkownika nie ma klucza i pokazuje własny opis — tak
   jak powinien, bo to jego tekst, nie nasz.
   ------------------------------------------------------------------ */

/** Opis formatu sygnałów (`fmt.<NAZWA>.desc`). */
export function opisFormatu(nazwa: string, oryginal: string): string {
  return tlumaczLubZostaw(`fmt.${nazwa}.desc`, oryginal);
}

/** Opis łańcucha presetów (`chain.<NAZWA>.desc`). Nazwy wbudowanych
 *  łańcuchów mają spacje („TYLKO ATFX") — klucz bierze je dosłownie. */
export function opisLancucha(nazwa: string, oryginal: string): string {
  return tlumaczLubZostaw(`chain.${nazwa}.desc`, oryginal);
}

/** Jednozdaniowy podpis presetu (`preset.<ID>.tag`).
 *
 *  Preset użytkownika (wgrany do `presets/`, zbudowany w laboratorium) nie ma
 *  klucza i pokazuje własny `tagline` — to jego tekst, nie nasz katalog. */
export function opisPresetu(id: string, oryginal: string): string {
  return tlumaczLubZostaw(`preset.${id}.tag`, oryginal);
}

/* ------------------------------------------------------------------
   2. ZDANIA Z SILNIKA (powody wstrzymania, zdarzenia koszyka, etapy
      scalania, etykiety zmiennych tematu poczty)

   Mapa jest po DOSŁOWNYM tekście, bo tyle przychodzi w migawce — kodu
   maszynowego backend dziś nie wysyła. Teksty z liczbą w środku
   obsługuje `wzorce` niżej (regexp → klucz + podstawienia).
   ------------------------------------------------------------------ */

/** Dosłowne dopasowanie: tekst z Rusta → klucz słownika. */
const DOSLOWNE: Record<string, string> = {
  "doba zamknięta przez strażnika": "eng.gate.dayClosed",
};

/**
 * Dopasowanie z liczbami: pierwsza pasująca reguła wygrywa.
 * `zmienne` nazywa grupy przechwytujące W KOLEJNOŚCI występowania.
 *
 * Wzorce są zakotwiczone (`^…$`) i biorą liczby jako `(.+?)`, a nie jako
 * `[\d.,]+` — formatowanie liczby po stronie Rusta może się zmienić
 * (separator, spacja przed jednostką) i wtedy luźniejszy wzorzec dalej
 * trafia, zamiast po cichu przestać działać.
 *
 * ŹRÓDŁO: `crates/core/src/engine.rs` (bramka wejścia + strażnik obsunięcia)
 * i `crates/app/src/live.rs` (blokada terminala). Przy dopisywaniu nowego
 * powodu w Ruście: dopisz tu wzorzec + klucz w `en.ts`/`pl.ts` — do tego
 * czasu tekst wyświetla się po polsku, czyli tak jak dziś.
 */
const WZORCE: { re: RegExp; klucz: string; zmienne: string[] }[] = [
  /* --- strażnik obsunięcia: WSTRZYMANIE handlu (czerwony baner) --- */
  { re: /^MAX DRAWDOWN (.+?)% ≥ (.+?)%$/, klucz: "eng.halt.maxDdPct", zmienne: ["a", "b"] },
  { re: /^MAX DRAWDOWN (.+?) \$ ≥ (.+?) \$$/, klucz: "eng.halt.maxDdUsd", zmienne: ["a", "b"] },
  {
    re: /^OBSUNIĘCIE RACHUNKU (.+?)% ≥ (.+?)% \(pułap łańcucha\)$/,
    klucz: "eng.halt.accDdPct",
    zmienne: ["a", "b"],
  },
  {
    re: /^OBSUNIĘCIE RACHUNKU (.+?) \$ ≥ (.+?) \$ \(pułap łańcucha\)$/,
    klucz: "eng.halt.accDdUsd",
    zmienne: ["a", "b"],
  },
  { re: /^PODŁOGA EQUITY ŁAŃCUCHA: (.+?) \$ ≤ (.+?) \$$/, klucz: "eng.halt.chainFloor", zmienne: ["a", "b"] },

  /* --- blokada terminala (rozjazd konta) --- */
  {
    re: /^TERMINAL NA INNYM KONCIE: oczekiwano (.+?), terminal jest na (.+?) \((.+?)\)\./,
    klucz: "eng.halt.wrongAccount",
    zmienne: ["chciany", "jest", "gdzie"],
  },

  /* --- bramka wejścia: sygnał ODRZUCONY (chmurka „wejście zablokowane") --- */
  { re: /^pauza po serii strat \((.+?) min\)$/, klucz: "eng.gate.streakPause", zmienne: ["n"] },
  { re: /^limit ekspozycji \((.+?)\)$/, klucz: "eng.gate.exposure", zmienne: ["n"] },
  { re: /^pułap pozycji rachunku \((.+?)\)$/, klucz: "eng.gate.accPositions", zmienne: ["n"] },
  { re: /^limit koszyków \((.+?)\)$/, klucz: "eng.gate.baskets", zmienne: ["n"] },
  { re: /^pułap koszyków rachunku \((.+?)\)$/, klucz: "eng.gate.accBaskets", zmienne: ["n"] },
  { re: /^podłoga equity (.+?) \$$/, klucz: "eng.gate.floor", zmienne: ["v"] },
  { re: /^podłoga equity łańcucha: (.+?) \$ ≤ (.+?) \$$/, klucz: "eng.gate.chainFloor", zmienne: ["a", "b"] },
  { re: /^cel dnia łańcucha: \+(.+?) \$ ≥ (.+?) \$$/, klucz: "eng.gate.dayTargetPct", zmienne: ["a", "b"] },
  { re: /^cel dnia łańcucha: \+(.+?) \$$/, klucz: "eng.gate.dayTarget", zmienne: ["a"] },
  {
    re: /^dzienny limit straty łańcucha: −(.+?) \$ ≥ (.+?) \$$/,
    klucz: "eng.gate.dayLossPct",
    zmienne: ["a", "b"],
  },
  { re: /^dzienny limit straty łańcucha: −(.+?) \$$/, klucz: "eng.gate.dayLoss", zmienne: ["a"] },
];

/**
 * Tłumaczy pojedyncze zdanie przysłane przez silnik.
 *
 * Nieznane zdanie wraca BEZ ZMIAN — to jest kontrakt tej funkcji i powód,
 * dla którego wolno jej używać wszędzie tam, gdzie panel wyświetla tekst
 * z migawki.
 */
export function tSilnik(tekst: string | undefined | null): string {
  if (!tekst) return "";
  const known = KNOWN_COPY.get(tekst);
  if (known) return t(known);
  const doslowny = DOSLOWNE[tekst];
  if (doslowny && maKlucz(doslowny)) return t(doslowny);
  for (const w of WZORCE) {
    const m = w.re.exec(tekst);
    if (m && maKlucz(w.klucz)) {
      const vars: Record<string, string> = {};
      w.zmienne.forEach((nazwa, i) => (vars[nazwa] = m[i + 1] ?? ""));
      return t(w.klucz, vars);
    }
  }
  return presentEngineText(tekst, getLanguage());
}
