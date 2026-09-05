import type { Format, Lancuch, Lancuchy, PulapyGlobalne, TradingMode } from "@/types";

/* ============================================================
   FORMATY SYGNAŁÓW I ŁAŃCUCHY PRESETÓW

   Odwzorowanie `crates/core/src/formaty.rs` — jeden do jednego, z tymi
   samymi nazwami pól (serializacja po stronie Rusta to camelCase, więc
   `maxPozycji` z tego pliku to dosłownie to, co przychodzi w JSON-ie).

   Trzy pojęcia, bo bez nich panel kłamie przy dwóch kanałach naraz:

   * FORMAT  — sposób podawania sygnałów przez kanał. Kanał (a w kanale-forum
     KAŻDY TEMAT osobno) ma DOKŁADNIE JEDEN format. Nie zero i nie kilka.
   * PRESET  — ustawienia zarządzania, zawsze przypisane do jednego formatu.
     Ustawienia, które zarabiają na ATFX, na innym formacie bywają szkodliwe
     — nie dlatego, że są złe, tylko dlatego, że opisują inny sposób handlu.
   * ŁAŃCUCH — mapa `format → preset` plus pułapy obowiązujące ponad presetami.
     Bot ma zawsze dokładnie jeden aktywny łańcuch.

   Ten plik jest KATALOGIEM WBUDOWANYM: obowiązuje w trybie projektowym
   (bez `conduit.exe`) i jako punkt wyjścia, zanim serwer przyśle własny
   zbiór łańcuchów.
   ============================================================ */


export const DOMYSLNY_FORMAT = "ATFX";

/** Formaty wbudowane. Lista rośnie, gdy dochodzi kanał o innym zapisie. */
export const FORMATY_WBUDOWANE: Format[] = [
  "ATFX", "Synergy", "ZEN", "PULSEX", "NOVA", "TWP", "DANGER", "CLUB1", "STORM",
].map((nazwa) => ({
  nazwa,
  parser: "atfx",
  opis: "Wbudowana składnia sygnałów. Przypisz własny kanał lub temat przed użyciem.",
  przyklad: "BUY LIMITS GOLD @ 2100/2095 AREA\nTP 2105\nTP 2110\nSL 2090",
}));

/** Pułapy „wszystko zero" — same presety rządzą. */
export const PUSTE_PULAPY: PulapyGlobalne = {
  maxPozycji: 0,
  maxKoszykow: 0,
  maxLotow: 0,
  maxLotowKierunkowo: 0,
  maxRyzykoPct: 0,
  maxDdPct: 0,
  maxDdUsd: 0,
  podlogaEquityUsd: 0,
  celDniaUsd: 0,
  celDniaPct: 0,
  celDniaZamyka: false,
  limitStratyDniaUsd: 0,
  limitStratyDniaPct: 0,
  blokujPrzeciwneKierunki: false,
  pauzaPoStratachN: 0,
  pauzaPoStratachMin: 0,
};

function lancuch(nazwa: string, opis: string, pary: [string, string][], pulapy: Partial<PulapyGlobalne> = {}): Lancuch {
  return {
    nazwa,
    opis,
    presety: Object.fromEntries(pary),
    pulapy: { ...PUSTE_PULAPY, ...pulapy },
  };
}

/** Łańcuchy wbudowane — punkt wyjścia, użytkownik dokłada własne w panelu. */
export const LANCUCHY_WBUDOWANE: Lancuch[] = [
  lancuch(
    "GOD-X7-SOLO",
    "Publiczny przykład: tylko jawnie przypisany format Synergy korzysta z presetu GOD-X7.",
    [
      ["Synergy", "GOD-X7"],
      ["ATFX", ""],
      ["ZEN", ""],
      ["PULSEX", ""],
      ["NOVA", ""],
      ["TWP", ""],
      ["DANGER", ""],
      ["CLUB1", ""],
      ["STORM", ""],
    ],
    {},
  ),
];

/** Domyślny zbiór łańcuchów — odpowiednik `Lancuchy::default()` z Rusta. */
export const DOMYSLNE_LANCUCHY: Lancuchy = {
  aktywny: "GOD-X7-SOLO",
  lista: LANCUCHY_WBUDOWANE,
};

/* ------------------------------------------------------------------
   POMOCNIKI — te same reguły co `Lancuch::preset_dla` i `PulapyGlobalne`
   ------------------------------------------------------------------ */

/**
 * Preset dla danego formatu; `null` znaczy „ten format NIE HANDLUJE".
 *
 * Pusty łańcuch znaków znaczy dokładnie to samo co brak klucza. Panel przy
 * wyborze „nie handluj" zapisuje pustkę, a nie kasuje wpis — obie postacie
 * muszą znaczyć to samo, inaczej wybór „żaden" cicho włączałby handel
 * presetem o pustej nazwie.
 */
