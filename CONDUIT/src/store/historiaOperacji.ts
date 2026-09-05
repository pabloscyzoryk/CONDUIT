

import { useCallback, useEffect, useMemo, useState } from "react";
import * as storage from "@/store/storage";
import type { KluczTlumaczenia } from "@/i18n";
// Typ mieszka w `AppStore` (tam powstaje migawka). Import TYPU znika przy
// kompilacji, więc mimo pozornej pętli nie ma cyklu w module runtime.
import type { EngineSnapshot } from "@/store/AppStore";

/** Komenda w formacie `backend.send` (`AppStore.wyslij`). */
export type Komenda = { cmd: string } & Record<string, unknown>;

/** Pole, które ręczna operacja zmieniła. Nazwy są kluczami słownika łatek. */
export type PoleZmiany = "cena" | "sl" | "tp" | "wolumen";

export interface Zmiana {
  pole: PoleZmiany;
  /** wartość PRZED zmianą; `null` = pole nie było ustawione */
  z: number | null;
  /** wartość PO zmianie */
  na: number | null;
}

export type RodzajCelu = "pending" | "position" | "basket";

export interface WpisOperacji {
  id: number;
  czas: number;
  rodzaj: RodzajCelu;
  /** ticket zlecenia/pozycji albo numer koszyka; `null` dla operacji zbiorczych */
  ticket: number | null;
  /** krótki opis operacji dla listy (np. „zamknięcie zbiorcze: 12 pozycji") */
  opis?: string;
  zmiany: Zmiana[];
  /** komenda, którą wysłano — służy do PONOWIENIA */
  komenda: Komenda | null;
  /** komenda przywracająca stan sprzed; `null` = operacja nieodwracalna */
  odwrotna: Komenda | null;
  /** powód, dla którego `odwrotna` jest pusta */
  powod?: KluczTlumaczenia;
  /** czy wpis został cofnięty (zostaje na liście jako ślad) */
  cofniety: boolean;
  /** Nieprzenoszalna sesja rachunku z chwili utworzenia intencji; "" = nieznana. */
  accountSession?: string;
  /** Trwały ślad unieważnienia: powrót A→B→A nie przywraca starej intencji. */
  accountInvalidated?: boolean;
}

export interface KontekstRachunkuHistorii {
  follow: boolean;
  sessionToken: string | null;
}

type NowyWpisOperacji = Omit<WpisOperacji, "id" | "czas" | "cofniety">;

/** Zapisujemy pochodzenie w chwili intencji, nigdy przy późniejszym wysłaniu.
 *  Jawny pusty token musi pozostać pusty — nie wolno przypisać mu nowego konta. */
export function powiazWpisZRachunkiem(
  w: NowyWpisOperacji,
  kontekst: KontekstRachunkuHistorii,
): NowyWpisOperacji {
  if (!kontekst.follow) return w;
  const accountSession = Object.prototype.hasOwnProperty.call(w, "accountSession")
    ? (typeof w.accountSession === "string" ? w.accountSession : "")
    : (kontekst.sessionToken ?? "");
  return { ...w, accountSession };
}

/** Widok jest blokowany już w pierwszym renderze nowej epoki, PRZED efektem
 *  zapisującym localStorage. Nie usuwamy dziennika ani nie przepisujemy tokenów.
 *  OFF zachowuje stare nieskopowane wpisy; tombstone z follow nie daje się cofnąć. */
export function uniewaznijHistorieRachunku(
  wpisy: WpisOperacji[],
  kontekst: KontekstRachunkuHistorii,
): WpisOperacji[] {
  if (!kontekst.follow) return wpisy;
  let zmiana = false;
  const wynik = wpisy.map((w) => {
    if (w.accountInvalidated) return w;
    if (kontekst.sessionToken && w.accountSession === kontekst.sessionToken) return w;
    zmiana = true;
    return { ...w, accountInvalidated: true };
  });
  return zmiana ? wynik : wpisy;
}

/** Ile czasu wpis pozostaje odwracalny. Po tym czasie zostaje samym zapisem. */
export const OKNO_COFNIECIA_MS = 60 * 60 * 1000;

const KLUCZ_MAGAZYNU = "historiaOperacji";
const LIMIT_WPISOW = 200;

/** Numer wpisu — rosnący, żeby dwie operacje w tej samej milisekundzie
 *  nie dostały tego samego identyfikatora. */
let kolejny = 1;

/** Czy dwie liczby to ta sama wartość ceny — porównanie z tolerancją grosza. */
const rowne = (a: number | null, b: number | null): boolean => {
  if (a === null || a === 0) return b === null || b === 0;
  if (b === null || b === 0) return false;
  return Math.abs(a - b) < 0.005;
};

