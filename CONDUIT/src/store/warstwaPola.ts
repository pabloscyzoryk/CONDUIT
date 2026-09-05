

import { SETTINGS_SCHEMA, type ZakresUstawien } from "@/data/settingsSchema";
import { POLA_RACHUNKU } from "@/store/polaRachunku.generated";
import type { SettingKey, Settings } from "@/types";

/** Pola, których warstwa RÓŻNI SIĘ od warstwy grupy, w której stoją. */
export const NADPISANIA_WARSTWY: Partial<Record<SettingKey, ZakresUstawien>> = {
  /* Grupa `parity` (rachunek) — a lot należy do presetu nogi.
     Dwie nogi na jednym rachunku MAJĄ prawo grać różnym lotem. */
  lot_min: "preset",
  lot_max: "preset",
  /* Grupa `panel` (rachunek) — alias na `basket_hint_tolerance`, które
     silnik czyta z presetu nogi (`settings_map.rs`). Jedyny alias panelu,
     który wychodzi poza warstwę rachunku. */
  price_tol: "preset",
  /* Grupa `gridextra` (preset) — a dźwignia jest własnością KONTA i stoi
     w `POLA_RACHUNKU`. Zapis do pliku presetu i tak zostałby nadpisany
     dokumentem przy składaniu ustawień formatu. */
  konto_dzwignia: "rachunek",
};

/** Warstwa POLA: nadpisanie wygrywa nad warstwą grupy. */
export function zakresPola(klucz: SettingKey, zakresGrupy: ZakresUstawien): ZakresUstawien {
  return NADPISANIA_WARSTWY[klucz] ?? zakresGrupy;
}

/** Czy pole stoi w grupie o INNEJ warstwie — wtedy trzeba to napisać przy polu. */
export function warstwaInnaNizGrupa(klucz: SettingKey, zakresGrupy: ZakresUstawien): boolean {
  const w = NADPISANIA_WARSTWY[klucz];
  return w !== undefined && w !== zakresGrupy;
}

export interface OpisWarstwyPola {
  zakres: ZakresUstawien;
  /** Source attribution, not a claim of full strategy/axis verification. */
  dowod: string;
  nieobslugiwane?: boolean;
}

/* UI-only keys have no core field to infer ownership from. Keep their scope
   explicit. Archived keys remain readable for compatibility; the mapper
   explicitly ignores them, so presenting an operative switch would mislead. */
const POLA_POZA_SCHEMATEM: Partial<Record<SettingKey, OpisWarstwyPola>> = {
  day_target_pct: { zakres: "preset", dowod: "settings_map::core_from_ui → Settings.day_target_pct; outside POLA_RACHUNKU" },
  day_trail_arm_pct: { zakres: "preset", dowod: "settings_map::core_from_ui → Settings.day_trail_arm_pct; outside POLA_RACHUNKU" },
  day_trail_stop_pct: { zakres: "preset", dowod: "settings_map::core_from_ui → Settings.day_trail_stop_pct; outside POLA_RACHUNKU" },
  alllogs_dir: { zakres: "rachunek", dowod: "UI/runtime log output directory; settings_map known operational keys" },
  price_log: { zakres: "rachunek", dowod: "settings_map::UI_ONLY_KEYS; runtime logging" },
  price_log_interval_s: { zakres: "rachunek", dowod: "settings_map::UI_ONLY_KEYS; runtime logging" },
  merge_chronological: { zakres: "rachunek", dowod: "settings_map::UI_ONLY_KEYS; merged logs" },
  merge_config: { zakres: "rachunek", dowod: "settings_map::UI_ONLY_KEYS; merged logs" },
  allow_mt5_modify: { zakres: "rachunek", dowod: "settings_map::UI_ONLY_KEYS: archived, ignored", nieobslugiwane: true },
  spp_refresh_after_modify: { zakres: "preset", dowod: "settings_map::UI_ONLY_KEYS: archived, ignored", nieobslugiwane: true },
  grid_fallback_best_edge: { zakres: "preset", dowod: "settings_map::UI_ONLY_KEYS: archived, ignored", nieobslugiwane: true },
  sim_clock_strict: { zakres: "rachunek", dowod: "settings_map::UI_ONLY_KEYS: archived, ignored", nieobslugiwane: true },
};

/** Every modeled UI field has attributable ownership; unknown new keys fail
 * closed instead of silently becoming a global or selected-preset write. */
export function opisWarstwyPola(klucz: SettingKey): OpisWarstwyPola | null {
  if ((POLA_RACHUNKU as readonly string[]).includes(klucz)) {
    return { zakres: "rachunek", dowod: "core/wielosilnik.rs::POLA_RACHUNKU (generated UI aliases)" };
  }
  const poza = POLA_POZA_SCHEMATEM[klucz];
  if (poza) return poza;
  for (const grupa of SETTINGS_SCHEMA) {
    if (grupa.fields.some((pole) => pole.key === klucz)) {
      return { zakres: zakresPola(klucz, grupa.zakres), dowod: NADPISANIA_WARSTWY[klucz]
        ? "warstwaPola::NADPISANIA_WARSTWY (audited alias/owner correction)"
        : `settingsSchema:${grupa.id} (UI declaration; semantics require separate verification)` };
    }
  }
  return null;
}

export interface EdycjaUstawienPresetu {
  nazwa: string;
  doc: Settings;
  set: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
}

/** The selected owner is captured at render time. Never re-resolve a different
 * preset when an input/confirmation commits later. No writes occur on bind. */
export function powiazPoleUstawien<K extends SettingKey>(
  klucz: K,
  globalny: { settings: Settings; setSetting: <T extends SettingKey>(key: T, value: Settings[T]) => void },
  edycja: EdycjaUstawienPresetu | null,
  zablokowane = false,
) {
  const opis = opisWarstwyPola(klucz);
  const doPresetu = opis?.zakres === "preset" && edycja !== null;
  return {
    opis,
    wlasciciel: doPresetu ? edycja.nazwa : null,
    value: doPresetu ? edycja.doc[klucz] : globalny.settings[klucz],
    zablokowane: zablokowane || opis === null || !!opis.nieobslugiwane,
    set: (value: Settings[K]) => {
      if (zablokowane || opis === null || opis.nieobslugiwane) return;
      if (doPresetu) edycja.set(klucz, value);
      else globalny.setSetting(klucz, value);
    },
  };
}
