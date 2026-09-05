/* ============================================================
   MODEL AI — format pliku, przebieg w przód, dekodowanie akcji.

   Wierne odwzorowanie `rust/crates/ai/src/policy.rs` i `obs.rs`:
   ten sam układ wag (`w[l][o * in + i]`), ta sama aktywacja (`tanh`
   w warstwach ukrytych, liniowa na wyjściu), te same progi dekodowania.
   Dzięki temu symulator w przeglądarce liczy DOKŁADNIE to, co policzyłby
   silnik — a nie „coś podobnego".

   Plik nie zna Reacta: to czysta matematyka i metadane.
   ============================================================ */

/* `t` z modułu (nie hak) — model demonstracyjny składa się TU, poza
   drzewem komponentów, a jego opis widać w widoku Modeli AI. */
import { t } from "@/i18n";

/* ------------------------------------------------------------
   FORMAT PLIKU (serde z `policy.rs::Model`)
   ------------------------------------------------------------ */

/** Perceptron wielowarstwowy. Wagi wierszami: `w[l][o * in + i]`. */
export interface AiNet {
  dims: number[];
  w: number[][];
  b: number[][];
}

/** Sieć bez wag — tyle wysyła `/api/models` w liście. */
export interface AiNetShape {
  dims: number[];
  params: number;
}

export interface AiSafety {
  equity_floor_pct: number;
  max_margin_util_pct: number;
  floor_headroom: number;
  max_positions: number;
  max_pendings: number;
  max_total_lots: number;
  max_order_lots: number;
  ratchet_sl_only: boolean;
}

export interface AiRewardWeights {
  k_dd: number;
  k_risk: number;
  no_sl_atr: number;
  k_hold: number;
  k_blow: number;
  w_mean: number;
  w_min: number;
}

export interface AiTrainMeta {
  algo: string;
  generations: number;
  pop: number;
  seed: number;
  sigma: number;
  lr: number;
  windows: string[];
  start_balance: number;
  decision_interval_s: number;
  engine_tp_mode: string;
  split: string;
  reward: AiRewardWeights;
}

export interface AiTrainScore {
  fitness: number;
  pnl: number;
  max_dd: number;
  profit_factor: number;
  trades: number;
  win_rate: number;
  note: string;
}

interface AiModelBase {
  /** nazwa pliku bez rozszerzenia — dokłada ją serwer */
  id: string;
  format: number;
  name: string;
  created: string;
  n_global: number;
  n_basket: number;
  n_position: number;
  feature_names: string[];
  safety: AiSafety;
  train: AiTrainMeta;
  score: AiTrainScore;
  /** rozmiar pliku na dysku (tylko z backendu) */
  bytes?: number;
  /** suma parametrów obu sieci (liczy serwer) */
  params?: number;
  /** model wbudowany w aplikację, nie wczytany z dysku */
  demo?: boolean;
}

/** Pozycja listy `/api/models` — kształty warstw zamiast wag. */
export interface AiModelSummary extends AiModelBase {
  policy: { pos: AiNetShape; bsk: AiNetShape };
}

/** Pełny dokument `/api/models/{id}` — z wagami. */
export interface AiModelFile extends AiModelBase {
  policy: { pos: AiNet; bsk: AiNet };
}

export type NetId = "pos" | "bsk";

/* ------------------------------------------------------------
   WYJŚCIA SIECI (stałe `O_*` i `B_*` z policy.rs)
   ------------------------------------------------------------ */

export const O_HOLD = 0;
export const O_CLOSE = 1;
export const O_PARTIAL = 2;
export const O_PART_FRAC = 3;
export const O_SL_KEEP = 4;
export const O_SL_SET = 5;
export const O_SL_GAP = 6;
export const O_TP_KEEP = 7;
export const O_TP_SET = 8;
export const O_TP_MULT = 9;
export const O_TP_DROP = 10;

export const B_HOLD = 0;
export const B_CANCEL = 1;
export const B_ADD = 2;
export const B_CLOSE = 3;
export const B_ADD_DIST = 4;
export const B_ADD_LOT = 5;
export const B_ADD_TP = 6;

