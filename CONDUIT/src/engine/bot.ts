import type {
  Basket,
  BasketEvent,
  ClosedPosition,
  CloseReason,
  Direction,
  ParsedSignal,
  PendingHistoryItem,
  PendingOrder,
  PendingKind,
  Position,
  Settings,
  TradingMode,
} from "@/types";
import { getSymbol } from "@/data/symbols";

/* ============================================================
   SILNIK BOTA
   Odwzorowanie petli zarzadzajacej z bot.py: rozstawianie siatki
   limitow (place_limit_grid), wypelnienia, drabinka TP, trailing
   (gap / lock_pct / tiered), BE-lock, harvest, out-at-entry,
   stagnacja, risk-free, straznicy ryzyka.
   Wszystko dziala na lokalnym stanie — bez backendu.
   ============================================================ */

export interface BotState {
  symbol: string;
  positions: Position[];
  pendings: PendingOrder[];
  baskets: Basket[];
  closed: ClosedPosition[];
  pendingHistory: PendingHistoryItem[];
  nextTicket: number;
  nextBasketId: number;
  balance: number;
  startBalance: number;
  lossStreak: number;
  pausedUntil: number;
  lastVslEval: number;
  events: { t: number; text: string; kind: BasketEvent["kind"] }[];
}

export function createBotState(symbol: string, balance: number): BotState {
  return {
    symbol,
    positions: [],
    pendings: [],
    baskets: [],
    closed: [],
    pendingHistory: [],
    nextTicket: 830_100_400,
    nextBasketId: 1,
    balance,
    startBalance: balance,
    lossStreak: 0,
    pausedUntil: 0,
    lastVslEval: 0,
    events: [],
  };
}

/* ---------------- POMOCNICZE ---------------- */

const now = () => Date.now();

function logEvt(s: BotState, text: string, kind: BasketEvent["kind"] = "info") {
  s.events.push({ t: now(), text, kind });
  if (s.events.length > 400) s.events.shift();
}

function bEvt(b: Basket, text: string, kind: BasketEvent["kind"] = "info") {
  b.events.push({ t: now(), text, kind });
  if (b.events.length > 60) b.events.shift();
}

/** Zysk pozycji w USD przy zadanej cenie. */
export function positionProfit(p: Position, price: number): number {
  const cs = getSymbol(p.symbol).contractSize;
  const dir = p.direction === "BUY" ? 1 : -1;
  return (price - p.openPrice) * dir * cs * p.volume;
}

/** Zysk pozycji w PUNKTACH ceny. */
export function positionPoints(p: Position, price: number): number {
  const dir = p.direction === "BUY" ? 1 : -1;
  return (price - p.openPrice) * dir;
}

/** Potencjalny wynik przy TP / SL (potential_pnl z bot.py). */
export function potentialAt(p: Position | PendingOrder, target: number | null): number | null {
  if (target === null) return null;
  const cs = getSymbol(p.symbol).contractSize;
  const open = "openPrice" in p ? p.openPrice : p.price;
  const dir = ("direction" in p ? p.direction : p.kind.startsWith("BUY") ? "BUY" : "SELL") === "BUY" ? 1 : -1;
  return (target - open) * dir * cs * p.volume;
}

/** Efektywny lot bota (current_lot_size z bot.py). */
export function currentLot(
  lotMode: "fixed" | "percent",
  fixed: number,
  percent: number,
  balance: number,
  scaleStep: number,
): number {
  let lot = lotMode === "percent" ? (balance * percent) / 100 / 100 : fixed;
  if (scaleStep > 0) {
    const steps = Math.floor(balance / scaleStep);
    lot = Math.max(lot, 0.01 * Math.max(1, steps));
  }
  return Math.max(0.01, Math.round(lot * 100) / 100);
}

/** Ile jednostek na poziomie — uwzglednia budzety ryzyka i rezim zmiennosci. */
function unitsForLevel(
  st: Settings,
  isLimit: boolean,
  levelPrice: number,
  sl: number | null,
  tp1: number | undefined,
  volFactor: number,
): number {
  const base = isLimit && st.entry_units_limit > 0 ? st.entry_units_limit : st.entry_units;
  let u = base;

  if (st.entry_risk_budget > 0 && sl !== null) {
    const dist = Math.abs(levelPrice - sl);
    if (dist > 0) u = Math.min(base, Math.max(1, Math.round(st.entry_risk_budget / dist)));
  }
  if (st.entry_tp1_budget > 0 && tp1 !== undefined) {
    const dist = Math.abs(tp1 - levelPrice);
    if (dist > 0) u = Math.min(u, Math.max(1, Math.round(st.entry_tp1_budget / dist)));
  }
  return Math.max(1, Math.round(u * volFactor));
}

