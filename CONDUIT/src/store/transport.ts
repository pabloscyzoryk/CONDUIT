/* ============================================================
   TRANSPORT — klient WebSocket + REST do backendu w Rust.

   Ten plik NIE zna Reacta. Zna protokół i sieć: łączy się, wraca po
   zerwaniu, kolejkuje komendy i mówi, czy backend w ogóle istnieje.
   Dzięki temu da się go przetestować i podmienić bez ruszania widoków.

   Protokół (crates/server/src/proto.rs):
     serwer → klient   snapshot | delta | event | ack | pong
     klient → serwer   subscribe | command | settingsPatch | ping
   ============================================================ */

/* CYKL IMPORTOW JEST SWIADOMY I BEZPIECZNY: `@/i18n` importuje `api` z tego
   pliku, a ten plik `t` z `@/i18n`. Zadna ze stron nie uzywa drugiej PODCZAS
   inicjalizacji modulu — `t()` wolamy dopiero w metodach, `api.patchSettings`
   dopiero przy zmianie jezyka. ESM wiaze takie pary leniwie. */
import { t } from "@/i18n";
import type { AiModelFile, AiModelSummary } from "@/lib/aiModel";
import type {
  Basket,
  ChannelBinding,
  ChatMessage,
  ClosedPosition,
  ConnectionState,
  EmailConfig,
  ForeignSummary,
  Format,
  Lancuchy,
  LogEntry,
  LotConfig,
  ParsedSignal,
  PendingHistoryItem,
  PendingOrder,
  Position,
  Quote,
  SimInstance,
  Stats,
  SubjectPreview,
  TradingMode,
} from "@/types";

/* ---------------- protokół: serwer → klient ---------------- */

export type AuthStage =
  | "loggedOut"
  /** brak api_id/api_hash — bez nich nie da się wydać tokenu QR */
  | "needCredentials"
  /** poświadczenia są, klient się podnosi (albo wznawia zapisaną sesję) */
  | "connecting"
  | "waitingScan"
  | "waitingPassword"
  | "confirming"
  | "expired"
  | "loggedIn"
  | "error";

export interface AuthState {
  stage: AuthStage;
  tokenUrl?: string;
  /** gotowy do wstawienia SVG — kolory dziedziczą z motywu (currentColor) */
  qrSvg?: string;
  expiresAt?: number;
  passwordHint?: string;
  user?: string;
  error?: string;

  /** `api_id` jest PUBLICZNY i wraca w całości — inaczej niż `api_hash` */
  apiId?: number;
  /** czy `api_hash` jest zapisany; sama wartość NIGDY nie opuszcza serwera */
  apiHashSet?: boolean;
  /** maska do pokazania w ustawieniach, np. „•••••••• (32 znaków)" */
  apiHashMasked?: string;
  /** czy w `secrets.json` leży sesja, czyli czy następny start pójdzie sam */
  sessionSaved?: boolean;
}

/** Co leży w `secrets.json` — bez ani jednej wartości. */
export interface SecretsSummary {
  telegram: {
    apiId: number;
    apiHashSet: boolean;
    apiHashMasked: string;
    sessionSet: boolean;
    userName: string;
    /** @handle z Telegrama. Pole NIE nazywa się `username`, bo obok jest
        `userName` — dla czytników JSON-a ignorujących wielkość liter byłby
        to ten sam klucz i plik przestawał się otwierać. */
    handle: string;
    savedAt: number;
  };
  smtp: { passwordSet: boolean; passwordMasked: string };
}

/* ---------------- laboratorium (backtesty i trening AI) ---------------- */

export type LabPhase = "running" | "done" | "cancelled" | "failed";

export interface LabRow {
  name: string;
  profit: number;
  perDay: number;
  maxDd: number;
  maxDdPct: number;
  risk: number;
  riskPct: number;
  /** `null` = nieskończony (przebieg bez ani jednej straty) */
  profitFactor: number | null;
  winDaysPct: number;
  winRate: number;
  trades: number;
  maxOpenPositions: number;
  blown: boolean;
  score: number;
  chart: string;
  /** przebieg przerwany — wynik CZĄSTKOWY, nieporównywalny z pełnymi */
  partial: boolean;
}

export interface LabGen {
  gen: number;
  center: number;
  best: number;
  median: number;
  worst: number;
  pnl: number;
  maxDd: number;
  trades: number;
  sigma: number;
  elapsedS: number;
}

export interface LabTrain {
  bestFitness: number;
  bestGen: number;
  medianFitness: number;
  bestDd: number;
  bestPnl: number;
  baselineTrain: number;
  baselineValid: number;
  validFitness: number;
  validPnl: number;
  evalDone: number;
  evalTotal: number;
  modelPath: string;
  checkpointPath: string;
  resumable: boolean;
}

export interface LabJob {
  id: string;
  kind: "backtest" | "train";
  title: string;
  phase: LabPhase;
  /** co silnik liczy W TEJ CHWILI */
  label: string;
  progress: number;
  done: number;
  total: number;
  speed: string;
  speed2: string;
  startedAt: number;
  finishedAt: number | null;
  elapsedMs: number;
  etaMs: number;
  outDir: string;
  note: string;
  error: string | null;
  rows?: LabRow[];
  gens?: LabGen[];
  train?: LabTrain;
  charts?: string[];
}

export interface LabState {
  busy: boolean;
  job: LabJob | null;
  history: LabJob[];
}

export interface LabPresetDir {
  name: string;
  path: string;
  count: number;
  presets: string[];
}

