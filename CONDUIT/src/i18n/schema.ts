/* ============================================================
   TŁUMACZENIE SCHEMATU USTAWIEŃ — scalanie nakładki PO KLUCZU pola.

   Wariant z JEZYKI_SPEC.md wybrany ŚWIADOMIE: `settingsSchema.ts`
   zostaje ŹRÓDŁEM PRAWDY (klucze pól, typy, zakresy, `when`) i nie
   zmienia się ani o bajt — kanarek Rusta
   (`zaden_klucz_presetu_nie_ginie_po_tlumaczeniu`) pilnuje kluczy
   silnika, a ta nakładka podmienia WYŁĄCZNIE napisy wyświetlane:
   tytuł grupy, opis, `label`, `hint`, etykiety `options` i funkcję
   `warn`. Pole nieobecne w nakładce zostaje po polsku (nigdy puste)
   i loguje ostrzeżenie w konsoli dev — dokładnie ta sama zasada
   „fallback zamiast dziury" co w słowniku płaskim.
   ============================================================ */

import type { FieldDef, GroupDef } from "@/data/settingsSchema";
import { SCHEMA_EN } from "./settingsSchema.en";
import type { Jezyk } from "./index";

export interface PoleEn {
  label?: string;
  hint?: string;
  /** etykiety opcji selecta po `value` — wartości NIE wolno ruszać */
  options?: Record<string, string>;
  /** angielska wersja ostrzeżenia — funkcja tego samego kształtu co w schemacie */
  warn?: FieldDef["warn"];
}

export interface GrupaEn {
  title?: string;
  desc?: string;
  fields?: Record<string, PoleEn>;
}

const zgloszone = new Set<string>();
function ostrzez(co: string): void {
  if (zgloszone.has(co)) return;
  zgloszone.add(co);
  console.warn(`[i18n/schema] brak tłumaczenia EN: ${co} — zostaje polski oryginał`);
}

function przetlumaczPole(f: FieldDef, pole: PoleEn | undefined, grupa: string): FieldDef {
  if (!pole) {
    ostrzez(`${grupa}.${String(f.key)}`);
    return f;
  }
  return {
    ...f,
    label: pole.label ?? f.label,
    hint: pole.hint !== undefined ? pole.hint : f.hint,
    unit: f.unit === "pkt" ? "pt" : f.unit === "dni" ? "days" : f.unit === "lot" ? "lots" : f.unit,
    options: pole.options
      ? f.options?.map((o) => ({ ...o, label: pole.options![o.value] ?? o.label }))
      : f.options,
    warn: pole.warn ?? f.warn,
  };
}

/**
 * Zwraca grupy z napisami w żądanym języku. Dla polskiego oddaje
 * WEJŚCIOWE obiekty bez kopiowania — schemat JEST polski.
 */
export function przetlumaczGrupy(grupy: GroupDef[], lang: Jezyk): GroupDef[] {
  if (lang === "pl") return grupy;
  return grupy.map((g) => {
    const nakladka = SCHEMA_EN[g.id];
    if (!nakladka) {
      ostrzez(`grupa ${g.id}`);
      return g;
    }
    return {
      ...g,
      title: nakladka.title ?? g.title,
      desc: nakladka.desc ?? g.desc,
      fields: g.fields.map((f) => przetlumaczPole(f, nakladka.fields?.[String(f.key)], g.id)),
    };
  });
}
