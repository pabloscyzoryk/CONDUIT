import { useState } from "react";
import { Badge, Button, Card, Field, NumberInput, Segmented } from "@/components/ui";
import { NogiLotu } from "@/components/panels/LotAuto";
import { useApp } from "@/store/AppStore";
import { RichT, useT } from "@/i18n";
import { money, num, pct } from "@/lib/format";
import {
  kontraktSymbolu,
  margines,
  poziomMarginesu,
  stanMarginesu,
  type Rachunek,
} from "@/lib/ekspozycja";
import { api, type ParsePreview } from "@/store/transport";
import type { Direction } from "@/types";

/** Nowe zlecenie — odpowiednik karty „Nowe zlecenie" z bot.py. */
export function TicketPanel() {
  const app = useApp();
  const tt = useT();
  const [kind, setKind] = useState<"market" | "limit" | "stop">("market");
  const [dir, setDir] = useState<Direction>("BUY");
  // Domyślny wolumen biletu bierze się z KARTY RĘCZNEJ, nie z sumy nóg
  // automatu — to są dwie różne liczby i mylenie ich kosztowało dzień.
  const [vol, setVol] = useState(app.lotReczny);
  const [price, setPrice] = useState(0);
  const [sl, setSl] = useState(0);
  const [tp, setTp] = useState(0);

  const px = app.primary.bid;
  
  const maCene = Number.isFinite(px);
  const effPrice = kind === "market" ? px : price || px;
  
  const kontrakt = kontraktSymbolu(app.primary.symbol);
  const liczy = kontrakt !== null && Number.isFinite(effPrice);
  const risk = liczy && sl ? Math.abs(effPrice - sl) * (kontrakt as number) * vol : 0;
  const reward = liczy && tp ? Math.abs(tp - effPrice) * (kontrakt as number) * vol : 0;
  const rr = risk > 0 && reward > 0 ? reward / risk : 0;

  /* CZY KONTO STAĆ NA TEN BILET. Rachunek ryzyka mówił dotąd wyłącznie, ile
     stracę na SL — a nie, czy zlecenie w ogóle przejdzie i co zrobi z
     poziomem marginesu. Wzór i progi są te same, co przy LOT AUTO
     (`lib/ekspozycja`), więc dwa miejsca panelu nie mogą się rozjechać.
     Tu liczymy PRZYROSTOWO: do marginesu, który rachunek już trzyma. */
  const rachunek: Rachunek = {
    balance: app.stats.balance,
    equity: app.stats.equity,
    dzwignia: app.connection.account.leverage,
    cena: effPrice,
    kontrakt,
  };
  const marginesBiletu = margines(vol, rachunek);
  const mlPo =
    marginesBiletu !== null ? poziomMarginesu(app.stats.equity, app.stats.margin + marginesBiletu) : null;
  const stanBiletu = stanMarginesu(mlPo);

  return (
    <Card title={tt("ticket.title")} icon="send" accent="var(--accent)">
      <div className="ticket">
        <Segmented<Direction>
          value={dir}
          onChange={setDir}
          options={[
            { value: "BUY", label: "BUY / LONG", bg: "var(--long-soft)", fg: "var(--long-text)" },
            { value: "SELL", label: "SELL / SHORT", bg: "var(--short-soft)", fg: "var(--short-text)" },
          ]}
          style={{ width: "100%" }}
        />

        <Segmented<"market" | "limit" | "stop">
          value={kind}
          onChange={setKind}
          size="sm"
          options={[
            { value: "market", label: "Market" },
            { value: "limit", label: "Limit" },
            { value: "stop", label: "Stop" },
          ]}
          style={{ width: "100%" }}
        />

        <div className="ticket__grid">
          <Field label={tt("ticket.volume")}>
            <NumberInput value={vol} onChange={setVol} step={0.01} min={0.01} />
          </Field>
          <Field label={kind === "market" ? tt("ticket.marketPrice") : tt("ticket.activationPrice")}>
            <NumberInput
              value={kind === "market" ? (maCene ? px : 0) : price}
              onChange={setPrice}
              step={0.1}
              unit={kind === "market" ? "mkt" : undefined}
            />
          </Field>
          <Field label={tt("ticket.sl")}>
            <NumberInput value={sl} onChange={setSl} step={0.1} />
          </Field>
          <Field label={tt("ticket.tp")}>
            <NumberInput value={tp} onChange={setTp} step={0.1} />
          </Field>
        </div>

        <div className="ticket__calc">
          <span>
            {tt("ticket.risk")} <b className="num down">{liczy ? `$${risk.toFixed(2)}` : "—"}</b>
          </span>
          <span>
            {tt("ticket.reward")} <b className="num up">{liczy ? `$${reward.toFixed(2)}` : "—"}</b>
          </span>
          <span>
            R:R <b className="num">{rr ? `1 : ${rr.toFixed(2)}` : "—"}</b>
          </span>
          {/* MARGINES BILETU — druga połowa pytania „czy mogę to wysłać".
              Sama strata na SL nie mówi, czy zlecenie zmieści się na
              rachunku ani ile zostanie z poziomu marginesu. */}
          <span>
            {tt("ticket.margin")}{" "}
            <b className="num" data-stan={stanBiletu}>
              {marginesBiletu !== null ? money(marginesBiletu, app.settings.display_currency) : "—"}
            </b>
            {mlPo !== null && (
              <span className="hint"> {tt("ticket.marginLevel", { v: pct(mlPo, 0) })}</span>
            )}
          </span>
        </div>

        <Button
          variant={dir === "BUY" ? "long" : "short"}
          size="lg"
          block
          icon="send"
          disabled={!maCene}
          title={maCene ? undefined : `Brak kwotowania dla ${app.primary.symbol}`}
          onClick={() => app.openManualOrder({ kind, direction: dir, volume: vol, price, sl, tp })}
        >
          {maCene
            ? `${dir} ${vol.toFixed(2)} lot ${
                kind !== "market" ? `@ ${num(price || px, 2)}` : tt("ticket.atMarket")
              }`
            : `Brak kwotowania — ${app.primary.symbol}`}
        </Button>

        {}
        <div className="ticket__manual">
          <header className="ticket__manual-head">{tt("ticket.manual.head")}</header>
          <p className="hint" style={{ margin: 0 }}>
            {tt("lot.manual.hint")}
          </p>
          <ManualnyLot onLot={setVol} />
          <PodstawaLota />
        </div>
      </div>
    </Card>
  );
}

