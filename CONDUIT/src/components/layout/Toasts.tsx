import { tSilnik } from "@/i18n/silnik";
import { Icon } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import type { Toast } from "@/types";

const ICON: Record<Toast["kind"], "check" | "alert" | "info" | "x"> = {
  success: "check",
  error: "x",
  warn: "alert",
  info: "info",
};

export function Toasts() {
  const { toasts, dismissToast } = useApp();
  const tt = useT();
  if (!toasts.length) return null;
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.kind}`}>
          <span className="toast__icon">
            <Icon name={ICON[t.kind]} size={14} strokeWidth={2.4} />
          </span>
          <div className="toast__body">
            <span className="toast__title">{tSilnik(t.title)}</span>
            {t.text && <span className="toast__text">{tSilnik(t.text)}</span>}
            {}
            {!!t.actions?.length && (
              <span className="toast__actions">
                {t.actions.map((a) => (
                  <button
                    key={tSilnik(a.label)}
                    className="toast__action"
                    onClick={() => {
                      a.onClick();
                      dismissToast(t.id);
                    }}
                  >
                    {tSilnik(a.label)}
                  </button>
                ))}
              </span>
            )}
          </div>
          <button className="toast__close" onClick={() => dismissToast(t.id)} aria-label={tt("common.close")}>
            <Icon name="x" size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
