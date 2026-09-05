import { useT } from "@/i18n";
import { useMemo } from "react";

/* ============================================================
   KOD QR (wizualny)
   Deterministyczny wzór modułów z prawidłowymi znacznikami
   pozycjonującymi. To atrapa na potrzeby designu — logowanie
   następuje po KLIKNIĘCIU w kod, nie po zeskanowaniu.
   ============================================================ */

const SIZE = 33;

function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 0xffffffff;
  };
}

function buildMatrix(seed: number): boolean[][] {
  const r = rng(seed || 0xc0ffee);
  const m: boolean[][] = Array.from({ length: SIZE }, () => Array<boolean>(SIZE).fill(false));

  const reserved = (x: number, y: number) =>
    (x < 9 && y < 9) || (x >= SIZE - 8 && y < 9) || (x < 9 && y >= SIZE - 8) || (x >= 13 && x <= 19 && y >= 13 && y <= 19);

  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      if (reserved(x, y)) continue;
      m[y][x] = r() > 0.52;
    }
  }

  // znaczniki pozycjonujące 7×7
  const finder = (ox: number, oy: number) => {
    for (let y = 0; y < 7; y++) {
      for (let x = 0; x < 7; x++) {
        const edge = x === 0 || x === 6 || y === 0 || y === 6;
        const core = x >= 2 && x <= 4 && y >= 2 && y <= 4;
        m[oy + y][ox + x] = edge || core;
      }
    }
  };
  finder(0, 0);
  finder(SIZE - 7, 0);
  finder(0, SIZE - 7);

  // wzorce synchronizacji
  for (let i = 8; i < SIZE - 8; i++) {
    m[6][i] = i % 2 === 0;
    m[i][6] = i % 2 === 0;
  }

  // znacznik wyrównania 5×5
  const ax = SIZE - 9;
  const ay = SIZE - 9;
  for (let y = 0; y < 5; y++) {
    for (let x = 0; x < 5; x++) {
      const edge = x === 0 || x === 4 || y === 0 || y === 4;
      m[ay + y][ax + x] = edge || (x === 2 && y === 2);
    }
  }

  return m;
}

export function QrCode({ seed = 42, size = 232, hole = true }: { seed?: number; size?: number; hole?: boolean }) {
  const t = useT();
  const m = useMemo(() => buildMatrix(seed), [seed]);
  const cell = size / (SIZE + 2);
  const off = cell;
  const holeFrom = 12;
  const holeTo = 20;

  const rects: React.ReactElement[] = [];
  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      if (!m[y][x]) continue;
      if (hole && x >= holeFrom && x <= holeTo && y >= holeFrom && y <= holeTo) continue;
      const isFinder =
        (x < 7 && y < 7) || (x >= SIZE - 7 && y < 7) || (x < 7 && y >= SIZE - 7);
      rects.push(
        <rect
          key={`${x}-${y}`}
          x={off + x * cell}
          y={off + y * cell}
          width={cell * (isFinder ? 1 : 0.86)}
          height={cell * (isFinder ? 1 : 0.86)}
          rx={isFinder ? cell * 0.18 : cell * 0.36}
        />,
      );
    }
  }

  return (
    <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} className="qr" role="img" aria-label={t("login.qr.aria")}>
      <rect width={size} height={size} rx={18} fill="var(--qr-bg, #fff)" />
      <g fill="var(--qr-fg, #0b0e14)">{rects}</g>
    </svg>
  );
}
