import { tSilnik } from "@/i18n/silnik";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  Empty,
  Field,
  Icon,
  NumberInput,
  Segmented,
  Select,
  TextInput,
} from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { formatPresetu } from "@/data/presets";
import { api, type BacktestReq, type LabGen, type LabInfo, type LabJob, type LabRow, type LabSourceCounts, type TrainReq } from "@/store/transport";
import { entrySourceCounts, optionalCount } from "@/lib/labMetrics";
import { duration, num, locale } from "@/lib/format";
import { useT, RichT } from "@/i18n";
import "./views.css";
import "./lab.css";

/* ============================================================
   LABORATORIUM

   Jeden widok na dwa silniki: backtesty (`crates/backtest`) i trening
   modelu AI (`crates/ai`). Formularze wysyłają zlecenie REST-em, a postęp
   przychodzi TĄ SAMĄ drogą co reszta stanu — sekcją `lab` snapshotu przez
   WebSocket. Dzięki temu okno natywne i karta przeglądarki pokazują ten sam
   pasek postępu, a karta otwarta w połowie przemiału od razu widzi, co się
   liczy.

   Widok nie liczy NICZEGO sam. Gdyby liczył, wynik z okna różniłby się od
   wyniku z `bt.exe` — a to jest dokładnie ten rodzaj rozjazdu, którego przy
   ocenie strategii nie da się zauważyć w porę.
   ============================================================ */

type Tab = "backtest" | "train";

/* ============================================================
   OCENA CZTEROTRYBOWA — typy

   Świadomie LOKALNE, a nie w `src/store/transport.ts`: tamten plik należy do
   zespołu PANEL, a laboratorium nie ma prawa go edytować (NAUKOWIEC §6).
   Kształt jest jeden do jednego z `crates/server/src/lab/mod.rs`; gdy PANEL
   zechce, przeniesie je do siebie i te znikną.
   ============================================================ */

type TrybKey = "dpd-staly" | "dpd-comp" | "dlugo-staly" | "dlugo-comp";

interface LabMonth {
  month: string;
  profit: number;
  days: number;
  lossDays: number;
}

interface LabCell extends LabSourceCounts {
  mode: TrybKey;
  profit: number;
  lossDaysPct: number;
  worstDay: number;
  minEquity: number;
  ruins: number;
  blown: boolean;
  units: number;
  maxDd: number;
  maxOpenRiskPct: number;
  endEquity: number;
  tradingDays: number;
  months: LabMonth[];
  partial: boolean;
  cutShort: boolean;
  chart: string;
}

interface LabQuad {
  name: string;
  cells: LabCell[];
  survives: boolean;
  sumProfit: number;
  worstProfit: number;
}

/** `LabJob` z transportu nie zna jeszcze `quads` — dokładamy je tutaj. */
type LabJobQ = LabJob & { quads?: LabQuad[] };
/** To samo po stronie zlecenia. */
type BacktestReqQ = BacktestReq & { fourModes?: boolean };

/** Tabela 2×2 z NAUKOWIEC §4. Kolejność musi zgadzać się z `TRYBY` w Ruście.
 *  `label`/`hint` to KLUCZE SŁOWNIKA — identyfikator `key` jest nietykalny. */
const TRYBY: { key: TrybKey; label: string; hint: string }[] = [
  { key: "dpd-staly", label: "lab.mode.dpdStaly", hint: "lab.mode.dpdStaly.hint" },
  { key: "dpd-comp", label: "lab.mode.dpdComp", hint: "lab.mode.dpdComp.hint" },
  { key: "dlugo-staly", label: "lab.mode.dlugoStaly", hint: "lab.mode.dlugoStaly.hint" },
  { key: "dlugo-comp", label: "lab.mode.dlugoComp", hint: "lab.mode.dlugoComp.hint" },
];

/** Poziom ruiny jako procent kapitału. Musi zgadzać się z `PODLOGA_RUINY_PCT`. */
const PODLOGA_RUINY_PCT = 20;

/** Ile skrótów do wykresów pokazujemy. Przemiał stu presetów daje sto plików —
 *  ściana jednakowych przycisków niczego nie ułatwia, a wykres każdego przebiegu
 *  jest o jedno kliknięcie w wierszu tabeli. */
const CHARTS_SHOWN = 24;

/* ---------------- formatowanie ---------------- */

const czasKrotki = (ms: number) => (ms <= 0 ? "—" : duration(ms));

function fmtPf(v: number | null): string {
  return v === null ? "∞" : v.toFixed(2);
}

function fmtKwota(v: number, d = 0): string {
  return `${v >= 0 ? "+" : "−"}${Math.abs(v).toLocaleString(locale(), {
    minimumFractionDigits: d,
    maximumFractionDigits: d,
  })} $`;
}

/** Fazy zadania — `label` to KLUCZ SŁOWNIKA. */
const FAZA: Record<string, { label: string; tone: "accent" | "long" | "warn" | "short" }> = {
  running: { label: "lab.phase.running", tone: "accent" },
  done: { label: "lab.phase.done", tone: "long" },
  cancelled: { label: "lab.phase.cancelled", tone: "warn" },
  failed: { label: "lab.phase.failed", tone: "short" },
};

/* ============================================================
   WYKRES POSTĘPU FITNESS
   ============================================================ */

