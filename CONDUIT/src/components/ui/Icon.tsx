/* ============================================================
   IKONY
   Jeden komponent, jedna spójna siatka 24×24, stroke 1.75.
   Bez zewnętrznych zależności — projekt musi działać offline.
   ============================================================ */

export type IconName =
  | "dashboard"
  | "signal"
  | "channels"
  | "settings"
  | "history"
  | "flask"
  | "logs"
  | "chart"
  | "sun"
  | "moon"
  | "check"
  | "x"
  | "chevron-down"
  | "chevron-right"
  | "chevron-left"
  | "plus"
  | "minus"
  | "search"
  | "trash"
  | "edit"
  | "lock"
  | "unlock"
  | "shield"
  | "shield-alert"
  | "brain"
  | "bolt"
  | "target"
  | "grid"
  | "flag"
  | "trend"
  | "sparkles"
  | "clipboard"
  | "hourglass"
  | "activity"
  | "microscope"
  | "sliders"
  | "mail"
  | "bell"
  | "refresh"
  | "play"
  | "pause"
  | "star"
  | "star-filled"
  | "expand"
  | "collapse"
  | "arrow-up"
  | "arrow-down"
  | "arrow-up-right"
  | "arrow-down-right"
  | "send"
  | "copy"
  | "info"
  | "alert"
  | "wallet"
  | "user"
  | "logout"
  | "telegram"
  | "layers"
  | "pencil"
  | "eraser"
  | "line"
  | "cursor"
  | "download"
  | "filter"
  | "clock"
  | "zap"
  | "hand"
  | "robot"
  | "book"
  | "eye"
  | "eye-off"
  | "external"
  | "menu"
  | "bars"
  | "image"
  | "palette"
  /* COFNIJ / PONÓW (TODO K16). Strzałka zawracająca w lewo i jej lustro —
     ten sam kształt co w edytorach tekstu, żeby nie trzeba było zgadywać. */
  | "undo"
  | "redo"
  | "dot";

