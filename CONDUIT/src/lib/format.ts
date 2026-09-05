import { FX_RATES } from "@/data/defaultSettings";
import { getLanguage, t } from "@/i18n";



/** Znacznik BCP-47 dla bieżącego języka panelu. Publiczny, bo kilka
 *  miejsc woła `toLocaleString` bezpośrednio (np. liczby parametrów sieci). */
export function locale(): string {
  return getLanguage() === "pl" ? "pl-PL" : "en-GB";
}

export function money(v: number, currency = "MT5", opts: { sign?: boolean; decimals?: number } = {}): string {
  const fx = FX_RATES[currency] ?? FX_RATES.USD;
  const val = v * fx.rate;
  const d = opts.decimals ?? (Math.abs(val) >= 10000 ? 0 : 2);
  const body = Math.abs(val).toLocaleString(locale(), { minimumFractionDigits: d, maximumFractionDigits: d });
  const sign = val < 0 ? "−" : opts.sign ? "+" : "";
  if (!fx.symbol) return `${sign}${body}`;
  return fx.symbol === "zł" || fx.symbol === "kr" || fx.symbol === "Kč" || fx.symbol === "Fr"
    ? `${sign}${body} ${fx.symbol}`
    : `${sign}${fx.symbol}${body}`;
}

/** Znak BRAKU DANEJ. Świadomie jeden na całą aplikację — patrz `num`. */
export const BRAK = "—";


export function num(v: number, d = 2): string {
  if (!Number.isFinite(v)) return BRAK;
  return v.toLocaleString(locale(), { minimumFractionDigits: d, maximumFractionDigits: d });
}

export function pct(v: number, d = 1): string {
  if (!Number.isFinite(v)) return BRAK;
  return `${v >= 0 ? "" : "−"}${Math.abs(v).toFixed(d)}%`;
}

export function signed(v: number, d = 2): string {
  if (!Number.isFinite(v)) return BRAK;
  return `${v >= 0 ? "+" : "−"}${Math.abs(v).toFixed(d)}`;
}

/* Formatery dat są drogie w budowie, więc trzymamy je w cache po lokalizacji.
   Godzina zostaje 24-godzinna także po angielsku: to terminal handlowy, a nie
   zegarek — „14:32:07" czyta się jednoznacznie i zgadza się z logami silnika. */
const CACHE_DAT = new Map<string, Intl.DateTimeFormat>();

function fmtDaty(id: string, opcje: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const loc = locale();
  const klucz = `${loc}|${id}`;
  let f = CACHE_DAT.get(klucz);
  if (!f) {
    f = new Intl.DateTimeFormat(loc, { hourCycle: "h23", ...opcje });
    CACHE_DAT.set(klucz, f);
  }
  return f;
}

export const time = (t: number) =>
  fmtDaty("time", { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(t);
export const timeShort = (t: number) => fmtDaty("short", { hour: "2-digit", minute: "2-digit" }).format(t);
export const date = (t: number) => fmtDaty("date", { day: "2-digit", month: "2-digit" }).format(t);
export const dateTime = (t: number) =>
  fmtDaty("full", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(t);

/** Broker wall-clock fields: UTC is a rendering convention, not an inferred broker offset. */
export const brokerTime = (value: number) => Number.isFinite(value) && value > 0
  ? fmtDaty("broker-time", { timeZone: "UTC", hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(value) : BRAK;
export const brokerDateTime = (value: number) => Number.isFinite(value) && value > 0
  ? fmtDaty("broker-full", { timeZone: "UTC", day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(value) : BRAK;

export function ago(czas: number): string {
  const s = Math.max(0, (Date.now() - czas) / 1000);
  if (s < 45) return t("fmt.ago.now");
  if (s < 90) return t("fmt.ago.min1");
  if (s < 3600) return t("fmt.ago.min", { n: Math.round(s / 60) });
  if (s < 7200) return t("fmt.ago.hour1");
  if (s < 86400) return t("fmt.ago.hours", { n: Math.round(s / 3600) });
  return t("fmt.ago.days", { n: Math.round(s / 86400) });
}

export function duration(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s} ${t("fmt.unit.s")}`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} ${t("fmt.unit.min")}`;
  const h = Math.floor(m / 60);
  return `${h} ${t("fmt.unit.h")} ${m % 60} ${t("fmt.unit.min")}`;
}

export function compact(v: number): string {
  if (Math.abs(v) >= 1e6) return `${(v / 1e6).toFixed(1)} M`;
  if (Math.abs(v) >= 1e3) return `${(v / 1e3).toFixed(1)} k`;
  return String(Math.round(v));
}

export function toneOf(v: number): "up" | "down" | "flat" {
  if (v > 0.0001) return "up";
  if (v < -0.0001) return "down";
  return "flat";
}
