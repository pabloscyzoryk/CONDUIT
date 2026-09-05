/* ============================================================
   KRONIKA — typy kontraktu z Rustem.

   Odpowiedniki struktur z `crates/server/src/kronika/`. Ten sam JSON
   wystawiają DWA programy:
     * `conduit.exe`  — zakładka „Kronika” (`/api/kronika/*`),
     * `kronika.exe`  — samodzielny rejestrator (te same trasy).

   Dzięki temu `KronikaView` jest JEDNYM komponentem obsługującym oba,
   tak samo jak `AiModelsView` obsługuje panel i `wizualizacja.exe`.

   ⚠ Zmiana pola tutaj MUSI iść razem ze zmianą w Ruście. Pole dodane
   po jednej stronie jest niewidoczne po drugiej i nie zgłasza błędu.
   ============================================================ */

/** Co się stało. `start`/`stop` to znaczniki sesji rejestratora, nie wiadomości. */
export type KronikaRodzaj = "nowa" | "edycja" | "skasowana" | "start" | "stop";

/** Jeden wiersz pliku — dokładnie to, co zobaczyliśmy, bez interpretacji. */
export interface KronikaWpis {
  v: number;
  seq: number;
  rodzaj: KronikaRodzaj;
  /** NASZ czas odbioru — jedyny, którego Telegram nie przepisuje */
  odebrano_ms: number;
  odebrano: string;
  /** znacznik od Telegrama; przy edycji bywa STARSZY niż `odebrano_ms` */
  ts_telegram_ms: number;
  chat_id: number;
  chat: string;
  temat?: number | null;
  msg_id: number;
  reply_to?: number | null;
  edit_of?: number | null;
  text: string;
  znakow: number;
  /** czy bot nasłuchiwał tego źródła w chwili odbioru */
  nasluchiwany: boolean;
  format?: string | null;
  rozpoznane: boolean;
  /** opis przy znaczniku sesji */
  uwaga?: string | null;
}

/* ---------------- opcje zapisu ---------------- */

export interface KronikaZrodlo {
  chat_id: number;
  /** brak tematu = CAŁY kanał, razem ze wszystkimi tematami */
  temat?: number | null;
}

export type KronikaZrodla =
  | { tryb: "wszystkie" }
  | { tryb: "wybrane"; lista: KronikaZrodlo[] };

export type KronikaFsync = { tryb: "kazda" } | { tryb: "co"; n: number } | { tryb: "nigdy" };

export interface KronikaUstawienia {
  wlaczona: boolean;
  /** JEDEN ciągły plik; ścieżka względna liczy się od katalogu roboczego */
  plik: string;
  zrodla: KronikaZrodla;
  /** zapisywać wiadomości, których parser nie rozpoznał (domyślnie TAK) */
  nierozpoznane: boolean;
  /** zapisywać zdarzenia bez treści tekstowej */
  puste: boolean;
  fsync: KronikaFsync;
  /** obrót pliku po przekroczeniu rozmiaru (MB); 0 = jeden plik bez końca */
  obrot_mb: number;
  /** ile obróconych plików trzymać; 0 = wszystkie */
  trzymaj_plikow: number;
  znaczniki_sesji: boolean;
  strefa_h: number;
}



export interface KronikaLiczniki {
  zapisanych: number;
  nowych: number;
  edycji: number;
  skasowanych: number;
  /** odrzucone przez opcje zapisu — NIE błędy */
  pominietych: number;
  bledow: number;
  ostatni_blad?: string | null;
  bajtow: number;
  obrotow: number;
  start_ms: number;
  ostatnie_ms: number;
}

export interface KronikaStan {
  ok: boolean;
  wersja: string;
  /** `wbudowana` = zakładka w Conduicie, `samodzielna` = kronika.exe */
  tryb: "samodzielna" | "wbudowana";
  plik: string;
  /** sciezka, ktora program wybralby SAM (domyslnie plik na pulpicie SERWERA) */
  domyslny_plik?: string;
  istnieje: boolean;
  /** co zastano w pliku przy otwarciu — „kontynuuje N wpisow" vs „nowy plik" */
  rozpoznanie?: KronikaRozpoznanie;
  /** gdzie lezy kopia zapasowa zrobiona przed pierwszym dopisaniem */
  kopia?: string;
  bajtow: number;
  plikow: number;
  ustawienia: KronikaUstawienia;
  liczniki: KronikaLiczniki;
  ostatnie: KronikaWpis[];
  zrodlo_zywe: boolean;
  /** jednozdaniowe DLACZEGO — panel nie może milczeć, gdy nic nie wpada */
  zrodlo_opis: string;
}