/** Krok siatki: 1 / PPM gdy PPM aktywny dla limitow (limit_step z bot.py). */
function gridStep(st: Settings, isLimit: boolean): number {
  if (!st.ppm_enabled) return 1;
  if (isLimit && !st.ppm_for_limits) return 1;
  if (!isLimit && !st.ppm_immediate) return 1;
  return 1 / Math.max(0.1, st.ppm);
}

/** Parsowanie "off:units:tp,..." dla pasm glebokosci (toucher). */
function parseTouchLevels(raw: string): { off: number; units: number; tp: number }[] {
  if (!raw.trim()) return [];
  return raw
    .split(",")
    .map((chunk) => chunk.split(":").map((x) => Number(x.trim())))
    .filter((a) => a.length >= 2 && a.every((x) => Number.isFinite(x)))
    .map(([off, units, tp]) => ({ off: off / 10, units, tp: tp || 2 }));
}

/** Drabinka progow "zysk:blokada". */
function parseTiers(raw: string): [number, number][] {
  return raw
    .split(",")
    .map((c) => c.split(":").map(Number))
    .filter((a) => a.length === 2 && a.every(Number.isFinite)) as [number, number][];
}

/* ---------------- STREFA WEJSCIA ---------------- */

function computeZone(sig: ParsedSignal, st: Settings): { low: number; high: number } {
  let low = sig.entryLow ?? 0;
  let high = sig.entryHigh ?? 0;
  const buy = sig.direction === "BUY";

  if (st.entry_offset_dir) {
    // offsety liczone wg JAKOSCI wejscia, nie wg ceny
    if (buy) {
      low -= st.entry_deep_offset;
      high += st.entry_tol_offset;
    } else {
      high += st.entry_deep_offset;
      low -= st.entry_tol_offset;
    }
  } else if (st.custom_entry) {
    high += st.entry_high_offset;
    low += st.entry_low_offset;
  }
  return { low: Math.min(low, high), high: Math.max(low, high) };
}

/* ---------------- OTWIERANIE KOSZYKA ---------------- */

export interface OpenBasketArgs {
  state: BotState;
  sig: ParsedSignal;
  st: Settings;
  lot: number;
  price: number;
  source: string;
  sourceKey: string;
  mode: TradingMode;
  volFactor?: number;
}

export function openBasket(args: OpenBasketArgs): Basket | null {
  const { state: s, sig, st, lot, price, source, sourceKey } = args;
  if (!sig.direction) return null;

  const dir: Direction = sig.direction;
  const buy = dir === "BUY";
  const zone = computeZone(sig, st);

  // TYLKO LIMIT: ignoruj sygnaly rynkowe
  const isLimit = sig.isLimit ?? false;
  if (st.only_limit_signals && !isLimit) return null;

  // minimalna szerokosc SL
  let sl = sig.sl ?? null;
  if (sl !== null && st.sl_min_dist > 0) {
    const mid = (zone.low + zone.high) / 2;
    const want = buy ? mid - st.sl_min_dist : mid + st.sl_min_dist;
    sl = buy ? Math.min(sl, want) : Math.max(sl, want);
  }

  const b: Basket = {
    id: s.nextBasketId++,
    symbol: s.symbol,
    direction: dir,
    isLimit,
    entryLow: sig.entryLow ?? price,
    entryHigh: sig.entryHigh ?? price,
    zoneLow: zone.low,
    zoneHigh: zone.high,
    sl,
    tps: (sig.tps ?? []).slice(),
    tpStage: 0,
    createdAt: now(),
    source,
    sourceKey,
    active: true,
    tickets: [],
    pendingTickets: [],
    events: [],
    riskFree: false,
  };

  s.baskets.push(b);
  bEvt(
    b,
    `Koszyk utworzony · ${dir}${isLimit ? " LIMIT" : ""} · strefa ${zone.low.toFixed(2)}–${zone.high.toFixed(2)}`,
    "entry",
  );

  const volFactor = args.volFactor ?? 1;
  const cmt = commentFor(st, source);

  if (isLimit || (st.auto_limit && (buy ? price > zone.high : price < zone.low))) {
    placeLimitGrid(s, b, st, lot, volFactor, cmt);
  } else {
    // wejscie rynkowe — jedna lub kilka jednostek
    const units = unitsForLevel(st, false, price, b.sl, b.tps[0], volFactor);
    for (let i = 0; i < units; i++) {
      openPosition(s, b, dir, lot, price, b.sl, targetTpFor(b, st, i, units), i, false, cmt);
    }
    bEvt(b, `Wejście rynkowe · ${units} × ${lot.toFixed(2)} lot @ ${price.toFixed(2)}`, "entry");
  }

  return b;
}