export interface LabInfo {
  ok: boolean;
  ticksPath: string;
  signalsPath: string;
  ticks: number;
  firstDay: string;
  lastDay: string;
  messages: number;
  presetDirs: LabPresetDir[];
  models: string[];
  outRoot: string;
  checkpoint: { gen: number; bestCenter: number; seed: number; algo: string; savedAt: number } | null;
  error: string | null;
}

export interface BacktestReq {
  period: string;
  from?: string;
  to?: string;
  presetDir?: string;
  preset?: string;
  balance: number;
  dailyReset: boolean;
  walkForward: number;
}

export interface TrainReq {
  from: string;
  to: string;
  validFrom: string;
  validTo: string;
  generations: number;
  pop: number;
  seed: number;
  algo: string;
  sigma: number;
  lr: number;
  windows: number;
  windowDays: number;
  validWindows: number;
  balance: number;
  interval: number;
  split: string;
  blocks: number;
  name: string;
  resume: boolean;
}

/* ---------------- tryb demo (wirtualny broker w czasie rzeczywistym) ---------------- */

export type DemoPhase = "idle" | "running" | "finished" | "stopped" | "failed";
export type PriceSource = "file" | "synthetic";

/** Plik rozpoznany PO ZAWARTOŚCI (nagłówek CDTK, klucze JSON-a, wiersz CSV). */
export interface DemoCandidate {
  path: string;
  name: string;
  bytes: number;
  kind: "ticksBin" | "ticksCsv" | "signalsJson" | "telegramJson" | "telegramHtml";
  role: "ticks" | "signals";
  /** czy tryb demo umie z tego pliku odtwarzać */
  usable: boolean;
  records: number;
  /** `false` = liczba rekordów oszacowana z rozmiaru, a nie policzona */
  exact: boolean;
  firstTs: number;
  lastTs: number;
  firstDay: string;
  lastDay: string;
  clock: "server" | "utc" | "other" | "unknown" | "";
  breakHour: number | null;
  /** ile dodać do znacznika WIADOMOŚCI, żeby trafić w zegar tego pliku ticków */
  suggestedMsgOffsetMs: number | null;
  note: string;
}

export interface DemoScan {
  candidates: DemoCandidate[];
  roots: string[];
  filesSeen: number;
  dirsSeen: number;
  elapsedMs: number;
  truncated: boolean;
}

export interface DemoConfig {
  balance: number;
  priceSource: PriceSource;
  ticksPath: string;
  ticksFrom: string;
  ticksTo: string;
  signalsPath: string;
  signalsFrom: string;
  signalsTo: string;
  useFileSignals: boolean;
  /** mnożnik odtwarzania; 0 = maksymalne tempo */
  speed: number;
  seed: number;
  synthStartPrice: number;
  synthVol: number;
  synthSpread: number;
  synthIntervalMs: number;
  msgClockOffsetMs: number | null;
  sourceName: string;
}

export interface DemoState {
  running: boolean;
  phase: DemoPhase;
  config: DemoConfig;
  clock: number;
  clockLabel: string;
  speed: number;
  startBalance: number;
  balance: number;
  equity: number;
  ticksDone: number;
  ticksTotal: number;
  progress: number;
  messages: number;
  signals: number;
  manualSignals: number;
  trades: number;
  openPositions: number;
  openPendings: number;
  baskets: number;
  source: string;
  startedAt: number;
  finishedAt: number | null;
  elapsedMs: number;
  note: string;
  error: string | null;
}

export interface UiSnapshot {
  rev: number;
  serverTime: number;
  mode: TradingMode;
  connection: ConnectionState;
  quotes: Record<string, Quote>;
  positions: Position[];
  pendings: PendingOrder[];
  baskets: Basket[];
  closed: ClosedPosition[];
  pendingHistory: PendingHistoryItem[];
  /** ile z tego, co widac na rachunku, NIE prowadzi bot */
  foreign: ForeignSummary;
  balance: number;
  stats: Stats;
  halt: { active: boolean; reason: string };
  riskOverride: { active: boolean; since: number; reason: string };
  settings: Record<string, unknown>;
  lot: LotConfig;
  presetId: string;
  messages: ChatMessage[];
  logs: LogEntry[];
  bindings: Record<string, ChannelBinding>;
  /**
   * FORMATY SYGNAŁÓW znane silnikowi (`formaty_wbudowane()` + własne).
   *
   * POLE OPCJONALNE, bo starsza binarka tej sekcji nie odsyła. Panel działa
   * wtedy na katalogu wbudowanym (`data/formaty.ts`) — a nie na pustej liście,
   * bo pusta lista formatów znaczyłaby „nic nie handluje" i wyglądałaby jak
   * poprawny stan, którym nie jest.
   */
  formaty?: Format[];
  /**
   * ŁAŃCUCHY (mapa `format → preset` + pułapy globalne) wraz z aktywnym.
   * Też opcjonalne — patrz `formaty` wyżej. Bez tej sekcji panel trzyma
   * łańcuchy lokalnie i mówi wprost, że to tryb projektowy.
   */
  lancuchy?: Lancuchy;
  /**
   * WSKAŹNIK AKTYWNEGO ŁAŃCUCHA DLA TRYBU **AUTO-EA** (projekt EA-2).
   *
   * Pusty łańcuch znaków (albo brak pola — starsza binarka) znaczy „AUTO-EA
   * nie ma własnego wskazania" i wtedy gra tym samym, co reszta trybów
   * (`lancuchy.aktywny`). Rozstrzyga to `aktywnyDla()` z `data/formaty.ts`,
   * bliźniak `ui::aktywny_dla` po stronie serwera.
   */
  aktywnyEa?: string;
  
