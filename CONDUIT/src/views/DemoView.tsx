import { tSilnik } from "@/i18n/silnik";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  Empty,
  Field,
  Icon,
  NumberInput,
  Segmented,
  Select,
  TextInput,
} from "@/components/ui";
import { DOMYSLNE_DEMO, useApp } from "@/store/AppStore";
import { api, type DemoCandidate, type DemoConfig, type DemoScan } from "@/store/transport";
import { duration, num, locale } from "@/lib/format";
import { useT, RichT } from "@/i18n";
import "./views.css";
import "./demo.css";

/* ============================================================
   TRYB DEMO

   Bot gra NAPRAWDĘ, tylko na wirtualnym brokerze: ten sam silnik i ten sam
   symulator, co w backteście, różni je wyłącznie to, że zegar płynie
   w czasie rzeczywistym (albo w zadanej wielokrotności).

   Ten widok NICZEGO nie liczy — składa zlecenie, wysyła je REST-em i pokazuje
   stan, który przychodzi sekcją `demo` snapshotu. Gdyby liczył cokolwiek sam,
   demo rozjechałoby się z backtestem, a to jest dokładnie ten rodzaj różnicy,
   którego nie da się zauważyć w porę.
   ============================================================ */

/** Tempa odtwarzania. Same mnożniki są językowo neutralne, tłumaczenia
 *  wymaga tylko „MAKS” — stąd funkcja, a nie stała modułu. */
function tempaOpcje(t: (k: string) => string) {
  return [
    { value: "1", label: "1×" },
    { value: "10", label: "10×" },
    { value: "60", label: "60×" },
    { value: "600", label: "600×" },
    { value: "0", label: t("demo.speed.maxShort") },
  ];
}

/* Fazy przebiegu: `label` to KLUCZ SŁOWNIKA (`t(f.label)`), ton zostaje
   w kodzie, bo to wygląd, nie tekst. */
const FAZA: Record<string, { label: string; tone: "accent" | "long" | "warn" | "short" }> = {
  idle: { label: "demo.phase.idle", tone: "accent" },
  running: { label: "demo.phase.running", tone: "accent" },
  finished: { label: "demo.phase.finished", tone: "long" },
  stopped: { label: "demo.phase.stopped", tone: "warn" },
  failed: { label: "demo.phase.failed", tone: "short" },
};

/** Rodzaj rozpoznanego pliku — klucze słownika po kodzie z sniffera. */
const RODZAJ: Record<string, string> = {
  ticksBin: "demo.kind.ticksBin",
  ticksCsv: "demo.kind.ticksCsv",
  signalsJson: "demo.kind.signalsJson",
  telegramJson: "demo.kind.telegramJson",
  telegramHtml: "demo.kind.telegramHtml",
};

/** Zegar pliku — klucze słownika po kodzie z sniffera. */
const ZEGAR: Record<string, string> = {
  server: "demo.clock.server",
  utc: "demo.clock.utc",
  other: "demo.clock.other",
  unknown: "demo.clock.unknown",
};

function mb(b: number): string {
  if (b >= 1 << 30) return `${(b / (1 << 30)).toFixed(2)} GB`;
  if (b >= 1 << 20) return `${(b / (1 << 20)).toFixed(1)} MB`;
  if (b >= 1 << 10) return `${(b / (1 << 10)).toFixed(0)} kB`;
  return `${b} B`;
}

function kwota(v: number): string {
  return `${v >= 0 ? "+" : "−"}${Math.abs(v).toLocaleString(locale(), {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })} $`;
}

/* ============================================================
   LISTA ZNALEZIONYCH PLIKÓW
   ============================================================ */

