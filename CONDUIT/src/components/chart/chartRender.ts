import type { Candle, Drawing, PendingOrder, Position, Timeframe } from "@/types";
import { t } from "@/i18n";

/* ============================================================
   RENDERER WYKRESU (Canvas 2D)
   Rysuje swiece / linie / obszar, siatke, osie, wolumen,
   nakladki pozycji (wejscie / SL / TP), pendingi, rysunki
   uzytkownika oraz krzyz celowniczy.
   ============================================================ */

export interface ChartTheme {
  bg: string;
  grid: string;
  gridStrong: string;
  text: string;
  textDim: string;
  axis: string;
  up: string;
  down: string;
  upFill: string;
  downFill: string;
  line: string;
  areaTop: string;
  areaBottom: string;
  crosshair: string;
  surface: string;
  border: string;
  long: string;
  short: string;
  accent: string;
  accentFg: string;
  warn: string;
}

export interface Viewport {
  /** ile swiec widocznych — liczba UŁAMKOWA, inaczej nie da sie zejsc ponizej 1 swiecy na krok */
  bars: number;
  /** przesuniecie od konca serii (0 = przyklejone do prawej) */
  offset: number;
  /**
   * Reczne okno ceny. `null` = automatyczne (z widocznych swiec / poziomow).
   * Osobna os pionowa jest tu obowiazkowa, a nie ozdobna: zakres swiec bywa
   * kilkaset razy szerszy niz odleglosc miedzy SL a TP jednej pozycji, a wtedy
   * obie linie lezą na sobie i nie da sie ich rozroznic mysza.
   */
  price?: { mid: number; span: number } | null;
}

export interface ChartGeom {
  padL: number;
  padR: number;
  padT: number;
  padB: number;
  plotW: number;
  plotH: number;
  w: number;
  h: number;
  barW: number;
  /** pierwszy indeks DO ITERACJI (calkowity, z zapasem 1 swiecy) */
  first: number;
  /** koniec iteracji, wylacznie */
  last: number;
  /** lewa krawedz widoku w ulamkowej przestrzeni indeksow */
  viewStart: number;
  /** prawa krawedz widoku w ulamkowej przestrzeni indeksow */
  viewEnd: number;
  /** ile swiec miesci sie na calej szerokosci (ulamkowo) */
  bars: number;
  minP: number;
  maxP: number;
  /** wysokosc panelu ceny (bez wolumenu) */
  priceH: number;
  xOf: (i: number) => number;
  yOf: (p: number) => number;
  iOf: (x: number) => number;
  pOf: (y: number) => number;
  volH: number;
  /** ile dolarow przypada na jeden piksel pionu — miara precyzji ciagniecia */
  perPx: number;
  /** skad wziela sie skala ceny */
  scale: "auto" | "fit" | "manual";
}

const AXIS_W = 66;
const TIME_H = 24;
const PAD_T = 12;


export const MIN_BARS = 1.5;
/** Gorna granica zoomu (ile swiec naraz). */
export const MAX_BARS = 1200;
/** Promien trafienia w linie poziomu (px w pionie). */
export const LEVEL_HIT_PX = 6;
/** Bok kwadracika „x" kasujacego poziom. */
export const LEVEL_X_SIZE = 14;

const MAX_BODY_PX = 90;

/** Wysokosc etykiety poziomu i minimalny odstep miedzy etykietami. */
export const LABEL_H = 16;
const LABEL_GAP = 2;
/**
 * Szerokosc znaku w uzywanym kroju. Font jest MONOSPACE, wiec jedna stala
 * daje wynik co do piksela — i pozwala policzyc uklad etykiet BEZ kontekstu
 * canvasa, czyli tak samo przy rysowaniu i przy trafianiu mysza.
 */
const CHAR_W = 6.02;

export interface LabelBox {
  key: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** srodek linii, do ktorej etykieta nalezy (moze byc inny niz srodek etykiety) */
  lineY: number;
  close: { x: number; y: number; w: number; h: number } | null;
}

const labelText = (o: OverlayLine) => (o.note ? `${o.label}  ${o.note}` : o.label);
const labelWidth = (o: OverlayLine) =>
  labelText(o).length * CHAR_W + 12 + (o.hot && o.drag ? LEVEL_X_SIZE + 4 : 0);

/**
 * Uklad etykiet poziomow — przy PRAWEJ krawedzi, jak u brokerow, i rozsuniete
 * tak, zeby na siebie nie nachodzily.
 *
 * Wczesniej kazda etykieta siedziala na sztywno przy lewej krawedzi na
 * wysokosci swojej linii. Przy osmiu poziomach (a siatka limitow ma ich
 * kilkanascie) zlewaly sie w jedna nieczytelna kolumne na srodku wykresu —
 * dokladnie to zglosil uzytkownik.
 *
 * Funkcja jest CZYSTA i wspolna dla rysowania i trafien: gdyby „x" liczyl
 * sobie pozycje osobno, przycisk rysowalby sie gdzie indziej, niz da sie go
 * kliknac.
 */
export interface LabelLayout {
  boxes: Map<string, LabelBox>;
  /** ile poziomow nie zmiescilo sie z etykieta */
  hidden: number;
}

/**
 * Uklad etykiet z informacja, ile ich UKRYTO.
 *
 * Rozsuwanie ma granice: na koncie potrafi stac 21 zlecen oczekujacych naraz,
 * a wtedy stos etykiet i tak dojedzie do krawedzi i zacznie sie sklejac.
 * Powyzej pojemnosci pokazujemy tylko te, ktore naprawde niosa informacje
 * (podswietlona, jej rodzenstwo, ciagnieta), a reszte zbieramy w jeden
 * licznik „+N poziomow". Linie zostaja narysowane wszystkie — znika sam
 * podpis, a najechanie myszą przywraca go natychmiast.
 */
export function layoutLabelsFull(g: ChartGeom, overlays: OverlayLine[]): LabelLayout {
  const wszystkie = overlays
    .filter((o) => !o.ghost && o.label)
    .map((o) => ({ o, lineY: g.yOf(o.price) }))
    .filter((x) => x.lineY >= g.padT - LABEL_H && x.lineY <= g.padT + g.priceH + LABEL_H)
    .sort((a, b) => a.lineY - b.lineY);

  /* Ile etykiet w ogole zmiesci sie jedna pod druga. */
  const pojemnosc = Math.max(1, Math.floor(g.priceH / (LABEL_H + LABEL_GAP)));

  let widoczne = wszystkie;
  let hidden = 0;
  if (wszystkie.length > pojemnosc) {
    const wazne = wszystkie.filter((x) => x.o.hot || x.o.kin || x.o.badge);
    const reszta = wszystkie.filter((x) => !(x.o.hot || x.o.kin || x.o.badge));
    /* Z reszty zostawiamy co n-ta, rownomiernie po CENIE — dzieki temu
       podpisy nie znikaja z jednego konca wykresu, tylko sie przerzedzaja. */
    const miejsca = Math.max(0, pojemnosc - wazne.length);
    const krok = miejsca > 0 ? reszta.length / miejsca : Infinity;
    const wybrane = new Set<number>();
    for (let i = 0; i < miejsca; i++) wybrane.add(Math.floor(i * krok));
    const przerzedzona = reszta.filter((_, i) => wybrane.has(i));
    hidden = reszta.length - przerzedzona.length;
    widoczne = [...wazne, ...przerzedzona].sort((a, b) => a.lineY - b.lineY);
  }

  const gora = g.padT + LABEL_H / 2;
  const dol = g.padT + g.priceH - LABEL_H / 2;

  // 1. w dol: zadna etykieta nie moze zachodzic na poprzednia
  let y = -Infinity;
  const kolejno = widoczne.map((x) => {
    y = Math.max(x.lineY, y + LABEL_H + LABEL_GAP, gora);
    return { ...x, y };
  });

  // 2. gdy stos wyszedl poza dol, dosuwamy calosc w gore (zachowujac odstepy)
  const nadmiar = kolejno.length ? kolejno[kolejno.length - 1].y - dol : 0;
  if (nadmiar > 0) {
    let limit = Infinity;
    for (let i = kolejno.length - 1; i >= 0; i--) {
      kolejno[i].y = Math.min(Math.max(kolejno[i].y - nadmiar, gora), limit);
      limit = kolejno[i].y - LABEL_H - LABEL_GAP;
    }
  }

  const boxes = new Map<string, LabelBox>();
  for (const { o, lineY, y: cy } of kolejno) {
    const w = labelWidth(o);
    const x = g.padL + g.plotW - 6 - w;
    boxes.set(o.key, {
      key: o.key,
      x,
      y: cy - LABEL_H / 2,
      w,
      h: LABEL_H,
      lineY,
      close:
        o.hot && o.drag
          ? { x: x + w - LEVEL_X_SIZE - 4, y: cy - LEVEL_X_SIZE / 2, w: LEVEL_X_SIZE, h: LEVEL_X_SIZE }
          : null,
    });
  }
  return { boxes, hidden };
}

