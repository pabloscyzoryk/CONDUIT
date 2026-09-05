import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Candle, Timeframe } from "@/types";
import { getCandles } from "@/engine/market";
import { backendBase } from "@/store/transport";
import { t } from "@/i18n";

/* ============================================================
   ŹRÓDŁO ŚWIEC

   Świece bierzemy z `GET /api/candles` (prawdziwe dane z MT5), a gdy
   backendu nie ma albo ich nie oddaje — z generatora panelu. Różnica
   NIE jest kosmetyczna i dlatego wychodzi na zewnątrz jako `source`:
   generator mylił się co do ceny złota o 71,8 $, przez co oś ceny
   obejmowała 745,7 $ i poziomów SL/TP nie dawało się rozdzielić myszą.

   Ten plik jest jedynym miejscem, które wie, skąd biorą się świece.
   ============================================================ */

/** Skąd pochodzą świece podane dalej do wykresu. */
export type CandleSource = "MT5" | "generator" | "brak";

export interface CandleFeed {
  candles: Candle[];
  source: CandleSource;
  /** różnica zegara świec względem przeglądarki — patrz `clockOffsetMs` wykresu */
  clockOffsetMs: number;
  /** czy sesja jest otwarta (z backendu); `null` = nie wiemy */
  marketOpen: boolean | null;
  /** czy ostatnia świeca jest domknięta */
  complete: boolean;
  /** powód, dla którego nie ma danych z MT5 — do pokazania wprost */
  error: string | null;
  /** doładuj starszą historię (przewijanie w lewo); nic nie robi, gdy nie ma czego */
  loadOlder: () => void;
  /** trwa doładowywanie — do pokazania „wczytuję…" */
  loading: boolean;
  /** backend oddał już całą historię, jaką ma */
  exhausted: boolean;
}

/** Ile świec ciągniemy na raz. 500 wystarcza na pełne przewinięcie w tył. */
const COUNT = 500;
/** Co ile odpytujemy backend o świeże świece. */
const POLL_MS = 3000;

/** Broker wall-clock timestamps are rendered with UTC getters, independently of browser DST. */
const brokerClockOffset = () => 0; // Labels use UTC getters: no browser-zone or DST correction.

interface Odpowiedz {
  source?: string;
  candles?: Candle[];
  marketOpen?: boolean;
  complete?: boolean;
  serverOffsetMs?: number;
}

/**
 * Pobiera świece dla instrumentu i interwału.
 *
 * Nie rzuca i nie zostawia pustego wykresu: gdy backend nie odpowiada albo
 * odpowiada czymś innym niż danymi z MT5, wraca generator — ale `source`
 * mówi o tym wprost, żeby wykres mógł zapalić ostrzeżenie.
 */
/** Stan samego POBIERANIA — bez rzeczy, które hook dokłada od siebie. */
interface StanPobrania {
  candles: Candle[] | null;
  source: CandleSource;
  clockOffsetMs: number;
  marketOpen: boolean | null;
  complete: boolean;
  error: string | null;
}

