import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { t } from "@/i18n";
import { Icon, type IconName } from "./Icon";

/* ============================================================
   PRYMITYWY UI
   ============================================================ */

/* ---------------- Card ---------------- */
export function Card({
  title,
  subtitle,
  icon,
  actions,
  children,
  footer,
  accent,
  tight,
  flush,
  className = "",
  style,
  id,
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  icon?: IconName;
  actions?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  accent?: string;
  tight?: boolean;
  flush?: boolean;
  className?: string;
  style?: CSSProperties;
  id?: string;
}) {
  return (
    <section
      id={id}
      className={`card ${accent ? "card--accent" : ""} ${className}`}
      style={{ ...style, ...(accent ? ({ "--card-accent": accent } as CSSProperties) : {}) }}
    >
      {(title || actions) && (
        <header className={`card__head ${tight ? "card__head--tight" : ""}`}>
          <div className="card__title">
            {icon && <Icon name={icon} size={15} />}
            <span className="truncate">{title}</span>
            {subtitle && <span className="card__sub">{subtitle}</span>}
          </div>
          {actions && <div className="card__actions">{actions}</div>}
        </header>
      )}
      {children !== undefined && (
        <div className={`card__body ${tight ? "card__body--tight" : ""} ${flush ? "card__body--flush" : ""}`}>
          {children}
        </div>
      )}
      {footer && <footer className="card__foot">{footer}</footer>}
    </section>
  );
}

/* ---------------- Button ---------------- */
type BtnVariant = "default" | "primary" | "ghost" | "danger" | "long" | "short" | "outline";

export function Button({
  children,
  variant = "default",
  size,
  icon,
  iconRight,
  block,
  active,
  title,
  disabled,
  onClick,
  type = "button",
  className = "",
  style,
}: {
  children?: ReactNode;
  variant?: BtnVariant;
  size?: "sm" | "lg";
  icon?: IconName;
  iconRight?: IconName;
  block?: boolean;
  active?: boolean;
  title?: string;
  disabled?: boolean;
  onClick?: () => void;
  type?: "button" | "submit";
  className?: string;
  style?: CSSProperties;
}) {
  const cls = [
    "btn",
    variant !== "default" ? `btn--${variant}` : "",
    size ? `btn--${size}` : "",
    block ? "btn--block" : "",
    !children ? "btn--icon" : "",
    className,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      type={type}
      className={cls}
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-pressed={active}
      style={{ ...style, ...(active ? { background: "var(--accent-soft)", borderColor: "var(--accent-line)", color: "var(--accent-text)" } : {}) }}
    >
      {icon && <Icon name={icon} size={size === "sm" ? 13 : 15} />}
      {children}
      {iconRight && <Icon name={iconRight} size={size === "sm" ? 13 : 15} />}
    </button>
  );
}

/* ---------------- Switch ---------------- */
export function Switch({
  checked,
  onChange,
  label,
  disabled,
  title,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: ReactNode;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <label className="switch" title={title}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(e.target.checked)}
      />
      <span className="switch__track">
        <span className="switch__thumb" />
      </span>
      {label && <span className="switch__text">{label}</span>}
    </label>
  );
}

/* ---------------- Checkbox ---------------- */
export function Checkbox({
  checked,
  onChange,
  label,
  disabled,
  title,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: ReactNode;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <label className="check" title={title}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="check__box">
        <Icon name="check" size={11} strokeWidth={3} />
      </span>
      {label && <span>{label}</span>}
    </label>
  );
}

/* ---------------- Field ---------------- */
export function Field({
  label,
  hint,
  children,
  htmlFor,
  warn,
}: {
  label?: ReactNode;
  hint?: ReactNode;
  children: ReactNode;
  htmlFor?: string;
  warn?: string | null;
}) {
  return (
    <div className="field">
      {label && (
        <label className="field__label" htmlFor={htmlFor}>
          {label}
        </label>
      )}
      {children}
      {hint && <span className="hint">{hint}</span>}
      {warn && (
        <span className="hint" style={{ color: "var(--warn-text)", display: "flex", gap: 5, alignItems: "flex-start" }}>
          <Icon name="alert" size={12} style={{ marginTop: 2, flex: "none" }} />
          {warn}
        </span>
      )}
    </div>
  );
}