  pieczecLancuch?: string;
  /** `pass` przychodzi ZAWSZE puste — hasło mieszka w `secrets.json`. */
  email: EmailConfig;
  notify: { channels: number[]; summaryEnabled: boolean; summaryIntervalMin: number };
  sims: SimInstance[];
  favorites: string[];
  /**
   * JĘZYK PANELU — klucz GŁÓWNEGO dokumentu ustawień (obok `favorites`),
   * NIE pole w `settings{}`: to preferencja panelu, nie reguła rachunku,
   * więc wczytanie presetu nie ma prawa jej zdjąć. Opcjonalne, bo starsza
   * binarka tego pola nie odsyła — panel zostaje wtedy przy localStorage.
   */
  language?: string;
  auth: AuthState;
  /** postęp backtestów i treningu AI — sekcja `lab` */
  lab: LabState;
  /** tryb demo: wirtualny broker odtwarzający dane w czasie rzeczywistym */
  demo: DemoState;
  /** postęp scalania dziennika — sekcja `scalanie` */
  scalanie: PostepScalania;
  /** drabinka ŁAŃCUCHÓW wg BALANCE konta — TRYBY AUTO / MANUAL / AI */
  drabinka: DrabinkaLancuchow;
  /**
   * DRABINKA TRYBU **AUTO-EA** — „SKYNET-1" (projekt EA-2c).
   *
   * Osobny stan, bo osobny łańcuch: ta drabinka przestawia WYŁĄCZNIE
   * `aktywnyEa`. Opcjonalna — binarka sprzed EA-2c pola nie odsyła i wtedy
   * panel pokazuje drabinkę pustą (kontrakt zera). Którą z dwóch pokazać,
   * rozstrzyga `drabinkaDla()` z `data/formaty.ts`, bliźniak
   * `ui::UiSnapshot::drabinka_biezaca` po stronie serwera.
   */
  drabinkaEa?: DrabinkaLancuchow;
}


export interface PostepScalania {
  /** czy scalanie trwa W TEJ CHWILI */
  aktywne: boolean;
  /** `""` (nigdy nie uruchamiane) | `trwa` | `gotowe` | `blad` */
  faza: string;
  /** co robi teraz, np. „dziennik decyzji · 2026-07-30.jsonl" */
  etap: string;
  /** 0…1 */
  postep: number;
  /** BAJTOW wejscia przetworzonych (nie plikow — sekcje sa skrajnie nierowne) */
  zrobione: number;
  /** bajtow wejscia lacznie */
  wszystkich: number;
  /** czytelna prędkość, np. „14,2 MB/s" */
  predkosc: string;
  /** szacowany czas do końca w ms; 0 = jeszcze nie wiadomo */
  etaMs: number;
  czasMs: number;
  plik: string;
  znakow: number;
  sciezka: string;
  /** ile ZRODEL weszlo do pliku (dziennik, kronika, archiwum…) */
  zrodel?: number;
  /** ile zrodel odrzucono (odznaczone w panelu albo puste) */
  pominietych?: number;
  blad: string | null;
}

/** Jeden szczebel drabinki presetów — od jakiego salda obowiązuje dany preset. */
export interface SzczebelDrabinki {
  /** BALANCE w dolarach, od którego ten łańcuch obowiązuje */
  progBalance: number;
  /** nazwa łańcucha z listy `Lancuchy.lista` */
  lancuch: string;
}

/** DRABINKA ŁAŃCUCHÓW (FFS-1C) — automatyczna zmiana AKTYWNEGO ŁAŃCUCHA
    wraz ze wzrostem i spadkiem konta. Progi liczą się po BALANCE, nigdy po
    equity (equity oddycha z pozycjami). Przełączenie przebudowuje silniki
    W LOCIE z pełną adopcją koszyków. */
export interface DrabinkaLancuchow {
  /**
   * WŁĄCZNIK SKUTECZNY — czy ta drabinka rządzi TERAZ. Serwer gasi drabinkę
   * trybu, w którym bot akurat nie pracuje (izolacja EA-2c), więc w panelu
   * jest to prawda o bocie, a nie o klikniętym przełączniku.
   */
  enabled: boolean;
  /**
   * WYŁĄCZNIK UŻYTKOWNIKA — intencja zapamiętana przez serwer; wraca razem
   * ze swoim trybem. Panel go NIE ustawia: wysyła `enabled`, a serwer czyta
   * to jako intencję (pokazujemy zawsze drabinkę bieżącego trybu, więc jedno
   * równa się drugiemu). Pole jedzie w obie strony, żeby echo nie kasowało
   * pamięci wyłącznika.
   */
  wlacznik?: boolean;
  szczeble: SzczebelDrabinki[];
  /** ile procent PONIŻEJ progu bieżącego szczebla musi spaść balance, żeby zejść */
  histerezaPct: number;
  /** próg szczebla, na którym drabinka ostatnio stała (-1 = nigdzie) */
  biezacyProg: number;
  /** kiedy drabinka ostatnio przełączyła łańcuch (ms epoki, 0 = nigdy) */
  ostatniaZmianaTs: number;
}

/** Delta niesie WYŁĄCZNIE zmienione sekcje — reszta pól jest nieobecna. */
export type DeltaPatch = Partial<UiSnapshot>;

