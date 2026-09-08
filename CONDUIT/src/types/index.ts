/* ============================================================
   CONDUIT — MODEL DOMENOWY
   Odwzorowanie struktur z bot.py (settings, Basket, pozycje,
   pendingi, kanaly, formaty sygnalow, presety, modele AI).
   ============================================================ */

/* ---------------- TRYBY PRACY ---------------- */
/* AUTO-EA (24.08.2026): „potwór" — kierunek i punkty szczególne z sygnałów
   traderów + zarządzanie klasy EA. DZIŚ zachowuje się identycznie jak AUTO
   (kontrakt zera); różni go wyłącznie flaga silnika `tryb_auto_ea`, którą
   będą konsultować nadchodzące osie warstwy EA (trailing S/R, harvest). */
export type TradingMode = "MANUAL" | "AUTO" | "AUTO-EA" | "AI";

export type ThemeName = "dark" | "light";

export type PaletteName =
  | "violet"
  | "red"
  | "green"
  | "yellow"
  | "orange"
  | "matrix"
  | "vantage"
  | "puprime";

/* ---------------- INSTRUMENTY / RYNEK ---------------- */
export interface SymbolMeta {
  symbol: string;
  name: string;
  digits: number;
  group: "Metals" | "Forex" | "Crypto" | "Indices" | "Energy";
  base: number;
  /** Zmiennosc dzienna uzywana przez generator swiec (w % ceny). */
  vol: number;
  contractSize: number;
  /** wartosc 1 punktu ceny na 1 lot, w USD */
  pointValue: number;
}

export interface Candle {
  t: number; // unix ms — poczatek swiecy
  o: number;
  h: number;
  l: number;
  c: number;
  v: number;
}

export type Timeframe = "1m" | "5m" | "15m" | "1h" | "4h" | "1d";

export interface Quote {
  timeBasis?: "broker_wall" | "utc" | null;
  timeUtc?: number | null;
  symbol: string;
  bid: number;
  ask: number;
  spread: number;
  time: number;
  changePct: number;
  change: number;
  dayHigh: number;
  dayLow: number;
}

/* ---------------- POZYCJE / ZLECENIA ---------------- */
export type Direction = "BUY" | "SELL";

export type PendingKind = "BUY_LIMIT" | "SELL_LIMIT" | "BUY_STOP" | "SELL_STOP";

/**
 * Skad wziela sie pozycja/zlecenie widoczne na koncie.
 *
 * CONDUIT jest ogolna platforma tradingowa — pokazuje WSZYSTKO, co dzieje sie
 * na rachunku, nie tylko wlasny handel z Telegrama. To pole mowi, czym bot
 * ZARZADZA: tylko `BOT`. Reszta jest w pelni prawidlowa, po prostu nie nasza —
 * i nie wolno jej ruszac.
 */
export type PositionSource = "BOT" | "EXTERNAL" | "MANUAL";

/** Podsumowanie tego, czego bot na rachunku NIE prowadzi. */
export interface ForeignSummary {
  positions: number;
  pendings: number;
  volume: number;
  profit: number;
  magics: number[];
  symbols: string[];
  comments: string[];
}

/** Etykieta i opis zrodla — jedno miejsce, zeby caly panel mowil tak samo. */
export const SOURCE_LABEL: Record<PositionSource, string> = {
  BOT: "BOT",
  EXTERNAL: "ZEWNĘTRZNE",
  MANUAL: "RĘCZNE",
};

export const SOURCE_HINT: Record<PositionSource, string> = {
  BOT: "Prowadzone przez CONDUIT — bot zarządza SL, TP i zamknięciem.",
  EXTERNAL: "Inny automat na tym samym rachunku (inny magic). CONDUIT to widzi, ale NIE zarządza.",
  MANUAL: "Otwarte ręcznie z terminala (magic 0). CONDUIT to widzi, ale NIE zarządza.",
};

/** Czy bot ma prawo tym sterowac (zamknac, przesunac SL/TP). */
export function isManaged(src: PositionSource | undefined): boolean {
  return (src ?? "BOT") === "BOT";
}

export interface Position {
  ticket: number;
  symbol: string;
  direction: Direction;
  volume: number;
  openPrice: number;
  openTime: number;
  sl: number | null;
  tp: number | null;
  /** wirtualny SL trzymany "u bota" (broker go nie widzi) */
  vsl: number | null;
  profit: number;
  swap: number;
  commission: number;
  comment: string;
  /** `null` = wartosc nieznana, NIE zero (zero znaczy w MT5 „reczne z terminala"). */
  magic: number | null;
  basketId: number | null;
  /** indeks poziomu siatki (0 = najlepsze wejscie) */
  level: number;
  /** pozycja "zamrozona" przez reczna edycje — bot jej nie rusza */
  frozen: boolean;
  /** szczyt zysku w punktach — do trailingu / harvestu */
  peakPts: number;
  /** czy pozycja jest runnerem (bez TP, tylko trailing) */
  runner: boolean;
  /** znacznik: pozycja pochodzi z toucher-a (szczyt strefy) */
  toucher: boolean;
  /** czas ostatniego nowego szczytu — do stagnacji */
  lastPeakTime: number;
  /** kto to otworzyl; brak = stary backend, traktujemy jak BOT */
  source?: PositionSource;
}

export interface PendingOrder {
  ticket: number;
  symbol: string;
  kind: PendingKind;
  volume: number;
  price: number;
  sl: number | null;
  tp: number | null;
  placedTime: number;
  comment: string;
  basketId: number | null;
  level: number;
  frozen: boolean;
  source?: PositionSource;
  magic?: number | null;
}

export type CloseReason =
  | "TP"
  | "SL"
  | "VSL"
  | "MANUAL"
  | "PARTIAL"
  | "BASKET"
  | "RISK_FREE"
  | "OAE"
  | "HARVEST"
  | "STALE"
  | "TRAIL"
  | "EOD"
  | "DAY_TARGET"
  | "MAX_DD"
  | "AI";

export type ClosedProfitBasis = "Unknown" | "PriceOnlyGross" | "PricePlusSwap" | "CanonicalClosedNetV1" | "ReportedNet";

export interface ClosedPosition {
  ticket: number;
  symbol: string;
  direction: Direction;
  volume: number;
  openPrice: number;
  closePrice: number;
  openTime: number;
  closeTime: number;
  profit: number;
  /** Missing legacy metadata is unknown, not gross or zero. */
  profitBasis?: ClosedProfitBasis;
  /** Server-validated receipt net; required for CanonicalClosedNetV1. */
  netProfit?: number | null;
  swap: number;
  commission: number;
  reason: CloseReason;
  comment: string;
  basketId: number | null;
  source?: PositionSource;
  magic?: number | null;
}

export type PendingHistoryStatus = "FILLED" | "CANCELLED" | "EXPIRED";

export interface PendingHistoryItem {
  ticket: number;
  symbol: string;
  kind: PendingKind;
  volume: number;
  price: number;
  sl: number | null;
  tp: number | null;
  placedTime: number;
  endTime: number;
  status: PendingHistoryStatus;
  basketId: number | null;
}

/* ---------------- KOSZYKI SYGNALOW ---------------- */
export interface Basket {
  id: number;
  symbol: string;
  direction: Direction;
  isLimit: boolean;
  entryLow: number;
  entryHigh: number;
  zoneLow: number;
  zoneHigh: number;
  sl: number | null;
  tps: number[];
  tpStage: number;
  createdAt: number;
  source: string;
  sourceKey: string;
  active: boolean;
  tickets: number[];
  pendingTickets: number[];
  /** log zdarzen koszyka — widoczny w karcie koszyka */
  events: BasketEvent[];
  riskFree: boolean;
  note?: string;
}

export interface BasketEvent {
  t: number;
  text: string;
  kind: "info" | "entry" | "tp" | "sl" | "mgmt" | "ai";
}

/* ---------------- TELEGRAM ---------------- */
export interface TelegramTopic {
  id: number;
  name: string;
  icon: string;
}

