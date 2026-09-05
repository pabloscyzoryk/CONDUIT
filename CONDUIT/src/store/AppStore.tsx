import {
  createContext,
  Fragment,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type {
  Basket,
  ChannelBinding,
  ChatMessage,
  ClosedPosition,
  ForeignSummary,
  ConnectionState,
  Direction,
  EmailConfig,
  Format,
  HaltState,
  Lancuch,
  Lancuchy,
  LogEntry,
  LotConfig,
  NotifyConfig,
  ParsedSignal,
  PendingHistoryItem,
  PendingKind,
  PendingOrder,
  Position,
  Preset,
  Quote,
  Settings,
  SimInstance,
  Stats,
  TelegramChannel,
  Toast,
  TradingMode,
} from "@/types";
import { DEFAULT_SETTINGS } from "@/data/defaultSettings";
import {
  useHistoriaOperacji,
  type Historia,
  type Komenda,
  type Zmiana,
} from "@/store/historiaOperacji";
import { polaRachunkuZ, POLA_RACHUNKU } from "@/store/polaRachunku.generated";
import { resolveBotSymbol, runtimeQuote } from "@/store/runtimeSymbol";

/** Strategy selection must not change account authorization in follow mode.
 * Server enforces the same rule; this also protects the offline UI fallback. */
function preserveFollowTerminalAccount(current: Settings, incoming: Partial<Settings>): Partial<Settings> {
  if (!current.mt5_follow_terminal_account) return incoming;
  const out = { ...incoming } as Record<string, unknown>;
  const before = current as unknown as Record<string, unknown>;
  // Generated UI names + canonical aliases/server-only account fields.
  const keys = [
    ...POLA_RACHUNKU, "server_tz_offset_ms", "msg_clock_offset_ms", "stops_level", "ai_enabled",
    "expo_cap_pct", "sim_margin_check_on_fill", "sim_validate_pending_stops", "sim_margin_at_market",
    "expo_cap_ml_pct", "expo_cap_close", "expo_cap_s", "lot_base",
    "mt5_follow_terminal_account", "mt5_allow_real_account", "mt5_login", "mt5_server", "mt5_password",
    "mt5_terminal_path", "mt5_python", "mt5_symbol", "mt5_magic", "mt5_deviation_points",
  ];
  for (const key of keys) {
    if (Object.prototype.hasOwnProperty.call(before, key)) out[key] = before[key];
    else delete out[key];
  }
  return out as Partial<Settings>;
}

/** USTAWIENIA JEDNEJ NOGI ŁAŃCUCHA — to, czym silnik naprawdę gra.
 *  `doc` powstaje jak `wielosilnik::ustawienia_formatu`: plik presetu nogi
 *  (albo dokument panelu, gdy pliku nie ma) + nadpisanie pól RACHUNKU
 *  z dokumentu. `zPliku` mówi, która z tych dwóch dróg zadziałała. */
export interface UstawieniaNogi {
  format: string;
  preset: string;
  zPliku: boolean;
  handluje: boolean;
  doc: Settings;
}


const DOSYPYWANE_KLUCZE = [
  "one_click",
  "display_currency",
  "poll_ms",
  "show_positions_on_chart",
  "show_potential_tpsl",
  "exclude_pending_potential",
  "comment_mode",
  "comment_custom",
  "price_log",
  "price_log_interval_s",
  "merge_config",
  "merge_chronological",
  "ui_theme",
  "ui_palette",
] as const;
import { CHANNELS } from "@/data/telegram";
import { PRESETS, findPreset, presetFromDisk, RODZINA_Z_DYSKU } from "@/data/presets";
import { DOMYSLNE_LANCUCHY, FORMATY_WBUDOWANE, aktywnyDla, drabinkaDla, lancuchDla, normalizujLancuchy } from "@/data/formaty";
import { PRIMARY_SYMBOL, SYMBOLS } from "@/data/symbols";
import { getQuote, getSeries, tickSeries } from "@/engine/market";
import { parseSignals } from "@/engine/parser";
import {
  makeChatterMessage,
  makeEntryMessage,
  makeManagementMessage,
  makeRmMessage,
  seedHistory,
  type FeedMessage,
} from "@/engine/feed";
import {
  addPending,
  closeAll as engineCloseAll,
  closeBasket as engineCloseBasket,
  closePosition,
  createBotState,
  currentLot,
  deletePending,
  entriesBlocked,
  handleCancel,
  handleOutAtEntry,
  handleRiskFree,
  handleSlHit,
  openBasket,
  openPosition,
  advanceTpStage,
  positionProfit,
  signalTagBlocked,
  tickBot,
  volUnitsFactor,
  type BotState,
} from "@/engine/bot";
import { aiEffectiveSettings, manualEffectiveSettings } from "./aiPolicy";
import * as storage from "./storage";
import { LANGUAGES, applyServerLanguage, t as tSlownik, type Jezyk } from "@/i18n";
import { opisPresetu, tSilnik } from "@/i18n/silnik";
import { KLUCZ_MOTYWU, KLUCZ_PALETY, applyServerAppearance } from "./useTheme";
import { useBackend } from "./useBackend";
import {
  api,
  backendBase,
  type AuthState,
  type DemoConfig,
  type DemoState,
  type LabState,
  type DrabinkaLancuchow,
  type PostepScalania,
  type ServerEvent,
  type TransportStatus,
} from "./transport";

/** Puste laboratorium — stan, gdy backendu nie ma. */
const PUSTE_LAB: LabState = { busy: false, job: null, history: [] };
/** Scalanie robi SERWER — bez backendu nie ma czego scalać ani z czego liczyć
    postępu, więc pokazujemy stan „nigdy nie uruchamiane" zamiast zer. */
/** Drabinka bez backendu — pusta i wyłączona. Plan wykonuje SILNIK (pętla
    co 15 min), więc w przeglądarce bez conduita nie ma czego pokazywać. */
const PUSTA_DRABINKA: DrabinkaLancuchow = {
  enabled: false,
  /* Lustro Default::default() z ui.rs — DRABINKA UKORONOWANA.
     Bez backendu pokazujemy dokładnie to, co silnik ma wbudowane.
     Lustro rozjechało się już raz: tu stały cztery szczeble
     (ZENONLY5/ZENONLY3/SENTINEL-0/SENTINEL-0A), a w silniku trzy inne
     — panel bez backendu obiecywał więc skład, którego bot nie miał. */
  szczeble: [{ progBalance: 0, lancuch: "MONOLIT-1" }],
  histerezaPct: 2, biezacyProg: -1, ostatniaZmianaTs: 0,
};
/** Drabinka trybu AUTO-EA („SKYNET-1") tam, gdzie jej nie ma: PUSTA, nie
    „domyślna" — lustro `ui::drabinka_ea_domyslna()`. Instalacja, która jej
    nigdy nie układała, nie może zobaczyć w panelu planu zmiany składu, bo
    zaraz włączy go jednym kliknięciem (kontrakt zera EA-2c). */
const PUSTA_DRABINKA_EA: DrabinkaLancuchow = {
  enabled: false,
  szczeble: [],
  histerezaPct: 2, biezacyProg: -1, ostatniaZmianaTs: 0,
};
const PUSTE_SCALANIE: PostepScalania = {
  aktywne: false, faza: "", etap: "", postep: 0, zrobione: 0, wszystkich: 0,
  predkosc: "", etaMs: 0, czasMs: 0, plik: "", znakow: 0, sciezka: "", blad: null,
};

/** Domyślna konfiguracja demo — musi zgadzać się z `DemoConfig::default()`
 *  w `crates/server/src/demo/mod.rs`. */
export const DOMYSLNE_DEMO: DemoConfig = {
  balance: 200,
  priceSource: "file",
  ticksPath: "",
  ticksFrom: "",
  ticksTo: "",
  signalsPath: "",
  signalsFrom: "",
  signalsTo: "",
  useFileSignals: true,
  speed: 60,
  seed: 1,
  synthStartPrice: 4118,
  synthVol: 0.15,
  synthSpread: 0.24,
  synthIntervalMs: 250,
  msgClockOffsetMs: null,
  sourceName: "ATFX VIP SIGNALS",
};

/** Tryb demo wyłączony — stan, gdy backendu nie ma. */
const PUSTE_DEMO: DemoState = {
  running: false,
  phase: "idle",
  config: DOMYSLNE_DEMO,
  clock: 0,
  clockLabel: "",
  speed: 0,
  startBalance: 0,
  balance: 0,
  equity: 0,
  ticksDone: 0,
  ticksTotal: 0,
  progress: 0,
  messages: 0,
  signals: 0,
  manualSignals: 0,
  trades: 0,
  openPositions: 0,
  openPendings: 0,
  baskets: 0,
  source: "",
  startedAt: 0,
  finishedAt: null,
  elapsedMs: 0,
  note: "",
  error: null,
};

/* ============================================================
   STORE APLIKACJI

   DWA ŹRÓDŁA DANYCH, JEDEN KONTRAKT.

   * Gdy `conduit.exe` działa — stan przychodzi z niego przez WebSocket,
     a akcje użytkownika lecą tam jako komendy. To jest tryb docelowy.
   * Gdy backendu nie ma — działa lokalna symulacja (rynek → bot →
     statystyki → strumień Telegrama), czyli zachowanie prototypu.

   `AppContextValue` jest w obu przypadkach IDENTYCZNY, więc żaden
   z siedmiu widoków nie wie, skąd biorą się dane, i nie wymagał zmian.
   Przełączenie jest samoczynne: backend uruchomiony po otwarciu strony
   zostaje wykryty w ciągu kilku sekund, bez odświeżania karty.
   ============================================================ */

const START_BALANCE = 2000;

export interface EngineSnapshot {
  positions: Position[];
  pendings: PendingOrder[];
  baskets: Basket[];
  closed: ClosedPosition[];
  pendingHistory: PendingHistoryItem[];
  balance: number;
  
  foreign: ForeignSummary;
}

/** Pusty stan „na rachunku nie ma nic obcego". */
export const NO_FOREIGN: ForeignSummary = {
  positions: 0,
  pendings: 0,
  volume: 0,
  profit: 0,
  magics: [],
  symbols: [],
  comments: [],
};


export type WstrzykniecieOpcje = {
  /** temat forum — dla silnika to OSOBNE źródło (własne koszyki) */
  topicId?: number;
  /** własny numer wiadomości; brak = numer nadany przez serwer */
  msgId?: number;
  /** numer wiadomości, NA KTÓRĄ odpowiadamy */
  replyTo?: number;
  /** numer wiadomości, KTÓRĄ edytujemy — to nie jest nowa wiadomość */
  editOf?: number;
};

export interface AppContextValue {
  /* --- sesja --- */
  loggedIn: boolean;
  /** Czy TELEGRAM jest zalogowany — co innego niż `loggedIn`.
      `loggedIn` znaczy „użytkownik wszedł do terminala" i jest trwałe;
      to znaczy „sesja MTProto istnieje i da się z niej korzystać".
      Rozdzielone, bo aplikacja ma DZIAŁAĆ bez Telegrama: MetaTrader,
      symulacje, tryb demo i laboratorium go nie potrzebują. */
  telegramZalogowany: boolean;
  /** Czy pokazać ekran logowania do Telegrama NA ŻĄDANIE (przycisk w panelu). */
  pokazLogowanieTg: boolean;
  otworzLogowanieTg: () => void;
  /** Wejście do terminala BEZ sesji Telegrama — bot obsłuży MetaTradera,
      symulacje, tryb demo i laboratorium; zabraknie tylko sygnałów z kanałów. */
  wejdzBezTelegrama: () => void;
  zamknijLogowanieTg: () => void;
  login: () => void;
  logout: () => void;
  connection: ConnectionState;

  /* --- tryb i konfiguracja --- */
  mode: TradingMode;
  setMode: (m: TradingMode) => void;
  settings: Settings;
  effectiveSettings: Settings;
  setSetting: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  setSettings: (patch: Partial<Settings>) => void;
  resetSettings: () => void;
  presetId: string;
  applyPreset: (id: string) => void;

  /* --- formaty i łańcuchy (crates/core/src/formaty.rs) --- */
  /** Formaty sygnałów znane silnikowi. Bez backendu — katalog wbudowany. */
  formaty: Format[];
  /** Zbiór łańcuchów wraz ze wskazaniem aktywnego. */
  lancuchy: Lancuchy;
  /**
   * WSKAŹNIK ŁAŃCUCHA DLA TRYBU AUTO-EA (projekt EA-2). Pusty = brak własnego
   * wskazania, czyli AUTO-EA gra tym samym, co reszta trybów. Do wyliczenia
   * „który łańcuch obowiązuje" służy `aktywnyDla()` — NIE porównuj sam.
   */
  aktywnyEa: string;
  /** Nazwa łańcucha, którym bot gra W BIEŻĄCYM TRYBIE (helper na skróty). */
  aktywnaNazwaLancucha: string;
  /** Aktywny łańcuch BIEŻĄCEGO TRYBU; `undefined` tylko gdy zbiór w rozsypce. */
  lancuch: Lancuch | undefined;
  /**
   * Zapis CAŁEGO zbioru: tworzenie, zmiana nazwy, usunięcie, edycja pułapów.
   * `aktywnyEa` podajemy WYŁĄCZNIE w trybie AUTO-EA — pominięty znaczy
   * „nie ruszaj wskazania warstwy EA" (kontrakt zera po stronie serwera).
   */
  setLancuchy: (l: Lancuchy, aktywnyEa?: string) => void;
  /** Samo przełączenie aktywnego łańcucha — POLE WYBIERA TRYB, nie panel. */
  setAktywnyLancuch: (nazwa: string) => void;
  /** Nazwa łańcucha z pieczęci `PACZKA.json`; pusta = brak pieczęci. */
  pieczecLancuch: string;
  /**
   * ŻĄDANIE „POKAŻ PARAMETRY EA TEGO PRESETU" (projekt EA-2).
   *
   * Panel łańcuchów i ekran ustawień to dwa różne widoki, a parametry warstwy
   * EA są ZWYKŁYMI polami presetu — żeby skrót z jednego formatu prowadził
   * dokładnie do jego sekcji EA, ktoś musi przenieść nazwę presetu przez
   * granicę widoków. Robi to ten uchwyt, a nie adres URL: przeżycie tej
   * intencji po odświeżeniu strony byłoby błędem, nie funkcją.
   */
  zadanieEa: { preset: string } | null;
  /** Skrót z panelu łańcuchów: „otwórz Ustawienia na warstwie EA tego presetu". */
  pokazParametryEa: (preset: string) => void;
  /** Zdejmuje żądanie — woła to ekran ustawień, gdy je obsłuży. */
  wyczyscZadanieEa: () => void;
  /** `true`, gdy łańcuchy przyszły z silnika, a nie są trzymane lokalnie. */
  lancuchyZSerwera: boolean;
  /** Presety do wyboru. Z podłączonym botem — TE Z DYSKU (`presets/`), bo
      tylko nimi bot naprawdę handluje. Bez backendu — katalog wbudowany. */
  presets: Preset[];
  findPreset: (id: string) => Preset | undefined;
  /** `true`, gdy lista pochodzi z katalogu `presets/` serwera. */
  presetsFromDisk: boolean;
  lot: LotConfig;
  setLot: (l: LotConfig) => void;
  /** LOT AUTO — suma bieżących lotów nóg HANDLUJĄCYCH (kafel pulpitu). */
  lotSize: number;
  
  ustawieniaNog: UstawieniaNogi[];
  /** LOT RĘCZNY — wielkość z karty „Lot size", używana przy ręcznym
      otwarciu z panelu. Świadomie OSOBNA liczba od `lotSize`: karta panelu
      nie jest lotem automatu (lot należy do presetu nogi). */
  lotReczny: number;

  /* --- rynek --- */
  quotes: Record<string, Quote>;
  primary: Quote;

  /* --- silnik --- */
  snapshot: EngineSnapshot;
  stats: Stats;
  halt: HaltState;
  /** ręczne wznowienie handlu MIMO przekroczonego limitu ryzyka */
  resetHalt: () => void;
  riskOverride: { active: boolean; since: number; reason: string };
  clearRiskOverride: () => void;

  /* --- akcje handlowe --- */
  openManualOrder: (a: {
    kind: "market" | "limit" | "stop";
    direction: Direction;
    volume: number;
    price?: number;
    sl?: number;
    tp?: number;
  }) => void;
  closePos: (ticket: number) => void;
  /** zamknięcie części pozycji (wolumen w lotach) */
  closePartial: (ticket: number, volume: number) => void;
  modifyPos: (ticket: number, sl: number | null, tp: number | null) => void;
  closeBulk: (which: "all" | "profit" | "loss") => void;
  delPending: (ticket: number) => void;
  modifyPending: (ticket: number, price: number, sl: number | null, tp: number | null) => void;
  delAllPendings: () => void;
  closeBasketById: (id: number) => void;
  updateBasket: (id: number, patch: { sl?: number | null; zoneLow?: number; zoneHigh?: number; tps?: number[] }) => void;
  /** Dziennik ręcznych operacji + cofnij/ponów (TODO K16, K17). Obejmuje
      WYŁĄCZNIE to, co zrobił człowiek z panelu — decyzje automatu mają
      własny dziennik po stronie silnika. */
  historia: Historia;

  /* --- telegram --- */
  messages: ChatMessage[];
  bindings: Record<number, ChannelBinding>;
  /** Czaty do wyboru na ekranie „Kanały". Podłączony bot oddaje tu PRAWDZIWĄ
      listę z konta; sam prototyp (bez serwera) — listę poglądową. */
  channels: TelegramChannel[];
  /** `true`, gdy lista pochodzi z konta, a nie z danych poglądowych. */
  channelsAreReal: boolean;
  channelsLoading: boolean;
  channelsError: string | null;
  refreshChannels: () => void;
  setBinding: (channelId: number, patch: Partial<ChannelBinding>) => void;
  simulateMessage: (text: string, channelId?: number, opcje?: WstrzykniecieOpcje) => void;
  executeMessage: (id: string) => void;
  dismissMessage: (id: string) => void;

  /* --- logi --- */
  logs: LogEntry[];
  clearLogs: () => void;
  mergedLogCount: number;

  /* --- e-mail / powiadomienia --- */
  email: EmailConfig;
  setEmail: (e: EmailConfig) => void;
  notify: NotifyConfig;
  setNotify: (n: NotifyConfig) => void;

  /* --- symulacje --- */
  sims: SimInstance[];
  /** `mode` — WŁASNY tryb handlu instancji; brak = dziedziczenie trybu głównego bota. */
  addSim: (preset: string, name: string, balance: number, lot: number, mode?: TradingMode) => void;
  removeSim: (id: string) => void;
  resetSim: (id: string) => void;

  /* --- toasty --- */
  toasts: Toast[];
  toast: (
    kind: Toast["kind"],
    title: string,
    text?: string,
    /** akcje i czas zycia — patrz `Toast`; bez tego zachowanie bez zmian */
    opcje?: Pick<Toast, "actions" | "ttlMs">,
  ) => void;
  dismissToast: (id: number) => void;

  /* --- ulubione instrumenty --- */
  favorites: string[];
  toggleFavorite: (symbol: string) => void;

  /* --- backend (dodane; brak backendu = wartości neutralne) --- */
  /** czy dane pochodzą z `conduit.exe`, czy z lokalnej symulacji */
  live: boolean;
  backendStatus: TransportStatus | "brak";
  /** czy działamy w oknie natywnym (Tauri) — steruje przyciskiem poniżej */
  nativeShell: boolean;
  openInBrowser: () => void;
  /** Postęp laboratorium (backtesty, trening AI). Bez backendu jest pusty —
      symulacja w przeglądarce nie ma 54 mln ticków, na których mogłaby liczyć. */
  lab: LabState;
  /** Stan trybu demo (wirtualny broker w czasie rzeczywistym). Bez backendu
      jest wyłączony — odtwarzanie 54 mln ticków nie dzieje się w przeglądarce. */
  demo: DemoState;
  
  drabinka: DrabinkaLancuchow;
  /** Zapis drabinki BIEŻĄCEGO trybu — komenda niesie adres trybu. */
  setDrabinka: (d: DrabinkaLancuchow) => void;
  
  scalanie: PostepScalania;
  /** stan logowania do Telegrama zgłaszany przez backend (null = brak backendu) */
  auth: AuthState | null;
  startQrLogin: () => Promise<AuthState | null>;
  submit2fa: (password: string) => Promise<AuthState | null>;
  /** KROK ZERO logowania: api_id/api_hash z my.telegram.org. */
  setTelegramCredentials: (apiId: string, apiHash: string) => Promise<AuthState | null>;
  /** Kasuje poświadczenia i sesję — pełny reset logowania. */
  forgetTelegramCredentials: () => Promise<AuthState | null>;
  /** Przycisk „wyślij mail testowy" — zwraca wynik prawdziwej próby. */
  sendTestEmail: () => Promise<void>;
  
  scalLogi: () => Promise<void>;
  /** Prawdziwa wysyłka testowa na kanały Telegrama (przycisk „Wyślij test”). */
  wyslijTestPowiadomienia: () => void;
}

const Ctx = createContext<AppContextValue | null>(null);

export function useApp(): AppContextValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("useApp musi być użyte wewnątrz <AppProvider>");
  return v;
}