export function useCandles(symbol: string, tf: Timeframe, livePrice: number): CandleFeed {
  const [feed, setFeed] = useState<StanPobrania>({
    candles: null,
    source: "generator",
    clockOffsetMs: 0,
    marketOpen: null,
    complete: true,
    error: null,
  });

  /* Instrument i interwał trzymamy w ref, żeby odpowiedź, która przyszła po
     przełączeniu zakładki, nie podmieniła świec pod nowym wykresem. */
  const chce = useRef({ symbol, tf });
  chce.current = { symbol, tf };

  /* Doładowana historia — ROZDZIELONA od świeżego okna.
     Odświeżanie co 3 s pobiera najnowsze 500 świec; gdyby historia siedziała
     w tym samym stanie, każde odświeżenie kasowałoby to, co użytkownik
     doładował przewijaniem. Dlatego starsze świece żyją osobno i doklejają
     się z przodu przy każdym renderze. */
  const [starsze, setStarsze] = useState<Candle[]>([]);
  const [loading, setLoading] = useState(false);
  const [exhausted, setExhausted] = useState(false);
  const wTrakcie = useRef(false);

  // zmiana instrumentu albo interwału unieważnia całą doładowaną historię
  useEffect(() => {
    setStarsze([]);
    setExhausted(false);
    wTrakcie.current = false;
  }, [symbol, tf]);

  useEffect(() => {
    let zyje = true;
    let timer: number | undefined;

    const pobierz = async () => {
      const adres = `${backendBase()}/api/candles?symbol=${encodeURIComponent(symbol)}&tf=${tf}&count=${COUNT}`;
      /** czy endpoint w ogóle istnieje w tej binarce */
      let jest = true;
      try {
        const r = await fetch(adres, { headers: { accept: "application/json" } });

        /* Trasa SPA oddaje `index.html` z kodem 200, gdy endpointu nie ma
           w binarce. Sam kod odpowiedzi więc NIE WYSTARCZY — sprawdzamy typ
           treści, inaczej `JSON.parse` wywala się na „<!doctype html>". */
        const typ = r.headers.get("content-type") ?? "";
        if (!typ.includes("application/json")) {
          jest = false;
          throw new Error(t("chart.err.noEndpoint"));
        }

        
        const tekst = await r.text();
        let j: (Odpowiedz & { error?: string }) | null = null;
        if (tekst.trim()) {
          try {
            j = JSON.parse(tekst) as Odpowiedz & { error?: string };
          } catch {
            j = null;
          }
        }

        if (!r.ok) {
          throw new Error(
            j?.error ||
              (r.status === 503
                ? t("chart.err.noBridge")
                : t("chart.err.status", { v: r.status })),
          );
        }
        if (!j || j.source !== "MT5" || !Array.isArray(j.candles) || j.candles.length === 0) {
          throw new Error(j?.error || t("chart.err.noCandles"));
        }

        if (!zyje || chce.current.symbol !== symbol || chce.current.tf !== tf) return;
        setFeed({
          candles: j.candles,
          source: "MT5",
          clockOffsetMs: brokerClockOffset(),
          marketOpen: j.marketOpen ?? null,
          complete: j.complete ?? true,
          error: null,
        });
      } catch (e) {
        if (!zyje || chce.current.symbol !== symbol || chce.current.tf !== tf) return;
        /* Rozróżnienie, na którym zależy użytkownikowi:
           - NIE MA endpointu (stara binarka, tryb projektowy) → generator, bo
             lepszy pogląd niż puste pole, ale z ostrzeżeniem;
           - endpoint JEST i odmawia (503, most padł, rynek zamknięty) →
             ŻADNEJ atrapy. Podstawianie zmyślonych świec pod komunikat
             o awarii to najgorsze, co można zrobić w terminalu handlowym. */
        setFeed((s) => ({
          ...s,
          candles: null,
          source: jest ? "brak" : "generator",
          clockOffsetMs: 0,
          complete: true,
          error: e instanceof Error ? e.message : String(e),
        }));
      }
    };

    void pobierz();
    timer = window.setInterval(pobierz, POLL_MS);
    return () => {
      zyje = false;
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [symbol, tf]);

  /* Generator tylko wtedy, gdy backend świec NIE MA. Gdy jest i odmawia,
     zostawiamy pusto — wykres pokaże powód, a nie wymyślony kształt. */
  const zMt5 = feed.candles !== null;
  const swieze = zMt5 ? feed.candles! : feed.source === "generator" ? getCandles(symbol, tf, 900) : [];

  /* Historia doklejana z przodu. Granica `to` u MOSTU jest WYŁĄCZNA i
     zmierzyli zero nakładek, ale i tak filtrujemy po znaczniku: jedna
     zdublowana świeca psuje oś czasu na całej szerokości, a filtr kosztuje
     tyle co nic. */
  const candles = useMemo(() => {
    if (!starsze.length || !swieze.length) return swieze;
    const prog = swieze[0].t;
    const ogon = starsze.filter((k) => k.t < prog);
    return ogon.length ? [...ogon, ...swieze] : swieze;
    // `swieze` to nowa tablica przy każdym odświeżeniu — zależność po długości
    // i pierwszym/ostatnim znaczniku wystarczy i nie przelicza się bez potrzeby
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [starsze, swieze.length, swieze[0]?.t, swieze[swieze.length - 1]?.t]);

  /**
   * Doładowanie starszej historii — wołane, gdy użytkownik dojedzie
   * przewijaniem do lewej krawędzi danych.
   *
   * `count: 0` i HTTP 200 znaczy „to już wszystko" (tak to oddaje MOST), więc
   * zapamiętujemy koniec i przestajemy pytać. Bez tego przewijanie w lewo
   * generowałoby zapytanie co klatkę.
   */
  const loadOlder = useCallback(() => {
    if (wTrakcie.current || exhausted || feed.source !== "MT5" || !candles.length) return;
    wTrakcie.current = true;
    setLoading(true);
    const doTego = candles[0].t;
    const mojSymbol = symbol;
    const mojTf = tf;
    (async () => {
      try {
        const r = await fetch(
          `${backendBase()}/api/candles?symbol=${encodeURIComponent(mojSymbol)}&tf=${mojTf}&count=${COUNT}&to=${doTego}`,
          { headers: { accept: "application/json" } },
        );
        if (!(r.headers.get("content-type") ?? "").includes("application/json")) return;
        const tekst = await r.text();
        if (!tekst.trim()) return;
        const j = JSON.parse(tekst) as Odpowiedz;
        // odpowiedź na poprzedni instrument nie ma prawa dokleić się do nowego
        if (chce.current.symbol !== mojSymbol || chce.current.tf !== mojTf) return;
        const nowe = Array.isArray(j.candles) ? j.candles.filter((k) => k.t < doTego) : [];
        if (!nowe.length) {
          setExhausted(true);
          return;
        }
        setStarsze((s) => {
          const znane = new Set(s.map((k) => k.t));
          const dodaj = nowe.filter((k) => !znane.has(k.t));
          return dodaj.length ? [...dodaj, ...s].sort((a, b) => a.t - b.t) : s;
        });
      } catch {
        /* cisza — brak historii nie jest awarią wykresu */
      } finally {
        wTrakcie.current = false;
        setLoading(false);
      }
    })();
  }, [candles, exhausted, feed.source, symbol, tf]);

  /* Świeca bieżąca dolepiana z kwotowania. Backend buforuje 800 ms, więc bez
     tego ostatnia świeca stałaby w miejscu między odpytaniami. Kopiujemy
     tablicę — mutowanie tej z odpowiedzi mieszałoby dane między klatkami. */
  const gotowe =
    zMt5 && !feed.complete && livePrice > 0 && candles.length > 0
      ? (() => {
          const out = candles.slice();
          const k = { ...out[out.length - 1] };
          k.c = livePrice;
          if (livePrice > k.h) k.h = livePrice;
          if (livePrice < k.l) k.l = livePrice;
          out[out.length - 1] = k;
          return out;
        })()
      : candles;

  return {
    candles: gotowe,
    source: feed.source,
    clockOffsetMs: feed.clockOffsetMs,
    marketOpen: feed.marketOpen,
    complete: feed.complete,
    error: feed.error,
    loadOlder,
    loading,
    exhausted,
  };
}



export interface SymbolInfo {
  stopsLevelPrice?: number;
  digits?: number;
  point?: number;
}

export function useSymbolInfo(symbol: string): SymbolInfo {
  const [info, setInfo] = useState<SymbolInfo>({});

  useEffect(() => {
    let zyje = true;
    setInfo({});
    (async () => {
      try {
        const r = await fetch(`${backendBase()}/api/symbol?symbol=${encodeURIComponent(symbol)}`, {
          headers: { accept: "application/json" },
        });
        if (!(r.headers.get("content-type") ?? "").includes("application/json") || !r.ok) return;
        const j = (await r.json()) as SymbolInfo;
        // brak wartości zostawiamy jako brak — panel ma wtedy NIE wymyślać limitu
        if (zyje && Number.isFinite(j.stopsLevelPrice)) setInfo(j);
      } catch {
        /* brak danych = brak własnego ograniczenia; decyduje broker */
      }
    })();
    return () => {
      zyje = false;
    };
  }, [symbol]);

  return info;
}
