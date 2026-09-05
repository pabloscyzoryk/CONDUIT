import type { Quote } from "@/types";

export function quoteUtcTime(q: Quote, backend: boolean): number | null {
  const candidate = q.timeBasis === "utc" ? q.time : q.timeUtc;
  if (Number.isFinite(candidate) && (candidate ?? 0) > 0) return candidate!;
  return !backend && Number.isFinite(q.time) && q.time > 0 ? q.time : null;
}
export type QuoteState = "brak" | "nieznane" | "swieze" | "stare" | "martwe";
export function quoteState(utc: number | null, now: number): QuoteState {
  if (utc === null) return "nieznane";
  if (!utc || !Number.isFinite(utc)) return "brak";
  const age = now - utc;
  if (age < -1000) return "nieznane";
  return age > 90000 ? "martwe" : age > 20000 ? "stare" : "swieze";
}
export function historyFrom(period: string, sessionUtc: number, nowUtc: number, backend: boolean, quoteTime: number): number | null {
  if (period === "all") return 0;
  if (period === "session") return backend ? null : sessionUtc;
  const anchor = backend ? quoteTime : nowUtc;
  const minutes = Number(period);
  return anchor > 0 && Number.isFinite(anchor) && minutes > 0 && Number.isFinite(minutes)
    ? anchor - minutes * 60000 : null;
}
