import { useMemo, useState } from "react";
import { Badge, Button, Card, Empty, Icon, NumberInput, Tooltip } from "@/components/ui";
import { usePotwierdzenie } from "@/components/ui/Potwierdzenie";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import { money, num, brokerTime, time } from "@/lib/format";
import { potentialAt } from "@/engine/bot";
import { isManaged } from "@/types";
import { odmiana, SourceTag } from "./PositionsPanel";

export function PendingsPanel() {
  const app = useApp();
  const tt = useT();
  const { zapytaj, okno } = usePotwierdzenie();
  const { pendings } = app.snapshot;
  const cur = app.settings.display_currency;
  const showPot = app.settings.show_potential_tpsl;
  const [edit, setEdit] = useState<number | null>(null);
  const [draft, setDraft] = useState({ price: 0, sl: 0, tp: 0 });

  /* Zlecenia bota i cala reszta rachunku. „Usun wszystkie" dotyczy CALEGO
     rachunku — takze zlecen spoza bota. Wczesniej przycisk liczyl wylacznie
     nasze i przy 48 obcych limitach pokazywal „(0)", czyli wygladal na zepsuty,
     a w istocie po cichu pomijal wszystko, co widac na ekranie. */
  const botPend = useMemo(() => pendings.filter((o) => isManaged(o.source)), [pendings]);
  const obcePend = useMemo(() => pendings.filter((o) => !isManaged(o.source)), [pendings]);

  return (
    <Card
      title={tt("pend.title")}
      icon="clock"
      subtitle={obcePend.length ? `${pendings.length} · ${tt("stats.bot")} ${botPend.length}` : `${pendings.length}`}
      accent="var(--warn)"
      flush
      actions={
        <Button
          size="sm"
          variant="danger"
          icon="trash"
          /* KASOWANIE CAŁEJ SIATKI PYTA (TODO K10) — i mówi, ile zleceń
             zniknie, wliczając te SPOZA bota, których przycisk też dotyczy. */
          onClick={() =>
            zapytaj({
              tytul: tt("potw.delAll.title", { n: pendings.length }),
              tresc: tt("potw.delAll.text"),
              nieodwracalne: true,
              onTak: app.delAllPendings,
            })
          }
          disabled={!pendings.length}
        >
          {tt("pend.deleteAll")}
          {obcePend.length ? tt("pend.deleteAll.suffix", { n: pendings.length, m: obcePend.length }) : ""}
        </Button>
      }
    >
      {obcePend.length > 0 && (
        <div className="foreignbar">
          <Icon name="info" size={13} />
          <span>
            <b>
              {obcePend.length} {odmiana("pend.count", obcePend.length)} {tt("pos.foreign.outside")}
            </b>
            {" · "}
            <span className="foreignbar__warn">{tt("pos.foreign.unmanaged")}</span>
          </span>
          <span className="foreignbar__meta">
            {app.snapshot.foreign.magics.length > 0 && `magic ${app.snapshot.foreign.magics.join(", ")}`}
          </span>
        </div>
      )}
      {pendings.length === 0 ? (
        <Empty icon="clock" title={tt("pend.empty.title")} text={tt("pend.empty.text")} />
      ) : (
        <div className="tbl-wrap">
          <table className="tbl">
            <thead>
              <tr>
                <th>{tt("pos.th.ticketTime")}</th>
                <th style={{ textAlign: "left" }}>{tt("pos.th.type")}</th>
                <th>{tt("pos.th.lot")}</th>
                <th>{tt("pend.th.price")}</th>
                <th>{tt("pend.th.distance")}</th>
                <th>SL</th>
                <th>TP</th>
                {showPot && <th>{tt("pos.th.pot")}</th>}
                <th />
              </tr>
            </thead>
            <tbody>
              {pendings.map((o) => {
                const managed = isManaged(o.source);
                const editing = edit === o.ticket;
                const dist = o.price - app.primary.bid;
                const buy = o.kind.startsWith("BUY");
                return (
                  <tr
                    key={o.ticket}
                    className={`${o.frozen ? "row--frozen" : ""} ${managed ? "" : "row--foreign"}`}
                  >
                    <td style={{ textAlign: "left" }}>
                      <div className="cell-stack">
                        <span className="num cell-strong">#{o.ticket}</span>
                        <span className="cell-sub">
                          {app.live ? brokerTime(o.placedTime) : time(o.placedTime)}<span className="hint"> · {tt(app.live ? "clock.server" : "clock.local")}</span>
                          {o.basketId !== null && ` · B${o.basketId}`}
                          {o.comment.includes("TOUCH") && " · TOUCH"}
                          {!managed && o.symbol && ` · ${o.symbol}`}
                        </span>
                      </div>
                    </td>
                    <td style={{ textAlign: "left" }}>
                      <Badge tone={buy ? "long" : "short"}>{o.kind.replace("_", " ")}</Badge>
                      <SourceTag src={o.source} magic={o.magic} />
                      {o.frozen && (
                        <Tooltip content={tt("pend.frozen.tooltip")}>
                          <span style={{ marginLeft: 4, color: "var(--warn-text)", display: "inline-flex" }}>
                            <Icon name="lock" size={11} />
                          </span>
                        </Tooltip>
                      )}
                    </td>
                    <td className="num">{o.volume.toFixed(2)}</td>
                    <td className="num">
                      {editing ? (
                        <NumberInput value={draft.price} onChange={(v) => setDraft((d) => ({ ...d, price: v }))} step={0.1} size="sm" style={{ width: 88 }} />
                      ) : (
                        num(o.price, 2)
                      )}
                    </td>
                    <td className={`num cell-sub ${Math.abs(dist) < 1 ? "up" : ""}`}>
                      {dist >= 0 ? "+" : "−"}
                      {num(Math.abs(dist), 2)}
                    </td>
                    <td className="num">
                      {editing ? (
                        <NumberInput value={draft.sl} onChange={(v) => setDraft((d) => ({ ...d, sl: v }))} step={0.1} size="sm" style={{ width: 88 }} />
                      ) : (
                        <span className={o.sl ? "down" : "flat"}>{o.sl ? num(o.sl, 2) : "—"}</span>
                      )}
                    </td>
                    <td className="num">
                      {editing ? (
                        <NumberInput value={draft.tp} onChange={(v) => setDraft((d) => ({ ...d, tp: v }))} step={0.1} size="sm" style={{ width: 88 }} />
                      ) : (
                        <span className={o.tp ? "up" : "flat"}>{o.tp ? num(o.tp, 2) : "—"}</span>
                      )}
                    </td>
                    {showPot && (
                      <td className="num cell-sub">
                        <span className="down">{potentialAt(o, o.sl) !== null ? money(potentialAt(o, o.sl)!, cur) : "—"}</span>
                        {" / "}
                        <span className="up">{potentialAt(o, o.tp) !== null ? money(potentialAt(o, o.tp)!, cur) : "—"}</span>
                      </td>
                    )}
                    <td>
                      <div className="row-actions">
                        {/* Automat cudzego zlecenia nie prowadzi — ale ręczna
                            edycja i usunięcie z panelu działają normalnie. */}
                        {!managed && !editing && (
                          <Tooltip content={`${tt(`src.${(o.source ?? "MANUAL").toLowerCase()}.hint`)} ${tt("pos.noAuto.tooltip")}`}>
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
                              title={tt("common.save")}
                              onClick={() => {
                                app.modifyPending(o.ticket, draft.price, draft.sl || null, draft.tp || null);
                                setEdit(null);
                              }}
                            />
                            <Button size="sm" variant="ghost" icon="x" title={tt("common.cancel")} onClick={() => setEdit(null)} />
                          </>
                        ) : (
                          <>
                            <Button
                              size="sm"
                              variant="ghost"
                              icon="edit"
                              title={tt("common.edit")}
                              onClick={() => {
                                setEdit(o.ticket);
                                setDraft({ price: o.price, sl: o.sl ?? 0, tp: o.tp ?? 0 });
                              }}
                            />
                            <Button
                              size="sm"
                              variant="danger"
                              icon="trash"
                              title={tt("common.delete")}
                              onClick={() =>
                                zapytaj({
                                  tytul: tt("potw.delPending.title", { t: o.ticket }),
                                  tresc: tt("potw.delPending.text"),
                                  nieodwracalne: true,
                                  onTak: () => app.delPending(o.ticket),
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
