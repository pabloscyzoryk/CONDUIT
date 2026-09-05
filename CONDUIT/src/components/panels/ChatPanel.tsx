import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Card, Empty, Icon, MultiSelect } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import { time } from "@/lib/format";
import { SIGNAL_LABEL, SIGNAL_TONE } from "@/engine/parser";
import type { ChatMessage, SignalType } from "@/types";

const TONE_MAP = {
  long: "long",
  short: "short",
  accent: "accent",
  warn: "warn",
  info: "info",
  muted: "muted",
} as const;

/** Tożsamość ŹRÓDŁA: kanał + temat forum. Odpowiednik `SourceKey` z rdzenia. */
function kluczZrodla(m: ChatMessage): string {
  return m.topicId == null ? String(m.channelId) : `${m.channelId}:${m.topicId}`;
}

/** Wartość filtra rodzaju: trzy tryby zbiorcze albo KONKRETNY typ sygnału. */
export type ChatKind = "all" | "signals" | "pending" | SignalType;

export function ChatPanel({
  height,
  dense,
  kind: kindProp,
  onKind,
}: {
  height?: number | string;
  dense?: boolean;
  /* Filtr rodzaju da się prowadzić Z ZEWNĄTRZ — widok Sygnałów robi
     z plakietek klikalne przełączniki. Bez propsów panel trzyma stan sam,
     więc użycie na Pulpicie zostaje bez zmian. */
  kind?: ChatKind;
  onKind?: (v: ChatKind) => void;
}) {
  const app = useApp();
  const tt = useT();
  /* PUSTA LISTA ZNACZY „WSZYSTKO". To jedyny sensowny stan początkowy filtra:
     użytkownik, który jeszcze nic nie wybrał, chce widzieć strumień. */
  const [zrodla, setZrodla] = useState<string[]>([]);
  const [rodzajeWlasne, setRodzajeWlasne] = useState<ChatKind[]>([]);
  const rodzaje = kindProp !== undefined ? (kindProp === "all" ? [] : [kindProp]) : rodzajeWlasne;
  const setRodzaje = (v: ChatKind[]) => {
    if (onKind) onKind(v.length === 1 ? v[0] : "all");
    setRodzajeWlasne(v);
  };
  const listRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  const filtered = useMemo(() => {
    /* KAŻDY warunek jest KONIUNKCJĄ między osiami i ALTERNATYWĄ wewnątrz osi:
       „ZEN albo Synergy" ORAZ „wejście albo TP hit". Pusta oś nie zawęża. */
    const pasujeRodzaj = (m: ChatMessage) =>
      rodzaje.length === 0 ||
      rodzaje.some((k) => {
        
        if (k === "all") return true;
        if (k === "signals") return m.types.some((t) => t !== "INFO");
        if (k === "pending") return m.pendingAction === "await";
        /* „UNKNOWN" = parser nic z tego nie wyciągnął; wiadomość bez ani
           jednego typu jest właśnie takim przypadkiem. */
        if (k === "UNKNOWN") return m.types.length === 0 || m.types.includes("UNKNOWN");
        return m.types.includes(k);
      });
    return app.messages.filter(
      (m) => (zrodla.length === 0 || zrodla.includes(kluczZrodla(m))) && pasujeRodzaj(m),
    );
  }, [app.messages, zrodla, rodzaje]);

  
  const zrodlaWStrumieniu = useMemo(() => {
    const m = new Map<string, string>();
    for (const b of Object.values(app.bindings)) {
      if (!b.monitored) continue;
      const tematy = Object.entries(b.topics ?? {});
      if (tematy.length === 0) {
        m.set(String(b.channelId), b.format || String(b.channelId));
      } else {
        // Kanał tematyczny: każdy nasłuchiwany temat jest OSOBNYM źródłem,
        // bo osobno trafia do formatu i osobno filtruje się go w strumieniu.
        for (const [tid, nazwa] of tematy) {
          m.set(`${b.channelId}:${tid}`, String(nazwa));
        }
      }
    }
    // Ruch nadpisuje nazwy — z Telegrama przychodzą ładniejsze niż z konfiguracji.
    for (const w of app.messages) {
      const nazwa = w.channelName || String(w.channelId);
      m.set(kluczZrodla(w), w.topicName ? `${nazwa} › ${w.topicName}` : nazwa);
    }
    // DEDUPLIKACJA PO ETYKIECIE. Jeden kanał potrafi trafić tu dwiema drogami:
    // raz jako wiązanie z `format`, raz jako temat w `topics`, a do tego jako
    // ruch pod kluczem bez tematu. Bez tego filtr pokazywał „ATFX, ATFX,
    // Synergy, Synergy, ZEN" — pięć pozycji na trzy prawdziwe źródła.
    // Zostawiamy wpis o NAJDŁUŻSZYM kluczu, bo ten z tematem jest dokładniejszy.
    const wgEtykiety = new Map<string, [string, string]>();
    for (const [id, nazwa] of m) {
      const b = wgEtykiety.get(nazwa);
      if (!b || id.length > b[0].length) wgEtykiety.set(nazwa, [id, nazwa]);
    }
    return [...wgEtykiety.values()].sort((a, b) => a[1].localeCompare(b[1]));
  }, [app.messages, app.bindings]);

  useEffect(() => {
    const el = listRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [filtered.length]);

  const awaiting = app.messages.filter((m) => m.pendingAction === "await").length;

  return (
    <Card
      title={tt("chat.title")}
      icon="telegram"
      subtitle={`${filtered.length}`}
      accent="var(--info)"
      flush
      actions={
        <>
          {awaiting > 0 && (
            <Badge tone="warn" dot>
              {tt("chat.waiting", { n: awaiting })}
            </Badge>
          )}
          <MultiSelect
            values={rodzaje}
            onChange={(v) => setRodzaje(v as ChatKind[])}
            size="sm"
            labelAll={tt("chat.filter.all")}
            labelSome={tt("chat.filter.some")}
            options={[
              { value: "signals", label: tt("chat.filter.signals") },
              { value: "pending", label: tt("chat.filter.pending") },
              
              ...(Object.keys(SIGNAL_LABEL) as SignalType[]).map((typ) => ({
                value: typ,
                label: tt(SIGNAL_LABEL[typ]),
              })),
            ]}
            style={{ width: 190 }}
          />
          <MultiSelect
            values={zrodla}
            onChange={setZrodla}
            size="sm"
            labelAll={tt("chat.allChannels", { n: zrodlaWStrumieniu.length })}
            labelSome={tt("chat.someChannels")}
            options={zrodlaWStrumieniu.map(([id, name]) => ({ value: id, label: name }))}
            style={{ width: 190 }}
          />
        </>
      }
    >
      <div
        className={`chat ${dense ? "chat--dense" : ""}`}
        ref={listRef}
        style={{ height: height ?? 420 }}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
        }}
      >
        {}
        <div key={`${zrodla.join(",")}|${rodzaje.join(",")}`} className="chat__lista">
          {filtered.length === 0 ? (
            <Empty icon="telegram" title={tt("chat.empty.title")} text={tt("chat.empty.text")} />
          ) : (
            filtered.map((m) => <Message key={m.id} m={m} />)
          )}
        </div>
      </div>
    </Card>
  );
}

