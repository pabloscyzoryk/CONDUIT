import { Badge, Card, Icon } from "@/components/ui";
import { ChartsSection } from "@/components/chart/ChartsSection";
import { PositionsPanel } from "@/components/panels/PositionsPanel";
import { PendingsPanel } from "@/components/panels/PendingsPanel";
import { BasketsPanel } from "@/components/panels/BasketsPanel";
import { LotPanel, TicketPanel } from "@/components/panels/TicketPanel";
import { ChatPanel } from "@/components/panels/ChatPanel";
import { WartoscNog, tonStraznika, type Ton } from "@/components/panels/WartoscNog";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import { AI_MODELS } from "@/data/telegram";
import { presetDlaFormatu } from "@/data/formaty";
import "./views.css";

/* Wygląd notki trybu (ikona i kolory) — treść mieszka w słowniku
   (`dash.note.<tryb>.title` / `.text`). */
const MODE_NOTE = {
  MANUAL: { icon: "hand" as const, color: "var(--info-text)", bg: "var(--info-soft)" },
  AUTO: { icon: "bolt" as const, color: "var(--accent-text)", bg: "var(--accent-soft)" },
  /* AUTO-EA dzieli barwy z AUTO (zaawansowane AUTO); odróżnia go ikona. */
  "AUTO-EA": { icon: "robot" as const, color: "var(--accent-text)", bg: "var(--accent-soft)" },
  AI: { icon: "brain" as const, color: "var(--ai-text)", bg: "var(--ai-soft)" },
};

export function DashboardView() {
  const app = useApp();
  const t = useT();
  const note = MODE_NOTE[app.mode];
  const noteKey = `dash.note.${app.mode.toLowerCase()}`;
  const model = AI_MODELS.find((m) => m.id === app.settings.ai_model);
  const preset = app.findPreset(app.presetId);

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("dash.title")}</h1>
          {/* AKTYWNY JEST ŁAŃCUCH, NIE POJEDYNCZY PRESET.
              Odkąd bot słucha kilku formatów naraz, pracuje kilkoma presetami
              jednocześnie — jedna nazwa w nagłówku mówiłaby nieprawdę. Preset
              wczytany ręcznie (`presetId`) pokazujemy OBOK, bo nadal istnieje:
              to ten, którego wartości siedzą w bieżących ustawieniach. */}
          <p>
            {t("dash.intro")} <b>{app.aktywnaNazwaLancucha}</b> —{" "}
            {app.formaty
              .map((f) => `${f.nazwa}: ${presetDlaFormatu(app.lancuch, f.nazwa) ?? t("topbar.notTrading")}`)
              .join(" · ")}
            .{preset ? ` ${t("dash.lastManualPreset", { name: preset.name })}` : ""}
          </p>
        </div>
        <div className="row row--tight">
          {app.connection.mt5 !== "connected" && <Badge tone="warn">{t("stats.mt5.staleAccount")}</Badge>}
          <Badge tone="muted" dot>
            {app.connection.account.broker}
          </Badge>
          <Badge tone={app.connection.mt5 !== "connected" ? "muted" : app.connection.account.type === "DEMO" ? "warn" : "long"}>{app.connection.account.type}</Badge>
          <Badge tone="muted">#{app.connection.account.login}</Badge>
          <Badge tone="muted">1:{app.connection.account.leverage}</Badge>
        </div>
      </div>

      <div className="modenote" style={{ background: note.bg, borderColor: "transparent" }}>
        <span className="modenote__icon" style={{ background: "var(--bg-surface)", color: note.color }}>
          <Icon name={note.icon} size={16} />
        </span>
        <div>
          <b style={{ color: note.color }}>{t(`${noteKey}.title`)}</b>
          <p>{t(`${noteKey}.text`)}</p>
          {app.mode === "AI" && model && (
            <p style={{ marginTop: 5 }}>
              {t("dash.activeModel")} <b style={{ color: "var(--ai-text)" }}>{model.name}</b> ·{" "}
              {t("dash.model.cadence", { v: model.cadence })} · {t("dash.model.params", { v: model.params })} ·{" "}
              {t("dash.model.trainedOn", { v: t(model.trainedOn) })}.
            </p>
          )}
        </div>
      </div>

      <div className="dash">
        <div className="dash__main">
          <ChartsSection />
          <PositionsPanel />
          <PendingsPanel />
          <BasketsPanel />
        </div>

        <aside className="dash__side">
          <TicketPanel />
          <LotPanel />
          <ChatPanel height={480} dense />
          <RiskCard />
        </aside>
      </div>
    </div>
  );
}