export interface OutputInfo {
  /** KLUCZ SŁOWNIKA krótkiej etykiety przy neuronie wyjściowym */
  short: string;
  /** KLUCZ SŁOWNIKA pełnego opisu — wołać przez `t(o.label)` */
  label: string;
  /** `akcja` rozstrzyga się przez argmax w grupie, `parametr` przez sigmoidę */
  kind: "akcja" | "parametr";
}

export const POS_OUTPUTS: OutputInfo[] = [
  { short: "aim.out.pos.0.short", label: "aim.out.pos.0.label", kind: "akcja" },
  { short: "aim.out.pos.1.short", label: "aim.out.pos.1.label", kind: "akcja" },
  { short: "aim.out.pos.2.short", label: "aim.out.pos.2.label", kind: "akcja" },
  { short: "aim.out.pos.3.short", label: "aim.out.pos.3.label", kind: "parametr" },
  { short: "aim.out.pos.4.short", label: "aim.out.pos.4.label", kind: "akcja" },
  { short: "aim.out.pos.5.short", label: "aim.out.pos.5.label", kind: "akcja" },
  { short: "aim.out.pos.6.short", label: "aim.out.pos.6.label", kind: "parametr" },
  { short: "aim.out.pos.7.short", label: "aim.out.pos.7.label", kind: "akcja" },
  { short: "aim.out.pos.8.short", label: "aim.out.pos.8.label", kind: "akcja" },
  { short: "aim.out.pos.9.short", label: "aim.out.pos.9.label", kind: "parametr" },
  { short: "aim.out.pos.10.short", label: "aim.out.pos.10.label", kind: "akcja" },
];

export const BSK_OUTPUTS: OutputInfo[] = [
  { short: "aim.out.bsk.0.short", label: "aim.out.bsk.0.label", kind: "akcja" },
  { short: "aim.out.bsk.1.short", label: "aim.out.bsk.1.label", kind: "akcja" },
  { short: "aim.out.bsk.2.short", label: "aim.out.bsk.2.label", kind: "akcja" },
  { short: "aim.out.bsk.3.short", label: "aim.out.bsk.3.label", kind: "akcja" },
  { short: "aim.out.bsk.4.short", label: "aim.out.bsk.4.label", kind: "parametr" },
  { short: "aim.out.bsk.5.short", label: "aim.out.bsk.5.label", kind: "parametr" },
  { short: "aim.out.bsk.6.short", label: "aim.out.bsk.6.label", kind: "parametr" },
];

export function outputsOf(net: NetId): OutputInfo[] {
  return net === "pos" ? POS_OUTPUTS : BSK_OUTPUTS;
}

/* ------------------------------------------------------------
   OPISY CECH WEJŚCIOWYCH (obs.rs)
   ------------------------------------------------------------ */

/**
 * Nazwy w pliku modelu są techniczne (`p_give_atr`). To jest ich tłumaczenie
 * na język, w którym trader myśli — bez niego panel ważności cech odpowiada
 * na pytanie „na co model patrzy" ciągiem skrótów.
 */