function targetTpFor(b: Basket, st: Settings, idx: number, total: number): number | null {
  if (!b.tps.length) return null;
  if (st.all_runners) return b.tps[b.tps.length - 1] + (b.direction === "BUY" ? st.tp_open_offset : -st.tp_open_offset);

  if (st.official_mode && st.official_assign_tps) {
    // kaskada: najgorsze pozycje dostaja najblizszy TP
    const per = Math.max(1, Math.floor(total / Math.max(1, b.tps.length)));
    const tier = Math.min(b.tps.length - 1, Math.floor(idx / per));
    return b.tps[tier];
  }
  if (st.scale_out) {
    const per = Math.max(1, Math.ceil((total * st.scale_out_pct) / 100));
    const tier = Math.min(b.tps.length - 1, Math.floor(idx / per));
    return b.tps[tier];
  }
  return b.tps[b.tps.length - 1];
}

/** place_limit_grid z bot.py — siatka limitow po strefie wejscia. */
function placeLimitGrid(s: BotState, b: Basket, st: Settings, lot: number, volFactor: number, cmt: string) {
  const buy = b.direction === "BUY";
  const step = gridStep(st, true);
  const lo = b.zoneLow;
  const hi = b.zoneHigh;

  const levels: number[] = [];
  const first = Math.ceil(lo / step) * step;
  for (let p = first; p <= hi + 1e-9; p += step) levels.push(Number(p.toFixed(5)));

  if (!levels.length) {
    // fallback na krawedzi — grid_fallback_best_edge naprawia zla krawedz dla SELL
    levels.push(st.grid_fallback_best_edge ? (buy ? lo : hi) : lo);
  }

  // najlepsze wejscia jako pierwsze (BUY: najnizej, SELL: najwyzej)
  levels.sort((a, c) => (buy ? a - c : c - a));

  let idx = 0;
  const total = levels.length;
  for (const lvl of levels) {
    const units = unitsForLevel(st, true, lvl, b.sl, b.tps[0], volFactor);
    for (let u = 0; u < units; u++) {
      const kind: PendingKind = buy ? "BUY_LIMIT" : "SELL_LIMIT";
      addPending(s, b, kind, lot, lvl, b.sl, targetTpFor(b, st, idx, total * units), idx, cmt);
    }
    idx++;
  }

  // TOUCHER — dodatkowe jednostki na szczycie strefy z wlasnym, wczesnym TP
  const touchLevels = parseTouchLevels(st.entry_touch_levels);
  const bands =
    touchLevels.length > 0
      ? touchLevels
      : st.entry_touch_units > 0
        ? [{ off: 0, units: st.entry_touch_units, tp: st.entry_touch_tp }]
        : [];

  for (const band of bands) {
    const px = buy ? hi - band.off : lo + band.off;
    const tp = b.tps[Math.max(0, band.tp - 1)] ?? b.tps[b.tps.length - 1] ?? null;
    for (let u = 0; u < band.units; u++) {
      addPending(s, b, buy ? "BUY_LIMIT" : "SELL_LIMIT", lot, px, b.sl, tp, -1, `${cmt} · TOUCH`);
    }
  }

  bEvt(
    b,
    `Rozstawiono ${b.pendingTickets.length} limitów · krok ${step.toFixed(2)}${
      bands.length ? ` · toucher ${bands.reduce((a, x) => a + x.units, 0)} j.` : ""
    }`,
    "entry",
  );
}

/* ---------------- ZLECENIA ---------------- */

function commentFor(st: Settings, source: string): string {
  if (st.comment_mode === "custom") return st.comment_custom.slice(0, 31);
  return source.slice(0, 31);
}

export function addPending(
  s: BotState,
  b: Basket | null,
  kind: PendingKind,
  volume: number,
  price: number,
  sl: number | null,
  tp: number | null,
  level: number,
  comment = "",
): PendingOrder {
  const o: PendingOrder = {
    ticket: s.nextTicket++,
    symbol: s.symbol,
    kind,
    volume,
    price,
    sl,
    tp,
    placedTime: now(),
    comment: comment || (b ? `B${b.id} ${b.source}`.slice(0, 31) : "manual"),
    basketId: b?.id ?? null,
    level,
    frozen: false,
  };
  s.pendings.push(o);
  b?.pendingTickets.push(o.ticket);
  return o;
}

export function openPosition(
  s: BotState,
  b: Basket | null,
  direction: Direction,
  volume: number,
  price: number,
  sl: number | null,
  tp: number | null,
  level: number,
  toucher: boolean,
  comment = "",
): Position {
  const p: Position = {
    ticket: s.nextTicket++,
    symbol: s.symbol,
    direction,
    volume,
    openPrice: price,
    openTime: now(),
    sl,
    tp,
    vsl: null,
    profit: 0,
    swap: 0,
    commission: 0,
    comment: comment || (b ? `B${b.id} ${b.source}`.slice(0, 31) : "manual"),
    magic: 771001,
    basketId: b?.id ?? null,
    level,
    frozen: false,
    peakPts: 0,
    runner: tp === null,
    toucher,
    lastPeakTime: now(),
  };
  s.positions.push(p);
  b?.tickets.push(p.ticket);
  return p;
}