/** Tryb i wielkość lota RĘCZNEGO biletu — przeniesione z karty „Lot size". */
function ManualnyLot({ onLot }: { onLot: (v: number) => void }) {
  const app = useApp();
  const tt = useT();
  const [fixed, setFixed] = useState(app.lot.fixed);
  const [percent, setPercent] = useState(app.lot.percent);

  return (
    <>
      <Segmented<"fixed" | "percent">
        value={app.lot.mode}
        onChange={(m) => app.setLot({ ...app.lot, mode: m })}
        size="sm"
        options={[
          { value: "fixed", label: tt("lot.fixed") },
          { value: "percent", label: tt("lot.percent") },
        ]}
      />

      {app.lot.mode === "fixed" ? (
        <Field label={tt("lot.size")} hint={tt("lot.size.hint")}>
          <NumberInput value={fixed} onChange={setFixed} step={0.01} min={0.01} unit="lot" />
        </Field>
      ) : (
        <Field label={tt("lot.pct")} hint={tt("lot.pct.hint")}>
          <NumberInput value={percent} onChange={setPercent} step={0.05} min={0.01} unit="%" />
        </Field>
      )}

      <div className="lot__result">
        <span className="eyebrow">{tt("lot.currentManual")}</span>
        <b className="num">{app.lotReczny.toFixed(2)}</b>
        <span className="hint">{tt("lot.manual.note")}</span>
      </div>

      <Button
        variant="primary"
        block
        icon="check"
        onClick={() => {
          app.setLot({ mode: app.lot.mode, fixed, percent });
          // Bilet ma od razu pokazać zapisany wolumen — inaczej trzeba
          // pamiętać o ręcznym przepisaniu go do pola wyżej.
          onLot(app.lot.mode === "fixed" ? fixed : app.lotReczny);
        }}
      >
        {tt("lot.saveBtn")}
      </Button>
    </>
  );
}