export const FEATURE_DESC: Record<string, string> = {
  /* --- rynek i konto --- */
  spread_atr: "aim.feat.spread_atr",
  atr_rel: "aim.feat.atr_rel",
  ret_1m: "aim.feat.ret_1m",
  ret_5m: "aim.feat.ret_5m",
  ret_15m: "aim.feat.ret_15m",
  ret_60m: "aim.feat.ret_60m",
  vol_5m: "aim.feat.vol_5m",
  vol_60m: "aim.feat.vol_60m",
  equity_ret: "aim.feat.equity_ret",
  balance_ret: "aim.feat.balance_ret",
  float_pnl: "aim.feat.float_pnl",
  dd_now: "aim.feat.dd_now",
  dd_max: "aim.feat.dd_max",
  margin_util: "aim.feat.margin_util",
  n_pos: "aim.feat.n_pos",
  n_pend: "aim.feat.n_pend",
  net_lots: "aim.feat.net_lots",
  gross_lots: "aim.feat.gross_lots",
  hour_sin: "aim.feat.hour_sin",
  hour_cos: "aim.feat.hour_cos",
  day_pnl: "aim.feat.day_pnl",
  room_to_floor: "aim.feat.room_to_floor",
  open_risk: "aim.feat.open_risk",

  /* --- koszyk --- */
  bk_side: "aim.feat.bk_side",
  bk_adv_zone: "aim.feat.bk_adv_zone",
  bk_adv_atr: "aim.feat.bk_adv_atr",
  bk_sl_dist_atr: "aim.feat.bk_sl_dist_atr",
  bk_has_sl: "aim.feat.bk_has_sl",
  bk_tp_dist_atr: "aim.feat.bk_tp_dist_atr",
  bk_has_tp: "aim.feat.bk_has_tp",
  bk_tp_span_atr: "aim.feat.bk_tp_span_atr",
  bk_stage_frac: "aim.feat.bk_stage_frac",
  bk_n_tps: "aim.feat.bk_n_tps",
  bk_age: "aim.feat.bk_age",
  bk_n_open: "aim.feat.bk_n_open",
  bk_n_pend: "aim.feat.bk_n_pend",
  bk_realized: "aim.feat.bk_realized",
  bk_floating: "aim.feat.bk_floating",
  bk_is_limit: "aim.feat.bk_is_limit",
  bk_armed: "aim.feat.bk_armed",
  bk_riskfree: "aim.feat.bk_riskfree",
  bk_sl_width_atr: "aim.feat.bk_sl_width_atr",

  /* --- pozycja --- */
  p_pnl_atr: "aim.feat.p_pnl_atr",
  p_pnl_rel: "aim.feat.p_pnl_rel",
  p_peak_atr: "aim.feat.p_peak_atr",
  p_give_atr: "aim.feat.p_give_atr",
  p_retrace: "aim.feat.p_retrace",
  p_age: "aim.feat.p_age",
  p_since_peak: "aim.feat.p_since_peak",
  p_sl_dist_atr: "aim.feat.p_sl_dist_atr",
  p_has_sl: "aim.feat.p_has_sl",
  p_locked_atr: "aim.feat.p_locked_atr",
  p_tp_dist_atr: "aim.feat.p_tp_dist_atr",
  p_has_tp: "aim.feat.p_has_tp",
  p_vol_rel: "aim.feat.p_vol_rel",
  p_level: "aim.feat.p_level",
  p_is_toucher: "aim.feat.p_is_toucher",
  p_is_runner: "aim.feat.p_is_runner",
  p_entry_depth: "aim.feat.p_entry_depth",
  p_risk_rel: "aim.feat.p_risk_rel",
};

export type FeatureGroup = "rynek" | "koszyk" | "pozycja";

export const GROUP_LABEL: Record<FeatureGroup, string> = {
  rynek: "aim.group.rynek",
  koszyk: "aim.group.koszyk",
  pozycja: "aim.group.pozycja",
};

/** Do którego bloku obserwacji należy cecha o danym indeksie. */
export function featureGroup(i: number, m: { n_global: number; n_basket: number }): FeatureGroup {
  if (i < m.n_global) return "rynek";
  if (i < m.n_global + m.n_basket) return "koszyk";
  return "pozycja";
}

/* ------------------------------------------------------------
   MATEMATYKA
   ------------------------------------------------------------ */

/** Przebieg w przód. `tanh` w warstwach ukrytych, wyjście liniowe. */
export function forward(net: AiNet, x: number[]): number[] {
  const n = net.w.length;
  let a = x.slice(0, net.dims[0]);
  for (let l = 0; l < n; l++) {
    const di = net.dims[l];
    const dof = net.dims[l + 1];
    const w = net.w[l];
    const bias = net.b[l];
    const out = new Array<number>(dof);
    const ostatnia = l === n - 1;
    for (let o = 0; o < dof; o++) {
      let acc = bias[o];
      const base = o * di;
      for (let i = 0; i < di; i++) acc += w[base + i] * a[i];
      out[o] = ostatnia ? acc : Math.tanh(acc);
    }
    a = out;
  }
  return a;
}