export interface TelegramChannel {
  id: number;
  name: string;
  handle: string;
  members: number;
  avatarHue: number;
  /** Adres PRAWDZIWEGO zdjecia profilowego z Telegrama.
   *  Brak = kanal nie ma zdjecia i zostaje sama literka. */
  photoUrl?: string;
  isForum: boolean;
  verified: boolean;
  topics: TelegramTopic[];
  /** Rodzaj czatu z Telegrama: `channel` / `group` / `user`.
   *  Opcjonalny, bo lista zastepcza (`data/telegram.ts`) go nie niesie. */
  kind?: string;
  /** ostatnia wiadomosc (podglad na liscie) */
  lastMessage: string;
  lastMessageTime: number;
}

/**
 * Powiazanie kanalu z formatem.
 *
 * JEDEN FORMAT NA KANAL, nie lista. Dwa formaty na jednym kanale znaczylyby,
 * ze ta sama wiadomosc rodzi dwa koszyki, z dwoch parserow, zarzadzane dwoma
 * presetami, na jednym rachunku — podwojna ekspozycja z jednego sygnalu.
 * Kanal podaje sygnaly w jeden sposob, bo pisze go jeden zespol.
 *
 * Pusty lancuch znakow = „ten kanal NIE HANDLUJE" (mozna go nasluchiwac
 * i zbierac sygnaly, nie ryzykujac na nieprzebadanym formacie).
 */
export interface ChannelBinding {
  channelId: number;
  monitored: boolean;
  notify: boolean;
  /** JEDEN format kanalu zwyklego. Pusty = kanal nie handluje. */
  format: string;
  /** temat forum -> JEDEN format. Brak klucza albo pusty = temat nie handluje. */
  topics: Record<number, string>;
  /**
   * STARY KSZTALT z serwera (`Vec<String>`). Panel go CZYTA przy normalizacji
   * i nigdy nie uzywa dalej — zostaje w typie wylacznie po to, zeby dane
   * z dzisiejszej binarki dalo sie przyjac bez wyjatku.
   * @deprecated
   */
  formats?: string[];
}

/* ---------------- FORMATY I LANCUCHY (crates/core/src/formaty.rs) ---------------- */

/**
 * Format, pod ktory powstaja presety i z ktorym parowane sa kanaly.
 *
 * Nazwa jest KLUCZEM — po niej lacza sie preset, kanal i lancuch — wiec
 * zmiana nazwy istniejacego formatu zrywa te powiazania. Dodawaj nowe,
 * nie przemianowuj starych.
 */
export interface Format {
  nazwa: string;
  /** silnik parsujacy; dzis `Synergy` celowo uzywa parsera `atfx` */
  parser: string;
  opis: string;
  /** przykladowa wiadomosc — tylko do pokazania w panelu */
  przyklad?: string;
}

/**
 * PULAPY OBOWIAZUJACE PONAD PRESETAMI — wlasnosc lancucha, nie presetu.
 *
 * Preset pilnuje SIEBIE. Dwa presety po 30 pozycji kazdy nie lamia wlasnych
 * limitow, a na rachunku robi sie 60 — bo rachunek jest jeden, a presety
 * o sobie nie wiedza. Dlatego te pola sa SUFITEM, nie zamiennikiem:
 *
 *     limit skuteczny = min(limit presetu, limit globalny)
 *
 * **`0` znaczy BRAK PULAPU**, nie „limit zero". Pomylka w te strone
 * wylaczylaby handel, wiec panel pisze to przy kazdym polu wprost.
 */
export interface PulapyGlobalne {
  /* --- ekspozycja liczona LACZNIE ze wszystkich formatow --- */
  maxPozycji: number;
  maxKoszykow: number;
  maxLotow: number;
  /** wolumen NETTO w jedna strone — chroni przed staniem calym kontem po jednej stronie rynku */
  maxLotowKierunkowo: number;
  maxRyzykoPct: number;

  /* --- obsuniecie CALEGO rachunku, nie pojedynczego presetu --- */
  maxDdPct: number;
  maxDdUsd: number;
  /** twarda podloga equity — bezpiecznik ostatniej instancji, nie narzedzie strojenia */
  podlogaEquityUsd: number;

  /* --- doba, liczac wszystkie formaty razem --- */
  celDniaUsd: number;
  celDniaPct: number;
  /** po osiagnieciu celu: zamknac wszystko czy tylko przestac otwierac */
  celDniaZamyka: boolean;
  limitStratyDniaUsd: number;
  limitStratyDniaPct: number;

  /* --- konflikty miedzy formatami --- */
  blokujPrzeciwneKierunki: boolean;
  pauzaPoStratachN: number;
  pauzaPoStratachMin: number;
}

/** Mapa `format → nazwa presetu` plus pulapy obowiazujace ponad presetami. */
export interface Lancuch {
  nazwa: string;
  opis: string;
  /** klucz = nazwa formatu, wartosc = nazwa presetu; pusto = format nie handluje */
  presety: Record<string, string>;
  pulapy: PulapyGlobalne;
}

/** Zbior lancuchow wraz ze wskazaniem aktywnego. */
export interface Lancuchy {
  aktywny: string;
  lista: Lancuch[];
}

/* ---------------- WIADOMOSCI / SYGNALY ---------------- */
export type SignalType =
  | "ENTRY"
  | "TP_HIT"
  | "SL_HIT"
  | "RISK_FREE"
  | "PARTIAL"
  | "CANCEL"
  | "OUT_AT_ENTRY"
  | "CLOSE_ALL"
  | "TP_CORRECTION"
  | "SET_SL"
  | "INFO"
  /** Rozpoznany przez silnik typ sygnalu, ktorego ten panel jeszcze nie
   *  zna — po dolozeniu wariantu w rdzeniu, przed aktualizacja panelu. */
  | "UNKNOWN";

export interface ParsedSignal {
  type: SignalType;
  direction?: Direction;
  isLimit?: boolean;
  /** zlecenie oczekujace na PRZEBICIE poziomu (STOP), a nie na powrot (LIMIT) */
  isStop?: boolean;
  entryLow?: number;
  entryHigh?: number;
  sl?: number | null;
  tps?: number[];
  tpIndex?: number;
  level?: number;
  raw: string;
}

export interface ChatMessage {
  timeBasis?: "broker_wall" | "utc" | null;
  receivedTimeUtc?: number | null;
  id: string;
  time: number;
  channelId: number;
  channelName: string;
  /** Temat forum. Tozsamoscia zrodla jest PARA (channelId, topicId) — na forum
   *  wszystkie tematy jednej grupy maja ten sam channelId i te sama nazwe. */
  topicId?: number;
  topicName?: string;
  /** Format przypisany zrodlu (ZEN / Synergy / …). Brak = tylko nasluch. */
  format?: string;
  text: string;
  types: SignalType[];
  basketId: number | null;
  replyToId?: string;
  replyText?: string;
  edited: boolean;
  /** MANUAL: sygnal czeka na reczne wykonanie */
  pendingAction?: "await" | "deferred" | "executed" | "dismissed";
  parsed?: ParsedSignal[];
  outgoing?: boolean;
}

/* ---------------- LOGI / ZDARZENIA BOTA ---------------- */
export type LogCategory =
  | "commands"
  | "events"
  | "messages"
  | "signals"
  | "trades"
  | "unpredicted_signals"
  | "signal_formats"
  | "backup_memory"
  | "poll_interval"
  | "update_performance"
  | "price_log"
  | "session_string"
  | "smtp"
  /* --- ZRODLA PLIKOWE (dolozone 04.08.2026) ---
     Do tej pory `alllogs` bral wylacznie migawke procesu, dziennik decyzji
     i log panelu. Wszystko ponizej bot produkuje na dysku, a scalanie tego
     nie widzialo — w tym KRONIKA, zgloszona przez uzytkownika wprost. */
  /** logs/journal/*.jsonl — rozumowanie silnika */
  | "journal"
  /** logs/journal/*.log — lustro tekstowe tego samego (duplikat tresci) */
  | "journal_log"
  /** plik kroniki (domyslnie na pulpicie) — caly strumien z Telegrama */
  | "kronika"
  /** logs/wiadomosci/*.jsonl — archiwum z edycjami i skasowaniami */
  | "wiadomosci"
  /** koszyki.json — zrzut koszykow silnika */
  | "koszyki"
  /** settings.json, channels.json, demo.json, kronika.json */
  | "konfiguracja"
  /** presets/*.json — spis biblioteki presetow */
  | "presety"
  /** mail_queue.json — niewyslane alerty */
  | "mail_queue"
  /** lab/ — wyniki backtestow i treningu (spis) */
  | "lab";