/** Zgodne wstecz: sam uklad, bez licznika ukrytych. */
export function layoutLabels(g: ChartGeom, overlays: OverlayLine[]): Map<string, LabelBox> {
  return layoutLabelsFull(g, overlays).boxes;
}

/** Szerokosc uchwytu „+SL" / „+TP" przy linii wejscia. */
export const HANDLE_W = 34;
export const HANDLE_H = 15;

/**
 * Prostokaty uchwytow tworzacych brakujacy poziom.
 * Jedno zrodlo prawdy dla rysowania i trafien — inaczej przycisk rysuje sie
 * gdzie indziej, niz da sie go kliknac.
 */
export function levelHandleRects(
  g: ChartGeom,
  o: OverlayLine,
  boxes?: Map<string, LabelBox>,
): { kind: "sl" | "tp"; tickets: number[]; x: number; y: number; w: number; h: number }[] {
  if (!o.handles?.length) return [];
  const box = boxes?.get(o.key);
  const y = (box ? box.y + box.h / 2 : g.yOf(o.price)) - HANDLE_H / 2;
  // startujemy na lewo od etykiety, zeby uchwyty jej nie przykrywaly
  let x = (box ? box.x : g.padL + g.plotW - 5) - HANDLE_W - 6;
  return o.handles.map((h) => {
    const r = { ...h, x, y, w: HANDLE_W, h: HANDLE_H };
    x -= HANDLE_W + 4;
    return r;
  });
}

/** Co znalazl kursor na wykresie. */
export type ChartHit =
  | { t: "level"; o: OverlayLine }
  | { t: "x"; o: OverlayLine }
  | { t: "handle"; o: OverlayLine; kind: "sl" | "tp"; tickets: number[] };

/**
 * Co jest pod kursorem. Funkcja CZYSTA i wyeksportowana celowo: rysowanie
 * i trafianie musza korzystac z tej samej geometrii, a bez wyciagniecia jej
 * z komponentu nie da sie tego sprawdzic inaczej niz mysza w przegladarce.
 *
 * @param hotKey klucz linii aktualnie podswietlonej — uchwyty „+SL/+TP" lapia
 *               tylko wtedy, gdy sa widoczne, czyli na podswietlonej linii
 */
export function hitTest(
  px: number,
  py: number,
  g: ChartGeom,
  base: OverlayLine[],
  hotKey: string | null,
  boxes?: Map<string, LabelBox>,
): ChartHit | null {
  if (px < g.padL || px > g.padL + g.plotW) return null;
  const uklad = boxes ?? layoutLabels(g, base);

  // 1. uchwyty tworzace brakujacy poziom — maja pierwszenstwo, bo leza przy etykiecie wejscia
  for (const o of base) {
    if (!o.handles?.length || !hotKey || hotKey !== o.key) continue;
    for (const r of levelHandleRects(g, o, uklad)) {
      if (px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h)
        return { t: "handle", o, kind: r.kind, tickets: r.tickets };
    }
  }

  // 2. przeciagalne poziomy: najpierw „x" w etykiecie, potem etykieta, potem sama linia
  let best: { o: OverlayLine; d: number } | null = null;
  for (const o of base) {
    if (o.drag !== true || !o.tickets?.length) continue;
    const box = uklad.get(o.key);
    if (box?.close) {
      const r = box.close;
      if (px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) return { t: "x", o };
    }
    // etykieta jest czescia poziomu — ma go lapac tak samo jak linia
    if (box && px >= box.x && px <= box.x + box.w && py >= box.y && py <= box.y + box.h)
      return { t: "level", o };
    const dy = Math.abs(g.yOf(o.price) - py);
    if (dy <= LEVEL_HIT_PX && (!best || dy < best.d)) best = { o, d: dy };
  }
  if (best) return { t: "level", o: best.o };

  // 3. linia wejscia — sama nieprzeciagalna, ale niesie uchwyty
  for (const o of base) {
    if (!o.handles?.length) continue;
    if (Math.abs(g.yOf(o.price) - py) <= LEVEL_HIT_PX) return { t: "level", o };
  }
  return null;
}

/** „4031,4" i „4031.4" maja znaczyc to samo — panel jest po polsku. */
export function parsePrice(s: string): number | null {
  const v = Number(s.trim().replace(",", ".").replace(/\s/g, ""));
  return Number.isFinite(v) && v > 0 ? v : null;
}

export interface OverlayArgs {
  positions: Position[];
  pendings: PendingOrder[];
  symbol: string;
  theme: ChartTheme;
  /** czy dopisywac wynik w dolarach (ustawienie „potencjal TP/SL") */
  showPotential: boolean;
  /** miejsca po przecinku INSTRUMENTU — nie 2 na sztywno */
  digits: number;
  /** cena biezaca — do liczenia dystansu w punktach */
  live: number;
  /** wynik w dolarach dla poziomu docelowego (z silnika) */
  moneyAt: (t: Position | PendingOrder, target: number) => number | null;
}

/**
 * Nakladki z pozycji i pendingow (linie na wykresie).
 *
 * Grupowanie po CENIE, nie po zleceniu. Dwie pozycje z tym samym SL dostaja
 * JEDNĄ linie — bo w tym samym miejscu ekranu nie da sie narysowac dwoch —
 * ale linia niesie oba numery zlecen i przeciagniecie rusza oba. Poprzednia
 * wersja odrzucala duplikat calkowicie: druga pozycja nie miala linii i jej
 * poziomu nie dalo sie ruszyc z wykresu w ogole.
 */
