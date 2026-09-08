import type { Stats } from "@/types";

export type RealDrawdownDay = NonNullable<Stats["realDrawdownDay"]>;

/** Read only qualified observations; never substitute current equity or a peak. */
export function realDrawdown(day: Stats["realDrawdownDay"]): { amount: number; percent: number | null } | null {
  if (!day || typeof day.startEquity !== "number" || !Number.isFinite(day.startEquity)
    || typeof day.minEquity !== "number" || !Number.isFinite(day.minEquity)
    || typeof day.day !== "number" || !Number.isFinite(day.day)) return null;
  const amount = Math.max(0, day.startEquity - day.minEquity);
  const percent = day.startEquity > 0 ? amount / day.startEquity * 100 : NaN;
  return Number.isFinite(amount) ? { amount, percent: Number.isFinite(percent) ? percent : null } : null;
}

/** Browser-only simulation uses its UTC clock, never the workstation's timezone. */
export function observeDemoEquity(previous: Stats["realDrawdownDay"], equity: number, utc: number): RealDrawdownDay {
  const day = Math.floor(utc / 86_400_000);
  if (!Number.isFinite(equity) || !Number.isFinite(day)) return previous ?? { day: null, startEquity: null, minEquity: null };
  if (previous?.day !== day) return { day, startEquity: equity, minEquity: equity };
  return { ...previous, minEquity: previous.minEquity === null ? null : Math.min(previous.minEquity, equity) };
}
