import { tSilnik } from "@/i18n/silnik";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Candle, Drawing, DrawTool, PendingOrder, Position, Timeframe } from "@/types";
import { getSymbol } from "@/data/symbols";
import { TIMEFRAMES } from "@/engine/market";
import { useCandles, useSymbolInfo } from "./useCandles";
import { useTrades } from "./useTrades";
import { potentialAt } from "@/engine/bot";
import { useApp } from "@/store/AppStore";
import {
  buildOverlays,
  computeGeom,
  hitTest,
  hitTrade,
  parsePrice,
  readTheme,
  renderChart,
  renderChartSvg,
  MAX_BARS,
  MIN_BARS,
  type ChartGeom,
  type LineOwner,
  type OverlayLine,
  type Viewport,
} from "./chartRender";
import { Button, Icon } from "@/components/ui";
import { useT } from "@/i18n";
import { num } from "@/lib/format";
import "./chart.css";

const DRAW_COLORS = ["#6d7bff", "#26d9a3", "#ff5c7a", "#f5b74e", "#c88dff", "#56b8ff"];
const EXPORT_FORMATS = ["PNG", "JPG", "SVG"] as const;
type ExportFormat = (typeof EXPORT_FORMATS)[number];



/**
 * Czy działamy z serwera deweloperskiego Vite.
 *
 * Odczyt przez rzutowanie, a NIE `import.meta.env.DEV` wprost: `tsconfig`
 * tego projektu nie wciąga `vite/client`, więc bezpośrednie odwołanie wywala
 * `npx tsc --noEmit` (TS2339) i blokuje budowanie binarki. Rzutowanie przez
 * `unknown` daje ten sam efekt w kodzie i milczy w kontroli typów.
 */
const DEV = (import.meta as unknown as { env?: { DEV?: boolean } }).env?.DEV === true;

/**
 * Rozjazd między ostatnią świecą a kwotowaniem, powyżej którego mówimy o tym
 * użytkownikowi. To jest informacja o SPÓJNOŚCI DANYCH, a NIE o ich źródle —
 * pochodzenie świec bierzemy z pola `candleSource`, nie zgadujemy go z ceny.
 *
 * Poprzednia wersja wnioskowała „to nie są dane brokera" właśnie z rozjazdu
 * i kłamała w obie strony: w weekend prawdziwa piątkowa świeca odjeżdża od
 * kwotowania i zapalała ostrzeżenie mimo danych z MT5, a generator, który
 * przypadkiem trafił w cenę, nie zapalał go wcale.
 */
/**
 * Ile pustych świec zostawiamy po prawej stronie wykresu.
 * Świeca bieżąca nie może dotykać osi ceny — jej etykieta nachodziłaby na
 * podpisy cen, a ruch w górę i w dół nie miałby się gdzie odbyć.
 */
const RIGHT_PAD_BARS = 4;

const QUOTE_DRIFT = 0.002;

/** Skąd pochodzą świece podane w `candles`. */
export type CandleSource = "MT5" | "generator";

type LevelKind = "sl" | "tp" | "price";

/** Który poziom trzymamy w ręku. Jedna linia potrafi nieść wiele zleceń. */
interface LevelRef {
  /** stała tożsamość linii (z `buildOverlays`) */
  key: string;
  kind: LevelKind;
  owner: LineOwner;
  tickets: number[];
  /** ile z tych zleceń nie należy do bota — do ostrzeżenia przed zatwierdzeniem */
  foreign?: number;
}

/** Trwające ciągnięcie linii. */
interface LevelDrag extends LevelRef {
  /** poziom wyjściowy; `null` = poziom dopiero powstaje */
  from: number | null;
  price: number;
  /** kursor wyjechał poza wykres — upuszczenie skasuje poziom */
  remove: boolean;
  y0: number;
  moved: boolean;
}

/** Propozycja czekająca na potwierdzenie (tryb bez ONE CLICK). */
interface LevelAsk extends LevelRef {
  from: number | null;
  /** `null` = skasowanie poziomu */
  want: number | null;
  /** treść pola tekstowego — to ONA jest źródłem prawdy przy zatwierdzaniu */
  text: string;
}

/** Modyfikacja wysłana do brokera — czekamy, aż wróci przez WebSocket. */
interface LevelBusy extends LevelRef {
  want: number | null;
  since: number;
}

/** Wpis do cofnięcia (Ctrl+Z). */
interface UndoStep {
  opis: string;
  cofnij: () => void;
}

const sameKey = (a: { key: string } | null, b: { key: string } | null) =>
  a === b || (!!a && !!b && a.key === b.key);

export interface TradingChartProps {
  symbol: string;
  positions: Position[];
  pendings: PendingOrder[];
  livePrice: number;
  showPositions: boolean;
  showPotential: boolean;
  height?: number;
  compactToolbar?: boolean;
  /**
   * Skąd pochodzą świece. Domyślnie `generator`, bo dziś panel nie ma żadnego
   * innego źródła — backend świec nie wystawia. Kto poda `MT5`, bierze na
   * siebie, że to prawda: od tego zależy, czy użytkownik widzi ostrzeżenie
   * „dane poglądowe".
   */
  candleSource?: CandleSource;
  /**
   * Minimalny dystans SL/TP od ceny, W CENIE, dla TEGO instrumentu
   * (`stopsLevelPrice` z `GET /api/symbol`). Podany — jest jedynym źródłem
   * prawdy. Niepodany — panel nie wymyśla własnego ograniczenia i zostawia
   * sprawdzenie brokerowi.
   */
  stopsLevelPrice?: number;
  
  clockOffsetMs?: number;
}