export function buildOverlays(a: OverlayArgs): OverlayLine[] {
  const { positions, pendings, symbol, theme, showPotential, digits, live, moneyAt } = a;

  const fmt = (v: number) => v.toFixed(digits);
  const tick = Math.pow(10, -digits);
  const pts = (v: number) => Math.round(Math.abs(v) / tick);

  /** klucz grupowania — cena zaokraglona do ticka instrumentu */
  const at = (v: number) => Math.round(v / tick);

  const groups = new Map<string, OverlayLine>();
  /** ile lotow i jaki wynik zbiera sie na danym poziomie */
  const sums = new Map<string, { vol: number; money: number | null; n: number; obce: number }>();

  /* „Nie nasze" = jawnie oznaczone jako cudze. Brak pola znaczy stary backend
     i traktujemy je jak bota, zgodnie z kontraktem typu `PositionSource`. */
  const obcy = (o: { source?: string }) => o.source !== undefined && o.source !== "BOT";

  const push = (
    key: string,
    make: () => Omit<OverlayLine, "key">,
    ticket: number,
    vol: number,
    money: number | null,
    czyObce = false,
  ) => {
    let g = groups.get(key);
    if (!g) {
      g = { key, tickets: [], ...make() };
      groups.set(key, g);
      sums.set(key, { vol: 0, money: money === null ? null : 0, n: 0, obce: 0 });
    }
    if (g.tickets!.includes(ticket)) return;
    g.tickets!.push(ticket);
    const s = sums.get(key)!;
    s.vol += vol;
    s.n += 1;
    if (czyObce) s.obce += 1;
    if (s.money !== null && money !== null) s.money += money;
    else s.money = null;
  };

  /* ---------- pozycje ---------- */
  const brakSl = new Map<string, number[]>();
  const brakTp = new Map<string, number[]>();

  for (const p of positions) {
    if (p.symbol !== symbol) continue;
    const eKey = `e:${p.direction}:${at(p.openPrice)}`;
    push(
      eKey,
      () => ({
        price: p.openPrice,
        color: p.direction === "BUY" ? theme.long : theme.short,
        label: `${p.direction} ${p.volume.toFixed(2)}`,
        kind: "entry",
        owner: "pos",
      }),
      p.ticket,
      p.volume,
      null,
      obcy(p),
    );
    if (p.sl === null) (brakSl.get(eKey) ?? brakSl.set(eKey, []).get(eKey)!).push(p.ticket);
    if (p.tp === null) (brakTp.get(eKey) ?? brakTp.set(eKey, []).get(eKey)!).push(p.ticket);

    if (p.sl !== null) {
      push(
        `pos:sl:${at(p.sl)}`,
        () => ({ price: p.sl!, color: theme.short, label: `SL ${fmt(p.sl!)}`, dash: [5, 4], kind: "sl", owner: "pos", drag: true }),
        p.ticket,
        p.volume,
        moneyAt(p, p.sl),
        obcy(p),
      );
    }
    if (p.tp !== null) {
      push(
        `pos:tp:${at(p.tp)}`,
        () => ({ price: p.tp!, color: theme.long, label: `TP ${fmt(p.tp!)}`, dash: [5, 4], kind: "tp", owner: "pos", drag: true }),
        p.ticket,
        p.volume,
        moneyAt(p, p.tp),
        obcy(p),
      );
    }
  }

  /* ---------- zlecenia oczekujace ---------- */
  for (const o of pendings) {
    if (o.symbol !== symbol) continue;
    push(
      `pend:px:${at(o.price)}`,
      () => ({
        price: o.price,
        color: theme.warn,
        label: `${o.kind.replace("_", " ")} ${o.volume.toFixed(2)}`,
        dash: [2, 3],
        kind: "pending",
        owner: "pend",
        drag: true,
      }),
      o.ticket,
      o.volume,
      null,
      obcy(o),
    );
    if (o.sl !== null) {
      push(
        `pend:sl:${at(o.sl)}`,
        () => ({ price: o.sl!, color: theme.short, label: `SL ${fmt(o.sl!)}`, dash: [2, 4], kind: "sl", owner: "pend", drag: true }),
        o.ticket,
        o.volume,
        moneyAt(o, o.sl),
        obcy(o),
      );
    }
    if (o.tp !== null) {
      push(
        `pend:tp:${at(o.tp)}`,
        () => ({ price: o.tp!, color: theme.long, label: `TP ${fmt(o.tp!)}`, dash: [2, 4], kind: "tp", owner: "pend", drag: true }),
        o.ticket,
        o.volume,
        moneyAt(o, o.tp),
        obcy(o),
      );
    }
  }

  /* ---------- opisy ---------- */
  const out: OverlayLine[] = [];
  for (const [key, g] of groups) {
    const s = sums.get(key)!;
    const n = g.tickets!.length;

    if (g.kind === "entry") {
      g.label = `${g.label.split(" ")[0]} ${s.vol.toFixed(2)}`;
      g.label += n > 1 ? ` · ${n} poz.` : ` · #${g.tickets![0] % 100000}`;
      const h: { kind: "sl" | "tp"; tickets: number[] }[] = [];
      const bs = brakSl.get(key);
      const bt = brakTp.get(key);
      if (bs?.length) h.push({ kind: "sl", tickets: bs });
      if (bt?.length) h.push({ kind: "tp", tickets: bt });
      if (h.length) g.handles = h;
    } else if (n > 1) {
      g.label += ` ×${n}`;
    }

    /* Ile z tego jest CUDZE. Piszemy to na linii, bo grupowanie po cenie
       potrafi zebrać pod jednym pociągnięciem kilkanaście zleceń starego
       bota — wolno je ruszyć, ale nie przez pomyłkę. */
    if (s.obce > 0) {
      g.foreign = s.obce;
      g.label += s.obce === n ? " · spoza bota" : ` · ${s.obce} spoza bota`;
    }

    /* Dystans od ceny i wynik w dolarach — na stale, nie tylko przy ciagnieciu.
       „O ile punktow i ile mnie to kosztuje" to jedyne dwie liczby, ktore
       naprawde sa potrzebne przy przesuwaniu stopa. */
    if (g.kind !== "entry" && Number.isFinite(live) && live > 0) {
      const d = pts(g.price - live);
      const kier = g.price >= live ? "↑" : "↓";
      const kasa = showPotential && s.money !== null ? ` · ${s.money >= 0 ? "+" : ""}${s.money.toFixed(2)} $` : "";
      g.note = `${kier}${d} pkt${kasa}`;
    }
    out.push(g);
  }

  /* Wejscia na spod — SL/TP maja byc na wierzchu i to one lapia mysz. */
  out.sort((x, y) => (x.kind === "entry" ? 0 : 1) - (y.kind === "entry" ? 0 : 1));
  return out;
}

/** Czego dotyczy linia: otwartej pozycji czy zlecenia oczekujacego. */
export type LineOwner = "pos" | "pend";

export interface OverlayLine {
  /** tozsamosc linii — stala miedzy klatkami, niezalezna od kolejnosci pozycji */
  key: string;
  price: number;
  color: string;
  label: string;
  /** dopisek: dystans od ceny w punktach i wynik w dolarach */
  note?: string;
  dash?: number[];
  kind: "entry" | "sl" | "tp" | "pending" | "zone";
  owner?: LineOwner;
  /**
   * WSZYSTKIE zlecenia lezace na tym poziomie.
   *
   * Kluczowe przy siatce limitow: kilkanascie pozycji potrafi miec ten sam SL.
   * Wczesniej linia niosla jeden `ticket` i przeciagniecie ruszalo wylacznie
   * pierwsza pozycje — reszta zostawala na starym poziomie bez zadnego sladu.
   */
  tickets?: number[];
  /** poziom da sie chwycic mysza */
  drag?: boolean;
  /** kursor nad linia albo trwa ciagniecie */
  hot?: boolean;
  /** linia nalezy do tej samej pozycji co ta pod kursorem */
  kin?: boolean;
  /** pigulka z cena i wynikiem / powodem odrzucenia (w trakcie ciagniecia) */
  badge?: string;
  /** poziom odrzucony przez walidacje — rysowany na czerwono */
  bad?: boolean;
  /** upuszczenie poza wykresem = skasowanie poziomu */
  remove?: boolean;
  /** slad po pierwotnym poziomie w trakcie ciagniecia */
  ghost?: boolean;
  /** uchwyty „+SL" / „+TP" przy linii wejscia — tworzenie brakujacego poziomu */
  handles?: { kind: "sl" | "tp"; tickets: number[] }[];
  
  foreign?: number;
}

export interface GeomArgs {
  candles: Candle[];
  vp: Viewport;
  w: number;
  h: number;
  showVolume?: boolean;
  /** poziomy pozycji i zlecen — probuja sie zmiescic w skali */
  levels?: number[];
  /** cena biezaca; razem z poziomami wyznacza „obszar zainteresowania" */
  live?: number;
  /** minimalny zakres ceny (np. 8 tickow) — chroni przed dzieleniem przez zero */
  minSpan?: number;
  /**
   * Ile pustych swiec zostawic po PRAWEJ, miedzy ostatnia swieca a osia ceny.
   * Bez tego marginesu swieca biezaca cierpi: dokleja sie do osi, jej etykieta
   * nachodzi na podpisy cen, a ruch ceny w gore i w dol nie ma sie gdzie
   * odbyc. Kazdy terminal to ma.
   */
  rightPadBars?: number;
}


const FIT_RATIO = 4;

