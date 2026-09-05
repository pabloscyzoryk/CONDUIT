import { useEffect, useState, type ReactNode } from "react";
import { Badge, Button, Icon, Segmented, Tooltip, type IconName } from "@/components/ui";
import { LancuchyPanel, WyborLancucha } from "@/components/panels/LancuchyPanel";
import { DymekLotAuto, useEkspozycjaAuto } from "@/components/panels/LotAuto";
import { CofnijPonow } from "./CofnijPonow";
import { WiekKwotowania, ZdrowieMT5, stanKwotowania, useTykanie } from "./PulsRynku";
import { presetDlaFormatu } from "@/data/formaty";
import { useApp } from "@/store/AppStore";
import { useTheme } from "@/store/useTheme";
import { t, useT } from "@/i18n";
import { tSilnik } from "@/i18n/silnik";
import { money, num, pct, timeShort, toneOf } from "@/lib/format";
import type { ConnectionState } from "@/types";
import type { TradingMode } from "@/types";
import "./shell.css";

export type ViewId =
  | "dashboard"
  | "signals"
  | "channels"
  | "settings"
  | "history"
  | "sims"
  | "logs"
  | "aimodels"
  | "lab"
  | "demo"
  | "kronika";

/* Nowe widoki dopisywane na KOŃCU listy celowo: skróty 1–7 prowadzą do tych
   samych ekranów co dotąd, a kolejne dostają wolne numery.
   Etykiety i podpowiedzi mieszkają w słowniku (`nav.<id>` / `nav.<id>.hint`). */
const NAV: { id: ViewId; icon: IconName }[] = [
  { id: "dashboard", icon: "dashboard" },
  { id: "signals", icon: "signal" },
  { id: "channels", icon: "channels" },
  { id: "settings", icon: "settings" },
  { id: "history", icon: "history" },
  { id: "sims", icon: "flask" },
  { id: "logs", icon: "logs" },
  { id: "aimodels", icon: "brain" },
  { id: "lab", icon: "microscope" },
  { id: "demo", icon: "robot" },
  { id: "kronika", icon: "edit" },
];

const MODE_META: Record<TradingMode, { icon: IconName; bg: string; fg: string }> = {
  MANUAL: { icon: "hand", bg: "var(--info-soft)", fg: "var(--info-text)" },
  AUTO: { icon: "bolt", bg: "var(--accent-soft)", fg: "var(--accent-text)" },
  /* AUTO-EA dzieli barwy z AUTO celowo (to zaawansowane AUTO — kontrakt
     zera); odróżnia go ikona. Tokeny akcentu istnieją w KAŻDEJ palecie,
     czego nie można powiedzieć np. o --warn-soft. */
  "AUTO-EA": { icon: "robot", bg: "var(--accent-soft)", fg: "var(--accent-text)" },
  AI: { icon: "brain", bg: "var(--ai-soft)", fg: "var(--ai-text)" },
};

/** Opis trybu — z klucza słownika (`mode.manual.desc` itd.). */
const modeDesc = (m: TradingMode) => t(`mode.${m.toLowerCase()}.desc`);

/** Cztery liczby, ktorych plakietka „connected" nie niesie. */
function zdrowieTelegrama(c: ConnectionState): string {
  if (c.telegram !== "connected") return t("shell.health.disconnected");
  const min = (ms?: number | null) =>
    !ms
      ? t("shell.health.never")
      : t("shell.health.minAgo", { n: Math.max(0, Math.round((Date.now() - ms) / 60000)) });
  const bledy = c.telegramPingFailures ?? 0;
  return [
    t("shell.health.lastMessage", { v: min(c.telegramLastMessageMs) }),
    t("shell.health.lastPing", { v: min(c.telegramLastPingOkMs) }),
    t("shell.health.pingFailures", { n: bledy }),
    t("shell.health.reconnects", { n: c.telegramReconnects ?? 0 }),
    bledy > 0 ? t("shell.health.warnDead") : t("shell.health.quietOk"),
  ].join(String.fromCharCode(10));
}