export type ServerEvent =
  | { kind: "signal"; message: ChatMessage }
  | { kind: "fill"; ticket: number; price: number; volume: number; direction: "BUY" | "SELL" }
  | { kind: "closed"; trade: ClosedPosition }
  | { kind: "toast"; level: string; title: string; text: string }
  | { kind: "log"; entry: LogEntry }
  | { kind: "alert"; reason: string; halted: boolean };

type ServerMsg =
  | { type: "snapshot"; state: UiSnapshot }
  | { type: "delta"; rev: number; patch: DeltaPatch }
  | { type: "event"; event: ServerEvent }
  | { type: "ack"; reqId: number; ok: boolean; error?: string }
  | { type: "pong"; ts: number; serverTime: number };

/* ---------------- protokół: klient → serwer ---------------- */

export type Section =
  | "quotes" | "positions" | "pendings" | "baskets" | "closed" | "pendingHistory"
  | "stats" | "logs" | "messages" | "settings" | "auth" | "connection"
  | "halt" | "sims" | "bindings" | "mode" | "lab" | "demo" | "lancuchy";

/**
 * Komenda = `cmd` + pola płasko obok (patrz `proto::Command`).
 *
 * Identyfikator korelacji nazywa się `reqId`, NIE `id` — bo `id` jest zajęte
 * przez same komendy (preset, koszyk, wiadomość, instancja symulacji).
 */
export type Command = { cmd: string } & Record<string, unknown>;

/** Bind at intent creation. Undo/queued commands keep their ORIGINAL token. */
export function bindCommandToAccount(cmd: Command, renderedAccountSession?: string): Command {
  if (Object.prototype.hasOwnProperty.call(cmd, "accountSession")) return cmd;
  return renderedAccountSession === undefined ? cmd : { ...cmd, accountSession: renderedAccountSession };
}

export type TransportStatus = "idle" | "connecting" | "open" | "reconnecting" | "closed";

export interface TransportHandlers {
  onSnapshot?: (s: UiSnapshot) => void;
  onDelta?: (p: DeltaPatch, rev: number) => void;
  onEvent?: (e: ServerEvent) => void;
  onStatus?: (s: TransportStatus) => void;
  onLatency?: (ms: number) => void;
}

/* ---------------- rozbiór wiadomości i eksporty ---------------- */

/** Odpowiedź `/api/parse` — co parser SILNIKA wyjął z wklejonego tekstu. */
export interface ParsePreview {
  ok: boolean;
  /** czy cokolwiek poza `INFO` — czyli czy bot w ogóle by zareagował */
  recognized: boolean;
  types: string[];
  parsed: ParsedSignal[];
  /** dosłownie to, co dostał parser — do wykrycia niewidocznych znaków */
  input: { text: string; chars: number; lines: number };
}

export interface ExportIndex {
  journalDir: string;
  journalFiles: { day: string; prefix: string; bytes: number; file: string }[];
  /** Wlasne archiwum wiadomosci — kazda wersja kazdej wiadomosci osobno. */
  archiveDir: string;
  archiveFiles: { day: string; bytes: number; file: string }[];
  counts: { closed: number; pendingHistory: number; logs: number; messages: number };
}

/* ---------------- adres backendu ---------------- */

/**
 * Skąd brać backend:
 *  1. `?api=` w adresie — awaryjne wskazanie ręczne,
 *  2. to samo pochodzenie — gdy stronę serwuje `conduit.exe`,
 *  3. `http://127.0.0.1:8787` — gdy UI leci z Vite (`npm run dev`) na :5180.
 *
 * Punkt 3 jest jedynym miejscem z zaszytym portem i istnieje wyłącznie dla
 * wygody pracy nad UI; w wersji zbudowanej zawsze wygrywa punkt 2.
 */
export function backendBase(): string {
  if (typeof window === "undefined") return "http://127.0.0.1:8787";
  const wymuszony = new URLSearchParams(window.location.search).get("api");
  if (wymuszony) return wymuszony.replace(/\/$/, "");
  const { protocol, host, origin } = window.location;
  if (protocol === "http:" || protocol === "https:") {
    // strona z serwera Vite (port deweloperski) → backend osobno
    if (host.endsWith(":5180") || host.endsWith(":5173")) return "http://127.0.0.1:8787";
    return origin;
  }
  return "http://127.0.0.1:8787";
}

export function wsUrl(base = backendBase()): string {
  return base.replace(/^http/, "ws") + "/ws";
}

/** Czy uruchomiono nas w oknie natywnym (`?shell=native` dokłada Tauri)? */
export function isNativeShell(): boolean {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("shell") === "native";
}

/** Czy backend odpowiada? Krótki limit — brak backendu to normalny scenariusz. */
export async function probeBackend(base = backendBase(), timeoutMs = 1200): Promise<boolean> {
  try {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), timeoutMs);
    const r = await fetch(`${base}/api/health`, { signal: ctl.signal });
    clearTimeout(timer);
    if (!r.ok) return false;
    const j = await r.json();
    return j?.ok === true && j?.app === "conduit";
  } catch {
    return false;
  }
}

/* ---------------- klient ---------------- */

interface Oczekujaca {
  id: number;
  wiadomosc: string;
  resolve: (ok: { ok: boolean; error?: string }) => void;
  timer?: ReturnType<typeof setTimeout>;
}

const MAX_KOLEJKA = 100;
const ACK_TIMEOUT_MS = 10_000;