function FitnessChart({ gens }: { gens: LabGen[] }) {
  const t = useT();
  if (gens.length < 2) {
    return <div className="lab__chartempty">{t("lab.fit.empty")}</div>;
  }
  const W = 100;
  const H = 40;
  const vals = gens.flatMap((g) => [g.center, g.best, g.median, g.worst]).filter(Number.isFinite);
  const min = Math.min(...vals);
  const max = Math.max(...vals);
  const span = Math.max(1e-9, max - min);
  const x = (i: number) => (i / (gens.length - 1)) * W;
  const y = (v: number) => H - ((v - min) / span) * (H - 2) - 1;
  const linia = (pick: (g: LabGen) => number) => gens.map((g, i) => `${x(i)},${y(pick(g))}`).join(" ");

  return (
    <div className="lab__chart">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={t("lab.fit.aria")}>
        <polyline points={linia((g) => g.worst)} className="lab__line lab__line--worst" vectorEffect="non-scaling-stroke" />
        <polyline points={linia((g) => g.median)} className="lab__line lab__line--median" vectorEffect="non-scaling-stroke" />
        <polyline points={linia((g) => g.best)} className="lab__line lab__line--best" vectorEffect="non-scaling-stroke" />
        <polyline points={linia((g) => g.center)} className="lab__line lab__line--center" vectorEffect="non-scaling-stroke" />
      </svg>
      <div className="lab__legend">
        <span className="lab__key lab__key--center">{t("lab.fit.center")}</span>
        <span className="lab__key lab__key--best">{t("lab.fit.best")}</span>
        <span className="lab__key lab__key--median">{t("lab.fit.median")}</span>
        <span className="lab__key lab__key--worst">{t("lab.fit.worst")}</span>
        <span className="spacer" />
        <span className="hint">
          {min.toFixed(3)} … {max.toFixed(3)}
        </span>
      </div>
    </div>
  );
}

/* ============================================================
   PANEL POSTĘPU
   ============================================================ */

