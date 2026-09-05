

import { readFileSync } from "node:fs";

const PLIK = new URL("../src/components/ui/Icon.tsx", import.meta.url);

/* Ikony, w których PRAWIE PEŁNY ŁUK jest kanonem, nie usterką.
   Każda pozycja to decyzja projektowa z uzasadnieniem — nie skracać. */
const DUZY_LUK_DOZWOLONY = new Map([
  ["history", "kołowa strzałka zegara — pełny obrót jest istotą tego znaku"],
  ["moon", "półksiężyc powstaje z dwóch łuków, zewnętrzny obejmuje >180°"],
  ["refresh", "pętla odświeżania domyka się prawie w koło"],
]);

/* Ikony celowo mniejsze od siatki — znaki punktowe, nie rysunki. */
const MALE_DOZWOLONE = new Set(["dot"]);

const PROG_LUKU = 300; // stopni — powyżej tego kształt przestaje być łukiem, a staje się kołem
const SIATKA = 24;
const MIN_WYPELNIENIE = 8; // ikona węższa/niższa niż tyle jednostek ginie w kafelku

const NUM = /[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?/y;
const FLAGA = /[01]/y;
const ARGS = { M: 2, L: 2, H: 1, V: 1, C: 6, S: 4, Q: 4, T: 2, A: 7, Z: 0 };

/** Parser ścieżki SVG z POPRAWNĄ obsługą flag łuku (mogą być sklejone z liczbą). */
function rozbierz(d, nazwa) {
  let i = 0;
  let cmd = null;
  const out = [];
  while (i < d.length) {
    const ch = d[i];
    if (ch === " " || ch === "," || ch === "\t" || ch === "\n") {
      i += 1;
      continue;
    }
    if (/[a-zA-Z]/.test(ch)) {
      cmd = ch;
      i += 1;
      if (cmd.toUpperCase() === "Z") {
        out.push([cmd, []]);
        cmd = null;
      }
      continue;
    }
    if (cmd === null) throw new Error(`${nazwa}: liczba przed poleceniem (poz. ${i})`);
    const n = ARGS[cmd.toUpperCase()];
    if (n === undefined) throw new Error(`${nazwa}: nieznane polecenie „${cmd}"`);
    const args = [];
    for (let k = 0; k < n; k += 1) {
      while (i < d.length && " ,\t\n".includes(d[i])) i += 1;
      const luk = cmd.toUpperCase() === "A" && (k === 3 || k === 4);
      const re = luk ? FLAGA : NUM;
      re.lastIndex = i;
      const m = re.exec(d);
      if (!m) throw new Error(`${nazwa}: oczekiwano ${luk ? "flagi 0/1" : "liczby"} przy „${d.slice(i, i + 12)}"`);
      args.push(Number(m[0]));
      i = re.lastIndex;
    }
    out.push([cmd, args]);
    if (cmd === "M") cmd = "L";
    else if (cmd === "m") cmd = "l";
  }
  return out;
}


function katLuku(x1, y1, rx0, ry0, phiDeg, laf, sf, x2, y2) {
  if (rx0 === 0 || ry0 === 0) return 0;
  const phi = (phiDeg * Math.PI) / 180;
  const dx2 = (x1 - x2) / 2;
  const dy2 = (y1 - y2) / 2;
  const x1p = Math.cos(phi) * dx2 + Math.sin(phi) * dy2;
  const y1p = -Math.sin(phi) * dx2 + Math.cos(phi) * dy2;
  let rx = Math.abs(rx0);
  let ry = Math.abs(ry0);
  const lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
  if (lam > 1) {
    const s = Math.sqrt(lam);
    rx *= s;
    ry *= s;
  }
  const licz = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
  const mian = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
  let co = Math.sqrt(Math.max(licz / mian, 0));
  if (laf === sf) co = -co;
  const cxp = (co * rx * y1p) / ry;
  const cyp = (-co * ry * x1p) / rx;
  const kat = (ux, uy, vx, vy) => {
    const d = (ux * vx + uy * vy) / (Math.hypot(ux, uy) * Math.hypot(vx, vy));
    const a = Math.acos(Math.max(-1, Math.min(1, d)));
    return ux * vy - uy * vx < 0 ? -a : a;
  };
  let dd = kat((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
  if (sf === 0 && dd > 0) dd -= 2 * Math.PI;
  else if (sf === 1 && dd < 0) dd += 2 * Math.PI;
  return Math.abs((dd * 180) / Math.PI);
}

/** Geometria jednej ikony: największy łuk i prostokąt obejmujący punkty węzłowe. */
function zbadaj(nazwa, d) {
  const toks = rozbierz(d, nazwa);
  let cx = 0;
  let cy = 0;
  let sx = 0;
  let sy = 0;
  let maxKat = 0;
  const xs = [];
  const ys = [];
  for (const [cmd, a] of toks) {
    const u = cmd.toUpperCase();
    const rel = cmd === cmd.toLowerCase() && u !== "Z";
    if (u === "M") {
      [cx, cy] = rel ? [cx + a[0], cy + a[1]] : [a[0], a[1]];
      [sx, sy] = [cx, cy];
    } else if (u === "L" || u === "T") {
      [cx, cy] = rel ? [cx + a[0], cy + a[1]] : [a[0], a[1]];
    } else if (u === "H") cx = rel ? cx + a[0] : a[0];
    else if (u === "V") cy = rel ? cy + a[0] : a[0];
    else if (u === "C") [cx, cy] = rel ? [cx + a[4], cy + a[5]] : [a[4], a[5]];
    else if (u === "S" || u === "Q") [cx, cy] = rel ? [cx + a[2], cy + a[3]] : [a[2], a[3]];
    else if (u === "Z") [cx, cy] = [sx, sy];
    else if (u === "A") {
      const [nx, ny] = rel ? [cx + a[5], cy + a[6]] : [a[5], a[6]];
      maxKat = Math.max(maxKat, katLuku(cx, cy, a[0], a[1], a[2], a[3], a[4], nx, ny));
      [cx, cy] = [nx, ny];
    }
    xs.push(cx);
    ys.push(cy);
  }
  return {
    maxKat,
    minX: Math.min(...xs),
    maxX: Math.max(...xs),
    minY: Math.min(...ys),
    maxY: Math.max(...ys),
  };
}

/* ---------------- przebieg ---------------- */

const src = readFileSync(PLIK, "utf8");
const wpisy = [...src.matchAll(/^\s*"?([a-z0-9-]+)"?:\s*"([^"]+)",\s*$/gm)];
if (wpisy.length === 0) {
  console.error("[ikony kanarek] nie znalazłem ani jednej ścieżki — zmienił się kształt Icon.tsx?");
  process.exit(1);
}

const bledy = [];
for (const [, nazwa, d] of wpisy) {
  let g;
  try {
    g = zbadaj(nazwa, d);
  } catch (e) {
    bledy.push(`${nazwa}: ścieżka nie parsuje się — ${e.message}`);
    continue;
  }
  if (g.maxKat > PROG_LUKU && !DUZY_LUK_DOZWOLONY.has(nazwa)) {
    bledy.push(
      `${nazwa}: łuk ${g.maxKat.toFixed(1)}° (próg ${PROG_LUKU}°) — to koło, nie łuk. ` +
        `Najczęstsza przyczyna: flaga „większego łuku" sklejona z liczbą, np. „0 103.2" ` +
        `zamiast „0 1 0 3.2". Jeśli pełne koło jest zamierzone, dopisz ikonę do ` +
        `DUZY_LUK_DOZWOLONY Z UZASADNIENIEM.`,
    );
  }
  const poza =
    g.minX < -0.01 || g.minY < -0.01 || g.maxX > SIATKA + 0.01 || g.maxY > SIATKA + 0.01;
  if (poza) {
    bledy.push(
      `${nazwa}: wychodzi poza siatkę ${SIATKA}×${SIATKA} — ` +
        `x[${g.minX.toFixed(1)}, ${g.maxX.toFixed(1)}] y[${g.minY.toFixed(1)}, ${g.maxY.toFixed(1)}]`,
    );
  }
  const szer = g.maxX - g.minX;
  const wys = g.maxY - g.minY;
  if (!MALE_DOZWOLONE.has(nazwa) && Math.max(szer, wys) < MIN_WYPELNIENIE) {
    bledy.push(
      `${nazwa}: rysunek ${szer.toFixed(1)}×${wys.toFixed(1)} w siatce ${SIATKA} — ` +
        `ikona ginie w kaflu. Tak wyglądał błąd „undo/redo" z 25.08: zwinięte w kulkę ` +
        `zamiast rozciągnięte na siatkę.`,
    );
  }
}

if (bledy.length > 0) {
  console.error(`[ikony kanarek] CZERWONY — ${bledy.length} usterek w ${wpisy.length} ikonach:`);
  for (const b of bledy) console.error(`  · ${b}`);
  process.exit(1);
}

console.log(
  `[ikony kanarek] zielony: ${wpisy.length} ikon · geometria łuków i wypełnienie siatki sprawdzone ` +
    `· wyjątków z uzasadnieniem: ${DUZY_LUK_DOZWOLONY.size + MALE_DOZWOLONE.size}`,
);
