import { DEFAULT_SETTINGS } from "@/data/defaultSettings";
import { powiazPoleUstawien, type EdycjaUstawienPresetu } from "@/store/warstwaPola";
import type { Settings, SettingKey, T100Config, TradingMode } from "@/types";

export type T100Key = keyof T100Config;
export const T100_KEYS = Object.keys(DEFAULT_SETTINGS.t100) as T100Key[];
const INTEGER_MAX: Partial<Record<T100Key, number>> = {
  experts: 255, max_positions: Number.MAX_SAFE_INTEGER, cooldown_bars: 4294967295,
  max_hold_min: 4294967295, session_start_utc: 255, session_end_utc: 255, friday_flat_utc: 255,
};

/** Defaults fill ABSENT children only. Existing invalid values and unknown keys
 * remain visible and cannot be saved as a silently normalized configuration. */
export function t100Document(value: unknown): T100Config | null {
  if (value === undefined) return { ...DEFAULT_SETTINGS.t100 };
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return { ...DEFAULT_SETTINGS.t100, ...value } as T100Config;
}

/** Mirrors Config::valid plus JSON type/known-field checks used by the mapper. */
export function t100Errors(value: unknown): T100Key[] | ["object"] {
  if (!value || typeof value !== "object" || Array.isArray(value)) return ["object"];
  const c = value as T100Config;
  if (Object.keys(c).some(key => !T100_KEYS.includes(key as T100Key))) return ["object"];
  const errors = new Set<T100Key>();
  for (const key of T100_KEYS) {
    const v = c[key];
    if (key === "enabled" || key === "signal_required") {
      if (typeof v !== "boolean") errors.add(key);
    } else if (typeof v !== "number" || !Number.isFinite(v) || v < 0) errors.add(key);
    const integerMax = INTEGER_MAX[key];
    if (integerMax !== undefined && (typeof v !== "number" || !Number.isInteger(v) || v > integerMax)) errors.add(key);
  }
  const require = (condition: boolean, ...keys: T100Key[]) => { if (!condition) keys.forEach(key => errors.add(key)); };
  require(c.experts > 0 && c.experts <= 15, "experts");
  require(c.signal_weight <= 1, "signal_weight");
  require(c.signal_half_life_min > 0, "signal_half_life_min");
  require(c.risk_pct > 0 && c.risk_pct <= 20, "risk_pct");
  require(c.portfolio_risk_pct <= 30, "portfolio_risk_pct");
  require(c.portfolio_risk_pct >= c.risk_pct, "portfolio_risk_pct", "risk_pct");
  require(c.margin_budget_pct > 0 && c.margin_budget_pct <= 80, "margin_budget_pct");
  require(c.max_positions > 0 && c.max_positions <= 100, "max_positions");
  for (const key of ["stop_atr", "reward_risk", "trail_atr"] as const) require(c[key] >= 0.3, key);
  require(c.daily_loss_pct > 0 && c.daily_loss_pct <= 50, "daily_loss_pct");
  require(c.daily_giveback_pct <= 100, "daily_giveback_pct");
  require(c.adaptation <= 1, "adaptation");
  require(c.session_start_utc < c.session_end_utc, "session_start_utc", "session_end_utc");
  require(c.session_end_utc <= 24, "session_end_utc");
  require(c.friday_flat_utc <= 24, "friday_flat_utc");
  require(c.max_hold_min > 0, "max_hold_min");
  require(c.min_atr > 0, "min_atr");
  return [...errors];
}

/** A blank numeric draft is invalid, not an implicit zero/off switch. */
export function t100Number(raw: string): number {
  return raw.trim() ? Number(raw.replace(",", ".")) : Number.NaN;
}

export function bindT100Settings(
  app: { mode: TradingMode; settings: Settings; setSetting: <K extends SettingKey>(key: K, value: Settings[K]) => void },
  editor: EdycjaUstawienPresetu | null,
  blocked = false,
) {
  const binding = powiazPoleUstawien("t100", app, editor, blocked);
  return {
    value: binding.value, owner: binding.wlasciciel, blocked: binding.zablokowane,
    save: (value: T100Config): boolean => {
      if (binding.zablokowane || t100Errors(value).length > 0) return false;
      if (value.enabled && t100Document(binding.value)?.enabled !== true && app.mode !== "AUTO-EA") return false;
      binding.set({ ...value });
      return true;
    },
  };
}

export const T100_BASIC_KEYS: readonly T100Key[] = ["signal_weight", "signal_required", "score_threshold", "risk_pct", "portfolio_risk_pct", "max_positions", "stop_atr", "reward_risk"];