export function computeGeom(a: GeomArgs): ChartGeom {
  const { candles, vp, w, h } = a;
  const showVolume = a.showVolume ?? false;
  const extraLevels = a.levels ?? [];
  const minSpan = a.minSpan ?? 0;
  const padL = 8;
  const padR = AXIS_W;
  const padT = PAD_T;
  const padB = TIME_H;
  const plotW = Math.max(10, w - padL - padR);
  const plotH = Math.max(10, h - padT - padB);
  const volH = showVolume ? Math.round(plotH * 0.16) : 0;
  const priceH = Math.max(8, plotH - volH);

  const n = candles.length;

  
  const rawBars = Number.isFinite(vp.bars) ? vp.bars : 140;
  const bars = Math.min(Math.max(rawBars, MIN_BARS), Math.max(MIN_BARS, Math.min(n || MIN_BARS, MAX_BARS)));
  const rawOff = Number.isFinite(vp.offset) ? vp.offset : 0;
  const offset = Math.min(Math.max(rawOff, 0), Math.max(0, n - 1));

  /* Margines z prawej liczony W SWIECACH, wiec trzyma sie przy kazdym zoomie.
     `offset = 0` nadal znaczy „przyklejone do najnowszej swiecy" — zmienia sie
     tylko to, ze najnowsza swieca nie dotyka juz osi ceny. */
  const rightPad = Math.max(0, a.rightPadBars ?? 0);
  const viewEnd = n - offset + rightPad;
  const viewStart = viewEnd - bars;
  const barW = plotW / bars;

  // indeksy do rysowania — z zapasem jednej swiecy po bokach (czesciowo widoczne)
  const first = Math.max(0, Math.floor(viewStart) - 1);
  const last = Math.min(n, Math.ceil(viewEnd) + 1);

  /* ---- zakres cen TYLKO z tego, co naprawde widac ----
     Przy glebokim zoomie w czasie os ceny musi sie zawezic razem z nim,
     inaczej swiece robia sie plaskie. */
  let minP = Infinity;
  let maxP = -Infinity;
  const i0 = Math.max(0, Math.floor(viewStart));
  const i1 = Math.min(n, Math.ceil(viewEnd));
  for (let i = i0; i < i1; i++) {
    const k = candles[i];
    if (!k) continue;
    if (k.l < minP) minP = k.l;
    if (k.h > maxP) maxP = k.h;
  }
  if (!Number.isFinite(minP) || !Number.isFinite(maxP)) {
    // brak swiec w widoku — oprzyj sie o cokolwiek sensownego
    const ref = candles[Math.min(Math.max(i0, 0), Math.max(0, n - 1))]?.c ?? extraLevels.find((x) => Number.isFinite(x)) ?? 1;
    minP = ref;
    maxP = ref;
  }

  /* ---- obszar zainteresowania: poziomy + cena biezaca ----
     Poziomy licza sie OSOBNO od ceny biezacej: sama cena biezaca to punkt,
     a nie zakres, i dopasowanie do niej zwijalo os do ulamka dolara wokol
     kreski — swiece znikaly z ekranu, chociaz nie bylo czego edytowac. */
  const lv: number[] = [];
  for (const x of extraLevels) if (Number.isFinite(x)) lv.push(x);
  const focus = lv.slice();
  if (Number.isFinite(a.live) && (a.live as number) > 0) focus.push(a.live as number);
  const fLo = focus.length ? Math.min(...focus) : NaN;
  const fHi = focus.length ? Math.max(...focus) : NaN;

  /* Najwezsze okno, do jakiego wolno dopasowac skale. Bez tej podlogi jedna
     pozycja z SL i TP obok siebie robila z wykresu lupe na trzy linie. */
  const minOkno = Math.max(Math.abs(fHi || 0) * 1.5e-3, minSpan * 40);
  const fitSpan = Math.max(fHi - fLo, minOkno);

  let scale: ChartGeom["scale"] = "auto";

  if (vp.price && Number.isFinite(vp.price.mid) && vp.price.span > 0) {
    /* --- 1. reczna skala (ciagniecie osi ceny) --- */
    scale = "manual";
    minP = vp.price.mid - vp.price.span / 2;
    maxP = vp.price.mid + vp.price.span / 2;
  } else if (
    lv.length > 0 &&
    maxP - minP > FIT_RATIO * fitSpan &&
    (() => {
      /* Straznik czytelnosci. Dopasowanie do poziomow ma UŁATWIĆ chwytanie
         SL i TP, a nie schowac wykres. Gdy okno poziomow nie obejmuje
         biezacej ceny albo miesci znikoma czesc ruchu swiec, na ekranie
         zostaja pionowe kreski w miejscach, gdzie cena akurat przecina kadr —
         i nie widac ani ceny, ani poziomow w kontekscie.

         Zgloszone przez uzytkownika: os 45 $ (3998–4043) przy swiecach
         o rozpietosci 745 $ dawala cztery kreski zamiast wykresu. */
      const mid = (fLo + fHi) / 2;
      const lo = mid - fitSpan * 0.85;
      const hi = mid + fitSpan * 0.85;
      const cena =
        Number.isFinite(a.live) && (a.live as number) > 0
          ? (a.live as number)
          : (candles[Math.max(0, i1 - 1)]?.c ?? (minP + maxP) / 2);
      if (!(cena >= lo && cena <= hi)) return false;
      const wspolne = Math.min(hi, maxP) - Math.max(lo, minP);
      return wspolne > 0 && wspolne / (maxP - minP) >= 0.12;
    })()
  ) {
    /* --- 2. swiece sa nieporownanie szersze niz to, co edytujemy --- */
    scale = "fit";
    const mid = (fLo + fHi) / 2;
    minP = mid - fitSpan * 0.85;
    maxP = mid + fitSpan * 0.85;
  } else {
    /* --- 3. swiece rzadza, poziomy tylko rozpychaja — do rozsadku ---
       Bez limitu jeden daleki SL splaszczal cale swiece do kreski. */
    const core = maxP - minP;
    const cap = Math.max(core * 2.5, Math.max(minSpan, Math.abs(maxP) * 2e-5, 1e-9));
    for (const x of extraLevels) {
      if (!Number.isFinite(x)) continue;
      const lo = Math.min(minP, x);
      const hi = Math.max(maxP, x);
      if (hi - lo <= cap) {
        minP = lo;
        maxP = hi;
      }
    }
  }

  /* Zerowy zakres (jedna plaska swieca, wszystkie ceny rowne) = dzielenie
     przez zero w yOf/pOf. Awaryjne minimum rozwiazuje to raz na zawsze. */
  let span = maxP - minP;
  const floor = Math.max(minSpan, Math.abs((maxP + minP) / 2) * 2e-5, 1e-9);
  if (!(span > floor)) {
    const mid = (maxP + minP) / 2;
    minP = mid - floor / 2;
    maxP = mid + floor / 2;
    span = floor;
  }
  if (scale !== "manual") {
    minP -= span * 0.07;
    maxP += span * 0.07;
  }
  const range = maxP - minP; // > 0 gwarantowane

  const xOf = (i: number) => padL + (i - viewStart + 0.5) * barW;
  const yOf = (p: number) => padT + ((maxP - p) / range) * priceH;
  const iOf = (x: number) => viewStart + (x - padL) / barW - 0.5;
  const pOf = (y: number) => maxP - ((y - padT) / priceH) * range;

  return {
    padL,
    padR,
    padT,
    padB,
    plotW,
    plotH,
    w,
    h,
    barW,
    first,
    last,
    viewStart,
    viewEnd,
    bars,
    minP,
    maxP,
    priceH,
    xOf,
    yOf,
    iOf,
    pOf,
    volH,
    perPx: range / priceH,
    scale,
  };
}

/** Ładne kroki osi cen. */
function niceStep(range: number, target: number): number {
  const raw = range / Math.max(1, target);
  if (!(raw > 0)) return 1;
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const step = norm < 1.5 ? 1 : norm < 3 ? 2 : norm < 7 ? 5 : 10;
  return step * mag;
}

/** Co ile swiec podpisywac os czasu — „ladne" krotnosci, zeby podpisy nie tanczyly. */
const EVERY_STEPS = [1, 2, 3, 5, 10, 15, 20, 30, 60, 120, 180, 240, 360, 720, 1440];
function niceEvery(x: number): number {
  for (const s of EVERY_STEPS) if (s >= x) return s;
  return Math.ceil(x / 1440) * 1440;
}

/** UTC getters preserve broker wall-clock labels; explicit offsets never depend on browser DST. */
export const labelDate = (t: number, clockOffsetMs = 0) => new Date(t + clockOffsetMs);

const TF_LABEL: Record<Timeframe, (d: Date) => string> = {
  "1m": (d) => `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}`,
  "5m": (d) => `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}`,
  "15m": (d) => `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}`,
  "1h": (d) => `${String(d.getUTCHours()).padStart(2, "0")}:00`,
  "4h": (d) => `${d.getUTCDate()}.${d.getUTCMonth() + 1} ${String(d.getUTCHours()).padStart(2, "0")}h`,
  "1d": (d) => `${d.getUTCDate()}.${String(d.getUTCMonth() + 1).padStart(2, "0")}`,
};

/**
 * Krok osi czasu + format podpisu dobierany do ZOOMU, nie na sztywno.
 * `barW` w pikselach decyduje, ile podpisow sie zmiesci bez nachodzenia —
 * mierzymy realna szerokosc tekstu, nie zgadujemy stalej 72 px.
 */
function timeAxis(
  candles: Candle[],
  g: ChartGeom,
  tf: Timeframe,
  measure: (s: string) => number,
  clockOffsetMs = 0,
): { every: number; slotMs: number; fmt: (d: Date) => string } {
  const dt = (candles[1]?.t ?? 0) - (candles[0]?.t ?? 0);
  const slotMs = dt > 0 ? dt : 60000;
  // przy bardzo szerokich swiecach warto dolozyc date — inaczej „14:35" wisi bez kontekstu
  const wide = g.barW > 130 && (tf === "1m" || tf === "5m" || tf === "15m" || tf === "1h");
  const base = TF_LABEL[tf];
  const fmt = wide ? (d: Date) => `${d.getUTCDate()}.${String(d.getUTCMonth() + 1).padStart(2, "0")} ${base(d)}` : base;
  const sample = fmt(labelDate(candles[Math.max(0, g.last - 1)]?.t ?? Date.now(), clockOffsetMs));
  const need = measure(sample) + 26;
  return { every: niceEvery(need / Math.max(0.01, g.barW)), slotMs, fmt };
}

