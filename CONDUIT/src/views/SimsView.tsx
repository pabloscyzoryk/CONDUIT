import { useState } from "react";
import { Badge, Button, Card, Empty, Field, Icon, NumberInput, Select, TextInput } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT, RichT } from "@/i18n";
import { czyCzempion, grupujPoFormacie } from "@/data/presets";
import { ago, money, toneOf } from "@/lib/format";
import type { SimInstance, TradingMode } from "@/types";
import "./views.css";


const TRYBY_SYM: TradingMode[] = ["AUTO", "AUTO-EA", "AI"];

/** Kolor plakietki trybu — spójny z resztą panelu (AI ma własny ton). */
function tonTrybu(m: TradingMode): "info" | "accent" | "ai" {
  return m === "AI" ? "ai" : m === "AUTO-EA" ? "accent" : "info";
}

/** Mini-wykres krzywej kapitału instancji. */
function Sparkline({ data, tone }: { data: number[]; tone: "up" | "down" | "flat" }) {
  if (data.length < 2) return <div style={{ height: 34 }} />;
  const min = Math.min(...data);
  const max = Math.max(...data);
  const span = Math.max(1e-6, max - min);
  const pts = data
    .map((v, i) => `${(i / (data.length - 1)) * 100},${34 - ((v - min) / span) * 30 - 2}`)
    .join(" ");
  const color = tone === "up" ? "var(--long)" : tone === "down" ? "var(--short)" : "var(--text-muted)";
  return (
    <svg viewBox="0 0 100 34" preserveAspectRatio="none" className="spark" style={{ height: 34 }}>
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.6" vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
    </svg>
  );
}