export function closePosition(s: BotState, ticket: number, price: number, reason: CloseReason): number {
  const i = s.positions.findIndex((p) => p.ticket === ticket);
  if (i < 0) return 0;
  const p = s.positions[i];
  const profit = positionProfit(p, price) + p.swap + p.commission;

  s.closed.unshift({
    ticket: p.ticket,
    symbol: p.symbol,
    direction: p.direction,
    volume: p.volume,
    openPrice: p.openPrice,
    closePrice: price,
    openTime: p.openTime,
    closeTime: now(),
    profit,
    profitBasis: "ReportedNet",
    netProfit: profit,
    swap: p.swap,
    commission: p.commission,
    reason,
    comment: p.comment,
    basketId: p.basketId,
  });
  if (s.closed.length > 600) s.closed.pop();

  s.positions.splice(i, 1);
  s.balance += profit;

  const b = s.baskets.find((x) => x.id === p.basketId);
  if (b) {
    b.tickets = b.tickets.filter((t) => t !== ticket);
    bEvt(
      b,
      `Zamknięto #${ticket} @ ${price.toFixed(2)} · ${profit >= 0 ? "+" : ""}${profit.toFixed(2)} $ (${reason})`,
      profit >= 0 ? "tp" : "sl",
    );
  }
  return profit;
}

export function deletePending(s: BotState, ticket: number, status: PendingHistoryItem["status"]) {
  const i = s.pendings.findIndex((o) => o.ticket === ticket);
  if (i < 0) return;
  const o = s.pendings[i];
  s.pendingHistory.unshift({
    ticket: o.ticket,
    symbol: o.symbol,
    kind: o.kind,
    volume: o.volume,
    price: o.price,
    sl: o.sl,
    tp: o.tp,
    placedTime: o.placedTime,
    endTime: now(),
    status,
    basketId: o.basketId,
  });
  if (s.pendingHistory.length > 600) s.pendingHistory.pop();
  s.pendings.splice(i, 1);
  const b = s.baskets.find((x) => x.id === o.basketId);
  if (b) b.pendingTickets = b.pendingTickets.filter((t) => t !== ticket);
}

/* ---------------- PETLA ZARZADZANIA ---------------- */

export interface TickResult {
  filled: number;
  closed: number;
  halted?: string;
}