export function sigmoid(x: number): number {
  return 1 / (1 + Math.exp(-x));
}

/** Softmax po WYBRANYCH indeksach — tak jak `argmax3` w policy.rs, tyle że
 *  z zachowaniem proporcji, żeby dało się narysować rozkład. */
export function softmaxOf(o: number[], idx: number[]): number[] {
  const max = Math.max(...idx.map((i) => o[i]));
  const e = idx.map((i) => Math.exp(o[i] - max));
  const s = e.reduce((a, b) => a + b, 0) || 1;
  return e.map((v) => v / s);
}

export function argmaxOf(o: number[], idx: number[]): number {
  let best = idx[0];
  for (const i of idx) if (o[i] > o[best]) best = i;
  return best;
}

export interface PosDecision {
  exit: "Hold" | "CloseAll" | "ClosePartial";
  partial: number;
  sl: { kind: "Keep" } | { kind: "Gap"; atr: number };
  tp: { kind: "Keep" } | { kind: "SetAtr"; atr: number } | { kind: "Drop" };
}

/** 1:1 z `decode_pos` — te same stałe skalujące. */
export function decodePos(o: number[]): PosDecision {
  const wyjscie = argmaxOf(o, [O_HOLD, O_CLOSE, O_PARTIAL]);
  const exit = wyjscie === O_CLOSE ? "CloseAll" : wyjscie === O_PARTIAL ? "ClosePartial" : "Hold";
  const partial = 0.1 + 0.8 * sigmoid(o[O_PART_FRAC]);
  const sl: PosDecision["sl"] =
    o[O_SL_SET] > o[O_SL_KEEP] ? { kind: "Gap", atr: 0.25 + 7.75 * sigmoid(o[O_SL_GAP]) } : { kind: "Keep" };
  const cel = argmaxOf(o, [O_TP_KEEP, O_TP_SET, O_TP_DROP]);
  const tp: PosDecision["tp"] =
    cel === O_TP_SET ? { kind: "SetAtr", atr: 0.5 + 7.5 * sigmoid(o[O_TP_MULT]) } : cel === O_TP_DROP ? { kind: "Drop" } : { kind: "Keep" };
  return { exit, partial, sl, tp };
}

/**
 * Ważność wejść: suma modułów wag PIERWSZEJ warstwy dla danego wejścia.
 *
 * To najprostsza uczciwa miara „na co model patrzy". Nie mówi, w którą
 * stronę cecha popycha decyzję (do tego trzeba by przejść całą sieć),
 * ale mówi, ile pojemności model przeznaczył na jej czytanie — cecha
 * z zerowymi wagami nie ma jak wpłynąć na cokolwiek.
 */
export function inputImportance(net: AiNet): number[] {
  const di = net.dims[0];
  const dof = net.dims[1];
  const w = net.w[0];
  const out = new Array<number>(di).fill(0);
  for (let o = 0; o < dof; o++) {
    const base = o * di;
    for (let i = 0; i < di; i++) out[i] += Math.abs(w[base + i]);
  }
  return out;
}

export function countParams(net: { dims: number[]; w?: number[][]; b?: number[][]; params?: number }): number {
  if (typeof net.params === "number") return net.params;
  const suma = (t?: number[][]) => (t ? t.reduce((a, r) => a + r.length, 0) : 0);
  return suma(net.w) + suma(net.b);
}

/* ------------------------------------------------------------
   MODEL DEMONSTRACYJNY
   ------------------------------------------------------------ */

/** PRNG mulberry32 — deterministyczny, więc demo wygląda tak samo po odświeżeniu. */
function prng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Bias warstwy wyjściowej: „trzymaj, nie ruszaj SL, nie ruszaj TP" (POS_BIAS). */
const POS_BIAS = [1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0];
const BSK_BIAS = [1, 0, 0, 0, 0, 0, 0];

