/* ============================================================
   KRONIKA — punkt wejścia samodzielnej aplikacji.

   TRZECIE wejście do tego samego kodu źródłowego co panel
   (`main.tsx`) i wizualizacja (`wizualizacja.tsx`), zbudowane
   osobno przez `vite.kronika.config.ts` i wkompilowane
   w `kronika.exe` przez `rust-embed`.

   Czego tu NIE MA i dlaczego:
     * `AppProvider` — cały stan bota (pozycje, ceny, WebSocket,
       timery) jest tu bezużyteczny; `KronikaView` rozmawia wyłącznie
       z `/api/kronika/*` i nie dotyka niczego innego,
     * `Shell` — nawigacja do widoków, których ten program nie
       serwuje, byłaby ślepą uliczką,
     * `I18nProvider` — jedyne, co dawał, to skrót `L`; `useT()` i
       `useLanguage()` działają na module, więc wystarczy wybór języka
       w pasku (niżej),
     * logowania — `kronika.exe` loguje się do Telegrama WŁASNĄ sesją
       przed startem serwera; jak nie jest zalogowana, mówi o tym
       w konsoli i nie startuje wcale.

   Co ZOSTAJE bez zmian: sam `KronikaView`, prymitywy UI, tokeny
   i palety — dzięki temu poprawka w widoku trafia w oba programy,
   a 16 kombinacji motyw × paleta działa tu tak samo jak w panelu.
   ============================================================ */

import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { Button, Select } from "@/components/ui";
import { KronikaView } from "@/views/KronikaView";
import { LANGUAGES, useLanguage, type Jezyk } from "@/i18n";
import { PALETTES } from "@/store/useTheme";
import * as storage from "@/store/storage";
import type { PaletteName, ThemeName } from "@/types";
import "./styles/tokens.css";
import "./styles/palettes.css";
import "./styles/global.css";
import "./styles/ui.css";
import "./wizualizacja.css";

/* Świadomie NIE `useTheme()` z panelu: tamten hook przy każdej zmianie wysyła
   `PATCH /api/settings` do backendu bota, którego ten program nie ma (i mieć
   nie powinien — nie zapisuje konfiguracji bota, tylko własną). Klucze
   magazynu są te same, więc wybór wyglądu jest wspólny z panelem. */
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

function Kronika() {
  const { theme, palette, toggleTheme, setPalette } = useWyglad();
  /* WYBÓR JĘZYKA MUSI TU BYĆ, nie jest ozdobą paska.
     `KronikaView` bierze napisy ze słownika, a domyślnym językiem jest
     angielski. Panel przełącza go skrótem `L` z `I18nProvider`, którego ten
     program nie stawia — a `--port` domyślnie wynosi 0, więc rejestrator
     dostaje przy każdym starcie inny port, czyli INNE źródło `localStorage`
     niż panel i nigdy nie odziedziczy jego wyboru. Bez tego selektora
     `kronika.exe` byłaby po angielsku na zawsze. */
  const { lang, setLanguage, t } = useLanguage();

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
              <rect width="32" height="32" rx="8" fill="var(--accent)" />
              <g stroke="var(--bg-surface)" strokeWidth="1.8" fill="none" strokeLinecap="round">
                <path d="M9 9h14M9 14h14M9 19h9" />
              </g>
              <circle cx="23" cy="21" r="3.4" fill="var(--bg-surface)" />
              <path d="M23 19.4v1.8l1.2.7" stroke="var(--accent)" strokeWidth="1.2" fill="none" strokeLinecap="round" />
            </svg>
          </span>
          <span className="wiz__name">
            <b>{t("kron.title")}</b>
            <small>{t("kron.app.tagline")}</small>
          </span>
        </div>

        <span className="wiz__spacer" />

        <div className="wiz__tools">
          <Select<Jezyk>
            value={lang}
            onChange={setLanguage}
            size="sm"
            options={LANGUAGES.map((l) => ({ value: l.id, label: `${l.flag} ${l.native}` }))}
          />
          <Select<PaletteName>
            value={palette}
            onChange={setPalette}
            size="sm"
            /* Nazwa palety ze SŁOWNIKA (`pal.<id>`), tak samo jak w karcie
               „Wygląd" panelu. `p.label` w `useTheme.ts` jest po polsku i w
               angielskiej kronice sterczał jako „Fioletowa / Żółta". */
            options={PALETTES.map((p) => ({ value: p.id, label: t(`pal.${p.id}`) }))}
          />
          <Button
            variant="ghost"
            size="sm"
            icon={theme === "dark" ? "sun" : "moon"}
            onClick={toggleTheme}
            title={t("topbar.theme.title")}
          />
        </div>
      </header>

      <main className="wiz__body">
        <KronikaView />
      </main>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Kronika />
  </StrictMode>,
);
