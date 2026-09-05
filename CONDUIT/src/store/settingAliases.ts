import type { SettingKey, Settings } from "@/types";

/** Only explicit edits use these patches. Loading/rendering never rewrites a preset.
 * The backend's existing canonical-value precedence remains unchanged. */
export type SettingControlPatch = Partial<Settings> & Record<string, unknown>;
type Document = Partial<Settings> & { smart_sl_delay_n?: unknown };
type TpSource = Settings["tp_source"];
const TP_SOURCES: readonly string[] = ["Either", "PriceOnly", "SignalOnly", "SignalConfirmedByPrice", "PriceFirstSignalWindow"];

export function effectiveTpSource(doc: Document): TpSource {
  if (typeof doc.tp_source === "string") return TP_SOURCES.includes(doc.tp_source) ? doc.tp_source : "Either";
  const price = doc.tp_detect_price === true;
  const signal = doc.tp_detect_signal === true;
  return price ? (signal ? "Either" : "PriceOnly") : (signal ? "SignalOnly" : "PriceOnly");
}

export function effectiveSmartSlDelay(doc: Document): number {
  const n = doc.smart_sl_delay_n;
  return typeof n === "number" && Number.isFinite(n) ? Math.max(0, Math.trunc(n)) : (doc.trail_after_tp2 ? 1 : 0);
}

function detectionPair(source: TpSource) {
  return { tp_detect_price: source !== "SignalOnly", tp_detect_signal: source !== "PriceOnly" };
}

export function settingControlValue(doc: Document, key: SettingKey): unknown {
  if (key === "trail_after_tp2") return effectiveSmartSlDelay(doc) > 0;
  if (key === "tp_detect_price" || key === "tp_detect_signal") return detectionPair(effectiveTpSource(doc))[key];
  return doc[key];
}

/** Sibling keys travel in ONE patch to ONE owner, not multiple asynchronous writes. */
export function settingControlPatch(doc: Document, key: SettingKey, value: unknown): SettingControlPatch {
  if (key === "trail_after_tp2") {
    const enabled = value === true;
    return { trail_after_tp2: enabled, smart_sl_delay_n: enabled ? 1 : 0 };
  }
  if (key === "tp_source" && typeof value === "string" && TP_SOURCES.includes(value)) {
    const source = value as TpSource;
    return { tp_source: source, ...detectionPair(source) };
  }
  if (key === "tp_detect_price" || key === "tp_detect_signal") {
    const pair = { ...detectionPair(effectiveTpSource(doc)), [key]: value === true };
    // Legacy mapper's explicit fallback: both OFF means PriceOnly, never disabling all TP detection.
    const source: TpSource = pair.tp_detect_price
      ? (pair.tp_detect_signal ? "Either" : "PriceOnly")
      : (pair.tp_detect_signal ? "SignalOnly" : "PriceOnly");
    return { tp_source: source, ...detectionPair(source) };
  }
  return { [key]: value } as SettingControlPatch;
}

export function isAdvancedTpSource(doc: Document): boolean {
  return ["SignalConfirmedByPrice", "PriceFirstSignalWindow"].includes(effectiveTpSource(doc));
}
