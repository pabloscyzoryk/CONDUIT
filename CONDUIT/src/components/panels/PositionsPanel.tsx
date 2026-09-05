import { useMemo, useState } from "react";
import { Badge, Button, Card, Checkbox, Empty, Icon, NumberInput, Tooltip } from "@/components/ui";
import { usePotwierdzenie } from "@/components/ui/Potwierdzenie";
import { useApp } from "@/store/AppStore";
import { useT, t } from "@/i18n";
import { money, num, time, toneOf } from "@/lib/format";
import { potentialAt } from "@/engine/bot";
import { isManaged, type Position, type PositionSource } from "@/types";
import "./panels.css";

type SortKey = "time" | "profit" | "open";
/** Filtr widoku: wszystko / tylko bot / tylko spoza bota. */
type Scope = "all" | "bot" | "foreign";

/** Polska odmiana liczebników: 1 pozycja · 2-4 pozycje · 5+ pozycji.
 *  Angielski ma `.few === .many`, więc ta sama funkcja obsługuje oba języki. */
export function odmiana(base: string, n: number): string {
  if (n === 1) return t(`${base}.one`);
  return t(n < 5 ? `${base}.few` : `${base}.many`);
}

/** Plakietka zrodla. Dla pozycji bota nie zasmiecamy wiersza — brak plakietki
 *  znaczy „nasze". Pokazujemy ja dopiero tam, gdzie zmienia znaczenie. */
export function SourceTag({ src, magic }: { src?: PositionSource; magic?: number | null }) {
  const s = src ?? "BOT";
  if (s === "BOT") return null;
  return (
    <Tooltip content={`${t(`src.${s.toLowerCase()}.hint`)}${magic ? t("src.magic", { v: magic }) : ""}`}>
      <span className={`srctag srctag--${s.toLowerCase()}`}>{t(`src.${s.toLowerCase()}`)}</span>
    </Tooltip>
  );
}