/** Zamkniety interes naniesiony na os czasu — wejscie, wyjscie i wynik. */
export interface TradeMark {
  ticket: number;
  dir: "BUY" | "SELL";
  volume: number;
  openTime: number;
  openPrice: number;
  closeTime: number;
  closePrice: number;
  /** wynik NETTO (z prowizja i swapem) — to on decyduje o kolorze */
  net: number | null;
  /** powod zamkniecia ze skonczonej listy (`CloseReason`) albo z dziennika */
  reason: string;
  /** czy to NIE nasz handel (inny magic) */
  foreign: boolean;
}


export function idxOfTime(candles: Candle[], t: number): number {
  const n = candles.length;
  if (n === 0) return 0;
  const krok = n > 1 ? candles[1].t - candles[0].t : 60000;
  if (t <= candles[0].t) return (t - candles[0].t) / Math.max(1, krok);
  if (t >= candles[n - 1].t) return n - 1 + (t - candles[n - 1].t) / Math.max(1, krok);

  let lo = 0;
  let hi = n - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (candles[mid].t <= t) lo = mid;
    else hi = mid;
  }
  const szer = Math.max(1, candles[hi].t - candles[lo].t);
  return lo + (t - candles[lo].t) / szer;
}

/** Promien trafienia w znacznik wejscia/wyjscia. */
export const TRADE_HIT_PX = 7;

/** Gdzie na plotnie leza oba konce interesu. */
export function tradePoints(g: ChartGeom, candles: Candle[], t: TradeMark) {
  return {
    x1: g.xOf(idxOfTime(candles, t.openTime)),
    y1: g.yOf(t.openPrice),
    x2: g.xOf(idxOfTime(candles, t.closeTime)),
    y2: g.yOf(t.closePrice),
  };
}

/** Ktory znacznik jest pod kursorem (blizszy koniec wygrywa). */
export function hitTrade(
  px: number,
  py: number,
  g: ChartGeom,
  candles: Candle[],
  trades: TradeMark[],
): { trade: TradeMark; koniec: "in" | "out" } | null {
  let best: { trade: TradeMark; koniec: "in" | "out"; d: number } | null = null;
  for (const t of trades) {
    const p = tradePoints(g, candles, t);
    const d1 = Math.hypot(p.x1 - px, p.y1 - py);
    const d2 = Math.hypot(p.x2 - px, p.y2 - py);
    if (d1 <= TRADE_HIT_PX && (!best || d1 < best.d)) best = { trade: t, koniec: "in", d: d1 };
    if (d2 <= TRADE_HIT_PX && (!best || d2 < best.d)) best = { trade: t, koniec: "out", d: d2 };
  }
  return best ? { trade: best.trade, koniec: best.koniec } : null;
}

export interface RenderArgs {
  ctx: CanvasRenderingContext2D;
  candles: Candle[];
  geom: ChartGeom;
  theme: ChartTheme;
  style: "candle" | "line" | "area";
  tf: Timeframe;
  digits: number;
  showVolume: boolean;
  overlays: OverlayLine[];
  drawings: Drawing[];
  cursor: { x: number; y: number } | null;
  livePrice: number;
  preview?: Drawing | null;
  /** roznica zegara swiec i przegladarki (swiece z MT5 ida w czasie brokera) */
  clockOffsetMs?: number;
  /** zamkniete interesy naniesione na os czasu */
  trades?: TradeMark[];
  /** znacznik pod kursorem — rysowany grubiej */
  hotTrade?: number | null;
}

