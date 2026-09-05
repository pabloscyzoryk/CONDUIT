import { useMemo, useState } from "react";
import { Badge, Button, Card, Empty, Select } from "@/components/ui";
import { EksportHistorii } from "@/components/panels/ExportPanel";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import { dateTime, duration, money, num, toneOf } from "@/lib/format";
import { isManaged, type CloseReason, type PositionSource } from "@/types";
import "./views.css";

/** Okresy: wartość jest LICZBĄ MINUT (albo `session`/`all`), etykieta
 *  mieszka w słowniku pod `hist.period.<wartość>`. */
const PERIODS = ["session", "10", "30", "60", "180", "720", "1440", "4320", "10080", "43200", "all"];

/** KLUCZE SŁOWNIKA powodów zamknięcia — etykietę daje `t(REASON_LABEL[r])`. */
const REASON_LABEL: Record<CloseReason, string> = {
  TP: "hist.reason.tp",
  SL: "hist.reason.sl",
  VSL: "hist.reason.vsl",
  MANUAL: "hist.reason.manual",
  PARTIAL: "hist.reason.partial",
  BASKET: "hist.reason.basket",
  RISK_FREE: "hist.reason.riskFree",
  OAE: "hist.reason.oae",
  HARVEST: "hist.reason.harvest",
  STALE: "hist.reason.stale",
  TRAIL: "hist.reason.trail",
  EOD: "hist.reason.eod",
  DAY_TARGET: "hist.reason.dayTarget",
  MAX_DD: "hist.reason.maxDd",
  AI: "hist.reason.ai",
};

const REASON_TONE: Record<CloseReason, "long" | "short" | "warn" | "info" | "muted" | "ai"> = {
  TP: "long",
  SL: "short",
  VSL: "short",
  MANUAL: "muted",
  PARTIAL: "long",
  BASKET: "muted",
  RISK_FREE: "info",
  OAE: "warn",
  HARVEST: "long",
  STALE: "warn",
  TRAIL: "long",
  EOD: "info",
  DAY_TARGET: "long",
  MAX_DD: "short",
  AI: "ai",
};

type SortKey = "closeTime" | "openTime" | "profit";
/** Zakres historii: caly rachunek / tylko bot / tylko spoza bota. */
type Scope = "all" | "bot" | "foreign";

/** KLUCZE SŁOWNIKA zakresu (dopełniacz w PL: „dotyczą całego rachunku”). */
const SCOPE_LABEL: Record<Scope, string> = {
  all: "hist.scope.all",
  bot: "hist.scope.bot",
  foreign: "hist.scope.foreign",
};

