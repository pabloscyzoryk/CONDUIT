/* ============================================================
   BUDOWA SAMODZIELNEJ KRONIKI

   Osobna konfiguracja, a nie drugie wejście w `vite.config.ts`,
   z tych samych powodów co przy wizualizacji:
     * `npm run build` panelu ma dawać DOKŁADNIE to co dotąd —
       jeden `index.html` i nic więcej (skrypt `panel_do_exe.mjs`
       kopiuje ten katalog do binarki `conduit.exe`),
     * pakiet kroniki zawiera tylko `KronikaView` i prymitywy UI,
       więc jest wielokrotnie mniejszy niż pakiet panelu.

   Wynik ląduje od razu w `rust/crates/kronika/web/`, skąd
   `rust-embed` wkompilowuje go w `kronika.exe`.

   Uruchamianie:  npm run build:kronika
   ============================================================ */

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";
import { rename } from "node:fs/promises";
import { join } from "node:path";

const OUT = fileURLToPath(new URL("./rust/crates/kronika/web", import.meta.url));

export default defineConfig({
  plugins: [
    react(),
    {
      /* Vite nazywa wyjście po pliku wejściowym (`kronika.html`).
         Serwer w Ruście ma być głupi i szukać `index.html` — zamiana
         nazwy tutaj jest tańsza niż wyjątek w kodzie serwera. */
      name: "kronika-index",
      closeBundle: async () => {
        await rename(join(OUT, "kronika.html"), join(OUT, "index.html"));
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
      input: fileURLToPath(new URL("./kronika.html", import.meta.url)),
    },
  },
});
