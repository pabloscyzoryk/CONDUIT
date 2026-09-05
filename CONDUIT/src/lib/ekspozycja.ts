

import { SYMBOLS } from "@/data/symbols";

/* ------------------------------------------------------------
   WIELKOŚĆ KONTRAKTU — BEZ CICHEGO SPADKU NA ZŁOTO
   ------------------------------------------------------------ */

const PO_SYMBOLU = new Map(SYMBOLS.map((s) => [s.symbol, s]));

/**
 * Wielkość kontraktu instrumentu bota albo `null`, gdy symbol jest nieznany.
 *
 * `data/symbols.getSymbol` przy nietrafieniu oddaje PIERWSZY wpis katalogu
 * (złoto, kontrakt 100). Dla wykresu to nieszkodliwe, dla rachunku marginesu
 * byłoby kłamstwem: na EURUSD kontrakt jest 1000× większy, więc panel
 * pokazałby margines 1000× za mały. Tutaj nietrafienie zwraca `null`.
 *
 * Sufiksy brokerskie: PUPrime publikuje złoto jako „XAUUSD.s", inni jako
 * „XAUUSDm". Odcinamy część po kropce, a przy sufiksie bez kropki
 * dopuszczamy dopasowanie po przedrostku, ale tylko gdy ogon ma najwyżej
 * 4 znaki — dłuższy ogon to już inny instrument, nie ozdobnik brokera.
 */
export function kontraktSymbolu(symbol: string): number | null {
  const s = (symbol || "").trim().toUpperCase();
  if (!s) return null;
  const dokladny = PO_SYMBOLU.get(s);
  if (dokladny) return dokladny.contractSize;
  const bezKropki = PO_SYMBOLU.get(s.split(".")[0]);
  if (bezKropki) return bezKropki.contractSize;
  for (const meta of SYMBOLS) {
    if (s.startsWith(meta.symbol) && s.length - meta.symbol.length <= 4) return meta.contractSize;
  }
  return null;
}

/* ------------------------------------------------------------
   SUFIT WOLUMENU
   ------------------------------------------------------------ */

/** Tyle nogi potrzebuje sufit — reszta pól `stats.lotNogi` jest tu zbędna. */
export interface NogaEkspozycji {
  /** wolumen JEDNEGO ZLECENIA (jeden szczebel siatki) */
  lot: number;
  /** planowany wolumen CAŁEGO koszyka (`Engine::lot_koszyka_planowany`) */
  lotKoszyka?: number;
  /** czy noga bierze NOWE sygnały */
  handluje: boolean;
  /** `max_lotow` łańcucha widziany przez SILNIK tej nogi (`0` = brak) */
  pulapLancucha?: number;
}

export interface PulapyEkspozycji {
  maxLotow: number;
  maxPozycji: number;
  maxKoszykow: number;
}

/** Co ustala sufit: sama fala nóg czy któryś pułap łańcucha. */
export type CoWiaze = "fala" | "loty" | "pozycje";

export interface SufitAuto {
  /** suma koszyków nóg handlujących — jedna fala, gdy każda dostanie sygnał */
  fala: number;
  /** suma lotów ZLECEŃ (stara liczba kafla, zostaje do podpisu) */
  falaZlecen: number;
  /** czy KAŻDA grająca noga podała `lotKoszyka` (inaczej nic nie obiecujemy) */
  koszykZnany: boolean;
  /** największy lot pojedynczego zlecenia — mnożnik pułapu pozycji */
  najwiekszeZlecenie: number;
  /** największy koszyk pojedynczej nogi — mnożnik pułapu koszyków */
  najwiekszyKoszyk: number;
  /** pułap `maxLotow` (`0` = brak) */
  pulapLotow: number;
  /** `maxPozycji × najwiekszeZlecenie` (`0` = brak albo nie ma z czego liczyć) */
  pulapZPozycji: number;
  maxPozycji: number;
  maxKoszykow: number;
  /** min(fala, pułapy) — liczba, którą pokazuje kafel */
  sufit: number;
  wiaze: CoWiaze;
  /**
   * GRANICA Z PUŁAPU KOSZYKÓW: `maxKoszykow × najwiekszyKoszyk` (`0` = nie
   * da się policzyć). To jest górna granica, której pułapy lotów i pozycji
   * NIE gwarantują — patrz komentarz przy `policzSufit`.
   */
  granicaKoszykow: number;
  /** czy w ogóle jest o czym mówić (jest choć jedna grająca noga) */
  sanNogi: boolean;
}

const koszykNogi = (n: NogaEkspozycji): number =>
  typeof n.lotKoszyka === "number" && n.lotKoszyka > 0 ? n.lotKoszyka : n.lot;