function RiskCard() {
  const { effectiveSettings: s, stats, snapshot, settings, ustawieniaNog, lancuch } = useApp();
  const t = useT();

  const grajace = ustawieniaNog.filter((n) => n.handluje);
  const perNoga = grajace.length > 0;
  /* Mianownik limitu pozycji to SUMA limitów nóg — tak samo liczy pułapy
     `LancuchyPanel`. Jedna liczba z dokumentu nie była limitem żadnego
     silnika: przy dwóch nogach 40+20 bot mógł mieć 60 pozycji, a pasek
     straszył czterdziestką.

     ZERO ZNACZY „BEZ LIMITU", NIE „ZERO POZYCJI" — więc jedna noga bez
     limitu robi sumę nieskończoną, a nie mniejszą o zero. */
  const limityNog = grajace.map((n) => Number(n.doc.max_open_positions) || 0);
  const nogaBezLimitu = limityNog.some((v) => v <= 0);
  const sumaLimitowNog = nogaBezLimitu ? 0 : limityNog.reduce((a, v) => a + v, 0);
  /* Pułap ŁAŃCUCHA liczy CAŁY rachunek (własne + cudze pozycje), więc ma
     własny licznik — mieszanie go z licznikiem bota zaniżałoby ułamek. */
  const pulapPozycji = lancuch?.pulapy?.maxPozycji ?? 0;
  const limitPozycji = perNoga ? sumaLimitowNog : s.max_open_positions;
  /* Próg paska DD to NAJCIAŚNIEJSZY limit spośród nóg — utnie najwcześniej,
     więc to on decyduje, kiedy pasek ma być czerwony.
     Limit KWOTOWY (`max_dd_usd`) też się liczy: przeliczamy go na procent
     szczytu equity dnia, bo pasek jest procentowy. Bez tego preset pilnowany
     kwotowo miał skalę 100 % i pasek stał zielony do samego stopu.
     Pułap ŁAŃCUCHA wchodzi do tego samego minimum — to trzecia bramka
     obsunięcia i bywa ciaśniejsza od obu presetowych. */
  const ddNaProcent = (pct: number, usd: number): number => {
    if (pct > 0) return pct;
    if (usd > 0 && stats.peakEquityToday > 0) return (usd / stats.peakEquityToday) * 100;
    return 100;
  };
  const limityDd = perNoga
    ? grajace.map((n) => ddNaProcent(Number(n.doc.max_dd_pct) || 0, Number(n.doc.max_dd_usd) || 0))
    : [ddNaProcent(s.max_dd_pct, s.max_dd_usd)];
  if (lancuch?.pulapy) {
    limityDd.push(ddNaProcent(lancuch.pulapy.maxDdPct || 0, lancuch.pulapy.maxDdUsd || 0));
  }
  const limitDd = Math.min(...limityDd);

  const guards: { label: string; value: React.ReactNode; ton: Ton }[] = [
    {
      label: t("risk.maxDrawdown"),
      value: (
        /* DWA POLA, JEDEN STRAŻNIK: procent ALBO kwota. Sam `max_dd_pct`
           pokazywał „—" na presecie pilnowanym kwotowo — czyli „nie ma
           strażnika" tam, gdzie strażnik jest. */
        <WartoscNog
          pole="max_dd_pct"
          wartosc={(d) => (d.max_dd_pct > 0 ? `${d.max_dd_pct}%` : d.max_dd_usd > 0 ? `$${d.max_dd_usd}` : "—")}
          zapas={s.max_dd_pct > 0 ? `${s.max_dd_pct}%` : s.max_dd_usd > 0 ? `$${s.max_dd_usd}` : "—"}
        />
      ),
      ton: tonStraznika(grajace, (d) => d.max_dd_pct > 0 || d.max_dd_usd > 0),
    },
    {
      label: t("risk.dayTarget"),
      value: (
        /* `day_target_pct` jest osobną bramką silnika (`bramka_wejscia`:
           „cel dzienny {x} % osiągnięty"), więc preset ustawiony procentowo
           przestaje brać sygnały — a karta mówiła „—". */
        <WartoscNog
          pole="day_target_usd"
          wartosc={(d) =>
            d.day_target_usd > 0 ? `$${d.day_target_usd}` : d.day_target_pct > 0 ? `${d.day_target_pct}%` : "—"
          }
          zapas={s.day_target_usd > 0 ? `$${s.day_target_usd}` : s.day_target_pct > 0 ? `${s.day_target_pct}%` : "—"}
        />
      ),
      ton: tonStraznika(grajace, (d) => d.day_target_usd > 0 || d.day_target_pct > 0),
    },
    /* Limit ekspozycji jest limitem SILNIKA — liczy wyłącznie pozycje, które
       bot sam otworzył. Doliczanie tu cudzych (widocznych od czasu, gdy panel
       pokazuje cały rachunek) zawyżałoby licznik i straszyło limitem, którego
       silnik w ogóle nie widzi.
       PUŁAP ŁAŃCUCHA jest drugą bramką i liczy CAŁY rachunek, więc dostaje
       własny ułamek obok — z licznikiem ze wszystkich pozycji konta. */
    {
      label: t("risk.exposureLimit"),
      value:
        limitPozycji > 0 || pulapPozycji > 0 || nogaBezLimitu ? (
          <>
            <span className="num">{snapshot.positions.length - snapshot.foreign.positions}</span>/
            <WartoscNog
              pole="max_open_positions"
              wartosc={(d) => (d.max_open_positions > 0 ? String(d.max_open_positions) : t("risk.noLimit"))}
              zapas={s.max_open_positions > 0 ? String(s.max_open_positions) : t("risk.noLimit")}
            />
            {pulapPozycji > 0 && (
              <span className="hint" style={{ marginLeft: 6 }}>
                {t("risk.chainCap", { n: String(snapshot.positions.length), v: String(pulapPozycji) })}
              </span>
            )}
          </>
        ) : (
          "—"
        ),
      ton: tonStraznika(grajace, (d) => d.max_open_positions > 0),
    },
    {
      label: t("risk.sessionFilter"),
      value: (
        /* PRZEŁĄCZNIK RZĄDZI GODZINAMI. `session_hours` zostaje w presecie
           po wyłączeniu filtru (STORM-A1 i SYN-A1: `session_filter = false`,
           `session_hours = 8-15`), więc czytanie samego pola godzin pisało
           filtr tam, gdzie żadnego nie ma — zgłoszenie właściciela. */
        <WartoscNog
          pole="session_hours"
          wartosc={(d) => (d.session_filter ? String(d.session_hours || "—") : "—")}
          zapas={s.session_filter ? s.session_hours : "—"}
        />
      ),
      ton: tonStraznika(grajace, (d) => d.session_filter),
    },
    {
      label: t("risk.eodFlat"),
      value: (
        <WartoscNog
          pole="eod_flat_hour"
          format={(v) => (Number(v) > 0 ? `${v}:00` : "—")}
          zapas={s.eod_flat_hour > 0 ? `${s.eod_flat_hour}:00` : "—"}
        />
      ),
      ton: tonStraznika(grajace, (d) => d.eod_flat_hour > 0),
    },
    {
      label: t("risk.pendingTtl"),
      value: (
        <WartoscNog
          pole="pending_ttl_h"
          format={(v) => (Number(v) > 0 ? `${v} h` : "—")}
          zapas={s.pending_ttl_h > 0 ? `${s.pending_ttl_h} h` : "—"}
        />
      ),
      ton: tonStraznika(grajace, (d) => d.pending_ttl_h > 0),
    },
  ];

  const ddPct = stats.peakEquityToday > 0 ? (stats.drawdownNow / stats.peakEquityToday) * 100 : 0;
  const limit = limitDd > 0 ? limitDd : 100;

  return (
    <Card title={t("risk.title")} icon="shield-alert" accent="var(--short)" subtitle={settings.display_currency}>
      <div className="col">
        <div>
          <div className="row" style={{ justifyContent: "space-between", marginBottom: 5 }}>
            <span className="hint">{t("risk.ddFromPeak")}</span>
            <b className="num" style={{ fontSize: "var(--fs-sm)" }}>
              {ddPct.toFixed(1)}%
            </b>
          </div>
          <span className="meter">
            <span
              className="meter__fill"
              style={{
                width: `${Math.min(100, (ddPct / limit) * 100)}%`,
                background: ddPct / limit > 0.7 ? "var(--short)" : ddPct / limit > 0.4 ? "var(--warn)" : "var(--long)",
              }}
            />
          </span>
        </div>

        <div className="divider" style={{ margin: "2px 0" }} />

        {guards.map((g) => (
          <div key={g.label} className="row" style={{ justifyContent: "space-between" }}>
            <span className="hint" style={{ display: "flex", alignItems: "center", gap: 6 }}>
              {/* ŻÓŁTA = strażnik działa u CZĘŚCI nóg. Zielona bez tego stanu
                  była kłamstwem: wystarczyło, żeby filtr miała JEDNA noga. */}
              <span
                className="dot"
                style={{
                  background:
                    g.ton === "wszystkie"
                      ? "var(--long)"
                      : g.ton === "czesc"
                        ? "var(--warn)"
                        : "var(--text-faint)",
                }}
              />
              {g.label}
            </span>
            <span
              className="num"
              style={{ fontSize: "var(--fs-xs)", color: g.ton === "zadna" ? "var(--text-faint)" : "var(--text)" }}
            >
              {g.value}
            </span>
          </div>
        ))}
      </div>
    </Card>
  );
}