export interface LogEntry {
  id: number;
  t: number;
  category: LogCategory;
  title: string;
  content: string;
  level: "info" | "warn" | "error" | "success";
}

/* ---------------- STATYSTYKI ---------------- */
export interface Stats {
  balance: number;
  equity: number;
  margin: number;
  freeMargin: number;
  marginLevel: number;
  /** KREDYT BONUSOWY raportowany przez terminal (ACCOUNT_CREDIT). */
  credit: number;
  /** Ile kredytu bot faktycznie odlicza od podstawy lota (0 = nie odlicza). */
  creditApplied: number;
  /** Skad wzieta liczba wyzej: "terminal" | "reczny" | "off". */
  creditSource: "terminal" | "reczny" | "off";
  /** Podstawa lota wg Balance/Equity/MinOfBoth i kontraktu oddzielnego kredytu. */
  lotBase: number;
  /** Loty per noga lancucha — kafel LOT AUTO sumuje, dymek rozbija.
      wolumenWykonany = suma wolumenu ZAMKNIETYCH tej nogi w pamieci panelu
      (mianownik "% dynamicznego"). zamrozona = tylko zarzadza, bez sumy.
      handluje = bierze NOWE sygnaly (falsz takze dla silnika bez formatu).
      zPliku = pola handlu z PLIKU presetu (nie z dokumentu panelu).
      pulapLancucha = `max_lotow` lancucha; to BRAMKA WEJSCIA, nie sufit
      wolumenu pojedynczego zlecenia — patrz `ui::LotNogi` po stronie Rusta.

      UWAGA NA `lot` (sprawa z 18.08.2026): to jest wolumen JEDNEGO ZLECENIA,
      nie ekspozycja koszyka. Jeden sygnal stawia `poziomyWejscia` szczebli
      i KAZDY dostaje wlasny lot, a `lotMax` tnie POJEDYNCZE zlecenie — wiec
      koszyk siega `poziomyWejscia` x `lotMax`. Ekspozycje mowi `lotKoszyka`;
      panel ma podpisywac slowami „tyle wejdzie na rachunek" WYLACZNIE ja. */
  lotNogi: {
    format: string;
    preset: string;
    lot: number;
    wolumenWykonany: number;
    zamrozona: boolean;
    handluje: boolean;
    zPliku: boolean;
    lotMax: number;
    /** Planowany wolumen CALEGO koszyka nogi (`Engine::lot_koszyka_planowany`).
        Starszy backend tego pola nie ma — panel spada wtedy na `lot`. */
    lotKoszyka?: number;
    /** Ile szczebli stawia jeden sygnal tej nogi (`entry_units` po bramkach). */
    poziomyWejscia?: number;
    pulapLancucha: number;
    /* MIEJSCE NOGI W DRABINCE (od 05.08.2026).
       stan: "aktywna" | "kolejka" | "nieaktywna".
       powod: kod, nie zdanie — tlumaczy panel ("brakZrodla", "zamrozona",
       "kolejka", "minieta", "brakFormatu"); pusty = noga gra.
       prog: prog BALANCE szczebla; -1 = laniuch spoza drabinki. */
    stan?: string;
    powod?: string;
    lancuch?: string;
    prog?: number;
  }[];
  /** Kwota reczna nie zgadza sie z terminalem — pokazac jako OSTRZEZENIE. */
  creditMismatch: boolean;
  pnlToday: number;
  pnlSession: number;
  sessionStart: number;
  drawdownNow: number;
  maxDdToday: number;
  peakEquityToday: number;
  dayStartEquity: number;
  /** RDD is unavailable when the day-start/minimum were not observed. */
  realDrawdownDay?: {
    day: number | null;
    startEquity: number | null;
    minEquity: number | null;
  } | null;
  messages: number;
  signals: number;
  /** krzywa equity (do sparkline) */
  equityCurve: { t: number; v: number }[];
}

/* ---------------- USTAWIENIA (1:1 z bot.py) ---------------- */
export interface Settings {
  explicit_pending_until_cancel: boolean;
  lot_base: "Balance" | "Equity" | "MinOfBoth";
  /* --- wejscia --- */
  auto_limit: boolean;
  custom_entry: boolean;
  entry_high_offset: number;
  entry_low_offset: number;
  entry_offset_dir: boolean;
  entry_deep_offset: number;
  /** M3: glebokosc siatki jako ulamek dystansu krawedz strefy -> SL (0 = stala kwota). */
  entry_deep_frac_to_sl: number;
  entry_tol_offset: number;
  sl_dist_limit: boolean;
  sl_dist_max: number;
  sl_min_dist: number;
  only_limit_signals: boolean;
  valid_till_tp2: boolean;
  ignore_old_after_min: number;

  /* --- jednostki i siatka --- */
  entry_units: number;
  entry_units_limit: number;
  entry_risk_budget: number;
  entry_tp1_budget: number;
  entry_touch_units: number;
  entry_touch_tp: number;
  entry_touch_levels: string;
  ppm_enabled: boolean;
  ppm: number;
  ppm_immediate: boolean;
  ppm_for_limits: boolean;
  grid_fallback_best_edge: boolean;
  pending_resize_on_vol: boolean;
  pending_relot_on_balance: boolean;
  pending_relot_topup: boolean;
  pending_relot_up: boolean;
  pending_relot_down: boolean;
  pending_relot_wg_planu: boolean;
  pending_relot_reconcile_target: boolean;
  pending_relot_up_od_salda: number;
  pending_resize_sec: number;
  pending_ttl_h: number;
  pending_never_cancel: boolean;

  /* --- zarzadzanie TP --- */
  all_runners: boolean;
  scale_out: boolean;
  scale_out_pct: number;
  scale_out_round: "up" | "down";
  scale_out_from: "worst" | "best";
  scale_out_last_runner: "runner" | "next_tp" | "no_tp";
  tp_open_offset: number;
  tp_detect_price: boolean;
  tp_detect_signal: boolean;
  tp_freeze_after_ladder: boolean;
  tp_hit_fill_stages: boolean;
  spp_max_age_h: number;