export function presetDlaFormatu(l: Lancuch | undefined, format: string): string | null {
  const v = l?.presety?.[format];
  return v && v.trim() !== "" ? v : null;
}

/** Aktywny łańcuch ze zbioru; `undefined`, gdy wskazanie jest nieaktualne. */
export function aktywnyLancuch(z: Lancuchy): Lancuch | undefined {
  return z.lista.find((l) => l.nazwa === z.aktywny);
}

/**
 * NAZWA ŁAŃCUCHA OBOWIĄZUJĄCA W DANYM TRYBIE (projekt EA-2).
 *
 * Bliźniak `ui::aktywny_dla` z serwera — ta sama reguła musi obowiązywać po
 * obu stronach drutu, inaczej panel pokazuje jeden skład, a bot gra innym.
 *
 * AUTO-EA prowadzi WŁASNY skład i czyta `aktywnyEa`. Wskazanie puste albo
 * pokazujące na łańcuch spoza listy (skasowany po zapisaniu) wraca na wspólne
 * `aktywny` — kontrakt zera: panel sprzed tej wersji i binarka sprzed tej
 * wersji zachowują się dokładnie jak dotąd.
 */
export function aktywnyDla(z: Lancuchy, aktywnyEa: string, tryb: TradingMode): string {
  if (tryb === "AUTO-EA" && aktywnyEa && z.lista.some((l) => l.nazwa === aktywnyEa)) {
    return aktywnyEa;
  }
  return z.aktywny;
}

/** Ten sam wybór, ale od razu jako rekord z listy. */
export function lancuchDla(z: Lancuchy, aktywnyEa: string, tryb: TradingMode): Lancuch | undefined {
  const n = aktywnyDla(z, aktywnyEa, tryb);
  return z.lista.find((l) => l.nazwa === n);
}


export function drabinkaDla<T>(drabinka: T, drabinkaEa: T, tryb: TradingMode): T {
  return tryb === "AUTO-EA" ? drabinkaEa : drabinka;
}

/** Formaty, które w tym łańcuchu mają przypisany preset (czyli handlują). */
export function formatyHandlujace(l: Lancuch | undefined): string[] {
  if (!l) return [];
  return Object.entries(l.presety ?? {})
    .filter(([, p]) => p && p.trim() !== "")
    .map(([f]) => f);
}

/**
 * Sufit dla liczby całkowitej: `0` po DOWOLNEJ stronie znaczy „bez pułapu".
 *
 * To jest wygodne, ale bywa mylące — mieliśmy już pole `reenter_max`, gdzie
 * `0` znaczyło „bez limitu", a czytano je jako „wyłączone", i kosztowało to
 * realny rozjazd wyników. Dlatego interfejs pisze to przy każdym polu wprost.
 */
export function sufit(globalny: number, presetu: number): number {
  if (globalny <= 0) return presetu;
  if (presetu <= 0) return globalny;
  return Math.min(globalny, presetu);
}

/** Czy nazwa łańcucha jest wolna (porównanie bez rozróżniania wielkości liter). */
export function nazwaWolna(z: Lancuchy, nazwa: string, pomin?: string): boolean {
  const n = nazwa.trim().toLowerCase();
  if (!n) return false;
  return !z.lista.some((l) => l.nazwa.toLowerCase() === n && l.nazwa !== pomin);
}

/**
 * Doprowadza zbiór przysłany z zewnątrz do stanu użytecznego.
 *
 * Serwer może przysłać łańcuch bez `pulapy` (starsza binarka) albo wskazać
 * aktywny, którego nie ma na liście. Panel nie ma prawa się na tym wywrócić:
 * to jest ekran, z którego użytkownik steruje handlem.
 */
export function normalizujLancuchy(z: Partial<Lancuchy> | null | undefined): Lancuchy {
  const lista = (z?.lista ?? []).map((l) => ({
    nazwa: l.nazwa,
    opis: l.opis ?? "",
    presety: { ...(l.presety ?? {}) },
    pulapy: { ...PUSTE_PULAPY, ...(l.pulapy ?? {}) },
  }));
  if (lista.length === 0) return { ...DOMYSLNE_LANCUCHY, lista: LANCUCHY_WBUDOWANE.map((l) => ({ ...l })) };
  const aktywny = lista.some((l) => l.nazwa === z?.aktywny) ? z!.aktywny! : lista[0].nazwa;
  return { aktywny, lista };
}
