import { useMemo, useState } from "react";
import { Badge, Card, Icon } from "@/components/ui";
import { ChatPanel, type ChatKind } from "@/components/panels/ChatPanel";
import { SimulateMessagePanel } from "@/components/panels/TicketPanel";
import { BasketsPanel } from "@/components/panels/BasketsPanel";
import { useApp } from "@/store/AppStore";
import { useT, RichT } from "@/i18n";
import { SIGNAL_LABEL } from "@/engine/parser";
import type { SignalType } from "@/types";
import "./views.css";

export function SignalsView() {
  const app = useApp();
  const t = useT();

  /* Plakietka jest PRZEŁĄCZNIKIEM filtra listy poniżej, nie ozdobą. */
  const [filtr, setFiltr] = useState<ChatKind>("all");

  const counts = useMemo(() => {
    const c: Partial<Record<SignalType, number>> = {};
    for (const m of app.messages) {
      /* Wiadomość BEZ ani jednego typu to „nierozpoznany rodzaj" — dokładnie
         to, co opisuje plakietka UNKNOWN. Wcześniej takie wiadomości nie
         trafiały nigdzie i wszystkie liczniki stały na zerze, choć lista
         była pełna. */
      for (const typ of m.types.length ? new Set(m.types) : new Set<SignalType>(["UNKNOWN"])) {
        c[typ] = (c[typ] ?? 0) + 1;
      }
    }
    return c;
  }, [app.messages]);

  /* Plakietki opisuja DOKLADNIE te liste, ktora widac ponizej. Wczesniej
     bralismy je z `app.stats`, czyli z licznikow SILNIKA — a te zeruja sie przy
     restarcie procesu, podczas gdy lista wraca z backupu. Panel pokazywal
     „0 wiadomosci" nad lista, na ktorej cos bylo. */
  const widoczneSygnaly = useMemo(
    () => app.messages.filter((m) => m.types.some((typ) => typ !== "INFO")).length,
    [app.messages],
  );

  const awaiting = app.messages.filter((m) => m.pendingAction === "await").length;
  const executed = app.messages.filter((m) => m.pendingAction === "executed").length;

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("signals.title")}</h1>
          <p>{t("signals.intro")}</p>
        </div>
        <div className="row row--tight">
          {app.mode === "MANUAL" && awaiting > 0 && (
            <Badge tone="warn" dot>
              {t("signals.awaiting", { n: awaiting })}
            </Badge>
          )}
          {executed > 0 && <Badge tone="long">{t("signals.executed", { n: executed })}</Badge>}
          <Badge tone="muted">{t("signals.messages", { n: app.messages.length })}</Badge>
          <Badge tone="accent">{t("signals.count", { n: widoczneSygnaly })}</Badge>
        </div>
      </div>

      {app.mode === "MANUAL" && (
        <div className="modenote" style={{ background: "var(--info-soft)", borderColor: "transparent" }}>
          <span className="modenote__icon" style={{ background: "var(--bg-surface)", color: "var(--info-text)" }}>
            <Icon name="hand" size={16} />
          </span>
          <div>
            <b style={{ color: "var(--info-text)" }}>{t("signals.manual.title")}</b>
            <p>
              <RichT k="signals.manual.text" />
            </p>
          </div>
        </div>
      )}

      <Card title={t("signals.types.title")} icon="signal" accent="var(--info)" tight>
        <div className="row row--tight">
          <button
            type="button"
            className="chip"
            onClick={() => setFiltr("all")}
            style={
              filtr === "all"
                ? { borderColor: "var(--accent)", color: "var(--text)", cursor: "pointer" }
                : { opacity: 0.7, cursor: "pointer" }
            }
          >
            {t("chat.filter.all")}
            <b className="num" style={{ color: "var(--text)" }}>
              {app.messages.length}
            </b>
          </button>
          {(Object.keys(SIGNAL_LABEL) as SignalType[]).map((typ) => (
            <button
              key={typ}
              type="button"
              /* Klik = filtr listy poniżej; drugi klik na tej samej plakietce
                 zdejmuje filtr. Liczniki bez akcji były mylące: pokazywały,
                 że coś JEST, i nie dawały tego zobaczyć. */
              onClick={() => setFiltr((p) => (p === typ ? "all" : typ))}
              className="chip"
              style={
                filtr === typ
                  ? { borderColor: "var(--accent)", color: "var(--text)", cursor: "pointer" }
                  : counts[typ]
                    ? { borderColor: "var(--accent-line)", color: "var(--text-dim)", cursor: "pointer" }
                    : { opacity: 0.55, cursor: "pointer" }
              }
            >
              {t(SIGNAL_LABEL[typ])}
              <b className="num" style={{ color: "var(--text)" }}>
                {counts[typ] ?? 0}
              </b>
            </button>
          ))}
        </div>
      </Card>

      <div className="dash">
        <div className="dash__main">
          <ChatPanel height="calc(100vh - 430px)" kind={filtr} onKind={setFiltr} />
        </div>
        <aside className="dash__side">
          <SimulateMessagePanel />
          <BasketsPanel limit={4} />
        </aside>
      </div>
    </div>
  );
}