export function HistoryView() {
  const app = useApp();
  const t = useT();
  const cur = app.settings.display_currency;
  const [period, setPeriod] = useState("session");
  const [reason, setReason] = useState("all");
  const [sort, setSort] = useState<SortKey>("closeTime");
  const [dir, setDir] = useState<1 | -1>(-1);
  const [scope, setScope] = useState<Scope>("all");

  const from = useMemo(() => {
    if (period === "all") return 0;
    if (period === "session") return app.stats.sessionStart;
    return Date.now() - Number(period) * 60000;
  }, [period, app.stats.sessionStart]);

  /* Ile w historii jest transakcji spoza bota — decyduje, czy w ogole
     pokazujemy przelacznik zakresu. */
  const obceCount = useMemo(
    () => app.snapshot.closed.filter((c) => !isManaged(c.source)).length,
    [app.snapshot.closed],
  );

  const closed = useMemo(() => {
    const arr = app.snapshot.closed.filter(
      (c) =>
        c.closeTime >= from &&
        (reason === "all" || c.reason === reason) &&
        (scope === "all" || (scope === "bot") === isManaged(c.source)),
    );
    arr.sort((a, b) => {
      const va = sort === "profit" ? a.profit : sort === "openTime" ? a.openTime : a.closeTime;
      const vb = sort === "profit" ? b.profit : sort === "openTime" ? b.openTime : b.closeTime;
      return (va - vb) * dir;
    });
    return arr;
  }, [app.snapshot.closed, from, reason, sort, dir, scope]);

  const pendHist = useMemo(
    () => app.snapshot.pendingHistory.filter((p) => p.endTime >= from),
    [app.snapshot.pendingHistory, from],
  );

  const summary = useMemo(() => {
    const total = closed.reduce((a, c) => a + c.profit, 0);
    const wins = closed.filter((c) => c.profit > 0);
    const losses = closed.filter((c) => c.profit < 0);
    const grossWin = wins.reduce((a, c) => a + c.profit, 0);
    const grossLoss = Math.abs(losses.reduce((a, c) => a + c.profit, 0));
    const best = closed.reduce((a, c) => Math.max(a, c.profit), 0);
    const worst = closed.reduce((a, c) => Math.min(a, c.profit), 0);
    const avgHold = closed.length
      ? closed.reduce((a, c) => a + (c.closeTime - c.openTime), 0) / closed.length
      : 0;
    return {
      total,
      count: closed.length,
      winRate: closed.length ? (wins.length / closed.length) * 100 : 0,
      pf: grossLoss > 0 ? grossWin / grossLoss : grossWin > 0 ? Infinity : 0,
      best,
      worst,
      avgHold,
      volume: closed.reduce((a, c) => a + c.volume, 0),
    };
  }, [closed]);

  const head = (key: SortKey, label: string) => (
    <th
      className="sortable"
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

  const usedReasons = Array.from(new Set(app.snapshot.closed.map((c) => c.reason)));

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("hist.title")}</h1>
          <p>{t("hist.intro")}</p>
        </div>
        <div className="row row--tight">
          {obceCount > 0 && (
            <div className="scope-switch" role="group" aria-label={t("hist.scopeAria")}>
              {(
                [
                  ["all", t("hist.scopeBtn.all")],
                  ["bot", t("hist.scopeBtn.bot")],
                  ["foreign", t("hist.scopeBtn.foreign")],
                ] as [Scope, string][]
              ).map(([k, lbl]) => (
                <button
                  key={k}
                  type="button"
                  className={`scope-switch__b ${scope === k ? "is-on" : ""}`}
                  onClick={() => setScope(k)}
                >
                  {lbl}
                </button>
              ))}
            </div>
          )}
          <Select
            value={period}
            onChange={setPeriod}
            options={PERIODS.map((p) => ({ value: p, label: t(`hist.period.${p}`) }))}
            size="sm"
            style={{ minWidth: 200 }}
          />
          <Select
            value={reason}
            onChange={setReason}
            size="sm"
            options={[
              { value: "all", label: t("hist.reason.all") },
              ...usedReasons.map((r) => ({ value: r, label: t(REASON_LABEL[r]) })),
            ]}
          />
          {/* Eksport bierze DOKŁADNIE te filtry, które widać na ekranie —
              plik z innym zakresem niż tabela nad nim byłby pułapką. */}
          <EksportHistorii
            ile={closed.length}
            filtr={{
              from: period === "all" ? undefined : from,
              scope: scope === "all" ? undefined : scope,
              reason: reason === "all" ? undefined : reason,
            }}
          />
        </div>
      </div>

      {/* Etykieta zakresu przy statystykach: bez niej „profit factor" milczaco
          zmienialby znaczenie po przelaczeniu filtra. */}
      <div className="hscope">
        {t("hist.scopeNote")} <b>{t(SCOPE_LABEL[scope])}</b>
        {obceCount > 0 && scope === "all" && t("hist.scopeNote.mixed")}
      </div>
      <div className="hstats">
        <div className="hstat">
          <b className={toneOf(summary.total)}>
            {summary.total >= 0 ? "+" : "−"}
            {money(Math.abs(summary.total), cur)}
          </b>
          <span>{t("hist.stat.net")}</span>
        </div>
        <div className="hstat">
          <b>{summary.count}</b>
          <span>{t("hist.stat.trades")}</span>
        </div>
        <div className="hstat">
          <b className={summary.winRate >= 50 ? "up" : "down"}>{summary.winRate.toFixed(1)}%</b>
          <span>{t("hist.stat.winRate")}</span>
        </div>
        <div className="hstat">
          <b>{summary.pf === Infinity ? "∞" : summary.pf.toFixed(2)}</b>
          <span>{t("hist.stat.pf")}</span>
        </div>
        <div className="hstat">
          <b className="up">+{money(summary.best, cur)}</b>
          <span>{t("hist.stat.best")}</span>
        </div>
        <div className="hstat">
          <b className="down">{money(summary.worst, cur)}</b>
          <span>{t("hist.stat.worst")}</span>
        </div>
        <div className="hstat">
          <b>{summary.avgHold ? duration(summary.avgHold) : "—"}</b>
          <span>{t("hist.stat.avgHold")}</span>
        </div>
        <div className="hstat">
          <b>{summary.volume.toFixed(2)}</b>
          <span>{t("hist.stat.volume")}</span>
        </div>
      </div>

      <Card
        title={scope === "bot" ? t("hist.closed.titleBot") : t("hist.closed.title")}
        icon="history"
        subtitle={`${closed.length}`}
        accent="var(--accent)"
        flush
        actions={
          <Button
            size="sm"
            variant="outline"
            icon="download"
            onClick={() => app.toast("info", t("hist.csv"), t("hist.csv.text"))}
          >
            {t("hist.csv")}
          </Button>
        }
      >
        {closed.length === 0 ? (
          <Empty icon="history" title={t("hist.closed.empty")} text={t("hist.closed.emptyText")} />
        ) : (
          <div className="tbl-wrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>{t("hist.col.position")}</th>
                  <th style={{ textAlign: "left" }}>{t("hist.col.type")}</th>
                  <th>{t("hist.col.lot")}</th>
                  {head("openTime", t("hist.col.open"))}
                  <th>{t("hist.col.openPrice")}</th>
                  {head("closeTime", t("hist.col.close"))}
                  <th>{t("hist.col.closePrice")}</th>
                  <th>{t("hist.col.time")}</th>
                  <th style={{ textAlign: "left" }}>{t("hist.col.source")}</th>
                  <th style={{ textAlign: "left" }}>{t("hist.col.reason")}</th>
                  <th>{t("hist.col.swap")}</th>
                  {head("profit", t("hist.col.profit"))}
                </tr>
              </thead>
              <tbody>
                {closed.map((c) => (
                  <tr key={c.ticket} className={isManaged(c.source) ? "" : "row--foreign"}>
                    <td style={{ textAlign: "left" }}>
                      <div className="cell-stack">
                        <span className="num cell-strong">#{c.ticket}</span>
                        <span className="cell-sub">{c.basketId !== null ? `B${c.basketId}` : t("hist.manualPos")}</span>
                      </div>
                    </td>
                    <td style={{ textAlign: "left" }}>
                      <Badge tone={c.direction === "BUY" ? "long" : "short"}>{c.direction}</Badge>
                    </td>
                    <td className="num">{c.volume.toFixed(2)}</td>
                    <td className="num cell-sub">{dateTime(c.openTime)}</td>
                    <td className="num">{num(c.openPrice, 2)}</td>
                    <td className="num cell-sub">{dateTime(c.closeTime)}</td>
                    <td className="num">{num(c.closePrice, 2)}</td>
                    <td className="num cell-sub">{duration(c.closeTime - c.openTime)}</td>
                    <td style={{ textAlign: "left" }}>
                      <ZrodloBadge src={c.source} magic={c.magic} symbol={c.symbol} />
                    </td>
                    <td style={{ textAlign: "left" }}>
                      {isManaged(c.source) ? (
                        <span title={c.reason === "RISK_FREE" ? t("hist.reason.riskFree.hint") : undefined}>
                          <Badge tone={REASON_TONE[c.reason]}>{t(REASON_LABEL[c.reason])}</Badge>
                        </span>
                      ) : (
                        /* Powodu cudzego zamkniecia NIE znamy — i tego nie udajemy. */
                        <span className="cell-sub">{t("hist.reason.unknown")}</span>
                      )}
                    </td>
                    <td className="num cell-sub">{money(c.swap + c.commission, cur)}</td>
                    <td className={`num cell-strong ${toneOf(c.profit)}`}>
                      {c.profit >= 0 ? "+" : "−"}
                      {money(Math.abs(c.profit), cur)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      <Card
        title={t("hist.pend.title")}
        icon="clock"
        subtitle={`${pendHist.length}`}
        accent="var(--warn)"
        flush
      >
        {pendHist.length === 0 ? (
          <Empty icon="clock" title={t("hist.pend.empty")} />
        ) : (
          <div className="tbl-wrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>{t("hist.col.ticket")}</th>
                  <th style={{ textAlign: "left" }}>{t("hist.col.type")}</th>
                  <th>{t("hist.col.lot")}</th>
                  <th>{t("hist.col.price")}</th>
                  <th>SL</th>
                  <th>TP</th>
                  <th>{t("hist.col.placed")}</th>
                  <th>{t("hist.col.ended")}</th>
                  <th style={{ textAlign: "left" }}>{t("hist.col.status")}</th>
                </tr>
              </thead>
              <tbody>
                {pendHist.map((p) => (
                  <tr key={`${p.ticket}-${p.endTime}`}>
                    <td className="num cell-strong" style={{ textAlign: "left" }}>
                      #{p.ticket}
                    </td>
                    <td style={{ textAlign: "left" }}>
                      <Badge tone={p.kind.startsWith("BUY") ? "long" : "short"}>{p.kind.replace("_", " ")}</Badge>
                    </td>
                    <td className="num">{p.volume.toFixed(2)}</td>
                    <td className="num">{num(p.price, 2)}</td>
                    <td className="num down">{p.sl ? num(p.sl, 2) : "—"}</td>
                    <td className="num up">{p.tp ? num(p.tp, 2) : "—"}</td>
                    <td className="num cell-sub">{dateTime(p.placedTime)}</td>
                    <td className="num cell-sub">{dateTime(p.endTime)}</td>
                    <td style={{ textAlign: "left" }}>
                      <Badge tone={p.status === "FILLED" ? "long" : p.status === "EXPIRED" ? "warn" : "muted"}>
                        {p.status === "FILLED"
                          ? t("hist.status.filled")
                          : p.status === "EXPIRED"
                            ? t("hist.status.expired")
                            : t("hist.status.cancelled")}
                      </Badge>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </div>
  );
}

/** Plakietka źródła w historii. Dla transakcji bota — dyskretna. */
function ZrodloBadge({
  src,
  magic,
  symbol,
}: {
  src?: PositionSource;
  magic?: number | null;
  symbol?: string;
}) {
  const t = useT();
  const s = src ?? "BOT";
  if (s === "BOT") return <span className="cell-sub">CONDUIT</span>;
  /* `null` znaczy „nie wiemy", a NIE „magic 0" — zero ma w MT5 konkretne
     znaczenie („otwarte recznie z terminala"), wiec podstawianie go w miejsce
     brakujacej wartosci bylo wypisywaniem nieprawdy. */
  const opisMagic = magic == null ? t("hist.magicUnknown") : t("hist.magic", { v: magic });
  return (
    <span className={`srctag srctag--${s.toLowerCase()}`} title={`${opisMagic} · ${symbol ?? ""}`}>
      {t(`src.${s.toLowerCase()}`)}
    </span>
  );
}