function Postep({ job, onCancel, onOpenDir, busy }: { job: LabJob; onCancel: () => void; onOpenDir: () => void; busy: boolean }) {
  const t = useT();
  const f = FAZA[job.phase] ?? FAZA.running;
  const pct = Math.round(job.progress * 1000) / 10;
  const tr = job.train;

  return (
    <Card
      title={job.kind === "train" ? t("lab.job.train") : t("lab.job.backtest")}
      icon={job.kind === "train" ? "brain" : "microscope"}
      accent={job.phase === "running" ? "var(--accent)" : job.phase === "failed" ? "var(--short)" : "var(--long)"}
      subtitle={job.id}
      actions={
        <div className="row row--tight">
          <Badge tone={f.tone} dot={job.phase === "running"}>
            {t(f.label)}
          </Badge>
          {busy && (
            <Button size="sm" variant="danger" icon="pause" onClick={onCancel}>
              {t("lab.cancelBtn")}
            </Button>
          )}
          <Button size="sm" variant="outline" icon="external" onClick={onOpenDir} title={job.outDir}>
            {t("lab.outDir")}
          </Button>
        </div>
      }
    >
      <div className="lab__title">{tSilnik(job.title)}</div>

      <div className="lab__bar">
        <div className="meter" style={{ height: 8 }}>
          <div
            className="meter__fill"
            style={{
              width: `${Math.max(1, pct)}%`,
              background:
                job.phase === "failed"
                  ? "var(--short)"
                  : job.phase === "cancelled"
                    ? "var(--warn)"
                    : job.phase === "done"
                      ? "var(--long)"
                      : "var(--accent)",
            }}
          />
        </div>
        <b className="num lab__pct">{pct.toFixed(1)}%</b>
      </div>

      {/* CO AKTUALNIE LICZY — to jest najważniejszy napis na tym ekranie */}
      <div className="lab__now">
        {job.phase === "running" && <span className="lab__spin" />}
        <span className="truncate">{tSilnik(job.label)}</span>
      </div>

      <div className="lab__metrics">
        <div className="lab__metric">
          <b className="num">{czasKrotki(job.elapsedMs)}</b>
          <span>{t("lab.m.elapsed")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{job.phase === "running" ? czasKrotki(job.etaMs) : "—"}</b>
          <span>{t("lab.m.eta")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{tSilnik(job.speed) || "—"}</b>
          <span>{t("lab.m.speed")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{tSilnik(job.speed2) || "—"}</b>
          <span>{t("lab.m.rate")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">
            {job.done} / {job.total}
          </b>
          <span>{job.kind === "train" ? t("lab.m.gens") : t("lab.m.runs")}</span>
        </div>
        {tr && (
          <>
            <div className="lab__metric">
              <b className={`num ${tr.bestFitness >= tr.baselineTrain ? "up" : "down"}`}>{tr.bestFitness.toFixed(4)}</b>
              <span>{t("lab.m.bestFitness", { n: tr.bestGen })}</span>
            </div>
            <div className="lab__metric">
              <b className="num">{tr.medianFitness.toFixed(4)}</b>
              <span>{t("lab.m.medianFitness")}</span>
            </div>
            <div className="lab__metric">
              <b className="num down">{tr.bestDd.toFixed(2)} $</b>
              <span>{t("lab.m.bestDd")}</span>
            </div>
            <div className="lab__metric">
              <b className="num">{tr.baselineValid.toFixed(4)}</b>
              <span>{t("lab.m.baseline")}</span>
            </div>
          </>
        )}
      </div>

      {job.error && (
        <div className="lab__err">
          <Icon name="alert" size={14} />
          <span>{tSilnik(job.error)}</span>
        </div>
      )}
      {job.note && !job.error && <div className="lab__note">{tSilnik(job.note)}</div>}
      {job.phase === "cancelled" && (
        <div className="lab__hintbox">
          <Icon name="info" size={13} />
          <span>
            {t("lab.cancelled.note")}
            {job.kind === "train" ? t("lab.cancelled.train") : t("lab.cancelled.bt")}
          </span>
        </div>
      )}
    </Card>
  );
}

/* ============================================================
   OCENA CZTEROTRYBOWA

   Kryterium z NAUKOWIEC §4: preset musi działać we WSZYSTKICH czterech
   trybach naraz. Dlatego widok jest zbudowany tak, żeby nie dało się
   przeczytać jednej dobrej liczby i przeoczyć trzech złych: najpierw
   zestawienie wszystkich czterech obok siebie, a szczegóły dopiero po
   kliknięciu.
   ============================================================ */


function BrakPomiaru() {
  const t = useT();
  return <span className="hint">{t("lab.notMeasured")}</span>;
}

function Werdykt({ q }: { q: LabQuad }) {
  const t = useT();
  if (q.cells.length < TRYBY.length) {
    return (
      <Badge tone="muted" title={t("lab.verdict.partialSet.title")}>
        {t("lab.verdict.partialSet", { n: q.cells.length })}
      </Badge>
    );
  }
  if (q.cells.some((c) => c.partial)) {
    return (
      <Badge tone="warn" title={t("lab.verdict.partial.title")}>
        {t("lab.partial")}
      </Badge>
    );
  }
  const ruiny = q.cells.reduce((s, c) => s + c.ruins, 0);
  if (ruiny > 0 || q.cells.some((c) => c.blown)) {
    return (
      <Badge tone="short" title={t("lab.verdict.ruined.title")}>
        {t("lab.verdict.ruined")}
      </Badge>
    );
  }
  return q.worstProfit > 0 ? (
    <Badge tone="long" title={t("lab.verdict.pass.title")}>
      {t("lab.verdict.pass")}
    </Badge>
  ) : (
    <Badge tone="warn" title={t("lab.verdict.negative.title")}>
      {t("lab.verdict.negative", { n: q.cells.filter((c) => c.profit <= 0).length })}
    </Badge>
  );
}

/** Pełny zestaw liczb z §4 dla JEDNEGO trybu. */
function Komorka({ cell, balance, onChart }: { cell: LabCell; balance: number; onChart: (n: string) => void }) {
  const t = useT();
  const tryb = TRYBY.find((x) => x.key === cell.mode);
  const prog = (balance * PODLOGA_RUINY_PCT) / 100;
  const ruina = cell.ruins > 0 || cell.blown;
  return (
    <div
      style={{
        border: `1px solid ${ruina ? "var(--short)" : "var(--line)"}`,
        borderRadius: 8,
        padding: "var(--sp-2)",
        background: ruina ? "color-mix(in srgb, var(--short) 7%, transparent)" : undefined,
        display: "grid",
        gap: 6,
      }}
    >
      <div className="row row--tight" style={{ alignItems: "baseline" }}>
        <b style={{ fontSize: 12 }} title={tryb ? t(tryb.hint) : undefined}>
          {tryb ? t(tryb.label) : cell.mode}
        </b>
        <span className="spacer" />
        {cell.partial && <Badge tone="warn">{t("lab.partial")}</Badge>}
        {cell.cutShort && (
          <Badge tone="short" title={t("lab.cutShort.title")}>
            {t("lab.cutShort")}
          </Badge>
        )}
      </div>

      <div className={`num ${cell.profit >= 0 ? "up" : "down"}`} style={{ fontSize: 20, fontWeight: 700 }}>
        {fmtKwota(cell.profit)}
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1fr auto", rowGap: 3, fontSize: 11 }}>
        <span className="hint">{t("lab.c.lossDays")}</span>
        <b className="num">{cell.lossDaysPct.toFixed(0)} %</b>

        <span className="hint">{t("lab.c.worstDay")}</span>
        <b className="num down">{fmtKwota(cell.worstDay, 2)}</b>

        <span className="hint">{t("lab.c.minEquity")}</span>
        <b className={`num ${cell.minEquity <= prog ? "down" : ""}`} title={t("lab.c.ruinLevel", { v: prog.toFixed(0) })}>
          {cell.minEquity.toFixed(0)} $
        </b>

        <span className="hint">{t("lab.c.ruins")}</span>
        <b className={`num ${cell.ruins > 0 ? "down" : ""}`}>{cell.ruins}</b>

        <span className="hint" title={t("lab.c.units.title")}>
          {t("lab.c.units")}
        </span>
        <b className="num">{cell.units}</b>

        {entrySourceCounts(cell).map(({ label, value }) => (
          <Fragment key={label}>
            <span className="hint" title={t("lab.sources.hint")}>{t(label)}</span>
            <b className="num">{optionalCount(value)}</b>
          </Fragment>
        ))}

        <span className="hint">{t("lab.c.tradingDays")}</span>
        <b className="num">{cell.tradingDays}</b>

        <span className="hint" style={{ opacity: 0.6 }} title={t("lab.c.maxDd.title")}>
          maxDD
        </span>
        <b className="num" style={{ opacity: 0.6 }}>
          {cell.maxDd.toFixed(0)} $
        </b>
      </div>

      {cell.months.length > 0 && (
        <div style={{ borderTop: "1px solid var(--line)", paddingTop: 5 }}>
          <div className="hint" style={{ fontSize: 10, marginBottom: 3 }}>
            {t("lab.c.months")}
          </div>
          <div className="row row--tight" style={{ flexWrap: "wrap", gap: 4 }}>
            {cell.months.map((m) => (
              <span
                key={m.month}
                className={`num ${m.profit >= 0 ? "up" : "down"}`}
                title={t("lab.c.monthTitle", { d: m.days, l: m.lossDays })}
                style={{ fontSize: 11, border: "1px solid var(--line)", borderRadius: 5, padding: "1px 5px" }}
              >
                {m.month.slice(5)}/{m.month.slice(2, 4)} {fmtKwota(m.profit)}
              </span>
            ))}
          </div>
        </div>
      )}

      {cell.chart && (
        <Button size="sm" variant="ghost" icon="chart" onClick={() => onChart(cell.chart)}>
          {t("lab.chart")}
        </Button>
      )}
    </div>
  );
}

function OcenaCzterotrybowa({
  quads,
  balance,
  onChart,
}: {
  quads: LabQuad[];
  balance: number;
  onChart: (n: string) => void;
}) {
  const t = useT();
  const [otwarty, setOtwarty] = useState<string | null>(null);
  const przeszlo = quads.filter((q) => q.survives).length;

  return (
    <Card
      title={t("lab.quad.title")}
      icon="grid"
      accent={przeszlo > 0 ? "var(--long)" : "var(--warn)"}
      subtitle={t("lab.quad.subtitle", { n: przeszlo, all: quads.length })}
      actions={<span className="hint">{t("lab.quad.action")}</span>}
    >
      <div className="lab__hintbox" style={{ marginBottom: "var(--sp-2)" }}>
        <Icon name="info" size={13} />
        <span>
          <RichT k="lab.quad.note" vars={{ v: ((balance * PODLOGA_RUINY_PCT) / 100).toFixed(0) }} />
        </span>
      </div>

      <div className="tbl-wrap">
        <table className="tbl">
          <thead>
            <tr>
              <th>{t("lab.col.preset")}</th>
              {TRYBY.map((tryb) => (
                <th key={tryb.key} title={t(tryb.hint)} style={{ textAlign: "right" }}>
                  {t(tryb.label).replace(" · ", "\n")}
                </th>
              ))}
              <th style={{ textAlign: "right" }} title={t("lab.col.floor.title")}>
                {t("lab.col.floor")}
              </th>
              <th style={{ textAlign: "right" }}>{t("lab.col.ruins")}</th>
              <th>{t("lab.col.verdict")}</th>
            </tr>
          </thead>
          <tbody>
            {quads.map((q) => {
              const dno = Math.min(...q.cells.map((c) => c.minEquity));
              const ruiny = q.cells.reduce((s, c) => s + c.ruins, 0);
              return (
                <tr
                  key={q.name}
                  onClick={() => setOtwarty(otwarty === q.name ? null : q.name)}
                  style={{ cursor: "pointer" }}
                >
                  <td>
                    <span className="truncate">{q.name}</span>
                  </td>
                  {TRYBY.map((tryb) => {
                    const c = q.cells.find((x) => x.mode === tryb.key);
                    return (
                      <td key={tryb.key} className="num" style={{ textAlign: "right" }}>
                        {c ? (
                          <span className={c.profit >= 0 ? "up" : "down"}>{fmtKwota(c.profit)}</span>
                        ) : (
                          <BrakPomiaru />
                        )}
                      </td>
                    );
                  })}
                  <td className={`num ${dno <= (balance * PODLOGA_RUINY_PCT) / 100 ? "down" : ""}`} style={{ textAlign: "right" }}>
                    {Number.isFinite(dno) ? `${dno.toFixed(0)} $` : "—"}
                  </td>
                  <td className={`num ${ruiny > 0 ? "down" : ""}`} style={{ textAlign: "right" }}>
                    {ruiny}
                  </td>
                  <td>
                    <Werdykt q={q} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {otwarty && (
        <div style={{ marginTop: "var(--sp-3)" }}>
          <div className="row" style={{ marginBottom: "var(--sp-2)" }}>
            <b>{otwarty}</b>
            <span className="spacer" />
            <Button size="sm" variant="ghost" icon="x" onClick={() => setOtwarty(null)}>
              {t("lab.collapse")}
            </Button>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(240px, 1fr))", gap: "var(--sp-2)" }}>
            {TRYBY.map((tryb) => {
              const c = quads.find((q) => q.name === otwarty)?.cells.find((x) => x.mode === tryb.key);
              return c ? (
                <Komorka key={tryb.key} cell={c} balance={balance} onChart={onChart} />
              ) : (
                <div
                  key={tryb.key}
                  style={{
                    border: "1px dashed var(--line)",
                    borderRadius: 8,
                    padding: "var(--sp-2)",
                    display: "grid",
                    gap: 6,
                  }}
                >
                  <b style={{ fontSize: 12 }} title={t(tryb.hint)}>
                    {t(tryb.label)}
                  </b>
                  <span className="hint">{t("lab.notMeasuredMode")}</span>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </Card>
  );
}

/* ============================================================
   TABELA WYNIKÓW
   ============================================================ */

type SortKey = keyof Pick<
  LabRow,
  "name" | "profit" | "perDay" | "maxDd" | "risk" | "riskPct" | "winDaysPct" | "winRate" | "trades" | "score"
>;

function Wyniki({ job, onChart }: { job: LabJob; onChart: (name: string) => void }) {
  const t = useT();
  const [sort, setSort] = useState<SortKey>("score");
  const [dir, setDir] = useState<1 | -1>(-1);
  const rows = job.rows ?? [];
  // przy ocenie czterotrybowej ta tabela pokazuje TYLKO pierwszy tryb —
  // przemilczenie tego zamieniłoby ją w czwartą część prawdy podaną jak całość
  const czterotrybowa = ((job as LabJobQ).quads?.length ?? 0) > 0;

  const sorted = useMemo(() => {
    const arr = [...rows];
    arr.sort((a, b) => {
      /* Przebiegi CZĄSTKOWE zawsze na dole, niezależnie od wybranej kolumny.
         Przerwany po tygodniu przebieg potrafi mieć najlepszą ocenę w tabeli
         wyłącznie dlatego, że nie zdążył zobaczyć złego reżimu — postawiony
         na szczycie rankingu byłby zwykłą pułapką. */
      if (a.partial !== b.partial) return a.partial ? 1 : -1;
      const x = a[sort];
      const y = b[sort];
      if (typeof x === "string" || typeof y === "string") {
        return String(x).localeCompare(String(y)) * dir;
      }
      return ((x as number) - (y as number)) * dir;
    });
    return arr;
  }, [rows, sort, dir]);

  const head = (key: SortKey, label: string, title?: string) => (
    <th
      className="sortable"
      title={title}
      onClick={() => {
        if (sort === key) setDir((d) => (d === 1 ? -1 : 1));
        else {
          setSort(key);
          setDir(-1);
        }
      }}
    >
      {label}
      {sort === key && <span className="sort-arrow">{dir === 1 ? "↑" : "↓"}</span>}
    </th>
  );

  if (rows.length === 0) return null;

  return (
    <Card
      title={czterotrybowa ? t("lab.res.titleQuad") : t("lab.res.title")}
      icon="chart"
      subtitle={
        czterotrybowa ? t("lab.res.subtitleQuad", { n: rows.length }) : t("lab.res.subtitle", { n: rows.length })
      }
      flush
      actions={<span className="hint">{t("lab.res.action")}</span>}
    >
      <div className="tbl-wrap">
        <table className="tbl">
          <thead>
            <tr>
              {head("name", t("lab.col.config"))}
              {head("profit", t("lab.col.profit"))}
              {head("perDay", t("lab.col.perDay"))}
              {head("maxDd", "maxDD $")}
              {head("risk", t("lab.col.risk"), t("lab.col.risk.title"))}
              {head("riskPct", t("lab.col.riskPct"))}
              <th>PF</th>
              {head("winDaysPct", t("lab.col.winDays"))}
              {head("winRate", t("lab.col.winRate"))}
              {head("trades", t("lab.col.trades"))}
              {entrySourceCounts({}).map(({ label }) => <th key={label} title={t("lab.sources.hint")}>{t(label)}</th>)}
              {head("score", t("lab.col.score"), t("lab.col.score.title"))}
              <th />
            </tr>
          </thead>
          <tbody>
            {sorted.map((r) => (
              <tr key={r.name} onClick={() => r.chart && onChart(r.chart)} style={{ cursor: r.chart ? "pointer" : "default" }}>
                <td>
                  <span className="truncate">{r.name}</span>
                  {r.blown && (
                    <Badge tone="short" title={t("lab.blown.title")}>
                      {t("lab.blown")}
                    </Badge>
                  )}
                  {r.partial && (
                    <Badge tone="warn" title={t("lab.partialRow.title")}>
                      {t("lab.partial")}
                    </Badge>
                  )}
                </td>
                <td className={`num ${r.profit >= 0 ? "up" : "down"}`}>{fmtKwota(r.profit)}</td>
                <td className="num">{r.perDay.toFixed(2)}</td>
                <td className="num down">{r.maxDd.toFixed(0)}</td>
                <td className="num">{r.risk.toFixed(0)}</td>
                <td className={`num ${r.riskPct > 50 ? "down" : ""}`}>{r.riskPct.toFixed(0)}</td>
                <td className="num">{fmtPf(r.profitFactor)}</td>
                <td className="num">{r.winDaysPct.toFixed(0)}</td>
                <td className="num">{r.winRate.toFixed(0)}</td>
                <td className="num">{r.trades}</td>
                {entrySourceCounts(r).map(({ label, value }) => <td key={label} className="num">{optionalCount(value)}</td>)}
                <td className="num">{r.score.toFixed(2)}</td>
                <td>{r.chart && <Icon name="chart" size={13} />}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Card>
  );
}

/* ============================================================
   WIDOK
   ============================================================ */

export function LabView() {
  const app = useApp();
  const t = useT();
  const [tab, setTab] = useState<Tab>("backtest");
  const [info, setInfo] = useState<LabInfo | null>(null);
  const [ladowanie, setLadowanie] = useState(true);
  const [podglad, setPodglad] = useState<string | null>(null);
  const [wyslane, setWyslane] = useState(false);

  const lab = app.lab;
  const job = lab.job;
  const busy = lab.busy;

  /* --- zlecenie backtestu --- */
  const [bt, setBt] = useState<BacktestReqQ>({
    period: "last-month",
    from: "",
    to: "",
    presetDir: "",
    preset: "",
    balance: 200,
    dailyReset: false,
    walkForward: 0,
    fourModes: false,
  });

  /* --- zlecenie treningu --- */
  const [tr, setTr] = useState<TrainReq>({
    from: "",
    to: "",
    validFrom: "",
    validTo: "",
    generations: 20,
    pop: 16,
    seed: 1,
    algo: "es",
    sigma: 0.05,
    lr: 0.05,
    windows: 6,
    windowDays: 3,
    validWindows: 5,
    balance: 1000,
    interval: 2,
    split: "chrono",
    blocks: 10,
    name: "",
    resume: false,
  });

  const odswiezInfo = useCallback(async () => {
    if (!app.live) {
      setLadowanie(false);
      return;
    }
    try {
      const i = await api.labInfo();
      setInfo(i);
      setBt((b) => (b.presetDir ? b : { ...b, presetDir: i.presetDirs[0]?.name ?? "" }));
      setTr((prev) => {
        if (prev.from || !i.ok) return prev;
        // domyślnie: dwie trzecie zakresu na trening, reszta na walidację
        const a = new Date(`${i.firstDay}T00:00:00Z`).getTime();
        const b = new Date(`${i.lastDay}T00:00:00Z`).getTime();
        const podzial = new Date(a + (b - a) * 0.66).toISOString().slice(0, 10);
        return { ...prev, from: i.firstDay, to: podzial, validFrom: podzial, validTo: i.lastDay };
      });
    } catch {
      setInfo(null);
    } finally {
      setLadowanie(false);
    }
  }, [app.live]);

  useEffect(() => {
    void odswiezInfo();
  }, [odswiezInfo]);

  /* po zakończeniu zadania listy na dysku się zmieniły (nowy model,
     nowy punkt kontrolny) — odświeżamy je raz, a nie w pętli */
  const poprzedniaFaza = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (poprzedniaFaza.current === "running" && job?.phase !== "running") void odswiezInfo();
    poprzedniaFaza.current = job?.phase;
  }, [job?.phase, odswiezInfo]);

  const start = async () => {
    setWyslane(true);
    try {
      // `fourModes` nie ma jeszcze w typie transportu (plik zespołu PANEL),
      // ale serwer to pole rozumie — patrz `BacktestReq` w `lab/backtests.rs`
      if (tab === "backtest") await api.labBacktest(bt as BacktestReq);
      else await api.labTrain(tr);
      app.toast("info", tab === "train" ? t("lab.toast.trainStarted") : t("lab.toast.btStarted"), t("lab.toast.progressBelow"));
    } catch (e) {
      app.toast("error", t("lab.toast.startFailed"), String(e).replace(/^Error:\s*/, ""));
    } finally {
      setWyslane(false);
    }
  };

  const przerwij = async () => {
    try {
      await api.labCancel();
      app.toast("warn", t("lab.toast.stopping"), t("lab.toast.stopping.text"));
    } catch (e) {
      app.toast("error", t("lab.toast.cancelFailed"), String(e).replace(/^Error:\s*/, ""));
    }
  };

  const otworzKatalog = async () => {
    if (!job) return;
    try {
      await api.labOpenDir(job.id);
    } catch (e) {
      app.toast("error", t("lab.toast.dirFailed"), String(e).replace(/^Error:\s*/, ""));
    }
  };

  /* ---------------- brak backendu ---------------- */
  if (!app.live) {
    return (
      <div className="view">
        <div className="view__head">
          <div className="view__headmain">
            <h1>{t("lab.title")}</h1>
            <p>{t("lab.subtitle")}</p>
          </div>
        </div>
        <Card>
          <Empty icon="microscope" title={t("lab.needServer")} text={t("lab.needServer.text")} />
        </Card>
      </div>
    );
  }

  const dirs = info?.presetDirs ?? [];
  const wybranyKatalog = dirs.find((d) => d.name === bt.presetDir);
  const ck = info?.checkpoint ?? null;

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("lab.title")}</h1>
          <p>
            <RichT k="lab.intro" />
          </p>
        </div>
        {info && (
          <div className="lab__data">
            {info.ok ? (
              <>
                <span className="num">{t("lab.data.ticks", { v: (info.ticks / 1e6).toFixed(1) })}</span>
                <span className="hint">
                  {info.firstDay} … {info.lastDay} · {t("lab.data.messages", { n: num(info.messages, 0) })}
                </span>
              </>
            ) : (
              <span className="down">{tSilnik(info.error) || t("lab.data.none")}</span>
            )}
          </div>
        )}
      </div>

      {/* ================= ZLECENIE ================= */}
      <Card
        title={t("lab.new.title")}
        icon="play"
        accent="var(--info)"
        actions={
          <Segmented<Tab>
            value={tab}
            onChange={setTab}
            size="sm"
            options={[
              { value: "backtest", label: <><Icon name="microscope" size={12} /> {t("lab.tab.backtest")}</> },
              { value: "train", label: <><Icon name="brain" size={12} /> {t("lab.tab.train")}</> },
            ]}
          />
        }
      >
        {tab === "backtest" ? (
          <>
            <div className="formgrid">
              <Field label={t("lab.f.period")}>
                <Select
                  value={bt.period}
                  onChange={(v) => setBt({ ...bt, period: v })}
                  options={[
                    { value: "all", label: t("lab.f.period.all") },
                    { value: "last-month", label: t("lab.f.period.month") },
                    { value: "last-week", label: t("lab.f.period.week") },
                    { value: "last-3-days", label: t("lab.f.period.3days") },
                    { value: "range", label: t("lab.f.period.range") },
                  ]}
                />
              </Field>
              {bt.period === "range" && (
                <>
                  <Field label={t("lab.f.from")} hint={t("lab.f.from.hint", { v: info?.firstDay ?? "?" })}>
                    <TextInput
                      type="date"
                      value={bt.from ?? ""}
                      onChange={(v) => setBt({ ...bt, from: v })}
                      placeholder={info?.firstDay}
                    />
                  </Field>
                  <Field label={t("lab.f.to")} hint={t("lab.f.to.hint", { v: info?.lastDay ?? "?" })}>
                    <TextInput
                      type="date"
                      value={bt.to ?? ""}
                      onChange={(v) => setBt({ ...bt, to: v })}
                      placeholder={info?.lastDay}
                    />
                  </Field>
                </>
              )}
              <Field label={t("lab.f.presetDir")}>
                <Select
                  value={bt.presetDir ?? ""}
                  onChange={(v) => setBt({ ...bt, presetDir: v, preset: "" })}
                  options={[
                    { value: "", label: t("lab.f.presetDir.default") },
                    ...dirs.map((d) => ({ value: d.name, label: `${d.name} (${d.count})` })),
                  ]}
                />
              </Field>
              {/* Presety ROZDZIELONE po formacie, tak samo jak w galerii i w
                  łańcuchach. Katalog laboratorium oddaje same nazwy plików,
                  więc format dobieramy z listy presetów bota; czego tam nie ma,
                  ląduje w grupie „format nieznany" — a nie w ATFX na wyrost.
                  Zmyślony format w backteście znaczy porównywanie wyników
                  z dwóch różnych sposobów handlu. */}
              <Field
                label={t("lab.f.preset")}
                hint={wybranyKatalog ? t("lab.f.preset.hint", { n: wybranyKatalog.count }) : ""}
              >
                <Select
                  value={bt.preset ?? ""}
                  onChange={(v) => setBt({ ...bt, preset: v })}
                  disabled={!wybranyKatalog}
                  options={[
                    { value: "", label: t("lab.f.preset.sweep") },
                    ...(wybranyKatalog?.presets ?? []).map((p) => {
                      const znany = app.findPreset(p);
                      return {
                        value: p,
                        label: p,
                        group: znany ? t("sims.group.format", { v: formatPresetu(znany) }) : t("lab.f.preset.unknownFmt"),
                      };
                    }),
                  ]}
                />
              </Field>
              <Field label={t("lab.f.balance")}>
                <NumberInput value={bt.balance} onChange={(v) => setBt({ ...bt, balance: v })} step={50} min={20} unit="$" />
              </Field>
              <Field
                label="Walk-forward"
                hint={bt.fourModes ? t("lab.f.wf.off") : t("lab.f.wf.hint")}
              >
                <NumberInput
                  value={bt.walkForward}
                  onChange={(v) => setBt({ ...bt, walkForward: v })}
                  step={1}
                  min={0}
                  unit={t("lab.f.wf.unit")}
                />
              </Field>
            </div>
            {}
            <div className="row row--tight" style={{ marginTop: "var(--sp-2)", flexWrap: "wrap" }}>
              <span className="hint">{t("lab.windows")}</span>
              {[
                { l: t("lab.win.fromJune"), f: "2026-06-01", t: "", h: t("lab.win.fromJune.title") },
                { l: t("lab.win.july"), f: "2026-07-01", t: "", h: t("lab.win.july.title") },
                { l: t("lab.win.june"), f: "2026-06-01", t: "2026-06-30", h: t("lab.win.june.title") },
                { l: t("lab.win.aprMay"), f: "2026-04-01", t: "2026-05-31", h: t("lab.win.aprMay.title") },
              ].map((w) => (
                <Button
                  key={w.l}
                  size="sm"
                  variant={bt.period === "range" && bt.from === w.f && (bt.to ?? "") === w.t ? "primary" : "outline"}
                  title={w.h}
                  onClick={() => setBt({ ...bt, period: "range", from: w.f, to: w.t })}
                >
                  {w.l}
                </Button>
              ))}
            </div>

            <div
              className="lab__resume"
              style={{ marginTop: "var(--sp-2)", borderColor: bt.fourModes ? "var(--accent)" : undefined }}
            >
              <Icon name="grid" size={14} />
              <div>
                <b>{t("lab.quad.title")}</b>
                <span>
                  <RichT k="lab.quad.desc" />
                </span>
              </div>
              <Checkbox checked={!!bt.fourModes} onChange={(v) => setBt({ ...bt, fourModes: v })} label={t("lab.enable")} />
            </div>

            <div className="row" style={{ marginTop: "var(--sp-3)" }}>
              <Checkbox
                checked={bt.dailyReset}
                onChange={(v) => setBt({ ...bt, dailyReset: v })}
                disabled={!!bt.fourModes}
                label={t("lab.dailyReset")}
                title={bt.fourModes ? t("lab.dailyReset.off") : t("lab.dailyReset.title")}
              />
              <span className="spacer" />
              <Button variant="primary" icon="play" disabled={busy || wyslane || !info?.ok} onClick={() => void start()}>
                {busy ? t("lab.busy") : bt.fourModes ? t("lab.runQuad") : t("lab.runBt")}
              </Button>
            </div>
          </>
        ) : (
          <>
            <div className="formgrid">
              <Field label={t("lab.f.trainFrom")} hint={t("lab.f.dateFmt")}>
                <TextInput value={tr.from} onChange={(v) => setTr({ ...tr, from: v })} placeholder={info?.firstDay} />
              </Field>
              <Field label={t("lab.f.trainTo")}>
                <TextInput value={tr.to} onChange={(v) => setTr({ ...tr, to: v })} />
              </Field>
              <Field label={t("lab.f.validFrom")} hint={t("lab.f.validFrom.hint")}>
                <TextInput value={tr.validFrom} onChange={(v) => setTr({ ...tr, validFrom: v })} />
              </Field>
              <Field label={t("lab.f.validTo")}>
                <TextInput value={tr.validTo} onChange={(v) => setTr({ ...tr, validTo: v })} placeholder={info?.lastDay} />
              </Field>
              <Field label={t("lab.f.generations")}>
                <NumberInput value={tr.generations} onChange={(v) => setTr({ ...tr, generations: v })} step={5} min={1} />
              </Field>
              <Field label={t("lab.f.pop")} hint={t("lab.f.pop.hint")}>
                <NumberInput value={tr.pop} onChange={(v) => setTr({ ...tr, pop: v })} step={4} min={4} />
              </Field>
              <Field label={t("demo.f.seed")} hint={t("lab.f.seed.hint")}>
                <NumberInput value={tr.seed} onChange={(v) => setTr({ ...tr, seed: v })} step={1} min={0} />
              </Field>
              <Field label={t("aim.m.algo")}>
                <Select
                  value={tr.algo}
                  onChange={(v) => setTr({ ...tr, algo: v })}
                  options={[
                    { value: "es", label: "ES (OpenAI-ES)" },
                    { value: "cem", label: t("lab.f.algo.cem") },
                  ]}
                />
              </Field>
              <Field label={t("aim.m.split")}>
                <Select
                  value={tr.split}
                  onChange={(v) => setTr({ ...tr, split: v })}
                  options={[
                    { value: "chrono", label: t("lab.f.split.chrono") },
                    { value: "interleave", label: t("lab.f.split.interleave") },
                  ]}
                />
              </Field>
              <Field label={t("aim.m.windows")} hint={t("lab.f.windows.hint", { n: tr.windowDays })}>
                <NumberInput value={tr.windows} onChange={(v) => setTr({ ...tr, windows: v })} step={1} min={1} />
              </Field>
              <Field label={t("lab.f.validWindows")}>
                <NumberInput value={tr.validWindows} onChange={(v) => setTr({ ...tr, validWindows: v })} step={1} min={1} />
              </Field>
              <Field label={t("aim.m.capital")}>
                <NumberInput value={tr.balance} onChange={(v) => setTr({ ...tr, balance: v })} step={100} min={100} unit="$" />
              </Field>
              <Field label={t("lab.f.modelName")} hint={t("lab.f.modelName.hint")}>
                <TextInput value={tr.name} onChange={(v) => setTr({ ...tr, name: v })} placeholder="atfx_v2" />
              </Field>
            </div>

            {ck && (
              <div className="lab__resume">
                <Icon name="hourglass" size={14} />
                <div>
                  <b>{t("lab.ck.title")}</b>
                  <span>
                    {t("lab.ck.text", {
                      gen: ck.gen,
                      fit: ck.bestCenter.toFixed(4),
                      seed: ck.seed,
                      algo: ck.algo.toUpperCase(),
                      next: ck.gen + 1,
                    })}
                  </span>
                </div>
                <Checkbox checked={tr.resume} onChange={(v) => setTr({ ...tr, resume: v })} label={t("lab.ck.resume")} />
              </div>
            )}

            <div className="row" style={{ marginTop: "var(--sp-3)" }}>
              <span className="hint" style={{ flex: 1, minWidth: 280 }}>
                {t("lab.train.note")}
              </span>
              <Button variant="primary" icon="play" disabled={busy || wyslane || !info?.ok} onClick={() => void start()}>
                {busy ? t("lab.busy") : tr.resume ? t("lab.resumeTrain") : t("lab.runTrain")}
              </Button>
            </div>
          </>
        )}
      </Card>

      {/* ================= POSTĘP ================= */}
      {ladowanie && !job && (
        <Card>
          <Empty icon="hourglass" title={t("lab.checking")} />
        </Card>
      )}

      {job && <Postep job={job} busy={busy} onCancel={() => void przerwij()} onOpenDir={() => void otworzKatalog()} />}

      {job?.kind === "train" && (job.gens?.length ?? 0) > 0 && (
        <Card title={t("lab.fit.title")} icon="trend" subtitle={t("lab.fit.subtitle", { n: job.gens?.length ?? 0 })}>
          <FitnessChart gens={job.gens ?? []} />
        </Card>
      )}

      {((job as LabJobQ | null)?.quads?.length ?? 0) > 0 && (
        <OcenaCzterotrybowa
          quads={(job as LabJobQ).quads ?? []}
          balance={bt.balance}
          onChart={setPodglad}
        />
      )}

      {job && <Wyniki job={job} onChart={setPodglad} />}

      {/* ================= PODGLĄD WYKRESU ================= */}
      {job && podglad && (
        <Card
          title={t("lab.chartOf", { v: podglad })}
          icon="chart"
          actions={
            <Button size="sm" variant="ghost" icon="x" onClick={() => setPodglad(null)}>
              {t("common.close")}
            </Button>
          }
        >
          <div className="lab__svg">
            <img src={api.labFileUrl(job.id, podglad)} alt={t("lab.chartAlt", { v: podglad })} />
          </div>
        </Card>
      )}

      {job && (job.charts?.length ?? 0) > 0 && !podglad && (
        <Card
          title={t("lab.charts")}
          icon="image"
          subtitle={`${job.charts?.length ?? 0}`}
          actions={
            (job.charts?.length ?? 0) > CHARTS_SHOWN ? (
              <span className="hint">{t("lab.charts.more", { n: CHARTS_SHOWN, all: job.charts?.length ?? 0 })}</span>
            ) : undefined
          }
        >
          <div className="row row--tight">
            {(job.charts ?? []).slice(0, CHARTS_SHOWN).map((c) => (
              <Button key={c} size="sm" variant="outline" icon="chart" onClick={() => setPodglad(c)}>
                {c.replace(/\.svg$/, "")}
              </Button>
            ))}
          </div>
        </Card>
      )}

      {/* ================= HISTORIA ================= */}
      {lab.history.length > 0 && (
        <Card title={t("lab.hist.title")} icon="history" subtitle={`${lab.history.length}`}>
          <div className="lab__hist">
            {lab.history.map((h) => (
              <article key={h.id} className="lab__histitem">
                <Badge tone={(FAZA[h.phase] ?? FAZA.done).tone}>{t((FAZA[h.phase] ?? FAZA.done).label)}</Badge>
                <div className="lab__histbody">
                  <b className="truncate">{tSilnik(h.title)}</b>
                  <span className="hint truncate">{tSilnik(h.note || h.error) || "—"}</span>
                </div>
                <span className="num hint">{czasKrotki(h.elapsedMs)}</span>
              </article>
            ))}
          </div>
        </Card>
      )}

      {info && !info.ok && (
        <Card>
          <Empty
            icon="alert"
            title={t("lab.noData")}
            text={t("lab.noData.text", { err: tSilnik(info.error), path: info.ticksPath })}
          />
        </Card>
      )}
    </div>
  );
}