export function Shell({ view, onView, children }: { view: ViewId; onView: (v: ViewId) => void; children: ReactNode }) {
  const app = useApp();
  const { theme, toggle } = useTheme();
  const tt = useT();
  const [collapsed, setCollapsed] = useState(() => window.innerWidth < 1180);
  const [mobileNav, setMobileNav] = useState(false);
  const [lancuchyOtwarte, setLancuchyOtwarte] = useState(false);

  const cur = app.settings.display_currency;
  const eqTone = toneOf(app.stats.pnlSession);

  /* skróty klawiszowe 1–7 przełączają widoki (L obsługuje I18nProvider) */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement;
      if (el && ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return;
      const i = Number(e.key);
      if (i >= 1 && i <= NAV.length) onView(NAV[i - 1].id);
      if (e.key.toLowerCase() === "t" && !e.ctrlKey && !e.metaKey) toggle();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onView, toggle]);

  /* SKRÓT „PARAMETRY EA" z panelu łańcuchów (projekt EA-2). Panel zgłasza
     żądanie do wspólnego stanu, Shell przełącza WIDOK, a ekran ustawień
     ustawia się na właściwym presecie i sekcji EA — każdy robi swoje,
     żaden nie musi znać wnętrza pozostałych. Żądania NIE kasujemy tutaj:
     zdejmuje je dopiero ten, kto je obsłużył. */
  const zadanieEa = app.zadanieEa;
  useEffect(() => {
    if (zadanieEa) onView("settings");
  }, [zadanieEa, onView]);

  /* GÓRNY PASEK NIE MA JUŻ WYBORU PRESETU.
     Preset przestał być jedną globalną decyzją w chwili, gdy bot słucha kilku
     formatów naraz: pod ATFX gra jeden zestaw ustawień, pod Synergy inny.
     Jedna lista presetów w pasku nie miała jak tego wyrazić i musiałaby
     kłamać, wskazując „aktywny" preset przy dwóch pracujących.
     Zastąpił ją wybór ŁAŃCUCHA (komplet decyzji format → preset) i przycisk
     do panelu, w którym się je układa. */
  const q = app.primary;
  const chgTone = toneOf(q.change);
  /* Wiek kwotowania liczy się z ZEGARA, nie z przyjścia danych — gdy strumień
     staje, panel przestaje się przerysowywać i licznik zamarłby razem z nim. */
  const teraz = useTykanie();
  const stanQ = stanKwotowania(q.time, teraz);
  const martwe = stanQ === "martwe" || stanQ === "brak";
  /* TRYB AUTO-EA ma WŁASNY wskaźnik aktywnego łańcucha (projekt EA-2), więc
     i własną nazwę wejścia do panelu. Jedna flaga, żeby warunek nie rozsypał
     się po trzech miejscach w tym pliku. */
  const ea = app.mode === "AUTO-EA";

  return (
    <div className={`shell ${collapsed ? "shell--collapsed" : ""} ${mobileNav ? "shell--navopen" : ""}`}>
      {/* ================= SIDEBAR ================= */}
      <aside className="rail">
        <div className="rail__brand">
          <span className="rail__logo">
            <svg viewBox="0 0 32 32" width="26" height="26" aria-hidden="true">
              <rect width="32" height="32" rx="8" fill="url(#rg)" />
              <path d="M8 20.5l5.5-7 3.8 4.6L24 9" stroke="#fff" strokeWidth="2.6" fill="none" strokeLinecap="round" strokeLinejoin="round" />
              <circle cx="24" cy="9" r="2.4" fill="#fff" />
              <defs>
                <linearGradient id="rg" x1="0" y1="0" x2="32" y2="32">
                  <stop stopColor="#8b95ff" />
                  <stop offset="1" stopColor="#5563f5" />
                </linearGradient>
              </defs>
            </svg>
          </span>
          <span className="rail__name">
            CONDUIT
            <small>Telegram → MT5</small>
          </span>
          <button className="rail__collapse" onClick={() => setCollapsed((c) => !c)} title={tt("shell.collapse")}>
            <Icon name={collapsed ? "chevron-right" : "chevron-left"} size={14} />
          </button>
        </div>

        <nav className="rail__nav">
          {NAV.map((n, i) => (
            <button
              key={n.id}
              className="rail__item"
              data-active={view === n.id}
              onClick={() => {
                onView(n.id);
                setMobileNav(false);
              }}
              title={collapsed ? `${tt(`nav.${n.id}`)} — ${tt(`nav.${n.id}.hint`)}` : tt(`nav.${n.id}.hint`)}
            >
              <Icon name={n.icon} size={17} />
              <span className="rail__label">{tt(`nav.${n.id}`)}</span>
              <kbd className="rail__kbd">{i + 1}</kbd>
              {n.id === "signals" && app.messages.some((m) => m.pendingAction === "await") && (
                <span className="rail__dot" />
              )}
            </button>
          ))}
        </nav>

        <div className="rail__foot">
          <div className="rail__conn">
            <Tooltip content={zdrowieTelegrama(app.connection)}>
              <span
                className={`conn ${
                  app.connection.telegram === "connected"
                    ? (app.connection.telegramPingFailures ?? 0) > 0
                      ? "conn--warn"
                      : "conn--ok"
                    : "conn--off"
                }`}
              >
                <span className="dot dot--pulse" />
                <span className="rail__label">
                  Telegram
                  {/* Nieudane pingi POD RZĄD to jedyna liczba, ktora odroznia
                      spokojna noc od martwego gniazda MTProto — `next_message()`
                      przy zerwanej sesji nie zwraca bledu, tylko milczy. */}
                  {(app.connection.telegramPingFailures ?? 0) > 0 && ` · ${app.connection.telegramPingFailures}✗`}
                </span>
              </span>
            </Tooltip>
            {}
            <span
              className={`conn ${
                app.connection.mt5 !== "connected" ? "conn--off" : martwe ? "conn--warn" : "conn--ok"
              }`}
            >
              <span className="dot dot--pulse" />
              <span className="rail__label">
                MT5 · {app.connection.latencyMs} ms
                {martwe && app.connection.mt5 === "connected" && " · ✗"}
              </span>
            </span>
          </div>

          {/* Okno natywne ładuje TEN SAM adres co przeglądarka, więc oba
              interfejsy mogą działać naraz na jednym stanie. Przycisk otwiera
              stronę przez serwer — w WebView2 `window.open` uruchomiłby
              kolejny webview zamiast przeglądarki systemowej. */}
          {app.nativeShell && (
            <button className="rail__browser" onClick={app.openInBrowser} title={tt("shell.openBrowser.title")}>
              <Icon name="external" size={14} />
              <span className="rail__label">{tt("shell.openBrowser")}</span>
            </button>
          )}

          {/* KONTO TELEGRAMA — pokazywane WYŁĄCZNIE, gdy sesja naprawdę istnieje.
              Aplikacja DZIAŁA bez Telegrama (MetaTrader, symulacje, tryb demo,
              laboratorium), więc brak sesji to normalny stan pracy, a nie błąd
              — stąd zwykły przycisk logowania zamiast blokady albo ostrzeżenia. */}
          {app.telegramZalogowany ? (
            <button className="rail__user" onClick={app.logout} title={tt("shell.tgLogout.title")}>
              <span className="rail__avatar">PC</span>
              <span className="rail__label rail__uinfo">
                <b>{app.connection.user.name}</b>
                <small>{app.connection.user.handle}</small>
              </span>
              <Icon name="logout" size={14} className="rail__logout" />
            </button>
          ) : (
            <button className="rail__browser" onClick={app.otworzLogowanieTg} title={tt("shell.tgLogin.title")}>
              <Icon name="telegram" size={14} />
              <span className="rail__label">{tt("shell.tgLogin")}</span>
            </button>
          )}
        </div>
      </aside>

      {/* ================= GŁÓWNA KOLUMNA ================= */}
      <div className="main">
        <header className="topbar">
          <button className="topbar__burger" onClick={() => setMobileNav((v) => !v)} title={tt("common.menu")}>
            <Icon name="menu" size={17} />
          </button>

          {/* COFNIJ / PONÓW — LEWY GÓRNY RÓG, tuż za menu (TODO K16).
              Miejsce nie jest przypadkowe: tam szuka ich ręka po pomyłce,
              bo tam siedzą w każdym edytorze. */}
          <CofnijPonow />

          <div className="ticker">
            {/* SYMBOL Z KWOTOWANIA, nie stała. U PUPrime złoto nazywa się
                „XAUUSD.s" i pasek pokazywał nazwę z ery Vantage obok ceny
                wziętej z generatora — dwa kłamstwa w jednym wierszu. */}
            <span className="ticker__sym">{q.symbol || "—"}</span>
            {}
            <span className={`ticker__px num ${martwe ? "" : chgTone}`}>{num(q.bid, 2)}</span>
            <span className={`ticker__chg num ${martwe ? "" : chgTone}`}>
              <Icon name={q.change >= 0 ? "arrow-up-right" : "arrow-down-right"} size={11} />
              {pct(q.changePct, 2)}
            </span>
            {}
            {}
            <span className="ticker__meta" title={tt("topbar.spread")}>
              <span className="ticker__meta-etykieta topbar__hide-lg">{tt("topbar.spread")}</span>
              <b className="num">{num(q.spread, 2)}</b>
            </span>
            <WiekKwotowania />
          </div>

          <ZdrowieMT5 />

          <div className="divider--v topbar__hide-sm" />

          <Tooltip content={modeDesc(app.mode)}>
            <Segmented<TradingMode>
              value={app.mode}
              onChange={app.setMode}
              options={(["MANUAL", "AUTO", "AUTO-EA", "AI"] as TradingMode[]).map((m) => ({
                value: m,
                bg: MODE_META[m].bg,
                fg: MODE_META[m].fg,
                label: (
                  <>
                    <Icon name={MODE_META[m].icon} size={13} />
                    {m}
                  </>
                ),
              }))}
            />
          </Tooltip>

          {}
          <div className="topbar__preset">
            <Tooltip
              content={
                app.lancuch
                  ? `${app.lancuch.nazwa} — ${app.formaty
                      .map((f) => {
                        const p = presetDlaFormatu(app.lancuch, f.nazwa);
                        return `${f.nazwa}: ${p ?? tt("topbar.notTrading")}`;
                      })
                      .join(", ")}`
                  : tt("topbar.noChain")
              }
            >
              {/* NAZWA IDZIE ZA TRYBEM (projekt EA-2). W AUTO-EA ten przycisk
                  otwiera ŁAŃCUCHY EA — inny wskaźnik aktywnego łańcucha,
                  inny skład na rachunku. Jedna nazwa dla dwóch różnych
                  wskazań kazałaby zgadywać, który z nich się właśnie
                  przestawia. */}
              <Button
                variant="outline"
                size="sm"
                icon="layers"
                onClick={() => setLancuchyOtwarte(true)}
                title={tt(ea ? "topbar.chains.ea.title" : "topbar.chains.title")}
              >
                {tt(ea ? "topbar.chains.ea" : "topbar.chains")}
              </Button>
            </Tooltip>
            <span className="topbar__hide-md">
              <WyborLancucha size="sm" />
            </span>
          </div>

          <div className="spacer" />

          <div className="topbar__acct topbar__hide-md">
            {}
            <span className="eyebrow">
              {app.connection.mt5 !== "connected" ? `${tt("stats.mt5.staleAccount")} · ` : ""}
              {app.connection.account.type}
              {app.connection.account.login ? ` · ${app.connection.account.server} · #${app.connection.account.login}` : ""}
            </span>
            <b className="num">{money(app.stats.equity, cur)}</b>
            <span className={`num ${eqTone}`}>
              {app.stats.pnlSession >= 0 ? "+" : "−"}
              {money(Math.abs(app.stats.pnlSession), cur)}
            </span>
          </div>

          <Button variant="ghost" size="sm" icon={theme === "dark" ? "sun" : "moon"} onClick={toggle} title={tt("topbar.theme.title")} />
        </header>

        <StatStrip />

        {app.halt.active && (
          <div className="halt">
            <Icon name="alert" size={16} />
            <div>
              <b>{tt("halt.title")}</b>
              <span>
                {tSilnik(app.halt.reason)} · {tt(app.halt.diagnoza?.trim() ? "halt.diagnosisNote" : "halt.resumeNote")}
              </span>
            </div>
            <Button variant="danger" size="sm" icon="play" onClick={app.resetHalt} disabled={Boolean(app.halt.diagnoza?.trim())}>
              {tt(app.halt.diagnoza?.trim() ? "halt.diagnosisAction" : "halt.resume")}
            </Button>
          </div>
        )}

        {app.riskOverride.active && (
          <div className="halt halt--override">
            <Icon name="shield-alert" size={16} />
            <div>
              <b>{tt("halt.overrideTitle")}</b>
              <span>
                {tSilnik(app.riskOverride.reason)} ·{" "}
                {tt("halt.overrideNote", { time: timeShort(app.riskOverride.since) })}
              </span>
            </div>
            <Button variant="outline" size="sm" icon="shield" onClick={app.clearRiskOverride}>
              {tt("halt.rearm")}
            </Button>
          </div>
        )}

        <main className="content">{children}</main>
      </div>

      {mobileNav && <div className="shell__scrim" onClick={() => setMobileNav(false)} />}

      <LancuchyPanel open={lancuchyOtwarte} onClose={() => setLancuchyOtwarte(false)} />
    </div>
  );
}



