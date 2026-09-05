
import { cp, mkdir, readdir, rm, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const KORZEN = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ZRODLO = join(KORZEN, "dist");
const CEL = join(KORZEN, "rust", "crates", "server", "web");

if (!existsSync(ZRODLO)) {
  console.error(`[panel→exe] BRAK ${ZRODLO} — najpierw \`vite build\``);
  process.exit(1);
}
if (!existsSync(join(ZRODLO, "index.html"))) {
  console.error("[panel→exe] dist/ bez index.html — build nie doszedl do konca");
  process.exit(1);
}

await rm(CEL, { recursive: true, force: true });
await mkdir(CEL, { recursive: true });
await cp(ZRODLO, CEL, { recursive: true });

let bajty = 0;
let ile = 0;
const obejdz = async (kat) => {
  for (const w of await readdir(kat, { withFileTypes: true })) {
    const p = join(kat, w.name);
    if (w.isDirectory()) await obejdz(p);
    else {
      bajty += (await stat(p)).size;
      ile += 1;
    }
  }
};
await obejdz(CEL);
console.log(
  `[panel→exe] ${ile} plikow, ${(bajty / 1024).toFixed(0)} KB -> rust/crates/server/web/ ` +
    `(przebuduj conduit.exe, zeby weszly do binarki)`,
);