/**
 * Buduje sieć o zadanych wymiarach. Warstwy ukryte losowe (Xavier), warstwa
 * wyjściowa mała plus bias — tak wygląda model PO treningu ewolucyjnym, który
 * startuje z wyzerowanego wyjścia i uczy się od niego odchylać.
 *
 * `boost` podbija wagi wybranych wejść, żeby model demonstracyjny miał
 * czytelną hierarchię ważności zamiast płaskiego szumu — inaczej panel cech
 * pokazywałby 60 identycznych słupków i niczego by nie tłumaczył.
 */
function demoNet(dims: number[], outBias: number[], rnd: () => number, boost: Record<number, number> = {}): AiNet {
  const w: number[][] = [];
  const b: number[][] = [];
  for (let l = 0; l < dims.length - 1; l++) {
    const di = dims[l];
    const dof = dims[l + 1];
    const ostatnia = l === dims.length - 2;
    const lim = Math.sqrt(6 / (di + dof));
    const wl = new Array<number>(di * dof);
    for (let o = 0; o < dof; o++) {
      for (let i = 0; i < di; i++) {
        // Warstwa wyjściowa dostaje wagi na tyle duże, żeby POKONAĆ bias
        // „nic nie rób" w skrajnych sytuacjach — inaczej model demonstracyjny
        // odpowiadałby „TRZYMAJ" na każde ustawienie suwaków i symulator nie
        // pokazywałby niczego poza drganiem słupków.
        const skala = ostatnia ? 1.25 : 1;
        const wzmocnienie = l === 0 ? (boost[i] ?? 0.35) : 1;
        wl[o * di + i] = (rnd() * 2 - 1) * lim * skala * wzmocnienie;
      }
    }
    const bl = new Array<number>(dof).fill(0);
    if (ostatnia) for (let k = 0; k < dof; k++) bl[k] = outBias[k] ?? 0;
    else for (let k = 0; k < dof; k++) bl[k] = (rnd() * 2 - 1) * 0.08;
    w.push(wl);
    b.push(bl);
  }
  return { dims, w, b };
}

const GLOBAL_NAMES = [
  "spread_atr", "atr_rel", "ret_1m", "ret_5m", "ret_15m", "ret_60m", "vol_5m", "vol_60m",
  "equity_ret", "balance_ret", "float_pnl", "dd_now", "dd_max", "margin_util", "n_pos", "n_pend",
  "net_lots", "gross_lots", "hour_sin", "hour_cos", "day_pnl", "room_to_floor", "open_risk",
];
const BASKET_NAMES = [
  "bk_side", "bk_adv_zone", "bk_adv_atr", "bk_sl_dist_atr", "bk_has_sl", "bk_tp_dist_atr",
  "bk_has_tp", "bk_tp_span_atr", "bk_stage_frac", "bk_n_tps", "bk_age", "bk_n_open", "bk_n_pend",
  "bk_realized", "bk_floating", "bk_is_limit", "bk_armed", "bk_riskfree", "bk_sl_width_atr",
];
const POSITION_NAMES = [
  "p_pnl_atr", "p_pnl_rel", "p_peak_atr", "p_give_atr", "p_retrace", "p_age", "p_since_peak",
  "p_sl_dist_atr", "p_has_sl", "p_locked_atr", "p_tp_dist_atr", "p_has_tp", "p_vol_rel",
  "p_level", "p_is_toucher", "p_is_runner", "p_entry_depth", "p_risk_rel",
];

export const FEATURE_NAMES = [...GLOBAL_NAMES, ...BASKET_NAMES, ...POSITION_NAMES];

/** Cechy, na które model demonstracyjny „patrzy" najmocniej — po nazwie, nie po indeksie. */
const DEMO_BOOST: Record<string, number> = {
  p_pnl_atr: 1.0,
  p_give_atr: 0.92,
  p_retrace: 0.86,
  p_sl_dist_atr: 0.8,
  p_age: 0.75,
  p_tp_dist_atr: 0.7,
  p_risk_rel: 0.68,
  p_peak_atr: 0.64,
  open_risk: 0.6,
  dd_now: 0.58,
  p_since_peak: 0.52,
  bk_adv_atr: 0.5,
  atr_rel: 0.46,
  bk_stage_frac: 0.42,
  spread_atr: 0.4,
};