export function renderChart(a: RenderArgs) {
  const { ctx, candles, geom: g, theme, style, tf, digits, showVolume, overlays, drawings, cursor, livePrice } = a;
  const { padL, padT, plotW, plotH, w, h, barW, first, last, minP, maxP, volH, priceH } = g;

  ctx.clearRect(0, 0, w, h);

  /* ---------- siatka pozioma + os cen ----------
     Krok NIGDY mniejszy niz jeden tick instrumentu — inaczej przy glebokim
     zoomie os rysowala kilkanascie linii z identycznym podpisem. */
  const tick = Math.pow(10, -digits);
  const step = Math.max(niceStep(maxP - minP, Math.max(2, Math.round(priceH / 54))), tick);

  ctx.font = "500 10.5px ui-monospace, SFMono-Regular, Consolas, monospace";
  ctx.textBaseline = "middle";

  // iteracja po KROTNOSCIACH kroku — dodawanie `+= step` kumulowalo blad zmiennoprzecinkowy
  const m0 = Math.ceil(minP / step - 1e-9);
  const m1 = Math.floor(maxP / step + 1e-9);
  for (let m = m0; m <= m1; m++) {
    const p = m * step;
    const y = g.yOf(p);
    if (y < padT - 1 || y > padT + priceH + 1) continue;
    ctx.strokeStyle = theme.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padL, Math.round(y) + 0.5);
    ctx.lineTo(padL + plotW, Math.round(y) + 0.5);
    ctx.stroke();

    ctx.fillStyle = theme.axis;
    ctx.textAlign = "left";
    ctx.fillText(p.toFixed(digits), padL + plotW + 8, y);
  }

  /* ---------- siatka pionowa + os czasu ---------- */
  const tz = a.clockOffsetMs ?? 0;
  const ax = timeAxis(candles, g, tf, (s) => ctx.measureText(s).width, tz);
  ctx.textAlign = "center";
  for (let i = first; i < last; i++) {
    const k = candles[i];
    if (!k) continue;
    // kotwiczenie po CZASIE, nie po `i - first` — inaczej podpisy tancza przy przesuwaniu
    if (Math.round(k.t / ax.slotMs) % ax.every !== 0) continue;
    const x = g.xOf(i);
    if (x < padL - 2 || x > padL + plotW + 2) continue;
    ctx.strokeStyle = theme.grid;
    ctx.beginPath();
    ctx.moveTo(Math.round(x) + 0.5, padT);
    ctx.lineTo(Math.round(x) + 0.5, padT + plotH);
    ctx.stroke();
    ctx.fillStyle = theme.axis;
    ctx.fillText(ax.fmt(labelDate(k.t, tz)), x, h - TIME_H / 2);
  }

  /* ---------- wolumen ---------- */
  if (showVolume && volH > 4) {
    let maxV = 0;
    for (let i = first; i < last; i++) maxV = Math.max(maxV, candles[i]?.v ?? 0);
    const base = padT + plotH;
    for (let i = first; i < last; i++) {
      const k = candles[i];
      if (!k) continue;
      const bh = maxV > 0 ? (k.v / maxV) * (volH - 4) : 0;
      ctx.fillStyle = k.c >= k.o ? theme.upFill : theme.downFill;
      ctx.fillRect(g.xOf(i) - barW * 0.34, base - bh, Math.max(1, barW * 0.68), bh);
    }
  }

  /* ---------- seria ---------- */
  ctx.save();
  ctx.beginPath();
  ctx.rect(padL, padT - 2, plotW, priceH + 4);
  ctx.clip();

  if (style === "candle") {
    const body = Math.max(1, Math.min(barW * 0.68, MAX_BODY_PX));
    for (let i = first; i < last; i++) {
      const k = candles[i];
      if (!k) continue;
      const x = g.xOf(i);
      const up = k.c >= k.o;
      const col = up ? theme.up : theme.down;
      ctx.strokeStyle = col;
      ctx.fillStyle = col;
      ctx.lineWidth = Math.max(0.8, Math.min(barW * 0.09, 5));

      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, g.yOf(k.h));
      ctx.lineTo(Math.round(x) + 0.5, g.yOf(k.l));
      ctx.stroke();

      const yO = g.yOf(k.o);
      const yC = g.yOf(k.c);
      const top = Math.min(yO, yC);
      const hgt = Math.max(1, Math.abs(yC - yO));
      if (barW > 2.4) ctx.fillRect(x - body / 2, top, body, hgt);
    }
  } else {
    ctx.beginPath();
    let started = false;
    for (let i = first; i < last; i++) {
      const k = candles[i];
      if (!k) continue;
      const x = g.xOf(i);
      const y = g.yOf(k.c);
      if (!started) {
        ctx.moveTo(x, y);
        started = true;
      } else ctx.lineTo(x, y);
    }
    if (style === "area" && started) {
      const grad = ctx.createLinearGradient(0, padT, 0, padT + priceH);
      grad.addColorStop(0, theme.areaTop);
      grad.addColorStop(1, theme.areaBottom);
      ctx.save();
      ctx.lineTo(g.xOf(last - 1), padT + priceH);
      ctx.lineTo(g.xOf(first), padT + priceH);
      ctx.closePath();
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.restore();

      ctx.beginPath();
      started = false;
      for (let i = first; i < last; i++) {
        const k = candles[i];
        if (!k) continue;
        const x = g.xOf(i);
        const y = g.yOf(k.c);
        if (!started) {
          ctx.moveTo(x, y);
          started = true;
        } else ctx.lineTo(x, y);
      }
    }
    ctx.strokeStyle = theme.line;
    ctx.lineWidth = 1.8;
    ctx.lineJoin = "round";
    ctx.stroke();
  }

  /* ---------- rysunki uzytkownika ---------- */
  const xOfTime = (t: number) => {
    // szukaj indeksu po czasie (kubelki sa rownomierne)
    const t0 = candles[0]?.t ?? 0;
    const t1 = candles[1]?.t ?? t0 + 60000;
    const dt = Math.max(1, t1 - t0);
    return g.xOf((t - t0) / dt);
  };

  const paintDrawing = (d: Drawing) => {
    if (d.pts.length < 2 && d.tool !== "hline") return;
    ctx.strokeStyle = d.color;
    ctx.lineWidth = d.width;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    if (d.tool === "hline") {
      const y = g.yOf(d.pts[0][1]);
      ctx.moveTo(padL, y);
      ctx.lineTo(padL + plotW, y);
    } else if (d.tool === "rect") {
      const [p0, p1] = [d.pts[0], d.pts[d.pts.length - 1]];
      const x0 = xOfTime(p0[0]);
      const y0 = g.yOf(p0[1]);
      const x1 = xOfTime(p1[0]);
      const y1 = g.yOf(p1[1]);
      ctx.rect(x0, y0, x1 - x0, y1 - y0);
    } else if (d.tool === "line" || d.tool === "ray") {
      const p0 = d.pts[0];
      const p1 = d.pts[d.pts.length - 1];
      ctx.moveTo(xOfTime(p0[0]), g.yOf(p0[1]));
      ctx.lineTo(xOfTime(p1[0]), g.yOf(p1[1]));
    } else {
      d.pts.forEach((p, i) => {
        const x = xOfTime(p[0]);
        const y = g.yOf(p[1]);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
    }
    ctx.stroke();
  };

  for (const d of drawings) paintDrawing(d);
  if (a.preview) paintDrawing(a.preview);

  ctx.restore();

  /* ---------- nakladki pozycji (SL / TP / wejscie / pending) ----------
     Etykiety licza sie RAZ, wspolnie z trafianiem mysza, i lezą przy PRAWEJ
     krawedzi — rozsuniete tak, zeby na siebie nie nachodzily. Przy siatce
     limitow poziomow bywa kilkanascie i stara wersja (kazda etykieta przy
     lewej krawedzi, na wysokosci swojej linii) zlewala je w nieczytelna plame. */
  ctx.font = "600 10px ui-monospace, SFMono-Regular, Consolas, monospace";
  const uklad = layoutLabelsFull(g, overlays);
  const boxes = uklad.boxes;

  for (const o of overlays) {
    const y = g.yOf(o.price);
    if (y < padT - 1 || y > padT + priceH + 1) continue;
    const col = o.bad ? theme.down : o.color;
    ctx.save();
    ctx.globalAlpha = o.ghost ? 0.34 : 1;
    ctx.strokeStyle = col;
    ctx.lineWidth = o.hot ? 2.2 : o.kin ? 1.6 : o.kind === "entry" ? 1.4 : 1;
    ctx.setLineDash(o.ghost ? [2, 4] : o.hot && !o.bad ? [] : (o.dash ?? []));
    ctx.beginPath();
    ctx.moveTo(padL, Math.round(y) + 0.5);
    ctx.lineTo(padL + plotW, Math.round(y) + 0.5);
    ctx.stroke();
    ctx.setLineDash([]);

    const box = boxes.get(o.key);
    if (box && !o.ghost) {
      /* Odnoga od linii do etykiety — bez niej po rozsunieciu nie wiadomo,
         ktora etykieta nalezy do ktorej linii. */
      if (Math.abs(box.y + box.h / 2 - y) > 1.5) {
        ctx.globalAlpha = 0.5;
        ctx.beginPath();
        ctx.moveTo(box.x - 6, Math.round(y) + 0.5);
        ctx.lineTo(box.x, Math.round(box.y + box.h / 2) + 0.5);
        ctx.stroke();
        ctx.globalAlpha = 1;
      }

      ctx.fillStyle = col;
      ctx.globalAlpha = o.hot ? 0.92 : o.kin ? 0.8 : 0.7;
      roundRect(ctx, box.x, box.y, box.w, box.h, 4);
      ctx.fill();
      ctx.globalAlpha = 1;

      ctx.fillStyle = theme.bg;
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.fillText(labelText(o), box.x + 6, box.y + box.h / 2 + 0.5);

      /* maly „x" — kasuje poziom (ta sama sciezka potwierdzenia co przeciagniecie) */
      if (box.close) {
        const r = box.close;
        ctx.strokeStyle = theme.bg;
        ctx.lineWidth = 1.6;
        ctx.beginPath();
        ctx.moveTo(r.x + 4, r.y + 4);
        ctx.lineTo(r.x + r.w - 4, r.y + r.h - 4);
        ctx.moveTo(r.x + r.w - 4, r.y + 4);
        ctx.lineTo(r.x + 4, r.y + r.h - 4);
        ctx.stroke();
      }
    }

    /* uchwyty „+SL" / „+TP" — pozycja bez poziomu nie ma czego przeciagac,
       wiec poziom trzeba dac czym UTWORZYC */
    if (o.hot && o.handles?.length) {
      ctx.font = "700 9.5px ui-monospace, SFMono-Regular, Consolas, monospace";
      for (const r of levelHandleRects(g, o, boxes)) {
        const hc = r.kind === "sl" ? theme.short : theme.long;
        ctx.fillStyle = theme.surface;
        ctx.strokeStyle = hc;
        ctx.lineWidth = 1;
        roundRect(ctx, r.x, r.y, r.w, r.h, 4);
        ctx.fill();
        ctx.stroke();
        ctx.fillStyle = hc;
        ctx.textAlign = "center";
        ctx.fillText(`+${r.kind.toUpperCase()}`, r.x + r.w / 2, r.y + r.h / 2 + 0.5);
      }
      ctx.textAlign = "left";
      ctx.font = "600 10px ui-monospace, SFMono-Regular, Consolas, monospace";
    }

    /* pigulka w trakcie ciagniecia: cena + wynik w $ albo powod odrzucenia */
    if (o.badge) {
      ctx.font = "600 10.5px ui-monospace, SFMono-Regular, Consolas, monospace";
      const bw = ctx.measureText(o.badge).width + 14;
      const bx = Math.max(padL + 4, (box ? box.x : padL + plotW) - bw - 10);
      ctx.fillStyle = theme.surface;
      ctx.strokeStyle = col;
      ctx.lineWidth = 1;
      roundRect(ctx, bx, y - 9, bw, 18, 4);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = col;
      ctx.textAlign = "left";
      ctx.fillText(o.badge, bx + 7, y + 0.5);
      ctx.font = "600 10px ui-monospace, SFMono-Regular, Consolas, monospace";
    }
    ctx.restore();
  }

  /* ---------- momenty decyzji: wejscia i wyjscia ----------
     Rysowane POD nakladkami poziomow (te sa wazniejsze, bo interaktywne),
     ale NAD swiecami. Przyciete do obszaru wykresu. */
  if (a.trades?.length) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(padL, padT - 2, plotW, priceH + 4);
    ctx.clip();

    for (const t of a.trades) {
      const p = tradePoints(g, candles, t);
      // poza kadrem w poziomie — nie ma czego rysowac
      if (Math.max(p.x1, p.x2) < padL - 20 || Math.min(p.x1, p.x2) > padL + plotW + 20) continue;

      const col = t.net === null ? theme.textDim : t.net >= 0 ? theme.up : theme.down;
      const gorace = a.hotTrade === t.ticket;

      /* Linia trzymania: od wejscia do wyjscia. Pokazuje JAK DLUGO pozycja
         zyla i ile ceny przeszla — przy runnerach trzymanych do 480 minut
         to jest najwazniejsza informacja na wykresie. */
      ctx.globalAlpha = gorace ? 0.95 : 0.5;
      ctx.strokeStyle = col;
      ctx.lineWidth = gorace ? 2 : 1.2;
      ctx.setLineDash(t.foreign ? [3, 3] : []);
      ctx.beginPath();
      ctx.moveTo(p.x1, p.y1);
      ctx.lineTo(p.x2, p.y2);
      ctx.stroke();
      ctx.setLineDash([]);

      ctx.globalAlpha = 1;
      const r = gorace ? 6 : 4.5;

      /* Wejscie: trojkat w strone handlu. Wypelniony = nasz, pusty = cudzy. */
      const wKierunku = t.dir === "BUY" ? -1 : 1;
      ctx.beginPath();
      ctx.moveTo(p.x1, p.y1 + r * wKierunku);
      ctx.lineTo(p.x1 - r, p.y1 - r * wKierunku);
      ctx.lineTo(p.x1 + r, p.y1 - r * wKierunku);
      ctx.closePath();
      ctx.fillStyle = t.foreign ? theme.bg : col;
      ctx.strokeStyle = col;
      ctx.lineWidth = 1.4;
      ctx.fill();
      ctx.stroke();

      /* Wyjscie: kolko. */
      ctx.beginPath();
      ctx.arc(p.x2, p.y2, r * 0.85, 0, Math.PI * 2);
      ctx.fillStyle = t.foreign ? theme.bg : col;
      ctx.fill();
      ctx.stroke();

      /* Podpis tylko przy najechaniu — inaczej dzien z kilkudziesiecioma
         wejsciami zamienia sie w scianie tekstu. */
      if (gorace) {
        ctx.font = "700 10px ui-monospace, SFMono-Regular, Consolas, monospace";
        const result = t.net === null ? "—" : `${t.net >= 0 ? "+" : ""}${t.net.toFixed(2)} $`;
        const txt = `${t.dir} ${t.volume.toFixed(2)} · ${t.reason} · ${result}`;
        const tw = ctx.measureText(txt).width + 12;
        const bx = Math.min(Math.max(p.x2 + 10, padL + 2), padL + plotW - tw - 2);
        const by = p.y2 - 9;
        ctx.fillStyle = theme.surface;
        ctx.strokeStyle = col;
        ctx.lineWidth = 1;
        roundRect(ctx, bx, by, tw, 18, 4);
        ctx.fill();
        ctx.stroke();
        ctx.fillStyle = col;
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        ctx.fillText(txt, bx + 6, by + 9);
        ctx.font = "600 10px ui-monospace, SFMono-Regular, Consolas, monospace";
      }
    }
    ctx.restore();
  }

  /* Licznik poziomow bez podpisu. Przy siatce limitow etykiet jest wiecej,
     niz miesci sie na wysokosci — linie rysujemy wszystkie, ale podpisy
     przerzedzamy i mowimy WPROST, ile ich brakuje. Milczace ukrycie polowy
     poziomow byloby gorsze niz nieczytelny stos. */
  if (uklad.hidden > 0) {
    ctx.save();
    ctx.font = "600 9.5px ui-monospace, SFMono-Regular, Consolas, monospace";
    const txt = t("chart.hiddenLevels", { n: uklad.hidden });
    const tw = ctx.measureText(txt).width + 12;
    const bx = padL + plotW - tw - 6;
    const by = padT + priceH - 17;
    ctx.fillStyle = theme.surface;
    ctx.strokeStyle = theme.border;
    ctx.lineWidth = 1;
    roundRect(ctx, bx, by, tw, 15, 4);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = theme.textDim;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText(txt, bx + 6, by + 8);
    ctx.restore();
  }

  /* ---------- linia ceny biezacej ---------- */
  const yLive = g.yOf(livePrice);
  if (yLive >= padT && yLive <= padT + priceH) {
    ctx.save();
    ctx.strokeStyle = theme.accent;
    ctx.setLineDash([3, 3]);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padL, Math.round(yLive) + 0.5);
    ctx.lineTo(padL + plotW, Math.round(yLive) + 0.5);
    ctx.stroke();
    ctx.setLineDash([]);

    const label = livePrice.toFixed(digits);
    ctx.font = "600 10.5px ui-monospace, SFMono-Regular, Consolas, monospace";
    const tw = ctx.measureText(label).width + 12;
    ctx.fillStyle = theme.accent;
    roundRect(ctx, padL + plotW + 3, yLive - 9, Math.max(tw, AXIS_W - 10), 18, 4);
    ctx.fill();
    ctx.fillStyle = theme.accentFg;
    ctx.textAlign = "left";
    ctx.fillText(label, padL + plotW + 9, yLive + 0.5);
    ctx.restore();
  }

  /* ---------- krzyz celowniczy ---------- */
  if (cursor) {
    const { x, y } = cursor;
    if (x >= padL && x <= padL + plotW && y >= padT && y <= padT + plotH) {
      ctx.save();
      ctx.strokeStyle = theme.crosshair;
      ctx.setLineDash([2, 3]);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, padT);
      ctx.lineTo(Math.round(x) + 0.5, padT + plotH);
      ctx.moveTo(padL, Math.round(y) + 0.5);
      ctx.lineTo(padL + plotW, Math.round(y) + 0.5);
      ctx.stroke();
      ctx.setLineDash([]);

      const price = g.pOf(y);
      const label = price.toFixed(digits);
      ctx.font = "600 10.5px ui-monospace, SFMono-Regular, Consolas, monospace";
      ctx.fillStyle = theme.surface;
      ctx.strokeStyle = theme.border;
      roundRect(ctx, padL + plotW + 3, y - 9, AXIS_W - 10, 18, 4);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = theme.text;
      ctx.textAlign = "left";
      ctx.fillText(label, padL + plotW + 9, y + 0.5);

      const idx = Math.round(g.iOf(x));
      const k = candles[idx];
      if (k) {
        const tl = TF_LABEL[tf](labelDate(k.t, tz));
        const tw2 = ctx.measureText(tl).width + 14;
        ctx.fillStyle = theme.surface;
        ctx.strokeStyle = theme.border;
        roundRect(ctx, x - tw2 / 2, h - TIME_H + 1, tw2, 17, 4);
        ctx.fill();
        ctx.stroke();
        ctx.fillStyle = theme.text;
        ctx.textAlign = "center";
        ctx.fillText(tl, x, h - TIME_H + 9.5);
      }
      ctx.restore();
    }
  }
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/* ============================================================
   EKSPORT SVG (prawdziwy wektor, nie raster w opakowaniu)
   Rysuje tę samą scenę co renderer canvasowy, ale jako elementy
   SVG — plik można skalować i edytować w Illustratorze/Figmie.
   ============================================================ */

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function renderChartSvg(a: Omit<RenderArgs, "ctx" | "cursor" | "preview">): string {
  const { candles, geom: g, theme, style, tf, digits, showVolume, overlays, drawings, livePrice } = a;
  const { padL, padT, plotW, plotH, w, h, barW, first, last, minP, maxP, volH } = g;
  const priceH = plotH - volH;
  const out: string[] = [];

  const MONO = "ui-monospace, SFMono-Regular, Consolas, monospace";

  out.push(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" font-family="${MONO}">`,
  );
  out.push(`<rect width="${w}" height="${h}" fill="${theme.bg}"/>`);

  /* --- siatka pozioma + oś cen --- */
  const tick = Math.pow(10, -digits);
  const step = Math.max(niceStep(maxP - minP, Math.max(2, Math.round(priceH / 54))), tick);
  const m0 = Math.ceil(minP / step - 1e-9);
  const m1 = Math.floor(maxP / step + 1e-9);
  for (let m = m0; m <= m1; m++) {
    const p = m * step;
    const y = g.yOf(p);
    if (y < padT - 1 || y > padT + priceH + 1) continue;
    out.push(
      `<line x1="${padL}" y1="${y.toFixed(1)}" x2="${padL + plotW}" y2="${y.toFixed(1)}" stroke="${theme.grid}" stroke-width="1"/>`,
    );
    out.push(
      `<text x="${padL + plotW + 8}" y="${(y + 3.5).toFixed(1)}" fill="${theme.axis}" font-size="10.5" font-weight="500">${p.toFixed(digits)}</text>`,
    );
  }

  /* --- siatka pionowa + oś czasu (ten sam dobór kroku co na canvasie) --- */
  const tz = a.clockOffsetMs ?? 0;
  const ax = timeAxis(candles, g, tf, (s) => s.length * 6.1, tz);
  for (let i = first; i < last; i++) {
    const k = candles[i];
    if (!k || Math.round(k.t / ax.slotMs) % ax.every !== 0) continue;
    const x = g.xOf(i);
    if (x < padL - 2 || x > padL + plotW + 2) continue;
    out.push(
      `<line x1="${x.toFixed(1)}" y1="${padT}" x2="${x.toFixed(1)}" y2="${padT + plotH}" stroke="${theme.grid}" stroke-width="1"/>`,
    );
    out.push(
      `<text x="${x.toFixed(1)}" y="${h - 8}" fill="${theme.axis}" font-size="10.5" text-anchor="middle">${esc(
        ax.fmt(labelDate(k.t, tz)),
      )}</text>`,
    );
  }

  /* --- wolumen --- */
  if (showVolume && volH > 4) {
    let maxV = 0;
    for (let i = first; i < last; i++) maxV = Math.max(maxV, candles[i]?.v ?? 0);
    const base = padT + plotH;
    for (let i = first; i < last; i++) {
      const k = candles[i];
      if (!k) continue;
      const bh = maxV > 0 ? (k.v / maxV) * (volH - 4) : 0;
      out.push(
        `<rect x="${(g.xOf(i) - barW * 0.34).toFixed(1)}" y="${(base - bh).toFixed(1)}" width="${Math.max(
          1,
          barW * 0.68,
        ).toFixed(1)}" height="${bh.toFixed(1)}" fill="${k.c >= k.o ? theme.up : theme.down}" fill-opacity="0.28"/>`,
      );
    }
  }

  out.push(`<clipPath id="plot"><rect x="${padL}" y="${padT - 2}" width="${plotW}" height="${priceH + 4}"/></clipPath>`);
  out.push(`<g clip-path="url(#plot)">`);

  /* --- seria --- */
  if (style === "candle") {
    const body = Math.max(1, Math.min(barW * 0.68, MAX_BODY_PX));
    for (let i = first; i < last; i++) {
      const k = candles[i];
      if (!k) continue;
      const x = g.xOf(i);
      const col = k.c >= k.o ? theme.up : theme.down;
      out.push(
        `<line x1="${x.toFixed(1)}" y1="${g.yOf(k.h).toFixed(1)}" x2="${x.toFixed(1)}" y2="${g
          .yOf(k.l)
          .toFixed(1)}" stroke="${col}" stroke-width="1"/>`,
      );
      if (barW > 2.4) {
        const yO = g.yOf(k.o);
        const yC = g.yOf(k.c);
        out.push(
          `<rect x="${(x - body / 2).toFixed(1)}" y="${Math.min(yO, yC).toFixed(1)}" width="${body.toFixed(
            1,
          )}" height="${Math.max(1, Math.abs(yC - yO)).toFixed(1)}" fill="${col}"/>`,
        );
      }
    }
  } else {
    const pts: string[] = [];
    for (let i = first; i < last; i++) {
      const k = candles[i];
      if (!k) continue;
      pts.push(`${g.xOf(i).toFixed(1)},${g.yOf(k.c).toFixed(1)}`);
    }
    if (style === "area" && pts.length) {
      out.push(
        `<defs><linearGradient id="area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${theme.line}" stop-opacity="0.28"/><stop offset="1" stop-color="${theme.line}" stop-opacity="0"/></linearGradient></defs>`,
      );
      out.push(
        `<polygon points="${pts.join(" ")} ${g.xOf(last - 1).toFixed(1)},${padT + priceH} ${g
          .xOf(first)
          .toFixed(1)},${padT + priceH}" fill="url(#area)"/>`,
      );
    }
    out.push(`<polyline points="${pts.join(" ")}" fill="none" stroke="${theme.line}" stroke-width="1.8" stroke-linejoin="round"/>`);
  }

  /* --- rysunki użytkownika --- */
  const t0 = candles[0]?.t ?? 0;
  const t1 = candles[1]?.t ?? t0 + 60000;
  const xOfTime = (t: number) => g.xOf((t - t0) / Math.max(1, t1 - t0));

  for (const d of drawings) {
    if (d.tool === "hline") {
      const y = g.yOf(d.pts[0][1]);
      out.push(`<line x1="${padL}" y1="${y.toFixed(1)}" x2="${padL + plotW}" y2="${y.toFixed(1)}" stroke="${d.color}" stroke-width="${d.width}"/>`);
    } else if (d.tool === "rect" && d.pts.length > 1) {
      const p0 = d.pts[0];
      const p1 = d.pts[d.pts.length - 1];
      const x0 = xOfTime(p0[0]);
      const y0 = g.yOf(p0[1]);
      const x1 = xOfTime(p1[0]);
      const y1 = g.yOf(p1[1]);
      out.push(
        `<rect x="${Math.min(x0, x1).toFixed(1)}" y="${Math.min(y0, y1).toFixed(1)}" width="${Math.abs(x1 - x0).toFixed(
          1,
        )}" height="${Math.abs(y1 - y0).toFixed(1)}" fill="none" stroke="${d.color}" stroke-width="${d.width}"/>`,
      );
    } else if (d.pts.length > 1) {
      const pts = (d.tool === "line" || d.tool === "ray" ? [d.pts[0], d.pts[d.pts.length - 1]] : d.pts)
        .map((p) => `${xOfTime(p[0]).toFixed(1)},${g.yOf(p[1]).toFixed(1)}`)
        .join(" ");
      out.push(
        `<polyline points="${pts}" fill="none" stroke="${d.color}" stroke-width="${d.width}" stroke-linecap="round" stroke-linejoin="round"/>`,
      );
    }
  }

  out.push(`</g>`);

  /* --- nakładki pozycji --- */
  for (const o of overlays) {
    const y = g.yOf(o.price);
    if (y < padT || y > padT + priceH) continue;
    const dash = o.dash ? ` stroke-dasharray="${o.dash.join(" ")}"` : "";
    out.push(
      `<line x1="${padL}" y1="${y.toFixed(1)}" x2="${padL + plotW}" y2="${y.toFixed(1)}" stroke="${o.color}" stroke-width="${
        o.kind === "entry" ? 1.4 : 1
      }"${dash}/>`,
    );
    const tw = o.label.length * 6 + 12;
    out.push(
      `<rect x="${padL + 4}" y="${(y - 8).toFixed(1)}" width="${tw}" height="16" rx="4" fill="${o.color}" fill-opacity="0.16"/>`,
    );
    out.push(
      `<text x="${padL + 10}" y="${(y + 3.5).toFixed(1)}" fill="${o.color}" font-size="10" font-weight="600">${esc(o.label)}</text>`,
    );
  }

  /* --- linia ceny bieżącej --- */
  const yLive = g.yOf(livePrice);
  if (yLive >= padT && yLive <= padT + priceH) {
    out.push(
      `<line x1="${padL}" y1="${yLive.toFixed(1)}" x2="${padL + plotW}" y2="${yLive.toFixed(
        1,
      )}" stroke="${theme.accent}" stroke-width="1" stroke-dasharray="3 3"/>`,
    );
    out.push(
      `<rect x="${padL + plotW + 3}" y="${(yLive - 9).toFixed(1)}" width="${AXIS_W - 10}" height="18" rx="4" fill="${theme.accent}"/>`,
    );
    out.push(
      `<text x="${padL + plotW + 9}" y="${(yLive + 3.5).toFixed(1)}" fill="${theme.accentFg}" font-size="10.5" font-weight="600">${livePrice.toFixed(
        digits,
      )}</text>`,
    );
  }

  out.push(`</svg>`);
  return out.join("\n");
}

