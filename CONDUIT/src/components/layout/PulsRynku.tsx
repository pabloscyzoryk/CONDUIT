

import { useEffect, useState } from "react";
import { Icon, Tooltip } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT, wiek } from "@/i18n";
import "./cofnij.css";
import { quoteUtcTime, quoteState } from "@/lib/clock";

/** Powyżej tego wieku kwotowanie przestaje być „teraz". */
export const PROG_ZOLTY_MS = 20_000;
/** Powyżej tego wieku traktujemy strumień jak martwy. */
export const PROG_CZERWONY_MS = 90_000;

/** Zegar przerysowujący — bez niego martwy strumień zamraża też licznik.
 *  To jest sedno: gdy dane przestają przychodzić, panel przestaje się
 *  przerysowywać i wiek zatrzymuje się na ostatniej wartości. */
export function useTykanie(ms = 5000): number {
  const [teraz, setTeraz] = useState(Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setTeraz(Date.now()), ms);
    return () => window.clearInterval(id);
  }, [ms]);
  return teraz;
}

export type StanKwotowania = "brak" | "nieznane" | "swieze" | "stare" | "martwe";

export function stanKwotowania(czasKwotowania: number | null, teraz: number): StanKwotowania {
  return quoteState(czasKwotowania, teraz);
}

/** Znacznik wieku obok ceny. Przy świeżym kwotowaniu nie zajmuje miejsca. */
export function WiekKwotowania() {
  const app = useApp();
  const tt = useT();
  const teraz = useTykanie();
  const q = app.primary;
  const utc = quoteUtcTime(q, app.live);
  const stan = !q.time ? "brak" : stanKwotowania(utc, teraz);
  if (stan === "swieze") return null;

  const tresc =
    stan === "nieznane" ? tt("clock.ageUnknown") : stan === "brak"
      ? tt("puls.quoteNone")
      : stan === "martwe"
        ? tt("puls.quoteStale", { v: utc === null ? tt("clock.ageUnknown") : wiek(teraz - utc) })
        : tt("puls.quoteAge", { v: utc === null ? tt("clock.ageUnknown") : wiek(teraz - utc) });

  return (
    <Tooltip content={tresc}>
      <span className="puls" data-stan={stan}>
        <Icon name="clock" size={11} />
        {stan === "brak" ? "—" : utc === null ? tt("clock.ageUnknown") : wiek(teraz - utc)}
      </span>
    </Tooltip>
  );
}

/**
 * Zdrowie mostu MT5 — kropka + opóźnienie + wiek kwotowania w jednym.
 *
 * Stan „połączony, ale kwotowania stoją" ma WŁASNY kolor. To jest jedyny
 * sposób odróżnienia spokojnej sesji od zniknięcia terminala: `mt5`
 * mówi o zestawionym połączeniu, nie o tym, czy cokolwiek płynie.
 */
export function ZdrowieMT5() {
  const app = useApp();
  const tt = useT();
  const teraz = useTykanie();
  const c = app.connection;
  const q = app.primary;
  const utc = quoteUtcTime(q, app.live);
  const stan = !q.time ? "brak" : stanKwotowania(utc, teraz);

  const polaczony = c.mt5 === "connected";
  const ton = !polaczony ? "off" : stan === "martwe" || stan === "brak" ? "dead" : stan === "stare" || stan === "nieznane" ? "warn" : "ok";

  const tresc = !polaczony
    ? tt("puls.mt5.off")
    : stan === "nieznane" ? tt("clock.ageUnknown") : ton === "dead"
      ? tt("puls.mt5.stale", { v: stan === "brak" ? "—" : utc === null ? tt("clock.ageUnknown") : wiek(teraz - utc) })
      : tt("puls.mt5.ok", {
          v: c.latencyMs,
          q: stan === "brak" ? "—" : utc === null ? tt("clock.ageUnknown") : wiek(teraz - utc),
        });

  return (
    <Tooltip content={tresc}>
      <span className="puls puls--mt5" data-ton={ton}>
        <span className="dot" />
        MT5
        {polaczony && <b className="num">{c.latencyMs} ms</b>}
      </span>
    </Tooltip>
  );
}