  /* --- trailing --- */
  smart_sl: boolean;
  breakeven_protection: boolean;
  trail_after_tp2: boolean;
  runner_trail: boolean;
  runner_trail_start: number;
  runner_trail_gap: number;
  trail_mode: "gap" | "lock_pct" | "tiered" | "atr" | "chandelier";
  trail_lock_pct: number;
  trail_tiers: string;
  trail_split: boolean;
  trail_runners_n: number;
  trail_runner_mode: "gap" | "lock_pct" | "tiered" | "atr" | "chandelier";
  trail_runner_start: number;
  trail_runner_lock_pct: number;
  trail_runner_gap: number;
  trail_runner_tiers: string;
  /** Causal market-quality adaptation for Gap/ATR/Chandelier trailing. */
  trail_adaptive_enabled: boolean;
  trail_adaptive_runners_only: boolean;
  trail_adaptive_window_s: number;
  trail_adaptive_min_samples: number;
  trail_adaptive_trend_er: number;
  trail_adaptive_reversal_er: number;
  trail_adaptive_trend_gap_mult: number;
  trail_adaptive_chop_gap_mult: number;
  trail_adaptive_reversal_gap_mult: number;
  trail_adaptive_fast_vol_s: number;
  trail_adaptive_slow_vol_s: number;
  trail_adaptive_vol_ratio: number;
  trail_adaptive_vol_favorable_mult: number;
  trail_adaptive_vol_adverse_mult: number;
  trail_adaptive_min_peak: number;
  trail_adaptive_min_gap: number;
  trail_adaptive_max_gap: number;
  /* trailing S/R po strukturze 1M (OS_SR_SPEC.md) — wartości w konwencji
     rdzenia (bez tłumaczenia w preset_to_ui) */
  trail_sr_enabled: boolean;
  sr_warmup_exact_ticks: boolean;
  trail_sr_scope: "Runner" | "Tp3Up" | "All";
  trail_sr_activation: "Entry" | "Gain" | "Tp1" | "Tp2" | "Tp3";
  trail_sr_min_gain: number;
  trail_sr_min_dist_price: number;
  /* EA-CORE — szkielet warstwy EA (FALA 0, wiedza/EA_PLAN_WDROZENIA.md).
     Kontrakt zera jest POTRÓJNY: (a) ea_enabled=false — silnik nie wchodzi
     do warstwy w ogóle i nie czyta rachunku ani razu więcej; (b) włączona
     z samymi zerami — stan Neutral na zawsze, modulatory 1,0; (c) tryb
     AUTO-EA bez osi — co do centa jak AUTO. Wartości enumów w konwencji
     rdzenia (bez tłumaczenia w preset_to_ui). */
  ea_enabled: boolean;
  ea_tick_s: number;
  ea_state_src: "FloatR" | "FloatPctEquity";
  ea_defense_enter: number;
  ea_defense_exit: number;
  ea_offense_enter: number;
  ea_offense_exit: number;
  ea_state_dwell_s: number;
  ea_state_ratchet: "NieLuzujWKoszyku" | "Swobodny";
  ea_state_journal: boolean;
  ea_dozor_sl: boolean;
  harvest: boolean;
  harvest_start: number;
  harvest_retrace_pct: number;
  trail_min_dist: number;
  ladder_from_tp: number;
  ladder_lag: number;
  ladder_offset: number;

  /* --- wirtualny SL --- */
  virtual_sl: boolean;
  virtual_sl_only_when_rejected: boolean;
  vsl_eval_s: number;
  virtual_sl_all: boolean;
  vsl_net_off: number;

  /* --- filozofia ATFX --- */
  be_lock: boolean;
  be_lock_points: number;
  be_at_tp1: boolean;
  reenter_after_tp: boolean;
  reenter_min_tp_stage: number;
  oae_timeout_min: number;
  oae_profit_min: number;
  ignore_out_at_entry: boolean;
  ignore_risk_free: boolean;

  /* --- oficjalny system --- */
  official_mode: boolean;
  official_pct_tp1: number;
  official_pct_tp2: number;
  official_pct_tp3: number;
  official_pct_spp: number;
  official_use_counts: boolean;
  official_counts: string;
  official_spp: boolean;
  official_round: "nearest" | "up" | "down";
  official_assign_tps: boolean;
  official_close_last: boolean;
  spp_keep_tp: boolean;
  partial_close: boolean;
  partial_min_lot: number;
  /** procenty inkasa liczone od wolumenu POCZATKOWEGO, nie od tego, co zostalo */
  partial_pct_od_pierwotnego: boolean;
  /** kazda pozycja celuje w najdalszy cel, niezaleznie od harmonogramu */
  cele_na_ostatnim: boolean;
  retarget_respects_final_target: boolean;
  /** SL w polowie drogi wejscie->cena, od N-tego celu OD KONCA (0 = off) */
  sl_polowa_od_konca: number;
  /** jaki ulamek drogi bierze regula sl_polowa_od_konca (0,5 = polowa) */
  sl_polowa_ulamek: number;

  /* --- risk free --- */
  risk_free_runners: number;
  risk_free_mode: "scale_out" | "all_runners";
  risk_free_smart_sl: boolean;
  sl_hit_verify_tol: number;

  /* --- stagnacja / wyjscia --- */
  stale_take_min: number;
  stale_take_profit: number;
  stale_take_min2: number;
  stale_take_profit2: number;
  rev_exit_range: number;
  rev_exit_slope: number;
  rev_exit_profit: number;

  /* --- reguly doswiadczonego tradera --- */
  exit_min_hold_min: number;
  exit_min_profit: number;
  exit_r_multiple: number;
  basket_target_usd: number;
  exit_round_dist: number;
  exit_round_step: number;
  exit_spread_mult: number;
  exit_on_opposite_signal: boolean;
  hold_after_tp_hit_min: number;
  toucher_tp_one_based: boolean;
  pending_drop_arm: boolean;
  /** Poziom marginesu liczony razem z zamrozonym marginesem zlecen oczekujacych, a nie tylko z otwartych pozycji. */
  ml_licz_wiszace: boolean;
  /** Ponizej tego poziomu marginesu bot nie otwiera nowego koszyka. */
  ml_min_wejscie: number;
  /** Ponizej tego poziomu bot nie doklada kolejnych szczebli siatki do koszyka, ktory juz istnieje. */
  ml_min_warstwa: number;
  /** Ponizej tego poziomu bot nie wchodzi ponownie po trafionym celu. */
  ml_min_reentry: number;
  /** Ponizej tego poziomu bot nie wystawia ponownie niewypelnionych szczebli. */
  ml_min_rearm: number;
  /** Ponizej tego poziomu bot nie doklada do pozycji, ktora jest na plusie. */
  ml_min_piramida: number;
  /** Ponizej tego poziomu bot nie robi szybkiej dokladki po gwaltownym wypelnieniu. */
  ml_min_fast_addon: number;
  /** Ponizej tego poziomu bot nie podnosi wolumenu istniejacych zlecen. */
  ml_min_relot_up: number;
  /** Ponizej tego poziomu bot nie stawia kolejnego szczebla drabinki wejscia rynkowego. */
  ml_min_drabina: number;
  /** Dzwignia uzyta do liczenia marginesu. */
  konto_dzwignia: number;
  /** Zegar wieku koszyka rusza od pierwszego wypelnienia, a nie od powstania siatki. */
  wiek_od_wypelnienia: boolean;
  /** Po zdarzeniu cel osiagniety bez wejscia siatka nie znika od razu, tylko czeka tyle minut. */
  pending_drop_grace_min: number;
  /** Okno laski obowiazuje tylko, dopoki cena jest blizej niz tyle dolarow od strefy; dalej siatka ginie od razu. */
  pending_drop_grace_max_dist: number;
  /** Przy kasowaniu siatki zostaw tyle zlecen najblizszych cenie. */
  pending_drop_keep_n: number;
  grid_anchor_absolute: boolean;
  /** G1: czy krata bezwzgledna mnozy zlecenia na poziomie (true = jak dotad). */
  units_per_level: boolean;
  tp_open_extra: boolean;
  /** Kasuj niewypelnione limity po osiagnieciu celu koszyka. */
  pending_drop_on_target: boolean;
  /** Po RISK FREE madry SL nie schodzi ponizej progu oplacalnosci. */
  smart_sl_floor_be_after_rf: boolean;

  /* --- madre wyjscie (smart exit) --- */
  smart_exit: boolean;
  smart_exit_take: number;
  smart_exit_giveback: number;
  smart_exit_min_peak: number;
  smart_exit_drop_speed: number;
  smart_exit_speed_window_s: number;
  smart_exit_hold_if_pending: number;
  smart_exit_min_pendings: number;
  smart_exit_pending_scope: "SameBasket" | "AnyBasket";
  smart_exit_pending_min_dist: number;

  /* --- rezim zmiennosci --- */
  vol_window_min: number;
  vol_range_usd: number;
  vol_units_mult: number;

