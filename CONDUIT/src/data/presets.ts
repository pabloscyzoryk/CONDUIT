import type { Preset } from "@/types";
import { DOMYSLNY_FORMAT, FORMATY_WBUDOWANE } from "./formaty";

/** Public preset registry used when the panel runs without the backend.
 * Runtime presets are loaded from the local `presets/` directory. */
export const PRESETS: Preset[] = [
  {
    id: "GOD-X7",
    name: "GOD-X7",
    tagline: "Public example preset; load the complete configuration from config/presets/GOD-X7.json.",
    badge: "◆",
    family: "Public example",
    format: "Synergy",
    risk: "extreme",
    metrics: { monthly: 0, winDays: 0, maxDd: 0, profitFactor: 0, worstDay: 0 },
    values: {},
  },
];

export const PRESET_FAMILIES = Array.from(new Set(PRESETS.map((p) => p.family)));

/* Format is part of preset identity: settings tuned for one signal syntax
   should not be offered silently for another parser family. */

/** Format presetu; brak pola = `ATFX` (tak samo jak `serde(default)` w Ruście). */
export function formatPresetu(p: { format?: string }): string {
  const f = (p.format ?? "").trim();
  return f === "" ? DOMYSLNY_FORMAT : f;
}

/** Tylko presety NALEŻĄCE do tego formatu. Filtr, nie sugestia. */
export function presetyDlaFormatu<T extends { format?: string }>(lista: T[], format: string): T[] {
  return lista.filter((p) => formatPresetu(p).toLowerCase() === format.toLowerCase());
}

/**
 * Presety pogrupowane po formacie, każda grupa posortowana rankingiem.
 *
 * Kolejność grup: najpierw formaty wbudowane (w kolejności z `formaty.ts`),
 * potem ewentualne nieznane — żeby dołożenie formatu po stronie silnika nie
 * wywracało układu ekranu i żeby preset o formacie, którego panel nie zna,
 * NIE ZNIKNĄŁ z listy. Zniknięcie byłoby gorsze niż dziwna nazwa grupy.
 */
export function grupujPoFormacie<T extends { name: string; format?: string }>(
  lista: T[],
): { format: string; presety: T[] }[] {
  const znane = FORMATY_WBUDOWANE.map((f) => f.nazwa);
  const obecne = Array.from(new Set(lista.map(formatPresetu)));
  const kolejnosc = [
    ...znane.filter((f) => obecne.some((o) => o.toLowerCase() === f.toLowerCase())),
    ...obecne.filter((o) => !znane.some((f) => f.toLowerCase() === o.toLowerCase())),
  ];
  return kolejnosc.map((format) => ({ format, presety: posortujPresety(presetyDlaFormatu(lista, format)) }));
}

export function findPreset(id: string): Preset | undefined {
  return PRESETS.find((p) => p.id === id);
}

/* When the backend is available, runtime presets are loaded from `presets/`.
   The small registry above is only the browser fallback catalogue. */

/** Surowy preset z `GET /api/presets`. */
export interface DiskPreset {
  name: string;
  description: string;
  settings: Record<string, unknown>;
  
  format?: string;
}

/** Rodzina, pod którą lądują presety wczytane z dysku. */
export const RODZINA_Z_DYSKU = "Z dysku";

function liczba(re: RegExp, s: string): number {
  const m = re.exec(s);
  if (!m) return 0;
  const v = Number(m[1].replace(/\s/g, "").replace(",", "."));
  return Number.isFinite(v) ? v : 0;
}

/** Pierwsze zdanie opisu — tyle, ile mieści się w karcie presetu. */
function haslo(opis: string): string {
  const jedna = opis.replace(/\s+/g, " ").trim();
  if (!jedna) return "preset z katalogu presets/";
  const kropka = jedna.search(/[.!?](\s|$)/);
  const kawalek = kropka > 20 ? jedna.slice(0, kropka) : jedna;
  return kawalek.length > 120 ? `${kawalek.slice(0, 117)}…` : kawalek;
}


/**
 * Public source distribution ranking.
 *
 * The distributed GOD-X7 preset is an example entry. Users can add and rank
 * their own presets without changing the engine.
 */
export const RANKING_PRESETOW: string[] = ["GOD-X7"];

/** Sortuje presety wg [[RANKING_PRESETOW]]; nieznane lądują niżej, alfabetycznie. */
export function posortujPresety<T extends { name: string }>(lista: T[]): T[] {
  const poz = (n: string) => {
    const i = RANKING_PRESETOW.findIndex((r) => r.toLowerCase() === n.toLowerCase());
    return i < 0 ? Number.MAX_SAFE_INTEGER : i;
  };
  return [...lista].sort((a, b) => {
    const d = poz(a.name) - poz(b.name);
    return d !== 0 ? d : a.name.localeCompare(b.name, "pl");
  });
}

/** Czy ten preset jest aktualnym czempionem (dostaje koronę)? */
export function czyCzempion(name: string): boolean {
  return name.toLowerCase() === RANKING_PRESETOW[0].toLowerCase();
}

export function presetFromDisk(p: DiskPreset): Preset {
  const opis = p.description ?? "";
  const dd = Number(p.settings?.max_dd_pct ?? 0);
  const risk: Preset["risk"] = dd <= 0 || dd > 60 ? "extreme" : dd <= 20 ? "low" : dd <= 40 ? "medium" : "high";
  return {
    id: p.name,
    name: p.name,
    tagline: haslo(opis),
    badge: "💾",
    family: RODZINA_Z_DYSKU,
    // Brak pola u serwera = ATFX. To NIE jest zgadywanie: tak samo rozstrzyga
    // `#[serde(default = "domyslny_format")]` w `crates/core/src/settings.rs`.
    format: (p.format ?? "").trim() || DOMYSLNY_FORMAT,
    risk,
    metrics: {
      monthly: liczba(/\+\s*([\d\s.,]+?)\s*USD/i, opis),
      winDays: liczba(/([\d.,]+)\s*%+\s*dni/i, opis),
      maxDd: liczba(/maxDD\s*([\d.,]+)/i, opis),
      profitFactor: liczba(/\bPF\s*([\d.,]+)/i, opis),
      worstDay: 0,
    },
    // Pełna treść presetu i tak mieszka na serwerze — `ApplyPreset` szuka go
    // po nazwie w `presets/`. Wartości wozimy tylko po to, żeby panel mógł
    // pokazać, co preset zmienia, zanim ktoś go kliknie.
    values: p.settings as Preset["values"],
  };
}