function boostMap(names: string[], mnoznik: number): Record<number, number> {
  const out: Record<number, number> = {};
  names.forEach((n, i) => {
    const v = DEMO_BOOST[n];
    if (v !== undefined) out[i] = v * mnoznik;
  });
  return out;
}

function demoModel(opts: {
  id: string;
  name: string;
  seed: number;
  hiddenPos: number[];
  hiddenBsk: number[];
  cadence: number;
  boost: number;
  score: Partial<AiTrainScore>;
  tpMode: string;
}): AiModelFile {
  const rnd = prng(opts.seed);
  const nG = GLOBAL_NAMES.length;
  const nB = BASKET_NAMES.length;
  const nP = POSITION_NAMES.length;
  const posDims = [nG + nB + nP, ...opts.hiddenPos, POS_OUTPUTS.length];
  const bskDims = [nG + nB, ...opts.hiddenBsk, BSK_OUTPUTS.length];
  const pos = demoNet(posDims, POS_BIAS, rnd, boostMap(FEATURE_NAMES, opts.boost));
  const bsk = demoNet(bskDims, BSK_BIAS, rnd, boostMap(FEATURE_NAMES.slice(0, nG + nB), opts.boost));

  return {
    id: opts.id,
    demo: true,
    format: 1,
    name: opts.name,
    created: "",
    n_global: nG,
    n_basket: nB,
    n_position: nP,
    feature_names: FEATURE_NAMES,
    policy: { pos, bsk },
    params: countParams(pos) + countParams(bsk),
    safety: {
      equity_floor_pct: 60,
      max_margin_util_pct: 30,
      floor_headroom: 1.1,
      max_positions: 40,
      max_pendings: 60,
      max_total_lots: 1,
      max_order_lots: 0.1,
      ratchet_sl_only: true,
    },
    train: {
      algo: "es",
      generations: 0,
      pop: 40,
      seed: opts.seed,
      sigma: 0.08,
      lr: 0.05,
      windows: [],
      start_balance: 1000,
      decision_interval_s: opts.cadence,
      engine_tp_mode: opts.tpMode,
      split: t("aim.demo.split"),
      reward: { k_dd: 0.3, k_risk: 0.15, no_sl_atr: 40, k_hold: 0.3, k_blow: 10, w_mean: 0.6, w_min: 0.4 },
    },
    score: {
      fitness: 0,
      pnl: 0,
      max_dd: 0,
      profit_factor: 0,
      trades: 0,
      win_rate: 0,
      note: t("aim.demo.note"),
      ...opts.score,
    },
  };
}

/**
 * Dwa modele wbudowane — widok musi działać także bez `conduit.exe`.
 * Różnią się architekturą i kadencją, więc porównanie dwóch modeli ma co
 * pokazywać w trybie demo.
 */
export const DEMO_MODELS: AiModelFile[] = [
  demoModel({
    id: "demo_manager_60x48x32",
    name: "DEMO · manager 60→48→32→11",
    seed: 20260727,
    hiddenPos: [48, 32],
    hiddenBsk: [32, 24],
    cadence: 2,
    boost: 1,
    tpMode: "tp1",
    score: {},
  }),
  demoModel({
    id: "demo_manager_lean",
    name: "DEMO · manager lean 60→32→11",
    seed: 8675309,
    hiddenPos: [32],
    hiddenBsk: [24],
    cadence: 5,
    boost: 0.72,
    tpMode: "last",
    score: {},
  }),
];

/** Lista (bez wag) zbudowana z modeli wbudowanych — ten sam kształt co z REST-a. */
export function demoSummaries(): AiModelSummary[] {
  return DEMO_MODELS.map((m) => ({
    ...m,
    policy: {
      pos: { dims: m.policy.pos.dims, params: countParams(m.policy.pos) },
      bsk: { dims: m.policy.bsk.dims, params: countParams(m.policy.bsk) },
    },
  }));
}
