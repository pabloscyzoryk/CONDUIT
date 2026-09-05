import type { LabSourceCounts } from "@/store/transport";
import type { KluczTlumaczenia } from "@/i18n";
import { num } from "./format";

export function optionalCount(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? num(value, 0) : "—";
}

export function entrySourceCounts(source: LabSourceCounts): { label: KluczTlumaczenia; value: number | null | undefined }[] {
  return [
    { label: "lab.sources.known", value: source.knownEntrySources },
    { label: "lab.sources.full", value: source.knownFullEntrySources },
    { label: "lab.sources.firstEdit", value: source.entrySourcesFirstSeenAsEdit },
  ];
}
