import { useEffect, useState } from "react";
import { Badge, Button, Card, Icon, Select } from "@/components/ui";
import { api, type ExportIndex } from "@/store/transport";
import { useT, RichT } from "@/i18n";

/* ============================================================
   EKSPORTY

   Pliki składa SERWER i oddaje z nagłówkiem `Content-Disposition`.
   Panel tylko otwiera adres — dzięki temu eksport dziennika (który leży
   na dysku, a nie w migawce) działa dokładnie tak samo jak eksport
   historii (która w migawce jest). Jedna droga zamiast dwóch.
   ============================================================ */

/** Pobranie pliku bez opuszczania strony. */
function pobierz(url: string) {
  const a = document.createElement("a");
  a.href = url;
  // nazwę pliku narzuca serwer nagłówkiem Content-Disposition; `download`
  // bez wartości mówi tylko „zapisz, nie nawiguj"
  a.download = "";
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export interface FiltrEksportu {
  from?: number;
  to?: number;
  scope?: string;
  reason?: string;
}

/** Przyciski eksportu historii — dziedziczą filtry z widoku. */
export function EksportHistorii({ filtr, ile }: { filtr: FiltrEksportu; ile: number }) {
  const t = useT();
  const [sep, setSep] = useState<"," | ";">(",");
  const q = { ...filtr, sep };
  return (
    <div className="row row--tight">
      <Button
        size="sm"
        variant="outline"
        icon="download"
        disabled={ile === 0}
        onClick={() => pobierz(api.exportUrl("history.csv", q))}
      >
        CSV
      </Button>
      <Button
        size="sm"
        variant="outline"
        icon="download"
        disabled={ile === 0}
        onClick={() => pobierz(api.exportUrl("history.json", q))}
      >
        JSON
      </Button>
      <button
        type="button"
        className="chip"
        title={t("exp.sep.title")}
        onClick={() => setSep((s) => (s === "," ? ";" : ","))}
      >
        {t("exp.sep")} <b>{sep === "," ? t("exp.sep.comma") : t("exp.sep.semicolon")}</b>
      </button>
    </div>
  );
}

/** Pełny warsztat eksportu: log panelu, dziennik zdarzeń, paczka. */
export function EksportPelny() {
  const t = useT();
  const [idx, setIdx] = useState<ExportIndex | null>(null);
  const [blad, setBlad] = useState("");
  const [doba, setDoba] = useState("");
  const [dobaArch, setDobaArch] = useState("");
  const [sep, setSep] = useState<"," | ";">(",");

  useEffect(() => {
    let zywy = true;
    api
      .exportIndex()
      .then((v) => zywy && setIdx(v))
      .catch((e) => zywy && setBlad((e as Error).message));
    return () => {
      zywy = false;
    };
  }, []);

  const dni = idx?.journalFiles ?? [];
  const zakres = doba ? { day: doba } : {};
  const bajty = dni.filter((d) => !doba || d.day === doba).reduce((a, d) => a + d.bytes, 0);

  const dniArch = idx?.archiveFiles ?? [];
  const zakresArch = dobaArch ? { day: dobaArch } : {};
  const bajtyArch = dniArch
    .filter((d) => !dobaArch || d.day === dobaArch)
    .reduce((a, d) => a + d.bytes, 0);

  return (
    <Card
      title={t("exp.title")}
      icon="download"
      subtitle={t("exp.subtitle")}
      accent="var(--info)"
    >
      <div className="col">
        {blad && (
          <div className="hint" style={{ color: "var(--short-text)" }}>
            {t("exp.err.index", { e: blad })}
          </div>
        )}

        {/* ---- 1. wynik ---- */}
        <div className="row">
          <span style={{ flex: 1, minWidth: 200 }}>
            <b>{t("exp.history")}</b>
            <span className="hint" style={{ display: "block" }}>
              {t("exp.history.desc")}
            </span>
          </span>
          <Badge tone="muted">{idx?.counts.closed ?? 0}</Badge>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("history.csv", { sep }))}>
            CSV
          </Button>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("history.json"))}>
            JSON
          </Button>
        </div>

        <div className="row">
          <span style={{ flex: 1, minWidth: 200 }}>
            <b>{t("exp.pendings")}</b>
            <span className="hint" style={{ display: "block" }}>
              {t("exp.pendings.desc")}
            </span>
          </span>
          <Badge tone="muted">{idx?.counts.pendingHistory ?? 0}</Badge>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("pendings.csv", { sep }))}>
            CSV
          </Button>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("pendings.json"))}>
            JSON
          </Button>
        </div>

        {/* ---- 2. przebieg ---- */}
        <div className="row">
          <span style={{ flex: 1, minWidth: 200 }}>
            <b>{t("exp.log")}</b>
            <span className="hint" style={{ display: "block" }}>
              {t("exp.log.desc")}
            </span>
          </span>
          <Badge tone="muted">{idx?.counts.logs ?? 0}</Badge>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("logs.csv", { sep }))}>
            CSV
          </Button>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("logs.json"))}>
            JSON
          </Button>
        </div>

        <div className="row">
          <span style={{ flex: 1, minWidth: 200 }}>
            <b>{t("exp.messages")}</b>
            <span className="hint" style={{ display: "block" }}>
              {t("exp.messages.desc")}
            </span>
          </span>
          <Badge tone="muted">{idx?.counts.messages ?? 0}</Badge>
          <Button size="sm" variant="outline" onClick={() => pobierz(api.exportUrl("messages.csv", { sep }))}>
            CSV
          </Button>
        </div>

        {/* ---- 3. dowod ---- */}
        <div className="modenote" style={{ background: "var(--info-soft)", borderColor: "transparent" }}>
          <span className="modenote__icon" style={{ background: "var(--bg-surface)", color: "var(--info-text)" }}>
            <Icon name="book" size={16} />
          </span>
          <div style={{ width: "100%" }}>
            <b style={{ color: "var(--info-text)" }}>{t("exp.journal")}</b>
            <p>
              <RichT k="exp.journal.desc" />
            </p>

            <div className="row row--tight" style={{ marginTop: 8 }}>
              <Select
                value={doba}
                onChange={setDoba}
                size="sm"
                options={[
                  { value: "", label: t("exp.allDays", { n: dni.length }) },
                  ...dni.map((d) => ({
                    value: d.day,
                    label: `${d.day} · ${d.prefix} · ${(d.bytes / 1024).toFixed(0)} kB`,
                  })),
                ]}
              />
              <Button
                size="sm"
                variant="outline"
                disabled={dni.length === 0}
                onClick={() => pobierz(api.exportUrl("journal.jsonl", zakres))}
              >
                {t("exp.jsonl")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={dni.length === 0}
                onClick={() => pobierz(api.exportUrl("journal.csv", { ...zakres, sep }))}
              >
                {t("exp.csvSheet")}
              </Button>
              <span className="hint">{(bajty / 1024).toFixed(0)} kB</span>
            </div>
          </div>
        </div>

        {/* ---- 3b. surowe wejście ---- */}
        <div className="modenote" style={{ background: "var(--accent-soft)", borderColor: "transparent" }}>
          <span className="modenote__icon" style={{ background: "var(--bg-surface)", color: "var(--accent)" }}>
            <Icon name="telegram" size={16} />
          </span>
          <div style={{ width: "100%" }}>
            <b style={{ color: "var(--accent)" }}>{t("exp.archive")}</b>
            <p>
              <RichT k="exp.archive.desc" />
            </p>

            <div className="row row--tight" style={{ marginTop: 8 }}>
              <Select
                value={dobaArch}
                onChange={setDobaArch}
                size="sm"
                options={[
                  { value: "", label: t("exp.allDays", { n: dniArch.length }) },
                  ...dniArch.map((d) => ({
                    value: d.day,
                    label: `${d.day} · ${(d.bytes / 1024).toFixed(0)} kB`,
                  })),
                ]}
              />
              <Button
                size="sm"
                variant="outline"
                disabled={dniArch.length === 0}
                onClick={() => pobierz(api.exportUrl("archive.jsonl", zakresArch))}
              >
                JSONL
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={dniArch.length === 0}
                onClick={() => pobierz(api.exportUrl("archive.csv", { ...zakresArch, sep }))}
              >
                CSV
              </Button>
              <span className="hint">
                {dniArch.length === 0 ? t("exp.archive.empty") : `${(bajtyArch / 1024).toFixed(0)} kB`}
              </span>
            </div>
          </div>
        </div>

        {/* ---- 4. wszystko ---- */}
        <div className="row">
          <span style={{ flex: 1, minWidth: 200 }}>
            <b>{t("exp.bundle")}</b>
            <span className="hint" style={{ display: "block" }}>
              {t("exp.bundle.desc")}
            </span>
          </span>
          <Button size="sm" variant="primary" icon="download" onClick={() => pobierz(api.exportUrl("bundle.json"))}>
            {t("exp.bundle.btn")}
          </Button>
        </div>

        <div className="row row--tight">
          <button
            type="button"
            className="chip"
            title={t("exp.sep.title")}
            onClick={() => setSep((s) => (s === "," ? ";" : ","))}
          >
            {t("exp.sep.csv")} <b>{sep === "," ? t("exp.sep.comma") : t("exp.sep.semicolon")}</b>
          </button>
          {idx && <span className="hint">{t("exp.journalDir", { v: idx.journalDir })}</span>}
        </div>
      </div>
    </Card>
  );
}
