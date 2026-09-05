import { useState } from "react";
import { Shell, type ViewId } from "@/components/layout/Shell";
import { Toasts } from "@/components/layout/Toasts";
import { I18nProvider } from "@/i18n";
import { AppProvider, useApp } from "@/store/AppStore";
import { LoginView } from "@/views/LoginView";
import { DashboardView } from "@/views/DashboardView";
import { SignalsView } from "@/views/SignalsView";
import { ChannelsView } from "@/views/ChannelsView";
import { SettingsView } from "@/views/SettingsView";
import { HistoryView } from "@/views/HistoryView";
import { SimsView } from "@/views/SimsView";
import { LogsView } from "@/views/LogsView";
import { AiModelsView } from "@/views/AiModelsView";
import { LabView } from "@/views/LabView";
import { DemoView } from "@/views/DemoView";
import { KronikaView } from "@/views/KronikaView";
import type { SettingsRequest } from "@/components/layout/CommandMenu";

/** Widok startowy z adresu: `?view=lab` otwiera od razu Laboratorium.
 *  Używa tego `conduit.exe --lab`, żeby okno wstało na właściwym ekranie. */
function widokZAdresu(): ViewId {
  if (typeof window === "undefined") return "dashboard";
  const v = new URLSearchParams(window.location.search).get("view");
  const znane: ViewId[] = [
    "dashboard",
    "signals",
    "channels",
    "settings",
    "history",
    "sims",
    "logs",
    "aimodels",
    "lab",
    "demo",
    "kronika",
  ];
  return (znane as string[]).includes(v ?? "") ? (v as ViewId) : "dashboard";
}

function Router() {
  const { loggedIn, pokazLogowanieTg } = useApp();
  const [view, setView] = useState<ViewId>(widokZAdresu);
  const [settingsRequest, setSettingsRequest] = useState<SettingsRequest>();

  // Ekran logowania pokazujemy w dwóch sytuacjach: przy pierwszym wejściu
  // ORAZ na żądanie z przycisku „Zaloguj się do Telegrama" w panelu bocznym.
  // Ta druga droga jest konieczna, bo po skasowaniu `telegram.session` nie
  // było ŻADNEGO sposobu, żeby dojść do kodu QR bez wychodzenia z terminala.
  if (!loggedIn || pokazLogowanieTg) {
    return (
      <>
        <LoginView />
        <Toasts />
      </>
    );
  }

  return (
    <>
      <Shell view={view} onView={setView} onSettings={(request) => { setSettingsRequest(request); setView("settings"); }}>
        {view === "dashboard" && <DashboardView />}
        {view === "signals" && <SignalsView />}
        {view === "channels" && <ChannelsView />}
        {view === "settings" && <SettingsView request={settingsRequest} />}
        {view === "history" && <HistoryView />}
        {view === "sims" && <SimsView />}
        {view === "logs" && <LogsView />}
        {view === "aimodels" && <AiModelsView />}
        {view === "lab" && <LabView />}
        {view === "demo" && <DemoView />}
        {view === "kronika" && <KronikaView />}
      </Shell>
      <Toasts />
    </>
  );
}

export default function App() {
  return (
    /* I18n NAD AppProviderem: toasty i logi AppStore wołają `t()` z tego
       samego modułu, a skrót L ma działać także na ekranie logowania. */
    <I18nProvider>
      <AppProvider>
        <Router />
      </AppProvider>
    </I18nProvider>
  );
}
