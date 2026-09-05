import type { Candle, Quote, Timeframe } from "@/types";
import { getSymbol } from "@/data/symbols";

/* ============================================================
   GENERATOR RYNKU
   Deterministyczny (seed z nazwy symbolu) szereg swiec 1-minutowych
   + zywy tick. Zastepuje strumien cen z MT5 — pozwala miec dzialajacy
   wykres, P&L i wypelnienia zlecen bez zadnego backendu.
   ============================================================ */

const TF_MINUTES: Record<Timeframe, number> = {
  "1m": 1,
  "5m": 5,
  "15m": 15,
  "1h": 60,
  "4h": 240,
  "1d": 1440,
};

export const TIMEFRAMES: Timeframe[] = ["1m", "5m", "15m", "1h", "4h", "1d"];

/** xorshift32 — szybki, deterministyczny PRNG. */
function makeRng(seed: number) {
  let s = seed >>> 0 || 0x9e3779b9;
  return () => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 0xffffffff;
  };
}

function hashString(str: string): number {
  let h = 2166136261;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** Rozklad normalny (Box-Muller) na bazie PRNG. */
function gauss(rng: () => number): number {
  const u = Math.max(rng(), 1e-9);
  const v = rng();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
}

const BARS_1M = 4320; // 3 dni handlowe minut

/**
 * Profil zmiennosci wewnatrzdniowej — sesje azjatycka / londynska / NY.
 * Godzina UTC -> mnoznik zmiennosci. Odwzorowuje realny rytm XAUUSD.
 */
function sessionFactor(hourUtc: number): number {
  if (hourUtc >= 7 && hourUtc < 11) return 1.55; // Londyn open
  if (hourUtc >= 12 && hourUtc < 16) return 1.85; // NY overlap
  if (hourUtc >= 16 && hourUtc < 20) return 1.25; // NY afternoon
  if (hourUtc >= 20 && hourUtc < 23) return 0.75;
  return 0.5; // Azja / noc
}

export interface SeriesState {
  symbol: string;
  base1m: Candle[];
  lastPrice: number;
  dayOpen: number;
  dayHigh: number;
  dayLow: number;
  /** dryf krotkoterminowy — zapewnia realistyczne "fale" zamiast bialego szumu */
  drift: number;
  rng: () => number;
}

const series = new Map<string, SeriesState>();

function buildSeries(symbol: string): SeriesState {
  const meta = getSymbol(symbol);
  const rng = makeRng(hashString(symbol));
  const now = Date.now();
  const startT = Math.floor(now / 60000) * 60000 - BARS_1M * 60000;

  // zmiennosc na minute wyprowadzona ze zmiennosci dziennej w %
  const sigma1m = (meta.vol / 100) * meta.base / Math.sqrt(1440);

  const candles: Candle[] = [];
  let price = meta.base * (1 - (meta.vol / 100) * 0.55);
  let drift = 0;

  for (let i = 0; i < BARS_1M; i++) {
    const t = startT + i * 60000;
    const hourUtc = new Date(t).getUTCHours();
    const sf = sessionFactor(hourUtc);

    // proces powracajacy do sredniej dla dryfu -> trendy + korekty
    drift = drift * 0.985 + gauss(rng) * sigma1m * 0.28;
    const step = drift + gauss(rng) * sigma1m * sf;

    const o = price;
    let c = o + step;
    if (c <= 0) c = o;

    const wick = Math.abs(gauss(rng)) * sigma1m * sf * 0.75;
    const h = Math.max(o, c) + wick * rng();
    const l = Math.min(o, c) - wick * rng();
    const v = Math.round((0.4 + rng() * 0.9) * sf * 1000);

    candles.push({ t, o, h, l, c, v });
    price = c;
  }

  // delikatne skalowanie na cene bazowa, zeby ostatnia swieca byla ~base
  const scale = meta.base / price;
  if (Number.isFinite(scale) && scale > 0) {
    for (const k of candles) {
      k.o *= scale;
      k.h *= scale;
      k.l *= scale;
      k.c *= scale;
    }
    price = meta.base;
  }

  const dayStart = candles.length - 1440;
  const todayBars = candles.slice(Math.max(0, dayStart));

  return {
    symbol,
    base1m: candles,
    lastPrice: price,
    dayOpen: todayBars[0]?.o ?? price,
    dayHigh: Math.max(...todayBars.map((k) => k.h)),
    dayLow: Math.min(...todayBars.map((k) => k.l)),
    drift,
    rng,
  };
}

export function getSeries(symbol: string): SeriesState {
  let s = series.get(symbol);
  if (!s) {
    s = buildSeries(symbol);
    series.set(symbol, s);
  }
  return s;
}

/**
 * Jeden tick rynkowy: aktualizuje ostatnia swiece 1m (lub tworzy nowa).
 * @param dtMs czas jaki uplynal od poprzedniego ticku
 */
export function tickSeries(symbol: string, dtMs: number): number {
  const s = getSeries(symbol);
  const meta = getSymbol(symbol);
  const now = Date.now();
  const hourUtc = new Date(now).getUTCHours();
  const sf = sessionFactor(hourUtc);

  const sigma1m = (meta.vol / 100) * meta.base / Math.sqrt(1440);
  const scale = Math.sqrt(Math.min(dtMs, 5000) / 60000);

  s.drift = s.drift * 0.995 + gauss(s.rng) * sigma1m * 0.1 * scale;
  const step = (s.drift * scale + gauss(s.rng) * sigma1m * sf * scale) * 1.0;

  let price = s.lastPrice + step;
  if (price <= 0) price = s.lastPrice;
  s.lastPrice = price;

  const bucket = Math.floor(now / 60000) * 60000;
  const last = s.base1m[s.base1m.length - 1];

  if (last && last.t === bucket) {
    last.c = price;
    if (price > last.h) last.h = price;
    if (price < last.l) last.l = price;
    last.v += 1;
  } else {
    s.base1m.push({ t: bucket, o: price, h: price, l: price, c: price, v: 1 });
    if (s.base1m.length > BARS_1M + 240) s.base1m.shift();
  }

  if (price > s.dayHigh) s.dayHigh = price;
  if (price < s.dayLow) s.dayLow = price;

  return price;
}

/** Agregacja swiec 1m do wyzszego interwalu. */
export function getCandles(symbol: string, tf: Timeframe, limit = 600): Candle[] {
  const s = getSeries(symbol);
  const mins = TF_MINUTES[tf];
  if (mins === 1) return s.base1m.slice(-limit);

  const out: Candle[] = [];
  const bucketMs = mins * 60000;
  let cur: Candle | null = null;

  for (const k of s.base1m) {
    const b = Math.floor(k.t / bucketMs) * bucketMs;
    if (!cur || cur.t !== b) {
      if (cur) out.push(cur);
      cur = { t: b, o: k.o, h: k.h, l: k.l, c: k.c, v: k.v };
    } else {
      cur.h = Math.max(cur.h, k.h);
      cur.l = Math.min(cur.l, k.l);
      cur.c = k.c;
      cur.v += k.v;
    }
  }
  if (cur) out.push(cur);
  return out.slice(-limit);
}

export function getQuote(symbol: string): Quote {
  const s = getSeries(symbol);
  const meta = getSymbol(symbol);
  const spread = meta.base * 0.00006 + (meta.digits <= 2 ? 0.08 : 0);
  const bid = s.lastPrice;
  const ask = bid + spread;
  const change = bid - s.dayOpen;
  return {
    symbol,
    bid,
    ask,
    spread,
    time: Date.now(),
    change,
    changePct: (change / s.dayOpen) * 100,
    dayHigh: s.dayHigh,
    dayLow: s.dayLow,
  };
}

export function fmtPrice(symbol: string, v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  const d = getSymbol(symbol).digits;
  return v.toLocaleString("pl-PL", { minimumFractionDigits: d, maximumFractionDigits: d });
}

/** Ile USD daje ruch o `pts` punktow ceny przy danym wolumenie. */
export function pointsToUsd(symbol: string, pts: number, volume: number): number {
  const meta = getSymbol(symbol);
  return pts * meta.contractSize * volume;
}
