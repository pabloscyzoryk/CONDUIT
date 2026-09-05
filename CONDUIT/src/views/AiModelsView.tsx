import { tSilnik } from "@/i18n/silnik";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Card, Empty, Field, Icon, Segmented, Select, Switch } from "@/components/ui";
import { api } from "@/store/transport";
import {
  B_ADD,
  B_ADD_DIST,
  B_ADD_LOT,
  B_ADD_TP,
  B_CANCEL,
  B_CLOSE,
  B_HOLD,
  DEMO_MODELS,
  FEATURE_DESC,
  GROUP_LABEL,
  O_CLOSE,
  O_HOLD,
  O_PARTIAL,
  O_SL_GAP,
  O_SL_KEEP,
  O_SL_SET,
  O_TP_DROP,
  O_TP_KEEP,
  O_TP_MULT,
  O_TP_SET,
  argmaxOf,
  countParams,
  decodePos,
  demoSummaries,
  featureGroup,
  forward,
  inputImportance,
  outputsOf,
  sigmoid,
  softmaxOf,
  type AiModelFile,
  type AiModelSummary,
  type AiNet,
  type FeatureGroup,
  type NetId,
} from "@/lib/aiModel";
import { num, locale } from "@/lib/format";
import { useT, useLanguage, RichT } from "@/i18n";
import "./views.css";
import "./aimodels.css";

/* ============================================================
   MODELE AI — podgląd wytrenowanej sieci

   Widok odpowiada na trzy pytania, na które metryka „fitness" nie
   odpowiada: JAK model jest zbudowany, NA CO patrzy i CO ZROBI
   w konkretnej sytuacji. Wszystko liczone lokalnie z wag z pliku —
   przebieg w przód sieci 60→48→32→11 to ułamek milisekundy, więc
   suwaki reagują natychmiast.
   ============================================================ */

/* ------------------------------------------------------------
   1. STAN SYMULOWANEJ SYTUACJI
   ------------------------------------------------------------ */

interface SimState {
  pnlAtr: number;
  ageMin: number;
  giveAtr: number;
  slAtr: number;
  tpAtr: number;
  ddNow: number;
  hasSl: boolean;
  hasTp: boolean;
  side: "BUY" | "SELL";
}

const SIM_DEFAULT: SimState = {
  pnlAtr: 1.6,
  ageMin: 45,
  giveAtr: 0.7,
  slAtr: 3,
  tpAtr: 4,
  ddNow: 0.05,
  hasSl: true,
  hasTp: true,
  side: "BUY",
};

/** `ln_age` z obs.rs: 0 min → 0, doba → 1. */
function lnAge(minutes: number): number {
  return Math.log(1 + Math.max(0, minutes)) / Math.log(1441);
}

/**
 * Wartości cech dla zadanej sytuacji — po NAZWACH, nie po indeksach.
 *
 * Cechy zależne (szczyt zysku, oddany procent, ryzyko do SL) liczone są
 * tak samo jak w `position_features`, żeby suwaki nie produkowały stanu,
 * który w silniku nie mógłby wystąpić: nie da się oddać od szczytu więcej,
 * niż się zarobiło, a pozycja bez SL ma ryzyko maksymalne, nie zerowe.
 *
 * Cechy pominięte zostają na 0 — po normalizacji z `obs.rs` zero znaczy
 * „neutralnie", więc to jest uczciwa wartość domyślna, nie brak danych.
 */
function simFeatures(s: SimState): Record<string, number> {
  const peak = Math.max(s.pnlAtr, 0) + s.giveAtr;
  const retrace = peak > 1e-9 ? Math.min(2, s.giveAtr / peak) : 0;
  // 1 ATR ≈ 0,4 % kapitału — jedyne założenie potrzebne, żeby przełożyć
  // odległości cenowe na cechy pieniężne
  const NA_PROCENT = 0.4;
  const v: Record<string, number> = {
    p_pnl_atr: s.pnlAtr,
    p_pnl_rel: s.pnlAtr * NA_PROCENT,
    p_peak_atr: peak,
    p_give_atr: s.giveAtr,
    p_retrace: retrace,
    p_age: lnAge(s.ageMin),
    p_since_peak: lnAge(s.ageMin * 0.35),
    p_has_sl: s.hasSl ? 1 : 0,
    p_has_tp: s.hasTp ? 1 : 0,
    p_vol_rel: 0.2,
    dd_now: s.ddNow,
    dd_max: Math.max(s.ddNow, 0.08),
    bk_side: s.side === "BUY" ? 1 : -1,
    bk_adv_atr: s.pnlAtr * 0.8,
    bk_has_sl: s.hasSl ? 1 : 0,
    bk_has_tp: s.hasTp ? 1 : 0,
    bk_age: lnAge(s.ageMin * 1.4),
    bk_n_open: 0.2,
    n_pos: 0.1,
    atr_rel: 0.35,
    spread_atr: 0.4,
  };
  if (s.hasSl) {
    v.p_sl_dist_atr = s.slAtr;
    // (SL − wejście) w ATR = zysk − odległość ceny do SL
    v.p_locked_atr = s.pnlAtr - s.slAtr;
    v.p_risk_rel = Math.max(0, s.slAtr - s.pnlAtr) * NA_PROCENT;
    v.bk_sl_dist_atr = s.slAtr;
    v.open_risk = Math.max(0, s.slAtr - s.pnlAtr) * NA_PROCENT * 0.5;
  } else {
    // obs.rs: brak SL = ryzyko nieograniczone, wskazywane maksymalną wartością
    v.p_risk_rel = 8;
    v.open_risk = 2;
  }
  if (s.hasTp) {
    v.p_tp_dist_atr = s.tpAtr;
    v.bk_tp_dist_atr = s.tpAtr;
  }
  return v;
}

/** Wektor wejściowy w kolejności z pliku modelu. */
function observation(m: AiModelFile, vals: Record<string, number>, len: number): number[] {
  const out = new Array<number>(len).fill(0);
  for (let i = 0; i < len; i++) {
    const nazwa = m.feature_names[i];
    if (nazwa && vals[nazwa] !== undefined) out[i] = vals[nazwa];
  }
  return out;
}