  /* --- ochrona kapitalu / bramki --- */
  max_dd_pct: number;
  max_dd_usd: number;
  /** Hamulec miękki (dławik) — zmniejsza NOWY koszyk, zamiast zamykać wszystko.
   *  Pola trafiły do `defaultSettings` i do schematu panelu bez wpisu tutaj,
   *  przez co `npm run build` przestał się kompilować. Wszystkie domyślnie
   *  neutralne: przy `0` żaden istniejący preset nie zmienia zachowania. */
  max_portfolio_risk_pct: number;
  dd_soft_pct: number;
  dd_soft_mult: number;
  dd_hard_pct: number;
  dd_hard_mult: number;
  /** Prog OSTRZEZENIA mailem (bez zatrzymywania handlu). Czyta go live.rs. */
  alert_dd_pct: number;
  /** Maks. wiek sygnalu OTWIERAJACEGO koszyk, w minutach. 0 = bez bramki. */
  signal_max_age_min: number;
  max_open_positions: number;
  exposure_count_pendings: boolean;
  lot_scale_step: number;
  day_target_usd: number;
  day_target_close: boolean;
  day_target_scale_lot: boolean;
  day_trail_stop_usd: number;
  usd_scale_with_lot: boolean;
  session_filter: boolean;
  session_hours: string;
  streak_pause_n: number;
  streak_pause_min: number;
  /* --- hamulec SL-HIT (Pakiet F2): stopy KANAŁU, nie nasze --- */
  slhit_pause_n: number;
  slhit_pause_min: number;
  slhit_pause_lot_mult: number;
  eod_flat_hour: number;
  flat_weekend: boolean;
  flat_weekend_hour: number;
  day_flat_broker_clock: boolean;
  signal_filter: boolean;
  skip_tags: string;
  require_tags: string;

  /* --- wagi glebokosci i limit ryzyka koszyka --- */
  entry_weights: string;
  risk_per_basket_pct: number;
  lot_min: number;
  lot_max: number;

  /* --- kredyt bonusowy (podstawa wielkosci pozycji) ---
     MT5: wplata 300 $ + kredyt 300 $ daje balance 300 $, equity 600 $ bez pozycji.
     Lot ma sie liczyc od 300 $; drugie 300 $ jest PODUSZKA marginesowa. */
  odlicz_kredyt: boolean;
  /** MT5 ACCOUNT_BALANCE nie zawiera ACCOUNT_CREDIT; OFF = historyczny model. */
  credit_balance_separate: boolean;
  /** 0 = AUTOMAT (bierz ACCOUNT_CREDIT z terminala). Dodatnia nadpisuje. */
  kredyt_reczny: number;

  /* --- bramki wejscia --- */
  skip_if_sl_breached: boolean;
  max_chase_beyond_zone: number;
  sl_max_dist: number;
  side_filter: "both" | "buy" | "sell";
  regime_filter: "off" | "trend" | "counter";
  regime_ma_hours: number;
  max_open_baskets: number;
  max_directional_lots: number;
  equity_floor_pct: number;
  dd_guard_scope: "daily" | "lifetime" | "lifetime_daily_reset";

  /* --- siatka --- */
  market_entry_step: number;
  pending_ttl_from_basket: boolean;

  /* --- zrodlo wiedzy o trafionym celu --- */
  tp_source: "Either" | "PriceOnly" | "SignalOnly" | "SignalConfirmedByPrice" | "PriceFirstSignalWindow";
  tp_price_tolerance: number;
  /** Autonomiczny soft-TP z Bid/Ask MT5; 0 = pełne dotknięcie (legacy). */
  tp_price_front_run_usd: number;
  tp_signal_max_lead_s: number;
  tp_signal_max_lag_s: number;
  tp_stage_from_broker_fill: boolean;

  /* --- reakcje na komunikaty --- */
  out_at_entry_mode: "close_all" | "losers" | "flat" | "be";
  oae_band_pts: number;
  sl_hit_mode: "cancel_pendings" | "close_all" | "verify" | "ignore";
  honor_cancel: boolean;
  honor_close_all: boolean;
  /** W33: zasieg komendy CLOSE ALL. Global = zachowanie sprzed 24.08.2026. */
  close_all_scope: "Global" | "Basket";
  /** W31b: czy wykonywac komende "Take partials" z kanalu (dzis tylko Info). */
  partials_wykonuj: boolean;
  /** W31b: % wolumenu inkasowanego po tej komendzie (0 = nic). */
  partials_pct: number;
  honor_market_open: boolean;
  dedup_edited_signals: boolean;
  /* --- Pakiet A: osie dedupu i edycji --- */
  dedup_pelny_status: boolean;
  edycja_wykonuje_reszte_akcji: boolean;
  dedup_klucz_z_wartoscia: boolean;
  edycja_sieroty_nie_otwiera: boolean;
  entry_idempotencja: boolean;
  dedup_management_po_restarcie: boolean;
  /** AT TPn = proximity/telemetry; nie wykonuje TpHit. */
  profit_update_telemetry_only: boolean;
  tp_price_only_strict: boolean;
  /* --- Pakiet B: osie z audytu TYLER --- */
  rf_wymaga_wykonania: boolean;
  market_entry_units: number;
  market_hybrid_now_units: number;
  market_hybrid_pending_units: number;
  market_hybrid_lot_mult: number;
  market_hybrid_max_chase_usd: number;
  market_hybrid_tp_stage: number;
  market_unfilled_cancel_stage: number;
  pending_cancel_on_riskfree: boolean;
  bank_all_at_stage: number;
  /* --- Pakiet E: statystyki (oś POMIARU, nie handlu) --- */
  stat_be_prog_usd: number;
  /* --- Pakiet F: błędy złapane na żywym bocie --- */
  reply_veto: boolean;
  risk_free_runner_target: "last" | "keep" | "next" | "none";
  risk_free_trail: boolean;
  /** M4: minimalny zysk pozycji ($), przy ktorym RISK FREE wolno ruszyc stopem na BE (0 = OFF). */
  risk_free_be_min_profit: number;

  /* --- stopy --- */
  be_offset: number;
  /** W31a: czy komenda "set BE" kryje pozycje wypelnione PO niej i re-entry. */
  be_covers_late_fills: boolean;
  be_never_loosen: boolean;
  sltp_retry_s: number;

  /* --- wyjscia --- */
  rev_exit_window_min: number;
  reenter_max: number;

  /* --- model wykonania --- */
  commission_per_lot: number;
  exec_latency_ms: number;
  slippage_pts: number;
  server_tz_offset_h: number;
  msg_clock_offset_h: number | null;

  /* --- audyt / parytet --- */
  sim_clock_strict: boolean;
  spp_refresh_after_modify: boolean;
  sim_stops_level: number;

  /* --- AI --- */
  ai_mode: boolean;
  ai_model: string;
  ai_decision_interval_s: number;

  /* --- nadzor nad terminalem MT5 --- */
  mt5_autostart: boolean;
  mt5_watchdog: boolean;
  /** Global broker receipt ownership contract; experimental and OFF by default. */
  close_receipt_reconcile: boolean;
  /** Experimental account-wide closed-net convention; live requires separate qualification. */
  closed_profit_net_costs: boolean;
  /** Experimental account/runtime continuation; never a strategy search axis. */
  restore_strategy_continuation: boolean;
  order_volume_contract_v2: boolean;
  mt5_terminal_path: string;
  mt5_retry_attempts: number;
  mt5_retry_delay_s: number;
  mt5_restart_after: number;
  mt5_health_interval_s: number;
  mt5_symbol: string;
  /** Attach to the currently selected terminal account without logging it in. */
  mt5_follow_terminal_account: boolean;
  /** Explicit permission for real-account trading in follow-terminal mode. */
  mt5_allow_real_account: boolean;
  mt5_magic: number;
  mt5_python: string;
  mt5_deviation_points: number;
  /* Tozsamosc rachunku: bot ODMAWIA handlu, gdy terminal jest na innym
     koncie niz mt5_login (0 = brak weryfikacji, ZOLTY stan wskaznika).
     04.08.2026: bot cala noc gral na Vantage, gdy uzytkownik myslal,
     ze na PUPrime — zielona kropka bez tozsamosci to polowa informacji. */
  mt5_login: number;
  mt5_server: string;
  /* SKRZYNKA PODAWCZA hasla rachunku: serwer przenosi wartosc do
     secrets.json i USUWA klucz z dokumentu — tu nigdy nic nie zalega. */
  mt5_password: string;
  /* Przerwa DOBOWA notowan (godziny czasu SERWERA, ulamkowo; od==do
     wylacza). Zloto: 00:00-01:00 + zapas. Parametr LACZNOSCI: watchdog
     ciszy nie przebudowuje mostu, gdy rynek po prostu spi. */
  przerwa_dobowa_od_h: number;
  przerwa_dobowa_do_h: number;
  /* Katalog docelowy scalania alllogs (sciezka SERWERA; puste = katalog
     logs bota). Wybierany modalem /api/fs/dirs, walidowany plikiem-sonda
     przy zapisie. */
  alllogs_dir: string;
  /* PULS: mail "zyje" co N godzin (0 = wylaczony). Cisza bez pulsu
     znaczy smierc bota, nie spokojna noc. */
  puls_h: number;