/* ============================================================
   PASEK STATYSTYK — odpowiednik `.stats-grid` z bot.py
   ============================================================ */
function StatStrip() {
  const { stats, snapshot, connection, settings, mode } = useApp();
  const tt = useT();
  /* LOT AUTO w pasku — SUFIT SKUTECZNY, nie sama suma koszyków nóg
     (`AppStore.lotSize`). Pułapy aktywnego łańcucha wiążą PIERWSZE, więc
     suma nóg bywa obietnicą wyższą od tego, co bot w ogóle wpuści na
     rachunek. Cały rachunek — margines, procent salda, poziom marginesu —
     stoi w dymku obok liczby. */
  const eks = useEkspozycjaAuto();
  const cur = settings.display_currency;

  const fgn = snapshot.foreign;
  const botPos = snapshot.positions.length - fgn.positions;
  const botPend = snapshot.pendings.length - fgn.pendings;

  const items = [
    { k: tt("stats.balance"), v: money(stats.balance, cur), tone: "" as const },
    { k: tt("stats.equity"), v: money(stats.equity, cur), tone: "" as const },
    {
      k: tt("stats.pnlToday"),
      v: `${stats.pnlToday >= 0 ? "+" : "−"}${money(Math.abs(stats.pnlToday), cur)}`,
      tone: toneOf(stats.pnlToday),
    },
    {
      k: tt("stats.pnlSession"),
      v: `${stats.pnlSession >= 0 ? "+" : "−"}${money(Math.abs(stats.pnlSession), cur)}`,
      tone: toneOf(stats.pnlSession),
    },
    { k: tt("stats.drawdown"), v: money(stats.drawdownNow, cur), tone: stats.drawdownNow > 0 ? ("down" as const) : ("" as const) },
    { k: tt("stats.maxDdToday"), v: money(stats.maxDdToday, cur), tone: "" as const },
    /* Liczniki mówią o CAŁYM rachunku — bo tyle realnie na nim wisi — a gdy
       coś nie należy do bota, etykieta od razu to rozbija. Wcześniej stało tu
       „Pozycje 0" przy trzech otwartych na koncie i to była nieprawda. */
    {
      k: fgn.positions ? tt("stats.positionsTotal") : tt("stats.positions"),
      v: fgn.positions
        ? `${snapshot.positions.length} · ${tt("stats.bot")} ${botPos}`
        : String(snapshot.positions.length),
      tone: "" as const,
    },
    {
      k: fgn.pendings ? tt("stats.pendingsTotal") : tt("stats.pendings"),
      v: fgn.pendings
        ? `${snapshot.pendings.length} · ${tt("stats.bot")} ${botPend}`
        : String(snapshot.pendings.length),
      tone: "" as const,
    },
    /* To sa liczniki SILNIKA — licza od uruchomienia procesu i zeruja sie przy
       restarcie, choc lista wiadomosci wraca z backupu. Etykieta musi to
       mowic, inaczej „Wiadomosci 0" nad niepusta lista wyglada na awarie. */
    { k: tt("stats.messages"), v: String(stats.messages), tone: "" as const },
    { k: tt("stats.signals"), v: String(stats.signals), tone: "" as const },
    
    {
      k: tt("stats.botLot"),
      v: num(eks.sufit.sufit, 2),
      
      tone: eks.koszt.stan === "alarm" || eks.koszt.stan === "zle" ? ("down" as const) : ("" as const),
      tip: <DymekLotAuto nogi={stats.lotNogi ?? []} />,
    },
    {
      
      k: "MT5",
      v:
        connection.mt5 !== "connected"
          ? tt("stats.mt5.none")
          : `${connection.account.server || "?"} · #${connection.account.login || "?"}${
              connection.accountVerified === "brak"
                ? ` · ${tt("stats.mt5.unverified")}`
                : connection.accountVerified === "rozjazd"
                  ? ` · ${tt("stats.mt5.mismatch")}`
                  : ""
            }`,
      tone:
        connection.mt5 !== "connected"
          ? ("down" as const)
          : connection.accountVerified === "rozjazd"
            ? ("down" as const)
            : connection.accountVerified === "brak"
              ? ("warn" as const)
              : ("up" as const),
    },
  ];

  return (
    <div className="statstrip">
      {items.map((it) => (
        <div className="stat" key={it.k}>
          <span className="stat__k">
            {it.k}
            {"tip" in it ? it.tip : null}
          </span>
          <span className={`stat__v num ${it.tone}`}>{it.v}</span>
        </div>
      ))}
      <div className="stat stat--mode">
        <span className="stat__k">{tt("stats.mode")}</span>
        <Badge tone={mode === "AI" ? "ai" : mode === "AUTO" || mode === "AUTO-EA" ? "accent" : "info"} dot>
          {mode}
        </Badge>
      </div>
    </div>
  );
}
