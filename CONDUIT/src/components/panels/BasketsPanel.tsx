import { tSilnik } from "@/i18n/silnik";
import { useState } from "react";
import { Badge, Button, Card, Empty, Icon, NumberInput, TextInput } from "@/components/ui";
import { usePotwierdzenie } from "@/components/ui/Potwierdzenie";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import { ago, num, time } from "@/lib/format";
import type { Basket } from "@/types";

export function BasketsPanel({ limit }: { limit?: number }) {
  const app = useApp();
  const tt = useT();
  const all = app.snapshot.baskets.filter((b) => b.active || b.tickets.length > 0);
  const baskets = [...all].reverse().slice(0, limit);

  return (
    <Card
      title={tt("bask.title")}
      icon="target"
      subtitle={`${all.length}`}
      accent="var(--long)"
      flush={baskets.length > 0}
    >
      {baskets.length === 0 ? (
        <Empty icon="target" title={tt("bask.empty.title")} text={tt("bask.empty.text")} />
      ) : (
        <div className="baskets">
          {baskets.map((b) => (
            <BasketCard key={b.id} b={b} />
          ))}
        </div>
      )}
    </Card>
  );
}

function BasketCard({ b }: { b: Basket }) {
  const app = useApp();
  const tt = useT();
  const { zapytaj, okno } = usePotwierdzenie();
  const [open, setOpen] = useState(false);
  const [sl, setSl] = useState(b.sl ?? 0);
  const [lo, setLo] = useState(b.zoneLow);
  const [hi, setHi] = useState(b.zoneHigh);
  const [tps, setTps] = useState(b.tps.map((t) => t.toFixed(2)).join(", "));

  const px = app.primary.bid;
  const buy = b.direction === "BUY";
  const inZone = px >= b.zoneLow && px <= b.zoneHigh;
  const progress = b.tps.length ? Math.min(100, (b.tpStage / b.tps.length) * 100) : 0;

  const posProfit = app.snapshot.positions.filter((p) => p.basketId === b.id).reduce((a, p) => a + p.profit, 0);

  return (
    <article className={`basket ${b.direction === "BUY" ? "basket--buy" : "basket--sell"}`}>
      <header className="basket__head">
        <span className="basket__id">B{b.id}</span>
        <Badge tone={buy ? "long" : "short"}>
          {b.direction}
          {b.isLimit ? " LIMIT" : ""}
        </Badge>
        {b.riskFree && <span title={tt("bask.riskFreeState.hint")}><Badge tone="info">{tt("bask.riskFreeState")}</Badge></span>}
        {!b.active && <Badge tone="muted">{tt("bask.closed")}</Badge>}
        <span className="basket__src truncate">{b.source}</span>
        <span className="spacer" />
        <span className={`num basket__pnl ${posProfit >= 0 ? "up" : "down"}`}>
          {posProfit >= 0 ? "+" : "−"}${Math.abs(posProfit).toFixed(2)}
        </span>
        <Button size="sm" variant="ghost" icon={open ? "chevron-down" : "chevron-right"} onClick={() => setOpen((o) => !o)} title={tt("bask.details")} />
      </header>

      <div className="basket__zone">
        <span className="basket__zonebar">
          <span
            className="basket__zonefill"
            style={{ width: `${Math.max(2, Math.min(100, ((px - b.zoneLow) / Math.max(0.01, b.zoneHigh - b.zoneLow)) * 100))}%` }}
          />
        </span>
        <span className="basket__zonelabels">
          <b className="num">{num(b.zoneLow, 2)}</b>
          <span className={`num ${inZone ? "up" : "flat"}`}>
            {inZone ? tt("bask.inZone") : tt("bask.price", { v: num(px, 2) })}
          </span>
          <b className="num">{num(b.zoneHigh, 2)}</b>
        </span>
      </div>

      <div className="basket__meta">
        <span title={tt("bask.targetsHit")}>
          <Icon name="flag" size={11} /> TP {b.tpStage}/{b.tps.length}
        </span>
        <span title={tt("bask.openPositions")}>
          <Icon name="layers" size={11} /> {b.tickets.length}
        </span>
        <span title={tt("bask.pendingOrders")}>
          <Icon name="clock" size={11} /> {b.pendingTickets.length}
        </span>
        <span title={tt("bask.sl.title")} className="down">
          SL {b.sl ? num(b.sl, 2) : "—"}
        </span>
        <span className="spacer" />
        <span className="cell-sub">{ago(b.createdAt)}</span>
      </div>

      <div className="basket__tps">
        <span className="meter" style={{ flex: 1 }}>
          <span className="meter__fill" style={{ width: `${progress}%`, background: buy ? "var(--long)" : "var(--short)" }} />
        </span>
        {b.tps.map((t, i) => (
          <span key={i} className={`basket__tp ${i < b.tpStage ? "basket__tp--hit" : ""}`} title={`TP${i + 1}`}>
            {num(t, 2)}
          </span>
        ))}
      </div>

      {open && (
        <div className="basket__body">
          <div className="basket__edit">
            <label className="hint">SL</label>
            <NumberInput value={sl} onChange={setSl} step={0.1} size="sm" style={{ width: 92 }} />
            <label className="hint">{tt("bask.zone")}</label>
            <NumberInput value={lo} onChange={setLo} step={0.1} size="sm" style={{ width: 92 }} />
            <NumberInput value={hi} onChange={setHi} step={0.1} size="sm" style={{ width: 92 }} />
            <label className="hint">TP</label>
            <TextInput value={tps} onChange={setTps} size="sm" style={{ width: 168 }} />
            <Button
              size="sm"
              variant="primary"
              icon="check"
              onClick={() =>
                app.updateBasket(b.id, {
                  sl: sl || null,
                  zoneLow: lo,
                  zoneHigh: hi,
                  tps: tps
                    .split(",")
                    .map((x) => Number(x.trim()))
                    .filter((x) => Number.isFinite(x) && x > 0),
                })
              }
            >
              {tt("bask.saveBtn")}
            </Button>
            {}
            <Button
              size="sm"
              variant="danger"
              icon="x"
              onClick={() =>
                zapytaj({
                  tytul: tt("potw.closeBasket.title", { id: b.id }),
                  tresc: tt("potw.closeBasket.text"),
                  nieodwracalne: true,
                  onTak: () => app.closeBasketById(b.id),
                })
              }
            >
              {tt("bask.closeBtn")}
            </Button>
          </div>

          <ul className="basket__log">
            {[...b.events]
              .reverse()
              .slice(0, 8)
              .map((e, i) => (
                <li key={i} className={`basket__ev basket__ev--${e.kind}`}>
                  <span className="num">{time(e.t)}</span>
                  {tSilnik(e.text)}
                </li>
              ))}
          </ul>
        </div>
      )}
      {okno}
    </article>
  );
}
