import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Card, Checkbox, Icon, Modal, Select, Switch, TextInput, Tooltip } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT, RichT } from "@/i18n";
import { opisFormatu } from "@/i18n/silnik";
import { presetDlaFormatu } from "@/data/formaty";
import { ago, compact } from "@/lib/format";
import type { ChannelBinding, TelegramChannel } from "@/types";
import "./views.css";

/** „Nie handluj tym kanałem". Pusty łańcuch znaków = brak formatu. */
const BEZ_FORMATU = "";

export function ChannelsView() {
  const app = useApp();
  const t = useT();
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<"all" | "monitored" | "notify">("all");
  const [fmtFor, setFmtFor] = useState<TelegramChannel | null>(null);

  const wszystkie = app.channels;

  const list = useMemo(
    () =>
      wszystkie.filter((c) => {
        const b = app.bindings[c.id];
        if (filter === "monitored" && !b?.monitored) return false;
        if (filter === "notify" && !b?.notify) return false;
        return c.name.toLowerCase().includes(q.toLowerCase()) || c.handle.toLowerCase().includes(q.toLowerCase());
      }),
    [q, filter, app.bindings, wszystkie],
  );

  const monitored = wszystkie.filter((c) => app.bindings[c.id]?.monitored).length;
  const notifying = wszystkie.filter((c) => app.bindings[c.id]?.notify).length;

  /** Realne liczniki z bieżącej sesji — zamiast deklarowanych statystyk jakości. */
  const live = useMemo(() => {
    const acc: Record<number, { messages: number; signals: number }> = {};
    for (const m of app.messages) {
      const e = (acc[m.channelId] ??= { messages: 0, signals: 0 });
      e.messages++;
      if (m.types.some((typ) => typ !== "INFO")) e.signals++;
    }
    return acc;
  }, [app.messages]);

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("chan.title")}</h1>
          <p>
            <RichT k="chan.intro" />
          </p>
        </div>
        <div className="row row--tight">
          <Badge tone="accent" dot>
            {t("chan.badge.monitored", { n: monitored })}
          </Badge>
          <Badge tone="info" dot>
            {t("chan.badge.notify", { n: notifying })}
          </Badge>
        </div>
      </div>

      <Card
        title={t("chan.list.title")}
        icon="channels"
        subtitle={t("chan.list.subtitle", { n: list.length, all: wszystkie.length })}
        accent="var(--accent)"
        actions={
          <>
            <TextInput
              value={q}
              onChange={setQ}
              placeholder={t("chan.search")}
              icon="search"
              size="sm"
              style={{ width: 210 }}
            />
            {(
              [
                ["all", t("chan.filter.all")],
                ["monitored", t("chan.filter.monitored")],
                ["notify", t("chan.filter.notify")],
              ] as const
            ).map(([id, label]) => (
              <Button key={id} size="sm" variant={filter === id ? "primary" : "outline"} onClick={() => setFilter(id)}>
                {label}
              </Button>
            ))}
            <Button
              size="sm"
              variant="ghost"
              icon="refresh"
              disabled={app.channelsLoading}
              onClick={() => app.refreshChannels()}
            >
              {app.channelsLoading ? t("chan.fetching") : t("chan.refresh")}
            </Button>
          </>
        }
      >
        {/* Pusta lista NIE jest stanem do przemilczenia: bez zaznaczonego
            kanału bot nie wykona ani jednego sygnału. */}
        {app.channelsAreReal && wszystkie.length === 0 && (
          <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
            {app.channelsLoading ? t("chan.empty.loading") : (app.channelsError ?? t("chan.empty.none"))}
          </p>
        )}
        {!app.channelsAreReal && <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>{t("chan.preview")}</p>}

        <div className="channels">
          {list.map((c) => {
            const b = app.bindings[c.id];
            const tematyZFormatem = Object.values(b?.topics ?? {}).filter((f) => f);
            const topicCount = tematyZFormatem.length;
            // Kanał zwykły ma 0 albo 1 format; forum tyle, ile tematów gra.
            const fmtCount = c.isForum ? topicCount : b?.format ? 1 : 0;
            const needsFormat = b?.monitored && fmtCount === 0;

            return (
              <article key={c.id} className="chan" data-monitored={!!b?.monitored}>
                <header className="chan__head">
                  <ChannelAvatar channel={c} />
                  <div className="chan__id">
                    <div className="chan__name">
                      <span className="truncate">{c.name}</span>
                      {c.verified && (
                        <Tooltip content={t("chan.verified")}>
                          <span style={{ color: "var(--info)", display: "inline-flex" }}>
                            <Icon name="check" size={12} strokeWidth={3} />
                          </span>
                        </Tooltip>
                      )}
                      {c.isForum && <Badge tone="muted">{t("chan.forum")}</Badge>}
                    </div>
                    <span className="chan__handle">
                      {c.handle || `id ${c.id}`}
                      {c.members > 0 ? ` · ${t("chan.members", { n: compact(c.members) })}` : ""}
                    </span>
                  </div>
                  <Switch
                    checked={!!b?.monitored}
                    onChange={(v) => app.setBinding(c.id, { monitored: v })}
                    title={t("chan.listen")}
                  />
                </header>

                {/* Liczniki liczone z TEJ sesji — żadnych deklarowanych
                    „skuteczności": każdy kanał ma inną i zmienia się w czasie. */}
                <div className="chan__stats">
                  <div className="chan__stat">
                    <b>{live[c.id]?.messages ?? 0}</b>
                    <span>{t("chan.stat.messages")}</span>
                  </div>
                  <div className="chan__stat">
                    <b className={live[c.id]?.signals ? "up" : undefined}>{live[c.id]?.signals ?? 0}</b>
                    <span>{t("chan.stat.signals")}</span>
                  </div>
                  {/* Kanał ma JEDEN format, więc pokazujemy jego nazwę,
                      a nie liczbę. „Formatów: 1" nie mówi nic, czego nie
                      wiadomo z góry; „ATFX" albo „nie handluje" mówi wszystko. */}
                  {!c.isForum && (
                    <div className="chan__stat">
                      <b className={b?.format ? undefined : "down"}>{b?.format || "—"}</b>
                      <span>{b?.format ? t("chan.stat.format") : t("chan.stat.notTrading")}</span>
                    </div>
                  )}
                  {c.isForum && (
                    <div className="chan__stat">
                      <b>
                        {topicCount}/{c.topics.length}
                      </b>
                      <span>{t("chan.stat.topicsTrading")}</span>
                    </div>
                  )}
                </div>

                {c.lastMessageTime > 0 && (
                  <p className="chan__last">
                    <span className="hint">{ago(c.lastMessageTime)}:</span> {c.lastMessage}
                  </p>
                )}

                <footer className="chan__foot">
                  <Checkbox
                    checked={!!b?.notify}
                    onChange={(v) => app.setBinding(c.id, { notify: v })}
                    label={<span className="hint">{t("chan.notifyHere")}</span>}
                  />
                  <span className="spacer" />
                  {needsFormat && (
                    <Tooltip content={t("chan.needFormat.tip")}>
                      <Badge tone="warn" dot>
                        {t("chan.needFormat")}
                      </Badge>
                    </Tooltip>
                  )}
                  <Button size="sm" variant="outline" icon="filter" onClick={() => setFmtFor(c)}>
                    {t("chan.formatBtn")}
                  </Button>
                </footer>
              </article>
            );
          })}
        </div>
      </Card>

      <Card
        title={t("chan.formats.title")}
        icon="book"
        accent="var(--info)"
        subtitle={t("chan.formats.subtitle", { n: app.formaty.length })}
      >
        <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
          <RichT k="chan.formats.intro" />
        </p>
        <div className="fmtlist">
          {app.formaty.map((f) => {
            const preset = presetDlaFormatu(app.lancuch, f.nazwa);
            return (
              <div key={f.nazwa} className="fmt" data-on={!!preset} style={{ cursor: "default" }}>
                <span
                  style={{
                    width: 30,
                    height: 30,
                    borderRadius: "var(--r-sm)",
                    display: "grid",
                    placeItems: "center",
                    background: "var(--bg-surface-3)",
                    color: "var(--accent-text)",
                    flex: "none",
                  }}
                >
                  <Icon name="book" size={15} />
                </span>
                <div className="fmt__body">
                  <div className="row row--tight">
                    <span className="fmt__name">{f.nazwa}</span>
                    <Badge tone="muted">{t("chan.fmt.parser", { v: f.parser })}</Badge>
                    {preset ? (
                      <Badge tone="accent">{t("chan.fmt.playsWith", { v: preset })}</Badge>
                    ) : (
                      <Badge tone="warn">{t("chan.fmt.idleInChain", { v: app.aktywnaNazwaLancucha })}</Badge>
                    )}
                  </div>
                  <span className="fmt__desc">{opisFormatu(f.nazwa, f.opis)}</span>
                  {f.przyklad && <pre className="fmt__sample">{f.przyklad}</pre>}
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <FormatModal channel={fmtFor} onClose={() => setFmtFor(null)} />
    </div>
  );
}

/* ============================================================
   AWATAR KANAŁU
   ============================================================ */
/**
 * PRAWDZIWE zdjęcie profilowe z Telegrama, a pod nim litera jako podkład.
 *
 * Litera NIE JEST wariantem „albo–albo": jest zawsze narysowana, a zdjęcie
 * dopiero ją zasłania, gdy się wczyta. Dzięki temu kółko wygląda tak samo
 * w trzech sytuacjach, w których obrazka nie ma: kanał go nie posiada,
 * jeszcze się nie pobrał, albo pobranie padło (`onError`). Żadna z nich nie
 * daje pustego kwadratu ani przeskoku układu — rozmiar kółka jest stały,
 * bo trzyma go kontener, a nie zawartość.
 */
function ChannelAvatar({ channel, size = 38 }: { channel: TelegramChannel; size?: number }) {
  const [stan, setStan] = useState<"czeka" | "jest" | "blad">("czeka");

  // Podmiana zdjęcia (nowe `?v=`) musi wrócić do stanu „czeka" — inaczej
  // stara, już nieaktualna klatka zostałaby widoczna do końca sesji.
  useEffect(() => setStan("czeka"), [channel.photoUrl]);

  return (
    <span
      className="chan__avatar"
      style={{
        background: `hsl(${channel.avatarHue} 62% 50%)`,
        ...(size !== 38 ? { width: size, height: size, fontSize: Math.round(size * 0.34) } : null),
      }}
    >
      {channel.name.slice(0, 1)}
      {channel.photoUrl && stan !== "blad" && (
        <img
          className="chan__avatarimg"
          src={channel.photoUrl}
          alt=""
          // Lista ma 200+ pozycji. Bez `lazy` przeglądarka zamówiłaby
          // WSZYSTKIE miniatury naraz, także te kilkaset pikseli poza ekranem.
          loading="lazy"
          decoding="async"
          draggable={false}
          data-ok={stan === "jest"}
          onLoad={() => setStan("jest")}
          onError={() => setStan("blad")}
        />
      )}
    </span>
  );
}

/* ============================================================
   MODAL FORMATÓW (i tematów dla grup-forów)
   ============================================================ */
function FormatModal({ channel, onClose }: { channel: TelegramChannel | null; onClose: () => void }) {
  const app = useApp();
  const t = useT();
  if (!channel) return null;
  // Kanał pobrany z konta nie ma jeszcze powiązania, dopóki użytkownik go nie
  // zaznaczy. Bez tej wartości zastępczej otwarcie okna kończyło się wyjątkiem
  // i pustym ekranem.
  const b: ChannelBinding = app.bindings[channel.id] ?? {
    channelId: channel.id,
    monitored: false,
    notify: false,
    format: BEZ_FORMATU,
    topics: {},
  };

  /* JEDEN format na kanał i JEDEN na temat forum.
     Wcześniej były to listy — nikt ich nie czytał, ale gdyby zaczęły działać,
     dwa formaty na jednym kanale znaczyłyby, że ta sama wiadomość rodzi dwa
     koszyki, z dwóch parserów, zarządzane dwoma presetami, na jednym
     rachunku. Podwójna ekspozycja z jednego sygnału, bez śladu w logach. */
  const ustawFormat = (nazwa: string) => app.setBinding(channel.id, { format: nazwa });

  const ustawFormatTematu = (id: number, nazwa: string) => {
    const topics = { ...b.topics };
    if (nazwa) topics[id] = nazwa;
    else delete topics[id];
    app.setBinding(channel.id, { topics });
  };

  /** Lista formatów + wpis „nie handluj", z informacją, czym każdy z nich gra. */
  const opcje = [
    { value: BEZ_FORMATU, label: t("chan.opt.none") },
    ...app.formaty.map((f) => {
      const p = presetDlaFormatu(app.lancuch, f.nazwa);
      return {
        value: f.nazwa,
        label: `${f.nazwa} — ${p ? t("chan.opt.preset", { v: p }) : t("chan.opt.idle")}`,
      };
    }),
  ];

  const grajacychTematow = Object.values(b.topics).filter((f) => f).length;

  return (
    <Modal
      open
      onClose={onClose}
      width={640}
      title={
        <span className="row row--tight">
          <Icon name="filter" size={16} />
          {t("chan.modal.title")}
        </span>
      }
      subtitle={channel.isForum ? t("chan.modal.forum") : t("chan.modal.plain")}
      footer={
        <Button variant="primary" icon="check" onClick={onClose}>
          {t("chan.modal.done")}
        </Button>
      }
    >
      <div className="row row--tight" style={{ marginBottom: "var(--sp-3)" }}>
        <ChannelAvatar channel={channel} size={30} />
        <b>{channel.name}</b>
        <span className="hint">{channel.handle}</span>
      </div>

      {!channel.isForum ? (
        <div className="setgrid">
          <div className="setfield setfield--wide">
            <label className="setfield__label">{t("chan.modal.fieldLabel")}</label>
            <Select value={b.format} onChange={ustawFormat} options={opcje} />
            <span className="setfield__hint">
              {b.format ? (
                <RichT k="chan.modal.hintTrading" vars={{ format: b.format, chain: app.aktywnaNazwaLancucha }} />
              ) : (
                <RichT k="chan.modal.hintIdle" />
              )}
            </span>
          </div>
          {app.formaty
            .filter((f) => f.nazwa === b.format)
            .map((f) => (
              <div key={f.nazwa} className="setfield setfield--wide">
                <span className="fmt__desc">{opisFormatu(f.nazwa, f.opis)}</span>
                {f.przyklad && <pre className="fmt__sample">{f.przyklad}</pre>}
              </div>
            ))}
        </div>
      ) : (
        <div className="col">
          <p className="hint">{t("chan.modal.topicsCount", { n: grajacychTematow, all: channel.topics.length })}</p>
          {channel.topics.map((temat) => {
            const wybrany = b.topics[temat.id] ?? BEZ_FORMATU;
            return (
              <div
                key={temat.id}
                className="topic"
                data-on={!!wybrany}
                style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}
              >
                <div className="row row--tight">
                  <span style={{ fontSize: 15 }}>{temat.icon}</span>
                  <span className="topic__name">{temat.name}</span>
                  <span className="spacer" />
                  {wybrany ? (
                    <Badge tone="accent">{wybrany}</Badge>
                  ) : (
                    <Badge tone="muted">{t("chan.stat.notTrading")}</Badge>
                  )}
                </div>
                <Select value={wybrany} onChange={(v) => ustawFormatTematu(temat.id, v)} options={opcje} size="sm" />
              </div>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