export class Transport {
  private base: string;
  private ws: WebSocket | null = null;
  private handlers: TransportHandlers;
  private status: TransportStatus = "idle";
  private nextId = 1;
  /** komendy wysłane i czekające na `ack` */
  private wLocie = new Map<number, Oczekujaca>();
  /** komendy złożone przy zerwanym połączeniu */
  private kolejka: Oczekujaca[] = [];
  private proba = 0;
  private timerPonow?: ReturnType<typeof setTimeout>;
  private timerPing?: ReturnType<typeof setInterval>;
  private zamkniete = false;
  private sekcje: Section[] = [];

  constructor(handlers: TransportHandlers = {}, base = backendBase()) {
    this.handlers = handlers;
    this.base = base;
  }

  get url(): string {
    return this.base;
  }

  connect(sections: Section[] = []): void {
    this.zamkniete = false;
    this.sekcje = sections;
    this.otworz();
  }

  close(): void {
    this.zamkniete = true;
    if (this.timerPonow) clearTimeout(this.timerPonow);
    if (this.timerPing) clearInterval(this.timerPing);
    this.ws?.close();
    this.ws = null;
    this.ustawStatus("closed");
    // nikt już nie odbierze odpowiedzi — nie zostawiamy wiszących obietnic
    for (const p of [...this.wLocie.values(), ...this.kolejka]) {
      if (p.timer) clearTimeout(p.timer);
      p.resolve({ ok: false, error: t("net.closed") });
    }
    this.wLocie.clear();
    this.kolejka = [];
  }

  /** Wysyła komendę. Obietnica spełnia się na `ack` z serwera. */
  send(cmd: Command, renderedAccountSession?: string): Promise<{ ok: boolean; error?: string }> {
    const reqId = this.nextId++;
    return this.wyslijZAck(reqId, JSON.stringify({ type: "command", reqId, ...bindCommandToAccount(cmd, renderedAccountSession) }));
  }

  /** Łatka ustawień — osobny typ komunikatu, bo nie jest to akcja handlowa. */
  patchSettings(patch: Record<string, unknown>): Promise<{ ok: boolean; error?: string }> {
    const reqId = this.nextId++;
    return this.wyslijZAck(reqId, JSON.stringify({ type: "settingsPatch", reqId, patch }));
  }

  private wyslijZAck(id: number, wiadomosc: string): Promise<{ ok: boolean; error?: string }> {
    return new Promise((resolve) => {
      const wpis: Oczekujaca = { id, wiadomosc, resolve };
      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        this.zarejestruj(wpis);
        this.ws.send(wiadomosc);
      } else {
        // Kolejkujemy zamiast gubić: użytkownik kliknął „Zamknij pozycję"
        // w chwili mikroprzerwy sieciowej i ma prawo oczekiwać, że to zadziała.
        // Kolejka jest ograniczona — po zerwaniu na godzinę nie chcemy wysłać
        // tysiąca zaległych komend naraz.
        if (this.kolejka.length >= MAX_KOLEJKA) {
          resolve({ ok: false, error: t("net.queueFull") });
          return;
        }
        this.kolejka.push(wpis);
      }
    });
  }

  private zarejestruj(w: Oczekujaca): void {
    w.timer = setTimeout(() => {
      this.wLocie.delete(w.id);
      w.resolve({ ok: false, error: t("net.timeout") });
    }, ACK_TIMEOUT_MS);
    this.wLocie.set(w.id, w);
  }

  private otworz(): void {
    if (this.zamkniete) return;
    this.ustawStatus(this.proba === 0 ? "connecting" : "reconnecting");

    let ws: WebSocket;
    try {
      ws = new WebSocket(wsUrl(this.base));
    } catch {
      this.zaplanujPonowienie();
      return;
    }
    this.ws = ws;

    ws.onopen = () => {
      this.proba = 0;
      this.ustawStatus("open");
      if (this.sekcje.length) {
        ws.send(JSON.stringify({ type: "subscribe", sections: this.sekcje }));
      }
      // opróżnij kolejkę zebraną podczas przerwy
      const zalegle = this.kolejka;
      this.kolejka = [];
      for (const w of zalegle) {
        this.zarejestruj(w);
        ws.send(w.wiadomosc);
      }
      this.timerPing = setInterval(() => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "ping", ts: Date.now() }));
        }
      }, 5000);
    };

    ws.onmessage = (ev) => {
      let msg: ServerMsg;
      try {
        msg = JSON.parse(ev.data as string);
      } catch {
        return;
      }
      switch (msg.type) {
        case "snapshot":
          this.handlers.onSnapshot?.(msg.state);
          break;
        case "delta":
          this.handlers.onDelta?.(msg.patch, msg.rev);
          break;
        case "event":
          this.handlers.onEvent?.(msg.event);
          break;
        case "ack": {
          const w = this.wLocie.get(msg.reqId);
          if (w) {
            if (w.timer) clearTimeout(w.timer);
            this.wLocie.delete(msg.reqId);
            w.resolve({ ok: msg.ok, error: msg.error });
          }
          break;
        }
        case "pong":
          this.handlers.onLatency?.(Math.max(0, Date.now() - msg.ts));
          break;
      }
    };

    ws.onclose = () => {
      if (this.timerPing) clearInterval(this.timerPing);
      this.ws = null;
      if (!this.zamkniete) this.zaplanujPonowienie();
    };

    ws.onerror = () => {
      // `onclose` i tak przyjdzie — tu nic nie robimy, żeby nie ponawiać dwa razy
    };
  }

  private zaplanujPonowienie(): void {
    this.proba += 1;
    // 0,5 s → 8 s z losowym rozrzutem, żeby okno i wszystkie karty
    // przeglądarki nie wracały do serwera dokładnie w tej samej milisekundzie
    const podstawa = Math.min(8000, 500 * 2 ** Math.min(this.proba - 1, 4));
    const opoznienie = podstawa * (0.7 + Math.random() * 0.6);
    this.ustawStatus("reconnecting");
    this.timerPonow = setTimeout(() => this.otworz(), opoznienie);
  }

  private ustawStatus(s: TransportStatus): void {
    if (this.status === s) return;
    this.status = s;
    this.handlers.onStatus?.(s);
  }
}