  /* --- ogolne / panel --- */
  one_click: boolean;
  display_currency: string;
  allow_mt5_modify: boolean;
  poll_ms: number;
  price_tol: number;
  show_positions_on_chart: boolean;
  show_potential_tpsl: boolean;
  exclude_pending_potential: boolean;
  comment_mode: "source" | "custom";
  comment_custom: string;
  comment_include_topic: boolean;
  price_log: boolean;
  price_log_interval_s: number;

  /* --- logi --- */
  merge_config: Record<LogCategory, boolean>;
  merge_chronological: boolean;

  /* --- dziennik zdarzen (JSON Lines) ---
     Strumien maszynowo czytelny obok logu tekstowego. Odpowiada na braki,
     przez ktore z logu bot.py nie dalo sie odczytac ani dziennego wyniku,
     ani tego, ktora decyzja kosztowala pieniadze. */
  journal_enabled: boolean;
  journal_min_level: "debug" | "info" | "ok" | "warn" | "error";
  journal_snapshots: boolean;
  journal_excursions: boolean;
  journal_text_mirror: boolean;
  journal_retention_days: number;
  journal_buffer_cap: number;
  /** Ile dob trzymac wlasne archiwum wiadomosci (logs/wiadomosci/*.jsonl). */
  archive_retention_days: number;

  /* ===== RISK FREE JAKO REGULA (RDZEN, 29.07) =====
     Automat, nie reakcja na komunikat z kanalu. Wszystko domyslnie
     neutralne, wiec wlaczenie jest swiadoma decyzja. */
  riskfree_enabled: boolean;
  riskfree_trigger_usd: number;
  riskfree_trigger_r: number;
  riskfree_keep_units: number;
  riskfree_be_offset: number;
  riskfree_runner_target: "KeepTp" | "LastTp" | "NoTpTrailOnly" | "NextTp";
  riskfree_runner_stop: "Be" | "BeOwn" | "TrailGap" | "Off";
  riskfree_runner_gap: number;
  riskfree_runner_max_hold_min: number;
  /** M15: czy limit trzymania runnera dziala TAKZE przy `riskfree_enabled = false`. */
  runner_max_hold_bez_reguly: boolean;
  /** Twardy czas zycia KAZDEGO koszyka w minutach (0 = wylaczone). */
  basket_max_age_min: number;

  /* ===== MODEL KOSZTOW BROKERA (Priorytet 0) =====
     Nie strategia, tylko model rzeczywistosci. Jako jedyne przychodza
     domyslnie WLACZONE: brak swapu nie jest ustawieniem neutralnym,
     tylko bledem modelu. */
  /** AI ZAMIAST regul (true) czy ROWNOLEGLE z nimi (false). */
  ai_replaces_management: boolean;
  swap_enabled: boolean;
  swap_long_points: number;
  swap_short_points: number;
  swap_point_value: number;
  swap_rollover_weekday: number;
  swap_rollover_mult: number;
  /* --- Pakiet D1/D1b: noce weekendowe i doba rolowania z serwera --- */
  swap_pomijaj_weekend: boolean;
  swap_rollover_z_serwera: boolean;
  swap_rollover3days_mt5: number;
  /* --- Pakiet D5/D6/D4 + D3: wierność pętli backtestu --- */
  runner_ksiegowanie_v2: boolean;
  msg_kurs_sprzed_luki: boolean;
  slippage_pending_pts: number;
  stop_out_level_pct: number;
  margin_call_level_pct: number;
  entry_depth_curve: number;
  /** W30: ile $ PRZED strefa wolno wejsc (0 = wylaczone). */
  entry_allowance_usd: number;
  /** W30: ile jednostek na warstwie allowance (0 = warstwy nie ma). */
  entry_allowance_units: number;

  /* ===== filtr trendu wyzszego rzedu ===== */
  trend_filter_enabled: boolean;
  trend_filter_window_h: number;
  trend_filter_drop_pct: number;
  trend_filter_mode: "Block" | "Shrink";
  trend_filter_shrink: number;

  /* ===== poprawki istniejacych regul (domyslnie stare zachowanie) ===== */
  pending_drop_require_zone_touch: boolean;
  trail_runners_by_depth: boolean;

  /* ===== wielkosc pozycji wg jakosci szczebla ===== */
  entry_weights_from_rr: boolean;
  entry_weights_rr_power: number;
  entry_weights_rr_cap: number;

  /* ===== parametry liczone z SYGNALU (dzialaja tylko przy adaptive_params) ===== */
  adaptive_params: boolean;
  sl_min_dist_zone_mult: number;
  sl_min_dist_atr_mult: number;
  sl_min_dist_floor: number;
  sl_min_dist_cap: number;
  entry_deep_zone_mult: number;
  entry_units_zone_ref: number;
  adaptive_atr_window_min: number;
  units_by_hour: string;

  /* ===== skalowanie po zdarzeniu ===== */
  rearm_grid_on_return: boolean;
  basket_realized_broker_only: boolean;
  confirmed_exit_retry: boolean;
  defer_entry_until_receipts: boolean;
  entry_edit_geometry_v2: boolean;
  deferred_entry_max_age_s: number;
  rearm_keep_empty_alive: boolean;
  rearm_block_after_secured: boolean;
  spp_blocks_rearm_when_flat: boolean;
  rearm_min_basket_profit: number;
  rearm_max_times: number;
  rearm_min_gap_min: number;

  /* ===== konto: cel i stop dnia w procentach ===== */
  day_target_pct: number;
  day_trail_stop_pct: number;
  day_trail_arm_pct: number;
  day_trail_basis: "equity_peak" | "profit_peak";
  profit_budget_arm_pct: number;
  profit_budget_keep_pct: number;
  profit_budget_deploy_pct: number;

  /* ===== budzet transakcji i jakosc sygnalu ===== */
  daily_signal_budget: number;
  signal_min_rr: number;
  signal_min_zone_width: number;
  signal_max_zone_width: number;

  /* ===== laczenie koszykow ===== */
  merge_same_side: boolean;
  merge_window_min: number;
  merge_min_overlap: number;

  /* ===== wyjscie limitem ===== */
  exit_via_limit: boolean;
  exit_limit_offset: number;
  exit_limit_wait_s: number;
  exit_limit_min_profit: number;

