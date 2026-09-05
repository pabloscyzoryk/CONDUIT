import { useCallback, useEffect, useState } from "react";
import type { PaletteName, ThemeName } from "@/types";
import * as storage from "./storage";
import { api } from "./transport";



/** Nazwy kluczy w `settings.json`. Jedno miejsce — używa ich też AppStore. */
export const KLUCZ_MOTYWU = "ui_theme";
export const KLUCZ_PALETY = "ui_palette";

/**
 * Nakłada wygląd przysłany przez serwer.
 *
 * Wołane przez `AppStore`, gdy dotrze migawka stanu. Nie dotyka Reacta —
 * ustawia atrybuty na `<html>` i magazyn lokalny, więc kolejne wywołanie
 * `useTheme()` wystartuje już z właściwą wartością.
 */
export function applyServerAppearance(theme?: string, palette?: string): void {
  if (theme === "dark" || theme === "light") {
    document.documentElement.dataset.theme = theme;
    storage.save("theme", theme);
  }
  if (palette && PALETTES.some((p) => p.id === palette)) {
    document.documentElement.dataset.palette = palette;
    storage.save("palette", palette);
  }
}

export const PALETTES: { id: PaletteName; label: string; swatch: string; note: string }[] = [
  { id: "violet", label: "Fioletowa", swatch: "#6d7bff", note: "domyślna — indygo" },
  { id: "red", label: "Czerwona", swatch: "#ff4d5e", note: "strata przesunięta w głębszy karmazyn" },
  { id: "green", label: "Zielona", swatch: "#22c55e", note: "zysk przesunięty w jaśniejszą miętę" },
  { id: "yellow", label: "Żółta", swatch: "#f5c518", note: "ciemny napis na akcencie" },
  { id: "orange", label: "Pomarańczowa", swatch: "#ff7a1a", note: "ciepła, wysokokontrastowa" },
  { id: "matrix", label: "Matrix", swatch: "#00ff41", note: "czerń terminala, font monospace" },
  { id: "vantage", label: "Vantage", swatch: "#00c389", note: "granat + zieleń (inspirowana)" },
  { id: "puprime", label: "PU Prime", swatch: "#b14aed", note: "fiolet + magenta (inspirowana)" },
];

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeName>(() => storage.load<ThemeName>("theme", "dark"));
  const [palette, setPaletteState] = useState<PaletteName>(() => storage.load<PaletteName>("palette", "violet"));

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    storage.save("theme", theme);
    // brak backendu jest normalny — cisza zamiast błędu
    void api.patchSettings({ [KLUCZ_MOTYWU]: theme }).catch(() => undefined);
  }, [theme]);

  useEffect(() => {
    document.documentElement.dataset.palette = palette;
    storage.save("palette", palette);
    void api.patchSettings({ [KLUCZ_PALETY]: palette }).catch(() => undefined);
  }, [palette]);

  /** Krótka, jednorazowa animacja przejścia kolorów (nie obciąża scrolla). */
  const animate = useCallback(() => {
    const root = document.documentElement;
    root.classList.add("theme-anim");
    window.setTimeout(() => root.classList.remove("theme-anim"), 320);
  }, []);

  const setTheme = useCallback(
    (t: ThemeName) => {
      animate();
      setThemeState(t);
    },
    [animate],
  );

  const setPalette = useCallback(
    (p: PaletteName) => {
      animate();
      setPaletteState(p);
    },
    [animate],
  );

  const toggle = useCallback(
    () => setTheme(document.documentElement.dataset.theme === "light" ? "dark" : "light"),
    [setTheme],
  );

  return { theme, setTheme, toggle, palette, setPalette };
}
