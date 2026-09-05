import { tSilnik, maKlucz } from "@/i18n/silnik";
import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Card, Checkbox, Empty, Field, Icon, NumberInput, Select, Switch, TextInput } from "@/components/ui";
import { EksportPelny } from "@/components/panels/ExportPanel";
import { useApp } from "@/store/AppStore";
import { api } from "@/store/transport";
import { MERGE_KEYS } from "@/data/defaultSettings";
import { duration, time } from "@/lib/format";
import { RichT, t, useT, useLanguage } from "@/i18n";
import type { LogCategory } from "@/types";
import "./views.css";

/** Czas do pokazania na pasku; 0 znaczy „jeszcze nie wiadomo", nie „zero sekund". */
const czasKrotki = (ms: number) => (ms <= 0 ? "—" : duration(ms));

/** Worek na pola, którym `MERGE_KEYS` nie nadaje grupy. To SENTINEL do
 *  grupowania, a nie napis — nagłówek grupy bierze się ze słownika, więc
 *  wartość nie może być tłumaczona (inaczej zmiana języka rozbiłaby klucz
 *  Reacta i podział na grupy). */
const GRUPA_INNE = "__inne__";


function PostepScalaniaPasek() {
  const app = useApp();
  const tt = useT();
  const s = app.scalanie;

  // Nigdy nie uruchamiane — nie zajmujemy miejsca pustym paskiem.
  if (!s.faza) return null;

  const pct = Math.round(s.postep * 1000) / 10;
  const kolor =
    s.faza === "blad"
      ? "var(--short)"
      : s.faza === "anulowane"
        ? "var(--warn)"
        : s.faza === "gotowe"
          ? "var(--long)"
          : "var(--accent)";
  const mb = (b: number) => (b / 1048576).toFixed(1);

  return (
    <div style={{ marginBottom: "var(--sp-3)" }}>
      <div className="lab__bar">
        <div className="meter" style={{ height: 8 }}>
          <div className="meter__fill" style={{ width: `${Math.max(1, pct)}%`, background: kolor }} />
        </div>
        <b className="num lab__pct">{pct.toFixed(1)}%</b>
      </div>

      <div className="lab__now">
        {s.aktywne && <span className="lab__spin" />}
        <span className="truncate">
          {s.faza === "blad"
            ? /* powód z silnika przychodzi w jego brzmieniu — tłumaczymy tylko
                 zastępnik, gdy go nie podał */
              (tSilnik(s.blad) || tt("logs.merge.failed"))
            : s.faza === "anulowane"
              ? tt("logs.merge.cancelled")
              : tSilnik(s.etap)}
        </span>
        {s.aktywne && (
          <Button
            size="sm"
            variant="danger"
            icon="x"
            onClick={() => void api.scalAnuluj().catch(() => undefined)}
          >
            {tt("common.cancel")}
          </Button>
        )}
      </div>

      <div className="lab__metrics">
        <div className="lab__metric">
          <b className="num">
            {mb(s.zrobione)} / {mb(s.wszystkich)}
          </b>
          <span>{tt("logs.merge.mb")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{mb(Math.max(0, s.wszystkich - s.zrobione))}</b>
          <span>{tt("logs.merge.mbLeft")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{s.predkosc || "—"}</b>
          <span>{tt("logs.merge.speed")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{s.aktywne ? czasKrotki(s.etaMs) : "—"}</b>
          <span>{tt("logs.merge.eta")}</span>
        </div>
        <div className="lab__metric">
          <b className="num">{czasKrotki(s.czasMs)}</b>
          <span>{tt("logs.merge.elapsed")}</span>
        </div>
      </div>

      {}
      {s.faza === "gotowe" && s.plik && (
        <div className="hint" style={{ marginTop: "var(--sp-2)" }}>
          <b>{s.plik}</b> · {(s.znakow / 1048576).toFixed(1)} MB
          {!!s.zrodel && ` · ${tt("logs.merge.sourcesN", { n: s.zrodel })}`}
          {!!s.pominietych && ` · ${tt("logs.merge.skippedN", { n: s.pominietych })}`}
          {s.czasMs > 0 && ` · ${czasKrotki(s.czasMs)}`}
          <br />
          <span className="truncate mono">{s.sciezka}</span>
          <div className="row row--tight" style={{ marginTop: "var(--sp-2)" }}>
            {panelObokBota() ? (
              <Button
                size="sm"
                variant="outline"
                icon="grid"
                onClick={() => void api.pokazPlik(s.sciezka).catch(() => undefined)}
              >
                {tt("logs.showFile")}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="outline"
                icon="copy"
                onClick={() => void navigator.clipboard?.writeText(s.sciezka).catch(() => undefined)}
              >
                {tt("logs.copyPath")}
              </Button>
            )}
            {!panelObokBota() && <span className="hint">{tt("logs.remote.note")}</span>}
          </div>
        </div>
      )}
    </div>
  );
}


function WyborKatalogu({ onWybierz, onZamknij }: { onWybierz: (p: string) => void; onZamknij: () => void }) {
  const tt = useT();
  const [dane, setDane] = useState<{ path: string; parent: string | null; dirs: { name: string; path: string }[] } | null>(null);
  const [blad, setBlad] = useState<string | null>(null);

  const wczytaj = (path?: string) => {
    setBlad(null);
    api
      .fsDirs(path)
      .then(setDane)
      .catch((e) => setBlad(String(e)));
  };
  // pierwsze otwarcie: katalog logs bota
  useState(() => {
    wczytaj();
    return 0;
  });

  return (
    <div className="modal-scrim" role="dialog" onClick={onZamknij}>
      <div className="modal" style={{ maxWidth: 520 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal__body">
        <div className="row" style={{ marginBottom: "var(--sp-2)" }}>
          <b>{tt("logs.dirPick.title")}</b>
          <span className="spacer" />
          <Button size="sm" variant="ghost" icon="x" onClick={onZamknij} />
        </div>
        {blad && <p className="hint" style={{ color: "var(--short-text)" }}>{tSilnik(blad)}</p>}
        {dane && (
          <>
            <p className="hint truncate" style={{ marginBottom: "var(--sp-2)" }}>
              {dane.path}
            </p>
            <div className="row row--tight" style={{ marginBottom: "var(--sp-2)" }}>
              <Button
                size="sm"
                variant="outline"
                icon="arrow-up"
                /* „NAPĘDY" to STAŁA PROTOKOŁU `/api/fs/dirs` (rest.rs), nie napis
                   dla oka — tłumaczenie jej rozsypałoby listę dysków. */
                onClick={() => wczytaj(dane.parent ?? "NAPĘDY")}
              >
                {tt("logs.dirPick.up")}
              </Button>
              <span className="spacer" />
              <Button size="sm" variant="primary" icon="check" onClick={() => onWybierz(dane.path)}>
                {tt("logs.dirPick.pick")}
              </Button>
            </div>
            <div style={{ maxHeight: 300, overflowY: "auto", display: "grid", gap: 2 }}>
              {dane.dirs.length === 0 && <span className="hint">{tt("logs.dirPick.empty")}</span>}
              {dane.dirs.map((d) => (
                <button
                  key={d.path}
                  className="settings__navitem"
                  onClick={() => wczytaj(d.path)}
                  title={d.path}
                >
                  <Icon name="chevron-right" size={12} />
                  <span className="truncate">{d.name}</span>
                </button>
              ))}
            </div>
          </>
        )}
        </div>
      </div>
    </div>
  );
}

const CAT_TONE: Partial<Record<string, "muted" | "accent" | "long" | "short" | "warn" | "info">> = {
  commands: "accent",
  events: "info",
  messages: "muted",
  signals: "accent",
  trades: "long",
  unpredicted_signals: "warn",
  signal_formats: "muted",
  backup_memory: "muted",
  poll_interval: "muted",
  update_performance: "muted",
  price_log: "muted",
  session_string: "short",
  smtp: "short",
  kronika: "info",
  journal: "accent",
  wiadomosci: "muted",
  telegram: "info",
  settings: "muted",
  email: "muted",
  trade: "long",
  backtests: "muted",
  logs: "muted",
};


const panelObokBota = () => {
  const h = window.location.hostname;
  return h === "localhost" || h === "127.0.0.1" || h === "::1" || h === "" || h === "[::1]";
};


function useChmurkaPoScaleniu() {
  const app = useApp();
  const s = app.scalanie;
  const poprzednia = useRef<string>("");

  useEffect(() => {
    const byla = poprzednia.current;
    poprzednia.current = s.faza;
    // reagujemy WYŁĄCZNIE na przejście „trwa" → „gotowe"/„blad".
    // Bez tego warunku chmurka wracałaby przy każdym przeładowaniu panelu,
    // bo faza „gotowe" zostaje w migawce stanu do następnego scalania.
    if (byla !== "trwa") return;

    if (s.faza === "blad") {
      app.toast("error", t("logs.toast.mergeFailed"), s.blad ?? t("logs.toast.noReason"));
      return;
    }
    if (s.faza !== "gotowe" || !s.plik) return;

    const rozmiarMB = (s.znakow / 1048576).toFixed(1);
    const zrodla = s.zrodel ? t("logs.merge.sourcesN", { n: s.zrodel }) : "";
    const pominiete = s.pominietych ? `, ${t("logs.merge.skippedN", { n: s.pominietych })}` : "";
    const opis = [`${rozmiarMB} MB`, zrodla].filter(Boolean).join(" · ") + pominiete;

    const pokazPlik = {
      label: t("logs.showFile"),
      onClick: () => {
        void api
          .pokazPlik(s.sciezka)
          .catch((e) => app.toast("warn", t("logs.toast.explorerFail"), String(e)));
      },
    };
    const kopiujSciezke = {
      label: t("logs.copyPath"),
      onClick: () => {
        void navigator.clipboard
          ?.writeText(s.sciezka)
          .then(() => app.toast("info", t("logs.toast.pathCopied"), s.sciezka))
          .catch(() => app.toast("warn", t("logs.toast.clipboardOff"), s.sciezka));
      },
    };
    const kopiujPlik = {
      label: t("logs.copyFile"),
      onClick: () => {
        void api
          .kopiujPlik(s.sciezka)
          .then((r) => app.toast("success", t("logs.toast.onClipboard"), t("logs.toast.pasteCtrlV", { p: r.path })))
          .catch((e) => app.toast("warn", t("logs.toast.copyFileFail"), String(e)));
      },
    };
    const pobierzPlik = {
      label: t("logs.downloadFile"),
      onClick: () => {
        // POBIERAMY PLIK, nie kopiujemy treści.
        //
        // Schowek przeglądarki przyjmuje TEKST, a nie plik — wklejenie
        // 9 MB dziennika jako tekstu jest bezużyteczne, bo nie da się go
        // podesłać jako załącznik. Zamiast tego robimy z treści Blob
        // i wymuszamy pobranie: użytkownik dostaje PRAWDZIWY plik
        // w katalogu pobierania, gotowy do wysłania.
        void api
          .czytajPlik(s.sciezka)
          .then((r) => {
            const nazwa = r.path.split(/[\/]/).pop() || "alllogs.txt";
            const url = URL.createObjectURL(
              new Blob([r.text], { type: "text/plain;charset=utf-8" }),
            );
            const a = document.createElement("a");
            a.href = url;
            a.download = nazwa;
            document.body.appendChild(a);
            a.click();
            a.remove();
            // Zwolnienie z opóźnieniem — natychmiastowe unieważnia
            // adres, zanim przeglądarka zdąży zacząć pobieranie.
            setTimeout(() => URL.revokeObjectURL(url), 60_000);
            app.toast(
              "success",
              t("logs.toast.downloaded", { n: nazwa }),
              t("logs.toast.downloadedText", { mb: (r.bytes / 1048576).toFixed(1) }),
            );
          })
          .catch((e) => app.toast("warn", t("logs.toast.downloadFail"), String(e)));
      },
    };

    const actions = panelObokBota()
      ? [pokazPlik, kopiujPlik, pobierzPlik]
      : [kopiujSciezke, kopiujPlik, pobierzPlik];

    app.toast("success", t("logs.toast.merged", { plik: s.plik }), `${opis}\n${s.sciezka}`, { actions });
    // `app` zmienia tożsamość co render store'u — zależność po samej fazie
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [s.faza, s.plik, s.sciezka, s.znakow, s.zrodel, s.pominietych, s.blad]);
}

export function LogsView() {
  const { lang } = useLanguage();
  const app = useApp();
  const tt = useT();
  const [q, setQ] = useState("");
  const [pokazKatalogi, setPokazKatalogi] = useState(false);
  const [cat, setCat] = useState("all");
  const [showPass, setShowPass] = useState(false);
  /** Hasło SMTP: stan LOKALNY. Nigdy nie wiązać z `app.email.pass` — patrz komentarz przy polu. */
  const [haslo, setHaslo] = useState("");
  useChmurkaPoScaleniu();

  const filtered = useMemo(
    () =>
      app.logs.filter(
        (l) =>
          (cat === "all" || l.category === cat) &&
          (q === "" || (l.title + l.content + tSilnik(l.title) + (["messages", "wiadomosci"].includes(l.category) ? "" : tSilnik(l.content))).toLowerCase().includes(q.toLowerCase())),
      ),
    [app.logs, q, cat, lang],
  );

  const setMerge = (k: LogCategory, v: boolean) =>
    app.setSettings({ merge_config: { ...app.settings.merge_config, [k]: v } });

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{tt("logs.title")}</h1>
          <p>{tt("logs.lead")}</p>
        </div>
        <Badge tone="muted">{tt("logs.inMemory", { n: app.logs.length })}</Badge>
      </div>

      <Card
        title={tt("logs.journal")}
        icon="logs"
        subtitle={`${filtered.length}`}
        accent="var(--accent)"
        flush
        actions={
          <>
            <TextInput value={q} onChange={setQ} placeholder={tt("logs.search")} icon="search" size="sm" style={{ width: 210 }} />
            <Select
              value={cat}
              onChange={setCat}
              size="sm"
              options={[
                { value: "all", label: tt("logs.allCats") },
                /* etykiety kategorii przychodzą z `data/defaultSettings.ts`
                   (MERGE_KEYS) i są tam na razie po polsku — to plik danych
                   poza tym widokiem */
                ...MERGE_KEYS.map((k) => ({ value: k.key, label: tt(`logs.source.${k.key}.label`) })),
              ]}
            />
            <Button size="sm" variant="danger" icon="trash" onClick={app.clearLogs}>
              {tt("logs.clear")}
            </Button>
          </>
        }
      >
        {filtered.length === 0 ? (
          <Empty icon="logs" title={tt("logs.empty.title")} text={tt("logs.empty.text")} />
        ) : (
          <div className="logrows">
            {filtered.map((l) => (
              <div key={l.id} className={`logrow logrow--${l.level}`}>
                <span className="logrow__t">{time(l.t)}</span>
                <span className="logrow__cat">
                  {/* `?? "muted"` NIE jest ozdobą: silnik loguje pod
                      kategoriami, których panel nie zna (`telegram`,
                      `kronika`, `backtests`…), a brak wpisu w mapie dawał
                      `tone={undefined}` i kategorię bez tła. */}
                  <Badge tone={CAT_TONE[l.category] ?? "muted"} title={l.category}>{maKlucz(`logs.source.${l.category}.label`) ? tt(`logs.source.${l.category}.label`) : maKlucz(`logs.category.${l.category}`) ? tt(`logs.category.${l.category}`) : l.category}</Badge>
                </span>
                <div className="logrow__body">
                  <span className="logrow__title" title={l.title}>{tSilnik(l.title)}</span>
                  {l.content && <span className="logrow__content" title={l.content}>{["messages", "wiadomosci"].includes(l.category) ? l.content : tSilnik(l.content)}</span>}
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      <EksportPelny />

      <Card
        title={tt("logs.merge.title")}
        icon="download"
        accent="var(--info)"
        subtitle={tt("logs.merge.subtitle", { n: app.mergedLogCount })}
        actions={
          <>
            <Button
              size="sm"
              variant="outline"
              icon="download"
              onClick={() => app.scalLogi()}
            >
              {tt("logs.merge.run")}
            </Button>
          </>
        }
      >
        <PostepScalaniaPasek />

        <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
          <RichT k="logs.merge.lead" />
        </p>

        {/* GRUPY, nie jedna płaska lista: po dołożeniu źródeł plikowych
            (kronika, archiwum, backup_memory, koszyki, presety…) pól jest
            ponad dwadzieścia i bez podziału nie widać, że najważniejsze
            cztery są na samej górze. */}
        {[...new Set(MERGE_KEYS.map((k) => k.grupa ?? GRUPA_INNE))].map((grupa) => (
          <div key={grupa} style={{ marginBottom: "var(--sp-3)" }}>
            <p className="hint" style={{ marginBottom: "var(--sp-1)", fontWeight: 600 }}>
              {grupa === GRUPA_INNE ? tt("logs.merge.groupOther") : tSilnik(grupa)}
            </p>
            <div className="mergegrid">
              {MERGE_KEYS.filter((k) => (k.grupa ?? GRUPA_INNE) === grupa).map((k) => (
                <label
                  key={k.key}
                  className="mergeitem"
                  data-sensitive={!!k.sensitive}
                  style={k.martwe ? { opacity: 0.55 } : undefined}
                  title={k.martwe ? tt("logs.merge.dead") : undefined}
                >
                  <Checkbox
                    checked={app.settings.merge_config[k.key] ?? !k.sensitive}
                    onChange={(v) => setMerge(k.key, v)}
                    disabled={k.martwe}
                  />
                  <span className="mergeitem__body">
                    <span className="mergeitem__name">
                      {tt(`logs.source.${k.key}.label`)}
                      {k.sensitive && " ⚠️"}
                    </span>
                    <span className="mergeitem__note">{tt(`logs.source.${k.key}.note`)}</span>
                  </span>
                </label>
              ))}
            </div>
          </div>
        ))}

        <div className="divider" />

        {/* KATALOG DOCELOWY: puste = logs bota. Ścieżka SERWERA — stąd modal
            /api/fs/dirs zamiast natywnego okna (panel bywa na innej maszynie).
            Walidacja plikiem-sondą przy zapisie i przy starcie scalania. */}
        {/* PRZYCISKI WYRÓWNANE DO POLA, NIE DO DNA WIERSZA.
            Przy `alignItems: flex-end` wysokość wiersza ustawiała PODPOWIEDŹ
            pod polem — a ta pojawia się i znika zależnie od tego, czy katalog
            jest wpisany. Przyciski skakały w pionie przy pierwszym wpisanym
            znaku. Podpowiedź wychodzi więc pod wiersz: pole i przyciski
            trzymają jedną linię niezależnie od jej obecności. */}
        <div className="row row--tight" style={{ alignItems: "flex-end" }}>
          <div style={{ flex: 1 }}>
            <Field label={tt("logs.merge.dir")}>
              <TextInput
                value={app.settings.alllogs_dir}
                onChange={(v) => app.setSetting("alllogs_dir", v)}
                placeholder={tt("logs.merge.dirPh")}
              />
            </Field>
          </div>
          <Button size="sm" variant="outline" icon="grid" onClick={() => setPokazKatalogi(true)}>
            {tt("logs.browse")}
          </Button>
          {app.settings.alllogs_dir && (
            <Button size="sm" variant="ghost" icon="refresh" onClick={() => app.setSetting("alllogs_dir", "")}>
              {tt("logs.default")}
            </Button>
          )}
        </div>
        {!app.settings.alllogs_dir && (
          <p className="hint" style={{ marginBottom: "var(--sp-2)" }}>
            {tt("logs.merge.dirEmpty")}
          </p>
        )}
        {pokazKatalogi && (
          <WyborKatalogu
            onWybierz={(p) => {
              app.setSetting("alllogs_dir", p);
              setPokazKatalogi(false);
            }}
            onZamknij={() => setPokazKatalogi(false)}
          />
        )}

        <div className="row">
          <Switch
            checked={app.settings.merge_chronological}
            onChange={(v) => app.setSetting("merge_chronological", v)}
            label={tt("logs.merge.chrono")}
          />
          <span className="spacer" />
          <Switch
            checked={app.settings.price_log}
            onChange={(v) => app.setSetting("price_log", v)}
            label={tt("logs.priceLog")}
          />
          <NumberInput
            value={app.settings.price_log_interval_s}
            onChange={(v) => app.setSetting("price_log_interval_s", v)}
            step={1}
            min={1}
            unit="s"
            size="sm"
            style={{ width: 88 }}
          />
        </div>

        <p className="hint" style={{ marginTop: "var(--sp-2)", color: "var(--warn-text)", display: "flex", gap: 6 }}>
          <Icon name="alert" size={12} style={{ flex: "none", marginTop: 2 }} />
          {tt("logs.merge.secrets")}
        </p>
      </Card>

      <div className="dash">
        <div className="dash__main">
          <Card title={tt("logs.mail.title")} icon="mail" accent="var(--long)">
            <div className="formgrid">
              <Field label={tt("logs.mail.send")}>
                <Switch
                  checked={app.email.enabled}
                  onChange={(v) => app.setEmail({ ...app.email, enabled: v })}
                  label={app.email.enabled ? tt("logs.mail.on") : tt("logs.mail.off")}
                />
              </Field>
              <Field label={tt("logs.mail.to")}>
                <TextInput value={app.email.to} onChange={(v) => app.setEmail({ ...app.email, to: v })} />
              </Field>
              <Field label={tt("logs.mail.every")} hint={tt("logs.mail.everyHint")}>
                <NumberInput
                  value={app.email.intervalMin}
                  onChange={(v) => app.setEmail({ ...app.email, intervalMin: v })}
                  step={15}
                  min={1}
                  unit="min"
                />
              </Field>
              <Field label={tt("logs.mail.host")}>
                <TextInput value={app.email.host} onChange={(v) => app.setEmail({ ...app.email, host: v })} />
              </Field>
              <Field label={tt("logs.mail.port")}>
                <NumberInput value={app.email.port} onChange={(v) => app.setEmail({ ...app.email, port: v })} step={1} />
              </Field>
              <Field label={tt("logs.mail.user")}>
                <TextInput
                  value={app.email.user}
                  onChange={(v) => app.setEmail({ ...app.email, user: v })}
                  placeholder={tt("logs.mail.userPh")}
                />
              </Field>
              {/* HASŁO MA WŁASNY STAN LOKALNY I ZAPISUJE SIĘ DOPIERO PRZYCISKIEM.
                  Wcześniej było wiązane wprost z `app.email.pass`, czyli
                  z migawką serwera — a serwer ZAWSZE zwraca puste hasło, bo
                  trzyma je w `secrets.json`, nie w stanie. Skutki były dwa
                  i oba paskudne: (1) każdy wpisany znak natychmiast znikał,
                  więc pola „nie dało się edytować"; (2) każde naciśnięcie
                  klawisza wysyłało `setEmail` z hasłem długości JEDNEGO znaku,
                  a serwer zapisuje każde niepuste hasło — więc próba wpisania
                  „abc" KASOWAŁA działające hasło aplikacji i zostawiała „c".
                  Puste pole nadal znaczy „zostaw zapisane". */}
              <Field label={tt("logs.mail.pass")}>
                <div className="row row--tight" style={{ flexWrap: "nowrap" }}>
                  <TextInput
                    value={haslo}
                    onChange={setHaslo}
                    type={showPass ? "text" : "password"}
                    placeholder={tt("logs.mail.passPh")}
                  />
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={showPass ? "eye-off" : "eye"}
                    onClick={() => setShowPass((s) => !s)}
                    title={tt("logs.mail.passToggle")}
                  />
                  <Button
                    size="sm"
                    variant="outline"
                    icon="check"
                    disabled={!haslo}
                    onClick={() => {
                      app.setEmail({ ...app.email, pass: haslo });
                      setHaslo("");
                      app.toast("success", tt("logs.mail.passSaved"), tt("logs.mail.passSavedText"));
                    }}
                    title={tt("logs.mail.passSave")}
                  />
                </div>
              </Field>
            </div>

            <div className="divider" />

            <div className="row">
              {}
              <Button variant="outline" icon="send" onClick={() => app.sendTestEmail()}>
                {tt("logs.sendTest")}
              </Button>
              <span className="hint">{tt("logs.mail.alerts")}</span>
            </div>
          </Card>
        </div>

        <aside className="dash__side">
          <Card title={tt("logs.tg.title")} icon="bell" accent="var(--info)">
            <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
              {tt("logs.tg.lead")}
            </p>

            {}
            {!app.channelsAreReal ? (
              <p className="hint">
                {app.channelsLoading
                  ? tt("logs.tg.loading")
                  : /* powód z serwera przychodzi w jego brzmieniu */
                    (app.channelsError ?? tt("logs.tg.noChats"))}
              </p>
            ) : (
              <div className="col" style={{ maxHeight: 260, overflowY: "auto" }}>
                {app.channels
                  // najpierw już zaznaczone, potem reszta — przy 210 czatach
                  // inaczej nie da się sprawdzić, co jest włączone
                  .filter((c) => app.bindings[c.id]?.notify || c.kind !== "user")
                  .sort(
                    (a, b) =>
                      Number(app.bindings[b.id]?.notify ?? false) -
                      Number(app.bindings[a.id]?.notify ?? false),
                  )
                  .map((c) => (
                    <Checkbox
                      key={c.id}
                      checked={app.bindings[c.id]?.notify ?? false}
                      onChange={(v) => app.setBinding(c.id, { notify: v })}
                      label={<span className="truncate">{c.name}</span>}
                    />
                  ))}
              </div>
            )}

            <div className="divider" />

            <div className="col">
              <Switch
                checked={app.notify.summaryEnabled}
                onChange={(v) => app.setNotify({ ...app.notify, summaryEnabled: v })}
                label={tt("logs.tg.summaries")}
              />
              <Field label={tt("logs.tg.everyMin")} hint={tt("logs.tg.everyMinHint")}>
                <NumberInput
                  value={app.notify.summaryIntervalMin}
                  onChange={(v) => app.setNotify({ ...app.notify, summaryIntervalMin: v })}
                  step={30}
                  min={1}
                  unit="min"
                />
              </Field>
              <div className="row row--tight">
                {}
                <Button
                  variant="outline"
                  icon="send"
                  onClick={() => app.wyslijTestPowiadomienia()}
                >
                  {tt("logs.sendTest")}
                </Button>
              </div>
            </div>
          </Card>
        </aside>
      </div>
    </div>
  );
}