export function tickBot(s: BotState, price: number, st: Settings, mode: TradingMode): TickResult {
  const res: TickResult = { filled: 0, closed: 0 };
  const t = now();

  /* --- 1. wypelnienia pendingow --- */
  for (const o of [...s.pendings]) {
    const buy = o.kind.startsWith("BUY");
    const hit =
      o.kind === "BUY_LIMIT"
        ? price <= o.price
        : o.kind === "SELL_LIMIT"
          ? price >= o.price
          : o.kind === "BUY_STOP"
            ? price >= o.price
            : price <= o.price;

    if (!hit) continue;

    // straznik ekspozycji
    if (st.max_open_positions > 0) {
      const count = s.positions.length + (st.exposure_count_pendings ? 0 : 0);
      if (count >= st.max_open_positions) continue;
    }

    const b = s.baskets.find((x) => x.id === o.basketId) ?? null;
    deletePending(s, o.ticket, "FILLED");
    openPosition(
      s,
      b,
      buy ? "BUY" : "SELL",
      o.volume,
      o.price,
      o.sl,
      o.tp,
      o.level,
      o.comment.includes("TOUCH"),
      o.comment,
    );
    res.filled++;
    if (b) bEvt(b, `Fill @ ${o.price.toFixed(2)} · ${o.volume.toFixed(2)} lot`, "entry");
  }

  /* --- 2. TTL pendingow --- */
  if (st.pending_ttl_h > 0) {
    const maxAge = st.pending_ttl_h * 3600_000;
    for (const o of [...s.pendings]) {
      if (t - o.placedTime > maxAge) deletePending(s, o.ticket, "EXPIRED");
    }
  }

  /* --- 3. aktualizacja pozycji: P&L, szczyt, SL/TP, trailing --- */
  const vslDue = st.vsl_eval_s <= 0 || t - s.lastVslEval >= st.vsl_eval_s * 1000;
  if (vslDue) s.lastVslEval = t;

  for (const p of [...s.positions]) {
    p.profit = positionProfit(p, price);
    const pts = positionPoints(p, price);
    if (pts > p.peakPts) {
      p.peakPts = pts;
      p.lastPeakTime = t;
    }

    const buy = p.direction === "BUY";

    /* SL / TP brokera */
    if (p.sl !== null && (buy ? price <= p.sl : price >= p.sl)) {
      closePosition(s, p.ticket, p.sl, "SL");
      res.closed++;
      continue;
    }
    if (p.tp !== null && (buy ? price >= p.tp : price <= p.tp)) {
      closePosition(s, p.ticket, p.tp, "TP");
      res.closed++;
      continue;
    }

    /* wirtualny SL — egzekucja po rynku wg kadencji */
    if (st.virtual_sl && p.vsl !== null && vslDue && (buy ? price <= p.vsl : price >= p.vsl)) {
      closePosition(s, p.ticket, price, "VSL");
      res.closed++;
      continue;
    }

    if (p.frozen || mode === "MANUAL") continue;

    /* BE-LOCK: po +X pkt SL na wejscie */
    if (st.be_lock && pts >= st.be_lock_points) {
      const be = p.openPrice;
      if (p.sl === null || (buy ? p.sl < be : p.sl > be)) {
        if (safeSetSl(p, be, price, st.sim_stops_level)) p.runner = p.tp === null;
      }
    }

    /* SAFETY TRAILING STOP */
    if (st.runner_trail && p.peakPts >= st.runner_trail_start) {
      const cand = trailCandidate(st, p, price);
      if (cand !== null) {
        const better = p.sl === null || (buy ? cand > p.sl : cand < p.sl);
        const aboveBe = buy ? cand >= p.openPrice : cand <= p.openPrice;
        if (better && aboveBe) {
          // MIN. DYSTANS SL — nie wysylaj SL, ktory broker odrzuci
          const minD = Math.max(st.trail_min_dist, st.sim_stops_level);
          const safe = buy ? Math.min(cand, price - minD) : Math.max(cand, price + minD);
          if (buy ? safe > (p.sl ?? -Infinity) : safe < (p.sl ?? Infinity)) {
            if (st.virtual_sl_all && st.virtual_sl) p.vsl = cand;
            else safeSetSl(p, safe, price, st.sim_stops_level);
          }
        }
      }
    }

    /* HARVEST — zamknij po cofnieciu o X% szczytu */
    if (st.harvest && p.peakPts >= st.harvest_start) {
      const giveBack = p.peakPts - pts;
      if (giveBack >= (p.peakPts * st.harvest_retrace_pct) / 100) {
        closePosition(s, p.ticket, price, "HARVEST");
        res.closed++;
        continue;
      }
    }

    /* OUT-AT-ENTRY — wisi bez zysku */
    if (st.oae_timeout_min > 0) {
      const ageMin = (t - p.openTime) / 60000;
      if (ageMin >= st.oae_timeout_min && pts < st.oae_profit_min) {
        closePosition(s, p.ticket, price, "OAE");
        res.closed++;
        continue;
      }
    }

    /* STAGNACJA — zysk bez nowego szczytu przez X minut */
    const stagMin = (t - p.lastPeakTime) / 60000;
    const stale1 = st.stale_take_min > 0 && stagMin >= st.stale_take_min && pts >= st.stale_take_profit;
    const stale2 = st.stale_take_min2 > 0 && stagMin >= st.stale_take_min2 && pts >= st.stale_take_profit2;
    if (stale1 || stale2) {
      closePosition(s, p.ticket, price, "STALE");
      res.closed++;
      continue;
    }
  }

  /* --- 4. wykrywanie TP z CENY (tp_detect_price) --- */
  if (st.tp_detect_price && mode !== "MANUAL") {
    for (const b of s.baskets) {
      if (!b.active) continue;
      const next = b.tps[b.tpStage];
      if (next === undefined) continue;
      const reached = b.direction === "BUY" ? price >= next : price <= next;
      if (reached) advanceTpStage(s, b, st, price, "cena");
    }
  }

  /* --- 5. sprzatanie martwych koszykow --- */
  for (const b of s.baskets) {
    if (!b.active) continue;
    if (b.tickets.length === 0 && b.pendingTickets.length === 0 && t - b.createdAt > 20_000) {
      b.active = false;
      bEvt(b, "Koszyk zakończony (brak pozycji i limitów)", "info");
    }
  }
  // usun bardzo stare, nieaktywne koszyki
  if (s.baskets.length > 40) {
    s.baskets = s.baskets.filter((b) => b.active || now() - b.createdAt < 3600_000).slice(-40);
  }

  return res;
}

/**
 * Ustawia SL tylko wtedy, gdy leży po WŁAŚCIWEJ stronie ceny. Broker odrzuca
 * stop po złej stronie rynku, a w symulacji taki SL zamykałby pozycję po cenie
 * lepszej niż rynkowa — czyli po cenie, której nigdy nie było.
 */
function safeSetSl(p: Position, sl: number, price: number, stopsLevel: number): boolean {
  const ok = p.direction === "BUY" ? sl <= price - stopsLevel : sl >= price + stopsLevel;
  if (!ok) return false;
  p.sl = sl;
  return true;
}