/* ---------------- NumberInput ---------------- */
export function NumberInput({
  value,
  onChange,
  step = 1,
  min,
  max,
  unit,
  size,
  id,
  placeholder,
  zeroPuste,
  zeroLabel,
  style,
}: {
  value: number;
  onChange: (v: number) => void;
  step?: number;
  min?: number;
  max?: number;
  unit?: string;
  size?: "sm";
  id?: string;
  placeholder?: string;
  /**
   * Zero POKAZUJ JAKO PUSTE POLE (z podpowiedzia `placeholder`).
   *
   * Do pol, w ktorych `0` znaczy „bez limitu", a nie „limit zero". Widoczne
   * zero w takim polu czyta sie jako zakaz — a to jest dokladnie odwrotne
   * znaczenie i w tej klasie bledow potrafi wylaczyc handel.
   *
   * WOLIMY `zeroLabel`. Puste pole rozwiazuje tylko polowe problemu: nie
   * klamie, ale tez nic nie mowi — nie da sie odroznic „nie ustawilem" od
   * „swiadomie zdjalem limit". `zeroLabel` pisze to wprost.
   */
  zeroPuste?: boolean;
  /**
   * Zero POKAZUJ JAKO NAPIS (np. „BEZ LIMITU", „WYŁĄCZONE", „AUTOMAT").
   *
   * Implikuje `zeroPuste`: wpisana wartosc dalej jest liczba, tylko widok
   * zera nazywa rzecz po imieniu zamiast pokazywac cyfre albo pustke.
   * Wpisanie czegokolwiek kasuje napis i wraca do zwyklej edycji liczby.
   */
  zeroLabel?: string;
  style?: CSSProperties;
}) {
  const zerowe = zeroPuste || zeroLabel !== undefined;
  const pokaz = (v: number) => (zerowe && v === 0 ? "" : String(v));
  const [draft, setDraft] = useState(() => pokaz(value));
  const focused = useRef(false);
  // Napis zastepczy widac tylko wtedy, gdy pole NIE jest edytowane i naprawde
  // stoi w nim zero — inaczej zaslanialby to, co uzytkownik wlasnie wpisuje.
  const pokazNapis = zeroLabel !== undefined && value === 0 && draft.trim() === "";

  useEffect(() => {
    if (!focused.current) setDraft(pokaz(value));
    // `pokaz` jest czysta funkcja dwoch propsow — celowo nie jest zaleznoscia
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, zeroPuste, zeroLabel]);

  const commit = (raw: string) => {
    // Puste pole przy `zeroPuste`/`zeroLabel` to swiadome „bez limitu",
    // nie blad wpisu.
    if (zerowe && raw.trim() === "") {
      onChange(min !== undefined && min > 0 ? min : 0);
      setDraft("");
      return;
    }
    const v = Number(raw.replace(",", "."));
    if (Number.isFinite(v)) {
      let out = v;
      if (min !== undefined) out = Math.max(min, out);
      if (max !== undefined) out = Math.min(max, out);
      onChange(out);
      setDraft(pokaz(out));
    } else {
      setDraft(pokaz(value));
    }
  };

  return (
    <div className="input-affix" style={style} data-zero={pokazNapis ? "1" : undefined}>
      {pokazNapis && (
        <span className="input-affix__zero" aria-hidden="true">
          {zeroLabel}
        </span>
      )}
      <input
        id={id}
        className={`input input--num ${size === "sm" ? "input--sm" : ""}`}
        type="text"
        inputMode="decimal"
        value={draft}
        placeholder={pokazNapis ? undefined : placeholder}
        title={pokazNapis ? zeroLabel : undefined}
        onFocus={() => (focused.current = true)}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={(e) => {
          focused.current = false;
          commit(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "ArrowUp") {
            e.preventDefault();
            commit(String(value + step));
          }
          if (e.key === "ArrowDown") {
            e.preventDefault();
            commit(String(value - step));
          }
        }}
      />
      {unit && <span className="input-affix__suffix">{unit}</span>}
    </div>
  );
}

