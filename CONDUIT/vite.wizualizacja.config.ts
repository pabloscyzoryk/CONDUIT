/* ============================================================
   BUDOWA SAMODZIELNEJ WIZUALIZACJI MODELI AI

   Osobna konfiguracja, a nie drugie wejście w `vite.config.ts`,
   z dwóch powodów:
     * `npm run build` panelu ma dawać DOKŁADNIE to co dotąd —
       jeden `index.html` i nic więcej (skrypt `panel_do_exe.mjs`
       kopiuje ten katalog do binarki `conduit.exe`),
     * pakiet wizualizacji zawiera tylko `AiModelsView` i prymitywy
       UI, więc jest kilkukrotnie mniejszy niż pakiet panelu.

   Wynik ląduje od razu w `rust/crates/wizualizacja/web/`, skąd
   `rust-embed` wkompilowuje go w `wizualizacja.exe`.

   Uruchamianie:  npm run build:wiz
   ============================================================ */

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";
import { rename } from "node:fs/promises";
import { join } from "node:path";

const OUT = fileURLToPath(new URL("./rust/crates/wizualizacja/web", import.meta.url));

export default defineConfig({
  plugins: [
    react(),
    {
      /* Vite nazywa wyjście po pliku wejściowym (`wizualizacja.html`).
         Serwer w Ruście ma być głupi i szukać `index.html` — zamiana
         nazwy tutaj jest tańsza niż wyjątek w kodzie serwera. */
      name: "wizualizacja-index",
      closeBundle: async () => {
        await rename(join(OUT, "wizualizacja.html"), join(OUT, "index.html"));
      },
    },
  ],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  build: {
    outDir: OUT,
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: {
      input: fileURLToPath(new URL("./wizualizacja.html", import.meta.url)),
    },
  },
});