export function PositionsPanel() {
  const app = useApp();
  const tt = useT();
  const { zapytaj, okno } = usePotwierdzenie();
  const { positions } = app.snapshot;

  
  const czytelny = (c: string) => /(?:\d+|x)\.\d+/.test(c);
  const bezKoszyka = useMemo(() => {
    const brak = positions.filter((p) => isManaged(p.source) && p.basketId == null);
    const reczne = brak.filter((p) => czytelny(p.comment ?? "")).length;
    return { razem: brak.length, reczne, sieroty: brak.length - reczne };
  }, [positions]);
  const cur = app.settings.display_currency;
  const [sort, setSort] = useState<SortKey>("time");
  const [dir, setDir] = useState<1 | -1>(-1);
  const [scope, setScope] = useState<Scope>("all");
  const [edit, setEdit] = useState<number | null>(null);
  const [draft, setDraft] = useState<{ sl: number; tp: number }>({ sl: 0, tp: 0 });

  const showPot = app.settings.show_potential_tpsl;

  /* Rozdzial na prowadzone przez bota i reszte rachunku. Bot ZARZADZA tylko
     pierwsza grupa — i tylko na niej wolno dzialac przyciskom hurtowym. */
  const botPos = useMemo(() => positions.filter((p) => isManaged(p.source)), [positions]);
  const obcePos = useMemo(() => positions.filter((p) => !isManaged(p.source)), [positions]);

  const rows = useMemo(() => {
    const base = scope === "bot" ? botPos : scope === "foreign" ? obcePos : positions;
    const arr = [...base];
    arr.sort((a, b) => {
      const va = sort === "profit" ? a.profit : sort === "open" ? a.openPrice : a.openTime;
      const vb = sort === "profit" ? b.profit : sort === "open" ? b.openPrice : b.openTime;
      return (va - vb) * dir;
    });
    return arr;
  }, [positions, botPos, obcePos, scope, sort, dir]);

  /* Suma CALEGO rachunku — bo tyle realnie plynie po equity. Obok, gdy jest
     co rozdzielac, pokazujemy ile z tego jest bota. */
  const total = positions.reduce((a, p) => a + p.profit, 0);
  const totalBot = botPos.reduce((a, p) => a + p.profit, 0);
  const totalObce = obcePos.reduce((a, p) => a + p.profit, 0);
  const volObce = obcePos.reduce((a, p) => a + p.volume, 0);

  /* Potencjal TP/SL liczymy WYLACZNIE dla pozycji bota: dla cudzych nie znamy
     zamiaru wlasciciela, a ich TP/SL moze siedziec na innym instrumencie. */
  const potentials = useMemo(() => {
    const pend = app.snapshot.pendings.filter((o) => isManaged(o.source));
    const src = app.settings.exclude_pending_potential
      ? { pos: botPos, pend: [] as typeof app.snapshot.pendings }
      : { pos: botPos, pend };
    let atTp = 0;
    let atSl = 0;
    for (const p of src.pos) {
      atTp += potentialAt(p, p.tp) ?? 0;
      atSl += potentialAt(p, p.sl) ?? 0;
    }
    for (const o of src.pend) {
      atTp += potentialAt(o, o.tp) ?? 0;
      atSl += potentialAt(o, o.sl) ?? 0;
    }
    return { atTp, atSl };
  }, [botPos, app.snapshot.pendings, app.settings.exclude_pending_potential]);

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

  const startEdit = (p: Position) => {
    setEdit(p.ticket);
    setDraft({ sl: p.sl ?? 0, tp: p.tp ?? 0 });
  };

  /* Ile pozycji naprawdę zniknie — liczba w pytaniu jest tu ważniejsza niż
     samo pytanie. „Zamknij stratne" przy 23 otwartych to inna decyzja niż
     przy dwóch, a przycisk wygląda tak samo. */
  const pytajOZbiorcze = (ktore: "all" | "profit" | "loss") => {
    const ile = botPos.filter((p) =>
      ktore === "all" ? true : ktore === "profit" ? p.profit > 0 : p.profit < 0,
    ).length;
    zapytaj({
      tytul: tt("potw.closeBulk.title", { n: ile }),
      tresc: tt("potw.closeBulk.text"),
      nieodwracalne: true,
      onTak: () => app.closeBulk(ktore),
    });
  };

  return (
    <Card
      title={tt("pos.title")}
      icon="layers"
      subtitle={obcePos.length ? `${positions.length} · ${tt("stats.bot")} ${botPos.length}` : `${positions.length}`}
      accent="var(--accent)"
      flush
      actions={
        <>
          {obcePos.length > 0 && (
            <div className="scope-switch" role="group" aria-label={tt("pos.filterAria")}>
              {(
                [
                  ["all", tt("pos.filter.all", { n: positions.length })],
                  ["bot", tt("pos.filter.bot", { n: botPos.length })],
                  ["foreign", tt("pos.filter.foreign", { n: obcePos.length })],
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
          <Checkbox
            checked={showPot}
            onChange={(v) => app.setSetting("show_potential_tpsl", v)}
            label={<span className="hint">{tt("pos.potential")}</span>}
            title={tt("pos.potential.title")}
          />
          {showPot && (
            <Checkbox
              checked={app.settings.exclude_pending_potential}
              onChange={(v) => app.setSetting("exclude_pending_potential", v)}
              label={<span className="hint">{tt("pos.noPendings")}</span>}
            />
          )}
          <Tooltip
            content={
              obcePos.length
                ? tt("pos.sum.tooltip", {
                    bot: `${totalBot >= 0 ? "+" : "−"}${money(Math.abs(totalBot), cur)}`,
                    foreign: `${totalObce >= 0 ? "+" : "−"}${money(Math.abs(totalObce), cur)}`,
                  })
                : tt("pos.sum.floating")
            }
          >
            <span className={`pos-sum num ${toneOf(total)}`}>
              {total >= 0 ? "+" : "−"}
              {money(Math.abs(total), cur)}
            </span>
          </Tooltip>
          {/* Przyciski hurtowe dzialaja WYLACZNIE na pozycjach bota — dlatego
              ich stan bierze sie z `botPos`, a nie z calej listy. */}
          {}
          <Button
            size="sm"
            variant="long"
            onClick={() => pytajOZbiorcze("profit")}
            disabled={!botPos.length}
          >
            {tt("pos.closeProfitable")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => pytajOZbiorcze("loss")} disabled={!botPos.length}>
            {tt("pos.closeLosing")}
          </Button>
          <Button size="sm" variant="danger" icon="x" onClick={() => pytajOZbiorcze("all")} disabled={!botPos.length}>
            {tt("pos.closeAll")}
          </Button>
        </>
      }
    >
      {/* Pasek prawdy o rachunku: to jest dokladnie ta roznica, ktora wczesniej
          robila z panelu klamce — equity liczylo te pozycje, lista milczala. */}
      {obcePos.length > 0 && (
        <div className="foreignbar">
          <Icon name="info" size={13} />
          <span>
            <b>
              {obcePos.length} {odmiana("pos.count", obcePos.length)} {tt("pos.foreign.outside")}
            </b>
            {" · "}
            {tt("pos.foreign.lots", { v: volObce.toFixed(2) })}
            {" · "}
            <span className={`num ${toneOf(totalObce)}`}>
              {totalObce >= 0 ? "+" : "−"}
              {money(Math.abs(totalObce), cur)}
            </span>
            {" · "}
            <span className="foreignbar__warn">{tt("pos.foreign.unmanaged")}</span>
          </span>
          <span className="foreignbar__meta">
            {app.snapshot.foreign.magics.length > 0 && `magic ${app.snapshot.foreign.magics.join(", ")}`}
            {app.snapshot.foreign.symbols.length > 0 && ` · ${app.snapshot.foreign.symbols.join(", ")}`}
            {app.snapshot.foreign.comments.length > 0 && ` · „${app.snapshot.foreign.comments.join("”, „")}”`}
          </span>
        </div>
      )}

      {/* BEZ KOSZYKA — z rozbiciem na rzeczy poprawne i realne sieroty.
          Reguła uzgodniona z NIEZAWODNOŚCIĄ (ta sama, którą stosuje eskalacja
          mailowa), żeby ekran i mail nie mówiły dwóch różnych rzeczy. */}
      {bezKoszyka.razem > 0 && (
        <div className="orphanbar">
          <span>
            {tt("pos.orphan.without")} <b className="num">{bezKoszyka.razem}</b>
            {` · ${tt("pos.orphan.manual")} `}
            <b className="num">{bezKoszyka.reczne}</b>
          </span>
          {bezKoszyka.sieroty > 0 ? (
            <span className="orphanbar__warn">{tt("pos.orphan.warn", { n: bezKoszyka.sieroty })}</span>
          ) : (
            <span className="orphanbar__ok">{tt("pos.orphan.ok")}</span>
          )}
        </div>
      )}

      {showPot && (
        <div className="potbar">
          <span>
            {tt("pos.pot.atTp")} <b>TP</b>:{" "}
            <span className={`num ${toneOf(potentials.atTp)}`}>
              {potentials.atTp >= 0 ? "+" : "−"}
              {money(Math.abs(potentials.atTp), cur)}
            </span>
          </span>
          <span>
            {tt("pos.pot.at")} <b>SL</b>:{" "}
            <span className={`num ${toneOf(potentials.atSl)}`}>
              {potentials.atSl >= 0 ? "+" : "−"}
              {money(Math.abs(potentials.atSl), cur)}
            </span>
          </span>
        </div>
      )}

      {rows.length === 0 ? (
        <Empty
          icon="layers"
          title={scope === "bot" ? tt("pos.empty.bot") : tt("pos.empty.none")}
          text={scope === "bot" ? tt("pos.empty.botText") : tt("pos.empty.noneText")}
        />
      ) : (
        <div className="tbl-wrap">
          <table className="tbl">
            <thead>
              <tr>
                {head("time", tt("pos.th.ticketTime"))}
                <th className="pos-typ">{tt("pos.th.type")}</th>
                <th>{tt("pos.th.lot")}</th>
                {head("open", tt("pos.th.open"))}
                <th>{tt("pos.th.now")}</th>
                <th>SL</th>
                <th>TP</th>
                {head("profit", tt("pos.th.profit"))}
                {showPot && <th>{tt("pos.th.pot")}</th>}
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => {
                const managed = isManaged(p.source);
                const editing = edit === p.ticket;
                const potTp = potentialAt(p, p.tp);
                const potSl = potentialAt(p, p.sl);
                return (
                  <tr
                    key={p.ticket}
                    className={`${p.frozen ? "row--frozen" : ""} ${managed ? "" : "row--foreign"}`}
                  >
                    <td style={{ textAlign: "left" }}>
                      <div className="cell-stack">
                        <span className="num cell-strong">#{p.ticket}</span>
                        <span className="cell-sub">
                          {time(p.openTime)}
                          {p.basketId !== null && ` · B${p.basketId}`}
                          {p.toucher && " · TOUCH"}
                          {/* Symbol pokazujemy tylko tam, gdzie moze byc INNY niz
                              instrument bota — czyli przy pozycjach spoza bota. */}
                          {!managed && p.symbol && ` · ${p.symbol}`}
                        </span>
                      </div>
                    </td>
                    <td className="pos-typ">
                      <Badge tone={p.direction === "BUY" ? "long" : "short"}>{p.direction}</Badge>
                      <SourceTag src={p.source} magic={p.magic} />
                      {p.frozen && (
                        <Tooltip content={tt("pos.frozen.tooltip")}>
                          <span style={{ marginLeft: 4, color: "var(--warn-text)", display: "inline-flex" }}>
                            <Icon name="lock" size={11} />
                          </span>
                        </Tooltip>
                      )}
                      {p.runner && (
                        <Tooltip content={tt("pos.runner.tooltip")}>
                          <span style={{ marginLeft: 4, color: "var(--accent-text)", display: "inline-flex" }}>
                            <Icon name="zap" size={11} />
                          </span>
                        </Tooltip>
                      )}
                    </td>
                    <td className="num">{p.volume.toFixed(2)}</td>
                    <td className="num">{num(p.openPrice, 2)}</td>
                    <td className="num">{num(app.primary.bid, 2)}</td>
                    <td className="num">
                      {editing ? (
                        <NumberInput value={draft.sl} onChange={(v) => setDraft((d) => ({ ...d, sl: v }))} step={0.1} size="sm" style={{ width: 88 }} />
                      ) : (
                        <span className={p.sl ? "lvl lvl--sl" : "lvl lvl--pusty"}>{p.sl ? num(p.sl, 2) : "—"}</span>
                      )}
                    </td>
                    <td className="num">
                      {editing ? (
                        <NumberInput value={draft.tp} onChange={(v) => setDraft((d) => ({ ...d, tp: v }))} step={0.1} size="sm" style={{ width: 88 }} />
                      ) : (
                        <span className={p.tp ? "lvl lvl--tp" : "lvl lvl--pusty"}>{p.tp ? num(p.tp, 2) : "—"}</span>
                      )}
                    </td>
                    <td className="num">
                      <span
                        className={`money money--${p.profit > 0 ? "up" : p.profit < 0 ? "down" : "flat"}`}
                      >
                        {p.profit >= 0 ? "+" : "−"}
                        {money(Math.abs(p.profit), cur)}
                      </span>
                    </td>
                    {showPot && (
                      <td className="num cell-sub">
                        <span className="down">{potSl !== null ? money(potSl, cur) : "—"}</span>
                        {" / "}
                        <span className="up">{potTp !== null ? money(potTp, cur) : "—"}</span>
                      </td>
                    )}
                    <td>
                      <div className="row-actions">
                        {/* Pozycja spoza bota nie ma koszyka i nie rusza jej ani
                            AUTO, ani AI — ale RĘCZNIE wolno na niej zrobić
                            wszystko: to rachunek użytkownika. Kłódka mówi tylko
                            o automacie, nie odbiera przycisków. */}
                        {!managed && !editing && (
                          <Tooltip content={`${tt(`src.${(p.source ?? "MANUAL").toLowerCase()}.hint`)} ${tt("pos.noAuto.tooltip")}`}>
                            <span className="row-actions__off">
                              <Icon name="lock" size={11} /> {tt("pos.noAuto")}
                            </span>
                          </Tooltip>
                        )}
                        {editing ? (
                          <>
                            <Button
                              size="sm"
                              variant="primary"
                              icon="check"
                              title={tt("pos.saveSlTp")}
                              onClick={() => {
                                app.modifyPos(p.ticket, draft.sl || null, draft.tp || null);
                                setEdit(null);
                              }}
                            />
                            <Button size="sm" variant="ghost" icon="x" title={tt("common.cancel")} onClick={() => setEdit(null)} />
                          </>
                        ) : (
                          <>
                            <Button size="sm" variant="ghost" icon="edit" title={tt("pos.editSlTp")} onClick={() => startEdit(p)} />
                            {/* Zamknięcie połowy. Przy 0,01 lota nie ma czego
                                dzielić — minimalny lot to podłoga brokera. */}
                            <Button
                              size="sm"
                              variant="ghost"
                              title={
                                p.volume >= 0.02
                                  ? tt("pos.closeHalf", { v: (Math.round((p.volume / 2) * 100) / 100).toFixed(2) })
                                  : tt("pos.tooSmall")
                              }
                              disabled={p.volume < 0.02}
                              onClick={() => {
                                const v = Math.round((p.volume / 2) * 100) / 100;
                                zapytaj({
                                  tytul: tt("potw.closePartial.title", { v: v.toFixed(2), t: p.ticket }),
                                  tresc: tt("potw.closePartial.text"),
                                  nieodwracalne: true,
                                  onTak: () => app.closePartial(p.ticket, v),
                                });
                              }}
                            >
                              ½
                            </Button>
                            <Button
                              size="sm"
                              variant="danger"
                              icon="x"
                              title={tt("pos.closeOne")}
                              onClick={() =>
                                zapytaj({
                                  tytul: tt("potw.closePos.title", { t: p.ticket }),
                                  tresc: tt("potw.closePos.text"),
                                  nieodwracalne: true,
                                  onTak: () => app.closePos(p.ticket),
                                })
                              }
                            />
                          </>
                        )}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {okno}
    </Card>
  );
}
