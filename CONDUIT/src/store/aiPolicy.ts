import type { Settings } from "@/types";
import { AI_MODELS } from "@/data/telegram";

/**
 * TRYB AI — polityka zarzadzania.
 *
 * W bot.py wlaczenie `ai_mode` sprawia, ze wytrenowana siec przejmuje
 * PELNE zarzadzanie pozycjami i NADPISUJE wszystkie inne ustawienia
 * zarzadzania. Tutaj odwzorowujemy to doslownie: dla trybu AI budujemy
 * efektywny zestaw ustawien wyprowadzony z wybranego modelu, a panel
 * ustawien zarzadzania w ogole sie nie pokazuje.
 */
export function aiEffectiveSettings(base: Settings, modelId: string): Settings {
  const m = AI_MODELS.find((x) => x.id === modelId) ?? AI_MODELS[0];
  // Public builds may have no bundled model. Preserve the actual document;
  // inventing a fallback policy would misrepresent a backend-loaded model.
  if (!m) return { ...base };

  // kadencja modelu -> jak czesto AI ocenia pozycje (wirtualny SL)
  const cadenceS = m.cadence.includes("min") ? parseFloat(m.cadence) * 60 : parseFloat(m.cadence) || 2;

  // agresywnosc wyprowadzona z metryk modelu — model o wyzszym PF trzyma
  // ciasniej, model o wyzszym zysku pozwala pozycjom biec dalej
  const tight = m.metrics.profitFactor >= 5;

  return {
    ...base,
    ai_mode: true,
    ai_model: m.id,

    /* AI zarzadza per-pozycja: zadnych recznych harmonogramow */
    official_mode: false,
    official_use_counts: false,
    official_spp: false,
    official_assign_tps: false,
    scale_out: false,
    all_runners: true,
    smart_sl: false,
    breakeven_protection: false,
    trail_after_tp2: false,
    partial_close: false,

    /* polityka wyjscia wyuczona przez siec */
    runner_trail: true,
    runner_trail_start: tight ? 3 : 6,
    trail_mode: "lock_pct",
    trail_lock_pct: tight ? 92 : 78,
    trail_split: true,
    trail_runners_n: tight ? 1 : 2,
    trail_runner_mode: "tiered",
    trail_runner_tiers: "5:1,10:4,20:12,35:26,60:50,100:88",

    be_lock: true,
    be_lock_points: tight ? 1.5 : 2.5,
    be_at_tp1: false,

    harvest: true,
    harvest_start: tight ? 6 : 10,
    harvest_retrace_pct: tight ? 35 : 45,

    /* dwuczlonowa stagnacja — destylat z sieci */
    stale_take_min: 120,
    stale_take_profit: 18,
    stale_take_min2: 60,
    stale_take_profit2: 35,

    /* wirtualny SL wg kadencji modelu */
    virtual_sl: true,
    virtual_sl_only_when_rejected: false,
    vsl_eval_s: Math.min(60, Math.max(2, cadenceS)),
    virtual_sl_all: false,

    oae_timeout_min: 45,
    oae_profit_min: 0.5,
    reenter_after_tp: true,
    reenter_min_tp_stage: 1,
  };
}

/** Ustawienia efektywne w trybie MANUAL — bot nie zarzadza niczym sam. */
export function manualEffectiveSettings(base: Settings): Settings {
  return {
    ...base,
    ai_mode: false,
    all_runners: false,
    scale_out: false,
    official_mode: false,
    smart_sl: false,
    breakeven_protection: false,
    runner_trail: false,
    harvest: false,
    be_lock: false,
    be_at_tp1: false,
    reenter_after_tp: false,
    oae_timeout_min: 0,
    stale_take_min: 0,
    stale_take_min2: 0,
    virtual_sl: false,
    partial_close: false,
    tp_detect_price: false,
    tp_detect_signal: false,
    day_target_close: false,
    eod_flat_hour: 0,
    flat_weekend: false,
  };
}