/* ------------------------------------------------------------
   2. DIAGRAM SIECI
   ------------------------------------------------------------ */

/** Ile neuronów pokazać w jednej kolumnie, zanim zaczniemy wybierać najsilniejsze. */
const MAX_NODES = 72;

interface Wezel {
  i: number;
  y: number;
}
interface Kolumna {
  x: number;
  r: number;
  nodes: Wezel[];
  total: number;
  label: string;
  pos: Map<number, number>;
}

interface Krawedz {
  l: number;
  from: number;
  to: number;
  w: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** Σ|w| wchodzących / wychodzących dla każdego neuronu warstwy. */
function sily(net: AiNet, layer: number): { we: number[]; wy: number[] } {
  const n = net.dims[layer];
  const we = new Array<number>(n).fill(0);
  const wy = new Array<number>(n).fill(0);
  if (layer > 0) {
    const di = net.dims[layer - 1];
    const w = net.w[layer - 1];
    for (let o = 0; o < n; o++) {
      let s = 0;
      for (let i = 0; i < di; i++) s += Math.abs(w[o * di + i]);
      we[o] = s;
    }
  }
  if (layer < net.w.length) {
    const di = net.dims[layer];
    const dof = net.dims[layer + 1];
    const w = net.w[layer];
    for (let i = 0; i < n; i++) {
      let s = 0;
      for (let o = 0; o < dof; o++) s += Math.abs(w[o * di + i]);
      wy[i] = s;
    }
  }
  return { we, wy };
}

function NetworkDiagram({
  net,
  netId,
  model,
  topEdges,
  hover,
  onHover,
}: {
  net: AiNet;
  netId: NetId;
  model: AiModelFile;
  topEdges: number;
  hover: { layer: number; i: number } | null;
  onHover: (h: { layer: number; i: number } | null) => void;
}) {
  const t = useT();
  /* Etykiety kolumn budują się w `useMemo`, więc SAMA zmiana języka musi
     unieważnić ten cache — stąd `lang` w zależnościach. Bez tego diagram
     zostawałby przy poprzednim języku do następnej zmiany rozmiaru okna. */
  const { lang } = useLanguage();
  const outputs = outputsOf(netId);

  /* Diagram rysuje się w skali 1:1 — szerokość `viewBox` jest równa realnej
     szerokości kontenera, więc etykiety mają zawsze 10 px, niezależnie od
     okna. Poniżej `MIN_W` viewBox przestaje się kurczyć i to kontener
     zaczyna się przewijać (ta sama zasada co w tabelach). */
  const MIN_W = 760;
  const wrapRef = useRef<HTMLDivElement>(null);
  const [szerokosc, setSzerokosc] = useState(940);
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const zmierz = () => setSzerokosc(Math.max(MIN_W, Math.round(el.clientWidth)));
    zmierz();
    const ro = new ResizeObserver(zmierz);
    ro.observe(el);
    // Zapasowe nasłuchiwanie okna: `ResizeObserver` dostarcza powiadomienia
    // w rytmie klatek, więc w karcie w tle potrafi milczeć aż do powrotu.
    window.addEventListener("resize", zmierz);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", zmierz);
    };
  }, []);

  const uklad = useMemo(() => {
    const nL = net.dims.length;
    const maks = Math.max(...net.dims);
    const H = Math.max(340, Math.min(620, maks * 8.6 + 74));
    const W = szerokosc;
    const x0 = 92;
    const x1 = W - 168;
    const gora = 52;
    const dol = H - 18;

    const kolumny: Kolumna[] = net.dims.map((n, l) => {
      const moce = sily(net, l);
      let widoczne = Array.from({ length: n }, (_, i) => i);
      if (n > MAX_NODES) {
        // przy bardzo szerokiej warstwie pokazujemy najsilniejsze neurony,
        // a nie pierwsze z brzegu — inaczej diagram kłamie o tym, co ważne
        widoczne = widoczne
          .sort((a, b) => moce.we[b] + moce.wy[b] - (moce.we[a] + moce.wy[a]))
          .slice(0, MAX_NODES)
          .sort((a, b) => a - b);
      }
      const k = widoczne.length;
      const rozstaw = k > 1 ? Math.min(13, (dol - gora) / (k - 1)) : 0;
      const srodek = (gora + dol) / 2;
      const y0 = srodek - ((k - 1) * rozstaw) / 2;
      const x = nL > 1 ? x0 + ((x1 - x0) * l) / (nL - 1) : (x0 + x1) / 2;
      const r = Math.max(2.4, Math.min(5.5, rozstaw * 0.34 || 5.5));
      const nodes = widoczne.map((i, k2) => ({ i, y: y0 + k2 * rozstaw }));
      const pos = new Map<number, number>();
      for (const w of nodes) pos.set(w.i, w.y);
      const label =
        l === 0
          ? t("aim.col.input", { n })
          : l === nL - 1
            ? t("aim.col.output", { n })
            : t("aim.col.hidden", { l, n });
      return { x, r, nodes, total: n, label, pos };
    });

    return { W, H, kolumny };
  }, [net, szerokosc, lang]);

  /* --- krawędzie tła: N najsilniejszych w każdej warstwie --- */
  const { edges, maxAbs } = useMemo(() => {
    const out: Krawedz[] = [];
    let maxAbs = 1e-9;
    for (let l = 0; l < net.w.length; l++) {
      const di = net.dims[l];
      const dof = net.dims[l + 1];
      const w = net.w[l];
      const a = uklad.kolumny[l];
      const b = uklad.kolumny[l + 1];
      const kand: Krawedz[] = [];
      for (let o = 0; o < dof; o++) {
        const y2 = b.pos.get(o);
        if (y2 === undefined) continue;
        for (let i = 0; i < di; i++) {
          const y1 = a.pos.get(i);
          if (y1 === undefined) continue;
          const v = w[o * di + i];
          const m = Math.abs(v);
          if (m > maxAbs) maxAbs = m;
          kand.push({ l, from: i, to: o, w: v, x1: a.x, y1, x2: b.x, y2 });
        }
      }
      kand.sort((p, q) => Math.abs(q.w) - Math.abs(p.w));
      out.push(...kand.slice(0, topEdges));
    }
    return { edges: out, maxAbs };
  }, [net, uklad, topEdges]);

  /* --- krawędzie podświetlone: wszystkie wchodzące i wychodzące z neuronu --- */
  const podswietlone = useMemo(() => {
    if (!hover) return [];
    const out: Krawedz[] = [];
    const { layer, i: idx } = hover;
    if (layer > 0) {
      const l = layer - 1;
      const di = net.dims[l];
      const w = net.w[l];
      const a = uklad.kolumny[l];
      const b = uklad.kolumny[layer];
      const y2 = b.pos.get(idx);
      if (y2 !== undefined) {
        for (let i = 0; i < di; i++) {
          const y1 = a.pos.get(i);
          if (y1 === undefined) continue;
          out.push({ l, from: i, to: idx, w: w[idx * di + i], x1: a.x, y1, x2: b.x, y2 });
        }
      }
    }
    if (layer < net.w.length) {
      const l = layer;
      const di = net.dims[l];
      const dof = net.dims[l + 1];
      const w = net.w[l];
      const a = uklad.kolumny[l];
      const b = uklad.kolumny[l + 1];
      const y1 = a.pos.get(idx);
      if (y1 !== undefined) {
        for (let o = 0; o < dof; o++) {
          const y2 = b.pos.get(o);
          if (y2 === undefined) continue;
          out.push({ l, from: idx, to: o, w: w[o * di + idx], x1: a.x, y1, x2: b.x, y2 });
        }
      }
    }
    return out;
  }, [hover, net, uklad]);

  /* --- ważność wejść steruje jasnością kropek pierwszej kolumny --- */
  const wagaWejsc = useMemo(() => {
    const imp = inputImportance(net);
    const max = Math.max(...imp, 1e-9);
    return imp.map((v) => v / max);
  }, [net]);

  const kolor = (w: number) => (w >= 0 ? "var(--long)" : "var(--short)");
  const grubosc = (w: number) => 0.35 + 2.1 * Math.min(1, Math.abs(w) / maxAbs);
  const przezr = (w: number) => 0.16 + 0.52 * Math.min(1, Math.abs(w) / maxAbs);

  const opisWezla = (layer: number, i: number) => {
    if (layer === 0) {
      const nazwa = model.feature_names[i] ?? t("aim.node.input", { i });
      const k = FEATURE_DESC[nazwa];
      return { tytul: nazwa, opis: k ? t(k) : t("aim.feat.fallback") };
    }
    if (layer === net.dims.length - 1) {
      const o = outputs[i];
      return {
        tytul: o ? t(o.label) : t("aim.node.output", { i }),
        opis: o?.kind === "parametr" ? t("aim.node.param") : t("aim.node.action"),
      };
    }
    return {
      tytul: t("aim.node.hidden", { l: layer, i }),
      opis: t("aim.node.bias", { v: num(net.b[layer - 1][i], 3) }),
    };
  };

  const info = hover ? opisWezla(hover.layer, hover.i) : null;

  return (
    <div className="aim__net">
      <div className="aim__svgwrap" ref={wrapRef}>
        <svg
          viewBox={`0 0 ${uklad.W} ${uklad.H}`}
          className="aim__svg"
          style={{ minWidth: MIN_W, height: uklad.H }}
          role="img"
          aria-label={t("aim.diagram.aria", {
            net: netId === "pos" ? t("aim.net.pos") : t("aim.net.bsk"),
            dims: net.dims.join(" → "),
          })}
          onMouseLeave={() => onHover(null)}
        >
          {/* nagłówki kolumn */}
          {uklad.kolumny.map((k, l) => (
            <text key={`h${l}`} x={k.x} y={26} textAnchor="middle" className="aim__collabel">
              {k.label}
            </text>
          ))}

          {/* krawędzie tła */}
          <g opacity={hover ? 0.16 : 1}>
            {edges.map((e, n) => (
              <line
                key={`e${n}`}
                x1={e.x1}
                y1={e.y1}
                x2={e.x2}
                y2={e.y2}
                stroke={kolor(e.w)}
                strokeWidth={grubosc(e.w)}
                opacity={przezr(e.w)}
              />
            ))}
          </g>

          {/* krawędzie neuronu pod kursorem */}
          {podswietlone.map((e, n) => (
            <line
              key={`p${n}`}
              x1={e.x1}
              y1={e.y1}
              x2={e.x2}
              y2={e.y2}
              stroke={kolor(e.w)}
              strokeWidth={Math.max(0.6, grubosc(e.w))}
              opacity={0.3 + 0.65 * Math.min(1, Math.abs(e.w) / maxAbs)}
            />
          ))}

          {/* neurony */}
          {uklad.kolumny.map((k, l) => {
            const wejscie = l === 0;
            const wyjscie = l === uklad.kolumny.length - 1;
            return (
              <g key={`c${l}`}>
                {k.nodes.map((w) => {
                  const akt = hover?.layer === l && hover.i === w.i;
                  const rel = wejscie ? (wagaWejsc[w.i] ?? 0) : 0;
                  return (
                    <g key={w.i}>
                      <circle
                        cx={k.x}
                        cy={w.y}
                        r={k.r}
                        fill={wejscie ? "var(--accent)" : wyjscie ? "var(--ai)" : "var(--bg-surface-3)"}
                        fillOpacity={wejscie ? 0.18 + 0.8 * rel : wyjscie ? 0.85 : 1}
                        stroke={akt ? "var(--text)" : wyjscie ? "var(--ai)" : "var(--border-strong)"}
                        strokeWidth={akt ? 1.8 : 0.8}
                      />
                      <circle
                        cx={k.x}
                        cy={w.y}
                        r={Math.max(6, k.r + 3)}
                        fill="transparent"
                        style={{ cursor: "pointer" }}
                        onMouseEnter={() => onHover({ layer: l, i: w.i })}
                      />
                    </g>
                  );
                })}
                {k.total > k.nodes.length && (
                  <text x={k.x} y={uklad.H - 4} textAnchor="middle" className="aim__collabel">
                    {t("aim.shownOf", { n: k.nodes.length, all: k.total })}
                  </text>
                )}
              </g>
            );
          })}

          {/* etykiety wyjść */}
          {uklad.kolumny[uklad.kolumny.length - 1].nodes.map((w) => {
            const o = outputs[w.i];
            const k = uklad.kolumny[uklad.kolumny.length - 1];
            return (
              <text
                key={`o${w.i}`}
                x={k.x + 12}
                y={w.y + 3.4}
                className={`aim__outlabel ${o?.kind === "parametr" ? "aim__outlabel--param" : ""}`}
              >
                {o ? t(o.short) : `#${w.i}`}
              </text>
            );
          })}
        </svg>
      </div>

      <div className="aim__info">
        {info ? (
          <>
            <b className="mono">{info.tytul}</b>
            <span className="hint">{info.opis}</span>
          </>
        ) : (
          <span className="hint">
            <RichT
              k="aim.diagram.hint"
              vars={{ n: topEdges, all: net.w.reduce((a, r) => a + r.length, 0).toLocaleString(locale()) }}
            />
          </span>
        )}
      </div>

      <div className="aim__legend">
        <span className="eyebrow">{t("aim.scale")}</span>
        <span className="aim__scale">
          <i style={{ background: "var(--short)", height: 5, width: 26 }} />
          <i style={{ background: "var(--short)", height: 3, width: 18, opacity: 0.6 }} />
          <i style={{ background: "var(--border-strong)", height: 1, width: 12 }} />
          <i style={{ background: "var(--long)", height: 3, width: 18, opacity: 0.6 }} />
          <i style={{ background: "var(--long)", height: 5, width: 26 }} />
        </span>
        <span className="hint">
          −{num(maxAbs, 2)} … +{num(maxAbs, 2)} · {t("aim.scale.desc")} (
          <b style={{ color: "var(--long-text)" }}>{t("aim.scale.up")}</b> /{" "}
          <b style={{ color: "var(--short-text)" }}>{t("aim.scale.down")}</b>)
        </span>
        <span className="spacer" />
        <span className="hint">{t("aim.scale.dot")}</span>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------
   3. PANEL CECH WEJŚCIOWYCH
   ------------------------------------------------------------ */

const GROUP_TONE: Record<FeatureGroup, "accent" | "warn" | "ai"> = {
  rynek: "accent",
  koszyk: "warn",
  pozycja: "ai",
};

function FeaturePanel({
  model,
  net,
  netId,
  onHover,
  hoverIndex,
}: {
  model: AiModelFile;
  net: AiNet;
  netId: NetId;
  onHover: (i: number | null) => void;
  hoverIndex: number | null;
}) {
  const t = useT();
  const [filtr, setFiltr] = useState<"all" | FeatureGroup>("all");
  const [wszystkie, setWszystkie] = useState(false);

  const cechy = useMemo(() => {
    const imp = inputImportance(net);
    const max = Math.max(...imp, 1e-9);
    return imp
      .map((v, i) => ({
        i,
        nazwa: model.feature_names[i] ?? `#${i}`,
        waga: v,
        rel: v / max,
        grupa: featureGroup(i, model),
      }))
      .sort((a, b) => b.waga - a.waga);
  }, [net, model]);

  const widoczne = cechy.filter((c) => filtr === "all" || c.grupa === filtr);
  const lista = wszystkie ? widoczne : widoczne.slice(0, 14);

  return (
    <Card
      title={t("aim.feats.title")}
      icon="sliders"
      accent="var(--accent)"
      subtitle={t("aim.feats.subtitle", {
        net: netId === "pos" ? t("aim.net.posFull") : t("aim.net.bskFull"),
        n: net.dims[0],
      })}
      actions={
        <Segmented<"all" | FeatureGroup>
          size="sm"
          value={filtr}
          onChange={setFiltr}
          options={[
            { value: "all", label: t("aim.feats.all") },
            { value: "rynek", label: t(GROUP_LABEL.rynek) },
            { value: "koszyk", label: t(GROUP_LABEL.koszyk) },
            { value: "pozycja", label: t(GROUP_LABEL.pozycja) },
          ]}
        />
      }
    >
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        {t("aim.feats.note")}
      </p>

      <div className="aim__feats">
        {lista.map((c, k) => (
          <div
            key={c.i}
            className="aimfeat"
            data-hot={hoverIndex === c.i}
            onMouseEnter={() => onHover(c.i)}
            onMouseLeave={() => onHover(null)}
          >
            <span className="aimfeat__rank num">{k + 1}</span>
            <span className="aimfeat__name">
              <b className="mono truncate">{c.nazwa}</b>
              <span className="hint truncate">{FEATURE_DESC[c.nazwa] ? t(FEATURE_DESC[c.nazwa]) : "—"}</span>
            </span>
            <span className="aimfeat__bar">
              <i style={{ width: `${Math.max(2, c.rel * 100)}%`, background: `var(--${GROUP_TONE[c.grupa]})` }} />
            </span>
            <span className="aimfeat__val num">{num(c.waga, 2)}</span>
          </div>
        ))}
      </div>

      {widoczne.length > 14 && (
        <Button
          variant="ghost"
          size="sm"
          icon={wszystkie ? "chevron-left" : "chevron-down"}
          onClick={() => setWszystkie((v) => !v)}
          style={{ marginTop: "var(--sp-3)" }}
        >
          {wszystkie ? t("aim.feats.showTop") : t("aim.feats.showAll", { n: widoczne.length })}
        </Button>
      )}
    </Card>
  );
}

/* ------------------------------------------------------------
   4. SYMULATOR DECYZJI
   ------------------------------------------------------------ */

function Bar({ v, tone, label, right }: { v: number; tone: string; label: string; right: string }) {
  return (
    <div className="aimout">
      <span className="aimout__k truncate">{label}</span>
      <span className="aimout__bar">
        <i style={{ width: `${Math.max(1.5, v * 100)}%`, background: tone }} />
      </span>
      <span className="aimout__v num">{right}</span>
    </div>
  );
}

function Slider({
  label,
  hint,
  value,
  min,
  max,
  step,
  unit,
  onChange,
  disabled,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <div className={`aimslider ${disabled ? "aimslider--off" : ""}`}>
      <div className="aimslider__head">
        <span className="aimslider__label">{label}</span>
        <b className="num">
          {num(value, step < 1 ? (step < 0.1 ? 2 : 1) : 0)} {unit}
        </b>
      </div>
      <input
        className="aim-range"
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      {hint && <span className="hint">{hint}</span>}
    </div>
  );
}

/** Rozkłady i decyzja sieci pozycji dla podanego wejścia. */
function DecisionBars({ out }: { out: number[] }) {
  const t = useT();
  const wyjscie = softmaxOf(out, [O_HOLD, O_CLOSE, O_PARTIAL]);
  const stop = softmaxOf(out, [O_SL_KEEP, O_SL_SET]);
  const cel = softmaxOf(out, [O_TP_KEEP, O_TP_SET, O_TP_DROP]);
  const d = decodePos(out);

  const pr = (v: number) => `${num(v * 100, 1)} %`;

  return (
    <div className="aim__outs">
      <div className="aim__outgrp">
        <span className="eyebrow">{t("aim.dec.exit")}</span>
        <Bar v={wyjscie[0]} tone="var(--accent)" label={t("aim.dec.hold")} right={pr(wyjscie[0])} />
        <Bar v={wyjscie[1]} tone="var(--short)" label={t("aim.dec.closeAll")} right={pr(wyjscie[1])} />
        <Bar v={wyjscie[2]} tone="var(--warn)" label={t("aim.dec.closePart")} right={pr(wyjscie[2])} />
        <span className="hint">
          <RichT k="aim.dec.partialNote" vars={{ v: (d.partial * 100).toFixed(0) }} />
        </span>
      </div>

      <div className="aim__outgrp">
        <span className="eyebrow">Stop loss</span>
        <Bar v={stop[0]} tone="var(--accent)" label={t("aim.dec.keep")} right={pr(stop[0])} />
        <Bar v={stop[1]} tone="var(--long)" label={t("aim.dec.moveSl")} right={pr(stop[1])} />
        <span className="hint">
          <RichT k="aim.dec.newSl" vars={{ v: num(0.25 + 7.75 * sigmoid(out[O_SL_GAP]), 2) }} />
        </span>
      </div>

      <div className="aim__outgrp">
        <span className="eyebrow">Take profit</span>
        <Bar v={cel[0]} tone="var(--accent)" label={t("aim.dec.keep")} right={pr(cel[0])} />
        <Bar v={cel[1]} tone="var(--long)" label={t("aim.dec.setTp")} right={pr(cel[1])} />
        <Bar v={cel[2]} tone="var(--ai)" label={t("aim.dec.dropTp")} right={pr(cel[2])} />
        <span className="hint">
          <RichT k="aim.dec.newTp" vars={{ v: num(0.5 + 7.5 * sigmoid(out[O_TP_MULT]), 2) }} />
        </span>
      </div>
    </div>
  );
}

/** Jednowierszowe podsumowanie decyzji — to, co silnik faktycznie wykona. */
function decisionText(out: number[], t: ReturnType<typeof useT>): string {
  const d = decodePos(out);
  const wyj =
    d.exit === "CloseAll"
      ? t("aim.verdict.closeAll")
      : d.exit === "ClosePartial"
        ? t("aim.verdict.closePart", { v: (d.partial * 100).toFixed(0) })
        : t("aim.verdict.hold");
  const sl = d.sl.kind === "Keep" ? t("aim.verdict.slKeep") : t("aim.verdict.slSet", { v: num(d.sl.atr, 2) });
  const tp =
    d.tp.kind === "Keep"
      ? t("aim.verdict.tpKeep")
      : d.tp.kind === "Drop"
        ? t("aim.verdict.tpDrop")
        : t("aim.verdict.tpSet", { v: num(d.tp.atr, 2) });
  return `${wyj} · ${sl} · ${tp}`;
}

function basketText(out: number[], t: ReturnType<typeof useT>): string {
  const best = argmaxOf(out, [B_HOLD, B_CANCEL, B_ADD, B_CLOSE]);
  if (best === B_CANCEL) return t("aim.verdict.bskCancel");
  if (best === B_CLOSE) return t("aim.verdict.bskClose");
  if (best === B_ADD) {
    return t("aim.verdict.bskAdd", {
      dist: num(0.3 + 4 * sigmoid(out[B_ADD_DIST]), 2),
      lot: num(0.5 + 2.5 * sigmoid(out[B_ADD_LOT]), 2),
      tp: num(0.5 + 5 * sigmoid(out[B_ADD_TP]), 2),
    });
  }
  return t("aim.verdict.bskHold");
}

/* ------------------------------------------------------------
   5. METRYKI
   ------------------------------------------------------------ */

function Metric({ k, v, tone, title }: { k: string; v: string; tone?: string; title?: string }) {
  return (
    <div className="aimmetric" title={title}>
      <span>{k}</span>
      <b className={`num ${tone ?? ""}`}>{v}</b>
    </div>
  );
}

function MetricsCard({ m }: { m: AiModelFile }) {
  const t = useT();
  const params = m.params ?? countParams(m.policy.pos) + countParams(m.policy.bsk);
  const pf = m.score.profit_factor;
  return (
    <Card
      title={t("aim.metrics.title")}
      icon="microscope"
      accent="var(--ai)"
      subtitle={m.demo ? t("aim.metrics.demo") : t("aim.metrics.fromFile")}
    >
      <div className="aim__metrics">
        <Metric k={t("aim.m.fitness")} v={num(m.score.fitness, 4)} tone={m.score.fitness >= 0 ? "up" : "down"} />
        <Metric k={t("aim.m.valPnl")} v={`${m.score.pnl >= 0 ? "+" : "−"}${num(Math.abs(m.score.pnl), 2)} $`} tone={m.score.pnl >= 0 ? "up" : "down"} />
        <Metric k={t("aim.m.valDd")} v={`${num(m.score.max_dd, 2)} $`} />
        <Metric
          k={t("aim.m.pf")}
          v={pf < 0 ? t("aim.m.pf.noLoss") : num(pf, 2)}
          title={pf < 0 ? t("aim.m.pf.title") : undefined}
        />
        <Metric k={t("aim.m.trades")} v={String(m.score.trades)} />
        <Metric k={t("aim.m.winRate")} v={`${num(m.score.win_rate * 100, 1)} %`} />
        <Metric k={t("aim.m.params")} v={params.toLocaleString(locale())} />
        <Metric k={t("aim.m.cadence")} v={`${num(m.train.decision_interval_s, 1)} s`} />
        <Metric k={t("aim.m.algo")} v={m.train.algo.toUpperCase()} />
        <Metric k={t("aim.m.genPop")} v={`${m.train.generations} × ${m.train.pop}`} />
        <Metric k={t("aim.m.capital")} v={`${num(m.train.start_balance, 0)} $`} />
        <Metric k={t("aim.m.engineTp")} v={m.train.engine_tp_mode || "—"} title={t("aim.m.engineTp.title")} />
        <Metric k={t("aim.m.windows")} v={String(m.train.windows.length)} title={m.train.windows.join("\n")} />
        <Metric k={t("aim.m.sigmaLr")} v={`${num(m.train.sigma, 3)} / ${num(m.train.lr, 3)}`} />
      </div>

      <div className="divider" />

      <span className="eyebrow">{t("aim.m.split")}</span>
      <p className="hint" style={{ marginTop: 4 }}>{m.train.split || "—"}</p>

      {m.score.note && (
        <>
          <div className="divider" />
          <span className="eyebrow">{t("aim.m.note")}</span>
          <p className="hint" style={{ marginTop: 4 }}>{m.score.note}</p>
        </>
      )}

      <div className="divider" />

      <span className="eyebrow">{t("aim.safety")}</span>
      <div className="aim__metrics" style={{ marginTop: 6 }}>
        <Metric k={t("aim.s.floor")} v={`${num(m.safety.equity_floor_pct, 0)} %`} />
        <Metric k={t("aim.s.margin")} v={`${num(m.safety.max_margin_util_pct, 0)} %`} />
        <Metric k={t("aim.s.maxPos")} v={String(m.safety.max_positions)} />
        <Metric k={t("aim.s.maxPend")} v={String(m.safety.max_pendings)} />
        <Metric k={t("aim.s.maxLots")} v={num(m.safety.max_total_lots, 2)} />
        <Metric k={t("aim.s.maxOrderLot")} v={num(m.safety.max_order_lots, 2)} />
        <Metric k={t("aim.s.ratchet")} v={m.safety.ratchet_sl_only ? t("aim.s.yes") : t("aim.s.no")} title={t("aim.s.ratchet.title")} />
        <Metric k={t("aim.s.headroom")} v={num(m.safety.floor_headroom, 2)} />
      </div>
    </Card>
  );
}

/* ------------------------------------------------------------
   6. WIDOK
   ------------------------------------------------------------ */

export function AiModelsView() {
  const t = useT();
  const [lista, setLista] = useState<AiModelSummary[]>(() => demoSummaries());
  const [zrodlo, setZrodlo] = useState<"demo" | "backend">("demo");
  const [selId, setSelId] = useState(() => demoSummaries()[0].id);
  const [cmpId, setCmpId] = useState("");
  const [model, setModel] = useState<AiModelFile | null>(DEMO_MODELS[0]);
  const [cmp, setCmp] = useState<AiModelFile | null>(null);
  const [blad, setBlad] = useState<string | null>(null);
  const [ladowanie, setLadowanie] = useState(false);

  const [netId, setNetId] = useState<NetId>("pos");
  const [topEdges, setTopEdges] = useState(120);
  const [hover, setHover] = useState<{ layer: number; i: number } | null>(null);
  const [sim, setSim] = useState<SimState>(SIM_DEFAULT);

  const cache = useRef(new Map<string, AiModelFile>());

  /* --- lista modeli: backend, a gdy go nie ma — modele wbudowane --- */
  const odswiez = useCallback(async () => {
    setLadowanie(true);
    try {
      const m = await api.models();
      if (m.length > 0) {
        setLista(m);
        setZrodlo("backend");
        setBlad(null);
        setSelId((cur) => (m.some((x) => x.id === cur) ? cur : m[0].id));
        return;
      }
      setLista(demoSummaries());
      setZrodlo("demo");
      setBlad(t("aim.err.emptyDir"));
      setSelId(demoSummaries()[0].id);
    } catch {
      setLista(demoSummaries());
      setZrodlo("demo");
      setBlad(null);
      setSelId((cur) => (DEMO_MODELS.some((x) => x.id === cur) ? cur : DEMO_MODELS[0].id));
    } finally {
      setLadowanie(false);
    }
    // `t` jest stałą referencją modułu — nie ma potrzeby wpisywać go w zależności
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    void odswiez();
  }, [odswiez]);

  /* --- pełny dokument modelu (z wagami) --- */
  const pobierz = useCallback(
    async (id: string): Promise<AiModelFile | null> => {
      if (!id) return null;
      const demo = DEMO_MODELS.find((x) => x.id === id);
      if (demo) return demo;
      const c = cache.current.get(id);
      if (c) return c;
      const m = await api.model(id);
      cache.current.set(id, m);
      return m;
    },
    [],
  );

  useEffect(() => {
    let anulowane = false;
    setHover(null);
    void (async () => {
      try {
        const m = await pobierz(selId);
        if (!anulowane) {
          setModel(m);
          setBlad(null);
        }
      } catch (e) {
        if (!anulowane) {
          setModel(null);
          setBlad(e instanceof Error ? e.message : t("aim.err.load"));
        }
      }
    })();
    return () => {
      anulowane = true;
    };
  }, [selId, pobierz]);

  useEffect(() => {
    let anulowane = false;
    void (async () => {
      if (!cmpId) {
        setCmp(null);
        return;
      }
      try {
        const m = await pobierz(cmpId);
        if (!anulowane) setCmp(m);
      } catch {
        if (!anulowane) setCmp(null);
      }
    })();
    return () => {
      anulowane = true;
    };
  }, [cmpId, pobierz]);

  /* --- sytuacja → wektor wejściowy → wyjście sieci --- */
  const cechy = useMemo(() => simFeatures(sim), [sim]);

  const wynik = useMemo(() => {
    if (!model) return null;
    const pos = model.policy.pos;
    const bsk = model.policy.bsk;
    const x = observation(model, cechy, pos.dims[0]);
    return { pos: forward(pos, x), bsk: forward(bsk, x.slice(0, bsk.dims[0])) };
  }, [model, cechy]);

  const wynikCmp = useMemo(() => {
    if (!cmp) return null;
    const x = observation(cmp, cechy, cmp.policy.pos.dims[0]);
    return forward(cmp.policy.pos, x);
  }, [cmp, cechy]);

  const opcje = lista.map((m) => ({
    value: m.id,
    label: `${m.demo ? "◇ " : ""}${m.name || m.id}${m.policy?.pos?.dims ? ` — ${m.policy.pos.dims.join("→")}` : ""}`,
  }));

  const siec = model ? (netId === "pos" ? model.policy.pos : model.policy.bsk) : null;
  const wybrany = lista.find((m) => m.id === selId);

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("aim.title")}</h1>
          <p>
            <RichT k="aim.intro" />
          </p>
        </div>
        <Badge tone={zrodlo === "backend" ? "long" : "warn"} dot>
          {zrodlo === "backend" ? t("aim.src.backend", { n: lista.length }) : t("aim.src.demo")}
        </Badge>
      </div>

      <Card
        title={t("aim.pick.title")}
        icon="brain"
        accent="var(--ai)"
        actions={
          <Button variant="ghost" size="sm" icon="refresh" onClick={() => void odswiez()} disabled={ladowanie}>
            {t("chan.refresh")}
          </Button>
        }
      >
        <div className="formgrid">
          <Field label={t("aim.f.model")} hint={zrodlo === "backend" ? t("aim.f.model.api") : t("aim.f.model.local")}>
            <Select value={selId} onChange={setSelId} options={opcje} />
          </Field>
          <Field label={t("aim.f.compare")} hint={t("aim.f.compare.hint")}>
            <Select
              value={cmpId}
              onChange={setCmpId}
              options={[{ value: "", label: t("aim.f.compare.none") }, ...opcje.filter((o) => o.value !== selId)]}
            />
          </Field>
          <Field label={t("aim.f.net")} hint={t("aim.f.net.hint")}>
            <Segmented<NetId>
              value={netId}
              onChange={setNetId}
              options={[
                { value: "pos", label: t("aim.net.posShort") },
                { value: "bsk", label: t("aim.net.bskShort") },
              ]}
            />
          </Field>
          <Field label={t("aim.f.edges")} hint={t("aim.f.edges.hint")}>
            <Select
              value={String(topEdges)}
              onChange={(v) => setTopEdges(Number(v))}
              options={[
                { value: "40", label: t("aim.f.edges.n", { n: 40 }) },
                { value: "120", label: t("aim.f.edges.n", { n: 120 }) },
                { value: "300", label: t("aim.f.edges.n", { n: 300 }) },
                { value: "5000", label: t("aim.f.edges.all") },
              ]}
            />
          </Field>
        </div>

        {wybrany && (
          <>
            <div className="divider" />
            <div className="row">
              <Badge tone="ai">{wybrany.policy?.pos?.dims?.join(" → ") ?? "—"}</Badge>
              <Badge tone="muted">
                {t("aim.net.bskShort")} {wybrany.policy?.bsk?.dims?.join(" → ") ?? "—"}
              </Badge>
              <Badge tone="muted">{t("aim.badge.format", { v: wybrany.format })}</Badge>
              {wybrany.created && <Badge tone="muted">{wybrany.created.slice(0, 19).replace("T", " ")}</Badge>}
              {wybrany.bytes ? <Badge tone="muted">{(wybrany.bytes / 1024).toFixed(0)} kB</Badge> : null}
              {wybrany.demo && (
                <Badge tone="warn" title={t("aim.badge.localW.title")}>
                  {t("aim.badge.localW")}
                </Badge>
              )}
              <span className="spacer" />
              <span className="hint mono truncate">{wybrany.name}</span>
            </div>
          </>
        )}

        {blad && (
          <div className="aim__err">
            <Icon name="alert" size={14} />
            <span>{tSilnik(blad)}</span>
          </div>
        )}
      </Card>

      {!model || !siec ? (
        <Card>
          <Empty icon="brain" title={t("aim.empty")} text={tSilnik(blad) || t("aim.empty.text")} />
        </Card>
      ) : (
        <>
          <div className="aim__grid">
            <Card
              title={t("aim.arch.title")}
              icon="layers"
              accent="var(--accent)"
              subtitle={t("aim.arch.subtitle", {
                dims: siec.dims.join(" → "),
                n: countParams(siec).toLocaleString(locale()),
              })}
            >
              <NetworkDiagram
                net={siec}
                netId={netId}
                model={model}
                topEdges={topEdges}
                hover={hover}
                onHover={setHover}
              />
            </Card>

            <MetricsCard m={model} />
          </div>

          <FeaturePanel
            model={model}
            net={siec}
            netId={netId}
            hoverIndex={hover?.layer === 0 ? hover.i : null}
            onHover={(i) => setHover(i === null ? null : { layer: 0, i })}
          />

          <Card
            title={t("aim.sim.title")}
            icon="target"
            accent="var(--long)"
            subtitle={t("aim.sim.subtitle")}
            actions={
              <Button variant="ghost" size="sm" icon="refresh" onClick={() => setSim(SIM_DEFAULT)}>
                {t("sims.reset")}
              </Button>
            }
          >
            <div className="aim__sim">
              <div className="aim__sliders">
                <Slider
                  label={t("aim.sl.pnl")}
                  value={sim.pnlAtr}
                  min={-6}
                  max={8}
                  step={0.1}
                  unit="× ATR"
                  onChange={(v) => setSim((s) => ({ ...s, pnlAtr: v }))}
                  hint={t("aim.sl.pnl.hint")}
                />
                <Slider
                  label={t("aim.sl.age")}
                  value={sim.ageMin}
                  min={0}
                  max={1440}
                  step={5}
                  unit="min"
                  onChange={(v) => setSim((s) => ({ ...s, ageMin: v }))}
                  hint={t("aim.sl.age.hint", { v: num(lnAge(sim.ageMin), 3) })}
                />
                <Slider
                  label={t("aim.sl.give")}
                  value={sim.giveAtr}
                  min={0}
                  max={6}
                  step={0.1}
                  unit="× ATR"
                  onChange={(v) => setSim((s) => ({ ...s, giveAtr: v }))}
                  hint={t("aim.sl.give.hint", { v: num(Math.max(sim.pnlAtr, 0) + sim.giveAtr, 1) })}
                />
                <Slider
                  label={t("aim.sl.slDist")}
                  value={sim.slAtr}
                  min={0}
                  max={10}
                  step={0.1}
                  unit="× ATR"
                  disabled={!sim.hasSl}
                  onChange={(v) => setSim((s) => ({ ...s, slAtr: v }))}
                  hint={sim.hasSl ? t("aim.sl.slDist.hint", { v: num(sim.pnlAtr - sim.slAtr, 1) }) : t("aim.sl.noSl")}
                />
                <Slider
                  label={t("aim.sl.tpDist")}
                  value={sim.tpAtr}
                  min={0}
                  max={12}
                  step={0.1}
                  unit="× ATR"
                  disabled={!sim.hasTp}
                  onChange={(v) => setSim((s) => ({ ...s, tpAtr: v }))}
                  hint={sim.hasTp ? "p_tp_dist_atr" : t("aim.sl.noTp")}
                />
                <Slider
                  label={t("aim.sl.dd")}
                  value={sim.ddNow * 100}
                  min={0}
                  max={60}
                  step={1}
                  unit="%"
                  onChange={(v) => setSim((s) => ({ ...s, ddNow: v / 100 }))}
                  hint={t("aim.sl.dd.hint")}
                />
                <div className="aim__switches">
                  <Switch checked={sim.hasSl} onChange={(v) => setSim((s) => ({ ...s, hasSl: v }))} label={t("aim.sl.hasSl")} />
                  <Switch checked={sim.hasTp} onChange={(v) => setSim((s) => ({ ...s, hasTp: v }))} label={t("aim.sl.hasTp")} />
                  <Segmented<"BUY" | "SELL">
                    size="sm"
                    value={sim.side}
                    onChange={(v) => setSim((s) => ({ ...s, side: v }))}
                    options={[
                      { value: "BUY", label: "BUY" },
                      { value: "SELL", label: "SELL" },
                    ]}
                  />
                </div>
                <p className="hint">
                  <RichT k="aim.sim.zeroNote" />
                </p>
              </div>

              <div className="aim__decision">
                {wynik && (
                  <>
                    <div className="aim__verdict">
                      <span className="eyebrow">{t("aim.sim.verdict")}</span>
                      <b>{decisionText(wynik.pos, t)}</b>
                      <span className="hint mono">{basketText(wynik.bsk, t)}</span>
                    </div>
                    <DecisionBars out={wynik.pos} />
                  </>
                )}
              </div>
            </div>
          </Card>

          {cmp && wynik && wynikCmp && (
            <Card title={t("aim.cmp.title")} icon="microscope" accent="var(--warn)" subtitle={t("aim.cmp.subtitle")}>
              <div className="aim__cmp">
                {[
                  { m: model, o: wynik.pos, tag: "A" },
                  { m: cmp, o: wynikCmp, tag: "B" },
                ].map(({ m, o, tag }) => {
                  const imp = inputImportance(m.policy.pos);
                  const max = Math.max(...imp, 1e-9);
                  const top = imp
                    .map((v, i) => ({ i, v, n: m.feature_names[i] ?? `#${i}` }))
                    .sort((a, b) => b.v - a.v)
                    .slice(0, 6);
                  return (
                    <div className="aim__cmpcol" key={tag}>
                      <div className="row">
                        <Badge tone={tag === "A" ? "ai" : "warn"}>{tag}</Badge>
                        <b className="truncate">{m.name || m.id}</b>
                      </div>
                      <div className="aim__verdict aim__verdict--sm">
                        <b>{decisionText(o, t)}</b>
                      </div>
                      <div className="aim__metrics">
                        <Metric k={t("aim.m.arch")} v={m.policy.pos.dims.join("→")} />
                        <Metric k={t("aim.m.params")} v={(m.params ?? countParams(m.policy.pos) + countParams(m.policy.bsk)).toLocaleString(locale())} />
                        <Metric k={t("aim.m.cadenceShort")} v={`${num(m.train.decision_interval_s, 1)} s`} />
                        <Metric k={t("aim.m.valPnl")} v={`${m.score.pnl >= 0 ? "+" : "−"}${num(Math.abs(m.score.pnl), 2)} $`} tone={m.score.pnl >= 0 ? "up" : "down"} />
                        <Metric k="Max DD" v={`${num(m.score.max_dd, 2)} $`} />
                        <Metric k={t("aim.m.winRate")} v={`${num(m.score.win_rate * 100, 1)} %`} />
                      </div>
                      <span className="eyebrow">{t("aim.cmp.topSix")}</span>
                      <div className="aim__feats">
                        {top.map((c) => (
                          <div className="aimfeat aimfeat--sm" key={c.i}>
                            <span className="aimfeat__name">
                              <b className="mono truncate">{c.n}</b>
                            </span>
                            <span className="aimfeat__bar">
                              <i style={{ width: `${Math.max(2, (c.v / max) * 100)}%`, background: tag === "A" ? "var(--ai)" : "var(--warn)" }} />
                            </span>
                            <span className="aimfeat__val num">{num(c.v, 2)}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  );
                })}
              </div>
              <p className="hint" style={{ marginTop: "var(--sp-3)" }}>
                {t("aim.cmp.note")}
              </p>
            </Card>
          )}

          <Card title={t("aim.help.title")} icon="info" tight>
            <div className="aim__help">
              <div>
                <b>{t("aim.help.1.t")}</b>
                <span className="hint">
                  <RichT k="aim.help.1.d" />
                </span>
              </div>
              <div>
                <b>{t("aim.help.2.t")}</b>
                <span className="hint">
                  <RichT k="aim.help.2.d" />
                </span>
              </div>
              <div>
                <b>{t("aim.help.3.t")}</b>
                <span className="hint">
                  <RichT k="aim.help.3.d" />
                </span>
              </div>
              <div>
                <b>{t("aim.help.4.t")}</b>
                <span className="hint">
                  <RichT k="aim.help.4.d" />
                </span>
              </div>
            </div>
          </Card>
        </>
      )}
    </div>
  );
}