export function readTheme(el: HTMLElement): ChartTheme {
  const cs = getComputedStyle(el);
  const v = (n: string, fb: string) => cs.getPropertyValue(n).trim() || fb;
  return {
    bg: v("--bg-surface", "#111520"),
    grid: v("--grid-line", "rgba(255,255,255,.05)"),
    gridStrong: v("--grid-line-strong", "rgba(255,255,255,.09)"),
    text: v("--text", "#e8ecf5"),
    textDim: v("--text-dim", "#a4adc2"),
    axis: v("--chart-axis-text", "#6b7690"),
    up: v("--long", "#26d9a3"),
    down: v("--short", "#ff5c7a"),
    upFill: v("--long-soft", "rgba(38,217,163,.13)"),
    downFill: v("--short-soft", "rgba(255,92,122,.13)"),
    line: v("--accent", "#6d7bff"),
    areaTop: v("--accent-soft", "rgba(109,123,255,.14)"),
    areaBottom: "transparent",
    crosshair: v("--chart-crosshair", "rgba(255,255,255,.35)"),
    surface: v("--bg-surface-3", "#1c2231"),
    border: v("--border-strong", "#2f374b"),
    long: v("--long", "#26d9a3"),
    short: v("--short", "#ff5c7a"),
    accent: v("--accent", "#6d7bff"),
    accentFg: v("--accent-fg", "#ffffff"),
    warn: v("--warn", "#f5b74e"),
  };
}