/**
 * Czy wpis da się dziś cofnąć — i jeśli nie, to dlaczego.
 *
 * Zwraca `null`, gdy cofnięcie jest bezpieczne. W przeciwnym razie klucz
 * słownika z powodem, który idzie PROSTO na ekran podglądu. Cisza w tym
 * miejscu byłaby najgorsza z możliwych: operator kliknąłby „cofnij"
 * i skasował świeżą decyzję bota, nie swoją pomyłkę.
 */
export function przeszkodaCofniecia(
  w: WpisOperacji,
  snapshot: EngineSnapshot,
  teraz = Date.now(),
): KluczTlumaczenia | null {
  if (w.accountInvalidated) return "undo.reason.accountChanged";
  if (w.cofniety) return "undo.reason.alreadyUndone";
  if (!w.odwrotna) return w.powod ?? "undo.reason.bulk";
  if (teraz - w.czas > OKNO_COFNIECIA_MS) return "undo.reason.stale";
  if (w.ticket === null) return "undo.reason.bulk";

  if (w.rodzaj === "pending") {
    const o = snapshot.pendings.find((x) => x.ticket === w.ticket);
    if (!o) return "undo.reason.gone";
    const terazniejsze: Record<PoleZmiany, number | null> = {
      cena: o.price,
      sl: o.sl,
      tp: o.tp,
      wolumen: o.volume,
    };
    for (const z of w.zmiany) {
      if (!rowne(terazniejsze[z.pole], z.na)) return "undo.reason.moved";
    }
    return null;
  }

  if (w.rodzaj === "position") {
    const p = snapshot.positions.find((x) => x.ticket === w.ticket);
    if (!p) return "undo.reason.gone";
    const terazniejsze: Record<PoleZmiany, number | null> = {
      cena: p.openPrice,
      sl: p.sl,
      tp: p.tp,
      wolumen: p.volume,
    };
    for (const z of w.zmiany) {
      if (!rowne(terazniejsze[z.pole], z.na)) return "undo.reason.moved";
    }
    return null;
  }

  return "undo.reason.bulk";
}

/** Odwrócenie zmian do podglądu „co zostanie cofnięte" (na ↔ z). */
export function odwroc(zmiany: Zmiana[]): Zmiana[] {
  return zmiany.map((z) => ({ pole: z.pole, z: z.na, na: z.z }));
}

/** Same pola, które REALNIE się zmieniły — reszta zaśmieca podgląd. */
export function tylkoRozne(zmiany: Zmiana[]): Zmiana[] {
  return zmiany.filter((z) => !rowne(z.z, z.na));
}

function wczytaj(): WpisOperacji[] {
  const l = storage.load<WpisOperacji[]>(KLUCZ_MAGAZYNU, []);
  if (!Array.isArray(l)) return [];
  const lista = l.slice(0, LIMIT_WPISOW);
  // Numeracja musi ruszyć POWYŻEJ wczytanych wpisów, inaczej nowa operacja
  // dostałaby identyfikator, który już jest na liście z poprzedniej sesji.
  for (const w of lista) if (typeof w.id === "number" && w.id >= kolejny) kolejny = w.id + 1;
  return lista;
}

export interface Historia {
  /** Wpisy od NAJNOWSZEGO. */
  wpisy: WpisOperacji[];
  /** Zapis operacji — wołane z akcji `AppStore` PRZED wysłaniem komendy. */
  zapisz: (w: NowyWpisOperacji) => void;
  /** Najnowszy wpis nadający się do cofnięcia (albo `null`). */
  doCofniecia: WpisOperacji | null;
  /** Najświeższy cofnięty wpis — kandydat do ponowienia. */
  doPonowienia: WpisOperacji | null;
  /** Ile nieodwracalnych operacji wykonano PO `doCofniecia`. */
  nieodwracalnePo: number;
  cofnij: () => void;
  ponow: () => void;
  wyczysc: () => void;
}

/**
 * Stan historii. `wykonaj` to ta sama droga, którą idą wszystkie akcje
 * panelu (`AppStore.wyslij` → `backend.send`) — cofnięcie NIE jest
 * osobnym kanałem do brokera, tylko zwykłą komendą z odwróconymi
 * wartościami. Dzięki temu silnik widzi je tak samo jak każdą inną
 * ręczną zmianę i tak samo je zapisuje.
 */
