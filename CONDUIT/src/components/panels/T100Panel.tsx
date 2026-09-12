import { useEffect, useId, useState } from "react";
import { Badge, Button, Card, Checkbox, Field } from "@/components/ui";
import { useT, type KluczTlumaczenia } from "@/i18n";
import { DEFAULT_SETTINGS } from "@/data/defaultSettings";
import { T100_BASIC_KEYS, T100_KEYS, t100Document, t100Errors, t100Number, type T100Key } from "@/store/t100Settings";
import type { T100Config, TradingMode } from "@/types";

export interface T100PanelProps {
  value: unknown;
  mode: TradingMode;
  owner: string | null;
  blocked: boolean;
  onSave: (config: T100Config) => boolean;
}

/** A local draft is committed to ONE captured owner, never to every engine. */
export function T100Panel({ value, mode, owner, blocked, onSave }: T100PanelProps) {
  const t = useT();
  const id = useId();
  const source = JSON.stringify(value);
  const [draft, setDraft] = useState<T100Config | null>(() => t100Document(value));
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    setDraft(t100Document(value));
    setRevision(n => n + 1);
    // The value's contents, not each snapshot object identity, own the draft.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, owner]);
  const errors: readonly string[] = t100Errors(draft);
  const initial = t100Document(value);
  const enableBlocked = mode !== "AUTO-EA" && draft?.enabled === true && initial?.enabled !== true;
  const dirty = JSON.stringify(draft) !== JSON.stringify(initial);
  const fieldText = (kind: "field" | "hint", key: T100Key) => t(`t100.${kind}.${key}` as KluczTlumaczenia);
  const change = (key: T100Key, next: unknown) => {
    if (key === "enabled" && next === true && mode !== "AUTO-EA") return;
    setDraft(current => current ? { ...current, [key]: next } as T100Config : current);
  };
  const restoreDraft = (next: T100Config | null) => { setDraft(next); setRevision(n => n + 1); };

  const field = (key: T100Key) => {
    const current = draft?.[key];
    const fieldId = `${id}-${key}`;
    const invalid = errors.includes(key);
    return <div key={key} data-t100-key={key}>
      <Field label={fieldText("field", key)} htmlFor={fieldId} hint={fieldText("hint", key)} warn={invalid ? t("t100.invalidField") : null}>
        {key === "signal_required" || key === "enabled" ? (
          <select id={fieldId} className="select" value={typeof current === "boolean" ? String(current) : "invalid"}
            onChange={event => change(key, event.target.value === "true")} aria-invalid={invalid}>
            {typeof current !== "boolean" && <option value="invalid" disabled>{t("t100.bool.invalid")}</option>}
            <option value="false">{key === "enabled" ? t("t100.inactive") : t("t100.bool.false")}</option>
            <option value="true" disabled={key === "enabled" && mode !== "AUTO-EA"}>{key === "enabled" ? "T-100" : t("t100.bool.true")}</option>
          </select>
        ) : key === "experts" ? (
          <div id={fieldId} className="row" role="group" aria-label={fieldText("field", key)} style={{ flexWrap: "wrap" }}>
            {([1, 2, 4, 8] as const).map(bit => <Checkbox key={bit}
              checked={typeof current === "number" && Number.isInteger(current) && (current & bit) !== 0}
              onChange={checked => {
                const mask = typeof current === "number" && Number.isInteger(current) && current >= 0 && current <= 15 ? current : 0;
                change(key, checked ? mask | bit : mask & ~bit);
              }} label={t(`t100.expert.${bit}`)} />)}
          </div>
        ) : (
          <input key={`${revision}:${key}`} id={fieldId} className="input input--num" type="text" inputMode="decimal"
            defaultValue={typeof current === "number" && Number.isNaN(current) ? "" : String(current)}
            onChange={event => change(key, t100Number(event.target.value))}
            onKeyDown={event => { if (event.key === "Enter") event.currentTarget.blur(); }} aria-invalid={invalid} />
        )}
      </Field>
    </div>;
  };

  return <Card title={t("t100.title")} icon="robot" actions={<Badge tone="warn">{t("t100.experimental")}</Badge>}>
    <p className="hint">{owner ? t("set.scope.presetName", { name: owner }) : t("t100.ownerDocument")}</p>
    {blocked ? <p role="status">{t("t100.blocked")}</p> : <>
      <div className="row" style={{ flexWrap: "wrap" }}>
        <Badge tone={initial?.enabled === true ? "warn" : "muted"}>
          {initial?.enabled === true ? t("t100.active") : t("t100.inactive")}
        </Badge>
        {dirty && <span role="status" className="hint">{t("t100.dirty")}</span>}
      </div>
      <p className="hint">{t("t100.configured")}</p>
      <p>{t("t100.routing")}</p>
      {mode !== "AUTO-EA" && <p className="halt" role="status">{t("t100.requiresMode", { mode })}</p>}
      <details className="context-help"><summary>{t("set.help.section")}</summary><p>{t("t100.intro")}</p></details>
      {errors.length > 0 && <p className="halt" role="alert">
        {errors.includes("object") ? t("t100.invalidObject") : t("t100.invalid")}
      </p>}
      {draft && <>
        <div className="setgrid">{field("enabled")}</div>
        {draft.enabled !== false && <>
          <div className="setgrid">{field("experts")}{T100_BASIC_KEYS.map(field)}</div>
        </>}
        <details className="context-help" style={{ marginTop: "var(--sp-3)" }}>
          <summary>{t("t100.advanced")}</summary>
          <div className="setgrid">{T100_KEYS.filter(key => key !== "enabled" && (draft.enabled === false || (key !== "experts" && !T100_BASIC_KEYS.includes(key)))).map(field)}</div>
        </details>
      </>}
      <div className="row" style={{ flexWrap: "wrap", marginTop: "var(--sp-3)" }}>
        <Button variant="primary" disabled={!dirty || errors.length > 0 || enableBlocked} onClick={() => { if (draft && !enableBlocked && t100Errors(draft).length === 0) onSave(draft); }}>{t("t100.save")}</Button>
        <Button variant="outline" disabled={!dirty} onClick={() => restoreDraft(t100Document(value))}>{t("t100.revert")}</Button>
        <Button variant="ghost" onClick={() => restoreDraft({ ...DEFAULT_SETTINGS.t100 })}>{t("t100.reset")}</Button>
      </div>
    </>}
  </Card>;
}
