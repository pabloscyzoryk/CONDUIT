/* ============================================================
   WIZUALIZACJA MODELI AI — punkt wejścia samodzielnej aplikacji

   To jest DRUGIE wejście do tego samego kodu źródłowego co panel
   (`main.tsx`), zbudowane osobno przez `vite.wizualizacja.config.ts`
   i wkompilowane w `wizualizacja.exe`.

   Czego tu NIE MA i dlaczego:
     * logowania — narzędzie ogląda pliki z dysku, nie konto Telegrama,
     * `AppProvider` — cały stan bota (pozycje, ceny, WebSocket, timery)
       jest tu bezużyteczny; `AiModelsView` go nie dotyka, więc nie ma
       powodu go podnosić,
     * `Shell` — nawigacja do widoków, których ten program nie serwuje,
       byłaby ślepą uliczką.

   Co ZOSTAJE bez zmian: sam `AiModelsView`, prymitywy UI, tokeny,
   palety. Dzięki temu 16 kombinacji motyw × paleta działa tu tak
   samo jak w panelu, a poprawka w widoku trafia w oba programy.
   ============================================================ */

import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { Button, Icon, Select } from "@/components/ui";
import { AiModelsView } from "@/views/AiModelsView";
import { PALETTES } from "@/store/useTheme";
import * as storage from "@/store/storage";
import type { PaletteName, ThemeName } from "@/types";
import "./styles/tokens.css";
import "./styles/palettes.css";
import "./styles/global.css";
import "./styles/ui.css";
import "./wizualizacja.css";

/* ------------------------------------------------------------
   Wygląd

   Świadomie NIE używamy `useTheme()` z panelu: tamten hook przy każdej
   zmianie wysyła `PATCH /api/settings` do backendu bota, którego ten
   program nie ma (i mieć nie powinien — nie zapisuje żadnej konfiguracji).
   Klucze magazynu są te same, więc wybór i tak jest wspólny z panelem.
   ------------------------------------------------------------ */
function useWyglad() {
  const [theme, setTheme] = useState<ThemeName>(() => storage.load<ThemeName>("theme", "dark"));
  const [palette, setPalette] = useState<PaletteName>(() => storage.load<PaletteName>("palette", "violet"));

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    storage.save("theme", theme);
  }, [theme]);

  useEffect(() => {
    document.documentElement.dataset.palette = palette;
    storage.save("palette", palette);
  }, [palette]);

  /* krótka animacja przejścia kolorów — jak w panelu */
  const przelacz = (fn: () => void) => {
    const root = document.documentElement;
    root.classList.add("theme-anim");
    window.setTimeout(() => root.classList.remove("theme-anim"), 320);
    fn();
  };

  return {
    theme,
    palette,
    toggleTheme: () => przelacz(() => setTheme((t) => (t === "dark" ? "light" : "dark"))),
    setPalette: (p: PaletteName) => przelacz(() => setPalette(p)),
  };
}

/* ------------------------------------------------------------
   Informacja o źródle modeli

   `wizualizacja.exe` wystawia `/api/wiz/info` z BEZWZGLĘDNĄ ścieżką
   katalogu, z którego czyta. To jest odpowiedź na pytanie „skąd te
   modele?" zadawane zawsze, gdy na liście widać coś nieoczekiwanego —
   i jedyny sposób, żeby pusty katalog nie wyglądał jak awaria programu.
   ------------------------------------------------------------ */
interface WizInfo {
  katalog: string;
  /** czy wybrany katalog w ogóle istnieje */
  istnieje: boolean;
  /** wszystkie sprawdzone ścieżki — pokazywane, gdy nie ma czego pokazać */
  szukano: string[];
  liczba: number;
  /** pliki `*.json` bez sieci (migawki treningu) — nie są modelami */
  pominiete: number;
  wersja: string;
}

