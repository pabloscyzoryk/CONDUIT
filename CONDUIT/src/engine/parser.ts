import type { ParsedSignal, SignalType } from "@/types";

/* ============================================================
   PARSER SYGNALOW
   Odwzorowanie `parse_signals` / silnika "atfx" z bot.py.
   Jedna wiadomosc moze zawierac kilka sygnalow (np. wejscie +
   korekta TP), dlatego funkcja zwraca liste.
   ============================================================ */

const NUM = String.raw`(\d{1,6}(?:[.,]\d{1,5})?)`;

function toNum(s: string | undefined): number | null {
  if (!s) return null;
  const v = Number(s.replace(",", "."));
  return Number.isFinite(v) ? v : null;
}

/** Wyciaga wszystkie poziomy TP z tresci (kolejnosc = drabinka). */
function extractTps(text: string): number[] {
  const tps: number[] = [];

  // "TP 4122" / "TP1 4122" / "🎯 4122" / "TAKE PROFIT 4122"
  //
  // `\b` przed grupą jest KONIECZNE. Bez niego opcjonalny numer celu (`\d?`)
  // zjada pierwszą cyfrę ceny: w „TP 4673" dopasowywał „4", a grupa łapała
  // „673" — bot dostawał cel 673 zamiast 4673. Granica słowa wymusza nawrót
  // i oddanie cyfry z powrotem do liczby.
  const re = /(?:TP\s*\d?|TAKE\s*PROFIT|🎯)\s*[:@=-]?\s*\b(\d{1,6}(?:[.,]\d{1,5})?)/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const v = toNum(m[1]);
    if (v !== null) tps.push(v);
  }

  // wariant "TP 4122 / 4127 / 4133" oraz "TP: 4122 4127 4133"
  const multi = /(?:TP|TAKE\s*PROFIT)\s*[:@=-]?\s*((?:\d{1,6}(?:[.,]\d{1,5})?[\s/,-]+){1,6}\d{1,6}(?:[.,]\d{1,5})?)/i.exec(
    text,
  );
  if (multi && tps.length <= 1) {
    const parts = multi[1].split(/[\s/,-]+/).map(toNum).filter((v): v is number => v !== null);
    if (parts.length > tps.length) return dedupe(parts);
  }

  return dedupe(tps);
}

function dedupe(arr: number[]): number[] {
  const out: number[] = [];
  for (const v of arr) if (!out.includes(v)) out.push(v);
  return out;
}

function extractSl(text: string): number | null {
  const m =
    /(?:\bSL\b|STOP\s*LOSS|⛔️?|🛑)\s*[:@=-]?\s*(\d{1,6}(?:[.,]\d{1,5})?)/i.exec(text);
  return m ? toNum(m[1]) : null;
}

/**
 * Format RM podaje TP/SL jako DYSTANS W PIPSACH, nie jako poziom ceny
 * (np. „SL 80 PIPS"). Zamieniamy taki zapis na poziom względem wejścia —
 * 1 pips złota = 0.10 $.
 */
const PIP = 0.1;

function isPipDistance(text: string, value: number, ref: number): boolean {
  return /PIPS?/i.test(text) && ref > 0 && value < ref * 0.25;
}

function pipsToLevel(value: number, ref: number, dir: 1 | -1): number {
  return ref + dir * value * PIP;
}

/**
 * Rozpoznaje wejscie: kierunek, typ (rynek / limit) i strefe.
 * Wspiera: "BUY LIMITS GOLD @ 4118/4112 AREA", "SELL 4131 — 4134",
 * "BUY XAUUSD 4120", "BUY LIMITS GOLD @ 4118 - 4112".
 */
