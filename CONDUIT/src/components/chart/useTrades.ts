import { useEffect, useMemo, useState } from "react";
import type { ClosedPosition } from "@/types";
import { backendBase } from "@/store/transport";
import type { TradeMark } from "./chartRender";



/** Surowy deal z `/api/deals`. */
interface Deal {
  position: number;
  time: number;
  type: string;
  entry: string;
  volume: number;
  price: number;
  net: number;
  magic: number;
  symbol: string;
  comment: string;
}

/** Magic bota. Wszystko inne to cudzy handel — w tym stary `bot.py` (202406). */
const NASZ_MAGIC = 770077;

/**
 * Powód zamknięcia odczytany z komentarza brokera.
 *
 * MT5 dokleja do dealu zamykającego `[sl 4052.00]` albo `[tp 4053.00]` i to
 * jest informacja OD BROKERA, nie nasza proza — dlatego wolno ją czytać.
 * Czego nie da się rozpoznać, zostaje `—`; NIE zgadujemy, bo powód wejścia
 * z dziennika to osobna sprawa (etap 2) i wymaga pól ze skończonej listy.
 */
function powodZKomentarza(c: string): string {
  const s = (c || "").toLowerCase();
  if (s.includes("[sl")) return "SL";
  if (s.includes("[tp")) return "TP";
  if (s.includes("so:")) return "STOP OUT";
  return "—";
}

/** Składa pary wejście/wyjście z dealów po numerze POZYCJI. */
function zDealow(deals: Deal[], symbol: string): TradeMark[] {
  const wg = new Map<number, Deal[]>();
  for (const d of deals) {
    if (d.type === "BALANCE" || d.symbol !== symbol || !d.position) continue;
    const l = wg.get(d.position);
    if (l) l.push(d);
    else wg.set(d.position, [d]);
  }

  const out: TradeMark[] = [];
  for (const [pos, l] of wg) {
    const we = l.filter((d) => d.entry === "IN").sort((a, b) => a.time - b.time);
    const wy = l.filter((d) => d.entry === "OUT").sort((a, b) => a.time - b.time);
    if (!we.length || !wy.length) continue; // pozycja wciąż otwarta albo dane niepełne

    const pierwsze = we[0];
    const ostatnie = wy[wy.length - 1];
    // wolumen i wynik sumujemy po WSZYSTKICH nogach — częściowe zamknięcia
    // to jedna pozycja, nie kilka
    const vol = we.reduce((a, d) => a + d.volume, 0);
    const net = l.reduce((a, d) => a + d.net, 0);

    out.push({
      ticket: pos,
      // deal WEJŚCIOWY typu BUY oznacza pozycję długą
      dir: pierwsze.type === "BUY" ? "BUY" : "SELL",
      volume: vol,
      openTime: pierwsze.time,
      openPrice: pierwsze.price,
      closeTime: ostatnie.time,
      closePrice: ostatnie.price,
      net,
      reason: powodZKomentarza(ostatnie.comment),
      foreign: pierwsze.magic !== NASZ_MAGIC,
    });
  }
  return out;
}

/** Zamknięte pozycje z bieżącej sesji → znaczniki. */
function zMigawki(closed: ClosedPosition[], symbol: string): TradeMark[] {
  return closed
    .filter((p) => p.symbol === symbol)
    .map((p) => ({
      ticket: p.ticket,
      dir: p.direction,
      volume: p.volume,
      openTime: p.openTime,
      openPrice: p.openPrice,
      closeTime: p.closeTime,
      closePrice: p.closePrice,
      net: p.profit + p.swap + p.commission,
      reason: p.reason,
      foreign: p.source !== undefined && p.source !== "BOT",
    }));
}

/**
 * Znaczniki wejść i wyjść dla instrumentu.
 *
 * Migawka bieżącej sesji ma PIERWSZEŃSTWO nad historią z dealów — niesie
 * prawdziwy `reason` z silnika (`TRAIL`, `HARVEST`, `OAE`…), a nie tylko
 * `SL`/`TP` odczytane z komentarza brokera.
 */
export function useTrades(symbol: string, closed: ClosedPosition[], enabled: boolean): TradeMark[] {
  const [historia, setHistoria] = useState<TradeMark[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let zyje = true;
    (async () => {
      try {
        /* Bierzemy jedną dużą paczkę od najnowszych. `/api/deals` stronicuje,
           ale do obejrzenia doby i tak potrzeba ostatnich kilkuset. */
        const r = await fetch(`${backendBase()}/api/deals?limit=2000`, { headers: { accept: "application/json" } });
        if (!(r.headers.get("content-type") ?? "").includes("application/json") || !r.ok) return;
        const tekst = await r.text();
        if (!tekst.trim()) return;
        const j = JSON.parse(tekst) as { deals?: Deal[] };
        if (!zyje || !Array.isArray(j.deals)) return;
        setHistoria(zDealow(j.deals, symbol));
      } catch {
        /* brak historii nie jest awarią wykresu */
      }
    })();
    return () => {
      zyje = false;
    };
  }, [symbol, enabled]);

  return useMemo(() => {
    if (!enabled) return [];
    const zSesji = zMigawki(closed, symbol);
    const znane = new Set(zSesji.map((t) => t.ticket));
    return [...zSesji, ...historia.filter((t) => !znane.has(t.ticket))].sort((a, b) => a.openTime - b.openTime);
  }, [closed, historia, symbol, enabled]);
}