function useInfo(): WizInfo | null {
  const [info, setInfo] = useState<WizInfo | null>(null);
  useEffect(() => {
    let anulowane = false;
    void fetch("/api/wiz/info")
      .then((r) => (r.ok ? (r.json() as Promise<WizInfo>) : null))
      .then((v) => {
        if (!anulowane) setInfo(v);
      })
      .catch(() => undefined);
    return () => {
      anulowane = true;
    };
  }, []);
  return info;
}

/* ------------------------------------------------------------
   Powłoka
   ------------------------------------------------------------ */
function Wizualizacja() {
  const { theme, palette, toggleTheme, setPalette } = useWyglad();
  const info = useInfo();

  /* skrót `T` na motyw — ten sam co w panelu */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement;
      if (el && ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return;
      if (e.key.toLowerCase() === "t" && !e.ctrlKey && !e.metaKey && !e.altKey) toggleTheme();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleTheme]);

  return (
    <div className="wiz">
      <header className="wiz__bar">
        <div className="wiz__brand">
          <span className="wiz__logo">
            <svg viewBox="0 0 32 32" width="24" height="24" aria-hidden="true">
              <rect width="32" height="32" rx="8" fill="var(--ai)" />
              <g stroke="var(--bg-surface)" strokeWidth="1.5" fill="none">
                <path d="M9 9.5l14 6.5M9 22.5l14-6.5M9 9.5v13" />
              </g>
              <g fill="var(--bg-surface)">
                <circle cx="9" cy="9.5" r="2.6" />
                <circle cx="9" cy="22.5" r="2.6" />
                <circle cx="23" cy="16" r="2.6" />
              </g>
            </svg>
          </span>
          <span className="wiz__name">
            <b>Wizualizacja modeli AI</b>
            <small>CONDUIT · podgląd sieci</small>
          </span>
        </div>

        <span className="wiz__spacer" />

        {info && (
          <span
            className="wiz__path"
            title={`${info.katalog}\n${info.liczba} modeli${info.pominiete ? `, pominięto ${info.pominiete} plików bez sieci` : ""}`}
          >
            <Icon name="layers" size={12} />
            <span className="mono">{info.katalog}</span>
          </span>
        )}

        <div className="wiz__tools">
          <Select<PaletteName>
            value={palette}
            onChange={setPalette}
            size="sm"
            options={PALETTES.map((p) => ({ value: p.id, label: p.label }))}
          />
          <Button
            variant="ghost"
            size="sm"
            icon={theme === "dark" ? "sun" : "moon"}
            onClick={toggleTheme}
            title="Motyw jasny / ciemny (T)"
          />
        </div>
      </header>

      <main className="wiz__body">
        {info && info.liczba === 0 && (
          <div className="wiz__pusto">
            <Icon name="alert" size={15} style={{ flex: "none", marginTop: 1 }} />
            <div>
              <b>Nie znalazłem ani jednego wytrenowanego modelu.</b>
              <div className="wiz__gdzie">
                Szukałem plików <code>*.json</code> z siecią (<code>policy.pos.dims</code>) w:
                <ul>
                  {info.szukano.map((p) => (
                    <li key={p}>
                      <code>{p}</code>
                    </li>
                  ))}
                </ul>
                {info.pominiete > 0 && (
                  <div>
                    Pominąłem {info.pominiete} plików <code>*.json</code> bez sieci — to migawki treningu
                    (<code>*.checkpoint.json</code>), nie modele.
                  </div>
                )}
              </div>
              Wszystko poniżej to <b>model demonstracyjny</b> z wagami generatora pseudolosowego (plakietka „wagi
              lokalne") — <b>nie</b> model wytrenowany. Wskaż właściwy katalog:{" "}
              <code>wizualizacja.exe --modele C:\ścieżka\do\models</code>
            </div>
          </div>
        )}
        <AiModelsView />
      </main>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Wizualizacja />
  </StrictMode>,
);
