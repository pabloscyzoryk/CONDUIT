import { Component, type ReactNode } from "react";
import { t } from "@/i18n";

/** Keep a recoverable screen instead of a blank window when one UI view fails.
 * This boundary sends no commands and does not reset server/local settings. */
export class AppErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    if (!this.state.failed) return this.props.children;
    return <div className="app-recovery" role="alert"><div>
      <span className="eyebrow">CONDUIT</span>
      <h1>{t("app.recovery.title")}</h1>
      <p>{t("app.recovery.text")}</p>
      <button className="btn btn--primary" onClick={() => window.location.reload()}>{t("app.recovery.reload")}</button>
    </div></div>;
  }
}
