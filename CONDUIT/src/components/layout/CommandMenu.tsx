import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "@/components/ui";
import { GENERAL_GROUPS, MANAGEMENT_GROUPS, AI_GROUP, COVERED_KEYS } from "@/data/settingsSchema";
import { DEFAULT_SETTINGS } from "@/data/defaultSettings";
import { przetlumaczGrupy } from "@/i18n/schema";
import { useLanguage, useT } from "@/i18n";
import { matchesSearch, searchScore } from "@/lib/search";
import type { ViewId } from "./Shell";
import type { SettingKey } from "@/types";

export type SettingsRequest = { query: string; raw: boolean; stamp: number };
type Destination = { id: string; label: string; detail: string; icon: IconName; view?: ViewId; raw?: boolean; anchor?: string };

export function CommandMenu({ open, onClose, onView, onSettings, views }: {
  open: boolean; onClose: () => void; onView: (view: ViewId) => void;
  onSettings: (request: SettingsRequest) => void; views: { id: ViewId; icon: IconName }[];
}) {
  const t = useT();
  const { lang } = useLanguage();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const dialog = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const choices = useMemo<Destination[]>(() => {
    const navigation = views.map(v => ({ id: v.id, label: t(`nav.${v.id}`), detail: t(`nav.${v.id}.hint`), icon: v.icon, view: v.id }));
    const schema = przetlumaczGrupy([...GENERAL_GROUPS, ...MANAGEMENT_GROUPS, AI_GROUP], lang);
    const axes = schema.flatMap(group => group.fields.map(field => ({
      id: String(field.key), label: field.label, detail: `${group.title} · ${String(field.key)}`,
      icon: "sliders" as const, keywords: field.hint,
    })));
    const raw = Object.keys(DEFAULT_SETTINGS).filter(key => !COVERED_KEYS.has(key as SettingKey) && key !== "merge_config")
      .map(key => ({ id: key, label: key, detail: t("command.raw"), icon: "microscope" as const, raw: true }));
    const lots = ["lot_mode_percent", "lot_fixed", "lot_percent", "lot_percent_small", "lot_percent_small_mult"]
      .map(key => ({ id: key, label: t("lotauto.card.title"), detail: `${t("nav.dashboard")} · ${key}`, icon: "layers" as const, view: "dashboard" as const, anchor: "lot-auto" }));
    const all = [...navigation, ...axes, ...raw, ...lots];
    const unique = all.filter((item, index) => all.findIndex(other => other.id === item.id) === index);
    return query.trim() ? unique.filter(item => matchesSearch(query, item.label, item.detail, "keywords" in item ? item.keywords as string : ""))
      .sort((a, b) => searchScore(query, b.id, b.label) - searchScore(query, a.id, a.label)).slice(0, 70) : navigation;
  }, [query, lang, t, views]);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery(""); setSelected(0);
    input.current?.focus();
    return () => previous?.focus();
  }, [open]);
  useEffect(() => { setSelected(0); }, [query]);
  useEffect(() => { dialog.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" }); }, [selected]);

  function choose(item: Destination) {
    if (item.view) {
      onView(item.view);
      if (item.anchor) requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById(item.anchor!)?.scrollIntoView({ block: "start" })));
    }
    else onSettings({ query: item.id, raw: !!item.raw, stamp: Date.now() });
    onClose();
  }
  if (!open) return null;
  return createPortal(
    <div className="command-scrim" onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="command" role="dialog" aria-modal="true" aria-label={t("command.title")} ref={dialog}
        onKeyDown={e => {
          if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); }
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault(); setSelected(i => Math.max(0, Math.min(choices.length - 1, i + (e.key === "ArrowDown" ? 1 : -1))));
          }
          if (e.key === "Enter" && e.target === input.current && choices[selected]) { e.preventDefault(); choose(choices[selected]); }
          if (e.key === "Tab") {
            const nodes = dialog.current?.querySelectorAll<HTMLElement>('input, button:not([tabindex="-1"])');
            if (!nodes?.length) return;
            const first = nodes[0], last = nodes[nodes.length - 1];
            if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
            if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
          }
        }}>
        <div className="command__search">
          <Icon name="search" size={21} />
          <input ref={input} value={query} onChange={e => setQuery(e.target.value)} placeholder={t("command.placeholder")}
            role="combobox" aria-expanded="true" aria-controls="command-options" aria-autocomplete="list"
            aria-activedescendant={choices[selected] ? `command-${choices[selected].id}` : undefined} aria-label={t("command.title")} />
          <button onClick={onClose} title={t("common.close")}><kbd>Esc</kbd></button>
        </div>
        <div className="command__caption">{query.trim() ? t("command.matches", { n: choices.length }) : t("command.navigate")}</div>
        <div className="command__results" role="listbox" id="command-options">
          {choices.map((item, i) => <button key={item.id} id={`command-${item.id}`} role="option" aria-selected={selected === i}
            className="command__item" tabIndex={-1} onMouseMove={() => setSelected(i)} onClick={() => choose(item)}>
            <Icon name={item.icon} size={18} /><span><b>{item.label}</b><small>{item.detail}</small></span><Icon name="chevron-right" size={15} />
          </button>)}
          {!choices.length && <p className="command__empty">{t("command.empty")}</p>}
        </div>
        <footer className="command__footer"><span>↑ ↓ &nbsp; {t("command.move")}</span><span>Enter &nbsp; {t("command.open")}</span><span>Ctrl K</span></footer>
      </div>
    </div>, document.body,
  );
}