/** Kandydat na nowy SL wg trybu trailingu (gap / lock_pct / tiered). */
function trailCandidate(st: Settings, p: Position, price: number): number | null {
  const buy = p.direction === "BUY";
  const mode = st.trail_split && p.runner ? st.trail_runner_mode : st.trail_mode;

  if (mode === "gap") {
    const gap = st.trail_split && p.runner ? st.trail_runner_gap : st.runner_trail_gap;
    return buy ? price - gap : price + gap;
  }
  if (mode === "lock_pct") {
    const pct = st.trail_split && p.runner ? st.trail_runner_lock_pct : st.trail_lock_pct;
    const lock = (p.peakPts * pct) / 100;
    return buy ? p.openPrice + lock : p.openPrice - lock;
  }
  // tiered
  const tiers = parseTiers(st.trail_split && p.runner ? st.trail_runner_tiers : st.trail_tiers);
  let lock: number | null = null;
  for (const [thr, keep] of tiers) if (p.peakPts >= thr) lock = keep;
  if (lock === null) return null;
  return buy ? p.openPrice + lock : p.openPrice - lock;
}

/* ---------------- ETAPY TP ---------------- */

export function advanceTpStage(s: BotState, b: Basket, st: Settings, price: number, src: string) {
  const stage = b.tpStage;
  const level = b.tps[stage];
  if (level === undefined) return;
  b.tpStage = stage + 1;
  bEvt(b, `TP${stage + 1} osiągnięty (${src}) @ ${level.toFixed(2)}`, "tp");

  const positions = s.positions.filter((p) => p.basketId === b.id && !p.frozen);
  const buy = b.direction === "BUY";

  /* BE na TP1 (system tradera) */
  if (st.be_at_tp1 && stage === 0) {
    let n = 0;
    for (const p of positions) if (safeSetSl(p, p.openPrice, price, st.sim_stops_level)) n++;
    bEvt(b, `SL na BE dla ${n}/${positions.length} poz. (be_at_tp1)`, "mgmt");
  }

  /* Kasowanie limitow po TP1 / TP2 */
  const cancelStage = st.valid_till_tp2 ? 2 : 1;
  if (!st.pending_never_cancel && b.tpStage >= cancelStage) {
    const toKill = [...b.pendingTickets];
    for (const tk of toKill) deletePending(s, tk, "CANCELLED");
    if (toKill.length) bEvt(b, `Skasowano ${toKill.length} niezafillowanych limitów`, "mgmt");
  }

  /* Harmonogram zamykania */
  if (st.official_mode) {
    const n = positions.length;
    if (!n) return;
    let closeCount: number;
    if (st.official_use_counts) {
      const counts = st.official_counts.split(",").map((x) => Number(x.trim()) || 0);
      closeCount = counts[stage] ?? (st.official_spp ? counts[counts.length - 1] : 0);
    } else {
      const pct =
        stage === 0
          ? st.official_pct_tp1
          : stage === 1
            ? st.official_pct_tp2
            : stage === 2
              ? st.official_pct_tp3
              : st.official_spp
                ? st.official_pct_spp
                : 0;
      const raw = (n * pct) / 100;
      closeCount =
        st.official_round === "up" ? Math.ceil(raw) : st.official_round === "down" ? Math.floor(raw) : Math.round(raw);
    }
    closeCount = Math.max(0, Math.min(n - (st.official_close_last ? 0 : 0), closeCount));
    if (closeCount > 0) {
      const sorted = [...positions].sort((a, c) => positionProfit(a, price) - positionProfit(c, price));
      for (let i = 0; i < closeCount && i < sorted.length; i++) {
        closePosition(s, sorted[i].ticket, price, "PARTIAL");
      }
      bEvt(b, `Zainkasowano ${closeCount} poz. na TP${stage + 1}`, "tp");
    }
  } else if (st.scale_out) {
    const n = positions.length;
    const raw = (n * st.scale_out_pct) / 100;
    const cnt = st.scale_out_round === "up" ? Math.ceil(raw) : Math.floor(raw);
    const sorted = [...positions].sort((a, c) =>
      st.scale_out_from === "worst"
        ? positionProfit(a, price) - positionProfit(c, price)
        : positionProfit(c, price) - positionProfit(a, price),
    );
    for (let i = 0; i < cnt && i < sorted.length - 1; i++) closePosition(s, sorted[i].ticket, price, "PARTIAL");
    if (cnt) bEvt(b, `SCALE-OUT: zamknięto ${cnt} poz. (${st.scale_out_pct}%)`, "tp");
  }

  /* SMART SL / drabinka SL */
  const left = s.positions.filter((p) => p.basketId === b.id && !p.frozen);
  if (st.smart_sl || st.ladder_from_tp > 0) {
    const lagIdx = Math.max(0, b.tpStage - 1 - st.ladder_lag);
    const anchor =
      st.ladder_from_tp > 0 && b.tpStage >= st.ladder_from_tp
        ? b.tps[lagIdx]
        : st.trail_after_tp2 && b.tpStage < 2
          ? undefined
          : b.tps[Math.max(0, b.tpStage - 2)];
    if (anchor !== undefined) {
      const off = st.ladder_offset;
      const target = buy ? anchor - off : anchor + off;
      let moved = 0;
      for (const p of left) {
        const better = p.sl === null || (buy ? target > p.sl : target < p.sl);
        if (better && safeSetSl(p, target, price, st.sim_stops_level)) moved++;
      }
      if (moved) bEvt(b, `Drabinka SL → ${target.toFixed(2)} (${moved} poz.)`, "mgmt");
    }
  }

  /* TP runnerow — kolejny cel lub zamrozenie */
  const nextTp = b.tps[b.tpStage];
  for (const p of left) {
    if (p.tp === null) continue;
    if (nextTp !== undefined) {
      p.tp = st.all_runners ? b.tps[b.tps.length - 1] : nextTp;
    } else if (!st.tp_freeze_after_ladder) {
      p.tp = buy ? p.tp + st.tp_open_offset : p.tp - st.tp_open_offset;
    }
  }
}