export function policzSufit(nogi: NogaEkspozycji[], pulapy: PulapyEkspozycji): SufitAuto {
  const grajace = nogi.filter((n) => n.handluje);
  const fala = grajace.reduce((a, n) => a + koszykNogi(n), 0);
  const falaZlecen = grajace.reduce((a, n) => a + n.lot, 0);
  const koszykZnany =
    grajace.length > 0 && grajace.every((n) => typeof n.lotKoszyka === "number" && n.lotKoszyka > 0);
  const najwiekszeZlecenie = grajace.reduce((a, n) => Math.max(a, n.lot), 0);
  const najwiekszyKoszyk = grajace.reduce((a, n) => Math.max(a, koszykNogi(n)), 0);

  /* PUŁAP LOTÓW BIERZEMY Z SILNIKA, gdy tylko go poda. `ui::LotNogi.
     pulap_lancucha` to `silniki.pulapy.max_lotow` — czyli pułap, którym
     silnik NAPRAWDĘ się bramkuje. Widok łańcucha (`lancuchy.json` przez
     store) jest zapasem dla starszej binarki; gdy oba są, wygrywa silnik,
     bo to on odmawia zleceń. */
  const zSilnika = grajace.reduce((a, n) => Math.max(a, n.pulapLancucha ?? 0), 0);
  const pulapLotow = zSilnika > 0 ? zSilnika : Math.max(0, pulapy.maxLotow);
  const pulapZPozycji =
    pulapy.maxPozycji > 0 && najwiekszeZlecenie > 0 ? pulapy.maxPozycji * najwiekszeZlecenie : 0;

  let sufit = fala;
  let wiaze: CoWiaze = "fala";
  if (pulapLotow > 0 && pulapLotow < sufit) {
    sufit = pulapLotow;
    wiaze = "loty";
  }
  if (pulapZPozycji > 0 && pulapZPozycji < sufit) {
    sufit = pulapZPozycji;
    wiaze = "pozycje";
  }

  return {
    fala,
    falaZlecen,
    koszykZnany,
    najwiekszeZlecenie,
    najwiekszyKoszyk,
    pulapLotow,
    pulapZPozycji,
    maxPozycji: Math.max(0, pulapy.maxPozycji),
    maxKoszykow: Math.max(0, pulapy.maxKoszykow),
    sufit,
    wiaze,
    granicaKoszykow:
      pulapy.maxKoszykow > 0 && najwiekszyKoszyk > 0 ? pulapy.maxKoszykow * najwiekszyKoszyk : 0,
    sanNogi: grajace.length > 0,
  };
}

/* ------------------------------------------------------------
   PRZELICZENIE NA RACHUNEK
   ------------------------------------------------------------ */

/** Czego brakuje, żeby policzyć margines. Kody, nie zdania — tłumaczy panel. */
export type BrakDanej = "cena" | "dzwignia" | "kontrakt" | "saldo" | "koszyk";

export interface Rachunek {
  balance: number;
  equity: number;
  /** dźwignia rachunku z terminala (`connection.account.leverage`) */
  dzwignia: number;
  /** cena, po której liczymy margines (ask, gdy jest — inaczej bid) */
  cena: number;
  /** wielkość kontraktu symbolu bota; `null` = symbol nieznany panelowi */
  kontrakt: number | null;
}

/** Czego brakuje w `Rachunek`, żeby policzyć cokolwiek. Pusta lista = komplet. */
export function brakiRachunku(r: Rachunek): BrakDanej[] {
  const braki: BrakDanej[] = [];
  if (!Number.isFinite(r.cena) || r.cena <= 0) braki.push("cena");
  if (!Number.isFinite(r.dzwignia) || r.dzwignia < 1) braki.push("dzwignia");
  if (!r.kontrakt || r.kontrakt <= 0) braki.push("kontrakt");
  if (!Number.isFinite(r.equity) || r.equity <= 0) braki.push("saldo");
  return braki;
}

/** Margines wymagany przez `lot` — `null`, gdy czegokolwiek brakuje. */
export function margines(lot: number, r: Rachunek): number | null {
  if (!Number.isFinite(lot) || lot <= 0) return null;
  if (brakiRachunku(r).length > 0) return null;
  return (lot * (r.kontrakt as number) * r.cena) / r.dzwignia;
}

/** Poziom marginesu (%) przy TAKIM marginesie — wzór `Engine::poziom_marginesu`. */
export function poziomMarginesu(equity: number, m: number | null): number | null {
  if (m === null || !Number.isFinite(m) || m <= 0) return null;
  if (!Number.isFinite(equity) || equity <= 0) return null;
  return (equity / m) * 100;
}


export type StanMarginesu = "ok" | "uwaga" | "zle" | "alarm" | "nieznany";

export function stanMarginesu(ml: number | null): StanMarginesu {
  if (ml === null || !Number.isFinite(ml)) return "nieznany";
  if (ml < 100) return "alarm";
  if (ml < 150) return "zle";
  if (ml < 300) return "uwaga";
  return "ok";
}

/** Komplet liczb dla JEDNEJ wielkości wolumenu. */
export interface KosztLota {
  lot: number;
  margines: number | null;
  /** margines jako % salda (balance) */
  pctSalda: number | null;
  /** poziom marginesu, gdy rachunek trzyma DOKŁADNIE ten wolumen */
  ml: number | null;
  stan: StanMarginesu;
}

export function kosztLota(lot: number, r: Rachunek): KosztLota {
  const m = margines(lot, r);
  const ml = poziomMarginesu(r.equity, m);
  return {
    lot,
    margines: m,
    pctSalda: m !== null && Number.isFinite(r.balance) && r.balance > 0 ? (m / r.balance) * 100 : null,
    ml,
    stan: stanMarginesu(ml),
  };
}