/* ---------------- TextInput ---------------- */
export function TextInput({
  value,
  onChange,
  placeholder,
  size,
  id,
  icon,
  type = "text",
  onKeyDown,
  autoFocus,
  style,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  size?: "sm";
  id?: string;
  icon?: IconName;
  type?: string;
  onKeyDown?: (e: React.KeyboardEvent) => void;
  autoFocus?: boolean;
  style?: CSSProperties;
}) {
  if (icon) {
    return (
      <div style={{ position: "relative", display: "flex", alignItems: "center", minWidth: 0, ...style }}>
        <Icon
          name={icon}
          size={14}
          style={{ position: "absolute", left: 9, color: "var(--text-faint)", pointerEvents: "none" }}
        />
        <input
          id={id}
          className={`input ${size === "sm" ? "input--sm" : ""}`}
          style={{ paddingLeft: 29 }}
          type={type}
          value={value}
          placeholder={placeholder}
          autoFocus={autoFocus}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
        />
      </div>
    );
  }
  return (
    <input
      id={id}
      className={`input ${size === "sm" ? "input--sm" : ""}`}
      style={style}
      type={type}
      value={value}
      placeholder={placeholder}
      autoFocus={autoFocus}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={onKeyDown}
    />
  );
}

/* ---------------- Select ---------------- */
/**
 * Opcja listy. `group` wrzuca ja do nazwanej sekcji (`<optgroup>`) — tak
 * prezentujemy presety rozdzielone po formacie sygnalow. Opcje BEZ `group`
 * ida na gore, poza sekcjami: tam mieszka wpis „— wybierz —" i „nie handluj",
 * ktore nie naleza do zadnego formatu.
 */
export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
  group?: string;
}

export function Select<T extends string>({
  value,
  onChange,
  options,
  size,
  id,
  style,
  disabled,
}: {
  value: T;
  onChange: (v: T) => void;
  options: SelectOption[];
  size?: "sm";
  id?: string;
  style?: CSSProperties;
  disabled?: boolean;
}) {
  const luzem = options.filter((o) => !o.group);
  // Kolejnosc sekcji = kolejnosc PIERWSZEGO wystapienia, a nie alfabetyczna:
  // wolajacy ustala, ktory format stoi wyzej, i ma to zostac tak, jak podal.
  const grupy: string[] = [];
  for (const o of options) if (o.group && !grupy.includes(o.group)) grupy.push(o.group);

  return (
    <select
      id={id}
      className={`select ${size === "sm" ? "select--sm" : ""}`}
      value={value}
      disabled={disabled}
      style={style}
      onChange={(e) => onChange(e.target.value as T)}
    >
      {luzem.map((o) => (
        <option key={o.value} value={o.value} disabled={o.disabled}>
          {o.label}
        </option>
      ))}
      {grupy.map((g) => (
        <optgroup key={g} label={g}>
          {options
            .filter((o) => o.group === g)
            .map((o) => (
              <option key={o.value} value={o.value} disabled={o.disabled}>
                {o.label}
              </option>
            ))}
        </optgroup>
      ))}
    </select>
  );
}

/* ---------------- MultiSelect ---------------- */