function parseEntry(text: string): ParsedSignal | null {
  const t = text.toUpperCase();

  const zoneRe = new RegExp(
    String.raw`\b(BUY|SELL)\s*(LIMITS?|STOPS?)?\s*(?:GOLD|XAUUSD|XAU)?\s*[@:]?\s*` +
      NUM +
      String.raw`\s*(?:[/–—\-]|\bTO\b)\s*` +
      NUM,
    "i",
  );
  const z = zoneRe.exec(t);
  if (z) {
    const dir = z[1].toUpperCase() as "BUY" | "SELL";
    const kind = (z[2] || "").toUpperCase();
    const a = toNum(z[3])!;
    const b = toNum(z[4])!;
    const mid = (a + b) / 2;
    return {
      type: "ENTRY",
      direction: dir,
      isLimit: kind.startsWith("LIMIT"),
      entryLow: Math.min(a, b),
      entryHigh: Math.max(a, b),
      ...normalizeTargets(text, extractSl(text), extractTps(text), mid, dir),
      raw: text,
    };
  }

  const singleRe = new RegExp(
    String.raw`\b(BUY|SELL)\s*(LIMITS?|STOPS?)?\s*(?:GOLD|XAUUSD|XAU|NOW)?\s*[@:]?\s*` + NUM,
    "i",
  );
  const s = singleRe.exec(t);
  if (s) {
    const dir = s[1].toUpperCase() as "BUY" | "SELL";
    const kind = (s[2] || "").toUpperCase();
    const p = toNum(s[3])!;
    return {
      type: "ENTRY",
      direction: dir,
      isLimit: kind.startsWith("LIMIT"),
      entryLow: p,
      entryHigh: p,
      ...normalizeTargets(text, extractSl(text), extractTps(text), p, dir),
      raw: text,
    };
  }

  // samo "BUY NOW" / "SELL NOW" bez ceny
  const bare = /\b(BUY|SELL)\s+(?:NOW|GOLD|XAUUSD)\b/i.exec(t);
  if (bare) {
    return {
      type: "ENTRY",
      direction: bare[1].toUpperCase() as "BUY" | "SELL",
      isLimit: false,
      sl: extractSl(text),
      tps: extractTps(text),
      raw: text,
    };
  }

  return null;
}

/** Zamienia pipsowe TP/SL na poziomy ceny względem środka strefy wejścia. */
function normalizeTargets(
  text: string,
  sl: number | null,
  tps: number[],
  ref: number,
  dir: "BUY" | "SELL",
): { sl: number | null; tps: number[] } {
  const up: 1 | -1 = dir === "BUY" ? 1 : -1;
  const outSl = sl !== null && isPipDistance(text, sl, ref) ? pipsToLevel(sl, ref, up === 1 ? -1 : 1) : sl;
  const outTps = tps.map((t) => (isPipDistance(text, t, ref) ? pipsToLevel(t, ref, up) : t));
  return { sl: outSl, tps: outTps };
}

/**
 * Glowna funkcja: zwraca liste rozpoznanych sygnalow.
 * Kolejnosc sprawdzania odwzorowuje priorytety z bot.py — komunikaty
 * zarzadzajace maja pierwszenstwo przed wejsciem.
 */