/** PODSTAWA WIELKOŚCI POZYCJI — saldo, kredyt bonusowy i to, co z nich zostaje.
 *
 *  Po co osobny blok, skoro saldo widać w nagłówku: bo na koncie z bonusem
 *  100 % saldo i podstawa lota to DWIE RÓŻNE LICZBY różniące się dwukrotnie,
 *  a bez ich rozdzielenia nie ma jak sprawdzić, od której bot liczy wolumen.
 *  Objaw błędu — lot dwa razy większy, niż zamierzał właściciel — widać
 *  dopiero po fakcie, w historii transakcji.
 *
 *  Blok znika, gdy odliczanie jest wyłączone i terminal nie raportuje żadnego
 *  kredytu: na zwykłym koncie nie ma o czym informować. */
function PodstawaLota() {
  const app = useApp();
  const tt = useT();
  const st = app.stats;
  const czynne = st.creditSource !== "off";

  // Konto bez bonusu i bez przełącznika — nie zaśmiecamy karty.
  if (!czynne && !(st.credit > 0)) return null;

  const zrodlo =
    st.creditSource === "reczny"
      ? tt("lot.base.manual")
      : st.creditSource === "terminal"
        ? tt("lot.base.terminal")
        : tt("lot.base.notDeducted");

  return (
    <div className="lot__base">
      <div className="lot__base-row">
        <span>{tt("lot.base.balance")}</span>
        <b className="num">{num(st.balance, 2)} $</b>
        <span />
      </div>
      <div className="lot__base-row">
        <span>{tt("lot.base.credit")}</span>
        <b className="num">{num(czynne ? st.creditApplied : st.credit, 2)} $</b>
        <span className="hint">({zrodlo})</span>
      </div>
      <div className="lot__base-row lot__base-row--sum">
        <span>{tt("lot.base.base")}</span>
        <b className="num">{num(czynne ? st.lotBase : st.balance, 2)} $</b>
        <span />
      </div>

      {st.creditMismatch && (
        <div className="lot__base-warn">
          <Badge tone="warn">{tt("lot.mismatch.badge")}</Badge>
          <span>{tt("lot.mismatch.text", { a: num(st.creditApplied, 2), b: num(st.credit, 2) })}</span>
        </div>
      )}

      {!czynne && st.credit > 0 && (
        <div className="lot__base-warn">
          <Badge tone="warn">{tt("lot.bonus.badge")}</Badge>
          <span>{tt("lot.bonus.text", { v: num(st.credit, 2) })}</span>
        </div>
      )}
    </div>
  );
}


export function LotPanel() {
  const tt = useT();
  return (
    <Card title={tt("lotauto.card.title")} icon="layers" accent="var(--info)">
      <NogiLotu />
    </Card>
  );
}

/** Rozbiór wiadomości: co parser SILNIKA z niej wyjął, pole po polu.
 *
 *  To jest odpowiedź na pytanie „dlaczego bot nie zareagował na ten sygnał".
 *  Bez tego widoku jedyną drogą było wysłanie wiadomości do silnika — czyli
 *  otwarcie prawdziwej pozycji po to, żeby sprawdzić format. */
