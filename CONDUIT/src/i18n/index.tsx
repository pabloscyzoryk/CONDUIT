

import { createContext, useCallback, useContext, useEffect, useSyncExternalStore, type ReactNode } from "react";
import * as storage from "@/store/storage";
import { api } from "@/store/transport";
import { EN } from "./en";
import { PL } from "./pl";

export type Jezyk = "en" | "pl";

/** Klucz istniejący w słowniku. EN jest źródłem prawdy (PL jest do niego
 *  typowany), więc ten typ obejmuje CAŁY zestaw kluczy panelu. Używają go
 *  miejsca, które przechowują sam klucz zamiast gotowego napisu —
 *  np. powód blokady cofnięcia w `store/historiaOperacji.ts`. */
export type KluczTlumaczenia = keyof typeof EN;

/** Kolejność = kolejność cyklu skrótu `L`. Pierwszy wpis to język domyślny.
 *  Nazwa języka jest W TYM języku (wymaganie nr 4) — niezależnie od tego,
 *  który język jest aktywny. Dodanie trzeciego języka: dopisz wpis tutaj,
 *  dołóż słownik w `DICTS` — cykl i selektor biorą listę stąd. */
export const LANGUAGES: { id: Jezyk; flag: string; native: string }[] = [
  { id: "en", flag: "🇬🇧", native: "English" },
  { id: "pl", flag: "🇵🇱", native: "Polski" },
];

export const DEFAULT_LANGUAGE: Jezyk = "en";

const DICTS: Record<Jezyk, Record<string, string>> = { en: EN, pl: PL };

/** Klucz w localStorage (prefiks `conduit.` dokłada `storage`). */
const KLUCZ_MAGAZYNU = "language";

/** Nazwa klucza w GŁÓWNYM dokumencie ustawień serwera. Jedno miejsce —
 *  używa go też AppStore przy czytaniu migawki. */
export const KLUCZ_JEZYKA = "language";

function poprawny(l: unknown): l is Jezyk {
  return typeof l === "string" && LANGUAGES.some((x) => x.id === l);
}

/* ---------------- mini external-store ---------------- */

let biezacy: Jezyk = (() => {
  const zapisany = storage.load<string>(KLUCZ_MAGAZYNU, DEFAULT_LANGUAGE);
  return poprawny(zapisany) ? zapisany : DEFAULT_LANGUAGE;
})();

/** Tryb testu czerwienienia: `?i18n=keys` — t() zwraca same klucze. */
const TYLKO_KLUCZE =
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).get("i18n") === "keys";

const nasluch = new Set<() => void>();

function subscribe(cb: () => void): () => void {
  nasluch.add(cb);
  return () => nasluch.delete(cb);
}

export function getLanguage(): Jezyk {
  return biezacy;
}

/** Szczegóły ostatniej zmiany — AppStore pokazuje toast TYLKO dla "user"
 *  (zmiana z serwera przy starcie nie ma prawa strzelać chmurką). */
export type ZrodloZmiany = "user" | "server";

function ustaw(l: Jezyk, zrodlo: ZrodloZmiany): void {
  if (l === biezacy) return;
  biezacy = l;
  storage.save(KLUCZ_MAGAZYNU, l);
  if (typeof document !== "undefined") document.documentElement.lang = l;
  if (zrodlo === "user") {
    // Klucz GŁÓWNEGO dokumentu — Rust wyjmuje go z łatki PRZED scaleniem
    // do settings{} (wzorzec mt5_password), więc nie zdejmie etykiety
    // presetu. Brak backendu to normalny stan — cisza zamiast błędu.
    void api.patchSettings({ [KLUCZ_JEZYKA]: l }).catch(() => undefined);
  }
  for (const cb of nasluch) cb();
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent("conduit:language", { detail: { language: l, source: zrodlo } }));
  }
}

/** Zmiana z UI (selektor w „Wygląd") — ta sama ścieżka co skrót L. */
export function setLanguage(l: Jezyk): void {
  ustaw(l, "user");
}

/** Skrót `L`: następny język z listy, cyklicznie. */
export function cycleLanguage(): void {
  const i = LANGUAGES.findIndex((x) => x.id === biezacy);
  const nast = LANGUAGES[(i + 1) % LANGUAGES.length];
  ustaw(nast.id, "user");
}

/** Wybór przysłany przez serwer w migawce (klucz główny `language`).
 *  NIE odsyłamy łatki z powrotem — serwer już to ma. */
export function applyServerLanguage(l: unknown): void {
  if (poprawny(l)) ustaw(l, "server");
}

/* ---------------- tłumaczenie ---------------- */

const zgloszone = new Set<string>();

/** Płaska podstawa: `{nazwa}` w tekście podmieniana wartością z `vars`. */
function podstaw(tekst: string, vars?: Record<string, string | number>): string {
  if (!vars) return tekst;
  return tekst.replace(/\{(\w+)\}/g, (calosc, nazwa: string) =>
    nazwa in vars ? String(vars[nazwa]) : calosc,
  );
}