export function parseSignals(text: string): ParsedSignal[] {
  const out: ParsedSignal[] = [];
  const t = text.toUpperCase();

  // --- SL HIT ---
  if (/\bSL\s*HIT\b|\bSTOP\s*LOSS\s*HIT\b/.test(t)) {
    out.push({ type: "SL_HIT", raw: text });
  }

  // --- TP HIT (numer / "TP2 AND 3 HIT" / "+30 PIPS HIT") ---
  const tp2 = /TP\s*(\d)\s*(?:AND|&|,|\+)\s*(\d)\s*HIT/.exec(t);
  if (tp2) {
    out.push({ type: "TP_HIT", tpIndex: Number(tp2[1]), raw: text });
    out.push({ type: "TP_HIT", tpIndex: Number(tp2[2]), raw: text });
  } else {
    const tp1 = /TP\s*(\d)\s*HIT|\bAT\s+TP\s*(\d)\b/.exec(t);
    if (tp1) {
      out.push({ type: "TP_HIT", tpIndex: Number(tp1[1] ?? tp1[2]), raw: text });
    } else if (/\+\d+\s*PIPS?\s*HIT/.test(t)) {
      out.push({ type: "TP_HIT", raw: text });
    }
  }

  // --- RISK FREE (poziom bywa podany z „AT" albo bez: „RISK FREE 4000") ---
  const rf = /RISK\s*FREE\s*(?:AT\s*)?(\d{1,6}(?:[.,]\d{1,5})?)?/.exec(t);
  if (rf) out.push({ type: "RISK_FREE", level: toNum(rf[1]) ?? undefined, raw: text });

  // --- SECURING PARTIAL PROFITS ---
  if (/SECURING\s+PARTIAL/.test(t) || (/SL\s+IS\s+SET\s+TO\s+BE/.test(t) && /TARGET/.test(t))) {
    out.push({ type: "PARTIAL", tps: extractTps(text), sl: extractSl(text), raw: text });
  }

  // --- korekta TP ---
  const corr =
    /USE\s+(\d{1,6}(?:[.,]\d{1,5})?)\s+AS\s+TP\s*(\d)|TP\s*(\d)\s*(?:IS\s+)?(?:ADJUSTED|MOVED|CHANGED|SET)\s+TO\s+(\d{1,6}(?:[.,]\d{1,5})?)/.exec(
      t,
    );
  if (corr) {
    const idx = Number(corr[2] ?? corr[3]);
    const val = toNum(corr[1] ?? corr[4]);
    out.push({ type: "TP_CORRECTION", tpIndex: idx, tps: val !== null ? [val] : [], raw: text });
  }

  // --- SET SL ---
  const setSl = /(?:MOVE|SET)\s+SL\s+(?:TO\s+)?(\d{1,6}(?:[.,]\d{1,5})?)/.exec(t);
  if (setSl) out.push({ type: "SET_SL", sl: toNum(setSl[1]), raw: text });

  // --- OUT AT ENTRY ---
  if (/OUT\s+AT\s+ENTRY/.test(t)) out.push({ type: "OUT_AT_ENTRY", raw: text });

  // --- CLOSE ALL ---
  if (/CLOSE\s+(?:ALL|EVERYTHING)|CLOSE\s+ALL\s+POSITIONS/.test(t)) {
    out.push({ type: "CLOSE_ALL", raw: text });
  }

  // --- CANCEL ---
  if (/\bCANCEL\b|\bDELETE\b.*\bLIMIT/.test(t)) out.push({ type: "CANCEL", raw: text });

  // --- WEJSCIE (tylko gdy nie ma juz komunikatu zarzadzajacego "HIT") ---
  const hasHit = out.some((s) => s.type === "TP_HIT" || s.type === "SL_HIT");
  if (!hasHit) {
    const entry = parseEntry(text);
    if (entry) out.push(entry);
  }

  if (out.length === 0) out.push({ type: "INFO", raw: text });
  return out;
}

/* KLUCZE SŁOWNIKA, nie gotowe napisy. Etykietę daje `t(SIGNAL_LABEL[typ])`
   w komponencie (`useT()` gwarantuje przerysowanie po zmianie języka).
   Kolejność wpisów = kolejność plakietek w widoku Sygnałów. */
export const SIGNAL_LABEL: Record<SignalType, string> = {
  ENTRY: "signal.entry",
  TP_HIT: "signal.tpHit",
  SL_HIT: "signal.slHit",
  RISK_FREE: "signal.riskFree",
  PARTIAL: "signal.partial",
  CANCEL: "signal.cancel",
  OUT_AT_ENTRY: "signal.outAtEntry",
  CLOSE_ALL: "signal.closeAll",
  TP_CORRECTION: "signal.tpCorrection",
  SET_SL: "signal.setSl",
  INFO: "signal.info",
  UNKNOWN: "signal.unknown",
};

export const SIGNAL_TONE: Record<SignalType, "long" | "short" | "accent" | "warn" | "info" | "muted"> = {
  ENTRY: "accent",
  TP_HIT: "long",
  SL_HIT: "short",
  RISK_FREE: "info",
  PARTIAL: "info",
  CANCEL: "warn",
  OUT_AT_ENTRY: "warn",
  CLOSE_ALL: "short",
  TP_CORRECTION: "warn",
  SET_SL: "warn",
  INFO: "muted",
  UNKNOWN: "warn",
};