function RozbiorSygnalu({ w }: { w: ParsePreview }) {
  const tt = useT();
  const wiersze: [string, string][] = [];
  for (const p of w.parsed) {
    if (p.type === "INFO") continue;
    if (p.direction) wiersze.push([tt("parse.direction"), p.direction]);
    if (p.isLimit !== undefined)
      wiersze.push([
        tt("parse.entryKind"),
        // STOP i LIMIT leżą po PRZECIWNYCH stronach ceny — mylenie ich to
        // wejście w drugą stronę rynku, więc muszą być rozróżnialne.
        p.isStop ? tt("parse.stop") : p.isLimit ? tt("parse.limit") : tt("parse.market"),
      ]);
    if (p.entryLow !== undefined)
      wiersze.push([
        tt("parse.zone"),
        p.entryLow === p.entryHigh
          ? tt("parse.point", { v: p.entryLow.toFixed(2) })
          : `${p.entryLow.toFixed(2)} – ${p.entryHigh?.toFixed(2)}`,
      ]);
    if (p.sl != null) wiersze.push([tt("parse.sl"), p.sl.toFixed(2)]);
    if (p.tps?.length)
      wiersze.push([tt("parse.targets", { n: p.tps.length }), p.tps.map((t) => t.toFixed(2)).join(" · ")]);
    if (p.tpIndex !== undefined) wiersze.push([tt("parse.tpIndex"), String(p.tpIndex)]);
    if (p.level !== undefined) wiersze.push([tt("parse.level"), p.level.toFixed(2)]);
  }

  return (
    <div className="col" style={{ gap: 8 }}>
      <div className="row row--tight">
        {w.recognized ? (
          w.types
            .filter((t) => t !== "INFO")
            .map((t, i) => (
              <Badge key={`${t}-${i}`} tone="accent">
                {t}
              </Badge>
            ))
        ) : (
          <Badge tone="warn" dot>
            {tt("parse.unrecognized")}
          </Badge>
        )}
        <span className="hint">{tt("parse.stats", { c: w.input.chars, l: w.input.lines })}</span>
      </div>

      {wiersze.length > 0 && (
        <div className="tbl-wrap">
          <table className="tbl">
            <tbody>
              {wiersze.map(([k, v], i) => (
                <tr key={`${k}-${i}`}>
                  <td style={{ color: "var(--text-dim)", width: "45%" }}>{k}</td>
                  <td className="num">{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {!w.recognized && <p className="hint">{tt("parse.help")}</p>}
    </div>
  );
}

/** Symulacja wiadomości z Telegrama (jak w bot.py). */
export function SimulateMessagePanel() {
  const app = useApp();
  const tt = useT();
  const [text, setText] = useState("");
  const [rozbior, setRozbior] = useState<ParsePreview | null>(null);
  const [bladRozbioru, setBladRozbioru] = useState("");
  const [liczy, setLiczy] = useState(false);
  const px = app.primary.bid;

  /* ---------------- ADRES WIADOMOŚCI (F5) ----------------
     Zero znaczy „nie podano" we WSZYSTKICH czterech polach i to nie jest
     skrót myślowy: Telegram numeruje wiadomości od 1 w górę, więc `0` nigdy
     nie jest prawdziwym numerem ani prawdziwym tematem. Dlatego `zeroLabel`
     pisze to wprost, zamiast pokazywać cyfrę, którą dałoby się wziąć za
     ustawioną wartość.

     `msgId` jest tu pierwsze co do wagi, a nie ostatnie: edycja i odpowiedź
     ODNOSZĄ SIĘ do numeru, więc bez własnego numeru pozostałe dwa pola nie
     mają czego wskazać. */
  const [pokazAdres, setPokazAdres] = useState(false);
  const [msgId, setMsgId] = useState(0);
  const [editOf, setEditOf] = useState(0);
  const [replyTo, setReplyTo] = useState(0);
  const [topicId, setTopicId] = useState(0);
  const adresUstawiony = msgId !== 0 || editOf !== 0 || replyTo !== 0 || topicId !== 0;

  const samples = [
    {
      label: "BUY LIMITS",
      text: `🟢 BUY LIMITS GOLD @ ${(px - 3).toFixed(2)}/${(px - 9).toFixed(2)} AREA\n🎯 TP1 ${(px + 1).toFixed(2)}\n🎯 TP2 ${(px + 6).toFixed(2)}\n🎯 TP3 ${(px + 12).toFixed(2)}\n⛔️ SL ${(px - 22).toFixed(2)}\nHIGH RISK TRADE`,
    },
    {
      label: "SELL LIMITS",
      text: `🔴 SELL LIMITS GOLD @ ${(px + 4).toFixed(2)}/${(px + 10).toFixed(2)} AREA\n🎯 TP1 ${(px - 1).toFixed(2)}\n🎯 TP2 ${(px - 7).toFixed(2)}\n⛔️ SL ${(px + 24).toFixed(2)}`,
    },
    { label: "TP1 HIT", text: "✅ TP1 HIT +32 PIPS 🎉\nSL IS SET TO BE — RISK FREE NOW 🔒" },
    { label: "RISK FREE", text: `RISK FREE AT ${px.toFixed(2)} 🔒` },
    { label: "OUT AT ENTRY", text: "OUT AT ENTRY ON THE REST — TRADE NOT MOVING" },
    { label: "CLOSE ALL", text: "CLOSE ALL POSITIONS NOW ⚠️" },
  ];

  return (
    <Card title={tt("sim.title")} icon="telegram" subtitle={tt("sim.subtitle")} accent="var(--info)">
      <div className="col">
        <div className="row row--tight">
          {samples.map((s) => (
            <Button
              key={s.label}
              size="sm"
              variant="outline"
              onClick={() => {
                setText(s.text);
                setRozbior(null);
                setBladRozbioru("");
              }}
            >
              {s.label}
            </Button>
          ))}
        </div>

        <textarea
          className="textarea"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            // rozbiór dotyczy POPRZEDNIEGO tekstu — zostawiony pod spodem
            // kłamałby o treści, która jest teraz w polu
            setRozbior(null);
            setBladRozbioru("");
          }}
          placeholder={tt("sim.placeholder")}
        />

        <div className="row row--tight">
          <Button
            size="sm"
            variant="outline"
            icon={pokazAdres ? "chevron-down" : "chevron-right"}
            onClick={() => setPokazAdres((v) => !v)}
          >
            {tt("sim.addr.toggle")}
          </Button>
          {!pokazAdres && adresUstawiony && <Badge tone="info">{tt("sim.addr.active")}</Badge>}
        </div>

        {pokazAdres && (
          <>
            <div className="ticket__grid">
              <Field label={tt("sim.msgId")} hint={tt("sim.msgId.hint")}>
                <NumberInput
                  value={msgId}
                  onChange={setMsgId}
                  step={1}
                  min={0}
                  zeroLabel={tt("sim.auto")}
                />
              </Field>
              <Field label={tt("sim.editOf")} hint={tt("sim.editOf.hint")}>
                <NumberInput
                  value={editOf}
                  onChange={setEditOf}
                  step={1}
                  min={0}
                  zeroLabel={tt("sim.none")}
                />
              </Field>
              <Field label={tt("sim.replyTo")} hint={tt("sim.replyTo.hint")}>
                <NumberInput
                  value={replyTo}
                  onChange={setReplyTo}
                  step={1}
                  min={0}
                  zeroLabel={tt("sim.none")}
                />
              </Field>
              <Field label={tt("sim.topicId")} hint={tt("sim.topicId.hint")}>
                <NumberInput
                  value={topicId}
                  onChange={setTopicId}
                  step={1}
                  min={0}
                  zeroLabel={tt("sim.none")}
                />
              </Field>
            </div>
            <span className="hint">
              <RichT k="sim.addr.hint" />
            </span>
          </>
        )}

        {bladRozbioru && <div className="hint" style={{ color: "var(--short-text)" }}>{bladRozbioru}</div>}
        {rozbior && <RozbiorSygnalu w={rozbior} />}

        <div className="row">
          <span className="hint" style={{ flex: 1, minWidth: 180 }}>
            <RichT k="sim.note" />
          </span>
          <Button
            variant="outline"
            icon="search"
            disabled={!text.trim() || liczy}
            onClick={async () => {
              setLiczy(true);
              setBladRozbioru("");
              try {
                setRozbior(await api.parse(text.trim()));
              } catch (e) {
                setRozbior(null);
                setBladRozbioru(tt("sim.parseFailed", { e: (e as Error).message }));
              } finally {
                setLiczy(false);
              }
            }}
          >
            {liczy ? tt("sim.parsing") : tt("sim.parse")}
          </Button>
          <Button
            variant="primary"
            icon="send"
            disabled={!text.trim()}
            onClick={() => {
              // Pole zerowe NIE JEDZIE — kontrakt zera stoi na tym, że brak
              // pola i pole równe zeru to dla serwera dwie różne rzeczy.
              app.simulateMessage(text.trim(), undefined, {
                msgId: msgId || undefined,
                editOf: editOf || undefined,
                replyTo: replyTo || undefined,
                topicId: topicId || undefined,
              });
              setText("");
              setRozbior(null);
              // ADRES ZOSTAJE. Wstrzyknięcie edycji to z natury DRUGI krok po
              // wstrzyknięciu wejścia — kasowanie numeru po każdej wysyłce
              // kazałoby przepisywać go z pamięci przy każdej próbie.
            }}
          >
            {tt("sim.send")}
          </Button>
        </div>
      </div>
    </Card>
  );
}