export function SimsView() {
  const app = useApp();
  const t = useT();
  const cur = app.settings.display_currency;
  const [preset, setPreset] = useState("");
  const [name, setName] = useState("");
  const [balance, setBalance] = useState(200);
  const [lot, setLot] = useState(0.01);
  // "" = dziedziczenie trybu głównego bota (kontrakt zera po stronie serwera)
  const [mode, setMode] = useState<"" | TradingMode>("");
  const [detail, setDetail] = useState<SimInstance | null>(null);

  const shown = app.sims.find((s) => s.id === detail?.id) ?? null;

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("sims.title")}</h1>
          <p>
            <RichT k="sims.intro" />
          </p>
        </div>
        <Badge tone="warn" dot>
          {t("sims.count", { n: app.sims.length })}
        </Badge>
      </div>

      <Card title={t("sims.add.title")} icon="plus" accent="var(--info)">
        <div className="formgrid">
          {/* Presety ROZDZIELONE po formacie sygnałów — tak samo jak w galerii
              i w łańcuchach. Instancja gra jednym presetem, więc format,
              do którego on należy, jest częścią odpowiedzi „co ja właściwie
              symuluję". */}
          <Field label={t("sims.field.preset")}>
            <Select
              value={preset || app.presets[0]?.id || ""}
              onChange={setPreset}
              options={grupujPoFormacie(app.presets).flatMap(({ format, presety }) =>
                presety.map((p) => ({
                  value: p.id,
                  label: `${czyCzempion(p.name) ? "👑 " : ""}${p.name}`,
                  group: t("sims.group.format", { v: format }),
                })),
              )}
            />
          </Field>
          <Field label={t("sims.field.name")} hint={t("sims.field.name.hint")}>
            <TextInput value={name} onChange={setName} placeholder={t("sims.field.name.ph")} />
          </Field>
          {/* WŁASNY tryb instancji: główny bot może grać AUTO-EA, a symulacja
              obok AUTO — i na odwrót. Domyślnie dziedziczenie trybu głównego,
              żeby istniejący nawyk „dodaj i porównaj" niczego nie zmieniał. */}
          <Field label={t("sims.field.mode")}>
            <Select
              value={mode}
              onChange={(v) => setMode(v as "" | TradingMode)}
              options={[
                { value: "", label: t("sims.mode.inherit", { v: app.mode }) },
                ...TRYBY_SYM.map((m) => ({ value: m, label: m })),
              ]}
            />
          </Field>
          <Field label={t("sims.field.balance")}>
            <NumberInput value={balance} onChange={setBalance} step={50} min={50} unit="$" />
          </Field>
          <Field label={t("sims.field.lot")}>
            <NumberInput value={lot} onChange={setLot} step={0.01} min={0.01} unit="lot" />
          </Field>
          <Field label="&nbsp;">
            <Button
              variant="primary"
              icon="plus"
              block
              onClick={() => {
                app.addSim(preset || app.presets[0]?.id || "", name, balance, lot, mode || undefined);
                setName("");
              }}
            >
              {t("sims.add.btn")}
            </Button>
          </Field>
        </div>

        <div className="divider" />

        <div className="row">
          <Field label={t("sims.stopsLevel")}>
            <NumberInput
              value={app.settings.sim_stops_level}
              onChange={(v) => app.setSetting("sim_stops_level", v)}
              step={0.05}
              min={0}
              unit="$"
            />
          </Field>
          <span className="hint" style={{ flex: 1, minWidth: 260 }}>
            {t("sims.stopsLevel.hint")}
          </span>
        </div>
      </Card>

      {app.sims.length === 0 ? (
        <Card>
          <Empty
            icon="flask"
            title={t("sims.empty")}
            text={t("sims.empty.text")}
          />
        </Card>
      ) : (
        <Card title={t("sims.list.title")} icon="flask" subtitle={`${app.sims.length}`} accent="var(--warn)">
          <div className="sims">
            {app.sims.map((s) => {
              const pnl = s.equity - s.startBalance;
              const tone = toneOf(pnl);
              return (
                <article key={s.id} className="sim">
                  <header className="sim__head">
                    <span className="sim__name truncate">{s.name}</span>
                    {/* Plakietka trybu: własny tryb instancji pełnym kolorem,
                        dziedziczony po bocie głównym — wyszarzony, z podpowiedzią.
                        Bez niej „bot AUTO-EA + symulacje AUTO" wyglądałoby
                        w panelu jak jedna i ta sama gra. */}
                    {s.mode ? (
                      <Badge tone={tonTrybu(s.mode)}>{s.mode}</Badge>
                    ) : (
                      <Badge tone="muted" title={t("sims.mode.inherited")}>
                        {app.mode}
                      </Badge>
                    )}
                    <span className="spacer" />
                    <Badge tone={tone === "up" ? "long" : tone === "down" ? "short" : "muted"}>
                      {pnl >= 0 ? "+" : "−"}
                      {money(Math.abs(pnl), cur)}
                    </Badge>
                  </header>

                  <div className="hint truncate">{s.preset}</div>

                  <Sparkline data={s.curve} tone={tone} />

                  <div className="sim__metrics">
                    <div className="sim__metric">
                      <b>{money(s.equity, cur)}</b>
                      <span>{t("sims.m.equity")}</span>
                    </div>
                    <div className="sim__metric">
                      <b>{s.trades}</b>
                      <span>{t("sims.m.trades")}</span>
                    </div>
                    <div className="sim__metric">
                      <b className="down">−{money(s.maxDd, cur)}</b>
                      <span>{t("sims.m.maxDd")}</span>
                    </div>
                    <div className="sim__metric">
                      <b>{s.positions}</b>
                      <span>{t("sims.m.positions")}</span>
                    </div>
                    <div className="sim__metric">
                      <b>{s.pendings}</b>
                      <span>{t("sims.m.pendings")}</span>
                    </div>
                    <div className="sim__metric">
                      <b>{s.baskets}</b>
                      <span>{t("sims.m.baskets")}</span>
                    </div>
                  </div>

                  <div className="row row--tight">
                    <Button size="sm" variant="outline" icon="chart" onClick={() => setDetail(s)}>
                      {t("sims.details")}
                    </Button>
                    <Button size="sm" variant="ghost" icon="refresh" onClick={() => app.resetSim(s.id)}>
                      {t("sims.reset")}
                    </Button>
                    <span className="spacer" />
                    <Button size="sm" variant="danger" icon="trash" onClick={() => app.removeSim(s.id)} />
                  </div>

                  <span className="hint" style={{ fontSize: "var(--fs-3xs)" }}>
                    {t("sims.created", { v: ago(s.createdAt), lot: s.lot.toFixed(2) })}
                  </span>
                </article>
              );
            })}
          </div>
        </Card>
      )}

      {shown && (
        <Card
          title={
            <span className="row row--tight">
              <Icon name="chart" size={15} />
              {t("sims.detail.title", { v: shown.name })}
            </span>
          }
          accent="var(--info)"
          actions={
            <Button size="sm" variant="ghost" icon="x" onClick={() => setDetail(null)}>
              {t("common.close")}
            </Button>
          }
        >
          <div className="hstats" style={{ marginBottom: "var(--sp-3)" }}>
            <div className="hstat">
              <b>{money(shown.startBalance, cur)}</b>
              <span>{t("sims.d.startBalance")}</span>
            </div>
            <div className="hstat">
              <b className={toneOf(shown.equity - shown.startBalance)}>{money(shown.equity, cur)}</b>
              <span>{t("sims.d.equityNow")}</span>
            </div>
            <div className="hstat">
              <b>{((shown.equity / shown.startBalance - 1) * 100).toFixed(1)}%</b>
              <span>{t("sims.d.return")}</span>
            </div>
            <div className="hstat">
              <b className="down">−{money(shown.maxDd, cur)}</b>
              <span>{t("sims.d.maxDd")}</span>
            </div>
            <div className="hstat">
              <b>{shown.trades}</b>
              <span>{t("sims.d.trades")}</span>
            </div>
            <div className="hstat">
              <b>{shown.winRate}%</b>
              <span>{t("sims.d.winDays")}</span>
            </div>
          </div>

          <div style={{ height: 120 }}>
            <Sparkline data={shown.curve} tone={toneOf(shown.equity - shown.startBalance)} />
          </div>

          <p className="hint" style={{ marginTop: "var(--sp-3)" }}>
            {t("sims.detail.note")}
          </p>
        </Card>
      )}
    </div>
  );
}