function Kandydat({
  c,
  wybrany,
  onWybierz,
}: {
  c: DemoCandidate;
  wybrany: boolean;
  onWybierz: (c: DemoCandidate) => void;
}) {
  const t = useT();
  return (
    <button
      type="button"
      className={`demo__cand ${wybrany ? "demo__cand--on" : ""} ${c.usable ? "" : "demo__cand--dim"}`}
      onClick={() => onWybierz(c)}
      title={c.path}
    >
      <div className="demo__candtop">
        <Icon name={c.role === "ticks" ? "bars" : "telegram"} size={13} />
        <b className="truncate">{c.name}</b>
        <Badge tone={c.role === "ticks" ? "accent" : "info"}>{RODZAJ[c.kind] ? t(RODZAJ[c.kind]) : c.kind}</Badge>
        {!c.usable && (
          <Badge tone="warn" title={tSilnik(c.note)}>
            {t("demo.unusable")}
          </Badge>
        )}
        <span className="spacer" />
        <span className="hint num">{mb(c.bytes)}</span>
      </div>
      <div className="demo__candpath truncate">{c.path}</div>
      <div className="demo__candmeta">
        <span className="num">
          {c.exact ? "" : "≈ "}
          {num(c.records, 0)} {c.role === "ticks" ? t("demo.ticks") : t("demo.messages")}
        </span>
        <span className="hint">
          {c.firstDay} … {c.lastDay}
        </span>
        {c.clock && c.clock !== "unknown" && (
          <span className="hint">{t("demo.clockOf", { v: ZEGAR[c.clock] ? t(ZEGAR[c.clock]) : c.clock })}</span>
        )}
      </div>
      {c.note && <div className="demo__candnote">{tSilnik(c.note)}</div>}
    </button>
  );
}

/* ============================================================
   POLE PLIKU: przeciągnij ALBO wpisz ścieżkę
   ============================================================ */

function PolePliku({
  label,
  hint,
  value,
  onChange,
  onRozpoznano,
  onBlad,
}: {
  label: string;
  hint: string;
  value: string;
  onChange: (v: string) => void;
  onRozpoznano: (c: DemoCandidate) => void;
  onBlad: (msg: string) => void;
}) {
  const t = useT();
  const [nad, setNad] = useState(false);
  const [szuka, setSzuka] = useState(false);

  const rozpoznaj = useCallback(
    async (path: string) => {
      setSzuka(true);
      try {
        onRozpoznano(await api.demoInspect(path));
      } catch (e) {
        onBlad(String(e).replace(/^Error:\s*/, ""));
      } finally {
        setSzuka(false);
      }
    },
    [onBlad, onRozpoznano],
  );

  /* Przeglądarka NIE podaje ścieżki upuszczonego pliku (i słusznie). Mamy więc
     trzy drogi, po kolei: ścieżkę doklejaną przez powłokę natywną, tekst
     upuszczony z eksploratora, a na końcu wyszukanie po nazwie i rozmiarze
     wśród pobliskich katalogów. Ta ostatnia trafia w praktyce zawsze, bo plik
     leży w tym samym drzewie co program. */
  const upuszczono = async (e: React.DragEvent) => {
    e.preventDefault();
    setNad(false);
    const tekst = e.dataTransfer.getData("text/plain")?.trim();
    const pliki = Array.from(e.dataTransfer.files ?? []);

    if (pliki.length > 0) {
      const f = pliki[0];
      const zPowloki = (f as File & { path?: string }).path;
      if (zPowloki) {
        onChange(zPowloki);
        void rozpoznaj(zPowloki);
        return;
      }
      setSzuka(true);
      try {
        const r = await api.demoLocate(f.name, f.size);
        if (r.ok && r.matches.length > 0) {
          onChange(r.matches[0].path);
          onRozpoznano(r.matches[0]);
        } else {
          onBlad(t("demo.drop.notFound", { name: f.name, size: mb(f.size) }));
        }
      } catch (err) {
        onBlad(String(err).replace(/^Error:\s*/, ""));
      } finally {
        setSzuka(false);
      }
      return;
    }

    if (tekst) {
      onChange(tekst);
      void rozpoznaj(tekst);
    }
  };

  return (
    <Field label={label} hint={hint}>
      <div
        className={`demo__drop ${nad ? "demo__drop--over" : ""}`}
        onDragOver={(e) => {
          e.preventDefault();
          setNad(true);
        }}
        onDragLeave={() => setNad(false)}
        onDrop={(e) => void upuszczono(e)}
      >
        <TextInput
          value={value}
          onChange={onChange}
          placeholder={t("demo.drop.placeholder")}
          icon="clipboard"
        />
        <Button
          size="sm"
          variant="outline"
          icon={szuka ? "hourglass" : "search"}
          disabled={!value.trim() || szuka}
          onClick={() => void rozpoznaj(value.trim())}
        >
          {t("demo.identify")}
        </Button>
      </div>
    </Field>
  );
}