/* ---------------- KOMUNIKATY ZARZADZAJACE ---------------- */

/**
 * RISK FREE — semantyka ATFX (opis od providera).
 *
 * Komunikat podaje POZIOM, np. „RISK FREE 4000" (zwykle dolna krawędź strefy
 * dla BUY). Mechanika jest koszykowa, nie per-pozycja:
 *
 *  1. Zamykane są WSZYSTKIE wejścia koszyka — i te w zysku, i te w stracie.
 *     Zysk głębokich wejść (bliżej poziomu RF) kompensuje stratę wejść
 *     płytkich, więc koszyk wychodzi mniej więcej na zero lub na małym plusie.
 *  2. Zostaje pozycja NAJBLIŻSZA poziomowi RISK FREE, a jej SL ląduje na
 *     breakeven.
 *
 * Efekt: najgorszy przypadek = brak straty, najlepszy = runner jedzie dalej
 * na TP2/TP3 i wyżej. Sens całości to ochrona kapitału przy zachowaniu
 * potencjału runnera — dlatego selekcja idzie po ODLEGŁOŚCI OD POZIOMU, a nie
 * po wielkości zysku.
 */
export function handleRiskFree(s: BotState, b: Basket, st: Settings, price: number, level?: number | null) {
  if (st.ignore_risk_free) {
    bEvt(b, "RISK FREE zignorowany (ustawienie)", "mgmt");
    return;
  }

  const positions = s.positions.filter((p) => p.basketId === b.id);
  if (!positions.length) {
    bEvt(b, "RISK FREE bez otwartych pozycji — pominięto", "mgmt");
    return;
  }

  // Poziom odniesienia: z komunikatu, a gdy go nie podano — krawędź strefy
  // po stronie najlepszych wejść (BUY: dół, SELL: góra).
  const ref = level ?? (b.direction === "BUY" ? b.zoneLow : b.zoneHigh);

  // Średnia ważona wejść koszyka — to ona decyduje, czy zamknięcie wychodzi
  // na zero (informacyjnie, do logu koszyka).
  const totalVol = positions.reduce((a, p) => a + p.volume, 0);
  const vwap = positions.reduce((a, p) => a + p.openPrice * p.volume, 0) / Math.max(1e-9, totalVol);

  // Runnery: N pozycji najbliższych poziomowi RISK FREE.
  const keep = Math.max(1, st.risk_free_runners);
  const byProximity = [...positions].sort(
    (a, c) => Math.abs(a.openPrice - ref) - Math.abs(c.openPrice - ref),
  );
  const runners = byProximity.slice(0, keep);
  const runnerSet = new Set(runners.map((p) => p.ticket));

  // 1) zamknij CAŁĄ resztę po rynku — zyski kompensują straty
  let realized = 0;
  let closed = 0;
  for (const p of positions) {
    if (runnerSet.has(p.ticket)) continue;
    realized += closePosition(s, p.ticket, price, "RISK_FREE");
    closed++;
  }

  // 2) runnery dostają SL na breakeven (o ile BE leży po właściwej stronie
  //    ceny — inaczej broker odrzuciłby stop, a symulacja zamykałaby pozycję
  //    po cenie lepszej od rynkowej)
  let armed = 0;
  for (const p of runners) {
    if (safeSetSl(p, p.openPrice, price, st.sim_stops_level)) armed++;
    if (st.risk_free_mode === "all_runners") {
      p.tp = null;
      p.runner = true;
    }
  }

  b.riskFree = true;
  bEvt(
    b,
    `RISK FREE @ ${ref.toFixed(2)} · zamknięto ${closed} poz. (${realized >= 0 ? "+" : ""}${realized.toFixed(2)} $, ` +
      `VWAP wejść ${vwap.toFixed(2)}) · runner${runners.length > 1 ? "y" : ""}: ${runners.length}, ` +
      `SL na BE: ${armed}`,
    "mgmt",
  );
}