  /* Canonical engine axes exposed by the full UI coverage audit. */
  basket_max_age_min_small: number;
  basket_max_age_min_small_mult: number;
  be_min_pozycji: number;
  be_od_etapu: number;
  cel_z_przeciwnego: "Off" | "DalszaKrawedz" | "Srodek";
  cel_z_przeciwnego_zapas: number;
  cele_pomin_za_cena: boolean;
  day_gate_do_salda: number;
  day_gate_od_salda: number;
  drop_unplaceable_levels: boolean;
  ea_lot_z_wolnego_marginesu: number;
  ea_redukcja_przy_zageszczeniu: number;
  ea_stan_dnia: "Off" | "TylkoInkaso";
  ea_stan_dnia_jednostki_mult: number;
  ea_stan_dnia_prog_sl: number;
  ea_stop_dokladek_powrot: number;
  ea_stop_dokladek_przy_stracie: number;
  ea_zageszczenie_podloga: number;
  enforce_position_limit_on_fill: boolean;
  entry_jeden_na_glebokiej: boolean;
  entry_krzywa_kotwica: string;
  entry_uklad: string;
  entry_uklad_kotwica: string;
  entry_units_small: number;
  entry_units_small_mult: number;
  entry_warstwy_offset: number;
  entry_warstwy_z_tekstu: boolean;
  expo_cap_close: boolean;
  expo_cap_ml_pct: number;
  expo_cap_pct: number;
  expo_cap_s: number;
  exposure_bonus_baskets: number;
  exposure_bonus_positions: number;
  exposure_bonus_profit_pct: number;
  fast_addon_cooldown_s: number;
  fast_addon_lot_mult: number;
  fast_addon_max: number;
  fast_addon_min_stage: number;
  fast_addon_move_usd: number;
  fast_addon_window_s: number;
  fast_fill_layers: number;
  fast_fill_reject_s: number;
  fast_fill_soft_age_min: number;
  fast_fill_soft_age_min_small: number;
  fast_fill_soft_age_min_small_mult: number;
  hint_veto: boolean;
  honor_stop_orders: boolean;
  limit_kasuje_tylko_nadmiar: boolean;
  live_tick_order_strict: boolean;
  lot_max_z_salda: number;
  market_entry_mode: "GridAtOnce" | "Single" | "Laddered";
  market_entry_step_small: number;
  market_entry_step_small_mult: number;
  max_open_baskets_small: number;
  max_open_baskets_small_mult: number;
  max_open_positions_small: number;
  max_open_positions_small_mult: number;
  no_reenter_from_stage: number;
  no_tp_after_stage: number;
  oae_pod_woda: "NicNieRob" | "Zamknij" | "DociagnijStop";
  oae_skip_after_riskfree: boolean;
  parser_geometryczny: boolean;
  parser_luz_interpunkcyjny: boolean;
  parser_min_pewnosc: number;
  pending_cross_policy: "Market" | "Stop" | "Shift" | "Skip";
  pyramid_after_stage: number;
  pyramid_lot_mult: number;
  pyramid_min_equity_mult: number;
  pyramid_regime_lookback: number;
  pyramid_regime_max_fast_pct: number;
  rearm_bez_pozycji: boolean;
  rearm_bez_pozycji_max_h: number;
  recap_guard: boolean;
  reenter_max_small: number;
  reenter_max_small_mult: number;
  reenter_min_return_s: number;
  reenter_respect_cap: boolean;
  reenter_stop_after_riskfree: boolean;
  regime_cena: "Rynkowa" | "Wejscia" | "Obie";
  regime_gdy_rozerwany: "Milcz" | "KrotkieOkno" | "Miekko";
  regime_miara: "Srednia" | "Mediana" | "Kanal" | "Wykladnicza" | "Percentyl";
  regime_okno2_h: number;
  regime_percentyl: number;
  regime_pilnuj_limitow: boolean;
  regime_soft: boolean;
  regime_soft_lot_mult: number;
  regime_soft_max_positions: number;
  regime_soft_risk_mult: number;
  regime_soft_units_mult: number;
  regime_strefa_martwa: number;
  regime_zmiennosc_max: number;
  regime_zmiennosc_min: number;
  reply_graph_transitive: boolean;
  rf_level_sanity_max_usd: number;
  risk_per_basket_pct_small: number;
  risk_per_basket_pct_small_mult: number;
  runner_cele_krok: number;
  runner_cele_n: number;
  runner_max_hold_rule_only: boolean;
  runner_partial_pct: number;
  sanity_tp_max: number;
  sanity_tp_rosnace: boolean;
  sanity_tp_strona: boolean;
  sanity_zone_max: number;
  sesja_bramka: "Sygnal" | "Wypelnienie" | "Oba";
  sim_margin_at_market: boolean;
  sim_margin_check_on_fill: boolean;
  sim_validate_pending_stops: boolean;
  sl_edit_reaches_pendings: boolean;
  sl_min_dist_small: number;
  sl_min_dist_small_mult: number;
  sl_po_tp1_na_krawedz: boolean;
  sl_wlasny_na_pozycje: number;
  spp_arms_runner_clock: boolean;
  spp_sl_mode: "Off" | "Stop" | "OnlyIfBetter" | "RunnersOnly" | "RunnersOnlyIfBetter" | "BankersOnly";
  spp_sl_pad: number;
  sync_only_live_levels: boolean;
  tp_correction_to_broker: boolean;
  tp_drabinka_kotwica: string;
  tp_hit_match_level: boolean;
  tp_unindexed_pips_require_price: boolean;
  trail_atr_mult: number;
  trail_sr_atr_period: number;
  trail_sr_fractal_n: number;
  trail_sr_min_dist_tp: number;
  trail_sr_min_prominence_atr: number;
  trail_sr_offset: number;
  trail_sr_offset_atr_mult: number;
  trail_sr_offset_spread_mult: number;
  trail_sr_struct_window_h: number;
  trail_sr_tf_min: number;
  units_per_level_zone: boolean;
  vol_size_max_mult: number;
  vol_size_min_mult: number;
  vol_size_mode: "Off" | "Target" | "Percentile";
  vol_size_odsezonuj: boolean;
  vol_size_percentile_okno: number;
  vol_size_target: number;
  zakaz_ponizej_krawedzi: boolean;
  zone_exit_adverse_close: boolean;
  zone_exit_adverse_s: number;
}

export type SettingKey = keyof Settings;

/* ---------------- LOT ---------------- */
export interface LotConfig {
  mode: "fixed" | "percent";
  fixed: number;
  percent: number;
}

/* ---------------- PRESETY ---------------- */
/** Cztery tryby oceny (NAUKOWIEC.md §4). Preset musi dzialac we WSZYSTKICH. */
export type TrybOceny = "dzienny" | "dziennyComp" | "dlugi" | "dlugiComp";

export const TRYB_LABEL: Record<TrybOceny, string> = {
  dzienny: "dzien po dniu",
  dziennyComp: "dzien po dniu + compounding",
  dlugi: "dlugoterminowo",
  dlugiComp: "dlugoterminowo + compounding",
};

export const TRYB_HINT: Record<TrybOceny, string> = {
  dzienny: "Konto wraca do 200 $ co dobe, lot staly. SLEPY NA SPIRALE SMIERCI — ruina potrzebuje kilku dni pod rzad, a ten tryb resetuje konto co noc.",
  dziennyComp: "Konto wraca do 200 $ co dobe, ale lot rosnie w obrebie doby.",
  dlugi: "Konto rosnie bez resetu, lot staly.",
  dlugiComp: "Konto i lot rosna razem. Tryb, w ktorym SWEEP-A-5 zerowal konto, wygladajac najlepiej we wszystkich pozostalych.",
};

/** Zestaw liczb wymagany przy KAZDYM z czterech trybow. */
export interface WynikTrybu {
  zysk: number;
  /** udzial dni stratnych w procentach */
  dniStratnePct: number;
  najgorszyDzien: number;
  /** najnizsze equity w calym przebiegu — to jest miara bliskosci ruiny */
  najnizszeEquity: number;
  /** ile razy konto doszlo do zera albo ponizej progu odrobienia */
  wyzerowania: number;
  /** RAPORTOWANY INFORMACYJNIE. Nie rangujemy po nim i nie odrzucamy po nim. */
  maxDd?: number;
}