/**
 * Tłumaczenie klucza w JĘZYKU BIEŻĄCYM.
 *
 * Dostępne także poza Reactem (toasty AppStore, opisy budowane w funkcjach).
 * Komponenty używają `useT()`, żeby przerysować się po zmianie języka —
 * sama funkcja `t` jest tą samą referencją, świeżość daje subskrypcja.
 */
export function t(key: string, vars?: Record<string, string | number>): string {
  if (TYLKO_KLUCZE) return key;
  const wlasny = DICTS[biezacy][key];
  if (wlasny !== undefined) return podstaw(wlasny, vars);
  const en = DICTS.en[key];
  if (en !== undefined) {
    if (biezacy !== "en" && !zgloszone.has(`${biezacy}:${key}`)) {
      zgloszone.add(`${biezacy}:${key}`);
      console.warn(`[i18n] brak klucza „${key}” w słowniku „${biezacy}” — użyto EN`);
    }
    return podstaw(en, vars);
  }
  if (!zgloszone.has(key)) {
    zgloszone.add(key);
    console.warn(`[i18n] klucz „${key}” nie istnieje w ŻADNYM słowniku — pokazano sam klucz`);
  }
  return key;
}

/**
 * Wiek w ludzkich jednostkach — JEDNA reguła dla paska „Puls rynku"
 * i dla dymków, żeby ten sam odstęp nie był raz w sekundach, raz w minutach.
 * Progi (90 s, 90 min) są celowo powyżej okrągłej minuty/godziny: przy 60
 * pasek migałby między „60 s" a „1 min" co drugi tick.
 */
export function wiek(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 90) return t("puls.age.sec", { n: s });
  const m = Math.round(s / 60);
  if (m < 90) return t("puls.age.min", { n: m });
  return t("puls.age.hour", { n: Math.round(m / 60) });
}

/* ---------------- React ---------------- */

interface I18nCtx {
  lang: Jezyk;
  t: typeof t;
  setLanguage: (l: Jezyk) => void;
  cycleLanguage: () => void;
}

const Ctx = createContext<I18nCtx | null>(null);

/** Czy element z fokusem przyjmuje tekst — wtedy `L` pisze literę. */
function fokusWPolu(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  if (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return true;
  return el.isContentEditable;
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const lang = useSyncExternalStore(subscribe, getLanguage, getLanguage);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  /* Globalny skrót `L`. Bez modyfikatorów — Ctrl+L (pasek adresu) i spółka
     zostają w spokoju. Ochrona pól tekstowych obowiązuje ZAWSZE. */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== "l" || e.ctrlKey || e.metaKey || e.altKey) return;
      if (fokusWPolu()) return;
      cycleLanguage();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const set = useCallback((l: Jezyk) => setLanguage(l), []);
  const cycle = useCallback(() => cycleLanguage(), []);

  return <Ctx.Provider value={{ lang, t, setLanguage: set, cycleLanguage: cycle }}>{children}</Ctx.Provider>;
}

/**
 * Hak tłumaczący. Zwraca `t` i subskrybuje komponent na zmianę języka —
 * bez subskrypcji komponent zostałby przy starym języku do następnego
 * przerysowania z innego powodu.
 */
export function useT(): typeof t {
  useSyncExternalStore(subscribe, getLanguage, getLanguage);
  return t;
}

/**
 * Wpis słownika z PROSTYM formatowaniem (`<b>…</b>`).
 *
 * Kilka dłuższych opisów niesie wytłuszczenia. Zamiast rozbijać je na
 * po trzy klucze wokół każdego `<b>`, renderujemy zaufaną (naszą własną,
 * statyczną) treść słownika przez `dangerouslySetInnerHTML`. Do słownika
 * nie trafia nic od użytkownika ani z sieci — podstawienia `{...}` są
 * wartościami liczbowymi/nazwami, które i tak przechodzą przez `String()`.
 */
export function RichT({ k, vars }: { k: string; vars?: Record<string, string | number> }) {
  useSyncExternalStore(subscribe, getLanguage, getLanguage);
  // Only dictionary markup is trusted. Paths, account names and API diagnostics
  // inserted into a translation remain text, including <, &, and quotes.
  const safe = vars && Object.fromEntries(Object.entries(vars).map(([key, value]) => [key,
    String(value).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]!)),
  ]));
  return <span dangerouslySetInnerHTML={{ __html: t(k, safe) }} />;
}

/** Pełny dostęp: język bieżący + akcje. Do selektora w „Wygląd". */
export function useLanguage(): I18nCtx {
  const v = useContext(Ctx);
  const lang = useSyncExternalStore(subscribe, getLanguage, getLanguage);
  if (v) return { ...v, lang };
  // Poza providerem (test czerwienienia, storybook) — działa na module.
  return { lang, t, setLanguage, cycleLanguage };
}