function Message({ m }: { m: ChatMessage }) {
  const app = useApp();
  const tt = useT();
  const uniqueTypes = Array.from(new Set(m.types)) as SignalType[];
  const isInfo = uniqueTypes.length === 1 && uniqueTypes[0] === "INFO";
  const entry = m.parsed?.find((p) => p.type === "ENTRY");
  /* Barwa awatara z identyfikatora czatu — stała dla danego kanału i nie
     wymaga listy kanałów pod ręką (wiadomość może przyjść z czatu, którego
     nie ma jeszcze w pobranej liście dialogów). */
  const hue = Math.abs(Number(m.channelId) % 360);
  const inicjal = (m.channelName || "?").trim().slice(0, 1).toUpperCase();

  return (
    <article className={`msg ${m.pendingAction === "await" ? "msg--await" : ""} ${isInfo ? "msg--info" : ""}`}>
      <div className="msg__side">
        <span className="msg__avatar" style={{ background: `hsl(${hue} 62% 52%)` }}>
          {inicjal}
        </span>
      </div>

      <div className="msg__main">
        <header className="msg__head">
          <b className="truncate">{m.channelName}</b>
          {/* Format = „czym bot to gra". Bez tego nie da się odróżnić kanału,
              który HANDLUJE, od takiego, który jest tylko nasłuchiwany —
              a to pierwsze pytanie przy „dlaczego nic nie zagrało". */}
          {m.topicName ? (
            <span className="msg__topic">{m.topicName}</span>
          ) : (
            m.format && <span className="msg__topic">{m.format}</span>
          )}
          <span className="msg__time num">{time(m.time)}</span>
          {m.basketId !== null && <Badge tone="accent">B{m.basketId}</Badge>}
          {m.edited && <span className="hint">{tt("chat.edited")}</span>}
        </header>

        <pre className="msg__text">{m.text}</pre>

        <footer className="msg__foot">
          {uniqueTypes.map((t) => (
            <Badge key={t} tone={TONE_MAP[SIGNAL_TONE[t]]}>
              {tt(SIGNAL_LABEL[t])}
            </Badge>
          ))}

          {entry && entry.entryLow !== undefined && (
            <span className="msg__meta num">
              {tt("chat.zone")} {entry.entryLow.toFixed(2)}–{entry.entryHigh?.toFixed(2)}
              {entry.sl != null && ` · SL ${entry.sl.toFixed(2)}`}
              {entry.tps?.length ? ` · ${entry.tps.length} TP` : ""}
            </span>
          )}

          {m.pendingAction === "await" && (
            <span className="msg__actions">
              <Button size="sm" variant="primary" icon="play" onClick={() => app.executeMessage(m.id)}>
                {tt("chat.execute")}
              </Button>
              <Button size="sm" variant="ghost" icon="x" onClick={() => app.dismissMessage(m.id)}>
                {tt("chat.dismiss")}
              </Button>
            </span>
          )}
          {m.pendingAction === "executed" && (
            <span className="msg__status up">
              <Icon name="check" size={12} /> {tt("chat.executed")}
            </span>
          )}
          {m.pendingAction === "deferred" && (
            <span className="msg__status flat" title={tt("chat.deferred.hint")}>
              {tt("chat.deferred")}
            </span>
          )}
          {m.pendingAction === "dismissed" && (
            <span className="msg__status flat">
              <Icon name="x" size={12} /> {tt("chat.dismissed")}
            </span>
          )}
        </footer>
      </div>
    </article>
  );
}