export function useHistoriaOperacji(
  wykonaj: (k: Komenda, opis: string, accountSession?: string) => void,
  snapshot: EngineSnapshot,
  kontekst: KontekstRachunkuHistorii = { follow: false, sessionToken: null },
): Historia {
  const [zapisaneWpisy, setWpisy] = useState<WpisOperacji[]>(wczytaj);
  const { follow, sessionToken } = kontekst;
  const wpisy = useMemo(
    () => uniewaznijHistorieRachunku(zapisaneWpisy, { follow, sessionToken }),
    [zapisaneWpisy, follow, sessionToken],
  );

  useEffect(() => {
    if (wpisy === zapisaneWpisy) return;
    setWpisy((stare) => {
      const nowe = uniewaznijHistorieRachunku(stare, { follow, sessionToken });
      if (nowe !== stare) storage.save(KLUCZ_MAGAZYNU, nowe);
      return nowe;
    });
  }, [wpisy, zapisaneWpisy, follow, sessionToken]);

  const zapiszStan = useCallback((l: WpisOperacji[]) => {
    setWpisy(l);
    storage.save(KLUCZ_MAGAZYNU, l.slice(0, LIMIT_WPISOW));
  }, []);

  /* ZAPIS IDZIE PRZED WYSŁANIEM KOMENDY — świadomie.
     Wartość sprzed zmiany istnieje tylko do chwili, w której broker ją
     nadpisze; czekanie na potwierdzenie znaczyłoby, że przy zerwanej
     łączności NIE MA śladu po tym, co operator próbował zrobić. Cena:
     odrzucona komenda też zostawia wpis. Nie jest to groźne — przy próbie
     cofnięcia `przeszkodaCofniecia` zobaczy, że wartości na rachunku nie
     odpowiadają temu, co zapisaliśmy, i zablokuje operację. */
  const zapisz = useCallback<Historia["zapisz"]>((w) => {
    // Scalar token belongs to this callback/intent, not a mutable latest ref.
    const intencja = powiazWpisZRachunkiem(w, { follow, sessionToken });
    setWpisy((stare) => {
      const nowe = [{ ...intencja, id: kolejny++, czas: Date.now(), cofniety: false }, ...stare].slice(0, LIMIT_WPISOW);
      storage.save(KLUCZ_MAGAZYNU, nowe);
      return nowe;
    });
  }, [follow, sessionToken]);

  const doCofniecia = useMemo(() => {
    for (const w of wpisy) {
      if (w.cofniety) continue;
      if (przeszkodaCofniecia(w, snapshot) === null) return w;
      // Pierwszy wpis od góry, który jest odwracalny CO DO ZASADY, ale
      // dziś zablokowany, i tak zwracamy — podgląd ma powiedzieć DLACZEGO
      // nie da się go cofnąć, zamiast udawać, że go nie ma.
      if (w.odwrotna) return w;
    }
    return null;
  }, [wpisy, snapshot]);

  const doPonowienia = useMemo(
    () => wpisy.find((w) => w.cofniety && w.komenda && !w.accountInvalidated) ?? null,
    [wpisy],
  );

  const nieodwracalnePo = useMemo(() => {
    if (!doCofniecia) return 0;
    let n = 0;
    for (const w of wpisy) {
      if (w.id === doCofniecia.id) break;
      if (!w.odwrotna) n += 1;
    }
    return n;
  }, [wpisy, doCofniecia]);

  const cofnij = useCallback(() => {
    if (!doCofniecia?.odwrotna) return;
    if (przeszkodaCofniecia(doCofniecia, snapshot) !== null) return;
    wykonaj(doCofniecia.odwrotna, `Cofnięcie #${doCofniecia.ticket ?? "—"}`, doCofniecia.accountSession);
    zapiszStan(wpisy.map((w) => (w.id === doCofniecia.id ? { ...w, cofniety: true } : w)));
  }, [doCofniecia, snapshot, wykonaj, wpisy, zapiszStan]);

  const ponow = useCallback(() => {
    if (!doPonowienia?.komenda || doPonowienia.accountInvalidated) return;
    wykonaj(doPonowienia.komenda, `Ponowienie #${doPonowienia.ticket ?? "—"}`, doPonowienia.accountSession);
    zapiszStan(wpisy.map((w) => (w.id === doPonowienia.id ? { ...w, cofniety: false } : w)));
  }, [doPonowienia, wykonaj, wpisy, zapiszStan]);

  const wyczysc = useCallback(() => zapiszStan([]), [zapiszStan]);

  return { wpisy, zapisz, doCofniecia, doPonowienia, nieodwracalnePo, cofnij, ponow, wyczysc };
}