/* ---------------- REST ---------------- */

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(`${backendBase()}${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
  });
  if (!r.ok) {
    let tresc = `HTTP ${r.status}`;
    try {
      const b = await r.json();
      if (b?.error) tresc = b.error;
    } catch {
      /* odpowiedź nie jest JSON-em — zostaje kod stanu */
    }
    throw new Error(tresc);
  }
  return (await r.json()) as T;
}

export const api = {
  health: () => json<{ ok: boolean; version: string; clients: number }>("/api/health"),
  state: () => json<UiSnapshot>("/api/state"),
  diag: () => json<Record<string, unknown>>("/api/diag"),
  presets: () => json<{ name: string; description: string; settings: Record<string, unknown> }[]>("/api/presets"),

  
  drabinki: () =>
    json<
      {
        nazwa: string;
        opis: string;
        korona: boolean;
        szczeble: { progBalance: number; lancuch: string }[];
        histerezaPct: number;
      }[]
    >("/api/drabinki"),

  /** Ustawienia presetu w KSZTALCIE PANELU — do edycji per preset (lancuch
      z wieloma presetami). Plik trzyma klucze silnika; tlumaczy serwer, ta
      sama para funkcji co przy wczytaniu presetu. */
  presetUi: (name: string) =>
    json<{ name: string; format: string; settings: Record<string, unknown> }>(
      `/api/presets/${encodeURIComponent(name)}/ui`,
    ),

  /** Latka ustawien PRESETU (klucze panelu) — zapis do pliku presetu na
      dysku. Silnik formatu przeladuje go w ciagu 2 s; zmiana przezywa
      restart, bo zrodlem prawdy jest plik. */
  savePresetSettings: (name: string, patch: Record<string, unknown>) =>
    json<{ ok: boolean }>(`/api/presets/${encodeURIComponent(name)}/settings`, {
      method: "POST",
      body: JSON.stringify(patch),
    }),
  history: (q: Record<string, string | number> = {}) =>
    json<ClosedPosition[]>(`/api/history?${new URLSearchParams(q as Record<string, string>)}`),
  pendingHistory: () => json<PendingHistoryItem[]>("/api/history/pendings"),
  logs: (q: Record<string, string | number> = {}) =>
    json<LogEntry[]>(`/api/logs?${new URLSearchParams(q as Record<string, string>)}`),
  backtests: () => json<{ id: string; bytes: number }[]>("/api/backtests"),
  backtest: (id: string) => json<Record<string, unknown>>(`/api/backtests/${encodeURIComponent(id)}`),

  /* --- modele AI: lista bez wag, dokument z wagami osobno --- */
  models: () => json<AiModelSummary[]>("/api/models"),
  model: (id: string) => json<AiModelFile>(`/api/models/${encodeURIComponent(id)}`),

  unmappedSettings: () => json<string[]>("/api/settings/unmapped"),

  /** ROZBIÓR WIADOMOŚCI NA SUCHO.
      Nie dotyka ani stanu, ani brokera — w odróżnieniu od `demoSignal`,
      który sygnał WYKONUJE. To jest jedyny bezpieczny sposób sprawdzenia
      formatu z nowego kanału przy otwartym rynku. */
  parse: (text: string) =>
    json<ParsePreview>("/api/parse", { method: "POST", body: JSON.stringify({ text }) }),

  /* --- EKSPORTY ---
      Pliki buduje serwer i oddaje z nagłówkiem `Content-Disposition`, więc
      panel tylko otwiera adres. Dzięki temu eksport dziennika (który leży na
      dysku, a nie w migawce) działa tak samo jak eksport historii. */
  exportIndex: () => json<ExportIndex>("/api/export/index"),
  exportUrl: (plik: string, q: Record<string, string | number | undefined> = {}) => {
    const p = new URLSearchParams();
    for (const [k, v] of Object.entries(q)) if (v !== undefined && v !== "") p.set(k, String(v));
    const qs = p.toString();
    return `${backendBase()}/api/export/${plik}${qs ? `?${qs}` : ""}`;
  },

  /** PRAWDZIWA lista czatow konta Telegram + zapisane powiazania.
      `channels` jest puste, dopoki nie ma zalogowanej sesji — wtedy
      `channelsError` mowi, czego brakuje. Panel NIE MOZE w takiej sytuacji
      pokazac listy zastepczej: zaznaczenie wymyslonego identyfikatora znaczy
      „nie handluj nigdy", bo bot porownuje chat_id wiadomosci z powiazaniem. */
  channels: () =>
    json<{
      bindings: Record<string, unknown>;
      channels: {
        id: number;
        name: string;
        handle?: string | null;
        kind: string;
        isForum: boolean;
        /** czy czat MA zdjecie profilowe — bez tego panel pytalby o obrazek
            takze te kanaly, ktore zdjecia nigdy nie mialy */
        hasPhoto?: boolean;
        /** wersja zdjecia (photo_id jako tekst) — doklejana do adresu, zeby
            podmiana obrazka przebila pamiec podreczna przegladarki */
        photoVersion?: string | null;
        topics: { id: number; title: string; closed: boolean }[];
      }[];
      channelsError: string | null;
    }>("/api/channels"),

  
  /** Symbole brokera z mostu MT5. Kontrakt agenta SYMBOLE:
      `[{name, visible, digits, tradeMode}]` + 503 przy padniętym moście;
      tolerujemy też wcześniejszy kształt `{symbols:[...]}`. Fallback po
      stronie wołającego: lista statyczna + ZAWSZE symbol bota. */
  symbols: async (q: string): Promise<{ name: string; visible: boolean; digits?: number; tradeMode?: number }[]> => {
    const surowe = await json<unknown>("/api/symbols" + (q ? `?q=${encodeURIComponent(q)}` : ""));
    const lista = Array.isArray(surowe)
      ? surowe
      : ((surowe as { symbols?: unknown[] })?.symbols ?? []);
    return (lista as { name?: unknown; visible?: unknown; digits?: unknown; tradeMode?: unknown }[])
      .filter((x) => typeof x?.name === "string")
      .map((x) => ({
        name: x.name as string,
        visible: Boolean(x.visible ?? true),
        digits: typeof x.digits === "number" ? x.digits : undefined,
        tradeMode: typeof x.tradeMode === "number" ? x.tradeMode : undefined,
      }));
  },

  scalAnuluj: () => json<{ ok: boolean }>("/api/logs/merge/cancel", { method: "POST" }),

  
  fsDirs: (path?: string, pliki?: string) => {
    const q = new URLSearchParams();
    if (path) q.set("path", path);
    if (pliki) q.set("pliki", pliki);
    const s = q.toString();
    return json<{
      path: string;
      parent: string | null;
      dirs: { name: string; path: string }[];
      pliki?: { name: string; path: string; bytes: number }[];
    }>("/api/fs/dirs" + (s ? `?${s}` : ""));
  },

  /** „POKAŻ PLIK" — otwiera Eksploratora z zaznaczonym plikiem NA MASZYNIE
      SERWERA. Serwer odrzuca żądanie spoza localhosta (409), bo dla panelu
      otwartego zdalnie okno wyskoczyłoby u nikogo — panel pokazuje wtedy
      ścieżkę do skopiowania. Ścieżka musi leżeć w katalogu bota albo
      w katalogu docelowym scalania. */
  /** PLIK na schowek Windows — jako PLIK (CF_HDROP), nie jako tekst.
      Wtedy Ctrl+V w oknie rozmowy wkleja załącznik. Działa tylko lokalnie,
      bo schowek jest na maszynie serwera. */
  kopiujPlik: (path: string) =>
    json<{ ok: boolean; path: string }>("/api/fs/copy", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),

  /** TREŚĆ pliku do schowka. Działa TAKŻE zdalnie — w odróżnieniu od
      `pokazPlik`, które otwiera okno na maszynie serwera i przy panelu
      otwartym z innego komputera jest bezużyteczne. */
  czytajPlik: (path: string) =>
    json<{ ok: boolean; path: string; bytes: number; text: string }>("/api/fs/read", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),

  pokazPlik: (path: string) =>
    json<{ ok: boolean; path: string }>("/api/shell/reveal", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),

  /** Łatka ustawień przez REST — dla miejsc, które nie mają uchwytu do WS
      (np. przełącznik motywu, dostępny także na ekranie logowania). */
  patchSettings: (patch: Record<string, unknown>) =>
    json<{ ok: boolean; rev: number }>("/api/settings", {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),

  /* --- logowanie do Telegrama ---
     KROK ZERO to `authSetCredentials`. Bez api_id/api_hash serwer Telegrama
     nie wyda tokenu, więc nie ma z czego zrobić kodu QR — a kod narysowany
     „na zapas" byłby atrapą, której telefon nie zaloguje. */
  authState: () => json<AuthState>("/api/auth/state"),
  authSetCredentials: (apiId: string | number, apiHash: string) =>
    json<AuthState>("/api/auth/credentials", {
      method: "POST",
      body: JSON.stringify({ apiId, apiHash }),
    }),
  /** Kasuje api_id, api_hash I sesję — „zacznij od zera". */
  authForgetCredentials: () =>
    json<AuthState>("/api/auth/credentials", { method: "DELETE" }),
  authStartQr: () => json<AuthState>("/api/auth/qr/start", { method: "POST", body: "{}" }),
  authSubmit2fa: (password: string) =>
    json<AuthState>("/api/auth/2fa", { method: "POST", body: JSON.stringify({ password }) }),
  authLogout: () => json<AuthState>("/api/auth/logout", { method: "POST", body: "{}" }),

  /* --- poświadczenia: WYŁĄCZNIE „jest / nie ma", nigdy wartości --- */
  secrets: () => json<SecretsSummary>("/api/secrets"),

  /* --- poczta ---
     Odpowiedź niesie wynik PRAWDZIWEJ próby wysyłki, nie „przyjęto do
     kolejki" — przycisk diagnostyczny musi diagnozować. */
  emailTest: () =>
    json<{ ok: boolean; detail: string }>("/api/email/test", { method: "POST", body: "{}" }),

  /* --- PODGLĄD TEMATU MAILA ---
     Podstawianie `${...}` robi WYŁĄCZNIE Rust, a panel pyta o wynik. Gdyby
     panel podstawiał sam, istniałyby dwa silniki szablonów i pierwsza
     rozbieżność (zaokrąglenie, znak waluty, definicja doby handlowej)
     zamieniłaby podgląd w obietnicę bez pokrycia.
     Bez `tpl` odpowiada dla szablonu ZAPISANEGO w ustawieniach. */
  emailSubject: (tpl?: string) =>
    json<SubjectPreview>(
      `/api/email/subject${tpl === undefined ? "" : `?tpl=${encodeURIComponent(tpl)}`}`,
    ),

  
  /** Startuje scalanie W TLE i wraca natychmiast — postęp leci w sekcji
      `scalanie` migawki stanu. Wcześniej to żądanie wisiało do końca pracy,
      więc pasek postępu nie miał czego pokazywać. */
  mergeLogs: () =>
    json<{ ok: boolean; started: boolean }>("/api/logs/merge", {
      method: "POST",
      body: "{}",
    }),

  /* --- powłoka --- */
  openInBrowser: () => json<{ ok: boolean; url: string }>("/api/shell/open-browser", { method: "POST", body: "{}" }),

  /* --- LABORATORIUM ---
     Start wraca NATYCHMIAST z identyfikatorem zadania; postęp przychodzi
     potem w sekcji `lab` snapshotu, tak samo do okna i do przeglądarki. */
  labInfo: () => json<LabInfo>("/api/lab/info"),
  labState: () => json<LabState>("/api/lab/state"),
  labBacktest: (req: BacktestReq) =>
    json<{ ok: boolean; id: string }>("/api/lab/backtest", { method: "POST", body: JSON.stringify(req) }),
  labTrain: (req: TrainReq) =>
    json<{ ok: boolean; id: string }>("/api/lab/train", { method: "POST", body: JSON.stringify(req) }),
  labCancel: () => json<{ ok: boolean }>("/api/lab/cancel", { method: "POST", body: "{}" }),
  labOpenDir: (job: string) =>
    json<{ ok: boolean; path: string }>("/api/lab/open-dir", { method: "POST", body: JSON.stringify({ job }) }),
  labFiles: (job: string) =>
    json<{ name: string; bytes: number }[]>(`/api/lab/${encodeURIComponent(job)}/files`),
  /** adres pliku wyniku — do wstawienia wprost w `<img src>` */
  labFileUrl: (job: string, name: string) =>
    `${backendBase()}/api/lab/${encodeURIComponent(job)}/file/${encodeURIComponent(name)}`,

  /* --- TRYB DEMO ---
     Konfiguracja i wyszukiwanie plików idą REST-em; postęp przebiegu wraca
     sekcją `demo` snapshotu, tak samo do okna i do przeglądarki. */
  demoState: () => json<DemoState>("/api/demo/state"),
  demoConfig: () => json<DemoConfig>("/api/demo/config"),
  demoSaveConfig: (cfg: DemoConfig) =>
    json<{ ok: boolean }>("/api/demo/config", { method: "POST", body: JSON.stringify(cfg) }),
  /** Przeszukuje pobliskie katalogi i rozpoznaje pliki PO ZAWARTOŚCI. */
  demoScan: (req: { depth?: number; timeoutMs?: number; root?: string } = {}) =>
    json<DemoScan>("/api/demo/scan", { method: "POST", body: JSON.stringify(req) }),
  /** Rozpoznaje pojedynczy plik wskazany ręcznie albo upuszczony w oknie. */
  demoInspect: (path: string) =>
    json<DemoCandidate>("/api/demo/inspect", { method: "POST", body: JSON.stringify({ path }) }),
  /** Szuka pliku po nazwie i rozmiarze — dla przeciągania, gdzie ścieżki brak. */
  demoLocate: (name: string, size?: number) =>
    json<{ ok: boolean; matches: DemoCandidate[]; scannedFiles: number; truncated: boolean }>(
      "/api/demo/locate",
      { method: "POST", body: JSON.stringify({ name, size }) },
    ),
  demoStart: (cfg: DemoConfig) =>
    json<{ ok: boolean }>("/api/demo/start", { method: "POST", body: JSON.stringify(cfg) }),
  demoStop: () => json<{ ok: boolean }>("/api/demo/stop", { method: "POST", body: "{}" }),
  /** Zmiana tempa BEZ restartu przebiegu. */
  demoSpeed: (speed: number) =>
    json<{ ok: boolean; speed: number }>("/api/demo/speed", { method: "POST", body: JSON.stringify({ speed }) }),
  /** Ręczny sygnał — działa też POZA trybem demo (leci do prawdziwego silnika). */
  demoSignal: (text: string, channelId?: number, accountSession?: string) =>
    json<{ ok: boolean; target: string; parsed: unknown[] }>("/api/demo/signal", {
      method: "POST",
      body: JSON.stringify({ text, channelId, accountSession }),
    }),
};

/** Scala deltę ze stanem. Sekcja nieobecna w delcie ZOSTAJE bez zmian. */
export function mergeDelta<T extends object>(stan: T, patch: Partial<T>): T {
  const out = { ...stan };
  for (const [k, v] of Object.entries(patch)) {
    if (v !== undefined) (out as Record<string, unknown>)[k] = v;
  }
  return out;
}