export interface KronikaRozpoznanie {
  istnial: boolean;
  bajtow: number;
  wierszy: number;
  ostatni_ms: number;
  ostatni_opis: string;
  schemat_pierwszy: number;
  schemat_ostatni: number;
  /** plik zapisany NOWSZA wersja schematu — dopisujemy, ale ostrzegamy */
  obcy_format: boolean;
  nieczytelny: boolean;
}

/* ---------------- statystyka ---------------- */

export interface KronikaRozklad {
  n: number;
  min_s: number;
  p50_s: number;
  p90_s: number;
  max_s: number;
  srednia_s: number;
}

export interface KronikaStatZrodla {
  chat_id: number;
  chat: string;
  temat?: number | null;
  nasluchiwany: boolean;
  wiadomosci: number;
  edytowanych: number;
  procent_edytowanych: number;
  edycji: number;
  skasowanych: number;
  p50_pierwsza_s: number;
  ostatnia_ms: number;
}

/** Czas, o którym kronika NIC nie wie — z znaczników sesji. */
export interface KronikaPrzerwa {
  od_ms: number;
  do_ms: number;
  sekund: number;
  /** brak znacznika `stop` = proces zginął, nie został zamknięty */
  nagle: boolean;
}

export interface KronikaStatystyki {
  wpisow: number;
  plikow: number;
  bajtow: number;
  uszkodzonych: number;

  wiadomosci: number;
  nowych: number;
  edycji: number;
  skasowanych: number;

  /** GŁÓWNA LICZBA CAŁEGO NARZĘDZIA */
  edytowanych: number;
  procent_edytowanych: number;
  /** sekundy do PIERWSZEJ edycji — tego eksport z Telegrama nie umie */
  do_pierwszej: KronikaRozklad;
  /** sekundy do OSTATNIEJ edycji — to jedyne, co pokazuje eksport */
  do_ostatniej: KronikaRozklad;
  max_edycji_jednej: number;
  wielokrotnie_edytowanych: number;

  /**
   * Wiadomości, których wersji PIERWSZEJ nigdy nie widzieliśmy — mamy samą
   * edycję, bo rejestrator ruszył po publikacji. Nie wchodzą do `wiadomosci`
   * ani do rozkładów: byłyby „edytowane w 100 %" z odstępem 0 s, czyli
   * zawyżałyby procent i zerowały medianę.
   */
  bez_oryginalu: number;

  /** edycje, które przyszły PO pierwszej odpowiedzi — łamią przyczynowość */
  edycji_po_odpowiedzi: number;
  po_odpowiedzi: KronikaRozklad;

  od_ms: number;
  do_ms: number;
  zrodel: number;
  wg_zrodla: KronikaStatZrodla[];
  przerwy: KronikaPrzerwa[];
  przerw_sekund: number;
}

/* ---------------- kanały ---------------- */

export interface KronikaTemat {
  id: number;
  nazwa: string;
  nagrywany: boolean;
  wpisow: number;
}

export interface KronikaKanal {
  chat_id: number;
  nazwa: string;
  handle?: string | null;
  forum: boolean;
  tematy: KronikaTemat[];
  /** czy BOT handluje tym kanałem — niezależne od nagrywania */
  nasluchiwany: boolean;
  nagrywany: boolean;
  wpisow: number;
}

export interface KronikaPodsumowanieEksportu {
  plik: string;
  plikow_zrodlowych: number;
  wpisow: number;
  uszkodzonych: number;
  sygnalow: number;
  zdarzen: number;
  /** ta część zbioru, której eksport z aplikacji Telegram NIE MA */
  z_edycji: number;
  bajtow: number;
}