/**
 * Lista rozwijana z ZAZNACZANIEM WIELU pozycji.
 *
 * Zwykły `<select>` wymusza „albo jedno, albo wszystko" — a przy filtrowaniu
 * strumienia sygnałów naturalne pytanie brzmi „pokaż mi ZEN i Synergy, ale nie
 * resztę". Wariant `multiple` przeglądarki tego nie rozwiązuje: wymaga
 * trzymania Ctrl, gubi zaznaczenie przy kliknięciu obok i wygląda inaczej
 * w każdej przeglądarce.
 *
 * PUSTY ZBIÓR ZNACZY „WSZYSTKO", nie „nic". To jest jedyny sensowny stan
 * początkowy dla filtra: użytkownik, który jeszcze nic nie wybrał, chce
 * widzieć strumień, a nie pustą listę.
 */
export function MultiSelect({
  values,
  onChange,
  options,
  size,
  style,
  labelAll,
  labelSome,
}: {
  values: string[];
  onChange: (v: string[]) => void;
  options: SelectOption[];
  size?: "sm";
  style?: CSSProperties;
  /** napis, gdy nic nie zaznaczono (czyli: wszystko) */
  labelAll: string;
  /** napis przy N zaznaczonych — `{n}` zostanie podmienione */
  labelSome: string;
}) {
  const [otwarte, setOtwarte] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  
  const [poz, setPoz] = useState<{
    prawo: number;
    gora?: number;
    dol?: number;
    minW: number;
    maxH: number;
  } | null>(null);

  
  const przelicz = () => {
    const b = box.current?.getBoundingClientRect();
    if (!b) return;
    const LUZ = 8;
    const pod = window.innerHeight - b.bottom - LUZ;
    const nad = b.top - LUZ;
    /* `scrollHeight`, nie `getBoundingClientRect().height`: ta druga jest już
       przycięta przez `max-height`, więc porównanie zawsze wychodziłoby
       „mieści się" i lista nigdy by się nie odwróciła. */
    const trzeba = pop.current?.scrollHeight ?? 0;
    const doGory = trzeba > pod && nad > pod;
    setPoz({
      prawo: Math.max(4, window.innerWidth - b.right),
      ...(doGory ? { dol: window.innerHeight - b.top + 4 } : { gora: b.bottom + 4 }),
      minW: b.width,
      maxH: Math.max(120, Math.min(320, doGory ? nad : pod)),
    });
  };

  useLayoutEffect(() => {
    if (!otwarte) {
      setPoz(null);
      return;
    }
    przelicz();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [otwarte, options.length, values.length]);

  useEffect(() => {
    if (!otwarte) return;
    /* KLIKNIĘCIE „POZA" MUSI ZNAĆ OBIE CZĘŚCI. Po przeniesieniu listy do
       portalu nie jest ona już potomkiem `.msel`, więc sam test na `box`
       uznawałby kliknięcie w pozycję listy za kliknięcie na zewnątrz —
       filtr zamykałby się, zanim zdążyłby przełączyć zaznaczenie. */
    const poza = (e: MouseEvent) => {
      const c = e.target as Node;
      if (box.current?.contains(c) || pop.current?.contains(c)) return;
      setOtwarte(false);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOtwarte(false);
    };
    /* Przewijanie i zmiana rozmiaru przesuwają przycisk, a lista jest
       przypięta do EKRANU — bez przeliczenia odjechałaby od swojego
       przycisku. `capture`, bo przewija się kontener wewnętrzny, nie okno. */
    document.addEventListener("mousedown", poza);
    document.addEventListener("keydown", esc);
    window.addEventListener("scroll", przelicz, true);
    window.addEventListener("resize", przelicz);
    return () => {
      document.removeEventListener("mousedown", poza);
      document.removeEventListener("keydown", esc);
      window.removeEventListener("scroll", przelicz, true);
      window.removeEventListener("resize", przelicz);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [otwarte]);

  const przelacz = (v: string) =>
    onChange(values.includes(v) ? values.filter((x) => x !== v) : [...values, v]);

  const etykieta = values.length === 0 ? labelAll : labelSome.replace("{n}", String(values.length));

  const lista = (
    <div
      className="msel__pop"
      ref={pop}
      style={{
        right: poz?.prawo ?? 0,
        top: poz?.gora,
        bottom: poz?.dol,
        minWidth: poz?.minW,
        maxHeight: poz?.maxH ?? 320,
        visibility: poz ? "visible" : "hidden",
      }}
    >
      {values.length > 0 && (
        <button type="button" className="msel__clear" onClick={() => onChange([])}>
          {labelAll}
        </button>
      )}
      {options.map((o) => (
        <div key={o.value} className="msel__row">
          <Checkbox
            checked={values.includes(o.value)}
            onChange={() => przelacz(o.value)}
            label={o.label}
          />
        </div>
      ))}
    </div>
  );

  return (
    <div className="msel" ref={box} style={style}>
      <button
        type="button"
        className={`select msel__btn ${size === "sm" ? "select--sm" : ""}`}
        onClick={() => setOtwarte((x) => !x)}
        aria-expanded={otwarte}
      >
        <span className="truncate">{etykieta}</span>
        <Icon name="chevron-down" size={13} />
      </button>
      {otwarte && createPortal(lista, document.body)}
    </div>
  );
}

/* ---------------- Segmented ---------------- */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  size,
  style,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: ReactNode; bg?: string; fg?: string; title?: string }[];
  size?: "sm";
  style?: CSSProperties;
}) {
  return (
    <div className={`seg ${size === "sm" ? "seg--sm" : ""}`} style={style} role="tablist">
      {options.map((o) => (
        <button
          key={o.value}
          role="tab"
          aria-selected={value === o.value}
          className="seg__btn"
          data-active={value === o.value}
          title={o.title}
          style={value === o.value ? ({ "--seg-bg": o.bg, "--seg-fg": o.fg } as CSSProperties) : undefined}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/* ---------------- Badge ---------------- */
export function Badge({
  children,
  tone = "muted",
  dot,
  title,
}: {
  children: ReactNode;
  tone?: "muted" | "accent" | "long" | "short" | "warn" | "ai" | "info";
  dot?: boolean;
  title?: string;
}) {
  return (
    <span className={`badge badge--${tone} ${dot ? "badge--dot" : ""}`} title={title}>
      {children}
    </span>
  );
}

/* ---------------- Empty ---------------- */
export function Empty({ icon = "info", title, text }: { icon?: IconName; title: string; text?: string }) {
  return (
    <div className="empty">
      <div className="empty__icon">
        <Icon name={icon} size={19} />
      </div>
      <div className="empty__title">{title}</div>
      {text && <div className="empty__text">{text}</div>}
    </div>
  );
}

/* ---------------- Modal ---------------- */
export function Modal({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  width = 560,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return createPortal(
    <div className="modal-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" style={{ "--modal-w": `${width}px` } as CSSProperties} role="dialog" aria-modal="true">
        <header className="modal__head">
          <div style={{ minWidth: 0 }}>
            <div className="modal__title">{title}</div>
            {subtitle && <div className="hint" style={{ marginTop: 3 }}>{subtitle}</div>}
          </div>
          <Button variant="ghost" size="sm" icon="x" onClick={onClose} title={t("common.close")} />
        </header>
        <div className="modal__body">{children}</div>
        {footer && <footer className="modal__foot">{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}

/* ---------------- Tooltip ---------------- */
/** Szerokość dymka — ta sama liczba jest w `.tip__bubble` w `ui.css`. */

export function Tooltip({
  content,
  children,
  szeroki,
}: {
  content: ReactNode;
  children: ReactNode;
  /** Treść bogata (tabelka, rozbicie) — dymek dostaje szerszy limit. */
  szeroki?: boolean;
}) {
  // Dymek trafia do `document.body`, a nie obok kotwicy.
  //
  // Poprzednio był pozycjonowany absolutnie WEWNĄTRZ tabeli pozycji, a ta ma
  // `overflow:auto` (poziomy pasek przewijania) — krawędź kontenera po prostu
  // ucinała dymek w połowie zdania. Portal + `position:fixed` wyprowadza go
  // ponad wszystko, a `clamp` trzyma go w granicach okna.
  const [poz, setPoz] = useState<{ x: number; y: number; nad: boolean } | null>(null);
  const kotwica = useRef<HTMLSpanElement | null>(null);
  const id = useId();

  const pokaz = () => {
    const r = kotwica.current?.getBoundingClientRect();
    if (!r) return;
    // brak miejsca u góry → dymek ląduje POD kotwicą
    setPoz({ x: r.left + r.width / 2, y: r.top > 140 ? r.top - 8 : r.bottom + 8, nad: r.top > 140 });
  };
  const schowaj = () => setPoz(null);

  // `position:fixed` nie jedzie z przewijaniem, więc przy przewinięciu dymek
  // znika, zamiast wisieć w oderwaniu od swojego wiersza.
  useEffect(() => {
    if (!poz) return;
    const off = () => setPoz(null);
    window.addEventListener("scroll", off, true);
    window.addEventListener("resize", off);
    return () => {
      window.removeEventListener("scroll", off, true);
      window.removeEventListener("resize", off);
    };
  }, [poz]);

  /* POZYCJA USTAWIANA W TYM SAMYM ZATWIERDZENIU, CO PIERWSZE MALOWANIE.
     Wcześniej dymek renderował się na zgadniętej szerokości (stałe 280 px),
     a poprawka jechała przez `setState` — czyli DRUGIE renderowanie. Widać
     to było jako „otwiera się za małe, przeskakuje i dopiero wtedy jest
     normalne": użytkownik oglądał animację `popIn` odpaloną dwa razy.
     Ref-callback wykonuje się w trakcie zatwierdzania, ZANIM przeglądarka
     cokolwiek narysuje, więc korekta `left` trafia już do pierwszej klatki
     i nie ma ani przeskoku, ani powtórzonej animacji. */
  const margines = 8;
  const ustawBanke = (el: HTMLSpanElement | null) => {
    if (!el || !poz) return;
    // `offsetWidth`, nie `getBoundingClientRect()`: ten drugi zwraca box PO
    // transformacjach, więc w pierwszej klatce animacji (scale 0.96) dawał
    // szerokość mniejszą od prawdziwej i przycięcie do krawędzi wychodziło
    // za wąskie.
    const w = el.offsetWidth;
    const polowa = Math.min(w, window.innerWidth - 2 * margines) / 2;
    el.style.left = `${Math.min(
      Math.max(poz.x, polowa + margines),
      window.innerWidth - polowa - margines,
    )}px`;
  };

  return (
    <>
      <span
        ref={kotwica}
        className="tip"
        onMouseEnter={pokaz}
        onMouseLeave={schowaj}
        onFocus={pokaz}
        onBlur={schowaj}
        aria-describedby={poz ? id : undefined}
      >
        {children}
      </span>
      {poz &&
        createPortal(
          <span
            ref={ustawBanke}
            className={`tip__bubble${poz.nad ? "" : " tip__bubble--pod"}${szeroki ? " tip__bubble--szeroki" : ""}`}
            id={id}
            role="tooltip"
            style={{ left: poz.x, top: poz.y }}
          >
            {content}
          </span>,
          document.body,
        )}
    </>
  );
}

/* ---------------- InfoDot ---------------- */
export function InfoDot({ text }: { text: ReactNode }) {
  return (
    <Tooltip content={text}>
      <span
        tabIndex={0}
        style={{
          display: "grid",
          placeItems: "center",
          width: 14,
          height: 14,
          borderRadius: "50%",
          border: "1px solid var(--border-strong)",
          color: "var(--text-faint)",
          fontSize: 9,
          fontWeight: 700,
          cursor: "help",
          flex: "none",
        }}
      >
        ?
      </span>
    </Tooltip>
  );
}

export { Icon };
export type { IconName };