/* ============================================================
   RĘCZNY SYGNAŁ — działa też POZA trybem demo
   ============================================================ */

export function RecznySygnal({ tryb }: { tryb: "demo" | "live" }) {
  const app = useApp();
  const t = useT();
  const [text, setText] = useState("");
  const [wysyla, setWysyla] = useState(false);
  const px = app.primary.bid || 4118;

  const wzory = [
    {
      label: "BUY LIMITS",
      text: `🟢 BUY LIMITS GOLD @ ${(px - 3).toFixed(2)}/${(px - 9).toFixed(2)} AREA\n🎯 TP1 ${(px + 1).toFixed(2)}\n🎯 TP2 ${(px + 6).toFixed(2)}\n🎯 TP3 ${(px + 12).toFixed(2)}\n⛔️ SL ${(px - 22).toFixed(2)}`,
    },
    {
      label: "SELL LIMITS",
      text: `🔴 SELL LIMITS GOLD @ ${(px + 4).toFixed(2)}/${(px + 10).toFixed(2)} AREA\n🎯 TP1 ${(px - 1).toFixed(2)}\n🎯 TP2 ${(px - 7).toFixed(2)}\n⛔️ SL ${(px + 24).toFixed(2)}`,
    },
    { label: "TP1 HIT", text: "✅ TP1 HIT +32 PIPS 🎉" },
    { label: "RISK FREE", text: `RISK FREE AT ${px.toFixed(2)} 🔒` },
    { label: "OUT AT ENTRY", text: "OUT AT ENTRY ON THE REST" },
    { label: "CANCEL", text: "CANCEL THE LIMITS" },
    { label: "CLOSE ALL", text: "CLOSE ALL POSITIONS NOW ⚠️" },
  ];

  const wyslij = async () => {
    const tresc = text.trim();
    if (!tresc) return;
    setWysyla(true);
    try {
      const r = await api.demoSignal(tresc, undefined, app.connection.accountSession);
      const rodzaje = (r.parsed as { type?: string }[]).map((p) => p.type).filter(Boolean);
      app.toast(
        "success",
        r.target === "demo" ? t("demo.sig.sentDemo") : t("demo.sig.sentLive"),
        rodzaje.length ? t("demo.sig.parsed", { v: rodzaje.join(", ") }) : t("demo.sig.notParsed"),
      );
      setText("");
    } catch (e) {
      app.toast("error", t("demo.sig.failed"), String(e).replace(/^Error:\s*/, ""));
    } finally {
      setWysyla(false);
    }
  };

  return (
    <Card
      title={t("demo.sig.title")}
      icon="send"
      accent="var(--info)"
      subtitle={tryb === "demo" ? t("demo.sig.toDemo") : t("demo.sig.toLive")}
    >
      <div className="col">
        <div className="row row--tight">
          {wzory.map((w) => (
            <Button key={w.label} size="sm" variant="outline" onClick={() => setText(w.text)}>
              {w.label}
            </Button>
          ))}
        </div>

        <textarea
          className="textarea"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={t("demo.sig.placeholder")}
        />

        <div className="row">
          <span className="hint" style={{ flex: 1, minWidth: 220 }}>
            {tryb === "demo" ? t("demo.sig.noteDemo") : t("demo.sig.noteLive")}
          </span>
          <Button variant="primary" icon="send" disabled={!text.trim() || wysyla} onClick={() => void wyslij()}>
            {t("demo.sig.send")}
          </Button>
        </div>
      </div>
    </Card>
  );
}

/* ============================================================
   WIDOK
   ============================================================ */