/* ------------------------------------------------------------------ */

/**
 * Doprowadza powiazanie kanalu do JEDNEGO formatu.
 *
 * Dzisiejszy serwer (`ui.rs: ChannelBinding`) odsyla jeszcze stary ksztalt:
 * `formats: Vec<String>` i `topics: {id: Vec<String>}`. Panel nie ma prawa
 * wywalic sie na danych, ktore realnie przychodza z dzialajacego bota — wiec
 * czyta oba ksztalty i bierze PIERWSZY wpis listy. Wybor „pierwszy" jest
 * swiadomy: kanal z dwoma formatami i tak nigdy nie byl obslugiwany (jedyne
 * uzycie pola w drzewie to zapis), wiec nie ma czego gubic.
 */
function normalizujBinding(raw: Partial<ChannelBinding> | undefined, channelId: number): ChannelBinding {
  const pierwszy = (v: unknown): string => {
    if (typeof v === "string") return v;
    if (Array.isArray(v)) return typeof v[0] === "string" ? v[0] : "";
    return "";
  };
  const topics: Record<number, string> = {};
  for (const [k, v] of Object.entries(raw?.topics ?? {})) {
    const f = pierwszy(v);
    if (f) topics[Number(k)] = f;
  }
  return {
    channelId,
    monitored: raw?.monitored ?? false,
    notify: raw?.notify ?? false,
    format: pierwszy(raw?.format ?? raw?.formats),
    topics,
  };
}

function initialBindings(): Record<number, ChannelBinding> {
  const out: Record<number, ChannelBinding> = {};
  for (const c of CHANNELS) {
    out[c.id] = { channelId: c.id, monitored: false, notify: false, format: "", topics: {} };
  }
  return out;
}

let toastSeq = 1;
let logSeq = 1;
let msgSeq = 1;

