import { useT } from "@/i18n";
import { tSilnik } from "@/i18n/silnik";


import { useEffect, useState } from "react";
import { Button, Icon, TextInput } from "@/components/ui";
import { api } from "@/store/transport";

interface Zawartosc {
  path: string;
  parent: string | null;
  dirs: { name: string; path: string }[];
  pliki?: { name: string; path: string; bytes: number }[];
}

const rozmiar = (b: number) =>
  b >= 1048576 ? `${(b / 1048576).toFixed(1)} MB` : b >= 1024 ? `${Math.round(b / 1024)} kB` : `${b} B`;

/**
 * @param tryb    `katalog` — zwraca ścieżkę katalogu; `plik` — zwraca ścieżkę
 *                wskazanego pliku (lista plików wg `rozszerzenia`).
 * @param start   od jakiego katalogu zacząć; puste = katalog `logs` bota.
 */
export function WyborSciezki({
  tryb,
  tytul,
  rozszerzenia = "jsonl",
  start,
  onWybierz,
  onZamknij,
}: {
  tryb: "katalog" | "plik";
  tytul: string;
  rozszerzenia?: string;
  start?: string;
  onWybierz: (p: string) => void;
  onZamknij: () => void;
}) {
  const t = useT();
  const [dane, setDane] = useState<Zawartosc | null>(null);
  const [blad, setBlad] = useState<string | null>(null);
  const [nazwa, setNazwa] = useState("");

  const wczytaj = (path?: string) => {
    setBlad(null);
    api
      .fsDirs(path, tryb === "plik" ? rozszerzenia : undefined)
      .then((d) => setDane(d as Zawartosc))
      .catch((e) => setBlad(String(e)));
  };

  useEffect(() => {
    wczytaj(start);
    // celowo raz: modal otwiera się na wskazanym katalogu i dalej nawiguje sam
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const zlacz = (dir: string, plik: string) =>
    dir.endsWith("\\") || dir.endsWith("/") ? dir + plik : `${dir}\\${plik}`;

  return (
    <div className="modal-scrim" role="dialog" onClick={onZamknij}>
      <div className="modal" style={{ maxWidth: 560 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal__body">
          <div className="row" style={{ marginBottom: "var(--sp-2)" }}>
            <b>{tytul}</b>
            <span className="spacer" />
            <Button size="sm" variant="ghost" icon="x" onClick={onZamknij} />
          </div>

          <p className="hint" style={{ marginBottom: "var(--sp-2)" }}>
            {t("path.server")}
          </p>

          {blad && (
            <p className="hint" style={{ color: "var(--short-text)" }}>
              {tSilnik(blad)}
            </p>
          )}

          {dane && (
            <>
              <p className="hint truncate mono" style={{ marginBottom: "var(--sp-2)" }}>
                {dane.path}
              </p>

              <div className="row row--tight" style={{ marginBottom: "var(--sp-2)" }}>
                <Button
                  size="sm"
                  variant="outline"
                  icon="arrow-up"
                  onClick={() => wczytaj(dane.parent ?? "NAPĘDY")}
                >
                  {t("logs.dirPick.up")}
                </Button>
                <span className="spacer" />
                {tryb === "katalog" && (
                  <Button size="sm" variant="primary" icon="check" onClick={() => onWybierz(dane.path)}>
                    {t("logs.dirPick.pick")}
                  </Button>
                )}
              </div>

              <div style={{ maxHeight: 280, overflowY: "auto", display: "grid", gap: 2 }}>
                {dane.dirs.length === 0 && !dane.pliki?.length && (
                  <span className="hint">{t("logs.dirPick.empty")}</span>
                )}
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
                {tryb === "plik" &&
                  (dane.pliki ?? []).map((f) => (
                    <button
                      key={f.path}
                      className="settings__navitem"
                      onClick={() => onWybierz(f.path)}
                      title={f.path}
                    >
                      <Icon name="logs" size={12} />
                      <span className="truncate">{f.name}</span>
                      <span className="spacer" />
                      <span className="hint mono">{rozmiar(f.bytes)}</span>
                    </button>
                  ))}
              </div>

              {/* NOWY plik w tym katalogu — bez tego nie da się założyć kroniki
                  w miejscu, w którym jeszcze niczego nie ma, a to jest właśnie
                  pierwsze uruchomienie. */}
              {tryb === "plik" && (
                <>
                  <div className="divider" />
                  <div className="row row--tight" style={{ alignItems: "center" }}>
                    <div style={{ flex: 1 }}>
                      <TextInput
                        value={nazwa}
                        onChange={setNazwa}
                        placeholder={t("path.newName")}
                      />
                    </div>
                    <Button
                      size="sm"
                      variant="primary"
                      icon="check"
                      disabled={!nazwa.trim()}
                      onClick={() => onWybierz(zlacz(dane.path, nazwa.trim()))}
                    >
                      {t("path.useName")}
                    </Button>
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