const P: Record<IconName, string> = {
  dashboard: "M4 5h6v6H4zM14 5h6v4h-6zM14 13h6v6h-6zM4 15h6v4H4z",
  signal: "M4 18v-4M9 18v-8M14 18V7M19 18V4",
  channels: "M4 7h10M4 12h16M4 17h7M18 5v4M20 7h-4",
  settings:
    "M12 15a3 3 0 100-6 3 3 0 000 6zM19.4 15a1.6 1.6 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.6 1.6 0 00-1.8-.3 1.6 1.6 0 00-1 1.5v.2a2 2 0 11-4 0v-.1a1.6 1.6 0 00-1-1.5 1.6 1.6 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.6 1.6 0 00.3-1.8 1.6 1.6 0 00-1.5-1H2a2 2 0 110-4h.1a1.6 1.6 0 001.5-1 1.6 1.6 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.6 1.6 0 001.8.3H9a1.6 1.6 0 001-1.5V2a2 2 0 114 0v.1a1.6 1.6 0 001 1.5 1.6 1.6 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.6 1.6 0 00-.3 1.8V9a1.6 1.6 0 001.5 1h.2a2 2 0 110 4h-.1a1.6 1.6 0 00-1.5 1z",
  history: "M3 3v5h5M3.05 13A9 9 0 106 5.3L3 8M12 7v5l4 2",
  flask: "M9 3h6M10 3v6L4.5 18a2 2 0 001.7 3h11.6a2 2 0 001.7-3L14 9V3M7.5 14h9",
  logs: "M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6M9 13h6M9 17h6",
  chart: "M3 3v18h18M7 15l4-5 3 3 5-7",
  sun: "M12 17a5 5 0 100-10 5 5 0 000 10zM12 1v2M12 21v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M1 12h2M21 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4",
  moon: "M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z",
  check: "M20 6L9 17l-5-5",
  x: "M18 6L6 18M6 6l12 12",
  "chevron-down": "M6 9l6 6 6-6",
  "chevron-right": "M9 18l6-6-6-6",
  "chevron-left": "M15 18l-6-6 6-6",
  plus: "M12 5v14M5 12h14",
  minus: "M5 12h14",
  search: "M11 19a8 8 0 100-16 8 8 0 000 16zM21 21l-4.35-4.35",
  trash: "M3 6h18M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6M10 11v6M14 11v6",
  edit: "M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7M18.5 2.5a2.1 2.1 0 013 3L12 15l-4 1 1-4z",
  lock: "M5 11h14v10H5zM8 11V7a4 4 0 018 0v4",
  unlock: "M5 11h14v10H5zM8 11V7a4 4 0 017.5-2",
  shield: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
  "shield-alert": "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10zM12 8v4M12 16h.01",
  brain:
    "M9.5 2A2.5 2.5 0 007 4.5v.4A2.5 2.5 0 005 7.3v.4a2.5 2.5 0 000 4.6v.4a2.5 2.5 0 002 2.4v.4a2.5 2.5 0 005 0V2.5A.5.5 0 0011.5 2zM14.5 2A2.5 2.5 0 0117 4.5v.4a2.5 2.5 0 012 2.4v.4a2.5 2.5 0 010 4.6v.4a2.5 2.5 0 01-2 2.4v.4a2.5 2.5 0 01-5 0",
  bolt: "M13 2L4.5 13H11l-1 9 8.5-11H12z",
  target: "M12 21a9 9 0 100-18 9 9 0 000 18zM12 16a4 4 0 100-8 4 4 0 000 8zM12 13a1 1 0 100-2 1 1 0 000 2z",
  grid: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
  flag: "M4 21V4M4 4h11l-1.5 4L15 12H4",
  trend: "M22 7l-8.5 8.5-4-4L2 19M16 7h6v6",
  sparkles: "M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9zM19 3v4M21 5h-4M5 17v4M7 19H3",
  clipboard: "M9 3h6v3H9zM7 5H5a2 2 0 00-2 2v13a2 2 0 002 2h14a2 2 0 002-2V7a2 2 0 00-2-2h-2M8 12h8M8 16h5",
  hourglass: "M6 2h12M6 22h12M8 2v4l4 4 4-4V2M8 22v-4l4-4 4 4v4",
  activity: "M22 12h-4l-3 9L9 3l-3 9H2",
  microscope: "M6 18h8M3 22h18M14 22a7 7 0 100-14h-1M9 14h2M9 12a2 2 0 012-2h1V5a2 2 0 00-2-2H9M9 3v9",
  sliders: "M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6",
  mail: "M4 4h16a2 2 0 012 2v12a2 2 0 01-2 2H4a2 2 0 01-2-2V6a2 2 0 012-2zM22 6l-10 7L2 6",
  bell: "M18 8A6 6 0 006 8c0 7-3 9-3 9h18s-3-2-3-9M13.7 21a2 2 0 01-3.4 0",
  refresh: "M23 4v6h-6M1 20v-6h6M3.5 9a9 9 0 0114.9-3.4L23 10M1 14l4.6 4.4A9 9 0 0020.5 15",
  play: "M6 3l14 9-14 9z",
  pause: "M6 4h4v16H6zM14 4h4v16h-4z",
  star: "M12 2l3.1 6.3 6.9 1-5 4.9 1.2 6.9-6.2-3.3-6.2 3.3L7 14.2l-5-4.9 6.9-1z",
  "star-filled": "M12 2l3.1 6.3 6.9 1-5 4.9 1.2 6.9-6.2-3.3-6.2 3.3L7 14.2l-5-4.9 6.9-1z",
  expand: "M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7",
  collapse: "M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7",
  "arrow-up": "M12 19V5M5 12l7-7 7 7",
  "arrow-down": "M12 5v14M19 12l-7 7-7-7",
  "arrow-up-right": "M7 17L17 7M7 7h10v10",
  "arrow-down-right": "M7 7l10 10M17 7v10H7",
  send: "M22 2L11 13M22 2l-7 20-4-9-9-4z",
  copy: "M9 9h10v12H9zM5 15H4a1 1 0 01-1-1V4a1 1 0 011-1h10a1 1 0 011 1v1",
  info: "M12 22a10 10 0 100-20 10 10 0 000 20zM12 16v-4M12 8h.01",
  alert: "M12 9v4M12 17h.01M10.3 3.9L1.8 18a2 2 0 001.7 3h17a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z",
  wallet: "M20 12V8H6a2 2 0 010-4h12v4M4 6v12a2 2 0 002 2h14v-4M18 12a2 2 0 000 4h4v-4z",
  user: "M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2M12 11a4 4 0 100-8 4 4 0 000 8z",
  logout: "M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9",
  telegram: "M21.5 4.5l-19 7.5 5.5 2 2 6 3-3.5 5 4z M8 14l11-8-7.5 11",
  layers: "M12 2L2 7l10 5 10-5zM2 17l10 5 10-5M2 12l10 5 10-5",
  pencil: "M17 3a2.8 2.8 0 014 4L7.5 20.5 2 22l1.5-5.5z",
  eraser: "M20 20H9l-5-5a2 2 0 010-3l8-8a2 2 0 013 0l6 6a2 2 0 010 3l-6 6M6 13l6 6",
  line: "M4 20L20 4",
  cursor: "M4 3l7 17 2.5-6.5L20 11z",
  download: "M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4M7 10l5 5 5-5M12 15V3",
  filter: "M22 3H2l8 9.5V19l4 2v-8.5z",
  clock: "M12 22a10 10 0 100-20 10 10 0 000 20zM12 6v6l4 2",
  zap: "M13 2L3 14h9l-1 8 10-12h-9z",
  hand: "M18 11V6a2 2 0 00-4 0v5M14 10V4a2 2 0 00-4 0v7M10 10.5V6a2 2 0 00-4 0v8M18 8a2 2 0 114 0v6a8 8 0 01-8 8h-2a8 8 0 01-8-8v-1a2 2 0 114 0",
  robot: "M12 2v3M6 8h12a2 2 0 012 2v8a2 2 0 01-2 2H6a2 2 0 01-2-2v-8a2 2 0 012-2zM9 13h.01M15 13h.01M9 17h6M2 12v4M22 12v4",
  book: "M4 19.5A2.5 2.5 0 016.5 17H20M6.5 2H20v20H6.5A2.5 2.5 0 014 19.5v-15A2.5 2.5 0 016.5 2z",
  eye: "M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8zM12 15a3 3 0 100-6 3 3 0 000 6z",
  "eye-off": "M17.9 17.9A10.8 10.8 0 0112 20C5 20 1 12 1 12a19.6 19.6 0 015.1-5.9M9.9 4.2A10.9 10.9 0 0112 4c7 0 11 8 11 8a19.5 19.5 0 01-2.2 3.2M14.1 14.1a3 3 0 11-4.2-4.2M1 1l22 22",
  external: "M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6M15 3h6v6M10 14L21 3",
  menu: "M3 12h18M3 6h18M3 18h18",
  bars: "M4 20v-6M9 20V9M14 20v-9M19 20v-4M2 20h20",
  image: "M3 3h18v18H3zM8.5 10a1.5 1.5 0 100-3 1.5 1.5 0 000 3zM21 16l-5-5-11 10",
  palette:
    "M12 21a9 9 0 110-18c4.97 0 9 3.58 9 8 0 2.21-1.79 4-4 4h-2a2 2 0 00-1.5 3.3A1.7 1.7 0 0112 21zM6.5 12.5a1 1 0 100-2 1 1 0 000 2zM9.5 8a1 1 0 100-2 1 1 0 000 2zM14.5 8a1 1 0 100-2 1 1 0 000 2z",
  
  undo: "M3 7v6h6M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13",
  redo: "M21 7v6h-6M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6 2.3L21 13",
  dot: "M12 13a1 1 0 100-2 1 1 0 000 2z",
};

const FILLED = new Set<IconName>(["star-filled", "play", "dot"]);

export function Icon({
  name,
  size = 16,
  className,
  strokeWidth = 1.75,
  style,
}: {
  name: IconName;
  size?: number;
  className?: string;
  strokeWidth?: number;
  style?: React.CSSProperties;
}) {
  const filled = FILLED.has(name);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={filled ? 0 : strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={style}
      aria-hidden="true"
      focusable="false"
    >
      <path d={P[name]} />
    </svg>
  );
}