export function handleOutAtEntry(s: BotState, b: Basket, st: Settings, price: number) {
  if (st.ignore_out_at_entry) {
    bEvt(b, "OUT AT ENTRY zignorowany (ustawienie)", "mgmt");
    return;
  }
  for (const p of s.positions.filter((x) => x.basketId === b.id)) {
    closePosition(s, p.ticket, price, "OAE");
  }
  for (const tk of [...b.pendingTickets]) deletePending(s, tk, "CANCELLED");
  b.active = false;
  bEvt(b, "OUT AT ENTRY — koszyk zamknięty", "mgmt");
}

export function handleCancel(s: BotState, b: Basket) {
  for (const tk of [...b.pendingTickets]) deletePending(s, tk, "CANCELLED");
  bEvt(b, "Limity anulowane komunikatem z kanału", "mgmt");
  if (!b.tickets.length) b.active = false;
}

export function handleSlHit(s: BotState, b: Basket, st: Settings, price: number) {
  // weryfikacja falszywego "SL HIT" po cenie
  if (st.sl_hit_verify_tol > 0 && b.sl !== null) {
    const far = b.direction === "BUY" ? price - b.sl : b.sl - price;
    if (far > st.sl_hit_verify_tol) {
      bEvt(b, `SL HIT odrzucony — cena ${far.toFixed(2)} od SL po stronie zysku`, "mgmt");
      return;
    }
  }
  for (const tk of [...b.pendingTickets]) deletePending(s, tk, "CANCELLED");
  bEvt(b, "SL HIT z kanału — limity skasowane", "sl");
}

export function closeBasket(s: BotState, b: Basket, price: number, reason: CloseReason = "BASKET") {
  for (const p of s.positions.filter((x) => x.basketId === b.id)) closePosition(s, p.ticket, price, reason);
  for (const tk of [...b.pendingTickets]) deletePending(s, tk, "CANCELLED");
  b.active = false;
  bEvt(b, "Koszyk zamknięty ręcznie", "mgmt");
}

export function closeAll(s: BotState, price: number, reason: CloseReason) {
  for (const p of [...s.positions]) closePosition(s, p.ticket, price, reason);
  for (const o of [...s.pendings]) deletePending(s, o.ticket, "CANCELLED");
  for (const b of s.baskets) b.active = false;
  logEvt(s, `Zamknięto wszystko (${reason})`, "mgmt");
}

/* ---------------- BRAMKI WEJSC ---------------- */

/** entries_blocked z bot.py — zwraca powod blokady albo pusty string. */
export function entriesBlocked(s: BotState, st: Settings, pnlToday: number, lot: number): string {
  const t = new Date();
  if (s.pausedUntil > now()) {
    const left = Math.ceil((s.pausedUntil - now()) / 60000);
    return `pauza po serii strat (${left} min)`;
  }
  if (st.session_filter && !hoursOk(st.session_hours, t.getHours())) {
    return `poza sesją (${st.session_hours})`;
  }
  if (st.day_target_usd > 0) {
    const scale = st.day_target_scale_lot || st.usd_scale_with_lot ? lot / 0.01 : 1;
    if (pnlToday >= st.day_target_usd * scale) return `cel dzienny osiągnięty (${(st.day_target_usd * scale).toFixed(0)} $)`;
  }
  if (st.flat_weekend && t.getDay() === 5 && t.getHours() >= st.flat_weekend_hour) {
    return "flat przed weekendem";
  }
  if (st.max_open_positions > 0) {
    const n = s.positions.length + (st.exposure_count_pendings ? s.pendings.length : 0);
    if (n >= st.max_open_positions) return `limit ekspozycji (${n}/${st.max_open_positions})`;
  }
  return "";
}

function hoursOk(spec: string, hour: number): boolean {
  for (const part of spec.split(",")) {
    const [a, b] = part.split("-").map((x) => Number(x.trim()));
    if (Number.isFinite(a) && Number.isFinite(b) && hour >= a && hour < b) return true;
    if (Number.isFinite(a) && !Number.isFinite(b) && hour === a) return true;
  }
  return false;
}

/** Filtr jakosci sygnalu po tagach (signal_filter). */
export function signalTagBlocked(text: string, st: Settings): string {
  if (!st.signal_filter) return "";
  const T = text.toUpperCase();
  const skip = st.skip_tags.split(",").map((x) => x.trim().toUpperCase()).filter(Boolean);
  for (const tag of skip) if (T.includes(tag)) return `tag pominięty: ${tag}`;
  const req = st.require_tags.split(",").map((x) => x.trim().toUpperCase()).filter(Boolean);
  if (req.length && !req.some((tag) => T.includes(tag))) return "brak wymaganego tagu";
  return "";
}

/** Reżim zmienności: mnożnik jednostek w sztormie (vol_units_factor). */
export function volUnitsFactor(st: Settings, range: number): number {
  if (st.vol_window_min <= 0) return 1;
  return range >= st.vol_range_usd ? st.vol_units_mult : 1;
}