export function AppProvider({ children }: { children: ReactNode }) {
  /* ---------------- stan konfiguracyjny (persystowany) ---------------- */
  const [loggedIn, setLoggedIn] = useState(() => storage.load("loggedIn", false));
  
  const [pokazLogowanieTg, setPokazLogowanieTg] = useState(false);
  const [mode, setModeRaw] = useState<TradingMode>(() => storage.load<TradingMode>("mode", "AUTO"));
  const [settings, setSettingsState] = useState<Settings>(() => ({
    ...DEFAULT_SETTINGS,
    ...storage.load<Partial<Settings>>("settings", {}),
    merge_config: { ...DEFAULT_SETTINGS.merge_config, ...storage.load<Partial<Settings>>("settings", {}).merge_config },
  }));
  const [presetId, setPresetId] = useState(() => storage.load("preset", ""));
  const [lot, setLotState] = useState<LotConfig>(() =>
    storage.load<LotConfig>("lot", { mode: "fixed", fixed: 0.01, percent: 1 }),
  );
  const [bindings, setBindings] = useState<Record<number, ChannelBinding>>(() => {
    // Zapis z localStorage może pochodzić sprzed przejścia na JEDEN format na
    // kanał (trzymał `formats: []`). Normalizujemy przy wczytaniu, bo inaczej
    // ekran „Kanały" dostałby tablicę tam, gdzie oczekuje napisu.
    const zapisane = storage.load<Record<number, Partial<ChannelBinding>>>("bindings", initialBindings());
    const out: Record<number, ChannelBinding> = {};
    for (const [k, v] of Object.entries(zapisane)) out[Number(k)] = normalizujBinding(v, Number(k));
    return out;
  });
  /* ŁAŃCUCHY trzymane lokalnie — obowiązują, dopóki silnik nie przyśle
     własnej sekcji `lancuchy`. To NIE jest atrapa: bez backendu panel i tak
     jest jedynym miejscem, w którym ta konfiguracja istnieje. */
  const [lancuchyLokalne, setLancuchyLokalne] = useState<Lancuchy>(() =>
    normalizujLancuchy(storage.load<Lancuchy>("lancuchy", DOMYSLNE_LANCUCHY)),
  );
  /* WSKAŹNIK ŁAŃCUCHA DLA AUTO-EA (projekt EA-2) — osobny od `aktywny`,
     trzymany lokalnie z tego samego powodu co reszta łańcuchów. Pusty
     łańcuch znaków = brak własnego wskazania (kontrakt zera). */
  const [aktywnyEaLokalny, setAktywnyEaLokalny] = useState<string>(() =>
    storage.load<string>("aktywnyEa", ""),
  );
  /* Żądanie „otwórz parametry EA presetu X" — intencja jednego kliknięcia,
     świadomie NIE zapisywana w localStorage: po odświeżeniu strony nikt nie
     oczekuje, że panel sam wskoczy w sekcję sprzed restartu. */
  const [zadanieEa, setZadanieEa] = useState<{ preset: string } | null>(null);
  /* Czaty pobrane z konta Telegram. Pusta tablica = jeszcze nie wiemy albo
     nie ma zalogowanej sesji; wtedy NIE podstawiamy listy poglądowej, bo
     zaznaczenie wymyślonego kanału cicho wyłączałoby handel. */
  const [serverChannels, setServerChannels] = useState<TelegramChannel[]>([]);
  const [channelsLoading, setChannelsLoading] = useState(false);
  const [channelsError, setChannelsError] = useState<string | null>(null);
  /* Presety wczytane z katalogu `presets/` serwera. Pusta tablica = jeszcze
     nie wiemy albo backendu nie ma; wtedy panel pokazuje katalog wbudowany. */
  const [serverPresets, setServerPresets] = useState<Preset[]>([]);
  const [email, setEmailState] = useState<EmailConfig>(() =>
    storage.load("email", {
      enabled: false,
      to: "",
      intervalMin: 60,
      host: "smtp.example.com",
      port: 587,
      user: "",
      pass: "",
      from: "",
      // Pusty temat = systemowy `[CONDUIT] kategoria — zdarzenie`.
      // Wypełnia go dopiero użytkownik w karcie „Powiadomienia".
      subject: "",
      security: "starttls" as const,
      categories: {
        lifecycle: true,
        mt5Connection: true,
        mt5RecoveryFailed: true,
        drawdown: true,
        orderError: true,
        // Jedyna kategoria, która sypie mailami przy CAŁKOWICIE poprawnej
        // pracy bota — dlatego wymaga świadomego włączenia.
        summary: false,
        // WŁĄCZONA domyślnie: to jedyny alarm, po którym widać, że kanał
        // zmienił zapis sygnałów, ZANIM minie dzień bez handlu.
        signalUnreadable: true,
      },
      throttle: { windowMin: 10, maxPerHour: 12 },
    }),
  );
  const [notify, setNotifyState] = useState<NotifyConfig>(() =>
    storage.load("notify", { channels: [CHANNELS[5].id], summaryEnabled: true, summaryIntervalMin: 240 }),
  );
  const [favorites, setFavorites] = useState<string[]>(() => storage.load("favorites", [PRIMARY_SYMBOL, "BTCUSD"]));

  /* ---------------- stan ulotny ---------------- */
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [halt, setHalt] = useState<HaltState>({ active: false, reason: "" });
  const [riskOverride, setRiskOverride] = useState({ active: false, since: 0, reason: "" });
  const [quotes, setQuotes] = useState<Record<string, Quote>>({});
  const [snapshot, setSnapshot] = useState<EngineSnapshot>({
    positions: [],
    pendings: [],
    baskets: [],
    closed: [],
    pendingHistory: [],
    balance: START_BALANCE,
    foreign: NO_FOREIGN,
  });
  const [sims, setSims] = useState<SimInstance[]>(() => storage.load("sims", [] as SimInstance[]));
  const [stats, setStats] = useState<Stats>(() => ({
    balance: START_BALANCE,
    equity: START_BALANCE,
    margin: 0,
    freeMargin: START_BALANCE,
    marginLevel: 0,
    credit: 0,
    creditApplied: 0,
    creditSource: "off",
    lotBase: START_BALANCE,
    lotNogi: [],
    creditMismatch: false,
    pnlToday: 0,
    pnlSession: 0,
    sessionStart: Date.now(),
    drawdownNow: 0,
    maxDdToday: 0,
    peakEquityToday: START_BALANCE,
    dayStartEquity: START_BALANCE,
    messages: 0,
    signals: 0,
    equityCurve: [{ t: Date.now(), v: START_BALANCE }],
  }));

  /* ================= BACKEND =================
     Zdarzenia z serwera trafiają wprost do setterów stanu — NIE przez
     `toast`/`addLog`, bo te są zdefiniowane niżej, a kolejność hooków
     musi być stała. Settery Reacta są stabilne, więc most nie przełącza
     się przy każdym renderze. */
  const handleServerEvent = useCallback((e: ServerEvent) => {
    switch (e.kind) {
      case "toast": {
        const id = toastSeq++;
        const kind = (["info", "success", "warn", "error"] as const).includes(e.level as never)
          ? (e.level as Toast["kind"])
          : "info";
        setToasts((t) => [...t.slice(-4), { id, kind, title: e.title, text: e.text }]);
        window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4600);
        break;
      }
      case "alert":
        setToasts((t) => [
          ...t.slice(-4),
          {
            id: toastSeq++,
            kind: e.halted ? "error" : "warn",
            title: e.halted ? tSlownik("toast.halted") : tSlownik("toast.warning"),
            /* Powód przychodzi z SILNIKA po polsku — mapa `tSilnik` zamienia
               znane zdania na język panelu, nieznane przepuszcza bez zmian. */
            text: tSilnik(e.reason),
          },
        ]);
        break;
      case "log":
        // dziennik i tak przychodzi w sekcji `logs`; tu tylko podbijamy
        // widok natychmiast, żeby wpis nie czekał na najbliższą deltę
        setLogs((l) => (l.some((x) => x.id === e.entry.id) ? l : [e.entry, ...l].slice(0, 400)));
        break;
      default:
        break;
    }
  }, []);

  const backend = useBackend(handleServerEvent);
  const B = backend.snapshot;
  const live = backend.live && B !== null;

  /* MIGAWKA SILNIKA — POLICZONA TU, NIE NA KOŃCU KOMPONENTU.
     Historia ręcznych operacji (`useHistoriaOperacji`, TODO K16) musi znać
     stan sprzed zmiany W CHWILI wysyłania komendy, a przycisk „cofnij" musi
     wiedzieć, czy cel nadal wygląda tak, jak go zostawiliśmy. Obie rzeczy
     potrzebują migawki WCZEŚNIEJ niż blok widoków na dole pliku. */
  const snapshotView: EngineSnapshot = live
    ? {
        positions: B!.positions,
        pendings: B!.pendings,
        baskets: B!.baskets,
        closed: B!.closed,
        pendingHistory: B!.pendingHistory,
        balance: B!.balance,
        // starszy backend nie zna tego pola — brak znaczy „nic obcego"
        foreign: B!.foreign ?? NO_FOREIGN,
      }
    : snapshot;
  /* Akcje czytają stan sprzed zmiany przez referencję: gdyby brały go z domknięcia,
     każdy tick przebudowywałby wszystkie `useCallback` akcji handlowych. */
  const migawkaRef = useRef(snapshotView);
  migawkaRef.current = snapshotView;

  /* ---------------- referencje silnika ---------------- */
  const engine = useRef<BotState>(createBotState(PRIMARY_SYMBOL, START_BALANCE));
  const statsRef = useRef(stats);
  const settingsRef = useRef(settings);
  const modeRef = useRef(mode);
  const lotRef = useRef(lot);
  const bindingsRef = useRef(bindings);
  const haltRef = useRef(halt);
  const overrideRef = useRef(riskOverride);
  const lastTick = useRef(Date.now());
  const nextSignalAt = useRef(Date.now() + 9000);
  const nextMgmtAt = useRef(Date.now() + 14000);

  /* ---------------- persystencja ---------------- */
  useEffect(() => storage.save("loggedIn", loggedIn), [loggedIn]);
  useEffect(() => storage.save("mode", mode), [mode]);
  useEffect(() => storage.save("settings", settings), [settings]);
  useEffect(() => storage.save("preset", presetId), [presetId]);
  useEffect(() => storage.save("lot", lot), [lot]);
  useEffect(() => storage.save("bindings", bindings), [bindings]);
  useEffect(() => storage.save("lancuchy", lancuchyLokalne), [lancuchyLokalne]);
  useEffect(() => storage.save("aktywnyEa", aktywnyEaLokalny), [aktywnyEaLokalny]);
  useEffect(() => storage.save("email", email), [email]);
  useEffect(() => storage.save("notify", notify), [notify]);
  useEffect(() => storage.save("favorites", favorites), [favorites]);
  useEffect(() => storage.save("sims", sims), [sims]);

  useEffect(() => void (settingsRef.current = settings), [settings]);
  useEffect(() => void (modeRef.current = mode), [mode]);
  useEffect(() => void (lotRef.current = lot), [lot]);
  useEffect(() => void (bindingsRef.current = bindings), [bindings]);
  useEffect(() => void (haltRef.current = halt), [halt]);
  useEffect(() => void (overrideRef.current = riskOverride), [riskOverride]);
  useEffect(() => void (statsRef.current = stats), [stats]);

  /* ---------------- widok konfiguracji ----------------
     Gdy backend działa, to ON jest właścicielem konfiguracji — lokalny stan
     zostaje nietknięty i wraca do gry po odłączeniu serwera. */
  const serverSettings = B?.settings;
  const settingsView = useMemo<Settings>(
    () => (live ? { ...DEFAULT_SETTINGS, ...settings, ...(serverSettings as Partial<Settings>) } : settings),
    [live, settings, serverSettings],
  );
  const modeView = live ? B!.mode : mode;
  const lotView = live ? B!.lot : lot;
  const presetView = live ? B!.presetId : presetId;
  const balanceView = live ? B!.balance : snapshot.balance;

  /* ---------------- ustawienia efektywne ---------------- */
  const effectiveSettings = useMemo<Settings>(() => {
    if (modeView === "AI") return aiEffectiveSettings(settingsView, settingsView.ai_model);
    if (modeView === "MANUAL") return manualEffectiveSettings(settingsView);
    return { ...settingsView, ai_mode: false };
  }, [modeView, settingsView]);

  const effRef = useRef(effectiveSettings);
  useEffect(() => void (effRef.current = effectiveSettings), [effectiveSettings]);

  
  const statsNogi = (live ? B!.stats.lotNogi : null) ?? [];
  const lotSize = useMemo(() => {
    const handlujace = statsNogi.filter((n) => n.handluje ?? !n.zamrozona);
    if (statsNogi.length > 0) {
      const suma = handlujace.reduce(
        (a, n) => a + (typeof n.lotKoszyka === "number" && n.lotKoszyka > 0 ? n.lotKoszyka : n.lot),
        0,
      );
      return Math.round(suma * 100) / 100;
    }
    return currentLot(lotView.mode, lotView.fixed, lotView.percent, balanceView, settingsView.lot_scale_step);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [statsNogi, lotView, balanceView, settingsView.lot_scale_step]);

  
  const lotReczny = useMemo(
    () => currentLot(lotView.mode, lotView.fixed, lotView.percent, balanceView, 0),
    [lotView, balanceView],
  );

  
  const [docyPresetow, setDocyPresetow] = useState<Record<string, Settings>>({});
  const kluczePresetow = useMemo(
    () => [...new Set(statsNogi.filter((n) => n.zPliku && n.preset).map((n) => n.preset))].sort().join("|"),
    [statsNogi],
  );
  useEffect(() => {
    if (!live || !kluczePresetow) {
      setDocyPresetow({});
      return;
    }
    let zywe = true;
    void Promise.all(
      kluczePresetow.split("|").map((p) =>
        api.presetUi(p).then(
          (r) => [p, { ...DEFAULT_SETTINGS, ...(r.settings as Partial<Settings>) }] as const,
          // Cisza jest tu zakazana: brak pliku zostawia nogę BEZ wpisu,
          // a panel pokaże „?" zamiast podstawionej liczby z dokumentu.
          () => null,
        ),
      ),
    ).then((pary) => {
      if (zywe) setDocyPresetow(Object.fromEntries(pary.filter(Boolean) as [string, Settings][]));
    });
    return () => {
      zywe = false;
    };
  }, [live, kluczePresetow]);

  const ustawieniaNog = useMemo<UstawieniaNogi[]>(
    () =>
      statsNogi
        .map((n) => {
          const bazowy = n.zPliku ? docyPresetow[n.preset] : settingsView;
          const doc = bazowy ? ({ ...bazowy, ...polaRachunkuZ(settingsView) } as Settings) : null;
          return {
            format: n.format,
            preset: n.preset,
            zPliku: n.zPliku,
            handluje: n.handluje ?? !n.zamrozona,
            doc,
          };
        })
        .filter((x): x is UstawieniaNogi => x.doc !== null),
    [statsNogi, docyPresetow, settingsView],
  );

  /* ---------------- narzedzia ---------------- */
  const toast = useCallback(
    (kind: Toast["kind"], title: string, text?: string, opcje?: Pick<Toast, "actions" | "ttlMs">) => {
      const id = toastSeq++;
      setToasts((t) => [...t.slice(-4), { id, kind, title, text, ...opcje }]);
      // Chmurka z przyciskiem musi przezyc siegniecie po mysz: 4,6 s wystarcza
      // na przeczytanie, ale nie na klikniecie „Pokaz plik".
      const ttl = opcje?.ttlMs ?? (opcje?.actions?.length ? 15_000 : 4600);
      window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), ttl);
    },
    [],
  );

  const dismissToast = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const addLog = useCallback(
    (category: LogEntry["category"], title: string, content = "", level: LogEntry["level"] = "info") => {
      setLogs((l) => [{ id: logSeq++, t: Date.now(), category, title, content, level }, ...l].slice(0, 400));
    },
    [],
  );

  /* ---------------- obsluga sygnalow ---------------- */
  const findBasketForSource = useCallback((sourceKey: string): Basket | undefined => {
    const alive = engine.current.baskets.filter((b) => b.active && b.sourceKey === sourceKey);
    return alive[alive.length - 1] ?? engine.current.baskets.filter((b) => b.active).slice(-1)[0];
  }, []);

  const applyParsed = useCallback(
    (parsed: ParsedSignal[], sourceKey: string, sourceName: string, rawText: string) => {
      const s = engine.current;
      const st = effRef.current;
      const price = getSeries(s.symbol).lastPrice;
      const lotNow = currentLot(lotRef.current.mode, lotRef.current.fixed, lotRef.current.percent, s.balance, st.lot_scale_step);

      for (const sig of parsed) {
        switch (sig.type) {
          case "ENTRY": {
            if (haltRef.current.active) {
              addLog("signals", "Sygnał zablokowany", `Handel wstrzymany: ${haltRef.current.reason}`, "warn");
              break;
            }
            const tagBlock = signalTagBlocked(rawText, st);
            if (tagBlock) {
              addLog("signals", "Sygnał odrzucony filtrem", tagBlock, "warn");
              break;
            }
            const gate = entriesBlocked(s, st, statsRef.current.pnlToday, lotNow);
            if (gate) {
              addLog("signals", "Wejścia zablokowane", gate, "warn");
              toast("warn", tSlownik("toast.blockedEntry"), tSilnik(gate));
              break;
            }
            const series = getSeries(s.symbol);
            const win = st.vol_window_min > 0 ? series.base1m.slice(-Math.round(st.vol_window_min)) : [];
            const range = win.length ? Math.max(...win.map((k) => k.h)) - Math.min(...win.map((k) => k.l)) : 0;

            const b = openBasket({
              state: s,
              sig,
              st,
              lot: lotNow,
              price,
              source: sourceName,
              sourceKey,
              mode: modeRef.current,
              volFactor: volUnitsFactor(st, range),
            });
            if (b) {
              addLog(
                "trades",
                `Koszyk B${b.id} · ${b.direction}${b.isLimit ? " LIMIT" : ""}`,
                `strefa ${b.zoneLow.toFixed(2)}–${b.zoneHigh.toFixed(2)} · SL ${b.sl?.toFixed(2) ?? "—"} · TP ${b.tps
                  .map((x) => x.toFixed(2))
                  .join(" / ")}`,
                "success",
              );
              toast("success", tSlownik("toast.basketCreated", { id: b.id }), `${b.direction}${b.isLimit ? " LIMIT" : ""} · ${sourceName}`);
            }
            break;
          }
          case "TP_HIT": {
            if (!st.tp_detect_signal) break;
            const b = findBasketForSource(sourceKey);
            if (b) {
              const target = sig.tpIndex ? sig.tpIndex : b.tpStage + 1;
              if (st.tp_hit_fill_stages) {
                while (b.tpStage < target) advanceTpStage(s, b, st, price, "wiadomość");
              } else {
                b.tpStage = Math.max(b.tpStage, target - 1);
                advanceTpStage(s, b, st, price, "wiadomość");
              }
              addLog("signals", `TP${target} HIT · koszyk B${b.id}`, rawText.slice(0, 120), "success");
            }
            break;
          }
          case "SL_HIT": {
            const b = findBasketForSource(sourceKey);
            if (b) {
              handleSlHit(s, b, st, price);
              addLog("signals", `SL HIT · koszyk B${b.id}`, rawText.slice(0, 120), "error");
            }
            break;
          }
          case "RISK_FREE": {
            const b = findBasketForSource(sourceKey);
            if (b) {
              // poziom z komunikatu („RISK FREE 4000") decyduje, która pozycja zostaje
              handleRiskFree(s, b, st, price, sig.level ?? null);
              addLog(
                "signals",
                `RISK FREE · koszyk B${b.id}`,
                sig.level ? `poziom ${sig.level.toFixed(2)}` : "bez poziomu — użyto krawędzi strefy",
                "info",
              );
            }
            break;
          }
          case "PARTIAL": {
            const b = findBasketForSource(sourceKey);
            if (b) {
              if (sig.tps?.length) b.tps = sig.tps;
              advanceTpStage(s, b, st, price, "SPP");
              addLog("signals", `SECURING PARTIAL PROFITS · B${b.id}`, "", "info");
            }
            break;
          }
          case "OUT_AT_ENTRY": {
            const b = findBasketForSource(sourceKey);
            if (b) {
              handleOutAtEntry(s, b, st, price);
              addLog("signals", `OUT AT ENTRY · koszyk B${b.id}`, "", "warn");
            }
            break;
          }
          case "CANCEL": {
            const b = findBasketForSource(sourceKey);
            if (b) {
              handleCancel(s, b);
              addLog("signals", `CANCEL · koszyk B${b.id}`, "", "warn");
            }
            break;
          }
          case "CLOSE_ALL": {
            engineCloseAll(s, price, "BASKET");
            addLog("commands", "CLOSE ALL z kanału", "", "warn");
            toast("warn", tSlownik("toast.closeAll.title"), tSlownik("toast.closeAll.text"));
            break;
          }
          case "TP_CORRECTION": {
            const b = findBasketForSource(sourceKey);
            if (b && sig.tpIndex && sig.tps?.length) {
              b.tps[sig.tpIndex - 1] = sig.tps[0];
              addLog("signals", `Korekta TP${sig.tpIndex} · B${b.id}`, `→ ${sig.tps[0].toFixed(2)}`, "info");
            }
            break;
          }
          case "SET_SL": {
            const b = findBasketForSource(sourceKey);
            if (b && sig.sl != null) {
              b.sl = sig.sl;
              for (const p of s.positions.filter((x) => x.basketId === b.id && !x.frozen)) p.sl = sig.sl!;
              addLog("signals", `Nowy SL koszyka B${b.id}`, `→ ${sig.sl.toFixed(2)}`, "info");
            }
            break;
          }
          default:
            break;
        }
      }
    },
    [addLog, findBasketForSource, toast],
  );

  /* ---------------- przyjecie wiadomosci ---------------- */
  const ingest = useCallback(
    (fm: FeedMessage, opts: { simulated?: boolean } = {}) => {
      const parsed = parseSignals(fm.text);
      const actionable = parsed.some((p) => p.type !== "INFO");
      const binding = bindingsRef.current[fm.channelId];
      const monitored = binding?.monitored ?? false;

      const msg: ChatMessage = {
        id: `m${msgSeq++}`,
        time: Date.now(),
        channelId: fm.channelId,
        channelName: fm.channelName,
        topicName: fm.topicName,
        text: fm.text,
        types: parsed.map((p) => p.type),
        basketId: null,
        edited: false,
        parsed,
        pendingAction: undefined,
      };

      setStats((s) => ({
        ...s,
        messages: s.messages + 1,
        signals: s.signals + (actionable ? 1 : 0),
      }));

      addLog("messages", `${fm.channelName}${fm.topicName ? ` · ${fm.topicName}` : ""}`, fm.text, "info");
      if (actionable) addLog("signals", `Rozpoznano: ${parsed.map((p) => p.type).join(", ")}`, fm.text, "info");
      else addLog("unpredicted_signals", "Brak dopasowania", fm.text, "info");

      const sourceKey = `${fm.channelId}${fm.topicName ? `:${fm.topicName}` : ""}`;

      if (!monitored && !opts.simulated) {
        // kanal nienasluchiwany — wiadomosc trafia tylko do czatu
        setMessages((m) => [...m, msg].slice(-220));
        return;
      }

      if (modeRef.current === "MANUAL" && actionable) {
        msg.pendingAction = "await";
        setMessages((m) => [...m, msg].slice(-220));
        toast("info", tSlownik("toast.newSignal.title"), tSlownik("toast.newSignal.text"));
        return;
      }

      applyParsed(parsed, sourceKey, fm.channelName, fm.text);
      const last = engine.current.baskets[engine.current.baskets.length - 1];
      if (last && Date.now() - last.createdAt < 1500) msg.basketId = last.id;
      setMessages((m) => [...m, msg].slice(-220));
    },
    [addLog, applyParsed, toast],
  );

  /* ---------------- wiadomosci startowe ---------------- */
  useEffect(() => {
    const price = getSeries(PRIMARY_SYMBOL).lastPrice;
    const seeded = seedHistory(price).map((h) => {
      const parsed = parseSignals(h.text);
      return {
        id: `m${msgSeq++}`,
        time: Date.now() - h.age,
        channelId: h.channelId,
        channelName: h.channelName,
        text: h.text,
        types: parsed.map((p) => p.type),
        basketId: null,
        edited: false,
        parsed,
      } satisfies ChatMessage;
    });
    setMessages(seeded);
    setStats((s) => ({ ...s, messages: seeded.length, signals: seeded.filter((m) => m.types[0] !== "INFO").length }));
    addLog("events", "Bot wystartował", "Sesja zainicjowana, backup pamięci wczytany", "success");
    addLog("poll_interval", "Interwał pętli", `${DEFAULT_SETTINGS.poll_ms} ms`, "info");
  }, [addLog]);

  /* ================= GLOWNA PETLA (TYLKO TRYB DEMO) =================
     Gdy backend działa, cena, pozycje i statystyki przychodzą z niego.
     Puszczanie tu drugiej, lokalnej symulacji dawałoby dwa różne obrazy
     tego samego konta — czyli najgorszy możliwy rodzaj błędu w terminalu
     handlowym. Dlatego pętla jest twardo wyłączona w trybie live. */
  useEffect(() => {
    if (!loggedIn || live) return;
    let raf = 0;
    let stopped = false;

    const loop = () => {
      if (stopped) return;
      const t = Date.now();
      const dt = t - lastTick.current;
      lastTick.current = t;

      /* --- rynek --- */
      const q: Record<string, Quote> = {};
      for (const sym of SYMBOLS) {
        tickSeries(sym.symbol, dt);
        q[sym.symbol] = getQuote(sym.symbol);
      }

      /* --- bot --- */
      const s = engine.current;
      const st = effRef.current;
      const price = q[s.symbol].bid;
      const r = tickBot(s, price, st, modeRef.current);

      if (r.filled) addLog("trades", `Wypełniono ${r.filled} zlecenie(a)`, "", "success");

      /* --- statystyki --- */
      const floating = s.positions.reduce((a, p) => a + positionProfit(p, price), 0);
      const equity = s.balance + floating;
      const prev = statsRef.current;
      const peak = Math.max(prev.peakEquityToday, equity);
      const dd = peak - equity;
      const curve =
        t - (prev.equityCurve[prev.equityCurve.length - 1]?.t ?? 0) > 2000
          ? [...prev.equityCurve, { t, v: equity }].slice(-400)
          : prev.equityCurve;

      const next: Stats = {
        ...prev,
        balance: s.balance,
        equity,
        margin: s.positions.reduce((a, p) => a + p.volume * 100 * price * 0.005, 0),
        freeMargin: equity - s.positions.reduce((a, p) => a + p.volume * 100 * price * 0.005, 0),
        marginLevel: 0,
        pnlToday: equity - prev.dayStartEquity,
        pnlSession: equity - START_BALANCE,
        drawdownNow: dd,
        maxDdToday: Math.max(prev.maxDdToday, dd),
        peakEquityToday: peak,
        equityCurve: curve,
      };
      next.marginLevel = next.margin > 0 ? (equity / next.margin) * 100 : 0;
      statsRef.current = next;

      /* --- strażnicy: MAX DRAWDOWN ---
         Gdy użytkownik świadomie wznowił handel mimo przekroczonego limitu
         (riskOverride), strażnik NIE uzbraja się ponownie — inaczej przycisk
         „Wznów handel" byłby bez znaczenia, bo pętla natychmiast zatrzymałaby
         handel na tym samym warunku. */
      if (!haltRef.current.active && !overrideRef.current.active) {
        const ddPct = (dd / Math.max(1, peak)) * 100;
        if (st.max_dd_pct > 0 && ddPct >= st.max_dd_pct) {
          engineCloseAll(s, price, "MAX_DD");
          /* Ten sam klucz, którym `tSilnik` tłumaczy powód z prawdziwego
             silnika — tryb bez backendu ma mówić dokładnie to samo zdanie. */
          const reason = tSlownik("eng.halt.maxDdPct", { a: ddPct.toFixed(1), b: st.max_dd_pct });
          haltRef.current = { active: true, reason };
          setHalt({ active: true, reason });
          addLog("events", "HANDEL WSTRZYMANY", reason, "error");
          toast("error", tSlownik("toast.halted"), reason);
        } else if (st.max_dd_usd > 0 && dd >= st.max_dd_usd) {
          engineCloseAll(s, price, "MAX_DD");
          const reason = tSlownik("eng.halt.maxDdUsd", { a: dd.toFixed(2), b: st.max_dd_usd });
          haltRef.current = { active: true, reason };
          setHalt({ active: true, reason });
          addLog("events", "HANDEL WSTRZYMANY", reason, "error");
          toast("error", tSlownik("toast.halted"), reason);
        } else if (st.day_trail_stop_usd > 0 && dd >= st.day_trail_stop_usd && s.positions.length) {
          engineCloseAll(s, price, "DAY_TARGET");
          addLog("events", "DAY-TRAIL", `Equity spadło o $${dd.toFixed(2)} od piku dnia`, "warn");
          toast("warn", tSlownik("toast.dayTrail.title"), tSlownik("toast.dayTrail.text"));
        }
      }

      /* --- cel dzienny z zamknięciem --- */
      if (st.day_target_usd > 0 && st.day_target_close && next.pnlToday >= st.day_target_usd && s.positions.length) {
        engineCloseAll(s, price, "DAY_TARGET");
        addLog("events", "CEL DZIENNY", `+$${next.pnlToday.toFixed(2)} — zamknięto wszystko`, "success");
        toast("success", tSlownik("toast.dayTarget.title"), tSlownik("toast.dayTarget.text"));
      }

      /* --- publikacja snapshotu --- */
      setQuotes(q);
      setStats(next);
      setSnapshot({
        positions: s.positions.map((p) => ({ ...p, profit: positionProfit(p, price) })),
        pendings: [...s.pendings],
        baskets: s.baskets.map((b) => ({ ...b })),
        closed: s.closed.slice(0, 200),
        pendingHistory: s.pendingHistory.slice(0, 200),
        foreign: NO_FOREIGN,
        balance: s.balance,
      });

      /* --- strumień Telegrama --- */
      if (t >= nextSignalAt.current) {
        nextSignalAt.current = t + 42_000 + Math.random() * 70_000;
        const roll = Math.random();
        if (roll < 0.6) ingest(makeEntryMessage(price));
        else if (roll < 0.75) ingest(makeRmMessage(price));
        else ingest(makeChatterMessage());
      }
      if (t >= nextMgmtAt.current) {
        nextMgmtAt.current = t + 26_000 + Math.random() * 40_000;
        const alive = s.baskets.filter((b) => b.active && b.tickets.length > 0);
        if (alive.length) {
          const b = alive[Math.floor(Math.random() * alive.length)];
          const fm = makeManagementMessage(b, price);
          if (fm) ingest(fm);
        }
      }

      raf = window.setTimeout(loop, Math.max(120, effRef.current.poll_ms || 250));
    };

    raf = window.setTimeout(loop, 250);
    return () => {
      stopped = true;
      window.clearTimeout(raf);
    };
  }, [loggedIn, live, addLog, ingest, toast]);

  /* ---------------- akcje ---------------- */
  const publish = useCallback(() => {
    const s = engine.current;
    const price = getSeries(s.symbol).lastPrice;
    setSnapshot({
      positions: s.positions.map((p) => ({ ...p, profit: positionProfit(p, price) })),
      pendings: [...s.pendings],
      baskets: s.baskets.map((b) => ({ ...b })),
      closed: s.closed.slice(0, 200),
      pendingHistory: s.pendingHistory.slice(0, 200),
      foreign: NO_FOREIGN,
      balance: s.balance,
    });
  }, []);

  /* ---------------- routing akcji ----------------
     Jedna reguła na wszystkie akcje: gdy backend działa, komenda leci do
     niego i to ON zmienia stan (a zmiana wraca deltą do WSZYSTKICH powłok).
     Gdy backendu nie ma, wykonuje się dotychczasowa ścieżka lokalna.
     `wyslij` zwraca `true`, jeśli przejął akcję. */
  const liveRef = useRef(live);
  useEffect(() => void (liveRef.current = live), [live]);
  /* TRYB BOTA W REFERENCJI — do komend, które muszą podać ADRES TRYBU (EA-2c).
     Świadomie `modeView` (tryb, w którym pracuje BOT), a nie lokalny `mode`
     z `modeRef` wyżej: ten drugi opisuje silnik projektowy w przeglądarce
     i przy podłączonym backendzie potrafi mówić co innego niż bot. Adres
     komendy ma opisywać stan, który serwer za chwilę porówna ze swoim.
     Referencja, a nie zależność `useCallback`: uchwyt `setDrabinka` trafia
     do kontekstu i przebudowywanie go przy każdej zmianie trybu przerysowałoby
     pół panelu, a jedyne, czego tu potrzeba, to ostatnia znana wartość. */
  const trybBotaRef = useRef(modeView);
  useEffect(() => void (trybBotaRef.current = modeView), [modeView]);
  const sendCmd = backend.send;

  const wyslij = useCallback(
    (cmd: { cmd: string } & Record<string, unknown>, opis: string): boolean => {
      if (!liveRef.current) return false;
      void sendCmd(cmd).then((r) => {
        if (!r.ok) toast("error", tSlownik("toast.rejected", { what: opis }), r.error ?? tSlownik("toast.noReason"));
      });
      return true;
    },
    [sendCmd, toast],
  );

  /* ---------------- HISTORIA RĘCZNYCH OPERACJI (TODO K16/K17) ----------------
     Cofnięcie idzie DOKŁADNIE tą samą drogą co każda inna akcja panelu —
     `wyslij` → `backend.send`. Osobny kanał do brokera byłby czwartym
     miejscem, w którym trzeba pamiętać o odmowach, opisach i logach. */
  const wykonajKomende = useCallback(
    (k: Komenda, opis: string, originAccountSession?: string) => {
      wyslij(originAccountSession === undefined ? k : { ...k, accountSession: originAccountSession }, opis);
    },
    [wyslij],
  );
  const accountIntentScope = settingsView.mt5_follow_terminal_account
    ? (B?.connection.accountSession ?? "") : undefined;
  const historia = useHistoriaOperacji(wykonajKomende, snapshotView, {
    follow: accountIntentScope !== undefined,
    sessionToken: accountIntentScope || null,
  });
  const zapiszHistorie = historia.zapisz;

  const login = useCallback(() => {
    setLoggedIn(true);
    lastTick.current = Date.now();
    toast("success", tSlownik("toast.tgIn.title"), tSlownik("toast.tgIn.text"));
  }, [toast]);

  const logout = useCallback(() => {
    setLoggedIn(false);
    if (liveRef.current) void api.authLogout().catch(() => undefined);
    toast("info", tSlownik("toast.tgOut.title"), tSlownik("toast.tgOut.text"));
  }, [toast]);

  const setMode = useCallback(
    (m: TradingMode) => {
      if (!wyslij({ cmd: "setMode", mode: m }, "Zmiana trybu")) setModeRaw(m);
      const label = m === "AI" ? "AI (BETAZERO)" : m;
      toast("info", tSlownik("mode.toast.title", { label }), tSlownik(`mode.toast.${m.toLowerCase()}`));
      addLog("commands", `Zmiana trybu → ${label}`, "", "info");
    },
    [addLog, toast, wyslij],
  );

  const patchSettingsRemote = backend.patchSettings;

  const setSetting = useCallback(
    <K extends keyof Settings>(key: K, value: Settings[K]) => {
      if (liveRef.current) {
        void patchSettingsRemote({ [key as string]: value }).then((r) => {
          if (!r.ok) toast("error", tSlownik("toast.settingRejected"), r.error ?? "");
        });
        return;
      }
      setSettingsState((s) => ({ ...s, [key]: value }));
    },
    [patchSettingsRemote, toast],
  );

  
  const dosypaneRef = useRef(false);
  useEffect(() => {
    if (!live || dosypaneRef.current || !serverSettings) return;
    const maSerwer = serverSettings as Record<string, unknown>;
    if (Object.keys(maSerwer).length < 50) {
      dosypaneRef.current = true;
      toast(
        "error",
        tSlownik("toast.settingsLost.title"),
        tSlownik("toast.settingsLost.text", { n: Object.keys(maSerwer).length }),
      );
      return;
    }
    const brakujace: Record<string, unknown> = {};
    for (const k of DOSYPYWANE_KLUCZE) {
      const domyslne = DEFAULT_SETTINGS as unknown as Record<string, unknown>;
      if (!(k in maSerwer) && k in domyslne) brakujace[k] = domyslne[k];
    }
    dosypaneRef.current = true;
    if (Object.keys(brakujace).length === 0) return;
    void patchSettingsRemote(brakujace).then((r) => {
      if (!r.ok) dosypaneRef.current = false;
    });
  }, [live, serverSettings, patchSettingsRemote]);

  const setSettingsPatch = useCallback(
    (patch: Partial<Settings>) => {
      if (liveRef.current) {
        void patchSettingsRemote(patch as Record<string, unknown>).then((r) => {
          if (!r.ok) toast("error", tSlownik("toast.settingsRejected"), r.error ?? "");
        });
        return;
      }
      setSettingsState((s) => ({ ...s, ...patch }));
    },
    [patchSettingsRemote, toast],
  );

  const resetSettings = useCallback(() => {
    if (!wyslij({ cmd: "resetSettings" }, "Przywrócenie ustawień")) {
      setSettingsState((s) => ({ ...preserveFollowTerminalAccount(s, DEFAULT_SETTINGS) } as Settings));
      setPresetId("");
    }
    toast("info", tSlownik("toast.settingsRestored.title"), tSlownik("toast.settingsRestored.text"));
  }, [toast, wyslij]);

  /* ---------------- presety ----------------
     Z podłączonym botem lista pochodzi z jego katalogu `presets/`. To NIE jest
     kosmetyka: `ApplyPreset` szuka presetu po nazwie właśnie tam i plik z dysku
     ma pierwszeństwo przed wartościami z UI. Panel pokazujący 29 nazw
     z prototypu obiecywał presety, których bot nie zna — kliknięcie takiego
     wczytywało wartości wbudowane w interfejs, a nie to, co użytkownik
     naprawdę ma w plikach. */
  const presetsFromDisk = live && serverPresets.length > 0;
  const presetsView = presetsFromDisk ? serverPresets : PRESETS;
  const presetsRef = useRef(presetsView);
  useEffect(() => void (presetsRef.current = presetsView), [presetsView]);

  const findPresetView = useCallback(
    (id: string): Preset | undefined =>
      presetsRef.current.find((p) => p.id === id || p.name.toLowerCase() === id.toLowerCase()) ?? findPreset(id),
    [],
  );

  useEffect(() => {
    if (!live) {
      setServerPresets([]);
      return;
    }
    let anulowane = false;
    api
      .presets()
      .then((lista) => {
        if (!anulowane) setServerPresets(lista.map(presetFromDisk));
      })
      .catch(() => {
        // Brak listy z dysku NIE jest powodem do pustego ekranu — panel
        // pokaże wtedy katalog wbudowany i powie, skąd pochodzi.
        if (!anulowane) setServerPresets([]);
      });
    return () => {
      anulowane = true;
    };
  }, [live]);

  const applyPreset = useCallback(
    (id: string) => {
      const p = findPresetView(id);
      if (!p) {
        toast("error", tSlownik("toast.noPreset.title"), tSlownik("toast.noPreset.text", { id }));
        return;
      }
      // Serwer szuka presetu w swoim katalogu `presets/`, a gdy go tam nie ma,
      // bierze wartości, które przysyła interfejs. Preset Z DYSKU wysyłamy
      // więc SAMĄ NAZWĄ: plik jest źródłem prawdy i nie ma powodu, żeby
      // przeglądarka odsyłała serwerowi jego własną treść.
      const zDysku = p.family === RODZINA_Z_DYSKU;
      if (!wyslij({ cmd: "applyPreset", id: p.name, values: zDysku ? null : p.values }, `Preset ${p.name}`)) {
        const { lot_mode, lot_percent, lot_fixed, ...rest } = p.values;
        setSettingsState((s) => ({ ...s, ...preserveFollowTerminalAccount(s, rest as Partial<Settings>) }));
        if (lot_mode === "percent") setLotState((l) => ({ ...l, mode: "percent", percent: lot_percent ?? l.percent }));
        else if (lot_mode === "fixed") setLotState((l) => ({ ...l, mode: "fixed", fixed: lot_fixed ?? l.fixed }));
        setPresetId(id);
      }
      /* Dziennik ZOSTAJE po polsku (decyzja z JEZYKI_SPEC.md), chmurka idzie
         w języku panelu — to dwa różne odbiorcze konteksty. */
      addLog("commands", `Wczytano preset ${p.name}`, p.tagline, "success");
      toast("success", tSlownik("toast.preset", { name: p.name }), opisPresetu(p.id, p.tagline));
    },
    [addLog, toast, wyslij],
  );

  const setLot = useCallback(
    (l: LotConfig) => {
      if (!wyslij({ cmd: "setLot", lot: l }, "Zmiana lota")) setLotState(l);
      addLog("commands", "Zmiana lota", l.mode === "fixed" ? `stały ${l.fixed}` : `${l.percent}% konta`, "info");
    },
    [addLog, wyslij],
  );

  /* ---------------- łańcuchy ----------------
     Zapisujemy CAŁY zbiór jednym komunikatem, a nie polami po kolei.
     Tworzenie, zmiana nazwy, usunięcie i edycja pułapów to operacje na
     LIŚCIE — łatka „zmień jedno pole" musiałaby nieść indeks albo starą
     nazwę, a przy dwóch otwartych oknach trafiłaby w nie ten wpis.
     Jeden zapis = jeden spójny stan, tak samo jak przy `setEmail`. */
  const setLancuchy = useCallback(
    (l: Lancuchy, aktywnyEa?: string) => {
      const czysty = normalizujLancuchy(l);
      /* POLE `aktywnyEa` LECI TYLKO WTEDY, GDY PANEL JE ZNA. Pominięcie
         znaczy po stronie serwera „nie ruszaj wskazania warstwy EA" — dzięki
         temu zapis zrobiony w trybie AUTO nie kasuje składu AUTO-EA. */
      const komenda =
        aktywnyEa === undefined
          ? { cmd: "setLancuchy", lancuchy: czysty }
          : { cmd: "setLancuchy", lancuchy: czysty, aktywnyEa };
      if (!wyslij(komenda, "Zapis łańcuchów")) {
        setLancuchyLokalne(czysty);
        if (aktywnyEa !== undefined) setAktywnyEaLokalny(aktywnyEa);
      }
      addLog(
        "commands",
        "Zapis łańcuchów",
        `${czysty.lista.length} · aktywny ${czysty.aktywny}${aktywnyEa ? ` · EA ${aktywnyEa}` : ""}`,
        "info",
      );
    },
    [addLog, wyslij],
  );

  /* PRZEŁĄCZENIE AKTYWNEGO ŁAŃCUCHA — POLE WYBIERA TRYB (projekt EA-2).
     Komenda niesie samą nazwę; to SERWER wie, czy zapisać ją do `aktywny`,
     czy do `aktywny_ea`. Panel nie podaje pola docelowego świadomie: dwie
     karty otwarte w dwóch trybach przepisywałyby sobie skład nawzajem,
     gdyby o miejscu zapisu decydowała strona wysyłająca.
     Gałąź offline musi rozstrzygać TAK SAMO — inaczej tryb projektowy
     zachowywałby się inaczej niż bot. */
  const setAktywnyLancuch = useCallback(
    (nazwa: string) => {
      if (!wyslij({ cmd: "setAktywnyLancuch", nazwa }, "Zmiana łańcucha")) {
        if (modeRef.current === "AUTO-EA") {
          setAktywnyEaLokalny((poprzedni) =>
            lancuchyLokalne.lista.some((l) => l.nazwa === nazwa) ? nazwa : poprzedni,
          );
        } else {
          setLancuchyLokalne((z) => (z.lista.some((l) => l.nazwa === nazwa) ? { ...z, aktywny: nazwa } : z));
        }
      }
      addLog("commands", `Aktywny łańcuch → ${nazwa}`, "", "info");
      toast("info", tSlownik("toast.chain.title", { name: nazwa }), tSlownik("toast.chain.text"));
    },
    [addLog, lancuchyLokalne.lista, toast, wyslij],
  );

  /* --- handel --- */
  const openManualOrder = useCallback(
    (a: { kind: "market" | "limit" | "stop"; direction: Direction; volume: number; price?: number; sl?: number; tp?: number }) => {
      const s = engine.current;
      const px = getSeries(s.symbol).lastPrice;
      // Wolumen ręcznego biletu: to, co wpisano w formularzu, a w razie
      // pustego pola LOT RĘCZNY z karty — nigdy suma lotów automatu.
      const vol = a.volume > 0 ? a.volume : lotReczny;
      // PUSTE POLE TO `null`, NIE `0`.
      //
      // Formularz zlecenia trzyma nieuzupełnione pola jako zero, a `?? null`
      // przepuszcza zero dalej (bo zero nie jest `nullish`). Do MT5 leciało
      // wtedy TP = 0.00, czyli dla BUY cel po niewłaściwej stronie ceny —
      // broker odrzucał każde ręczne zlecenie jako `InvalidStops`, a pozycja
      // nie powstawała. To jest ta „nie działa i nic nie mówi" część usterki.
      const opcjonalna = (x?: number): number | null =>
        typeof x === "number" && Number.isFinite(x) && x > 0 ? x : null;
      const cena = opcjonalna(a.price);
      if (a.kind !== "market" && cena === null) {
        toast("error", tSlownik("toast.noActivation.title"), tSlownik("toast.noActivation.text", { kind: a.kind.toUpperCase() }));
        return;
      }
      if (
        wyslij(
          {
            cmd: "openOrder",
            kind: a.kind,
            direction: a.direction,
            volume: vol,
            price: cena,
            sl: opcjonalna(a.sl),
            tp: opcjonalna(a.tp),
          },
          `${a.direction} ${vol} lot`,
        )
      ) {
        return;
      }
      if (a.kind === "market") {
        openPosition(s, null, a.direction, vol, px, a.sl || null, a.tp || null, 0, false, "manual");
        addLog("trades", `Ręczne wejście ${a.direction}`, `${vol} lot @ ${px.toFixed(2)}`, "success");
        toast("success", `${a.direction} ${vol} lot`, tSlownik("toast.market.text", { px: px.toFixed(2) }));
      } else {
        const p = a.price || px;
        const kind: PendingKind =
          a.kind === "limit"
            ? a.direction === "BUY"
              ? "BUY_LIMIT"
              : "SELL_LIMIT"
            : a.direction === "BUY"
              ? "BUY_STOP"
              : "SELL_STOP";
        addPending(s, null, kind, vol, p, a.sl || null, a.tp || null, 0, "manual");
        addLog("trades", `Pending ${kind}`, `${vol} lot @ ${p.toFixed(2)}`, "success");
        toast("success", `${kind.replace("_", " ")}`, `${vol} lot @ ${p.toFixed(2)}`);
      }
      publish();
    },
    [addLog, lotReczny, publish, toast, wyslij],
  );

  const closePos = useCallback(
    (ticket: number) => {
      /* ZAPIS BEZ MOŻLIWOŚCI COFNIĘCIA. Zamkniętej pozycji nie da się otworzyć
         z powrotem — „undo" byłoby nowym wejściem po innej cenie. Wartości
         zapisujemy mimo to: dziennik ma odpowiadać na pytanie „co ja zrobiłem
         o 14:07", a nie tylko na „co da się odkręcić". */
      const stara = migawkaRef.current.positions.find((x) => x.ticket === ticket);
      if (stara) {
        zapiszHistorie({
          rodzaj: "position",
          ticket,
          zmiany: [
            { pole: "wolumen", z: stara.volume, na: 0 },
            { pole: "cena", z: stara.openPrice, na: stara.openPrice },
            { pole: "sl", z: stara.sl, na: stara.sl },
            { pole: "tp", z: stara.tp, na: stara.tp },
          ],
          komenda: null,
          odwrotna: null,
          powod: "undo.reason.close",
        });
      }
      if (wyslij({ cmd: "closePosition", ticket }, `Zamknięcie #${ticket}`)) return;
      const s = engine.current;
      const price = getSeries(s.symbol).lastPrice;
      const profit = closePosition(s, ticket, price, "MANUAL");
      addLog("trades", `Zamknięto #${ticket}`, `${profit >= 0 ? "+" : ""}${profit.toFixed(2)} $`, profit >= 0 ? "success" : "warn");
      toast(profit >= 0 ? "success" : "warn", tSlownik("toast.posClosed", { t: ticket }), `${profit >= 0 ? "+" : ""}$${profit.toFixed(2)}`);
      publish();
    },
    [addLog, publish, toast, wyslij, zapiszHistorie],
  );

  // Zamknięcie CZĘŚCI pozycji. Bez podłączonego bota tylko meldunek — lokalny
  // silnik podglądowy nie prowadzi księgi wolumenu z dokładnością brokera,
  // a udawanie tutaj częściowego zamknięcia dałoby liczby, których na
  // rachunku nie ma.
  const closePartial = useCallback(
    (ticket: number, volume: number) => {
      const stara = migawkaRef.current.positions.find((x) => x.ticket === ticket);
      if (stara) {
        zapiszHistorie({
          rodzaj: "position",
          ticket,
          zmiany: [{ pole: "wolumen", z: stara.volume, na: Math.max(0, stara.volume - volume) }],
          komenda: null,
          odwrotna: null,
          powod: "undo.reason.close",
        });
      }
      if (wyslij({ cmd: "closePartial", ticket, volume }, `Zamknięcie ${volume} lota z #${ticket}`)) return;
      toast("warn", tSlownik("toast.noBot.title"), tSlownik("toast.noBot.text"));
    },
    [toast, wyslij, zapiszHistorie],
  );

  const modifyPos = useCallback(
    (ticket: number, sl: number | null, tp: number | null) => {
      const stara = migawkaRef.current.positions.find((x) => x.ticket === ticket);
      if (stara) {
        zapiszHistorie({
          rodzaj: "position",
          ticket,
          zmiany: [
            { pole: "sl", z: stara.sl, na: sl },
            { pole: "tp", z: stara.tp, na: tp },
          ],
          komenda: { cmd: "modifyPosition", ticket, sl, tp },
          odwrotna: liveRef.current ? { cmd: "modifyPosition", ticket, sl: stara.sl, tp: stara.tp } : null,
          powod: liveRef.current ? undefined : "undo.reason.offline",
        });
      }
      if (wyslij({ cmd: "modifyPosition", ticket, sl, tp }, `Modyfikacja #${ticket}`)) return;
      const p = engine.current.positions.find((x) => x.ticket === ticket);
      if (!p) return;
      p.sl = sl;
      p.tp = tp;
      p.frozen = true; // reczna edycja ZAWSZE zamraza pozycje (jak w bot.py)
      addLog("trades", `Modyfikacja #${ticket}`, `SL ${sl ?? "—"} · TP ${tp ?? "—"} · pozycja zamrożona`, "info");
      toast("info", tSlownik("toast.posUpdated.title", { t: ticket }), tSlownik("toast.posUpdated.text"));
      publish();
    },
    [addLog, publish, toast, wyslij, zapiszHistorie],
  );

  const closeBulk = useCallback(
    (which: "all" | "profit" | "loss") => {
      zapiszHistorie({
        rodzaj: "position",
        ticket: null,
        opis: `closeBulk:${which} · ${migawkaRef.current.positions.length}`,
        zmiany: [],
        komenda: null,
        odwrotna: null,
        powod: "undo.reason.bulk",
      });
      if (wyslij({ cmd: "closeBulk", which }, "Zamknięcie zbiorcze")) return;
      const s = engine.current;
      const price = getSeries(s.symbol).lastPrice;
      const targets = s.positions.filter((p) => {
        const pr = positionProfit(p, price);
        return which === "all" || (which === "profit" ? pr > 0 : pr < 0);
      });
      let total = 0;
      for (const p of targets) total += closePosition(s, p.ticket, price, "MANUAL");
      addLog("commands", `Zamknięto ${targets.length} pozycji (${which})`, `${total >= 0 ? "+" : ""}${total.toFixed(2)} $`, "info");
      toast(total >= 0 ? "success" : "warn", tSlownik("toast.bulkClosed", { n: targets.length }), `${total >= 0 ? "+" : ""}$${total.toFixed(2)}`);
      publish();
    },
    [addLog, publish, toast, wyslij, zapiszHistorie],
  );

  const delPending = useCallback(
    (ticket: number) => {
      const stare = migawkaRef.current.pendings.find((x) => x.ticket === ticket);
      if (stare) {
        zapiszHistorie({
          rodzaj: "pending",
          ticket,
          opis: stare.kind.replace("_", " "),
          zmiany: [
            { pole: "cena", z: stare.price, na: null },
            { pole: "sl", z: stare.sl, na: null },
            { pole: "tp", z: stare.tp, na: null },
            { pole: "wolumen", z: stare.volume, na: null },
          ],
          komenda: null,
          odwrotna: null,
          powod: "undo.reason.delete",
        });
      }
      if (wyslij({ cmd: "deletePending", ticket }, `Usunięcie pendinga #${ticket}`)) return;
      deletePending(engine.current, ticket, "CANCELLED");
      addLog("trades", `Usunięto pending #${ticket}`, "", "info");
      publish();
    },
    [addLog, publish, wyslij, zapiszHistorie],
  );

  const modifyPending = useCallback(
    (ticket: number, price: number, sl: number | null, tp: number | null) => {
      
      const stare = migawkaRef.current.pendings.find((x) => x.ticket === ticket);
      if (stare) {
        const zmiany: Zmiana[] = [
          { pole: "cena", z: stare.price, na: price },
          { pole: "sl", z: stare.sl, na: sl },
          { pole: "tp", z: stare.tp, na: tp },
        ];
        zapiszHistorie({
          rodzaj: "pending",
          ticket,
          zmiany,
          komenda: { cmd: "modifyPending", ticket, price, sl, tp },
          // Bez bota nie ma komu wysłać cofnięcia — lokalny silnik podglądowy
          // nie jest rachunkiem, więc „cofnij" nie miałoby czego dotknąć.
          odwrotna: liveRef.current
            ? { cmd: "modifyPending", ticket, price: stare.price, sl: stare.sl, tp: stare.tp }
            : null,
          powod: liveRef.current ? undefined : "undo.reason.offline",
        });
      }
      if (wyslij({ cmd: "modifyPending", ticket, price, sl, tp }, `Modyfikacja pendinga #${ticket}`)) return;
      const o = engine.current.pendings.find((x) => x.ticket === ticket);
      if (!o) return;
      o.price = price;
      o.sl = sl;
      o.tp = tp;
      o.frozen = true;
      addLog("trades", `Modyfikacja pendinga #${ticket}`, `cena ${price} · SL ${sl ?? "—"} · TP ${tp ?? "—"}`, "info");
      toast("info", tSlownik("toast.pendUpdated.title", { t: ticket }), tSlownik("toast.pendUpdated.text"));
      publish();
    },
    [addLog, publish, toast, wyslij, zapiszHistorie],
  );

  const delAllPendings = useCallback(() => {
    zapiszHistorie({
      rodzaj: "pending",
      ticket: null,
      opis: `deleteAllPendings · ${migawkaRef.current.pendings.length}`,
      zmiany: [],
      komenda: null,
      odwrotna: null,
      powod: "undo.reason.delete",
    });
    if (wyslij({ cmd: "deleteAllPendings" }, "Usunięcie wszystkich pendingów")) return;
    const s = engine.current;
    const n = s.pendings.length;
    for (const o of [...s.pendings]) deletePending(s, o.ticket, "CANCELLED");
    addLog("commands", `Usunięto wszystkie pendingi (${n})`, "", "warn");
    toast("warn", tSlownik("toast.pendDeleted", { n }));
    publish();
  }, [addLog, publish, toast, wyslij, zapiszHistorie]);

  const closeBasketById = useCallback(
    (id: number) => {
      const koszyk = migawkaRef.current.baskets.find((x) => x.id === id);
      zapiszHistorie({
        rodzaj: "basket",
        ticket: id,
        opis: `B${id} · ${koszyk?.tickets.length ?? 0}`,
        zmiany: koszyk ? [{ pole: "sl", z: koszyk.sl, na: null }] : [],
        komenda: null,
        odwrotna: null,
        powod: "undo.reason.close",
      });
      if (wyslij({ cmd: "closeBasket", id }, `Zamknięcie koszyka B${id}`)) return;
      const s = engine.current;
      const b = s.baskets.find((x) => x.id === id);
      if (!b) return;
      engineCloseBasket(s, b, getSeries(s.symbol).lastPrice);
      addLog("commands", `Zamknięto koszyk B${id}`, "", "warn");
      toast("warn", tSlownik("toast.basketClosed", { id }));
      publish();
    },
    [addLog, publish, toast, wyslij, zapiszHistorie],
  );

  const updateBasket = useCallback(
    (id: number, patch: { sl?: number | null; zoneLow?: number; zoneHigh?: number; tps?: number[] }) => {
      if (wyslij({ cmd: "updateBasket", id, patch }, `Zapis koszyka B${id}`)) return;
      const b = engine.current.baskets.find((x) => x.id === id);
      if (!b) return;
      if (patch.sl !== undefined) {
        b.sl = patch.sl;
        for (const p of engine.current.positions.filter((p) => p.basketId === id && !p.frozen)) p.sl = patch.sl!;
      }
      if (patch.zoneLow !== undefined) b.zoneLow = patch.zoneLow;
      if (patch.zoneHigh !== undefined) b.zoneHigh = patch.zoneHigh;
      if (patch.tps) b.tps = patch.tps;
      addLog("commands", `Aktualizacja koszyka B${id}`, JSON.stringify(patch), "info");
      toast("success", tSlownik("toast.basketSaved", { id }));
      publish();
    },
    [addLog, publish, toast, wyslij],
  );

  /* --- telegram --- */

  /** Pobiera z bota PRAWDZIWĄ listę czatów konta.
   *
   *  Dopóki jej nie ma, ekran „Kanały" nie pozwala niczego zaznaczyć — i to
   *  jest właściwe zachowanie. Wcześniej pokazywał trzy wymyślone kanały
   *  z prototypu; ich identyfikatory nie odpowiadały żadnej rozmowie, więc
   *  bot nigdy nie rozpoznawał źródła wiadomości i po cichu nie handlował. */
  const refreshChannels = useCallback(() => {
    if (!liveRef.current) {
      setChannelsError(null);
      return;
    }
    setChannelsLoading(true);
    api
      .channels()
      .then((r) => {
        setChannelsError(r.channelsError ?? null);
        setServerChannels(
          (r.channels ?? []).map((c, i) => ({
            id: c.id,
            name: c.name || String(c.id),
            handle: c.handle ? `@${c.handle}` : "",
            members: 0,
            // Telegram nie daje koloru awatara; barwę wyprowadzamy
            // deterministycznie z identyfikatora, żeby ten sam kanał miał
            // zawsze ten sam kolor.
            avatarHue: Math.abs(Number(BigInt(c.id) % 360n)) || (i * 47) % 360,
            // Adres budujemy TYLKO dla kanałów, które zdjęcie naprawdę mają —
            // inaczej połowa listy strzelałaby w 404. `?v=` to wersja zdjęcia
            // z Telegrama: podmiana obrazka zmienia adres, więc przeglądarka
            // pobiera nowy zamiast pokazywać zapamiętany stary.
            photoUrl: c.hasPhoto
              ? `${backendBase()}/api/channels/${c.id}/photo${c.photoVersion ? `?v=${encodeURIComponent(c.photoVersion)}` : ""}`
              : undefined,
            isForum: c.isForum,
            kind: c.kind,
            verified: false,
            topics: (c.topics ?? []).map((t) => ({
              id: t.id,
              name: t.title,
              icon: t.closed ? "🔒" : "#",
            })),
            lastMessage: "",
            lastMessageTime: 0,
          })),
        );
      })
      .catch((e: unknown) => setChannelsError(e instanceof Error ? e.message : String(e)))
      .finally(() => setChannelsLoading(false));
  }, []);

  const setBinding = useCallback(
    (channelId: number, patch: Partial<ChannelBinding>) => {
      
      const naDrut: Record<string, unknown> = { ...patch };
      if (patch.format !== undefined) naDrut.formats = patch.format ? [patch.format] : [];
      if (wyslij({ cmd: "setBinding", channelId, patch: naDrut }, "Zapis kanału")) return;
      setBindings((b) => ({
        ...b,
        [channelId]: normalizujBinding({ ...b[channelId], ...patch }, channelId),
      }));
    },
    [wyslij],
  );

  
  const kanalWstrzykniecia = useCallback((): { id: number; name: string } => {
    const zSerwera = Object.entries((live ? B?.bindings : null) ?? {})
      .map(([k, v]) => ({ id: Number(k), b: v as Partial<ChannelBinding> }))
      .find(
        ({ b }) =>
          b?.monitored !== false &&
          ((b?.format ?? "") !== "" || Object.values(b?.topics ?? {}).some((f) => f !== "")),
      );
    if (zSerwera) {
      const nazwa = serverChannels.find((c) => c.id === zSerwera.id)?.name ?? String(zSerwera.id);
      return { id: zSerwera.id, name: nazwa };
    }
    return { id: CHANNELS[0].id, name: CHANNELS[0].name };
  }, [live, B?.bindings, serverChannels]);

  const simulateMessage = useCallback(
    (text: string, channelId?: number, opcje?: WstrzykniecieOpcje) => {
      const jawny = channelId !== undefined ? (live ? serverChannels : CHANNELS).find((c) => c.id === channelId) : undefined;
      const ch = jawny ?? (channelId !== undefined ? { id: channelId, name: String(channelId) } : kanalWstrzykniecia());
      // POLE POMINIĘTE MA ZNIKNĄĆ Z ŁADUNKU, a nie pojechać jako `undefined`
      // czy `0`. Na tym stoi kontrakt zera: serwer odróżnia „nie podano"
      // od „podano zero", bo to są dwie różne wiadomości.
      const ladunek: Record<string, unknown> = { cmd: "simulateMessage", text, channelId: ch.id };
      for (const [k, v] of Object.entries(opcje ?? {})) {
        if (Number.isFinite(v)) ladunek[k] = v;
      }
      if (!wyslij(ladunek as { cmd: string } & Record<string, unknown>, "Symulacja wiadomości")) {
        ingest({ channelId: ch.id, channelName: ch.name, text }, { simulated: true });
      }
      toast("info", tSlownik("toast.msgProcessed.title"), tSlownik("toast.msgProcessed.text"));
    },
    [ingest, toast, wyslij, kanalWstrzykniecia, live, serverChannels],
  );

  const executeMessage = useCallback(
    (id: string) => {
      if (wyslij({ cmd: "executeMessage", id }, "Wykonanie sygnału")) return;
      setMessages((ms) => {
        const m = ms.find((x) => x.id === id);
        if (m?.parsed) {
          const sourceKey = `${m.channelId}${m.topicName ? `:${m.topicName}` : ""}`;
          applyParsed(m.parsed, sourceKey, m.channelName, m.text);
          publish();
        }
        return ms.map((x) => (x.id === id ? { ...x, pendingAction: "executed" as const } : x));
      });
      toast("success", tSlownik("toast.signalExec.title"), tSlownik("toast.signalExec.text"));
    },
    [applyParsed, publish, toast, wyslij],
  );

  const dismissMessage = useCallback(
    (id: string) => {
      if (wyslij({ cmd: "dismissMessage", id }, "Odrzucenie sygnału")) return;
      setMessages((ms) => ms.map((x) => (x.id === id ? { ...x, pendingAction: "dismissed" as const } : x)));
    },
    [wyslij],
  );

  /* --- logi --- */
  const clearLogs = useCallback(() => {
    if (!wyslij({ cmd: "clearLogs" }, "Czyszczenie logów")) setLogs([]);
    toast("info", tSlownik("toast.logsCleared"));
  }, [toast, wyslij]);

  const logsView = live ? B!.logs : logs;
  const mergedLogCount = useMemo(
    () => logsView.filter((l) => settingsView.merge_config?.[l.category]).length,
    [logsView, settingsView.merge_config],
  );

  /* --- symulacje --- */
  const addSim = useCallback(
    (preset: string, name: string, balance: number, simLot: number, mode?: TradingMode) => {
      const p = findPresetView(preset) ?? PRESETS[0];
      // `mode` idzie po drucie TYLKO, gdy użytkownik tryb wybrał — brak pola
      // to kontrakt zera serwera (dziedziczenie trybu głównego bota).
      if (wyslij({ cmd: "addSim", preset: p.id, name, balance, lot: simLot, ...(mode ? { mode } : {}) }, "Dodanie instancji")) {
        toast("success", tSlownik("toast.simAdded.title"), `${p.name} · $${balance} · ${simLot} lot`);
        return;
      }
      const id = `sim${Date.now()}`;
      const curve = Array.from({ length: 40 }, (_, i) => balance * (1 + (p.metrics.monthly / 2000) * (i / 40) * Math.random()));
      setSims((s) => [
        ...s,
        {
          id,
          name: name || `${p.id}-${s.length + 1}`,
          preset: p.id,
          ...(mode ? { mode } : {}),
          balance,
          startBalance: balance,
          equity: balance,
          lot: simLot,
          positions: 0,
          pendings: 0,
          baskets: 0,
          trades: 0,
          winRate: p.metrics.winDays,
          maxDd: 0,
          createdAt: Date.now(),
          curve,
        },
      ]);
      toast("success", tSlownik("toast.simAdded.title"), `${p.name} · $${balance} · ${simLot} lot`);
    },
    [findPresetView, toast, wyslij],
  );

  const removeSim = useCallback(
    (id: string) => {
      if (wyslij({ cmd: "removeSim", id }, "Usunięcie instancji")) return;
      setSims((s) => s.filter((x) => x.id !== id));
    },
    [wyslij],
  );

  const resetSim = useCallback((id: string) => {
    if (wyslij({ cmd: "resetSim", id }, "Reset instancji")) return;
    setSims((s) =>
      s.map((x) =>
        x.id === id
          ? { ...x, balance: x.startBalance, equity: x.startBalance, positions: 0, pendings: 0, baskets: 0, trades: 0, maxDd: 0 }
          : x,
      ),
    );
  }, [wyslij]);

  /* --- symulacja postępu instancji (tylko tryb demo) --- */
  useEffect(() => {
    if (!loggedIn || live || sims.length === 0) return;
    const h = window.setInterval(() => {
      setSims((list) =>
        list.map((s) => {
          const p = findPreset(s.preset);
          const bias = ((p?.metrics.winDays ?? 60) - 50) / 900;
          const delta = (Math.random() - 0.5 + bias) * s.startBalance * 0.006;
          const equity = Math.max(s.startBalance * 0.2, s.equity + delta);
          const dd = Math.max(s.maxDd, Math.max(0, Math.max(...s.curve, s.startBalance) - equity));
          return {
            ...s,
            equity,
            balance: equity - delta * 0.4,
            maxDd: dd,
            trades: s.trades + (Math.random() < 0.28 ? 1 : 0),
            positions: Math.max(0, Math.round(s.positions + (Math.random() < 0.3 ? (Math.random() < 0.5 ? 1 : -1) : 0))),
            pendings: Math.max(0, Math.round(s.pendings + (Math.random() < 0.3 ? (Math.random() < 0.5 ? 2 : -2) : 0))),
            baskets: Math.max(0, Math.round(s.baskets + (Math.random() < 0.12 ? (Math.random() < 0.6 ? 1 : -1) : 0))),
            curve: [...s.curve, equity].slice(-60),
          };
        }),
      );
    }, 2400);
    return () => window.clearInterval(h);
  }, [loggedIn, sims.length]);

  /**
   * Wznowienie handlu MIMO przekroczonego limitu ryzyka — świadoma decyzja
   * użytkownika. Poza zdjęciem blokady:
   *  - uzbraja `riskOverride`, żeby strażnik nie zatrzymał handlu ponownie na
   *    tym samym, wciąż spełnionym warunku,
   *  - przestawia szczyt equity na wartość bieżącą, więc obsunięcie liczy się
   *    od nowa (inaczej licznik startowałby od razu z przekroczonym progiem).
   */
  const resetHalt = useCallback(() => {
    if (wyslij({ cmd: "resumeTrading" }, "Wznowienie handlu")) {
      toast("warn", tSlownik("toast.resumed.title"), tSlownik("toast.resumed.text"));
      return;
    }
    const reason = haltRef.current.reason;
    haltRef.current = { active: false, reason: "" };
    setHalt({ active: false, reason: "" });
    setRiskOverride({ active: true, since: Date.now(), reason });
    overrideRef.current = { active: true, since: Date.now(), reason };

    const eq = statsRef.current.equity;
    statsRef.current = { ...statsRef.current, peakEquityToday: eq, drawdownNow: 0 };
    setStats((s) => ({ ...s, peakEquityToday: eq, drawdownNow: 0 }));

    addLog(
      "commands",
      "Wznowiono handel mimo przekroczonego limitu",
      `${reason} · strażnik wyłączony na własną odpowiedzialność, licznik obsunięcia wyzerowany`,
      "warn",
    );
    toast("warn", tSlownik("toast.resumed.title"), tSlownik("toast.resumed.text"));
  }, [addLog, toast, wyslij]);

  /** Ponowne uzbrojenie strażnika ryzyka. */
  const clearRiskOverride = useCallback(() => {
    if (wyslij({ cmd: "rearmGuard" }, "Uzbrojenie strażnika")) {
      toast("success", tSlownik("toast.guardOn.title"), tSlownik("toast.guardOn.text"));
      return;
    }
    setRiskOverride({ active: false, since: 0, reason: "" });
    overrideRef.current = { active: false, since: 0, reason: "" };
    const eq = statsRef.current.equity;
    statsRef.current = { ...statsRef.current, peakEquityToday: eq, drawdownNow: 0 };
    setStats((s) => ({ ...s, peakEquityToday: eq, drawdownNow: 0 }));
    addLog("commands", "Strażnik ryzyka uzbrojony ponownie", "", "success");
    toast("success", tSlownik("toast.guardOn.title"), tSlownik("toast.guardOn.text"));
  }, [addLog, toast, wyslij]);

  const toggleFavorite = useCallback(
    (symbol: string) => {
      if (wyslij({ cmd: "toggleFavorite", symbol }, "Zmiana ulubionych")) return;
      setFavorites((f) => (f.includes(symbol) ? f.filter((x) => x !== symbol) : [...f, symbol]));
    },
    [wyslij],
  );

  const setEmail = useCallback(
    (e: EmailConfig) => {
      if (wyslij({ cmd: "setEmail", email: e }, "Zapis poczty")) return;
      setEmailState(e);
    },
    [wyslij],
  );

  const setNotify = useCallback(
    (n: NotifyConfig) => {
      if (wyslij({ cmd: "setNotify", notify: n }, "Zapis powiadomień")) return;
      setNotifyState(n);
    },
    [wyslij],
  );

  /* ---------------- powłoka natywna + logowanie ---------------- */
  const openInBrowser = useCallback(() => {
    void api
      .openInBrowser()
      .catch(() => window.open(backend.base, "_blank", "noopener"));
  }, [backend.base]);

  const startQrLogin = useCallback(async (): Promise<AuthState | null> => {
    try {
      return await api.authStartQr();
    } catch (e) {
      toast("error", tSlownik("toast.qrFailed"), String(e));
      return null;
    }
  }, [toast]);

  const submit2fa = useCallback(
    async (password: string): Promise<AuthState | null> => {
      try {
        return await api.authSubmit2fa(password);
      } catch (e) {
        return { stage: "error", error: String(e).replace(/^Error:\s*/, "") };
      }
    },
    [],
  );

  const setTelegramCredentials = useCallback(
    async (apiId: string, apiHash: string): Promise<AuthState | null> => {
      try {
        // Serwer podnosi klienta MTProto i od razu oddaje pierwszy kod QR,
        // więc formularz nie musi wołać `startQrLogin` osobno.
        return await api.authSetCredentials(apiId.trim(), apiHash.trim());
      } catch (e) {
        return { stage: "error", error: String(e).replace(/^Error:\s*/, "") };
      }
    },
    [],
  );

  const forgetTelegramCredentials = useCallback(async (): Promise<AuthState | null> => {
    try {
      const s = await api.authForgetCredentials();
      toast("info", tSlownik("toast.credsCleared.title"), tSlownik("toast.credsCleared.text"));
      return s;
    } catch (e) {
      toast("error", tSlownik("toast.credsClearFailed"), String(e));
      return null;
    }
  }, [toast]);

  const sendTestEmail = useCallback(async () => {
    if (!liveRef.current) {
      toast("warn", tSlownik("toast.noBackend"), tSlownik("toast.noBackend.mail"));
      return;
    }
    try {
      const r = await api.emailTest();
      toast("success", tSlownik("toast.mailSent"), r.detail);
    } catch (e) {
      // Treść błędu SMTP jest tu najcenniejsza (zła nazwa serwera, odrzucone
      // hasło aplikacji, zablokowany port) — pokazujemy ją w całości.
      toast("error", tSlownik("toast.mailFailed"), String(e).replace(/^Error:\s*/, ""));
    }
  }, [toast]);

  
  const scalLogi = useCallback(async () => {
    if (!liveRef.current) {
      toast("warn", tSlownik("toast.noBackend"), tSlownik("toast.noBackend.merge"));
      return;
    }
    try {
      await api.mergeLogs();
    } catch (e) {
      toast("error", tSlownik("toast.mergeFailed"), String(e).replace(/^Error:\s*/, ""));
    }
  }, [toast]);

  /** Wysyła PRAWDZIWE powiadomienie testowe na zaznaczone kanały Telegrama. */
  const wyslijTestPowiadomienia = useCallback(() => {
    if (!liveRef.current) {
      toast("warn", tSlownik("toast.noBackend"), tSlownik("toast.noBackend.notify"));
      return;
    }
    // Odpowiedź serwera niesie powód odmowy (brak odbiorców, usługa Telegrama
    // nie działa) — bez niej przycisk znów byłby ozdobą.
    wyslij({ cmd: "sendTestNotify" }, "Wiadomość testowa");
  }, [toast, wyslij]);

  /* Gdy backend zgłosi udane logowanie do Telegrama, terminal otwiera się sam —
     także w drugiej powłoce, która logowania nie inicjowała. */
  /** Zapisuje drabinkę ŁAŃCUCHÓW. Wykonuje ją SILNIK — panel tylko ustawia.
   *
   *  KOMENDA NIESIE ADRES TRYBU (projekt EA-2c). Bez niego serwer musiałby
   *  zgadywać, której drabinki dotyczy zapis, a karta otwarta w jednym trybie
   *  potrafi wysłać komendę już po przełączeniu bota w drugi — i wtedy
   *  „zgadywanie" znaczy ustawienie cudzego składu. Z jawnym adresem taka
   *  komenda dostaje ODMOWĘ z wpisem w dzienniku, zamiast zadziałać nie tam,
   *  gdzie użytkownik patrzył. */
  const setDrabinka = useCallback(
    (d: DrabinkaLancuchow) => {
      wyslij(
        { cmd: "setDrabinka", drabinka: d, tryb: trybBotaRef.current },
        "Zmiana drabinki łańcuchów",
      );
    },
    [wyslij],
  );

  const otworzLogowanieTg = useCallback(() => setPokazLogowanieTg(true), []);
  const zamknijLogowanieTg = useCallback(() => setPokazLogowanieTg(false), []);
  const wejdzBezTelegrama = useCallback(() => {
    setLoggedIn(true);
    setPokazLogowanieTg(false);
  }, []);

  const authStage = B?.auth?.stage;
  useEffect(() => {
    if (authStage === "loggedIn") {
      setLoggedIn(true);
      // Ekran otwarty ręcznie ma się zamknąć SAM po udanym logowaniu —
      // inaczej użytkownik zostaje na kodzie QR, który już zadziałał.
      setPokazLogowanieTg(false);
    }
  }, [authStage]);

  /* Lista czatów ma sens dopiero po zalogowaniu do Telegrama — i wtedy trzeba
     ją pobrać SAMEMU, bez czekania, aż użytkownik trafi na zakładkę „Kanały".
     Bez zaznaczonego kanału bot nie wykonuje żadnego sygnału, więc ten ekran
     musi być gotowy w chwili, w której ktoś na niego wejdzie. */
  useEffect(() => {
    if (live && authStage === "loggedIn") refreshChannels();
  }, [live, authStage, refreshChannels]);

  
  const serwerowyMotyw = B?.settings?.[KLUCZ_MOTYWU] as string | undefined;
  const serwerowaPaleta = B?.settings?.[KLUCZ_PALETY] as string | undefined;
  useEffect(() => {
    applyServerAppearance(serwerowyMotyw, serwerowaPaleta);
  }, [serwerowyMotyw, serwerowaPaleta]);

  
  const serwerowyJezyk = B?.language;
  useEffect(() => {
    applyServerLanguage(serwerowyJezyk);
  }, [serwerowyJezyk]);

  /* Toast po przełączeniu języka (skrót L ALBO selektor w „Wygląd" — jedna
     ścieżka, jedno zdarzenie). Tylko zmiana użytkownika; echo z serwera przy
     starcie nie ma prawa strzelać chmurką. */
  useEffect(() => {
    const h = (e: Event) => {
      const d = (e as CustomEvent).detail as { language: Jezyk; source: string } | undefined;
      if (d?.source !== "user") return;
      const wpis = LANGUAGES.find((x) => x.id === d.language);
      if (wpis) toast("info", `${wpis.flag} ${wpis.native}`, tSlownik("lang.switched"));
    };
    window.addEventListener("conduit:language", h);
    return () => window.removeEventListener("conduit:language", h);
  }, [toast]);

  /* ---------------- polaczenie ---------------- */
  const demoConnection = useMemo<ConnectionState>(
    () => ({
      telegram: loggedIn ? "connected" : "disconnected",
      mt5: loggedIn ? "connected" : "disconnected",
      account: {
        login: 1_000_001,
        server: "Broker-Demo",
        broker: "Example Broker",
        currency: "USD",
        leverage: 500,
        type: "DEMO",
      },
      user: { name: "Demo User", handle: "@demo_user", phone: "+00 •••• ••• 000" },
      latencyMs: 18 + Math.round(Math.sin(Date.now() / 9000) * 6),
    }),
    [loggedIn],
  );

  /* ================= WIDOK: SERWER ALBO SYMULACJA =================
     Poniżej powstaje jedyne miejsce w całej aplikacji, które decyduje,
     skąd biorą się dane. Komponenty tego nie widzą — dostają ten sam
     `AppContextValue` co zawsze. */
  const connection: ConnectionState = live
    ? { ...B!.connection, latencyMs: backend.latencyMs || B!.connection.latencyMs }
    : demoConnection;

  const quotesView = live ? B!.quotes : quotes;
  
  // AUTO remains empty in settings: only the bound runtime symbol identifies
  // the bot instrument. Persisting it would pin the next broker to this one.
  const symbolBota = resolveBotSymbol(live, connection, settingsView.mt5_symbol || "", PRIMARY_SYMBOL);
  const primary = live
    ? runtimeQuote(symbolBota, quotesView)
    : (quotes[symbolBota] ?? getQuote(symbolBota));

  /* Kanały: serwer trzyma klucze jako napisy (wymóg JSON-a), UI jako liczby.
     Nakładamy je na komplet domyślnych, żeby kanał znany UI, a nieznany
     serwerowi, nie zniknął z listy. */
  const bindingsView: Record<number, ChannelBinding> = useMemo(() => {
    if (!live) return bindings;
    const out: Record<number, ChannelBinding> = { ...initialBindings() };
    for (const [k, v] of Object.entries(B!.bindings ?? {})) out[Number(k)] = normalizujBinding(v, Number(k));
    return out;
  }, [live, bindings, B?.bindings]);

  /* Formaty i łańcuchy: sekcje OPCJONALNE w migawce. Gdy ich nie ma (starsza
     binarka albo brak backendu), obowiązuje katalog wbudowany i stan lokalny.
     Pusta lista formatów NIE jest tu poprawnym stanem — znaczyłaby „nic nie
     handluje" i wyglądała jak konfiguracja, a jest brakiem danych. */
  const lancuchyZSerwera = live && !!B?.lancuchy;
  const formatyView: Format[] = useMemo(
    () => (live && B?.formaty?.length ? B.formaty : FORMATY_WBUDOWANE),
    [live, B?.formaty],
  );
  const lancuchyView: Lancuchy = useMemo(
    () => (lancuchyZSerwera ? normalizujLancuchy(B!.lancuchy) : lancuchyLokalne),
    [lancuchyZSerwera, B?.lancuchy, lancuchyLokalne],
  );
  /* WSKAŹNIK AUTO-EA (projekt EA-2): z serwera, gdy sekcja przyszła; lokalnie
     w trybie projektowym. Starsza binarka pola nie odsyła — wtedy pustka,
     czyli AUTO-EA gra tym samym, co reszta trybów (kontrakt zera). */
  const aktywnyEaView = lancuchyZSerwera ? (B!.aktywnyEa ?? "") : aktywnyEaLokalny;
  const aktywnaNazwaLancucha = useMemo(
    () => aktywnyDla(lancuchyView, aktywnyEaView, modeView),
    [lancuchyView, aktywnyEaView, modeView],
  );
  /* Łańcuch bieżącego TRYBU, nie `lancuchy.aktywny`. Cały panel czyta stąd —
     dzięki temu przełączenie na AUTO-EA zmienia to, co widać, dokładnie tam,
     gdzie zmienia to, czym gra bot. */
  const lancuchView = useMemo(
    () => lancuchDla(lancuchyView, aktywnyEaView, modeView),
    [lancuchyView, aktywnyEaView, modeView],
  );
  
  const drabinkaView = useMemo(
    () =>
      drabinkaDla(
        (live ? B!.drabinka : null) ?? PUSTA_DRABINKA,
        (live ? B!.drabinkaEa : null) ?? PUSTA_DRABINKA_EA,
        modeView,
      ),
    [live, B?.drabinka, B?.drabinkaEa, modeView],
  );

  const value: AppContextValue = {
    loggedIn,
    telegramZalogowany: authStage === "loggedIn",
    pokazLogowanieTg,
    otworzLogowanieTg,
    wejdzBezTelegrama,
    zamknijLogowanieTg,
    login,
    logout,
    connection,
    mode: modeView,
    setMode,
    settings: settingsView,
    effectiveSettings,
    setSetting,
    setSettings: setSettingsPatch,
    resetSettings,
    presetId: presetView,
    applyPreset,
    formaty: formatyView,
    lancuchy: lancuchyView,
    aktywnyEa: aktywnyEaView,
    aktywnaNazwaLancucha,
    lancuch: lancuchView,
    setLancuchy,
    setAktywnyLancuch,
    lancuchyZSerwera,
    pieczecLancuch: (live ? B!.pieczecLancuch : "") ?? "",
    zadanieEa,
    pokazParametryEa: (preset: string) => setZadanieEa({ preset }),
    wyczyscZadanieEa: () => setZadanieEa(null),
    presets: presetsView,
    findPreset: findPresetView,
    presetsFromDisk,
    lot: lotView,
    setLot,
    lotSize,
    ustawieniaNog,
    lotReczny,
    quotes: quotesView,
    primary,
    snapshot: snapshotView,
    stats: live ? B!.stats : stats,
    halt: live ? B!.halt : halt,
    resetHalt,
    riskOverride: live ? B!.riskOverride : riskOverride,
    clearRiskOverride,
    openManualOrder,
    closePos,
    closePartial,
    modifyPos,
    closeBulk,
    delPending,
    modifyPending,
    delAllPendings,
    closeBasketById,
    updateBasket,
    historia,
    messages: live ? B!.messages : messages,
    bindings: bindingsView,
    // Podłączony bot → wyłącznie prawdziwe czaty konta (nawet gdy lista jest
    // pusta). Sam prototyp bez serwera → lista poglądowa, żeby dało się
    // obejrzeć ekran. Mieszanka tych dwóch byłaby najgorsza z możliwych:
    // wymyślony kanał wygląda na zaznaczalny, a bot nigdy z niego nie zagra.
    channels: live ? serverChannels : CHANNELS,
    channelsAreReal: live,
    channelsLoading,
    channelsError,
    refreshChannels,
    setBinding,
    simulateMessage,
    executeMessage,
    dismissMessage,
    logs: logsView,
    clearLogs,
    mergedLogCount,
    email: live ? B!.email : email,
    setEmail,
    notify: live ? B!.notify : notify,
    setNotify,
    sims: live ? B!.sims : sims,
    addSim,
    removeSim,
    resetSim,
    toasts,
    toast,
    dismissToast,
    favorites: live ? B!.favorites : favorites,
    toggleFavorite,

    live,
    backendStatus: backend.status,
    nativeShell: backend.nativeShell,
    openInBrowser,
    lab: (live ? B!.lab : null) ?? PUSTE_LAB,
    demo: (live ? B!.demo : null) ?? PUSTE_DEMO,
    scalanie: (live ? B!.scalanie : null) ?? PUSTE_SCALANIE,
    drabinka: drabinkaView,
    setDrabinka,
    auth: B?.auth ?? null,
    startQrLogin,
    submit2fa,
    setTelegramCredentials,
    forgetTelegramCredentials,
    sendTestEmail,
    scalLogi,
    wyslijTestPowiadomienia,
  };

  // A modal may retain an old ticket while its callbacks re-render on a NEW
  // account. Remount the trading UI on each binding generation, clearing old
  // dialogs/forms/selections; already-created callbacks keep their old token.
  return <Ctx.Provider value={value}><Fragment key={accountIntentScope === undefined ? "legacy" : `account:${accountIntentScope}`}>{children}</Fragment></Ctx.Provider>;
}