export interface Preset {
  id: string;
  name: string;
  tagline: string;
  badge?: string;
  family: string;
  /**
   * FORMAT SYGNALOW, DLA KTOREGO TEN PRESET POWSTAL.
   *
   * Brak pola = `"ATFX"` — dokladnie jak `#[serde(default)]` po stronie Rusta.
   * Wszystkie presety sprzed 03.08.2026 powstaly pod ATFX i tylko pod niego
   * byly mierzone, wiec to jest wlasciwa domyslnosc, a nie wygodny skrot.
   * Czytaj przez `formatPresetu()` z `data/presets.ts`, nie wprost.
   */
  format?: string;
  /** metryki z backtestu (do karty presetu) */
  metrics: {
    monthly: number;
    winDays: number;
    maxDd: number;
    profitFactor: number;
    worstDay: number;
    /** Ocena czterotrybowa. Brak trybu = NIE ZMIERZONO — panel ma to napisac
     *  wprost, a nie podstawiac zera. */
    tryby?: Partial<Record<TrybOceny, WynikTrybu>>;
  };
  risk: "low" | "medium" | "high" | "extreme";
  best?: boolean;
  values: Partial<Settings> & Partial<{ lot_mode: string; lot_percent: number; lot_fixed: number }>;
}

/* ---------------- MODELE AI ---------------- */
export interface AiModel {
  id: string;
  name: string;
  version: string;
  description: string;
  params: string;
  trainedOn: string;
  cadence: string;
  metrics: { monthly: number; winDays: number; maxDd: number; profitFactor: number };
  recommended?: boolean;
}

/* ---------------- SYMULACJE ---------------- */
export interface SimInstance {
  id: string;
  name: string;
  preset: string;
  /** WŁASNY tryb handlu instancji (AUTO / AUTO-EA / AI). Brak pola =
   *  dziedziczenie trybu głównego bota — kontrakt zera: każda instancja
   *  sprzed 24.08 zachowuje się jak dotąd. MANUAL w symulacji nie istnieje
   *  (nie ma komu klikać „Wykonaj"), serwer go odrzuca. */
  mode?: TradingMode;
  balance: number;
  startBalance: number;
  equity: number;
  lot: number;
  positions: number;
  pendings: number;
  baskets: number;
  trades: number;
  winRate: number;
  maxDd: number;
  createdAt: number;
  curve: number[];
}

/* ---------------- E-MAIL ---------------- */

/** Kategorie zdarzen z osobnymi przelacznikami. */
export interface MailCategories {
  lifecycle: boolean;
  mt5Connection: boolean;
  mt5RecoveryFailed: boolean;
  drawdown: boolean;
  orderError: boolean;
  summary: boolean;
  /** Wiadomosc ze zrodla, KTORE HANDLUJE, ma ksztalt sygnalu (cel + stop),
   *  a parser nie widzi w niej ani wejscia, ani polecenia. Domyslnie WLACZONE. */
  signalUnreadable: boolean;
}

/** Dlawienie: okno scalania powtorek + sufit maili na godzine. */
export interface MailThrottle {
  windowMin: number;
  maxPerHour: number;
}

export type MailSecurity = "starttls" | "ssl" | "none";

export interface EmailConfig {
  enabled: boolean;
  /** odbiorcy — po przecinku, sredniku albo spacji */
  to: string;
  intervalMin: number;
  host: string;
  port: number;
  user: string;
  /**
   * Haslo SMTP. Serwer ZAWSZE odsyla tu pusty ciag — haslo mieszka
   * w `secrets.json` i nie opuszcza maszyny. Wyslanie pustej wartosci
   * przy zapisie znaczy „zostaw stare haslo".
   */
  pass: string;
  from: string;
  /**
   * WLASNY TEMAT MAILA ze zmiennymi `${...}`, np. `Aktualny balans: ${balance}`.
   * Puste = temat systemowy `[CONDUIT] kategoria — zdarzenie`.
   *
   * Podstawia WYLACZNIE Rust (`mailer::render_subject`) — panel nigdy nie
   * podstawia sam, tylko prosi serwer o gotowy podglad. Dwa silniki szablonow
   * rozjechalyby sie przy pierwszej zmianie formatu liczby.
   */
  subject: string;
  security: MailSecurity;
  categories: MailCategories;
  throttle: MailThrottle;
}

/** Jedna zmienna dostepna w temacie maila — opis i wartosc „na teraz". */
export interface SubjectVar {
  /** nazwa bez `${}` */
  name: string;
  /** opis po polsku; JEDYNY opis, jaki widzi uzytkownik (nie ma osobnej tabelki) */
  label: string;
  /** wartosc policzona przez serwer, juz sformatowana */
  value: string;
}

/** Odpowiedz `GET /api/email/subject` — podglad tematu + katalog zmiennych. */
export interface SubjectPreview {
  ok: boolean;
  /** szablon, dla ktorego policzono podglad */
  template: string;
  /** szablon zapisany w ustawieniach */
  saved: string;
  /** gotowy temat — dokladnie ten, ktory wyszedlby w mailu */
  preview: string;
  /** temat systemowy (uzywany, gdy szablon jest pusty) */
  systemowy: string;
  pusty: boolean;
  vars: SubjectVar[];
}

export interface NotifyConfig {
  channels: number[];
  summaryEnabled: boolean;
  summaryIntervalMin: number;
}

/* ---------------- POLACZENIA ---------------- */
export interface ConnectionState {
  telegram: "connected" | "connecting" | "disconnected";
  mt5: "connected" | "connecting" | "disconnected";
  account: {
    login: number;
    server: string;
    broker: string;
    currency: string;
    leverage: number;
    type: "DEMO" | "REAL";
  };
  /* CZY KONTO TERMINALA JEST TYM, KTOREGO OCZEKUJE UZYTKOWNIK.
     "ok" = numer rachunku ustawiony i zgodny; "brak" = pole puste, bot
     przyjmuje kazde konto (ZOLTY stan); "rozjazd" = terminal na INNYM koncie
     (CZERWONY, handel zablokowany); "" = MT5 niepodlaczony. Zielona kropka
     bez tozsamosci to polowa informacji — 04.08.2026 bot cala noc pokazywal
     "MT5 OK" grajac na innym brokerze, niz uzytkownik myslal. */
  accountVerified?: "" | "ok" | "brak" | "rozjazd";
  /** Opaque broker binding generation, never reused after reconnect/switch. */
  accountSession?: string;
  /** Actual broker-bound instrument; empty while disconnected/unresolved.
   * Not the persisted mt5_symbol preference (which may be empty for AUTO). */
  resolvedSymbol?: string;
  user: {
    name: string;
    handle: string;
    phone: string;
  };
  latencyMs: number;

  /* ZDROWIE SESJI TELEGRAMA.
     Plakietka „connected" nie odroznia spokojnej nocy od MARTWEGO GNIAZDA
     MTProto: `next_message()` przy zerwanej sesji nie zwraca bledu, tylko
     nigdy nic nie oddaje. Trzecia liczba (`tgPingErrors`) jest jedyna,
     ktora te dwa stany rozroznia. */
  telegramLastMessageMs?: number | null;
  telegramLastPingOkMs?: number | null;
  telegramPingFailures?: number;
  telegramReconnects?: number;
}

/* ---------------- TOASTY ---------------- */
/** Klikalny przycisk w chmurce — np. „Pokaz plik" po scaleniu alllogs. */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface Toast {
  id: number;
  kind: "info" | "success" | "warn" | "error";
  title: string;
  text?: string;
  /** Przyciski akcji. Chmurka z akcja NIE znika sama tak szybko — patrz `ttlMs`. */
  actions?: ToastAction[];
  /** Ile ms trzymac na ekranie; domyslnie 4600. */
  ttlMs?: number;
}

/* ---------------- HALT ---------------- */
export interface HaltState {
  active: boolean;
  reason: string;
  diagnoza?: string;
  ryzyko?: string;
}

/* ---------------- WYKRESY ---------------- */
export interface ChartPanel {
  id: number;
  symbol: string;
  tf: Timeframe;
  style: "candle" | "line" | "area";
  height: number;
  showPositions: boolean;
  showVolume: boolean;
  drawings: Drawing[];
  tool: DrawTool;
  toolsOpen: boolean;
  drawHidden: boolean;
}

export type DrawTool = "cursor" | "brush" | "line" | "hline" | "ray" | "rect" | "eraser";

export interface Drawing {
  id: number;
  tool: DrawTool;
  color: string;
  width: number;
  /** punkty w przestrzeni danych: [czas(ms), cena] */
  pts: [number, number][];
}