export function DemoView() {
  const app = useApp();
  const t = useT();
  const demo = app.demo;
  const [cfg, setCfg] = useState<DemoConfig>(DOMYSLNE_DEMO);
  const [skan, setSkan] = useState<DemoScan | null>(null);
  const [skanuje, setSkanuje] = useState(false);
  const [wyslane, setWyslane] = useState(false);
  const [zaladowano, setZaladowano] = useState(false);

  /* konfiguracja z serwera — raz, przy wejściu na widok */
  useEffect(() => {
    if (!app.live || zaladowano) return;
    setZaladowano(true);
    void api
      .demoConfig()
      .then((c) => setCfg({ ...DOMYSLNE_DEMO, ...c }))
      .catch(() => undefined);
  }, [app.live, zaladowano]);

  /* gdy przebieg chodzi, formularz pokazuje jego konfigurację */
  const biegnie = demo.running;
  const poprzednioBieglo = useRef(false);
  useEffect(() => {
    if (biegnie && !poprzednioBieglo.current) setCfg({ ...DOMYSLNE_DEMO, ...demo.config });
    poprzednioBieglo.current = biegnie;
  }, [biegnie, demo.config]);

  const blad = useCallback((msg: string) => app.toast("error", t("demo.err.inspect"), msg), [app]);

  const przypisz = useCallback(
    (c: DemoCandidate) => {
      setCfg((s) => {
        if (c.role === "ticks") {
          return {
            ...s,
            priceSource: "file",
            ticksPath: c.path,
            ticksFrom: s.ticksFrom || c.firstDay,
            ticksTo: s.ticksTo || c.lastDay,
            // sniffer rozpoznał zegar pliku — bierzemy jego propozycję zamiast
            // zgadywać; to jest ta różnica, która potrafi zrobić z backtestu
            // darmowy zysk
            msgClockOffsetMs: c.suggestedMsgOffsetMs ?? s.msgClockOffsetMs,
          };
        }
        return {
          ...s,
          signalsPath: c.path,
          signalsFrom: s.signalsFrom || c.firstDay,
          signalsTo: s.signalsTo || c.lastDay,
        };
      });
      app.toast(
        "success",
        t("demo.assigned"),
        `${c.name} → ${c.role === "ticks" ? t("demo.ticks") : t("demo.signals")}`,
      );
    },
    [app],
  );

  const skanuj = async () => {
    setSkanuje(true);
    try {
      const r = await api.demoScan({ depth: 3, timeoutMs: 5000 });
      setSkan(r);
      if (r.candidates.length === 0) {
        app.toast("warn", t("demo.scan.nothing"), t("demo.scan.nothing.text", { n: num(r.filesSeen, 0), d: r.roots.length }));
      }
    } catch (e) {
      app.toast("error", t("demo.scan.failed"), String(e).replace(/^Error:\s*/, ""));
    } finally {
      setSkanuje(false);
    }
  };

  const start = async () => {
    setWyslane(true);
    try {
      await api.demoStart(cfg);
      app.toast("info", t("demo.started"), t("demo.started.text"));
    } catch (e) {
      app.toast("error", t("demo.start.failed"), String(e).replace(/^Error:\s*/, ""));
    } finally {
      setWyslane(false);
    }
  };

  const stop = async () => {
    try {
      await api.demoStop();
      app.toast("warn", t("demo.stopping"), t("demo.stopping.text"));
    } catch (e) {
      app.toast("error", t("demo.stop.failed"), String(e).replace(/^Error:\s*/, ""));
    }
  };

  const zapisz = async () => {
    try {
      await api.demoSaveConfig(cfg);
      app.toast("success", t("demo.saved"), t("demo.saved.text"));
    } catch (e) {
      app.toast("error", t("demo.save.failed"), String(e).replace(/^Error:\s*/, ""));
    }
  };

  const tempo = async (v: number) => {
    setCfg((s) => ({ ...s, speed: v }));
    if (!demo.running) return;
    try {
      await api.demoSpeed(v);
    } catch (e) {
      app.toast("error", t("demo.speed.failed"), String(e).replace(/^Error:\s*/, ""));
    }
  };

  const tempa = tempaOpcje(t);
  const ticki = useMemo(() => (skan?.candidates ?? []).filter((c) => c.role === "ticks"), [skan]);
  const sygnaly = useMemo(() => (skan?.candidates ?? []).filter((c) => c.role === "signals"), [skan]);

  if (!app.live) {
    return (
      <div className="view">
        <div className="view__head">
          <div className="view__headmain">
            <h1>{t("demo.title")}</h1>
            <p>{t("demo.subtitle")}</p>
          </div>
        </div>
        <Card>
          <Empty icon="robot" title={t("demo.needServer")} text={t("demo.needServer.text")} />
        </Card>
      </div>
    );
  }

  const f = FAZA[demo.phase] ?? FAZA.idle;
  const zysk = demo.equity - demo.startBalance;

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("demo.title")}</h1>
          <p>
            <RichT k="demo.intro" />
          </p>
        </div>
        <div className="demo__badge">
          <Badge tone={f.tone} dot={demo.running}>
            {t(f.label)}
          </Badge>
          {demo.running && <span className="hint num">{demo.clockLabel}</span>}
        </div>
      </div>

      {/* ================= PRZEBIEG ================= */}
      {(demo.running || demo.phase !== "idle") && (
        <Card
          title={t("demo.run.title")}
          icon="robot"
          accent={demo.phase === "failed" ? "var(--short)" : demo.running ? "var(--accent)" : "var(--long)"}
          subtitle={demo.source === "synthetic" ? t("demo.src.synthetic") : t("demo.src.file")}
          actions={
            <div className="row row--tight">
              <Segmented
                value={String(cfg.speed)}
                onChange={(v) => void tempo(Number(v))}
                size="sm"
                options={tempa}
              />
              {demo.running ? (
                <Button size="sm" variant="danger" icon="pause" onClick={() => void stop()}>
                  {t("demo.stop")}
                </Button>
              ) : (
                <Button size="sm" variant="primary" icon="play" disabled={wyslane} onClick={() => void start()}>
                  {t("demo.restart")}
                </Button>
              )}
            </div>
          }
        >
          <div className="demo__bar">
            <div className="meter" style={{ height: 8 }}>
              <div
                className="meter__fill"
                style={{
                  width: `${Math.max(1, demo.progress * 100)}%`,
                  background: demo.phase === "failed" ? "var(--short)" : "var(--accent)",
                }}
              />
            </div>
            <b className="num demo__pct">
              {demo.ticksTotal > 0 ? `${(demo.progress * 100).toFixed(1)}%` : "∞"}
            </b>
          </div>

          <div className="demo__clock">
            {demo.running && <span className="demo__spin" />}
            <span className="truncate">
              {demo.clockLabel || "—"} ·{" "}
              {demo.speed <= 0 ? t("demo.speed.max") : t("demo.speed.n", { n: demo.speed })}
            </span>
          </div>

          <div className="demo__metrics">
            <div className="demo__metric">
              <b className={`num ${zysk >= 0 ? "up" : "down"}`}>{kwota(zysk)}</b>
              <span>{t("demo.m.result")}</span>
            </div>
            <div className="demo__metric">
              <b className="num">{demo.equity.toFixed(2)} $</b>
              <span>{t("demo.m.equity", { v: demo.startBalance.toFixed(2) })}</span>
            </div>
            <div className="demo__metric">
              <b className="num">{num(demo.ticksDone, 0)}</b>
              <span>
                {t("demo.m.quotes")} {demo.ticksTotal > 0 ? t("demo.m.outOf", { n: num(demo.ticksTotal, 0) }) : ""}
              </span>
            </div>
            <div className="demo__metric">
              <b className="num">{duration(demo.elapsedMs)}</b>
              <span>{t("demo.m.elapsed")}</span>
            </div>
            <div className="demo__metric">
              <b className="num">
                {demo.openPositions} / {demo.openPendings}
              </b>
              <span>{t("demo.m.posPend")}</span>
            </div>
            <div className="demo__metric">
              <b className="num">{demo.baskets}</b>
              <span>{t("demo.m.baskets")}</span>
            </div>
            <div className="demo__metric">
              <b className="num">{demo.trades}</b>
              <span>{t("demo.m.trades")}</span>
            </div>
            <div className="demo__metric">
              <b className="num">
                {demo.signals} / {demo.messages}
              </b>
              <span>{t("demo.m.sigMsg")}</span>
            </div>
            <div className="demo__metric">
              <b className="num">{demo.manualSignals}</b>
              <span>{t("demo.m.manual")}</span>
            </div>
          </div>

          {demo.error && (
            <div className="demo__err">
              <Icon name="alert" size={14} />
              <span>{tSilnik(demo.error)}</span>
            </div>
          )}
          {demo.note && !demo.error && <div className="demo__note">{tSilnik(demo.note)}</div>}
          {demo.running && (
            <div className="demo__hintbox">
              <Icon name="info" size={13} />
              <span>
                <RichT k="demo.run.note" />
              </span>
            </div>
          )}
        </Card>
      )}

      {/* ================= RĘCZNY SYGNAŁ ================= */}
      <RecznySygnal tryb={demo.running ? "demo" : "live"} />

      {/* ================= KONFIGURACJA ================= */}
      <Card
        title={t("demo.cfg.title")}
        icon="sliders"
        actions={
          <div className="row row--tight">
            <Button size="sm" variant="outline" icon="check" onClick={() => void zapisz()}>
              {t("common.save")}
            </Button>
            <Button
              variant="primary"
              icon="play"
              size="sm"
              disabled={demo.running || wyslane}
              onClick={() => void start()}
            >
              {demo.running ? t("demo.alreadyRunning") : t("demo.start")}
            </Button>
          </div>
        }
      >
        <div className="formgrid">
          <Field label={t("demo.f.balance")} hint="0 … 1 000 000 000 $">
            <NumberInput
              value={cfg.balance}
              onChange={(v) => setCfg({ ...cfg, balance: v })}
              step={100}
              min={0}
              max={1_000_000_000}
              unit="$"
            />
          </Field>
          <Field label={t("demo.f.priceSource")}>
            <Select
              value={cfg.priceSource}
              onChange={(v) => setCfg({ ...cfg, priceSource: v as DemoConfig["priceSource"] })}
              options={[
                { value: "file", label: t("demo.f.priceSource.file") },
                { value: "synthetic", label: t("demo.f.priceSource.synth") },
              ]}
            />
          </Field>
          <Field label={t("demo.f.speed")} hint={t("demo.f.speed.hint")}>
            <Segmented value={String(cfg.speed)} onChange={(v) => void tempo(Number(v))} options={tempa} />
          </Field>
        </div>

        {/* ---- ticki z pliku ---- */}
        {cfg.priceSource === "file" && (
          <>
            <PolePliku
              label={t("demo.f.ticksFile")}
              hint={t("demo.f.ticksFile.hint")}
              value={cfg.ticksPath}
              onChange={(v) => setCfg({ ...cfg, ticksPath: v })}
              onRozpoznano={przypisz}
              onBlad={blad}
            />
            <div className="formgrid">
              <Field label={t("demo.f.ticksFrom")} hint={t("demo.f.ticksFrom.hint")}>
                <TextInput type="date" value={cfg.ticksFrom} onChange={(v) => setCfg({ ...cfg, ticksFrom: v })} />
              </Field>
              <Field label={t("demo.f.ticksTo")} hint={t("demo.f.ticksTo.hint")}>
                <TextInput type="date" value={cfg.ticksTo} onChange={(v) => setCfg({ ...cfg, ticksTo: v })} />
              </Field>
            </div>
          </>
        )}

        {/* ---- generator ---- */}
        {cfg.priceSource === "synthetic" && (
          <div className="formgrid">
            <Field label={t("demo.f.seed")} hint={t("demo.f.seed.hint")}>
              <NumberInput value={cfg.seed} onChange={(v) => setCfg({ ...cfg, seed: v })} step={1} min={0} />
            </Field>
            <Field label={t("demo.f.startPrice")}>
              <NumberInput
                value={cfg.synthStartPrice}
                onChange={(v) => setCfg({ ...cfg, synthStartPrice: v })}
                step={10}
                min={1}
                unit="$"
              />
            </Field>
            <Field label={t("demo.f.vol")} hint={t("demo.f.vol.hint")}>
              <NumberInput
                value={cfg.synthVol}
                onChange={(v) => setCfg({ ...cfg, synthVol: v })}
                step={0.01}
                min={0}
              />
            </Field>
            <Field label="Spread" hint={t("demo.f.spread.hint")}>
              <NumberInput
                value={cfg.synthSpread}
                onChange={(v) => setCfg({ ...cfg, synthSpread: v })}
                step={0.01}
                min={0}
                unit="$"
              />
            </Field>
            <Field label={t("demo.f.interval")}>
              <NumberInput
                value={cfg.synthIntervalMs}
                onChange={(v) => setCfg({ ...cfg, synthIntervalMs: v })}
                step={50}
                min={1}
                unit="ms"
              />
            </Field>
          </div>
        )}

        {/* ---- sygnały ---- */}
        <div className="row" style={{ marginTop: "var(--sp-3)" }}>
          <Checkbox
            checked={cfg.useFileSignals}
            onChange={(v) => setCfg({ ...cfg, useFileSignals: v })}
            label={t("demo.f.useFileSignals")}
            title={t("demo.f.useFileSignals.title")}
          />
        </div>

        {cfg.useFileSignals && (
          <>
            <PolePliku
              label={t("demo.f.signalsFile")}
              hint={t("demo.f.signalsFile.hint")}
              value={cfg.signalsPath}
              onChange={(v) => setCfg({ ...cfg, signalsPath: v })}
              onRozpoznano={przypisz}
              onBlad={blad}
            />
            <div className="formgrid">
              <Field label={t("demo.f.signalsFrom")} hint={t("demo.f.signalsFrom.hint")}>
                <TextInput
                  type="date"
                  value={cfg.signalsFrom}
                  onChange={(v) => setCfg({ ...cfg, signalsFrom: v })}
                />
              </Field>
              <Field label={t("demo.f.signalsTo")}>
                <TextInput type="date" value={cfg.signalsTo} onChange={(v) => setCfg({ ...cfg, signalsTo: v })} />
              </Field>
              <Field
                label={t("demo.f.msgOffset")}
                hint={t("demo.f.msgOffset.hint")}
              >
                <NumberInput
                  value={(cfg.msgClockOffsetMs ?? 0) / 3_600_000}
                  onChange={(v) => setCfg({ ...cfg, msgClockOffsetMs: Math.round(v * 3_600_000) })}
                  step={0.5}
                  unit="h"
                />
              </Field>
              <Field label={t("demo.f.sourceName")} hint={t("demo.f.sourceName.hint")}>
                <TextInput value={cfg.sourceName} onChange={(v) => setCfg({ ...cfg, sourceName: v })} />
              </Field>
            </div>
            <div className="demo__hintbox">
              <Icon name="info" size={13} />
              <span>
                <RichT k="demo.clockWarning" />
              </span>
            </div>
          </>
        )}
      </Card>

      {/* ================= WYSZUKIWANIE DANYCH ================= */}
      <Card
        title={t("demo.scan.title")}
        icon="search"
        subtitle={skan ? t("demo.scan.subtitle", { n: num(skan.filesSeen, 0), ms: skan.elapsedMs }) : undefined}
        actions={
          <Button size="sm" variant="outline" icon={skanuje ? "hourglass" : "refresh"} disabled={skanuje} onClick={() => void skanuj()}>
            {skanuje ? t("demo.scan.searching") : t("demo.scan.btn")}
          </Button>
        }
      >
        {!skan && (
          <Empty
            icon="search"
            title={t("demo.scan.empty")}
            text={t("demo.scan.empty.text")}
          />
        )}

        {skan && (
          <>
            {skan.truncated && (
              <div className="demo__hintbox">
                <Icon name="alert" size={13} />
                <span>{t("demo.scan.truncated")}</span>
              </div>
            )}
            <div className="demo__cols">
              <div>
                <h4 className="demo__coltitle">{t("demo.col.ticks", { n: ticki.length })}</h4>
                {ticki.length === 0 && <div className="hint">{t("demo.col.none")}</div>}
                {ticki.map((c) => (
                  <Kandydat key={c.path} c={c} wybrany={c.path === cfg.ticksPath} onWybierz={przypisz} />
                ))}
              </div>
              <div>
                <h4 className="demo__coltitle">{t("demo.col.signals", { n: sygnaly.length })}</h4>
                {sygnaly.length === 0 && <div className="hint">{t("demo.col.none")}</div>}
                {sygnaly.map((c) => (
                  <Kandydat key={c.path} c={c} wybrany={c.path === cfg.signalsPath} onWybierz={przypisz} />
                ))}
              </div>
            </div>
            <div className="hint demo__roots">{t("demo.scan.roots", { v: skan.roots.join(" · ") })}</div>
          </>
        )}
      </Card>
    </div>
  );
}