export function TradingChart({
  symbol,
  positions,
  pendings,
  livePrice,
  showPositions,
  showPotential,
  height = 380,
  compactToolbar = false,
  // BEZ wartości domyślnych: `undefined` znaczy „nie narzucam, weź z feedu".
  // Domyślka `"generator"` zjadłaby prawdziwe źródło i ostrzeżenie świeciłoby
  // na okrągło mimo danych z MT5.
  candleSource,
  clockOffsetMs,
  stopsLevelPrice,
}: TradingChartProps) {
  const app = useApp();
  const t = useT();
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const askInputRef = useRef<HTMLInputElement>(null);
  const [tf, setTf] = useState<Timeframe>("5m");
  const [style, setStyle] = useState<"candle" | "line" | "area">("candle");
  const [showVolume, setShowVolume] = useState(true);
  const [tool, setTool] = useState<DrawTool>("cursor");
  const [color, setColor] = useState(DRAW_COLORS[0]);
  const [drawings, setDrawings] = useState<Drawing[]>([]);
  const [drawHidden, setDrawHidden] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [format, setFormat] = useState<ExportFormat>("PNG");
  const [hover, setHover] = useState<{ x: number; y: number; candle: Candle | null } | null>(null);
  /** momenty decyzji bota — wejścia i wyjścia naniesione na oś czasu */
  const [showTrades, setShowTrades] = useState(true);
  const [hotTrade, setHotTrade] = useState<number | null>(null);

  const vp = useRef<Viewport>({ bars: 140, offset: 0, price: null });
  const drag = useRef<{ x: number; y: number; offset: number } | null>(null);
  /** ciągnięcie OSI CENY — pionowe skalowanie */
  const axisDrag = useRef<{ y: number; mid: number; span: number } | null>(null);
  const drawing = useRef<Drawing | null>(null);
  const [, force] = useState(0);
  const redraw = useCallback(() => force((n) => n + 1), []);

  /* ---------------- edycja poziomów myszą ---------------- */
  const lvDrag = useRef<LevelDrag | null>(null);
  const [lvHot, setLvHot] = useState<LevelRef | null>(null);
  const [onX, setOnX] = useState(false);
  const [ask, setAsk] = useState<LevelAsk | null>(null);
  const [busy, setBusy] = useState<LevelBusy | null>(null);
  const undo = useRef<UndoStep[]>([]);
  /** ostatnia wyliczona geometria — do umieszczenia paska potwierdzenia */
  const geomRef = useRef<ChartGeom | null>(null);

  const meta = getSymbol(symbol);
  /* Świece z MT5, a gdy backend ich nie oddaje — z generatora panelu.
     `feed.source` mówi WPROST, co jest na ekranie; nie wnioskujemy tego z ceny. */
  const feed = useCandles(symbol, tf, livePrice);
  const candles = feed.candles;
  const info = useSymbolInfo(symbol);
  const trades = useTrades(symbol, app.snapshot.closed, showTrades);
  const tick = Math.pow(10, -meta.digits);
  /** minimalny zakres osi ceny — ratunek przed zerowym zakresem (jedna płaska świeca) */
  const minSpan = tick * 8;
  const fmtP = (v: number) => v.toFixed(meta.digits).replace(".", ",");
  const round = useCallback((v: number) => Number(v.toFixed(meta.digits)), [meta.digits]);

  
  const botSymbol = app.primary.symbol;
  const fromState = app.settings.sim_stops_level;
  const zBackendu = Number.isFinite(stopsLevelPrice) ? stopsLevelPrice : info.stopsLevelPrice;
  const zBrokera = Number.isFinite(zBackendu) && (zBackendu as number) >= 0;
  const zPresetu = !zBrokera && symbol === botSymbol && Number.isFinite(fromState) && fromState > 0;

  const stopsLevel = zBrokera ? (zBackendu as number) : zPresetu ? fromState : 0;

  
  const stopsSource: "broker" | "preset" | "brak" = zBrokera ? "broker" : zPresetu ? "preset" : "brak";
  const stopsKnown = stopsSource !== "brak";

  const quote = app.quotes[symbol];
  const bid = Number.isFinite(quote?.bid) && quote.bid > 0 ? quote.bid : livePrice;
  const askPx = Number.isFinite(quote?.ask) && quote.ask > 0 ? quote.ask : livePrice;

  const posOf = useCallback((ticket: number) => positions.find((p) => p.ticket === ticket) ?? null, [positions]);
  const pendOf = useCallback((ticket: number) => pendings.find((o) => o.ticket === ticket) ?? null, [pendings]);
  const orderOf = useCallback(
    (owner: LineOwner, ticket: number): Position | PendingOrder | null =>
      owner === "pos" ? posOf(ticket) : pendOf(ticket),
    [posOf, pendOf],
  );

  /** Świece nie z brokera — fakt o ŹRÓDLE, brany wprost, nie zgadywany z ceny.
      Props jest nadrzędny (tryb projektowy i testy), inaczej decyduje feed. */
  const zrodlo = candleSource ?? feed.source;
  const zGeneratora = zrodlo !== "MT5";
  /** korekta zegara: znaczniki z MT5 są w czasie serwera brokera */
  const zegar = clockOffsetMs ?? feed.clockOffsetMs;

  /** Rozjazd świec i kwotowania — osobny fakt o SPÓJNOŚCI, cichszy. */
  const drift = useMemo(() => {
    const last = candles[candles.length - 1]?.c;
    if (!Number.isFinite(last) || !(livePrice > 0)) return 0;
    return ((last as number) - livePrice) / livePrice;
  }, [candles, livePrice]);
  const driftWidoczny = Math.abs(drift) > QUOTE_DRIFT;

  /** Powód, dla którego broker odrzuciłby ten poziom — albo `null`, gdy jest wykonalny. */
  const checkLevel = useCallback(
    (kind: LevelKind, owner: LineOwner, o: Position | PendingOrder, price: number): string | null => {
      const pts = (v: number) => `${Math.round(Math.abs(v) / tick)} pkt`;
      /* Mówimy, KTO wymaga — brokera cytujemy, presetu nie podajemy za brokera. */
      const wymaga =
        stopsSource === "broker"
          ? t("chart.req.broker", { v: pts(stopsLevel) })
          : t("chart.req.preset", { v: pts(stopsLevel) });
      /* Dystansu pilnujemy WYŁĄCZNIE, gdy znamy prawdziwy `stops_level`.
         Nie znamy — nie zgadujemy; niech odmówi broker, bo on jeden wie. */
      const zaBlisko = (d: number) => stopsKnown && stopsLevel > 0 && Math.abs(d) < stopsLevel - 1e-12;

      if (owner === "pend") {
        const q = o as PendingOrder;
        const buy = q.kind.startsWith("BUY");
        if (kind === "price") {
          // limit czeka po LEPSZEJ stronie rynku, stop po gorszej
          const limit = q.kind.endsWith("LIMIT");
          const ref = buy ? askPx : bid;
          const ponizej = buy === limit;
          if (ponizej ? price >= ref : price <= ref)
            return t("chart.err.pendSide", {
              kind: q.kind.replace("_", " "),
              side: ponizej ? t("chart.below") : t("chart.above"),
              px: fmtP(ref),
            });
          if (zaBlisko(price - ref)) return t("chart.err.tooClose", { d: pts(price - ref), req: wymaga });
          return null;
        }
        // SL/TP zlecenia oczekującego broker mierzy od CENY AKTYWACJI, nie od rynku
        const ref = q.price;
        if (kind === "sl") {
          if (buy ? price >= ref : price <= ref)
            return t("chart.err.pendSl", {
              kind: q.kind.replace("_", " "),
              side: buy ? t("chart.below") : t("chart.above"),
              px: fmtP(ref),
            });
        } else if (buy ? price <= ref : price >= ref) {
          return t("chart.err.pendTp", {
            kind: q.kind.replace("_", " "),
            side: buy ? t("chart.above") : t("chart.below"),
            px: fmtP(ref),
          });
        }
        if (zaBlisko(price - ref)) return t("chart.err.tooCloseTrigger", { d: pts(price - ref), req: wymaga });
        return null;
      }

      const p = o as Position;
      const buy = p.direction === "BUY";
      // MT5: BUY zamyka się po bid, SELL po ask — walidacja musi używać tej samej ceny
      const ref = buy ? bid : askPx;
      if (kind === "sl") {
        if (buy ? price >= ref : price <= ref)
          return t("chart.err.posSl", {
            dir: p.direction,
            side: buy ? t("chart.below") : t("chart.above"),
            px: fmtP(ref),
          });
        if (zaBlisko(ref - price)) return t("chart.err.tooClose", { d: pts(ref - price), req: wymaga });
      } else {
        if (buy ? price <= ref : price >= ref)
          return t("chart.err.posTp", {
            dir: p.direction,
            side: buy ? t("chart.above") : t("chart.below"),
            px: fmtP(ref),
          });
        if (zaBlisko(price - ref)) return t("chart.err.tooClose", { d: pts(price - ref), req: wymaga });
      }
      return null;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [bid, askPx, stopsLevel, stopsKnown, stopsSource, tick, meta.digits],
  );

  /** Pierwsze zlecenie z linii — do walidacji i podglądu wyniku. */
  const headOf = useCallback(
    (r: LevelRef): Position | PendingOrder | null => {
      for (const t of r.tickets) {
        const o = orderOf(r.owner, t);
        if (o) return o;
      }
      return null;
    },
    [orderOf],
  );

  /** Wysyła modyfikację DO WSZYSTKICH zleceń tej linii i zapisuje krok cofnięcia. */
  const commit = useCallback(
    (r: LevelRef, want: number | null) => {
      const stan: { ticket: number; owner: LineOwner; sl: number | null; tp: number | null; price: number }[] = [];

      for (const t of r.tickets) {
        const o = orderOf(r.owner, t);
        if (!o) continue;
        if (r.owner === "pos") {
          const p = o as Position;
          stan.push({ ticket: t, owner: "pos", sl: p.sl, tp: p.tp, price: p.openPrice });
          app.modifyPos(t, r.kind === "sl" ? want : p.sl, r.kind === "tp" ? want : p.tp);
        } else {
          const q = o as PendingOrder;
          stan.push({ ticket: t, owner: "pend", sl: q.sl, tp: q.tp, price: q.price });
          app.modifyPending(
            t,
            r.kind === "price" ? (want ?? q.price) : q.price,
            r.kind === "sl" ? want : q.sl,
            r.kind === "tp" ? want : q.tp,
          );
        }
      }

      if (stan.length === 0) return;

      undo.current.push({
        opis: `${r.kind === "price" ? t("chart.priceCaps") : r.kind.toUpperCase()} ${
          stan.length > 1 ? t("chart.nOrders", { n: stan.length }) : `#${stan[0].ticket % 100000}`
        }`,
        cofnij: () => {
          for (const s of stan) {
            if (s.owner === "pos") app.modifyPos(s.ticket, s.sl, s.tp);
            else app.modifyPending(s.ticket, s.price, s.sl, s.tp);
          }
        },
      });
      if (undo.current.length > 20) undo.current.shift();

      setBusy({ ...r, want, since: Date.now() });
      setAsk(null);
    },
    [app, orderOf],
  );

  /** Wspólna ścieżka: przeciągnięcie, „×", uchwyt „+SL" i skasowanie trafiają tutaj. */
  const propose = useCallback(
    (r: LevelRef, want: number | null, from: number | null) => {
      const head = headOf(r);
      if (!head) return;

      if (want !== null) {
        const why = checkLevel(r.kind, r.owner, head, want);
        if (why) {
          app.toast("error", t("chart.rejected", { what: r.kind === "price" ? t("chart.price") : r.kind.toUpperCase() }), why);
          return;
        }
        if (from !== null && Math.abs(want - from) < tick / 2) return; // nic się nie zmieniło
      } else if (r.kind === "price") {
        return; // ceny aktywacji nie da się „skasować"
      }

      if (app.settings.one_click) commit(r, want);
      else setAsk({ ...r, want, from, text: want === null ? "" : want.toFixed(meta.digits) });
    },
    [app, checkLevel, commit, headOf, tick, meta.digits],
  );

  /* Stan „w toku" gasimy, gdy backend przyśle zlecenie z nowym poziomem —
     albo po 6 s, gdy broker odrzucił i nic się nie zmieniło. */
  useEffect(() => {
    if (!busy) return;
    const o = headOf(busy);
    const cur = !o ? undefined : busy.kind === "price" ? ("price" in o ? o.price : null) : busy.kind === "sl" ? o.sl : o.tp;
    const matches =
      cur === undefined
        ? true
        : busy.want === null
          ? cur === null
          : cur !== null && Math.abs(cur - busy.want) < tick / 2;
    if (matches) {
      setBusy(null);
      return;
    }
    const h = setTimeout(() => setBusy(null), Math.max(200, 6000 - (Date.now() - busy.since)));
    return () => clearTimeout(h);
  }, [busy, headOf, tick]);

  /** Zatwierdza to, co STOI W POLU — nie to, co zostało przeciągnięte. */
  const zatwierdz = useCallback(
    (a: LevelAsk) => {
      if (a.want === null) {
        commit(a, null);
        return;
      }
      const v = parsePrice(a.text);
      if (v === null) {
        app.toast("error", t("chart.badPrice"), t("chart.notANumber", { v: a.text }));
        return;
      }
      const head = headOf(a);
      const why = head ? checkLevel(a.kind, a.owner, head, round(v)) : t("chart.orderGone");
      if (why) {
        app.toast("error", t("chart.rejected", { what: a.kind === "price" ? t("chart.price") : a.kind.toUpperCase() }), why);
        return;
      }
      commit(a, round(v));
    },
    [app, checkLevel, commit, headOf, round],
  );

  /* Enter zatwierdza, Esc anuluje — także gdy skupienie jest poza polem.
     Faza CAPTURE, inaczej Esc najpierw zamknąłby pełny ekran. */
  useEffect(() => {
    if (!ask) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        zatwierdz(ask);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setAsk(null); // linia sama wraca na poziom z backendu — nic lokalnie nie zmienialiśmy
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [ask, zatwierdz]);

  /* Pole ma być gotowe do nadpisania od razu — bez celowania myszą. */
  const askOpen = ask ? `${ask.key}|${ask.want === null}` : "";
  useEffect(() => {
    if (!askOpen || askOpen.endsWith("|true")) return;
    const el = askInputRef.current;
    if (!el) return;
    el.focus();
    el.select();
  }, [askOpen]);

  /* Ctrl+Z cofa ostatnią zmianę poziomu. */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== "z") return;
      const cel = e.target as HTMLElement | null;
      if (cel && /^(INPUT|TEXTAREA)$/.test(cel.tagName)) return;
      const krok = undo.current.pop();
      if (!krok) return;
      e.preventDefault();
      krok.cofnij();
      app.toast("info", t("chart.undone"), krok.opis);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [app]);

  /** Nakładki „surowe" (ceny prosto z backendu) + geometria liczona WŁAŚNIE z nich.
      Gdyby geometria uwzględniała cenę z ciągnięcia, skala uciekałaby spod kursora. */
  const baseScene = useCallback(
    (w: number, h: number) => {
      const wrap = wrapRef.current!;
      const theme = readTheme(wrap);
      const base = showPositions
        ? buildOverlays({
            positions,
            pendings,
            symbol,
            theme,
            showPotential,
            digits: meta.digits,
            live: livePrice,
            moneyAt: potentialAt,
          })
        : [];
      const geom = computeGeom({
        candles,
        vp: vp.current,
        w,
        h,
        showVolume,
        levels: base.map((o) => o.price),
        live: livePrice,
        minSpan,
        rightPadBars: RIGHT_PAD_BARS,
      });
      return { theme, base, geom };
    },
    [candles, livePrice, meta.digits, minSpan, pendings, positions, showPositions, showPotential, showVolume, symbol],
  );

  /** Dokłada do linii stan interakcji: podświetlenie, ciągnięty poziom, „w toku". */
  const decorate = useCallback(
    (base: OverlayLine[]): OverlayLine[] => {
      const d = lvDrag.current;
      /* Zlecenia, których dotyczy linia pod kursorem — ich pozostałe linie
         podświetlamy słabiej. Przy kilku pozycjach naraz to jedyny sposób,
         żeby zobaczyć, CZYJ to stop. */
      const rodzina = new Set<number>();
      const trzymane = d ?? lvHot ?? ask;
      if (trzymane) for (const t of trzymane.tickets) rodzina.add(t);

      const out: OverlayLine[] = [];
      for (const o of base) {
        const kin = !!o.tickets?.some((t) => rodzina.has(t));

        if (d && o.key === d.key) {
          if (d.from !== null) out.push({ ...o, ghost: true, hot: false, kin: false, label: "", note: undefined });
          const head = headOf(d);
          const zle = d.remove || !head ? null : checkLevel(d.kind, d.owner, head, d.price);
          const money = head && !d.remove ? potentialAt(head, d.price) : null;
          const badge = d.remove
            ? t("chart.dropToClear")
            : zle
              ? zle
              : `${fmtP(d.price)} · ${money === null ? "—" : `${money >= 0 ? "+" : ""}${money.toFixed(2)} $`}`;
          out.push({
            ...o,
            price: d.price,
            hot: true,
            kin: false,
            bad: !!zle || d.remove,
            remove: d.remove,
            label: `${d.kind === "price" ? "CENA" : d.kind.toUpperCase()} ${fmtP(d.price)}${d.tickets.length > 1 ? ` ×${d.tickets.length}` : ""}`,
            note: undefined,
            badge,
            handles: undefined,
          });
          continue;
        }

        const pytany = !!ask && o.key === ask.key;
        out.push({
          ...o,
          hot: sameKey(lvHot, o) || pytany,
          kin: kin && !sameKey(lvHot, o) && !pytany,
          // czeka na potwierdzenie skasowania — linia na czerwono, żeby było widać czego dotyczy
          bad: pytany && ask!.want === null,
          badge: busy && o.key === busy.key ? "w toku…" : undefined,
        });

        if (pytany && ask!.want !== null) {
          out.push({
            ...o,
            key: `${o.key}#want`,
            price: ask!.want!,
            hot: true,
            drag: false,
            handles: undefined,
            dash: [3, 3],
            label: `${ask!.kind === "price" ? "CENA" : ask!.kind.toUpperCase()} → ${fmtP(ask!.want!)}`,
            note: undefined,
            badge: undefined,
          });
        }
      }
      return out;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [ask, busy, checkLevel, headOf, lvHot, meta.digits],
  );

  const buildScene = useCallback(
    (w: number, h: number) => {
      const { theme, base, geom } = baseScene(w, h);
      return { theme, geom, overlays: decorate(base) };
    },
    [baseScene, decorate],
  );

  /* ---------------- rysowanie ---------------- */
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;

    const rect = wrap.getBoundingClientRect();
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.max(200, rect.width);
    const h = Math.max(160, rect.height);

    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const { theme, overlays, geom } = buildScene(w, h);
    geomRef.current = geom;

    /* Podgląd geometrii dla testów klikanych. Wykres rysuje się na canvasie,
       więc bez tego jedyną drogą do „gdzie naprawdę leży ta linia" jest
       czytanie pikseli. Znika przy budowaniu wersji produkcyjnej. */
    if (DEV) {
      const bag = ((window as unknown as { __chart?: Record<string, unknown> }).__chart ??= {});
      bag[symbol] = { geom, overlays, vp: { ...vp.current }, stopsLevel, stopsKnown, stopsSource, source: zrodlo, clockOffsetMs: zegar, candles: candles.length, oldest: candles[0]?.t ?? 0 };
    }

    renderChart({
      ctx,
      candles,
      geom,
      theme,
      style,
      tf,
      digits: meta.digits,
      showVolume,
      overlays,
      drawings: drawHidden ? [] : drawings,
      cursor: hover ? { x: hover.x, y: hover.y } : null,
      livePrice,
      preview: drawing.current,
      clockOffsetMs: zegar,
      trades,
      hotTrade,
    });
  });

  /* Zmiana instrumentu albo interwału = inne dane, więc widok wraca na
     najnowszą świecę. Bez tego po przełączeniu z 1d na 1m zostawało
     `offset` z poprzedniej serii i wykres otwierał się gdzieś w przeszłości,
     na pustym miejscu. Ręczna skala ceny też nie ma prawa przechodzić między
     instrumentami — 4000 $ złota i 1,08 na EURUSD to inne światy. */
  useEffect(() => {
    vp.current = { bars: 140, offset: 0, price: null };
    redraw();
  }, [symbol, tf, redraw]);

  /* Doładowanie historii, gdy użytkownik dojedzie do lewej krawędzi danych.
     Próg w świecach, nie w pikselach: przy każdym zoomie znaczy to samo. */
  useEffect(() => {
    const g = geomRef.current;
    if (!g || feed.loading || feed.exhausted) return;
    if (g.viewStart <= Math.max(20, g.bars * 0.25)) feed.loadOlder();
  });

  /* ---------------- reakcja na zmianę rozmiaru ---------------- */
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const ro = new ResizeObserver(() => redraw());
    ro.observe(wrap);
    return () => ro.disconnect();
  }, [redraw]);

  /* Aktualne dane dla listenera `wheel`, który jest podpinany raz. */
  const live = useRef({ candles, showVolume, livePrice });
  live.current = { candles, showVolume, livePrice };

  /* ----------------------------------------------------------------
     ZOOM KÓŁKIEM — natywny listener z { passive: false }.
     React rejestruje `wheel` jako listener PASYWNY, więc wywołanie
     preventDefault() w propsie onWheel jest ignorowane i strona
     przewija się razem z zoomowaniem. Jedyne poprawne rozwiązanie to
     własny listener na elemencie.
     ---------------------------------------------------------------- */
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const rect = wrap.getBoundingClientRect();
      const { candles: cs, showVolume: sv, livePrice: lp } = live.current;
      const n = cs.length;
      const g = computeGeom({ candles: cs, vp: vp.current, w: rect.width, h: rect.height, showVolume: sv, live: lp, rightPadBars: RIGHT_PAD_BARS });
      const px = e.clientX - rect.left;
      const k = e.deltaY > 0 ? 1.16 : 1 / 1.16;

      /* Kółko nad OSIĄ CENY (albo z Shiftem) skaluje PIONOWO. Bez tego nie ma
         jak rozsunąć poziomów leżących na sobie, gdy świece obejmują kilkaset
         dolarów, a SL i TP jednej pozycji dzieli kilka. */
      if (e.shiftKey || px > g.padL + g.plotW) {
        const mid = vp.current.price?.mid ?? (g.minP + g.maxP) / 2;
        const span = (vp.current.price?.span ?? g.maxP - g.minP) * k;
        vp.current = { ...vp.current, price: { mid, span: Math.max(span, minSpan) } };
        redraw();
        return;
      }

      const cap = Math.max(MIN_BARS, Math.min(n || MIN_BARS, MAX_BARS));
      const bars = Math.min(Math.max(vp.current.bars * k, MIN_BARS), cap);

      /* Zoom POD KURSOREM: świeca, na którą patrzysz, zostaje w miejscu.
         Bez tego przy 2 świecach na ekran nie da się dojechać tam, gdzie chcesz. */
      const cx = Math.min(Math.max(px, g.padL), g.padL + g.plotW);
      const frac = (cx - g.padL) / g.plotW;
      const iCur = g.iOf(cx);
      const offset = Math.min(Math.max(n + RIGHT_PAD_BARS - (iCur + 0.5 + bars * (1 - frac)), 0), Math.max(0, n - 1));

      vp.current = { ...vp.current, bars, offset };
      redraw();
    };

    wrap.addEventListener("wheel", onWheel, { passive: false });
    return () => wrap.removeEventListener("wheel", onWheel);
  }, [redraw, minSpan]);

  /* Trafianie w linie mieszka w `chartRender` razem z rysowaniem — jedno
     źródło geometrii i jedyny sposób, żeby sprawdzić je bez przeglądarki. */
  const hitAt = useCallback(
    (px: number, py: number, geom: ChartGeom, base: OverlayLine[], hot: LevelRef | null) =>
      hitTest(px, py, geom, base, hot?.key ?? null),
    [],
  );

  const refOf = (o: OverlayLine, kind: LevelKind): LevelRef => ({
    key: o.key,
    kind,
    owner: o.owner ?? "pos",
    tickets: o.tickets ?? [],
    foreign: o.foreign,
  });

  const kindOf = (o: OverlayLine): LevelKind => (o.kind === "pending" ? "price" : (o.kind as LevelKind));

  /* ---------------- pan / rysowanie ---------------- */
  const dataPoint = useCallback(
    (clientX: number, clientY: number): [number, number] | null => {
      const wrap = wrapRef.current;
      if (!wrap) return null;
      const rect = wrap.getBoundingClientRect();
      const geom = computeGeom({
        candles,
        vp: vp.current,
        w: rect.width,
        h: rect.height,
        showVolume,
        live: livePrice,
        minSpan,
        rightPadBars: RIGHT_PAD_BARS,
      });
      const i = geom.iOf(clientX - rect.left);
      const t0 = candles[0]?.t ?? 0;
      const t1 = candles[1]?.t ?? t0 + 60000;
      return [t0 + i * (t1 - t0), geom.pOf(clientY - rect.top)];
    },
    [candles, livePrice, minSpan, showVolume],
  );

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
      const wrap = wrapRef.current;
      if (!wrap) return;
      const rect = wrap.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const py = e.clientY - rect.top;

      if (tool === "cursor") {
        const { base, geom } = baseScene(rect.width, rect.height);

        /* Oś ceny: ciągnięcie w pionie skaluje, dwuklik wraca do automatu. */
        if (px > geom.padL + geom.plotW) {
          if (e.detail >= 2) {
            vp.current = { ...vp.current, price: null };
            redraw();
            return;
          }
          axisDrag.current = { y: py, mid: (geom.minP + geom.maxP) / 2, span: geom.maxP - geom.minP };
          return;
        }

        /* Trafienie w linię ma PIERWSZEŃSTWO przed przeciąganiem tła —
           inaczej próba złapania SL przesuwałaby wykres w czasie. */
        const hit = hitAt(px, py, geom, base, lvHot);
        if (hit) {
          if (hit.t === "x") {
            propose(refOf(hit.o, kindOf(hit.o)), null, hit.o.price);
            return;
          }
          if (hit.t === "handle") {
            /* Poziomu, którego nie ma, nie da się przeciągnąć — więc go
               TWORZYMY: uchwyt zaczyna zwykłe ciągnięcie od bezpiecznego
               dystansu po właściwej stronie ceny. */
            const head = orderOf("pos", hit.tickets[0]) as Position | null;
            if (!head) return;
            const buy = head.direction === "BUY";
            const dist = Math.max(stopsLevel * 3, tick * 30);
            const start = hit.kind === "sl" ? (buy ? bid - dist : askPx + dist) : buy ? bid + dist : askPx - dist;
            lvDrag.current = {
              key: `${hit.o.key}+${hit.kind}`,
              kind: hit.kind,
              owner: "pos",
              tickets: hit.tickets,
              from: null,
              price: round(start),
              remove: false,
              y0: py,
              moved: false,
            };
            setAsk(null);
            redraw();
            return;
          }
          if (hit.o.drag === true) {
            setAsk(null);
            lvDrag.current = {
              ...refOf(hit.o, kindOf(hit.o)),
              from: hit.o.price,
              price: hit.o.price,
              remove: false,
              y0: py,
              moved: false,
            };
            redraw();
            return;
          }
        }
        drag.current = { x: e.clientX, y: e.clientY, offset: vp.current.offset };
        return;
      }

      if (tool === "eraser") {
        const geom = computeGeom({
          candles,
          vp: vp.current,
          w: rect.width,
          h: rect.height,
          showVolume,
          live: livePrice,
          minSpan,
          rightPadBars: RIGHT_PAD_BARS,
        });
        const t0 = candles[0]?.t ?? 0;
        const t1 = candles[1]?.t ?? t0 + 60000;
        const xOfTime = (t: number) => geom.xOf((t - t0) / Math.max(1, t1 - t0));
        setDrawings((ds) => ds.filter((d) => !d.pts.some((p) => Math.hypot(xOfTime(p[0]) - px, geom.yOf(p[1]) - py) < 14)));
        return;
      }

      const pt = dataPoint(e.clientX, e.clientY);
      if (!pt) return;
      drawing.current = { id: Date.now(), tool, color, width: 2, pts: [pt, pt] };
      redraw();
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      tool,
      color,
      dataPoint,
      baseScene,
      hitAt,
      propose,
      candles,
      livePrice,
      minSpan,
      showVolume,
      redraw,
      lvHot,
      orderOf,
      round,
      stopsLevel,
      tick,
      bid,
      askPx,
    ],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const wrap = wrapRef.current;
      if (!wrap) return;
      const rect = wrap.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;

      /* --- skalowanie osi ceny --- */
      const ad = axisDrag.current;
      if (ad) {
        const span = Math.max(minSpan, ad.span * Math.pow(2, (y - ad.y) / 150));
        vp.current = { ...vp.current, price: { mid: ad.mid, span } };
        redraw();
        return;
      }

      const d = lvDrag.current;
      if (d) {
        const { geom } = baseScene(rect.width, rect.height);
        const inside = x >= geom.padL && x <= geom.padL + geom.plotW && y >= geom.padT && y <= geom.padT + geom.priceH;
        // poziomu, którego jeszcze nie ma, nie ma czego kasować
        d.remove = !inside && d.from !== null;
        // przycięcie do WIDOCZNEGO zakresu ceny — poza nim poziom i tak nie ma sensu
        if (inside) d.price = round(Math.min(geom.maxP, Math.max(geom.minP, geom.pOf(y))));
        if (Math.abs(y - d.y0) > 2) d.moved = true;
        setHover({ x, y, candle: candles[Math.round(geom.iOf(x))] ?? null });
        redraw();
        return;
      }

      if (drag.current) {
        const g0 = computeGeom({
          candles,
          vp: vp.current,
          w: rect.width,
          h: rect.height,
          showVolume,
          live: livePrice,
          minSpan,
          rightPadBars: RIGHT_PAD_BARS,
        });
        const dxBars = (e.clientX - drag.current.x) / g0.barW;
        vp.current = {
          ...vp.current,
          offset: Math.min(Math.max(drag.current.offset + dxBars, 0), Math.max(0, candles.length - 1)),
        };
      } else if (drawing.current) {
        const pt = dataPoint(e.clientX, e.clientY);
        if (pt) {
          if (drawing.current.tool === "brush") drawing.current.pts.push(pt);
          else drawing.current.pts[1] = pt;
        }
      }

      // geometria PO ewentualnym przesunięciu widoku
      const { base, geom } = baseScene(rect.width, rect.height);
      const hit = tool === "cursor" && !drag.current ? hitAt(x, y, geom, base, lvHot) : null;
      const next = hit ? refOf(hit.o, hit.t === "handle" ? hit.kind : kindOf(hit.o)) : null;
      if (!sameKey(next, lvHot)) setLvHot(next);
      const wskaz = hit?.t === "x" || hit?.t === "handle";
      if (wskaz !== onX) setOnX(wskaz);

      /* Znacznik decyzji pod kursorem — tylko gdy nie celujemy w poziom,
         bo linie SL/TP są interaktywne i mają pierwszeństwo. */
      const th = !hit && trades.length ? hitTrade(x, y, geom, candles, trades) : null;
      const thId = th ? th.trade.ticket : null;
      if (thId !== hotTrade) setHotTrade(thId);

      setHover({ x, y, candle: candles[Math.round(geom.iOf(x))] ?? null });
      redraw();
    },
    [baseScene, candles, dataPoint, hitAt, hotTrade, livePrice, lvHot, minSpan, onX, redraw, round, showVolume, tool, trades],
  );

  const onPointerUp = useCallback(() => {
    drag.current = null;
    axisDrag.current = null;

    const d = lvDrag.current;
    if (d) {
      lvDrag.current = null;
      // nowy poziom potwierdzamy nawet bez ruchu — samo kliknięcie „+SL" ma coś zrobić
      if (d.moved || d.remove || d.from === null) propose(d, d.remove ? null : round(d.price), d.from);
      redraw();
      return;
    }

    if (drawing.current) {
      const dr = drawing.current;
      drawing.current = null;
      if (dr.tool === "hline" || dr.pts.length > 1) setDrawings((ds) => [...ds, dr]);
    }
    redraw();
  }, [propose, redraw, round]);

  const resetView = useCallback(() => {
    vp.current = { bars: 140, offset: 0, price: null };
    redraw();
  }, [redraw]);

  /** Skala ceny prosto na poziomy pozycji — jeden klik zamiast celowania kółkiem. */
  const fitLevels = useCallback(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const rect = wrap.getBoundingClientRect();
    const { base } = baseScene(rect.width, rect.height);
    const ceny = base.map((o) => o.price).filter((p) => Number.isFinite(p));
    if (livePrice > 0) ceny.push(livePrice);
    if (!ceny.length) return;
    const lo = Math.min(...ceny);
    const hi = Math.max(...ceny);
    vp.current = { ...vp.current, price: { mid: (lo + hi) / 2, span: Math.max(hi - lo, minSpan) * 1.5 } };
    redraw();
  }, [baseScene, livePrice, minSpan, redraw]);

  /* ---------------- eksport: PNG / JPG / SVG ---------------- */
  const download = (href: string, ext: string) => {
    const a = document.createElement("a");
    a.download = `${symbol}_${tf}_${new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-")}.${ext}`;
    a.href = href;
    a.click();
  };

  const exportChart = useCallback(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;

    if (format === "SVG") {
      const rect = wrap.getBoundingClientRect();
      const { theme, base, geom } = baseScene(rect.width, rect.height);
      const svg = renderChartSvg({
        candles,
        geom,
        theme,
        style,
        tf,
        digits: meta.digits,
        showVolume,
        overlays: base,
        drawings: drawHidden ? [] : drawings,
        livePrice,
        clockOffsetMs: zegar,
      });
      const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml;charset=utf-8" }));
      download(url, "svg");
      setTimeout(() => URL.revokeObjectURL(url), 2000);
      return;
    }

    if (format === "JPG") {
      // JPEG nie ma kanału alfa — podkładamy tło motywu, inaczej przezroczystość
      // zostałaby wypełniona czernią.
      const flat = document.createElement("canvas");
      flat.width = canvas.width;
      flat.height = canvas.height;
      const fctx = flat.getContext("2d");
      if (!fctx) return;
      fctx.fillStyle = readTheme(wrap).bg;
      fctx.fillRect(0, 0, flat.width, flat.height);
      fctx.drawImage(canvas, 0, 0);
      download(flat.toDataURL("image/jpeg", 0.94), "jpg");
      return;
    }

    download(canvas.toDataURL("image/png"), "png");
  }, [format, baseScene, candles, style, tf, meta.digits, showVolume, drawings, drawHidden, livePrice, symbol]);

  /* ---------------- pełny ekran ---------------- */
  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setFullscreen(false);
    window.addEventListener("keydown", onKey);
    // zablokuj przewijanie strony pod nakładką
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [fullscreen]);

  /* Legenda pokazuje świecę spod kursora, a gdy kursora nie ma — ostatnią.
     Pustka przy zjechaniu myszą byłaby migotaniem, a przy odczycie w pasku
     była wręcz przyczyną przeskakiwania przycisków. */
  const legenda = hover?.candle ?? candles[candles.length - 1] ?? null;
  const legendaChg = legenda ? ((legenda.c - legenda.o) / legenda.o) * 100 : 0;

  /* ---------------- pasek potwierdzenia (bez ONE CLICK) ---------------- */
  const askHead = ask ? headOf(ask) : null;
  const askY = ask && geomRef.current ? geomRef.current.yOf(ask.want ?? ask.from ?? livePrice) : 0;
  const askMoney = ask && askHead && ask.want !== null ? potentialAt(askHead, ask.want) : null;
  const skala = geomRef.current?.scale ?? "auto";

  const body = (
    <div className={`chart ${fullscreen ? "chart--fs" : ""}`}>
      {}
      <div className="chart__bar">
        <div className="chart__bar-scroll">
        {fullscreen && (
          <span className="chart__fstitle">
            <b>{symbol}</b>
            <span className="hint">{tSilnik(meta.name)}</span>
          </span>
        )}

        <div className="chart__tfs">
          {TIMEFRAMES.map((t) => (
            <button key={t} className="chart__tf" data-active={t === tf} onClick={() => setTf(t)}>
              {t}
            </button>
          ))}
        </div>

        <div className="divider--v" />

        <div className="chart__styles">
          {(
            [
              ["candle", "chart", t("chart.tool.candles")],
              ["line", "trend", "linia"],
              ["area", "activity", "obszar"],
            ] as const
          ).map(([s, ic, title]) => (
            <button key={s} className="chart__tf" data-active={style === s} onClick={() => setStyle(s)} title={title}>
              <Icon name={ic} size={13} />
            </button>
          ))}
        </div>

        {(!compactToolbar || fullscreen) && (
          <>
            <div className="divider--v" />
            <div className="chart__tools">
              {(
                [
                  ["cursor", "cursor", "kursor / przesuwanie · edycja SL i TP"],
                  ["brush", "pencil", t("chart.tool.brush")],
                  ["line", "line", "linia prosta"],
                  ["hline", "minus", "linia pozioma"],
                  ["rect", "grid", t("chart.tool.rect")],
                  ["eraser", "eraser", "gumka"],
                ] as const
              ).map(([t, ic, title]) => (
                <button key={t} className="chart__tf" data-active={tool === t} onClick={() => setTool(t)} title={title}>
                  <Icon name={ic} size={13} />
                </button>
              ))}
              <div className="chart__colors">
                {DRAW_COLORS.map((c) => (
                  <button
                    key={c}
                    className="chart__color"
                    data-active={color === c}
                    style={{ background: c }}
                    onClick={() => setColor(c)}
                    title={t("chart.draw.color")}
                  />
                ))}
              </div>
            </div>
          </>
        )}

        </div>

        <div className="chart__bar-fix">
        <Button
          variant="ghost"
          size="sm"
          icon="minus"
          active={skala !== "auto"}
          onClick={fitLevels}
          title={t("chart.fitAxis.title")}
        />

        {/* wolumen — ikona słupków, nie oka (oko = widoczność rysunków) */}
        <Button
          variant="ghost"
          size="sm"
          icon="activity"
          active={showTrades}
          onClick={() => setShowTrades((v) => !v)}
          title={showTrades ? t("chart.trades.hide", { n: trades.length }) : t("chart.trades.show", { n: trades.length })}
        />

        <Button
          variant="ghost"
          size="sm"
          icon="bars"
          active={showVolume}
          onClick={() => setShowVolume((v) => !v)}
          title={showVolume ? t("chart.volume.hide") : t("chart.volume.show")}
        />

        {drawings.length > 0 && (
          <>
            <Button
              variant="ghost"
              size="sm"
              icon={drawHidden ? "eye-off" : "eye"}
              active={!drawHidden}
              onClick={() => setDrawHidden((v) => !v)}
              title={drawHidden ? t("chart.draw.show", { n: drawings.length }) : t("chart.draw.hide", { n: drawings.length })}
            />
            <Button variant="ghost" size="sm" icon="trash" onClick={() => setDrawings([])} title={t("chart.draw.clear")} />
          </>
        )}

        <Button variant="ghost" size="sm" icon="refresh" onClick={resetView} title={t("chart.reset")} />

        <span className="chart__export">
          <Button variant="ghost" size="sm" icon="download" onClick={exportChart} title={`Pobierz wykres jako ${format}`} />
          <span className="chart__fmts">
            {EXPORT_FORMATS.map((f) => (
              <button key={f} className="chart__fmt" data-active={format === f} onClick={() => setFormat(f)} title={`Format ${f}`}>
                {f}
              </button>
            ))}
          </span>
        </span>

        <Button
          variant="ghost"
          size="sm"
          icon={fullscreen ? "collapse" : "expand"}
          onClick={() => setFullscreen((f) => !f)}
          title={fullscreen ? t("chart.fs.exit") : t("chart.fs.enter")}
        />
        </div>
      </div>

      <div
        ref={wrapRef}
        className="chart__canvas"
        style={{ height: fullscreen ? "calc(100vh - 62px)" : height }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={() => {
          setHover(null);
          setHotTrade(null);
          drag.current = null;
          axisDrag.current = null;
          if (!lvDrag.current) setLvHot(null);
        }}
        data-tool={tool}
        data-hot={lvDrag.current ? "level" : onX ? "x" : lvHot ? "level" : undefined}
      >
        <canvas ref={canvasRef} />
        <div className="chart__watermark">
          <span>{symbol}</span>
          <small>
            {tSilnik(meta.name)} · {tf}
          </small>
        </div>

        {/* Legenda OHLC. NIE w pasku narzędzi — pasek to układ przepływowy
            i pojawiający się w nim odczyt rozpychał go na drugi rząd, przez
            co przyciski eksportu i pełnego ekranu skakały przy każdym
            najechaniu na wykres. Tutaj jest nakładką: `position:absolute`,
            `pointer-events:none`, więc nie może przesunąć niczego.
            Wiersz jest ZAWSZE — bez kursora opisuje ostatnią świecę, jak
            u brokerów — dzięki czemu nie miga i nie zmienia wysokości. */}
        {legenda && (
          <div className="chart__legend num" aria-hidden>
            <span>
              O <b>{legenda.o.toFixed(meta.digits)}</b>
            </span>
            <span>
              H <b>{legenda.h.toFixed(meta.digits)}</b>
            </span>
            <span>
              L <b>{legenda.l.toFixed(meta.digits)}</b>
            </span>
            <span>
              C <b>{legenda.c.toFixed(meta.digits)}</b>
            </span>
            <span className={legendaChg >= 0 ? "up" : "down"}>
              {legendaChg >= 0 ? "+" : ""}
              {legendaChg.toFixed(2)}%
            </span>
          </div>
        )}

        <div className="chart__flags">
          {/* ŹRÓDŁO świec. Fakt, nie domysł — zapala się dokładnie wtedy, gdy
              świece nie przyszły z MT5, niezależnie od tego, jak blisko ceny
              trafił generator. */}
          {zGeneratora && (
            <span
              className="chart__flag chart__flag--warn"
              title={feed.error ? t("chart.synth.title", { e: tSilnik(feed.error) }) : t("chart.synth.titleNoErr")}
            >
              <Icon name="info" size={11} /> {t("chart.synth")}
            </span>
          )}

          {/* Powód wprost na ekranie, a nie tylko w dymku: „rynek zamknięty"
              i „most padł" to różne rzeczy i użytkownik ma je rozróżnić bez
              zaglądania do dziennika. */}
          {feed.error && (
            <span className="chart__flag chart__flag--warn" title={feed.error}>
              {tSilnik(feed.error).length > 72 ? `${tSilnik(feed.error).slice(0, 69)}…` : tSilnik(feed.error)}
            </span>
          )}

          {feed.marketOpen === false && !zGeneratora && (
            <span className="chart__flag" title={t("chart.closed.title")}>
              {t("chart.closed")}
            </span>
          )}

          {/* SPÓJNOŚĆ danych. Osobno i ciszej, bo to inna informacja: przy
              zamkniętym rynku prawdziwe świece też odjeżdżają od kwotowania. */}
          {driftWidoczny && !zGeneratora && (
            <span
              className="chart__flag"
              title={t("chart.drift.title")}
            >
              {t("chart.drift", { v: num(drift * 100, 2) })}
            </span>
          )}
          {skala !== "auto" && (
            <button
              className="chart__flag"
              onClick={() => {
                vp.current = { ...vp.current, price: null };
                redraw();
              }}
              title={t("chart.axis.back")}
            >
              {t("chart.axis.state", { v: skala === "fit" ? t("chart.axis.fit") : t("chart.axis.manual") })}
            </button>
          )}
        </div>

        {ask && askHead && (
          <div
            className="chart__ask"
            style={{ top: Math.max(6, Math.min((geomRef.current?.h ?? 300) - 46, askY + 10)) }}
            onPointerDown={(e) => e.stopPropagation()}
          >
            <span className="chart__ask-txt num">
              <b>{ask.kind === "price" ? "CENA" : ask.kind.toUpperCase()}</b>
              {ask.tickets.length > 1 && <span className="hint">×{ask.tickets.length}</span>}
              {!!ask.foreign && (
                <span className="chart__ask-obce" title={t("chart.foreign.title")}>
                  {ask.foreign === ask.tickets.length
                    ? t("chart.foreign")
                    : t("chart.foreignN", { n: ask.foreign })}
                </span>
              )}
              {ask.from !== null ? fmtP(ask.from) : <span className="hint">{t("chart.new")}</span>}
              <span className="hint">→</span>
            </span>

            {ask.want === null ? (
              <b className="down">{t("chart.remove")}</b>
            ) : (
              <input
                ref={askInputRef}
                className="chart__askin num"
                value={ask.text}
                inputMode="decimal"
                spellCheck={false}
                aria-label={t("chart.newLevel")}
                onChange={(e) => {
                  const text = e.target.value;
                  const v = parsePrice(text);
                  // linia na wykresie podąża za tym, co wpisujesz
                  setAsk((a) => (a ? { ...a, text, want: v === null ? a.want : round(v) } : a));
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    e.stopPropagation();
                    zatwierdz(ask);
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    e.stopPropagation();
                    setAsk(null);
                  } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
                    // strzałki przesuwają o tick (Shift = 10) — precyzja bez celowania myszą
                    e.preventDefault();
                    const v = parsePrice(ask.text);
                    if (v === null) return;
                    const n = round(v + tick * (e.shiftKey ? 10 : 1) * (e.key === "ArrowUp" ? 1 : -1));
                    setAsk((a) => (a ? { ...a, text: n.toFixed(meta.digits), want: n } : a));
                  }
                }}
              />
            )}

            {askMoney !== null && (
              <span className={`num ${askMoney >= 0 ? "up" : "down"}`}>
                {askMoney >= 0 ? "+" : ""}
                {askMoney.toFixed(2)} $
              </span>
            )}

            <Button size="sm" variant="primary" icon="check" onClick={() => zatwierdz(ask)}>
              {t("chart.confirm")}
            </Button>
            <Button size="sm" variant="ghost" icon="x" onClick={() => setAsk(null)}>
              {t("common.cancel")}
            </Button>
          </div>
        )}
      </div>
    </div>
  );

  /* Pełny ekran renderujemy PORTALEM do <body>. Bez tego nakładka zostaje
     uwięziona w kontekście układania karty (.card__body ma z-index:1), przez
     co późniejsze karty — pendingi, koszyki — rysują się NAD wykresem. */
  return fullscreen ? createPortal(body, document.body) : body;
}
