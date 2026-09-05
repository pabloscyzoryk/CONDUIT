import type { SettingKey, Settings } from "@/types";
import { CURRENCIES } from "./defaultSettings";

/* ============================================================
   SCHEMAT USTAWIEN
   Zrodlo prawdy dla widoku "Ustawienia". Kazde pole ma etykiete,
   podpowiedz (przeniesiona z komentarzy bot.py) i kategorie, ktora
   decyduje o widocznosci w trybach MANUAL / AUTO / AI.

   category:
     "management" — konfiguracja ZARZADZANIA POZYCJA (tylko AUTO)
     "ai"         — wybor modelu AI (tylko AI)
     "general"    — panel, polaczenie, logi (widoczne w kazdym trybie)
   ============================================================ */

export type FieldType = "bool" | "num" | "text" | "select";

export interface FieldDef {
  key: SettingKey;
  label: string;
  hint?: string;
  type: FieldType;
  options?: { value: string; label: string }[];
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  /** pokazuj tylko gdy warunek spelniony (zwijanie zaleznych opcji) */
  when?: (s: Settings) => boolean;
  /** ostrzezenie pod polem, gdy warunek spelniony */
  warn?: (s: Settings) => string | null;
  wide?: boolean;
}



/**
 * Trzy znaczenia zera. Podzial idzie po TYM, CO POLE OBIECUJE, a nie po
 * slowie uzytym w podpowiedzi:
 *
 *  * `"bez-limitu"` — pole jest PULAPEM albo BRAMKA („nie wiecej niz N",
 *    „nie wchodz ponizej N"). Zero ZDEJMUJE granice, czyli pozwala
 *    NIESKONCZENIE WIELE. To jest znaczenie, ktore kosztowalo konto.
 *  * `"wylaczone"` — pole jest PROGIEM REGULY („zrob X, gdy…"). Zero znaczy,
 *    ze regula nie dziala i bot nie robi NIC PONAD to, co zwykle.
 *  * `"automat"` — zero znaczy „wez wartosc skadinad albo zachowaj sie jak
 *    dotad" (np. dzwignia z rachunku brokera, kredyt z terminala).
 */
export type ZnaczenieZera = "bez-limitu" | "wylaczone" | "automat";

/**
 * Pola, w ktorych „bez limitu" jest RYZYKOWNE — zdjecie pulapu zwieksza
 * ekspozycje, zamiast ja ograniczac. Panel dopisuje przy nich ostrzezenie.
 */
export const BRAK_LIMITU_RYZYKOWNY: SettingKey[] = [
  "reenter_max",
  "max_open_positions",
  "max_open_baskets",
];

/**
 * Znaczenie zera per pole. Pole nieobecne w tabeli = zero jest zwykla
 * liczba i nie dostaje zadnej odznaki (lepiej nic nie napisac, niz
 * napisac nieprawde).
 */
export const ZNACZENIE_ZERA: Partial<Record<SettingKey, ZnaczenieZera>> = {
  /* ---------- PULAPY I BRAMKI: zero = BRAK GRANICY ---------- */
  // Trzy pola z ostrzezeniem — patrz `BRAK_LIMITU_RYZYKOWNY`.
  reenter_max: "bez-limitu",
  max_open_positions: "bez-limitu",
  max_open_baskets: "bez-limitu",
  // pozostale pulapy ekspozycji i ryzyka
  max_directional_lots: "bez-limitu",
  risk_per_basket_pct: "bez-limitu",
  max_portfolio_risk_pct: "bez-limitu",
  entry_risk_budget: "bez-limitu",
  entry_tp1_budget: "bez-limitu",
  equity_floor_pct: "bez-limitu",
  max_dd_pct: "bez-limitu",
  max_dd_usd: "bez-limitu",
  lot_max: "bez-limitu",
  daily_signal_budget: "bez-limitu",
  rearm_max_times: "bez-limitu",
  basket_max_age_min: "bez-limitu",
  // bramki marginesu — zero znaczy „wchodz przy kazdym poziomie"
  ml_min_wejscie: "bez-limitu",
  ml_min_warstwa: "bez-limitu",
  ml_min_reentry: "bez-limitu",
  ml_min_rearm: "bez-limitu",
  ml_min_piramida: "bez-limitu",
  ml_min_fast_addon: "bez-limitu",
  ml_min_relot_up: "bez-limitu",
  ml_min_drabina: "bez-limitu",
  // bramki wieku, odleglosci i jakosci sygnalu
  signal_max_age_min: "bez-limitu",
  ignore_old_after_min: "bez-limitu",
  max_chase_beyond_zone: "bez-limitu",
  sl_max_dist: "bez-limitu",
  sl_min_dist_cap: "bez-limitu",
  signal_min_rr: "bez-limitu",
  signal_max_zone_width: "bez-limitu",
  spp_max_age_h: "bez-limitu",
  tp_signal_max_lag_s: "bez-limitu",
  pending_ttl_h: "bez-limitu",
  pending_drop_grace_max_dist: "bez-limitu",
  hold_after_tp_hit_min: "wylaczone",
  riskfree_runner_max_hold_min: "bez-limitu",
  exit_spread_mult: "wylaczone",
  pending_relot_up_od_salda: "bez-limitu",
  journal_retention_days: "bez-limitu",
  archive_retention_days: "bez-limitu",
  mt5_restart_after: "bez-limitu",

  /* ---------- REGULY: zero = REGULA NIE DZIALA ---------- */
  sl_min_dist: "wylaczone",
  trail_min_dist: "wylaczone",
  ladder_from_tp: "wylaczone",
  oae_timeout_min: "wylaczone",
  sl_polowa_od_konca: "wylaczone",
  // sl_hit_verify_tol: zero is strict tolerance, NOT disabled verification.
  bank_all_at_stage: "wylaczone",
  market_unfilled_cancel_stage: "wylaczone",
  stale_take_min: "wylaczone",
  stale_take_min2: "wylaczone",
  rev_exit_range: "wylaczone",
  smart_exit_take: "wylaczone",
  smart_exit_giveback: "wylaczone",
  smart_exit_drop_speed: "wylaczone",
  smart_exit_hold_if_pending: "wylaczone",
  vol_window_min: "wylaczone",
  alert_dd_pct: "wylaczone",
  dd_soft_pct: "wylaczone",
  dd_hard_pct: "wylaczone",
  day_target_usd: "wylaczone",
  day_trail_stop_usd: "wylaczone",
  eod_flat_hour: "wylaczone",
  streak_pause_n: "wylaczone",
  slhit_pause_n: "wylaczone",
  puls_h: "wylaczone",
  exit_r_multiple: "wylaczone",
  exit_round_dist: "wylaczone",
  basket_target_usd: "wylaczone",
  riskfree_trigger_usd: "wylaczone",
  riskfree_trigger_r: "wylaczone",
  sl_min_dist_zone_mult: "wylaczone",
  sl_min_dist_atr_mult: "wylaczone",
  entry_units_zone_ref: "wylaczone",
  trend_filter_drop_pct: "wylaczone",
  sltp_retry_s: "wylaczone",

  /* ---------- ZASTEPSTWO: zero = wartosc skadinad / „jak dotad" ---------- */
  konto_dzwignia: "automat",
  kredyt_reczny: "automat",
  lot_scale_step: "automat",
  entry_units_limit: "automat",
  market_entry_units: "automat",
  market_hybrid_now_units: "automat",
  market_hybrid_pending_units: "automat",
  market_hybrid_max_chase_usd: "automat",
  market_hybrid_tp_stage: "automat",
  vsl_eval_s: "automat",
  slhit_pause_min: "automat",
  slhit_pause_lot_mult: "automat",
  pending_drop_grace_min: "automat",
  pending_drop_keep_n: "automat",
  stat_be_prog_usd: "automat",
  poll_ms: "automat",
};


export type ZakresUstawien = "rachunek" | "preset";

export const ZAKRES_LABEL: Record<ZakresUstawien, string> = {
  rachunek: "Rachunek — wspólne dla całego bota",
  preset: "Handel formatem — należy do presetu",
};

export const ZAKRES_HINT: Record<ZakresUstawien, string> = {
  rachunek:
    "Te ustawienia opisują rachunek i maszynę: dźwignię, koszty brokera, opóźnienie wykonania, terminal MT5, pocztę i dziennik. Wczytanie presetu ICH NIE RUSZA i są takie same dla każdego formatu sygnałów.",
  preset:
    "Te ustawienia opisują SPOSÓB HANDLU i należą do presetu — wczytanie innego presetu je przestawi. Każdy format sygnałów ma własny preset, więc te same pola mogą mieć różne wartości dla ATFX i dla Synergy. Sufit obowiązujący wszystkie presety naraz ustawia się w panelu łańcuchów.",
};

export interface GroupDef {
  id: string;
  title: string;
  icon: string;
  desc: string;
  category: "management" | "ai" | "general";
  /** Czy to ustawienie RACHUNKU (wspólne), czy HANDLU FORMATEM (per preset). */
  zakres: ZakresUstawien;
  accent?: string;
  fields: FieldDef[];
}

/**
 * Ostrzezenie przy polach z `BRAK_LIMITU_RYZYKOWNY`.
 *
 * Nie powtarza slowa „bez limitu" — mowi, ILE to znaczy i CO sie stalo, gdy
 * ktos przeczytal to zero jako ostroznosc. Odznaka w polu mowi „BEZ LIMITU",
 * a to zdanie tlumaczy, ze to jest ZDJETY hamulec, nie zaciagniety.
 */
const brakLimituOstrzezenie = (n: number, tresc: string) => (n === 0 ? tresc : null);

const gapTrap = (s: Settings) =>
  s.trail_mode === "gap" && s.runner_trail && s.runner_trail_gap >= s.runner_trail_start
    ? "Luka ≥ próg aktywacji: SL wyląduje na BE i oddasz CAŁY zysk (realny przypadek: próg 10, luka 12 → +$11 poszło na zero)."
    : null;

export const SETTINGS_SCHEMA: GroupDef[] = [
  /* ================= WEJSCIA ================= */
  {
    id: "entry",
    title: "Wejścia i strefa",
    icon: "target",
    desc: "Jak bot interpretuje strefę wejścia z sygnału i kiedy w ogóle wchodzi.",
    category: "management",
    zakres: "preset",
    accent: "var(--accent)",
    fields: [
      {
        key: "auto_limit",
        label: "AUTO LIMIT",
        hint:
          "Sygnał BEZ słowa LIMIT rozstaw jako siatkę zleceń oczekujących w strefie, zamiast wchodzić po rynku. " +
          "Dotyczy KAŻDEGO sygnału rynkowego, nie tylko takiego, którego cena uciekła poza strefę. " +
          "Wyłączenie: cała siatka (poziomy × jednostki) otwiera się w jednym ticku po jednej cenie — " +
          "limit ryzyka koszyka nie może wtedy ograniczyć każdej warstwy osobno. Zostaw włączone.",
        type: "bool",
      },
      {
        key: "only_limit_signals",
        label: "TYLKO SYGNAŁY LIMIT",
        hint: "Ignoruj sygnały rynkowe BUY/SELL — handluj wyłącznie limitami (najlepsza rodzina w sweepach).",
        type: "bool",
      },
      {
        key: "valid_till_tp2",
        label: "VALID TILL TP2",
        hint: "Pendingi i wejścia żyją do TP2 zamiast do TP1.",
        type: "bool",
      },
      {
        key: "custom_entry",
        label: "CUSTOM ENTRY LEVEL",
        hint: "Własne przesunięcia granic strefy (zamiast FIRST ENTRY ANY LEVEL).",
        type: "bool",
      },
      { key: "entry_high_offset", label: "górna granica ±", type: "num", step: 0.1, unit: "$", when: (s) => s.custom_entry },
      { key: "entry_low_offset", label: "dolna granica ±", type: "num", step: 0.1, unit: "$", when: (s) => s.custom_entry },
      {
        key: "entry_offset_dir",
        label: "OFFSETY WG KIERUNKU (poprawka SELL)",
        hint: "Bez tego strefa liczy się czysto cenowo i dla SELL role są odwrócone. Włączone: BUY lepiej = niżej, SELL lepiej = WYŻEJ.",
        type: "bool",
      },
      {
        key: "entry_deep_offset",
        label: "głębokość (w stronę lepszych wejść)",
        type: "num",
        step: 0.1,
        unit: "$",
        when: (s) => s.entry_offset_dir,
      },
      {
        key: "entry_deep_frac_to_sl",
        label: "głębokość jako UŁAMEK dystansu do SL",
        hint: "0 używa stałej kwoty. Wartość dodatnia liczy głębokość jako ułamek dystansu krawędź→SL; wartość ≥ 1 może umieścić szczebel na poziomie stopu.",
        type: "num",
        min: 0,
        max: 1.5,
        step: 0.05,
        when: (s) => s.entry_offset_dir,
      },
      {
        key: "entry_tol_offset",
        label: "tolerancja (poza krawędź)",
        hint: "Opcjonalne przesunięcie pierwszego wejścia, wyrażone w cenie instrumentu.",
        type: "num",
        step: 0.1,
        unit: "$",
        when: (s) => s.entry_offset_dir,
      },
      {
        key: "sl_dist_limit",
        label: "LIMIT ODLEGŁOŚCI OD SL",
        hint: "Nie otwieraj wejścia oddalonego od SL o więcej niż X. UWAGA: w backteście ta opcja NIE pomogła — przy X ≤ 6 tnie zysk o połowę.",
        type: "bool",
      },
      { key: "sl_dist_max", label: "maks. dystans do SL", type: "num", step: 0.1, unit: "$", when: (s) => s.sl_dist_limit },
      {
        key: "sl_max_dist",
        label: "MAKS. SZEROKOŚĆ SL",
        hint: "Przytnij SL koszyka do tej odległości od środka strefy. 0 = bierz SL sygnału bez zmian.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
      },
      {
        key: "skip_if_sl_breached",
        label: "POMIŃ SYGNAŁ Z PRZEBITYM SL",
        hint: "Odrzuca setup, jeśli w chwili odbioru sygnału cena już przebiła stop-loss; zapobiega to wykorzystaniu nieosiągalnego wejścia.",
        type: "bool",
      },
      {
        key: "max_chase_beyond_zone",
        label: "NIE GOŃ SETUPU DALEJ NIŻ",
        hint: "Maksymalna odległość ceny od lepszej krawędzi strefy, przy której jeszcze wchodzimy po rynku. 0 = bez ograniczenia.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
      },
      {
        key: "side_filter",
        label: "KIERUNEK SYGNAŁÓW",
        hint: "Opcjonalnie dopuszcza tylko wybrane kierunki. Wpływ należy zmierzyć na własnym zbiorze danych.",
        type: "select",
        options: [
          { value: "both", label: "oba kierunki" },
          { value: "buy", label: "tylko BUY" },
          { value: "sell", label: "tylko SELL" },
        ],
      },
      {
        key: "sl_min_dist",
        label: "MIN. SZEROKOŚĆ SL",
        hint: "Rozszerz SL koszyka do co najmniej tej odległości od środka strefy. Wąski SL może wyrzucać pozycje przed odwróceniem ceny; wpływ trzeba mierzyć na własnych danych. 0 = OFF.",
        type: "num",
        step: 0.5,
        unit: "$",
      },
      {
        key: "ignore_old_after_min",
        label: "IGNORUJ STARE SYGNAŁY PO",
        hint: "Czekający sygnał (bez otwartej pozycji) starszy niż X min jest anulowany. 0 = wyłączone.",
        type: "num",
        step: 1,
        unit: "min",
      },
    ],
  },

  /* ================= SIATKA ================= */
  {
    id: "grid",
    title: "Jednostki i siatka pozycji",
    icon: "grid",
    desc: "Ile pozycji i w jakim rozstawie bot rozkłada w strefie wejścia.",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "entry_units",
        label: "JEDNOSTKI WEJŚCIA na poziom",
        hint: "Struktura champion MEGA: N pozycji po minimalnym locie na każdym poziomie wejścia.",
        type: "num",
        min: 1,
        step: 1,
      },
      {
        key: "entry_units_limit",
        label: "jednostki dla koszyków LIMIT",
        hint: "Osobny sizing dla koszyków limitowych. 0 = jak wyżej.",
        type: "num",
        min: 0,
        step: 1,
      },
      {
        key: "ppm_enabled",
        label: "PLACING POSITION MULTIPLIER",
        hint: "Zagęszcza pozycje: krok ceny = 1/PPM (PPM 2 → co 0.50, PPM 10 → co 0.10).",
        type: "bool",
      },
      { key: "ppm", label: "PPM", type: "num", min: 0.1, step: 0.1, when: (s) => s.ppm_enabled },
      { key: "ppm_immediate", label: "PPM dla wejść rynkowych", type: "bool", when: (s) => s.ppm_enabled },
      { key: "ppm_for_limits", label: "PPM dla pendingów", type: "bool", when: (s) => s.ppm_enabled },
      {
        key: "entry_weights",
        label: "WAGI WOLUMENU WG GŁĘBOKOŚCI",
        hint: 'Rozkład wolumenu w strefie, od najpłytszego wejścia do najgłębszego — np. „1,2,4". Strefa ma medianę 5 $: przy płytkiej krawędzi R:R wynosi 0,50 (SL 6 $, TP1 3 $), przy głębokiej 8,0 (SL 1 $, TP1 8 $). Równe wolumeny oddają decyzję najgorszemu wejściu. Wagi tylko PRZEWAŻAJĄ koszyk — jego łączna wielkość zostaje bez zmian. Puste = drabinka równa.',
        type: "text",
        wide: true,
      },
      {
        key: "risk_per_basket_pct",
        label: "LIMIT RYZYKA KOSZYKA",
        hint: "Twardy sufit na sumę |wejście − SL| × 100 × wolumen po wszystkich zleceniach koszyka, w % kapitału. Koszyk ma jeden wspólny SL, więc albo wychodzi na celach, albo ginie w całości. Po przekroczeniu silnik najpierw skaluje wolumeny, potem odrzuca najpłytsze poziomy, na końcu cały koszyk. 0 = bez limitu.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "%",
        warn: (s) =>
          s.risk_per_basket_pct > 10
            ? "Ponad 10% kapitału na jeden koszyk. Trzy takie koszyki naraz to już blisko połowa konta."
            : null,
      },
      {
        key: "market_entry_step",
        label: "KROK WEJŚĆ RYNKOWYCH",
        hint: "O ile cena musi się przesunąć na naszą korzyść, zanim bot dołoży kolejną pozycję rynkową lub powtórne wejście. Bez tego kroku bot dokładałby na każdym ticku w strefie.",
        type: "num",
        min: 0.1,
        step: 0.1,
        unit: "$",
      },
      {
        key: "pending_ttl_from_basket",
        label: "TTL liczony od powstania koszyka",
        hint: 'Reguła brzmi „stary setup wypełnia się dopiero w krachu", a przestawienie siatki nie odmładza setupu. Wyłączone = wiek liczony od wystawienia pojedynczego zlecenia.',
        type: "bool",
        when: (s) => s.pending_ttl_h > 0,
      },
      {
        key: "entry_risk_budget",
        label: "BUDŻET RYZYKA NA POZIOM",
        hint: "Liczba jednostek = clamp(budżet / dystans do SL, 1, jednostki). Szeroki SL dostaje mniej jednostek. 0 = OFF.",
        type: "num",
        step: 1,
        unit: "$",
      },
      {
        key: "entry_tp1_budget",
        label: "SIZING Z GEOMETRII SYGNAŁU (TP1)",
        hint: "Jednostki = clamp(budżet / |TP1 − cena poziomu|, 1, jednostki). Bliżej TP1 = więcej jednostek. 0 = OFF.",
        type: "num",
        step: 1,
        unit: "$",
      },
      {
        key: "entry_touch_units",
        label: "TOUCHER — jednostki na szczycie strefy",
        hint: "Dodatkowe jednostki na krawędzi strefy (BUY: góra) z własnym, wczesnym TP — łapią sygnały, które ledwo musnęły strefę i odjechały.",
        type: "num",
        min: 0,
        step: 1,
      },
      {
        key: "entry_touch_tp",
        label: "TP touchera",
        hint: "1 = TP1, 2 = TP2…",
        type: "num",
        min: 1,
        max: 5,
        step: 1,
        when: (s) => s.entry_touch_units > 0 || !!s.entry_touch_levels,
      },
      {
        key: "entry_touch_levels",
        label: "PASMA GŁĘBOKOŚCI (off:units:tp)",
        hint: 'Lista poziomów toucherowych, offset w PIPSACH od szczytu strefy w dół. Champion: „0:4:2,7:4:2". Nadpisuje pole wyżej.',
        type: "text",
        wide: true,
      },
      {
        key: "pending_resize_on_vol",
        label: "PENDING-RESIZE wg zmienności",
        hint: "Przeliczaj liczbę jednostek niezafillowanych pendingów co X sekund — parytet z silnikiem (vol@fill zamiast vol@placement).",
        type: "bool",
      },
      { key: "pending_resize_sec", label: "co ile sekund", type: "num", step: 5, unit: "s", when: (s) => s.pending_resize_on_vol },
      {
        key: "pending_relot_on_balance",
        label: "PENDING-RELOT wg salda",
        hint: "Aktualizuj WOLUMEN leżących limitów, gdy zmieni się lot bazowy. W górę odzyskuje zysk (siatka z 200 $ nie wypełnia się lotem 0,01 przy koncie 500 $), w DÓŁ chroni kapitał (zlecenie z 5000 $ nie otwiera ogromnej pozycji, gdy konto spadło do 200 $). Przelicza się z tym samym okresem co PENDING-RESIZE.",
        type: "bool",
      },
      {
        key: "pending_relot_topup",
        label: "…metodą dokładki",
        hint: "WŁĄCZONE: dołóż osobne zlecenie na samą różnicę — pierwotny szczebel nie znika z rynku i nie traci miejsca w kolejce. WYŁĄCZONE: anuluj i złóż od nowa (szczebel na moment znika i może zostać odrzucony).",
        type: "bool",
        when: (s) => s.pending_relot_on_balance,
      },
      {
        key: "pending_relot_up",
        label: "…w GÓRĘ (dokładaj)",
        hint: "Strona ZYSKU: gdy szczebel jest mniejszy niż powinien, dołóż wolumen. Wyłączenie daje wariant tylko-redukcyjny (HYPER-X1A), nastawiony na mniejsze ryzyko zamiast na większy wzrost.",
        type: "bool",
        when: (s) => s.pending_relot_on_balance,
      },
      {
        key: "pending_relot_down",
        label: "…w DÓŁ (zmniejszaj)",
        hint: "Strona RYZYKA: gdy szczebel jest większy, niż na to stać konto, zdejmij nadmiar. Kasowane są najpierw dokładki, dopiero potem ruszana jest baza.",
        type: "bool",
        when: (s) => s.pending_relot_on_balance,
      },
      {
        key: "pending_relot_up_od_salda",
        label: "…dokładaj dopiero od salda",
        hint: "Kierunek w GÓRĘ włącza się dopiero, gdy saldo sięgnie tej kwoty; poniżej działa sama redukcja. 0 = bez progu.",
        type: "num",
        step: 100,
        unit: "$",
        when: (s) => s.pending_relot_on_balance && s.pending_relot_up,
      },
      {
        key: "pending_relot_wg_planu",
        label: "…cel wg PLANU, nie gołego lota",
        hint: "WŁĄCZONE: celem szczebla jest wolumen z planu siatki przeliczonego na bieżące saldo — z wagami RR i z limitem ryzyka koszyka. WYŁĄCZONE (jak dotąd): cel to liczba sztuk × goły lot bazowy, co spłaszcza drabinkę i omija dławik ryzyka.",
        type: "bool",
        warn: (s) => s.pending_relot_reconcile_target ? "Ten wybór jest zastąpiony: nowy kontrakt relotu zawsze używa pełnego sprawdzonego planu. Zapisana wartość legacy pozostaje zachowana na powrót do OFF." : null,
        when: (s) => s.pending_relot_on_balance,
      },
      {
        key: "pending_relot_reconcile_target",
        label: "RELOT — UZGODNIJ ŁĄCZNY CEL SZCZEBLA",
        hint: "Nowy kontrakt używa pełnego sprawdzonego planu i łącznego wolumenu szczebla, zamiast traktować każdą dokładkę jak niezależny cel. Zastępuje pending_relot_wg_planu, ale nie wyłącza bramek relotu: głównego włącznika, kierunku góra/dół, progów ani marginesu. OFF zachowuje wcześniejszy mechanizm. To ustawienie wybranego presetu, nie rachunku.",
        type: "bool",
        when: (s) => s.pending_relot_on_balance || s.pending_relot_reconcile_target,
      },
      {
        key: "pending_ttl_h",
        label: "TTL PENDINGÓW",
        hint: "Kasuj pendingi koszyków starszych niż X h (stare limity wypełniają się w krachach). 0 = OFF.",
        type: "num",
        step: 1,
        unit: "h",
      },
      {
        key: "pending_never_cancel",
        label: "PENDINGI BEZ WYGASANIA",
        hint: "Limity nigdy nie kasowane po TP (grupa często wypełnia limity godzinami).",
        type: "bool",
      },
      
      {
        key: "entry_allowance_usd",
        label: "WARSTWA ALLOWANCE PRZED STREFĄ",
        hint: "Ile jednostek ceny przed strefą wolno wejść: przy BUY nad górną krawędzią, przy SELL pod dolną. Ta oś zwiększa ekspozycję; 0 ją wyłącza.",
        type: "num",
        min: 0,
        max: 5,
        step: 0.5,
        unit: "$",
      },
      {
        key: "entry_allowance_units",
        label: "jednostek na warstwie allowance",
        hint: "Dodatkowe zlecenia ponad podstawową siatkę. 0 jednostek lub 0 kwoty wyłącza tę warstwę.",
        type: "num",
        min: 0,
        max: 10,
        step: 1,
        when: (s) => Number(s.entry_allowance_usd) > 0,
      },
      {
        key: "entry_depth_curve",
        label: "krzywa głębokości wejścia",
        hint: "1,0 = bez nagięcia. Głębsze wejścia zmieniają cenę wykonania, a nie jakość informacji w sygnale; nie należy podwójnie liczyć tej samej przewagi. Domyślną wartością pozostaje 1,0.",
        type: "num",
        min: 0.1,
        max: 4,
        step: 0.1,
      },
    ],
  },

  /* ================= TP ================= */
  {
    id: "targets",
    title: "Zarządzanie celami (TP)",
    icon: "flag",
    desc: "Kto dostaje który TP, kiedy bot uznaje cel za trafiony i co wtedy inkasuje.",
    category: "management",
    zakres: "preset",
    accent: "var(--long)",
    fields: [
      {
        key: "all_runners",
        label: "ALL TPS ARE RUNNERS",
        hint: "Każda pozycja dostaje najkorzystniejszy TP + trailing SL po hitach. Wyklucza się ze SCALE-OUT.",
        type: "bool",
      },
      {
        key: "scale_out",
        label: "SCALE-OUT",
        hint: "Zamyka % pozycji na KAŻDYM kolejnym TP (kaskada). Wyklucza się z ALL RUNNERS.",
        type: "bool",
      },
      { key: "scale_out_pct", label: "% pozycji na każdy TP", type: "num", min: 1, max: 100, step: 5, unit: "%", when: (s) => s.scale_out },
      {
        key: "scale_out_round",
        label: "zaokrąglanie liczby pozycji",
        type: "select",
        options: [
          { value: "up", label: "w górę (30% z 13 → 4)" },
          { value: "down", label: "w dół (30% z 13 → 3)" },
        ],
        when: (s) => s.scale_out,
      },
      {
        key: "scale_out_from",
        label: "licz % od",
        type: "select",
        options: [
          { value: "worst", label: "najgorszych — zamykają się pierwsze" },
          { value: "best", label: "najlepszych — zamykają się pierwsze" },
        ],
        when: (s) => s.scale_out,
      },
      {
        key: "scale_out_last_runner",
        label: "ostatnia pozycja (LAST RUNNER)",
        type: "select",
        options: [
          { value: "runner", label: "runner — najdalszy cel drabinki" },
          { value: "next_tp", label: "kolejny TP — dostaje najbliższy nietrafiony cel" },
          { value: "no_tp", label: "bez celu — prowadzona wyłącznie trailingiem" },
        ],
      },
      {
        key: "tp_open_offset",
        label: "TP OPEN — krok kolejnych celów",
        hint: "O ile punktów odsuwane są TP OPEN i kolejne generowane cele.",
        type: "num",
        min: 0.1,
        step: 0.5,
        unit: "$",
      },
      {
        key: "tp_source",
        label: "SKĄD WIEMY O TRAFIENIU CELU",
        hint: "Źródłem potwierdzenia może być cena, komunikat lub ich połączenie. Tryb z ceną ogranicza skutki opóźnionych albo przedwczesnych komunikatów.",
        type: "select",
        options: [
          { value: "Either", label: "co przyjdzie pierwsze (cena lub kanał)" },
          { value: "PriceOnly", label: "wyłącznie cena z MT5" },
          { value: "SignalOnly", label: "wyłącznie komunikat z kanału" },
          { value: "SignalConfirmedByPrice", label: "komunikat potwierdzony ceną" },
          { value: "PriceFirstSignalWindow", label: "cena decyduje, komunikat w oknie czasu" },
        ],
      },
      {
        key: "tp_price_tolerance",
        label: "tolerancja potwierdzenia ceną",
        hint: 'Broker sygnalisty ma inny BID niż nasz — cel „prawie" trafiony u jednego jest trafiony u drugiego.',
        type: "num",
        min: 0,
        step: 0.05,
        unit: "$",
        when: (s) => s.tp_source === "SignalConfirmedByPrice" || s.tp_source === "PriceFirstSignalWindow",
      },
      {
        key: "tp_price_front_run_usd",
        label: "AUTONOMICZNY FRONT-RUN TP Z CENY MT5",
        hint: "0 = wyłączone: pełne dotknięcie celu jak dotąd. Wartość > 0 zalicza kolejny TP tyle USD przed jego poziomem, wyłącznie z bieżącego Bid/Ask MT5 i bez wiadomości Telegram. Działa tylko dla koszyka z pozycją; nie skraca życia oczekującej siatki.",
        type: "num",
        min: 0,
        step: 0.05,
        unit: "$",
      },
      {
        key: "tp_signal_max_lead_s",
        label: "ile sekund PRZED dotknięciem ceny",
        hint: "Jak bardzo komunikat może wyprzedzić rynek. 0 = wymagaj potwierdzenia ceną.",
        type: "num",
        min: 0,
        step: 5,
        unit: "s",
        when: (s) => s.tp_source === "PriceFirstSignalWindow",
      },
      {
        key: "tp_signal_max_lag_s",
        label: "ile sekund PO fakcie",
        hint: "Po tym czasie spóźniony komunikat przestaje być aktualny. 0 = bez limitu.",
        type: "num",
        min: 0,
        step: 30,
        unit: "s",
        when: (s) => s.tp_source === "PriceFirstSignalWindow",
      },
      {
        key: "tp_stage_from_broker_fill",
        label: "ZALICZAJ CEL Z REALIZACJI U BROKERA",
        hint: "Broker zamknął pozycję na jej take-proficie — to najtwardszy dowód trafienia celu, niezależny i od naszego odczytu ceny, i od tego, czy sygnalista zdążył napisać.",
        type: "bool",
      },
      {
        key: "tp_detect_price",
        label: "wykrywaj TP z CENY (MT5)",
        hint: "Skrót do wyboru źródła TP powyżej. Kliknięcie zapisuje razem oba przełączniki i tryb. Wyłączenie obu nie istnieje: pozostaje PriceOnly. Osobna opcja realizacji TP u brokera nie jest tym przełącznikiem.",
        type: "bool",
      },
      { key: "tp_detect_signal", label: "wykrywaj TP z WIADOMOŚCI", hint: "Skrót do wyboru źródła TP. Kliknięcie zmienia tryb na prosty: cena, wiadomość albo pierwsze z obu. Nie wyłącza pozostałych poleceń zarządzania z Telegrama.", type: "bool" },
      {
        key: "tp_freeze_after_ladder",
        label: "ZAMROŻENIE TP PO DRABINCE",
        hint: "Po wyczerpaniu realnej drabinki TP runnerów zostaje na ostatnim realnym celu (bez ekstrapolacji, która potrafi uciekać przed ceną).",
        type: "bool",
      },
      {
        key: "tp_hit_fill_stages",
        label: "DOMYKAJ POMINIĘTE ETAPY",
        hint: '„TP3 HIT" przy etapie 0 wykonuje też plany TP1 i TP2 zamiast je gubić.',
        type: "bool",
      },
      {
        key: "spp_max_age_h",
        label: "GUARD SPP-REARM",
        hint: "Przezbrojenie koszyka nową drabinką celów dotyczy tylko koszyków młodszych niż X h. 0 = bez guardu.",
        type: "num",
        step: 1,
        unit: "h",
      },
    ],
  },

  /* ================= TRAILING ================= */
  {
    id: "trailing",
    title: "Trailing SL",
    icon: "trend",
    desc: "Jak stop-loss podąża za zyskiem — najważniejsza oś zysku w backtestach.",
    category: "management",
    zakres: "preset",
    accent: "var(--warn)",
    fields: [
      {
        key: "runner_trail",
        label: "SAFETY TRAILING STOP",
        hint: "Gwarancja zysku 24/7: pozycja w dużym zysku dostaje SL trailowany za ceną — nie odda zysku do BE i zamknie się na plusie bez Ciebie. UWAGA: ten przełącznik rządzi TYLKO trailingiem podstawowym. Rodzina „trailing rozdzielony (bank + runner)” niżej jest OSOBNA i działa dalej, nawet gdy to jest wyłączone.",
        type: "bool",
        warn: (s) =>
          !s.runner_trail && s.trail_split
            ? "Trailing podstawowy jest wyłączony, ale niezależny trailing rozdzielony nadal działa dla runnerów."
            : null,
      },
      {
        key: "runner_trail_start",
        label: "aktywuj przy zysku ≥",
        type: "num",
        min: 0,
        step: 1,
        unit: "pkt",
        when: (s) => s.runner_trail,
      },
      {
        key: "trail_mode",
        label: "TRYB TRAILINGU",
        type: "select",
        options: [
          { value: "gap", label: "gap — SL o lukę za ceną (klasyczny)" },
          { value: "lock_pct", label: "lock_pct — blokuj % szczytu (zalecany)" },
          { value: "tiered", label: "tiered — drabinka progów" },
        ],
        when: (s) => s.runner_trail,
      },
      {
        key: "runner_trail_gap",
        label: "luka SL za ceną",
        type: "num",
        min: 0.1,
        step: 1,
        unit: "pkt",
        when: (s) => s.runner_trail && s.trail_mode === "gap",
        warn: gapTrap,
      },
      {
        key: "trail_lock_pct",
        label: "blokuj % szczytu",
        hint: "95 = oddajesz tylko 5% najlepszego zysku pozycji. Sprawdzone na 3 oknach: 91% dni na plusie.",
        type: "num",
        min: 1,
        max: 99,
        step: 5,
        unit: "%",
        when: (s) => s.runner_trail && s.trail_mode === "lock_pct",
      },
      {
        key: "trail_tiers",
        label: 'drabinka „zysk:blokada”',
        hint: "Np. 10:5 = przy szczycie +10 zablokuj +5.",
        type: "text",
        wide: true,
        when: (s) => s.runner_trail && s.trail_mode === "tiered",
      },
      {
        key: "trail_split",
        label: "TRAILING ROZDZIELONY (bank + runner)",
        hint: "N najlepiej położonych pozycji dostaje luźniejszy trailing, a pozostałe ciaśniejszy. Ta rodzina jest niezależna od podstawowego safety trailing.",
        type: "bool",
        warn: (s) =>
          s.trail_split && !s.runner_trail
            ? "Działa MIMO wyłączonego trailingu podstawowego. Jeśli chcesz wyłączyć trailing całkowicie, wyłącz TAKŻE to pole."
            : null,
      },
      {
        key: "trail_runners_by_depth",
        label: "RUNNERY WG GŁĘBOKOŚCI WEJŚCIA",
        hint: "Bez tego „ile pozycji jako runner” jest MARTWE: n=1 i n=3 dawały wynik identyczny do szóstego miejsca po przecinku, bo „runner” znaczyło po prostu „pozycja bez TP”. Dopiero to wybiera runnerów wg tego, jak głęboko weszli.",
        type: "bool",
        when: (s) => s.trail_split,
      },
      {
        key: "trail_runners_n",
        label: "ile pozycji jako runner",
        hint: "Działa DOPIERO przy włączonym „runnery wg głębokości wejścia” — inaczej wartość nie zmienia niczego.",
        type: "num",
        min: 1,
        step: 1,
        when: (s) => s.trail_split,
      },
      {
        key: "trail_runner_mode",
        label: "tryb runnera",
        type: "select",
        options: [
          { value: "gap", label: "gap" },
          { value: "lock_pct", label: "lock_pct" },
          { value: "tiered", label: "tiered" },
        ],
        when: (s) => s.trail_split,
      },
      { key: "trail_runner_start", label: "próg runnera", type: "num", step: 1, unit: "pkt", when: (s) => s.trail_split },
      {
        key: "trail_runner_gap",
        label: "luka runnera",
        type: "num",
        step: 1,
        unit: "pkt",
        when: (s) => s.trail_split && s.trail_runner_mode === "gap",
      },
      {
        key: "trail_runner_lock_pct",
        label: "% szczytu runnera",
        type: "num",
        min: 1,
        max: 99,
        step: 5,
        unit: "%",
        when: (s) => s.trail_split && s.trail_runner_mode === "lock_pct",
      },
      {
        key: "trail_runner_tiers",
        label: "drabinka runnera",
        hint: "Luźna na początku (daje miejsce na ruch), ciasna gdy zysk duży — odwrotnie niż przy zwykłej pozycji.",
        type: "text",
        wide: true,
        when: (s) => s.trail_split && s.trail_runner_mode === "tiered",
      },
      {
        key: "trail_sr_enabled",
        label: "TRAILING S/R PO STRUKTURZE 1M",
        hint:
          "Bot wyznacza potwierdzone swingi na zamkniętych świecach i przesuwa SL tylko w stronę zysku, z konfigurowalnym offsetem. Odmowa brokera pomija daną aktualizację.",
        type: "bool",
      },
      {
        key: "sr_warmup_exact_ticks",
        label: "EKSPERYMENT: DOKŁADNA ROZGRZEWKA DYNAMICZNEGO S/R",
        hint: "Domyślnie OFF: dotychczasowa rozgrzewka. V2 wymaga ticków Bid/Ask sprzed granicy czasu, prawdziwego czasu pierwszego ticka, ekstremów MID i ostatniego spreadu. Dotyczy istniejących dynamicznych osi S/R, nie włącza rodzica ani osi ATR. Pełne ticki brokera nie są tym samym co próbki odebrane przez bota. Wdrożenie badawcze; obsługa LIVE i natywny parytet nie są jeszcze zatwierdzone.",
        type: "bool",
        when: (s) => s.trail_sr_enabled || s.sr_warmup_exact_ticks,
        warn: (s) => s.sr_warmup_exact_ticks ? "NIEZATWIERDZONE DO LIVE: brak ukończonego producenta historii V2 i dowodu zgodności strumienia." : null,
      },
      {
        key: "trail_sr_scope",
        label: "zakres pozycji",
        hint: "Runner = pozycje Hold (ta sama definicja co w trailingu rozdzielonym). Tp3Up i All rozszerzają zakres i wymagają osobnej walidacji.",
        type: "select",
        options: [
          { value: "Runner", label: "Runner — tylko runner Hold" },
          { value: "Tp3Up", label: "Tp3Up — warstwa TP3 i wyżej" },
          { value: "All", label: "All — każda pozycja" },
        ],
        when: (s) => s.trail_sr_enabled,
      },
      {
        key: "trail_sr_activation",
        label: "aktywacja od",
        hint: "Etapy liczone po DOTKNIĘCIACH celów sygnału (etap koszyka), nie po celach warstwy. Krzywa monotoniczna ku późnej aktywacji, plaskowyż Tp2–Tp3.",
        type: "select",
        options: [
          { value: "Entry", label: "Entry — od wejścia" },
          { value: "Gain", label: "Gain — od progu zysku $" },
          { value: "Tp1", label: "Tp1 — po pierwszym celu" },
          { value: "Tp2", label: "Tp2 — po drugim celu (zalecane)" },
          { value: "Tp3", label: "Tp3 — po trzecim celu" },
        ],
        when: (s) => s.trail_sr_enabled,
      },
      {
        key: "trail_sr_min_gain",
        label: "próg zysku (Gain)",
        hint: "Czytane TYLKO przy aktywacji Gain. 0 = od wejścia.",
        type: "num",
        min: 0,
        step: 1,
        unit: "$",
        when: (s) => s.trail_sr_enabled && s.trail_sr_activation === "Gain",
      },
      {
        key: "trail_sr_min_dist_price",
        label: "oddech od ceny ≥",
        hint: "Kandydat na poziom musi leżeć co najmniej o tę odległość od bieżącej ceny mid. 0 wyłącza filtr.",
        type: "num",
        min: 0,
        step: 1,
        unit: "$",
        when: (s) => s.trail_sr_enabled,
      },
      {
        key: "smart_sl",
        label: "SMART SL TRAILING",
        hint: "Po n-tym TP n najlepszych pozycji dostaje SL wg drabinki (Initial → TP1 → TP2…).",
        type: "bool",
      },
      {
        key: "breakeven_protection",
        label: "BREAKEVEN PROTECTION LAYER",
        hint: "Z drabinką dodaje etap BE: Initial → BE → TP1 → TP2… Bez SMART SL samodzielnie włącza tryb BreakevenOnly; nie wymaga scale-out.",
        type: "bool",
      },
      {
        key: "trail_after_tp2",
        label: "OPÓŹNIENIE DRABINKI SMART SL",
        hint: "Skrót: ON ustawia opóźnienie 1 etapu (dla najlepszej pozycji od TP2), OFF ustawia 0. Liczba większa niż 1 w presecie zostaje zachowana, dopóki nie klikniesz. Nie blokuje niezależnych BE, RF ani innych metod trailing.",
        type: "bool",
        when: (s) => s.smart_sl || s.risk_free_smart_sl || s.breakeven_protection,
      },
      {
        key: "harvest",
        label: "ZBIERANIE ZYSKU (harvest)",
        hint: 'Nie czeka aż SL zostanie trafiony — łapie „+20 i nagły zwrot”.',
        type: "bool",
      },
      { key: "harvest_start", label: "pilnuj od zysku", type: "num", min: 1, step: 1, unit: "pkt", when: (s) => s.harvest },
      {
        key: "harvest_retrace_pct",
        label: "cofnięcie o % szczytu → zamknij",
        type: "num",
        min: 5,
        max: 95,
        step: 5,
        unit: "%",
        when: (s) => s.harvest,
      },
      {
        key: "be_offset",
        label: "ODDECH NAD WEJŚCIEM (BE)",
        hint: "O ile powyżej ceny wejścia stawiać breakeven. Dokładnie na wejściu pozycja wychodzi na zero MINUS spread — czyli na minusie.",
        type: "num",
        min: 0,
        step: 0.05,
        unit: "$",
      },
      {
        key: "be_covers_late_fills",
        label: "BE KRYJE PÓŹNE FILLE I RE-ENTRY",
        hint: "Po włączeniu pozycja wypełniona po komendzie BE dziedziczy zabezpieczenie liczone od własnej ceny wejścia. Pozwala to zachować pendingi bez pozostawienia ich na pierwotnym stopie.",
        type: "bool",
      },
      {
        key: "be_never_loosen",
        label: "BE NIE COFA LEPSZEGO SL",
        hint: "ON: automatyczne BE po TP/RISK FREE nie pogarsza już korzystniejszego SL. Nie wymaga włączenia trailingu S/R. Nie uzbraja BE samodzielnie, nie gwarantuje zysku netto i nie blokuje świadomej ręcznej modyfikacji. OFF zachowuje dawną ścieżkę.",
        type: "bool",
      },
      {
        key: "sltp_retry_s",
        label: "PONAWIAJ ODRZUCONE SL/TP CO",
        hint: 'Broker odrzuca stop zbyt blisko ceny albo przy requote. Jedna próba i cisza oznacza, że podciągnięty stop po prostu ZNIKA — to różnica między „zamknięte na +18" a „zamknięte na SL". 0 = bez ponawiania.',
        type: "num",
        min: 0,
        step: 0.5,
        unit: "s",
      },
      {
        key: "trail_min_dist",
        label: "MIN. DYSTANS SL",
        hint: "Zamiast wysyłać SL, który broker odrzuci (bliżej niż STOPS LEVEL), dosuń go na tę odległość od ceny. 0 = OFF.",
        type: "num",
        min: 0,
        step: 0.01,
        unit: "$",
      },
      {
        key: "ladder_from_tp",
        label: "DRABINKA SL — start od TP",
        hint: "SL = osiągnięty TP[etap − lag] minus oddech. 0 = wyłączona.",
        type: "num",
        min: 0,
        max: 4,
        step: 1,
      },
      { key: "ladder_lag", label: "lag drabinki", type: "num", min: 0, max: 2, step: 1, when: (s) => s.ladder_from_tp > 0 },
      {
        key: "ladder_offset",
        label: "oddech drabinki",
        hint: "SL poniżej/powyżej poziomu, żeby nie wytrząsało pozycji tuż przed hitem.",
        type: "num",
        min: 0,
        step: 0.25,
        unit: "$",
        when: (s) => s.ladder_from_tp > 0,
      },
    ],
  },

  /* ================= WIRTUALNY SL ================= */
  {
    id: "vsl",
    title: "Wirtualny SL",
    icon: "shield",
    desc: "Poziom SL trzymany u bota — broker go nie widzi, więc nie da się go wyhuntować.",
    category: "management",
    zakres: "preset",
    accent: "var(--ai)",
    fields: [
      {
        key: "virtual_sl",
        label: "WIRTUALNY SL",
        hint: "Bot zamyka po rynku, gdy cena przetnie poziom. Prawdziwy SL z sygnału zostaje jako siatka bezpieczeństwa na wypadek padu VPS.",
        type: "bool",
      },
      {
        key: "virtual_sl_only_when_rejected",
        label: "tylko gdy broker odrzuca SL",
        type: "bool",
        when: (s) => s.virtual_sl,
      },
      {
        key: "vsl_eval_s",
        label: "kadencja sprawdzania",
        hint: '0 = co pętlę. 15–30 s = „tolerancja wicków”: szpilka między sprawdzeniami jest niewidzialna.',
        type: "num",
        min: 0,
        step: 5,
        unit: "s",
        when: (s) => s.virtual_sl,
      },
      {
        key: "virtual_sl_all",
        label: "WSZYSTKIE SL WIRTUALNE",
        hint: "Każdy SL trzymany u bota, a broker dostaje SL odsunięty o siatkę ratunkową. Wymaga WIRTUALNY SL = ON.",
        type: "bool",
        when: (s) => s.virtual_sl,
      },
      {
        key: "vsl_net_off",
        label: "odsunięcie siatki brokera",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.virtual_sl && s.virtual_sl_all,
      },
    ],
  },

  /* ================= FILOZOFIA ATFX ================= */
  {
    id: "atfx",
    title: "Filozofia ATFX",
    icon: "sparkles",
    desc: "Odwzorowanie zachowań grupy sygnałowej — więcej dni na plusie.",
    category: "management",
    zakres: "preset",
    accent: "var(--warn)",
    fields: [
      {
        key: "be_lock",
        label: "BE-LOCK",
        hint: 'Po osiągnięciu +X punktów SL ląduje na wejściu. „Nigdy nie oddawaj tego, co już było na plusie”.',
        type: "bool",
      },
      { key: "be_lock_points", label: "próg BE-LOCK", type: "num", min: 0.1, step: 0.5, unit: "pkt", when: (s) => s.be_lock },
      {
        key: "be_at_tp1",
        label: "BE DOPIERO NA TP1",
        hint: 'System tradera: „as soon as the trade hits tp1 set break even on ALL positions".',
        type: "bool",
      },
      {
        key: "reenter_after_tp",
        label: "RE-ENTRY po TP",
        hint: 'Wchodź ponownie, gdy cena wraca do strefy („TP1 HIT AGAIN AFTER PULLING BACK").',
        type: "bool",
      },
      {
        key: "reenter_max",
        label: "maks. powtórnych wejść na koszyk",
        hint: "0 = BEZ LIMITU (nie „zero powtórzeń”). Ogranicza, jak długo jeden setup może się odnawiać.",
        type: "num",
        min: 0,
        step: 1,
        when: (s) => s.reenter_after_tp,
        warn: (s) =>
          brakLimituOstrzezenie(
            s.reenter_max,
            "0 znaczy NIESKOŃCZENIE WIELE powtórnych wejść do jednego koszyka, a nie „zero”. " +
              "Z „VALID TILL TP2” bot może dokładać przy każdym powrocie ceny do strefy. " +
              "Chcesz hamulec — wpisz skończoną liczbę i zweryfikuj ją na rachunku demo.",
          ),
      },
      {
        key: "reenter_min_tp_stage",
        label: "wymagany etap TP dla re-entry",
        hint: "0 = wchodź przy pierwszym dotknięciu strefy, 1 = dopiero po TP1.",
        type: "num",
        min: 0,
        max: 4,
        step: 1,
        when: (s) => s.reenter_after_tp,
      },
      {
        key: "oae_timeout_min",
        label: "OUT-AT-ENTRY po",
        hint: "Zamknij pozycję ~na BE, jeśli wisi X minut bez zysku. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 5,
        unit: "min",
      },
      {
        key: "oae_profit_min",
        label: '„bez zysku” to poniżej',
        type: "num",
        min: 0,
        step: 0.1,
        unit: "pkt",
        when: (s) => s.oae_timeout_min > 0,
      },
      { key: "ignore_out_at_entry", label: "IGNORUJ „OUT AT ENTRY” z kanału", type: "bool" },
      {
        key: "ignore_risk_free",
        label: 'IGNORUJ „RISK FREE” z kanału',
        hint: "Test hipotezy permisywności: nie zamykaj siatki na komunikat RISK FREE.",
        type: "bool",
      },
    ],
  },

  /* ================= OFICJALNY SYSTEM ================= */
  {
    id: "official",
    title: "Oficjalny system ATFX",
    icon: "clipboard",
    desc: "Harmonogram procentowy podany wprost przez tradera grupy: 15 / 30 / 30 / 20.",
    category: "management",
    zakres: "preset",
    accent: "var(--long)",
    fields: [
      {
        key: "official_mode",
        label: "TRYB OFICJALNY",
        hint: "Harmonogram % zamiast płaskiej kaskady scale-out. Ma pierwszeństwo nad SCALE-OUT.",
        type: "bool",
      },
      { key: "official_pct_tp1", label: "zamykaj na TP1", type: "num", min: 0, max: 100, step: 5, unit: "%", when: (s) => s.official_mode && !s.official_use_counts },
      { key: "official_pct_tp2", label: "na TP2", type: "num", min: 0, max: 100, step: 5, unit: "%", when: (s) => s.official_mode && !s.official_use_counts },
      { key: "official_pct_tp3", label: "na TP3", type: "num", min: 0, max: 100, step: 5, unit: "%", when: (s) => s.official_mode && !s.official_use_counts },
      { key: "official_pct_spp", label: "każdy cel SPP", type: "num", min: 0, max: 100, step: 5, unit: "%", when: (s) => s.official_mode && !s.official_use_counts },
      {
        key: "official_use_counts",
        label: "LICZBY zamiast procentów",
        hint: 'Tak liczy trader: „1,2,1" = 1 pozycja na TP1, 2 na TP2, 1 na TP3, reszta = runnery.',
        type: "bool",
        when: (s) => s.official_mode,
      },
      { key: "official_counts", label: "harmonogram liczb", type: "text", when: (s) => s.official_mode && s.official_use_counts },
      {
        key: "official_spp",
        label: "NOWE SPP",
        hint: "Po TP3 zamykaj % na KAŻDYM celu + trailing SL, pozycje zostają otwarte.",
        type: "bool",
        when: (s) => s.official_mode,
      },
      {
        key: "official_round",
        label: "zaokrąglanie całych pozycji",
        hint: '„nearest" gubi TP1 na małym koncie: 15% z 3 pozycji = 0.45 → 0 zamkniętych. „up" zawsze inkasuje ≥ 1.',
        type: "select",
        options: [
          { value: "nearest", label: "nearest — najbliższa (domyślne)" },
          { value: "up", label: "up — zawsze ≥ 1 pozycja" },
          { value: "down", label: "down — ostrożnie" },
        ],
        when: (s) => s.official_mode,
      },
      {
        key: "official_assign_tps",
        label: "TP PER POZYCJA",
        hint: "Każda pozycja dostaje swój TP wg harmonogramu — MT5 zamyka je sam na dokładnym poziomie, nawet gdy bot offline.",
        type: "bool",
        when: (s) => s.official_mode,
      },
      { key: "official_close_last", label: "zamykaj ostatnią pozycję", type: "bool", when: (s) => s.official_mode },
      { key: "spp_keep_tp", label: "SPP zachowuje TP", type: "bool", when: (s) => s.official_mode },
      {
        key: "partial_close",
        label: "PARTIALE Z WOLUMENU",
        hint: "Zamykaj % wolumenu KAŻDEJ pozycji (0.05 → 0.04) zamiast % liczby całych pozycji. Główny tryb grupy.",
        type: "bool",
      },
      {
        key: "partial_min_lot",
        label: "próg partiali: lot ≥",
        hint: "0.01 jest niepodzielny. UDOWODNIONE: od 0.02 partiale dają +13 pkt proc. dni zyskownych.",
        type: "num",
        min: 0.01,
        step: 0.01,
        when: (s) => s.partial_close,
      },
      {
        key: "partial_pct_od_pierwotnego",
        label: "procent OD POZYCJI POCZĄTKOWEJ",
        hint:
          "Bez tego 15/40/15 liczy się od tego, CO ZOSTAŁO: 15 %, potem 40 % z 85 % (=34 %), " +
          "potem 15 % z 51 % (=7,7 %) — razem 56,7 % zamiast 70 % i ciąg, który nigdy nie domyka. " +
          "Włącz dla presetów rodziny TYLER.",
        type: "bool",
      },
      {
        key: "cele_na_ostatnim",
        label: "KAŻDA POZYCJA CELUJE W NAJDALSZY CEL",
        hint:
          "Rozdziela CELE od INKASA. Bez tego harmonogram procentowy rozkłada pozycje po szczeblach " +
          "drabinki, więc pozycja przypisana do TP1 zamyka się u brokera na TP1 i nie dożywa TP2 — " +
          "a wtedy nie ma z czego kroić kolejnych transz.",
        type: "bool",
      },
      {
        key: "retarget_respects_final_target",
        label: "PRZEPISYWANIE TP SZANUJE NAJDALSZY CEL",
        hint: "ON razem z cele_na_ostatnim: retarget pozostawia żywe pozycje przy najdalszym celu zamiast przepisać je na bliższy TP. Dotyczy pozycji, które już mają brokerski TP; nie odtwarza celowo usuniętego TP=None. Nie zmienia samego inkasa transz. OFF zachowuje dawną ścieżkę.",
        type: "bool",
        when: (s) => s.cele_na_ostatnim || s.retarget_respects_final_target,
      },
      {
        key: "sl_polowa_od_konca",
        label: "SL w połowie drogi — od którego celu OD KOŃCA",
        hint:
          "0 = wyłączone. 1 = po wszystkich celach poza ostatnim. Liczone od końca, bo Synergy podaje " +
          "różną liczbę celów w różnych sygnałach. Zapadka: stop rusza tylko w stronę zysku.",
        type: "num",
        min: 0,
        step: 1,
      },
      {
        key: "sl_polowa_ulamek",
        label: "ułamek drogi wejście → cena",
        hint: "0,5 = połowa. 0 czyta się jak 0,5. Mniej zostawia runnerowi więcej powietrza.",
        type: "num",
        min: 0,
        max: 1,
        step: 0.05,
        when: (s) => (s.sl_polowa_od_konca ?? 0) > 0,
      },
    ],
  },

  /* ================= RISK FREE ================= */
  {
    id: "riskfree",
    title: "RISK FREE",
    icon: "lock",
    desc: "Co bot robi z koszykiem po komunikacie RISK FREE z kanału.",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "risk_free_runner_target",
        label: "CEL RUNNERA PO RISK FREE",
        type: "select",
        options: [
          { value: "last", label: "najdalszy cel drabinki" },
          { value: "keep", label: "zostaw dotychczasowy" },
          { value: "next", label: "kolejny nietrafiony cel" },
          { value: "none", label: "bez celu — tylko trailing" },
        ],
      },
      {
        key: "risk_free_trail",
        label: "runner dostaje trailing po RISK FREE",
        type: "bool",
      },
      {
        key: "risk_free_be_min_profit",
        label: "RISK FREE rusza stopem dopiero przy zysku",
        hint: "Oddziela miejsce ustawienia stopu (be_offset) od minimalnego bieżącego zysku wymaganego do jego przesunięcia. 0 zachowuje starszą semantykę.",
        type: "num",
        min: 0,
        max: 50,
        step: 0.5,
        unit: "$",
      },
      {
        key: "out_at_entry_mode",
        label: "OUT AT ENTRY — co zamykać",
        hint: 'Komunikat znaczy różne rzeczy u różnych sygnalistów: raz „wychodzę z całości", raz „wyrzuć to, co stoi w miejscu".',
        type: "select",
        options: [
          { value: "close_all", label: "cały koszyk" },
          { value: "losers", label: "tylko stratne" },
          { value: "flat", label: "tylko te ~na zero" },
          { value: "be", label: "nic nie zamykaj, przesuń SL na wejście" },
        ],
        when: (s) => !s.ignore_out_at_entry,
      },
      {
        key: "oae_band_pts",
        label: 'pasmo „na zero”',
        type: "num",
        min: 0,
        step: 0.1,
        unit: "pkt",
        when: (s) => !s.ignore_out_at_entry && s.out_at_entry_mode === "flat",
      },
      {
        key: "sl_hit_mode",
        label: "SL HIT z kanału — co zrobić",
        type: "select",
        options: [
          { value: "cancel_pendings", label: "skasuj limity, pozycje zostaw na własnym SL" },
          { value: "close_all", label: "zamknij wszystko po rynku" },
          { value: "verify", label: "zweryfikuj ceną (odrzuć fałszywy)" },
          { value: "ignore", label: "ignoruj" },
        ],
      },
      {
        key: "honor_cancel",
        label: "reaguj na CANCEL z kanału",
        type: "bool",
      },
      {
        key: "honor_close_all",
        label: "reaguj na CLOSE ALL z kanału",
        hint: "Ten komunikat zamyka WSZYSTKO, nie tylko wskazany koszyk — chyba że zawęzisz go polem obok.",
        type: "bool",
      },
      {
        key: "close_all_scope",
        label: "zasięg CLOSE ALL",
        hint: "Tryb globalny pomija adres odpowiedzi i może zamknąć także inne koszyki. Używaj go tylko wtedy, gdy składnia źródła jednoznacznie oznacza zamknięcie całego portfela.",
        type: "select",
        options: [
          { value: "Global", label: "Global — zamknij wszystko (jak dotąd)" },
          { value: "Basket", label: "Basket — tylko koszyk-adresat" },
        ],
        when: (s) => s.honor_close_all,
      },
      {
        key: "partials_wykonuj",
        label: "WYKONUJ „TAKE PARTIALS” Z KANAŁU",
        hint: "Jednoznaczne polecenie częściowego zamknięcia może realizować zysk od razu. Wariant warunkowy lub sugestia pozostaje informacją.",
        type: "bool",
      },
      {
        key: "partials_pct",
        label: "transza na „take partials”",
        hint: "% WOLUMENU inkasowanego po komendzie. Osobna liczba, a NIE szczebel drabinki inkasa (TP1/TP2/TP3): komenda nie niesie etapu, więc użycie drabinki albo zmyśliłoby etap, albo zjadło szczebel, którego prawdziwy meldunek TP będzie za chwilę potrzebował. Kolejność, próg partiali i ochrona ostatniej pozycji — wspólne z inkasem na celu. 0 = nic nie inkasujemy.",
        type: "num",
        min: 0,
        max: 100,
        step: 5,
        unit: "%",
        when: (s) => s.partials_wykonuj,
      },
      {
        key: "honor_market_open",
        label: 'reaguj na „BUY NOW” / „SELL NOW”',
        hint: "Taki komunikat nie niesie ani SL, ani celów — pozycja powstaje bez planu wyjścia. Domyślnie wyłączone.",
        type: "bool",
      },
      {
        key: "dedup_edited_signals",
        label: "NIE POWTARZAJ AKCJI PRZY EDYCJI",
        hint: 'Sygnalista dopisuje treść do wysłanej już wiadomości („TP1 HIT" → „TP1 HIT · SECURING PARTIAL PROFITS"). Bez pamięci wykonanych akcji edycja inkasuje TP1 drugi raz.',
        type: "bool",
      },
      {
        key: "risk_free_runners",
        label: "ile najlepszych pozycji zostawić",
        hint: "Reszta zamykana, te zostają otwarte z SL na breakeven.",
        type: "num",
        min: 1,
        step: 1,
      },
      {
        key: "risk_free_mode",
        label: "tryb koszyka po RISK FREE",
        type: "select",
        options: [
          { value: "scale_out", label: "zostaw SCALE-OUT (runnery kaskadą)" },
          { value: "all_runners", label: "ALL TPS ARE RUNNERS (tylko ten koszyk)" },
        ],
      },
      {
        key: "risk_free_smart_sl",
        label: "SMART SL dla runnerów po RISK FREE",
        hint: "Schodkowy SL liczony względem siebie — najlepszy runner ma najmocniejszy SL.",
        type: "bool",
        when: (s) => s.risk_free_mode === "all_runners",
      },
      {
        key: "sl_hit_verify_tol",
        label: 'tolerancja weryfikacji „SL HIT”',
        hint: "Tylko przy trybie SL HIT = weryfikuj ceną. Gdy cena mid jest dalej niż X $ od SL po stronie zysku, komunikat jest ignorowany. 0 = ścisła tolerancja bez bufora, NIE wyłączenie weryfikacji. Tryb wybierasz osobno.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.sl_hit_mode === "verify",
      },
    ],
  },

  /* ================= EDYCJE I DEDUP (Pakiet A) ================= */
  {
    id: "edycje",
    title: "Edycje i dedup wiadomości",
    icon: "edit",
    desc: "Zaawansowane osie parsera: co bot robi z EDYCJAMI wiadomości kanału i z powtórkami po rekonekcie (Pakiet A).",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "dedup_pelny_status",
        label: "PEŁNY STATUS WYKONANIA AKCJI",
        hint: "Akcja ZIGNOROWANA (np. odrzucona ceną lub bez koszyka celu) nie zapisuje się jako wykonana, więc jej ponowienie w późniejszej edycji nadal może zadziałać. Wyłączenie przywraca starszą semantykę dedupu.",
        type: "bool",
      },
      {
        key: "edycja_wykonuje_reszte_akcji",
        label: "edycja z wejściem wykonuje resztę akcji",
        hint: "Dziś edycja trafiająca w koszyk przezbraja go i KOŃCZY obsługę wiadomości — doklejone „TP1 HIT”/„SPP” w tej samej edycji przepada. Włączenie puszcza resztę akcji normalną ścieżką.",
        type: "bool",
      },
      {
        key: "dedup_klucz_z_wartoscia",
        label: "klucz dedupu niesie wartość",
        hint: "Edycja zmieniająca POZIOM („MOVE SL TO 4120” → „4110”) nie ginie jako duplikat: klucz akcji zawiera wartość (setsl@4110, corr2@4162, rf@4536, spp@cele|sl|be).",
        type: "bool",
      },
      {
        key: "edycja_sieroty_nie_otwiera",
        label: "edycja-sierota nie otwiera koszyka",
        hint: "Edycja wiadomości, której bot nie zna (np. po restarcie), z wejściem w treści NIE zakłada nowego koszyka na starych cenach. Komunikaty zarządzające z tej wiadomości idą dalej normalnie.",
        type: "bool",
      },
      {
        key: "entry_idempotencja",
        label: "idempotencja nowej wiadomości",
        hint: "Re-delivery po rekonekcie: nowa wiadomość z msg_id, który już otworzył koszyk, jest traktowana jak edycja tego koszyka zamiast otwierać drugi.",
        type: "bool",
      },
      {
        key: "dedup_management_po_restarcie",
        label: "trwały dedup zarządzania po restarcie",
        hint: "Zapisuje przy żywym koszyku wykonane TP/SPP/BE/SL/CANCEL i odtwarza tę pamięć z koszyki.json. Ta sama wiadomość lub jej kolejna edycja po restarcie nie wykona starych akcji drugi raz. Domyślnie OFF dla kontraktu legacy.",
        type: "bool",
      },
      {
        key: "profit_update_telemetry_only",
        label: "AT TP jest tylko informacją (TP HIT pozostaje aktywne)",
        hint: "AT TP1/2/3 oznacza jedynie bliskość ceny i nie bankuje ani nie przesuwa etapu. Pozostałe jawne akcje z tej samej wiadomości (RF, SPP, CANCEL, BE/SL, nowe cele) nadal są wykonywane. TPn HIT pozostaje informacją, a przy tp_source=SignalConfirmedByPrice o wykonaniu decyduje bieżący Bid/Ask MT5. Domyślnie OFF = legacy 1:1.",
        type: "bool",
      },
    ],
  },

  {
    id: "tpPriceOnlyContract",
    title: "Jednoznaczne źródło TP",
    desc: "Jawny kontrakt PriceOnly; osobne polecenia zarządzania pozostają niezależne.",
    icon: "target",
    category: "management",
    zakres: "preset",
    fields: [{
      key: "tp_price_only_strict",
      label: "PriceOnly: żaden komunikat TP nie steruje etapem",
      hint: "W PriceOnly blokuje także historyczny wyjątek +N PIPS HIT z tp_unindexed_pips_require_price. RF, SPP, SL i inne jawne komendy nadal działają; ticki i wykonania brokera są osobną ścieżką. Inne tryby TP bez zmian. OFF zachowuje stary wyjątek Rust; ON odpowiada blokadzie PriceOnly w testerze MT5.",
      type: "bool",
    }],
  },

  
  {
    id: "tyler",
    title: "Wykonanie sygnału rynkowego",
    icon: "target",
    desc: "Osie rozróżniające zapowiedź od polecenia, kontrolujące rozmiar wejścia rynkowego, limity po RF i realizację celu.",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "rf_wymaga_wykonania",
        label: "RISK FREE wymaga wykonania, nie zapowiedzi",
        hint: "Zapowiedź lub warunkowa intencja bez poziomu liczbowego zostaje informacją, natomiast jednoznaczne polecenie z poziomem może zostać wykonane.",
        type: "bool",
      },
      {
        key: "market_entry_units",
        label: "jednostki wejścia RYNKOWEGO (0 = pełna siatka)",
        hint: "Limit całego planu dla sygnału bez „LIMITS”, również przy auto_limit=ON. Zachowuje N jednostek od najgłębszej prawidłowej warstwy; nie oznacza N natychmiastowych wejść po rynku. Przy N=1 bot może wystawić tylko głęboki pending i nie wejść podczas płytkiego odbicia do TP. Ogranicza też plan z entry_units; sygnał jawnie LIMITS nie podlega temu limitowi. 0 wyłącza wyłącznie to przycinanie, nie pozostałe limity ryzyka.",
        type: "num",
      },
      {
        key: "market_hybrid_now_units",
        label: "hybryda: jednostki TERAZ (0 = OFF)",
        hint: "Dla sygnału bez LIMITS otwiera N jednostek z pierwszej/płytkiej części planu natychmiast po rynku, a resztę pozostawia jako niższe limity. Ma pierwszeństwo przed market_entry_units, które bez hybrydy potrafi zostawić tylko jeden głęboki pending.",
        type: "num",
      },
      {
        key: "market_hybrid_pending_units",
        label: "hybryda: maks. jednostek oczekujących (0 = wszystkie)",
        hint: "Ile pozostałych jednostek limit zachować, licząc od najlepszych/najgłębszych. Nie obejmuje nóg otwieranych natychmiast.",
        type: "num",
      },
      {
        key: "market_hybrid_lot_mult",
        label: "hybryda: mnożnik lota nogi TERAZ",
        hint: "Skaluje wyłącznie natychmiastowe jednostki hybrydy. 1 = bez zmiany; wartości niepoprawne lub ≤0 są bezpiecznie traktowane jak 1.",
        type: "num",
      },
      {
        key: "market_hybrid_max_chase_usd",
        label: "hybryda: maks. chase od strefy, $ (0 = bez limitu)",
        hint: "Jeśli bieżąca cena jest dalej od płytkiej krawędzi niż próg, noga market nie goni ceny; pełna siatka nadal czeka jako limity, więc sygnał nie jest filtrowany.",
        type: "num",
      },
      {
        key: "market_hybrid_tp_stage",
        label: "hybryda: TP nogi TERAZ (0 = planer, 255 = OPEN)",
        hint: "1/2/3… przypisuje natychmiastowej nodze konkretny TP sygnału; numer ponad listę jest ograniczany do ostatniego. 255 zdejmuje stały TP i zostawia zarządzanie runnerom.",
        type: "num",
      },
      {
        key: "market_unfilled_cancel_stage",
        label: "market bez fillu: anuluj pendingi na TP (0 = wspólne ustawienie)",
        hint: "Dla sygnału bez LIMITS i bez własnej pozycji wskazany etap TP kończy spóźnioną siatkę. 1 zapobiega wejściu dopiero po TP1; 2/3 pozwala jej żyć dłużej; 255 wyłącza tę regułę. Jawne LIMITS są nietknięte.",
        type: "num",
      },
      {
        key: "pending_cancel_on_riskfree",
        label: "RISK FREE kasuje wiszące limity",
        hint: "Kasuje niewypełnione zlecenia koszyka przy wykonaniu RISK FREE, aby późniejsze wejścia nie zwiększały ponownie ryzyka.",
        type: "bool",
      },
      {
        key: "bank_all_at_stage",
        label: "bank CAŁOŚCI koszyka od etapu TP (0 = OFF)",
        hint: "Na etapie ≥ N cały koszyk jest zamykany (pozycje + pendingi) zamiast partiali i runnerów. Tyler bankuje hurtem w strefie TP3. Niejednoznaczne w pomiarze (+930 $ / −478 $) — oś do sweepa, nie do produkcji.",
        type: "num",
      },
      {
        key: "stat_be_prog_usd",
        label: "próg remisu (BE) w raportach, $",
        hint: "Wyłącznie POMIAR — nie zmienia ani jednej decyzji. Transakcja o |wyniku| ≤ progu liczy się jako remis, a nie przegrana; bez tego wyjścia „OUT AT ENTRY” i „SL na BE” zaniżały skuteczność. 0 = tylko dokładne zero (stare tabele bez zmian).",
        type: "num",
      },
    ],
  },

  /* ================= PAKIET F: ADRESOWANIE I HAMULEC ================= */
  {
    id: "pakiet_f",
    title: "Adresat komunikatu i hamulec SL-HIT",
    icon: "shield",
    desc: "Osie chroniące adresowanie odpowiedzi do koszyka oraz sterujące hamulcem po komunikatach o stopie (Pakiet F).",
    category: "management",
    zakres: "preset",
    accent: "var(--warn)",
    fields: [
      {
        key: "reply_veto",
        label: "odpowiedź do NIEZNANEJ wiadomości nie rusza koszyka",
        hint: "Odpowiedź do nieznanego sygnału jest ignorowana zamiast korzystać z awaryjnego wyboru najnowszego koszyka. Komunikat bez reply_to zachowuje dotychczasową ścieżkę.",
        type: "bool",
      },
    ],
  },

  /* ================= WYJSCIA ================= */
  {
    id: "exits",
    title: "Stagnacja i wyjścia awaryjne",
    icon: "hourglass",
    desc: "Bankowanie pozycji, które przestały pracować, i wyjścia na odwróceniu.",
    category: "management",
    zakres: "preset",
    accent: "var(--short)",
    fields: [
      {
        key: "stale_take_min",
        label: "STAGNACJA — brak nowego szczytu przez",
        hint: "Wyrocznia: mediana pik → zamknięcie 51 min. Pozycja z zyskiem, która przez X minut nie zrobiła nowego piku, jest zamykana po rynku. 0 = OFF.",
        type: "num",
        min: 0,
        step: 10,
        unit: "min",
      },
      { key: "stale_take_profit", label: "przy zysku ≥", type: "num", step: 1, unit: "pkt", when: (s) => s.stale_take_min > 0 },
      {
        key: "stale_take_min2",
        label: "drugi człon stagnacji — po",
        hint: "Dodaje drugą gałąź OR, dzięki czemu wystarczy spełnić jeden z dwóch warunków wieku i zysku. Obie gałęzie należy walidować na niezależnych danych.",
        type: "num",
        min: 0,
        step: 10,
        unit: "min",
      },
      { key: "stale_take_profit2", label: "przy zysku ≥", type: "num", step: 1, unit: "pkt", when: (s) => s.stale_take_min2 > 0 },
      {
        key: "rev_exit_range",
        label: "REV-EXIT — zakres odwrócenia",
        hint: "Wyjście na wykrytym odwróceniu momentum. 0 = OFF.",
        type: "num",
        min: 0,
        step: 1,
        unit: "$",
      },
      { key: "rev_exit_slope", label: "nachylenie", type: "num", step: 1, when: (s) => s.rev_exit_range > 0 },
      { key: "rev_exit_profit", label: "min. zysk do wyjścia", type: "num", step: 1, unit: "pkt", when: (s) => s.rev_exit_range > 0 },
      { key: "rev_exit_window_min", label: "okno pomiaru", hint: "Zakres i oddanie przewagi liczone są w tym oknie.", type: "num", min: 5, step: 5, unit: "min", when: (s) => s.rev_exit_range > 0 },
    ],
  },

  /* ================= MADRE WYJSCIE ================= */
  {
    id: "smartexit",
    title: "Mądre wyjście",
    icon: "brain",
    desc: "To, co człowiek robi patrząc na wykres: bierz duży zysk, uciekaj przy nagłym spadku — ale NIE uciekaj, gdy tuż pod ceną czeka własny limit.",
    category: "management",
    zakres: "preset",
    accent: "var(--ai)",
    fields: [
      {
        key: "smart_exit",
        label: "WŁĄCZ MĄDRE WYJŚCIE",
        hint: "Domyślnie wyłączone. Działa obok zapadki i celów — nie zastępuje ich.",
        type: "bool",
      },
      {
        key: "smart_exit_take",
        label: "bierz zysk natychmiast od",
        hint: "0 = nigdy nie zamykaj z tego powodu.",
        type: "num",
        min: 0,
        step: 1,
        unit: "$",
        when: (s) => s.smart_exit,
      },
      {
        key: "smart_exit_giveback",
        label: "zamknij po oddaniu części szczytu",
        hint: "0,30 = oddane 30 % najlepszego wyniku pozycji. 0 = OFF.",
        type: "num",
        min: 0,
        max: 1,
        step: 0.05,
        when: (s) => s.smart_exit,
      },
      {
        key: "smart_exit_min_peak",
        label: "…ale dopiero od szczytu",
        hint: "Bez tego progu reguła ścinałaby pozycje, które ledwo weszły na plus.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.smart_exit && s.smart_exit_giveback > 0,
      },
      {
        key: "smart_exit_drop_speed",
        label: "zamknij przy spadku szybszym niż",
        hint: "0 = nie patrz na prędkość.",
        type: "num",
        min: 0,
        step: 0.25,
        unit: "$/min",
        when: (s) => s.smart_exit,
      },
      {
        key: "smart_exit_speed_window_s",
        label: "okno pomiaru prędkości",
        type: "num",
        min: 5,
        step: 5,
        unit: "s",
        when: (s) => s.smart_exit && s.smart_exit_drop_speed > 0,
      },
      {
        key: "smart_exit_hold_if_pending",
        label: "NIE zamykaj, gdy limit czeka bliżej niż",
        hint: "Serce reguły. Pozycja spadająca w stronę WŁASNEJ siatki to inna sytuacja niż pozycja spadająca w próżnię: limit obniży średnią koszyka, a powrót wyprowadzi całość na plus. 0 = nie uwzględniaj siatki.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.smart_exit,
      },
      {
        key: "smart_exit_min_pendings",
        label: "wystarczy tylu czekających limitów",
        type: "num",
        min: 1,
        step: 1,
        when: (s) => s.smart_exit && s.smart_exit_hold_if_pending > 0,
      },
      {
        key: "smart_exit_pending_scope",
        label: "czyje limity się liczą",
        type: "select",
        options: [
          { value: "SameBasket", label: "tylko tego koszyka (uśredniają cenę)" },
          { value: "AnyBasket", label: "dowolne nasze (wsparcie dla ceny)" },
        ],
        when: (s) => s.smart_exit && s.smart_exit_hold_if_pending > 0,
      },
      {
        key: "smart_exit_pending_min_dist",
        label: "pomiń limity bliższe niż",
        hint: "Limit tuż pod ceną i tak zaraz się wypełni, więc nie niesie informacji „cena ma dokąd wrócić”.",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "$",
        when: (s) => s.smart_exit && s.smart_exit_hold_if_pending > 0,
      },
    ],
  },

  /* ================= REZIM ZMIENNOSCI ================= */
  {
    id: "volregime",
    title: "Reżim zmienności",
    icon: "activity",
    desc: "Tnij rozmiar w sztormie — mniejszy drawdown przy wyższym zysku.",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "vol_window_min",
        label: "okno pomiaru zakresu",
        hint: "Mediana zakresu 60 min na XAUUSD to ~19 $, więc próg 15 przy oknie 30 min jest realny. 0 = OFF.",
        type: "num",
        min: 0,
        step: 5,
        unit: "min",
      },
      { key: "vol_range_usd", label: "próg zakresu H−L", type: "num", step: 1, unit: "$", when: (s) => s.vol_window_min > 0 },
      { key: "vol_units_mult", label: "mnożnik jednostek w sztormie", type: "num", min: 0.1, max: 1, step: 0.1, when: (s) => s.vol_window_min > 0 },
    ],
  },

  /* ================= OCHRONA KAPITALU ================= */
  {
    id: "guards",
    title: "Ochrona kapitału i bramki wejść",
    icon: "shield-alert",
    desc: "Strażnicy, które zatrzymują bota zanim zły dzień zamieni się w katastrofę. Jeśli próg obsunięcia ma działać, musi mieć wartość większą od zera.",
    category: "management",
    zakres: "preset",
    accent: "var(--short)",
    fields: [
      {
        key: "max_dd_pct",
        label: "MAX DRAWDOWN — STRAŻNIK KAPITAŁU",
        hint: "Po osiągnięciu progu bot zamyka wszystko i blokuje nowe sygnały. 0 = STRAŻNIK WYŁĄCZONY, konto nie ma żadnej dolnej granicy. Wszystkie presety przychodzą z 0 — to jedyne miejsce, w którym da się go uzbroić. Zdjęcie strażnika zmieniło wynik presetu KRATA z 520 na 2 381 $ w długim compoundingu, ale zmieniło też ryzyko z ograniczonego na nieograniczone.",
        type: "num",
        min: 0,
        step: 1,
        unit: "%",
        warn: (s) =>
          s.max_dd_pct === 0 && s.max_dd_usd === 0
            ? "STRAŻNIK WYŁĄCZONY — bot nie zatrzyma handlu przy żadnym obsunięciu. Wpisz wartość w tym polu albo w kwotowym, żeby go uzbroić."
            : null,
      },
      {
        key: "max_dd_usd",
        label: "MAX DRAWDOWN kwotowo",
        hint: "To samo w dolarach. Działa niezależnie od progu procentowego — wystarczy, że zadziała którykolwiek. 0 = ten próg nieaktywny.",
        type: "num",
        min: 0,
        step: 10,
        unit: "$",
      },
      {
        key: "alert_dd_pct",
        label: "OSTRZEŻENIE MAILEM przy obsunięciu",
        hint: "Sam e-mail, BEZ zatrzymywania handlu — działa również (a właściwie zwłaszcza) przy wyłączonym strażniku. Bez tego przy progach 0/0 kategoria „drawdown” nie wysłałaby ani jednego maila, cokolwiek stałoby się z kontem w nocy. Kolejne ostrzeżenia co +5 pp pogłębienia. 0 = bez ostrzeżeń.",
        type: "num",
        min: 0,
        max: 100,
        step: 1,
        unit: "%",
      },
      {
        key: "signal_max_age_min",
        label: "MAKS. WIEK SYGNAŁU OTWIERAJĄCEGO",
        hint: "Sygnał wejścia starszy niż próg nie zostanie wykonany. Komunikaty zarządzające i edycje istniejących koszyków przechodzą niezależnie od wieku. 0 wyłącza bramkę.",
        type: "num",
        min: 0,
        max: 240,
        step: 1,
        unit: "min",
      },
      {
        key: "dd_guard_scope",
        label: "ZASIĘG BLOKADY PO OBSUNIĘCIU",
        hint: 'Wariant „od szczytu wszech czasów" przy limicie 40% zatrzymał backtest 8 kwietnia i bot nie handlował do końca lipca — trzy dni danych zamiast czterech miesięcy. Dla bota pracującego 24/7 właściwą odpowiedzią na zły dzień jest przerwa do północy.',
        type: "select",
        options: [
          { value: "daily", label: "dzienny — od szczytu dnia, wygasa o północy" },
          { value: "lifetime", label: "dożywotni — od szczytu wszech czasów, do ręcznego wznowienia" },
          { value: "lifetime_daily_reset", label: "od szczytu wszech czasów, blokada do północy" },
        ],
      },
      {
        key: "max_portfolio_risk_pct",
        label: "SUFIT OTWARTEGO RYZYKA — CAŁY RACHUNEK",
        hint: "Ogranicza łączne otwarte ryzyko pozycji i aktywnych zleceń oczekujących. Przekroczenie zmniejsza nowy koszyk; 0 wyłącza limit.",
        type: "num",
        min: 0,
        step: 5,
        unit: "%",
      },
      {
        key: "dd_soft_pct",
        label: "DŁAWIK — PIERWSZY PRÓG OBSUNIĘCIA",
        hint: "Powyżej tego obsunięcia bot gra MNIEJSZĄ siatką zamiast przestać grać. Baza obsunięcia jest ta sama, co u strażnika wyżej (pole „zasięg blokady”). 0 = wyłączony.",
        type: "num",
        min: 0,
        max: 100,
        step: 5,
        unit: "%",
      },
      {
        key: "dd_soft_mult",
        label: "…mnożnik budżetu przy pierwszym progu",
        hint: "0,5 = graj połową. Dławik nie dotyka pozycji, które już żyją.",
        type: "num",
        min: 0,
        max: 1,
        step: 0.05,
      },
      {
        key: "dd_hard_pct",
        label: "DŁAWIK — DRUGI PRÓG OBSUNIĘCIA",
        hint: "Głębsze obsunięcie, mocniejsze dławienie. 0 = wyłączony.",
        type: "num",
        min: 0,
        max: 100,
        step: 5,
        unit: "%",
      },
      {
        key: "dd_hard_mult",
        label: "…mnożnik budżetu przy drugim progu",
        hint: "0,25 = graj ćwiartką. Konto dalej zarabia, tylko wolniej — więc ma z czego wrócić.",
        type: "num",
        min: 0,
        max: 1,
        step: 0.05,
      },
      {
        key: "max_open_baskets",
        label: "MAKS. OTWARTYCH KOSZYKÓW",
        hint: "Limit liczby jednoczesnych setupów, niezależny od limitu pozycji. 0 = BEZ LIMITU (nie „zero koszyków”).",
        type: "num",
        min: 0,
        step: 1,
        warn: (s) =>
          brakLimituOstrzezenie(
            s.max_open_baskets,
            "0 znaczy DOWOLNIE WIELE jednoczesnych setupów, a nie „zero”. Każdy koszyk to własna " +
              "siatka i własny margines — przy serii sygnałów w jedną stronę rachunek stoi całym " +
              "wolumenem po jednej stronie rynku. Chcesz sufit — wpisz liczbę.",
          ),
      },
      {
        key: "max_directional_lots",
        label: "MAKS. WOLUMEN W JEDNĄ STRONĘ",
        hint: "Sygnały chodzą seriami i bywają jednokierunkowe — bez tego limitu konto stoi całym wolumenem po jednej stronie rynku. 0 = bez limitu.",
        type: "num",
        min: 0,
        step: 0.01,
        unit: "lot",
      },
      {
        key: "equity_floor_pct",
        label: "PODŁOGA EQUITY",
        hint: "Poniżej tego % kapitału startowego bot nie otwiera już nic nowego. Otwarte pozycje dokańczają normalnie. 0 = wyłączone.",
        type: "num",
        min: 0,
        max: 100,
        step: 5,
        unit: "%",
      },
      {
        key: "regime_filter",
        label: "FILTR REŻIMU RYNKU",
        hint: "Handluj tylko zgodnie z nachyleniem średniej z N godzin — albo wyłącznie przeciw niemu.",
        type: "select",
        options: [
          { value: "off", label: "wyłączony" },
          { value: "trend", label: "zgodnie z trendem" },
          { value: "counter", label: "przeciw trendowi (fade)" },
        ],
      },
      {
        key: "regime_ma_hours",
        label: "okno średniej",
        type: "num",
        min: 2,
        step: 6,
        unit: "h",
        when: (s) => s.regime_filter !== "off",
      },
      {
        key: "max_open_positions",
        label: "STRAŻNIK EKSPOZYCJI",
        hint:
          "Maks. liczba jednocześnie otwartych pozycji. 0 = BEZ LIMITU (nie „zero pozycji”) — " +
          "wtedy strażnik NIE ISTNIEJE. Chroni małe konto przed zerowaniem w nagłym obsunięciu.",
        type: "num",
        min: 0,
        step: 1,
        warn: (s) =>
          brakLimituOstrzezenie(
            s.max_open_positions,
            "0 znaczy DOWOLNIE WIELE otwartych pozycji naraz, a nie „zero” — strażnik jest wtedy " +
              "wyłączony. Wypełnione pendingi mogą zwiększać ekspozycję, jeśli nie uwzględnia ich pole towarzyszące. " +
              "Chcesz strażnika — wpisz liczbę i włącz „licz pendingi do limitu”.",
          ),
      },
      { key: "exposure_count_pendings", label: "licz pendingi do limitu", type: "bool", when: (s) => s.max_open_positions > 0 },
      {
        key: "lot_scale_step",
        label: "AUTO LOT-SCALING: +0.01 lota co",
        hint: "Compound. Np. 1000 → po $1000 balansu lot 0.02. 0 = stały lot.",
        type: "num",
        min: 0,
        step: 50,
        unit: "$",
      },
      {
        key: "day_target_usd",
        label: "CEL DZIENNY",
        hint: "Po osiągnięciu +X $ na dzień bot nie otwiera nic nowego. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 10,
        unit: "$",
      },
      {
        key: "day_target_close",
        label: "po celu ZAMKNIJ wszystko",
        hint: '„Weź wygraną i idź spać" — poprawia każde okno backtestu.',
        type: "bool",
        when: (s) => s.day_target_usd > 0,
      },
      { key: "day_target_scale_lot", label: "skaluj cel z lotem", type: "bool", when: (s) => s.day_target_usd > 0 },
      {
        key: "day_trail_stop_usd",
        label: "DAY-TRAIL: zamknij po spadku od piku dnia",
        hint: "Automat ręcznego zamknięcia. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 10,
        unit: "$",
      },
      {
        key: "usd_scale_with_lot",
        label: "MASTER: skaluj progi $ z lotem",
        hint: "Wszystkie progi dolarowe mnożone przez lot/0.01. Progi punktowe skalują się naturalnie.",
        type: "bool",
      },
      {
        key: "eod_flat_hour",
        label: "EOD-FLAT o godzinie",
        hint: "Codziennie o tej godzinie zamknij wszystko i skasuj pendingi. Sweep: oś EOD23 = +360/mies vs −84 bez. 0 = wyłączone.",
        type: "num",
        min: 0,
        max: 23,
        step: 1,
        unit: "h",
      },
      { key: "flat_weekend", label: "FLAT PRZED WEEKENDEM", hint: "Ochrona przed luką weekendową.", type: "bool" },
      { key: "flat_weekend_hour", label: "godzina w piątek", type: "num", min: 0, max: 23, step: 1, unit: "h", when: (s) => s.flat_weekend },
      { key: "day_flat_broker_clock", label: "zegar brokera zamiast lokalnego", type: "bool" },
      {
        key: "session_filter",
        label: "FILTR SESJI",
        hint: "Handluj tylko w podanych godzinach. Złoto ma godziny o różnej charakterystyce.",
        type: "bool",
      },
      { key: "session_hours", label: "godziny sesji", hint: 'Format: „7-20" albo „7-11,13-20".', type: "text", when: (s) => s.session_filter },
      {
        key: "streak_pause_n",
        label: "PAUZA PO SERII STRAT — po N koszykach",
        hint: "Sygnały chodzą seriami. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 1,
      },
      { key: "streak_pause_min", label: "długość pauzy", type: "num", step: 15, unit: "min", when: (s) => s.streak_pause_n > 0 },
      {
        key: "slhit_pause_n",
        label: "HAMULEC SL-HIT — po N stopach KANAŁU w dobie",
        hint: "Inne źródło niż pauza po serii strat: liczy komunikaty „SL HIT” samego kanału, więc może reagować zanim strata zmaterializuje się na rachunku. Zbyt małe N może odciąć znaczną część późniejszych sygnałów. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 1,
      },
      { key: "slhit_pause_min", label: "długość pauzy (0 = do końca doby)", type: "num", step: 30, unit: "min", when: (s) => s.slhit_pause_n > 0 },
      {
        key: "slhit_pause_lot_mult",
        label: "hamulec MIĘKKI — mnożnik lota zamiast blokady",
        hint: "0 = hamulec twardy (wejścia zablokowane, jak dotąd). Powyżej zera sygnał WCHODZI, ale mniejszym lotem — wzorzec miękkiego reżimu. Twarda blokada traktuje słabszą przewagę tak samo jak jej brak i może odrzucić zbyt dużo handlu na podstawie jednej wiadomości. Przy limicie ryzyka koszyka (risk_per_basket_pct > 0) mnożnik lota bazowego bywa bez wpływu.",
        type: "num",
        min: 0,
        step: 0.05,
        when: (s) => s.slhit_pause_n > 0,
      },
      {
        key: "signal_filter",
        label: "FILTR JAKOŚCI SYGNAŁU (tagi)",
        hint: 'Opcjonalnie klasyfikuje sygnały według jawnych tagów tekstowych, np. „HIGH RISK TRADE" albo „MAY NOT BE AROUND".',
        type: "bool",
      },
      { key: "skip_tags", label: "pomiń sygnały z tagami", type: "text", wide: true, when: (s) => s.signal_filter },
      { key: "require_tags", label: "wymagaj tagów", type: "text", wide: true, when: (s) => s.signal_filter },
    ],
  },

  /* ================= AUDYT ================= */
  {
    id: "parity",
    title: "Audyt i parytet silnika",
    icon: "microscope",
    desc: "Naprawy błędów wykrytych w forensyce rozjazdów bot ↔ silnik Rust. Wszystkie domyślnie OFF = stare zachowanie.",
    category: "management",
    zakres: "rachunek",
    accent: "var(--ai)",
    fields: [
      {
        key: "commission_per_lot",
        label: "PROWIZJA ZA LOT",
        hint: "Konto Vantage Standard STP ma 0 — koszt siedzi w spreadzie, który jest już w danych tickowych. Konto ECN wymaga wpisania prowizji, inaczej backtest zawyża wynik.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
      },
      {
        key: "exec_latency_ms",
        label: "OPÓŹNIENIE WYKONANIA",
        hint: "Modelowany czas od wiadomości do zlecenia. Zero oznaczałoby, że bot wchodzi po cenie z chwili, w której sygnalista dopiero naciskał wyślij.",
        type: "num",
        min: 0,
        step: 50,
        unit: "ms",
      },
      {
        key: "slippage_pts",
        label: "POŚLIZG zleceń RYNKOWYCH",
        hint: "Zero oznacza tu „NIE WIEMY”, a nie gwarancję braku poślizgu. Wpisz konserwatywną wartość na podstawie wykonania u docelowego brokera.",
        type: "num",
        min: 0,
        step: 0.01,
        unit: "$",
      },
      {
        key: "server_tz_offset_h",
        label: "STREFA CZASOWA SERWERA",
        hint: 'Znaczniki ticków są zapisane w czasie serwera brokera. To pole służy do przeliczenia zegara wiadomości (UTC) na zegar ticków — bez niego silnik dostaje sygnał razem ze strumieniem cen sprzed trzech godzin i „zarabia" na transakcjach, których na żywo nigdy by nie było.',
        type: "num",
        min: -12,
        max: 14,
        step: 1,
        unit: "h",
      },
      {
        key: "msg_clock_offset_h",
        label: "przesunięcie zegara wiadomości",
        hint: "Puste = użyj strefy serwera (poprawne, gdy eksport Telegrama jest w UTC). Wartość jawna przydaje się, gdy źródło wiadomości ma własną strefę.",
        type: "num",
        min: -14,
        max: 14,
        step: 1,
        unit: "h",
      },
      {
        key: "lot_min",
        label: "MINIMALNY LOT",
        type: "num",
        min: 0.01,
        step: 0.01,
        unit: "lot",
      },
      {
        key: "lot_max",
        label: "MAKSYMALNY LOT",
        type: "num",
        min: 0,
        step: 1,
        unit: "lot",
      },
      {
        key: "sim_stops_level",
        label: "STOPS LEVEL (emulacja brokera)",
        hint: "Broker odrzuca SL/TP bliżej niż minimalna odległość ze specyfikacji symbolu. Wpisz wartość docelowego brokera, aby symulacja nie zakładała niewykonalnych poziomów.",
        type: "num",
        min: 0,
        step: 0.05,
        unit: "$",
      },
    ],
  },

  /* ================= AI ================= */
  {
    id: "ai",
    title: "Model AI",
    icon: "brain",
    desc: "Wytrenowana sieć neuronowa BETAZERO przejmuje pełne zarządzanie pozycjami — kierunek nadal pochodzi z sygnału.",
    category: "ai",
    zakres: "rachunek",
    accent: "var(--ai)",
    fields: [
      { key: "ai_model", label: "Model", type: "select", options: [] },
      {
        key: "ai_replaces_management",
        label: "AI ZAMIAST REGUŁ ZARZĄDZANIA",
        hint: "WŁĄCZONE = model prowadzi pozycję sam, a wszystkie reguły zarządzania (drabinka TP, trailing, risk free, strażnicy) przestają działać. To jest zamierzone, ale nazwa „włącz AI” tego nie mówiła — wyglądała na „dodaj AI”, a znaczyła „zdejmij zabezpieczenia”. WYŁĄCZONE = model działa RÓWNOLEGLE z regułami i to jest właściwy tryb, dopóki model nie bije presetów w dolarach.",
        type: "bool",
        warn: (s) =>
          s.ai_replaces_management
            ? "Reguły zarządzania są WYŁĄCZONE — pozycję prowadzi wyłącznie model."
            : null,
      },
      {
        key: "ai_decision_interval_s",
        label: "co ile sekund model podejmuje decyzję",
        hint: "Sieć była trenowana przy określonej kadencji. Zmiana tutaj oznacza, że model widzi rynek inaczej niż podczas treningu.",
        type: "num",
        min: 0.5,
        step: 0.5,
        unit: "s",
      },
    ],
  },

  /* ================= TERMINAL MT5 ================= */
  /* ================= KREDYT BONUSOWY (RACHUNEK) ================= */
  
  {
    id: "kredyt",
    title: "Kredyt bonusowy",
    icon: "wallet",
    desc:
      "MT5 raportuje saldo i kredyt oddzielnie (np. saldo 300 $ + kredyt 300 $ = equity 600 $ bez pozycji). Te pola mówią, " +
      "od czego liczy się LOT; sam kredyt zostaje poduszką marginesową. Trzy liczby " +
      "(saldo / kredyt / podstawa lota) widać na karcie „Lot size”.",
    category: "general",
    zakres: "rachunek",
    accent: "var(--info)",
    fields: [
      {
        key: "credit_balance_separate",
        label: "KREDYT ODDZIELNY OD SALDA (model MT5)",
        hint: "ON: Balance już nie zawiera bonusu, więc nie odejmujemy go drugi raz. Przy odliczaniu Equity pomniejszamy o kredyt, a MinOfBoth liczymy jako min(Balance, Equity minus kredyt). OFF przywraca historyczny model wliczonego kredytu 1:1.",
        type: "bool",
      },
      {
        key: "odlicz_kredyt",
        label: "ODLICZAJ KREDYT BONUSOWY od podstawy lota",
        hint:
          "ON pomija bonus w podstawie lota. W modelu oddzielnego kredytu Balance jest już saldem własnym; odejmowanie dotyczy tylko Equity/MinOfBoth. OFF nie odlicza kredytu od wybranej podstawy. Kredyt nadal stanowi poduszkę dla equity i wolnego depozytu.",
        type: "bool",
      },
      {
        key: "kredyt_reczny",
        label: "kwota kredytu (0 = AUTOMAT z terminala)",
        hint:
          "ZERO ZNACZY AUTOMAT, a nie „kredytu nie ma” — bot bierze wtedy ACCOUNT_CREDIT " +
          "z MT5. Wpisuj kwotę tylko wtedy, gdy terminal jej nie raportuje, i PAMIĘTAJ ją " +
          "wyzerować, gdy broker zdejmie bonus: inaczej bot nadal pomniejszy podstawę Equity " +
          "(w starym modelu również Balance) i może grać za małym lotem. Rozjazd z terminalem panel " +
          "pokaże jako ostrzeżenie.",
        type: "num",
        min: 0,
        step: 50,
        unit: "$",
        when: (s) => s.odlicz_kredyt || s.credit_balance_separate,
      },
    ],
  },

  {
    id: "mt5",
    title: "MetaTrader 5",
    icon: "bolt",
    desc: "Wybierz podążanie za kontem ręcznie wybranym w MT5 albo dotychczasowe logowanie na stałe konto. W trybie podążania terminal musi już działać; bot nie uruchamia go ani nie przelogowuje.",
    category: "general",
    zakres: "rachunek",
    accent: "var(--warn)",
    fields: [
      {
        key: "mt5_follow_terminal_account",
        label: "PODĄŻAJ ZA KONTEM WYBRANYM W MT5",
        hint: "Łączy z działającym terminalem bez przekazywania loginu, serwera i hasła. Po ręcznej zmianie konta most odtwarza połączenie z nową tożsamością. Dobiera XAUUSD / XAUUSD.s; niejednoznaczny terminal lub symbol blokuje handel. Autostart i watchdog są w tym trybie wyłączone.",
        type: "bool",
      },
      {
        key: "mt5_allow_real_account",
        label: "ZEZWALAJ NA REAL w trybie podążania",
        hint: "Włącz wyłącznie świadomie, jeśli chcesz, żeby bot handlował prawdziwymi pieniędzmi na ręcznie wybranym koncie REAL. Wyłączone = tryb podążania nie wysyła zleceń na REAL, nawet po zmianie konta terminala. Nie dotyczy starego trybu stałego loginu.",
        type: "bool",
        when: (s) => s.mt5_follow_terminal_account,
        warn: (s) => s.mt5_allow_real_account ? "Po ręcznym wybraniu konta REAL bot może wysyłać rzeczywiste zlecenia. Sprawdź broker, login, preset i ekspozycję." : null,
      },
      {
        key: "mt5_symbol",
        label: "symbol instrumentu",
        hint: 'Dokładnie tak, jak nazywa go TWÓJ broker w Podglądzie rynku. Bywa "XAUUSD", "XAUUSD.m", "GOLD". Zły symbol = most nie wstanie i bot nie złoży ani jednego zlecenia.',
        type: "text",
        when: (s) => !s.mt5_follow_terminal_account,
      },
      
      {
        key: "mt5_login",
        label: "numer rachunku (login)",
        hint:
          "Bot ODMÓWI handlu, jeśli terminal będzie zalogowany na inne konto — " +
          "czerwony baner + mail zamiast cichej linijki w logu. " +
          "Puste (0) = brak weryfikacji: bot przyjmuje KAŻDE konto, a wskaźnik MT5 " +
          "pokazuje żółte „NIEZWERYFIKOWANE”.",
        type: "num",
        min: 0,
        step: 1,
        when: (s) => !s.mt5_follow_terminal_account,
        warn: (s) =>
          !s.mt5_login
            ? "Bez numeru rachunku bot handluje na KAŻDYM koncie, na które terminal jest zalogowany. Na prawdziwych pieniądzach to scenariusz „myślę, że gram na X, a gram na Y”."
            : null,
      },
      {
        key: "mt5_server",
        label: "serwer brokera",
        hint:
          'Np. "Broker-Demo" albo "Broker-Live". Razem z loginem ' +
          "pozwala sidecarowi zalogować terminal na właściwe konto przy starcie.",
        type: "text",
        when: (s) => !s.mt5_follow_terminal_account,
      },
      {
        key: "mt5_password",
        label: "hasło rachunku (do logowania headless)",
        hint:
          "POLE PODAWCZE: po zapisie wartość trafia do secrets.json (osobny plik, prawa " +
          "tylko dla właściciela) i znika stąd — w settings.json nigdy nie ląduje. " +
          "Puste = zostaw stare / terminal loguje się zapamiętanymi poświadczeniami. " +
          "Potrzebne dopiero, gdy bot ma SAM przełączyć terminal na konto z pola wyżej.",
        type: "text",
        wide: true,
        when: (s) => !s.mt5_follow_terminal_account && s.mt5_login > 0,
      },
      {
        key: "mt5_magic",
        warn: (s) =>
          s.mt5_magic !== 770077
            ? `Magic ${s.mt5_magic} NIE JEST domyślnym numerem CONDUIT-a (770077). Pozycje otwarte innym numerem bot uzna za CUDZE i przestanie nimi zarządzać — zostaną z samym stop-lossem. Zmieniaj tylko świadomie, np. gdy prowadzisz dwie instancje na jednym rachunku.`
            : null,
        label: "numer magiczny",
        hint: "Znacznik, po którym bot rozpoznaje SWOJE zlecenia. Pozycji z innym numerem nie dotyka — dzięki temu można na jednym koncie trzymać handel ręczny albo drugiego bota.",
        type: "num",
        min: 1,
        step: 1,
      },
      {
        key: "mt5_python",
        label: "interpreter Pythona",
        hint: 'Puste = "python" z PATH. Wypełnij pełną ścieżką (np. C:\\Python313\\python.exe), gdy pakiet MetaTrader5 jest zainstalowany w innym interpreterze niż domyślny.',
        type: "text",
        wide: true,
      },
      {
        key: "mt5_deviation_points",
        label: "dopuszczalny poślizg",
        hint: "Ile punktów ceny wolno brokerowi zjechać przy zleceniu rynkowym, zanim je odrzuci. Za mało = odmowy przy szybkim rynku.",
        type: "num",
        min: 0,
        step: 1,
        unit: "pkt",
      },
      {
        key: "mt5_autostart",
        label: "URUCHAMIAJ MT5 PRZY STARCIE",
        hint: "Jeśli terminal nie działa, bot włącza go sam zaraz po uruchomieniu .exe.",
        type: "bool",
        when: (s) => !s.mt5_follow_terminal_account,
      },
      {
        key: "close_receipt_reconcile",
        label: "UZGADNIAJ OPÓŹNIONE ZAMKNIĘCIA",
        hint: "Eksperymentalny, wspólny dla rachunku kontrakt przypisania rozliczeń do koszyków. Zachowuje właściciela pozycji po zamknięciu RPC i uzgadnia późniejsze deale, również partiale. OFF zachowuje dawną ścieżkę. Zmiana wymaga ponownego połączenia mostu. Nie jest to gwarancja pełnych kosztów ani trwałego odtworzenia po restarcie.",
        type: "bool",
        wide: true,
      },
      {
        key: "closed_profit_net_costs",
        label: "ZAMKNIĘTY WYNIK NETTO Z KOSZTAMI — EKSPERYMENT",
        hint: "Domyślnie OFF, wspólna konwencja księgowa rachunku, nie optymalizacja strategii. ON przypisuje do zamkniętej transzy kompletny, podpisany koszt wejścia i wyjścia oraz swap dokładnie raz; nie pobiera kosztów z salda ponownie. Wymaga basket_realized_broker_only=true we wszystkich aktywnych presetach, a dla adaptera MT5 także close_receipt_reconcile=true. Sam widoczny włącznik nie potwierdza gotowości adaptera: kwalifikacja live i trwałego odtwarzania kosztów pozostaje osobną bramką. Nie włączaj na VPS bez jej weryfikacji. OFF zachowuje historyczną, zależną od źródła konwencję profit.",
        type: "bool",
        wide: true,
        warn: (s) => s.closed_profit_net_costs
          ? "Eksperymentalny kontrakt kosztów: bieżący etap służy badaniu w Sim. Zgodność live i restartów nie jest jeszcze potwierdzona; wymagania aktywnych presetów sprawdza osobna bramka."
          : null,
      },
      {
        key: "restore_strategy_continuation",
        label: "ODTWARZANIE ZAMIARÓW STRATEGII — EKSPERYMENT",
        hint: "Domyślnie OFF, wspólna opcja rachunku/runtime, nie parametr szukania zysku. Etap A odtwarza oczekujące zmiany SL/TP, odroczone wyjścia i blokadę dnia po uzgodnieniu rachunku oraz pozycji brokera. Niezgodny lub brakujący stan przy wznowieniu wymaga Review nowych wejść; ochronne zamknięcia pozostają dostępne. Nie odtwarza całej pamięci strategii, nie zapewnia atomowego zapisu ani trwałego księgowania receiptów. Włączenie nie naprawia wstecz starych plików pamięci. OFF zachowuje dawną ścieżkę restartu, w tym jej znane ograniczenia.",
        type: "bool",
        wide: true,
        warn: (s) => s.restore_strategy_continuation
          ? "EKSPERYMENT W WALIDACJI: tylko trzy kontrakty kontynuacji. Brak jeszcze kwalifikacji pełnego restartu live; zwykłe Resume nie zastępuje uzgodnienia niepełnej pamięci."
          : null,
      },
      {
        key: "order_volume_contract_v2",
        label: "ŚCISŁY KONTRAKT WOLUMENU BROKERA",
        hint: "Eksperymentalny, wspólny dla rachunku. ON sprawdza końcowy wolumen min/step/max brokera oraz limit wyznaczony przez strategię i zaokrągla w dół do kroku. Niewykonalny wolumen odrzuca zamiast podnosić ponad limit. Nie wprowadza stałego sufitu lota ani nie wyłącza dynamicznego sizingu. OFF zachowuje dawną ścieżkę.",
        type: "bool",
        wide: true,
      },
      {
        key: "mt5_watchdog",
        label: "PILNUJ I WZNAWIAJ",
        hint: "Wyłączenie zostawia autostart, ale bot przestaje reagować na zamknięcie terminala w trakcie pracy.",
        type: "bool",
        when: (s) => !s.mt5_follow_terminal_account,
      },
      {
        key: "mt5_terminal_path",
        label: "ścieżka do terminal64.exe",
        hint: 'Podążanie za kontem: puste wymaga jednego działającego MT5, a wpisana ścieżka wybiera działający terminal; nigdy go nie uruchamia. Tryb stałego loginu: puste = wykryj instalację automatycznie.',
        type: "text",
        wide: true,
      },
      {
        key: "mt5_retry_attempts",
        label: "prób w jednej serii",
        hint: "Po wyczerpaniu serii bot czeka 1 min, 3 min, 15 min, 30 min, 1 h, 2 h, 4 h, 8 h — i dalej co 8 h. Nigdy się nie poddaje.",
        type: "num",
        min: 1,
        max: 100,
        step: 1,
        when: (s) => s.mt5_watchdog && !s.mt5_follow_terminal_account,
      },
      {
        key: "mt5_retry_delay_s",
        label: "odstęp między próbami",
        type: "num",
        min: 0.5,
        step: 1,
        unit: "s",
        when: (s) => s.mt5_watchdog && !s.mt5_follow_terminal_account,
      },
      {
        key: "mt5_restart_after",
        label: "restart aplikacji po N próbach",
        hint: '1 = pierwsza próba „na sucho”, restart dopiero przed drugą. 0 = nigdy nie restartuj. bot.py restartował terminal przed KAŻDĄ próbą — chwilowa zadyszka kosztowała wtedy pełny cykl zamknij-zabij-uruchom.',
        type: "num",
        min: 0,
        max: 50,
        step: 1,
        when: (s) => s.mt5_watchdog && !s.mt5_follow_terminal_account,
        warn: (s) =>
          s.mt5_restart_after === 0 && s.mt5_watchdog
            ? "Bot nie będzie restartował terminala — zawieszony MT5 zostanie zawieszony."
            : null,
      },
      {
        key: "mt5_health_interval_s",
        label: "co ile sprawdzać stan",
        type: "num",
        min: 1,
        step: 1,
        unit: "s",
        when: (s) => s.mt5_watchdog && !s.mt5_follow_terminal_account,
      },
      
      {
        key: "przerwa_dobowa_od_h",
        label: "przerwa dobowa notowań OD",
        hint:
          "Godzina czasu SERWERA brokera, ułamkowo (1,083 = 01:05). W tym oknie brak " +
          "ticków jest snem rynku, nie awarią: watchdog nie przebudowuje mostu i nie " +
          "wysyła „Utracono/Połączono”. OD równe DO wyłącza przerwę.",
        type: "num",
        min: 0,
        max: 24,
        step: 0.25,
        unit: "h",
      },
      {
        key: "przerwa_dobowa_do_h",
        label: "przerwa dobowa notowań DO",
        hint:
          "Domyślnie 1,083 (= 01:05) — pięć minut zapasu ponad przerwę złota, bo " +
          "notowania po przerwie wracają nierówno. Okno może przechodzić przez północ " +
          "(np. 23,5 → 0,5).",
        type: "num",
        min: 0,
        max: 24,
        step: 0.25,
        unit: "h",
      },
      {
        key: "puls_h",
        label: "PULS: raport życia co N godzin",
        hint:
          "Mail „żyję: konto, saldo, most, sygnały” co N godzin, nawet gdy nic się nie " +
          "dzieje — bo cisza jest nieodróżnialna od zatrzymania bota. Wyciszony, gdy " +
          "rynek stoi (weekend, przerwa " +
          "dobowa); zaległy puls wychodzi po otwarciu. 0 = wyłączony.",
        type: "num",
        min: 0,
        max: 48,
        step: 1,
        unit: "h",
      },
    ],
  },

  /* ================= DZIENNIK ZDARZEN ================= */
  {
    id: "journal",
    title: "Dziennik zdarzeń",
    icon: "clipboard",
    desc:
      "Strumień JSON Lines obok logu tekstowego: jedna linia = jedno zdarzenie, z pełną datą i strefą, " +
      "identyfikatorami (koszyk / zlecenie / wiadomość), migawką stanu przy każdej decyzji i powodem z zamkniętej listy. " +
      "Odpowiada na pytania, na których log poprzedniego bota się wykładał: ile zarobiono danego dnia i która decyzja kosztowała pieniądze.",
    category: "general",
    zakres: "rachunek",
    accent: "var(--accent)",
    fields: [
      {
        key: "journal_enabled",
        label: "ZAPISUJ DZIENNIK (.jsonl)",
        hint:
          "Jeden plik na dobę handlową serwera, nazwa z datą: logs/journal/demo-RRRR-MM-DD.jsonl. " +
          "Analiza: loganaliza logs/journal. Wyłączenie jest nieodwracalne — zdarzeń z przeszłości nie da się odtworzyć.",
        type: "bool",
      },
      {
        key: "journal_min_level",
        label: "najniższy zapisywany poziom",
        hint: "debug zapisuje też wiadomości, które nic nie zmieniły. info to rozsądna baza; warn zostawia same problemy.",
        type: "select",
        options: [
          { value: "debug", label: "debug — wszystko" },
          { value: "info", label: "info — praca bota" },
          { value: "ok", label: "ok — udane akcje i wyżej" },
          { value: "warn", label: "warn — tylko ostrzeżenia i błędy" },
          { value: "error", label: "error — tylko błędy" },
        ],
        when: (s) => s.journal_enabled,
      },
      {
        key: "journal_snapshots",
        label: "MIGAWKA STANU PRZY DECYZJI",
        hint:
          "Bid, ask, spread, equity, saldo, margines, liczba pozycji, suma wolumenu i bieżące obsunięcie w chwili decyzji. " +
          "Bez tego nie da się ocenić, czy pominięcie sygnału było słuszne.",
        type: "bool",
        when: (s) => s.journal_enabled,
      },
      {
        key: "journal_excursions",
        label: "MIERZ WYCHYLENIA CENY (MFE / MAE)",
        hint:
          "Największy zysk i największa strata, jakie pozycja pokazała w trakcie życia. To jedyne źródło raportu " +
          "„ile pieniędzy zostawiono na stole” — bez tego nie wiadomo, która reguła zarządzania kosztuje najwięcej.",
        type: "bool",
        when: (s) => s.journal_enabled,
        warn: (s) =>
          s.journal_enabled && !s.journal_excursions
            ? "Bez pomiaru wychyleń raport „na stole” będzie pusty — nie da się odpowiedzieć, czy dało się zamknąć lepiej."
            : null,
      },
      {
        key: "journal_text_mirror",
        label: "lustrzany plik .log dla oka",
        hint: "To samo zdarzenie w jednej linijce po polsku, obok pliku .jsonl.",
        type: "bool",
        when: (s) => s.journal_enabled,
      },
      {
        key: "journal_retention_days",
        label: "trzymaj dni",
        hint: "0 = nigdy nie kasuj. Liczone po dacie W NAZWIE pliku, nie po czasie modyfikacji — kopiowanie katalogu nie odmładza dób.",
        type: "num",
        min: 0,
        max: 3650,
        step: 1,
        unit: "dni",
        when: (s) => s.journal_enabled,
      },
      {
        key: "archive_retention_days",
        label: "ARCHIWUM WIADOMOŚCI: trzymaj dni",
        hint: "Własna historia kanałów (logs/wiadomosci/*.jsonl): każda wiadomość i KAŻDA JEJ EDYCJA jako osobny wiersz. Eksport z Telegrama tego nie zastąpi — zwija edycje do wersji końcowej ze znacznikiem oryginału i gubi wiadomości skasowane. 0 = nigdy nie kasuj.",
        type: "num",
        min: 0,
        max: 3650,
        step: 30,
        unit: "dni",
        when: (s) => s.journal_enabled,
      },
      {
        key: "journal_buffer_cap",
        label: "sufit bufora zdarzeń",
        hint: "Ile zdarzeń rdzeń może przetrzymać między zrzutami na dysk. Po przekroczeniu najstarsze przepadają — jawnie, z licznikiem.",
        type: "num",
        min: 64,
        max: 500000,
        step: 1000,
        when: (s) => s.journal_enabled,
      },
    ],
  },

  /* ================= PANEL ================= */
  {
    id: "panel",
    title: "Panel i wykonanie",
    icon: "sliders",
    desc: "Zachowanie interfejsu, waluta wyświetlania i częstotliwość pętli bota.",
    category: "general",
    zakres: "rachunek",
    accent: "var(--accent)",
    fields: [
      { key: "one_click", label: "ONE CLICK TRADING", hint: "Panel bez potwierdzeń i alertów.", type: "bool" },
      { key: "display_currency", label: "WALUTA PANELU", hint: "Wartości pieniężne przeliczane niezależnie od MT5; ceny/SL/TP zostają w cenie instrumentu.", type: "select", options: CURRENCIES },
      { key: "poll_ms", label: "Odświeżanie bota", hint: "0 = maksymalna częstotliwość (limit 10 ms).", type: "num", min: 0, step: 50, unit: "ms" },
      { key: "price_tol", label: "TOLERANCJA CEN", hint: "Dopasowywanie poziomów TP/hitów z sygnałów do cen (różni brokerzy = różne spready).", type: "num", min: 0.05, step: 0.05, unit: "$" },
      { key: "show_positions_on_chart", label: "Pokaż pozycje na wykresie", type: "bool" },
      { key: "show_potential_tpsl", label: "SHOW POTENTIAL TP/SL", hint: "Potencjalny zysk/strata przy TP i przy SL — per pozycja, sumy i na wykresie.", type: "bool" },
      { key: "exclude_pending_potential", label: "EXCLUDE PENDING ORDERS z sum", type: "bool", when: (s) => s.show_potential_tpsl },
      {
        key: "comment_mode",
        label: "KOMENTARZ POZYCJI (MT5)",
        type: "select",
        options: [
          { value: "source", label: "nazwa serwera / źródła" },
          { value: "custom", label: "własny tekst" },
        ],
      },
      { key: "comment_include_topic", label: "dodaj nazwę tematu (forum)", type: "bool", when: (s) => s.comment_mode === "source" },
      {
        key: "comment_custom",
        label: "własny komentarz",
        hint: "MAKS. 18 ZNAKÓW. Most dokleja jeszcze sufiks koszyka/poziomu i etykietę źródła, a komentarze MT5 mają ścisły limit całkowitej długości. Tekst ponad budżet może zostać po cichu obcięty albo odrzucony przez API. Puste = znacznik domyślny.",
        type: "text",
        wide: true,
        when: (s) => s.comment_mode === "custom",
        // BUDŻET LICZONY Z GÓRY, a nie obcinanie na końcu. 29 znaków limitu
        // MINUS najgorszy przypadek części maszynowej („12.3t") MINUS
        // etykieta źródła („-panel"). Ostrzeżenie pokazuje LICZBĘ nadmiaru,
        // bo „za długi" bez liczby nie mówi, ile trzeba skrócić.
        warn: (s) => {
          const n = (s.comment_custom ?? "").trim().length;
          const budzet = 29 - 5 - 6;
          return n > budzet
            ? `O ${n - budzet} znaków za długo: komentarz zostanie PO CICHU OBCIĘTY na koncie (budżet to ${budzet} znaków, resztę zajmuje numer koszyka i etykieta źródła).`
            : null;
        },
      },
    ],
  },

  /* ================= WYJSCIA WARUNKOWE =================
     Rodzina `exit_*` trafiala do silnika (settings_map.rs mapuje ja komplet-
     nie), ale panel nie mial dla niej ANI JEDNEJ kontrolki: ustawienie dzialalo,
     a uzytkownik nie mial jak sprawdzic ani zmienic. To ta sama klasa bledu co
     ukryte pozycje — panel milczal o czyms, co naprawde dziala. */
  {
    id: "exitrules",
    title: "Wyjścia warunkowe",
    icon: "log-out",
    desc: "Dodatkowe warunki zamknięcia, niezależne od TP i SL. Wszystkie domyślnie wyłączone (0 = nieaktywne).",
    category: "management",
    zakres: "preset",
    accent: "var(--warn)",
    fields: [
      {
        key: "exit_r_multiple",
        label: "zamknij po osiągnięciu R",
        hint: "Wielokrotność ryzyka (dystans do SL). 2 = zamknij, gdy zysk = 2× ryzyko. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "R",
      },
      {
        key: "exit_min_profit",
        label: "minimalny zysk do wyjścia",
        hint: "Warunki wyjścia nie zadziałają poniżej tej kwoty — chroni przed zamykaniem za darmo.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
      },
      {
        key: "exit_min_hold_min",
        label: "minimalny czas trzymania",
        hint: "Pozycja młodsza niż to nie zostanie zamknięta warunkiem wyjścia.",
        type: "num",
        min: 0,
        step: 1,
        unit: "min",
      },
      {
        key: "exit_on_opposite_signal",
        label: "zamknij na sygnale przeciwnym",
        hint: "Nowy sygnał w drugą stronę zamyka otwarte pozycje poprzedniego kierunku.",
        type: "bool",
      },
      {
        key: "exit_spread_mult",
        label: "wyjdź z zyskownej pozycji przy szerokim spreadzie",
        hint: "Spread ≥ ta krotność mediany uruchamia wyjście z zyskownej pozycji (lub kolejkę exit-via-limit), a NIE blokadę wyjść. Obowiązują exit_min_hold_min, exit_min_profit i hold_after_tp_hit_min. 0 = ta reguła wyłączona.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "×",
      },
      {
        key: "exit_round_dist",
        label: "wyjście przy okrągłej cenie — zasięg",
        hint: "Zamknij, gdy cena podejdzie tak blisko okrągłego poziomu. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "$",
      },
      {
        key: "exit_round_step",
        label: "krok okrągłych poziomów",
        hint: "Co ile dolarów leży „okrągły” poziom (domyślnie co 10).",
        type: "num",
        min: 1,
        step: 1,
        unit: "$",
        when: (s) => s.exit_round_dist > 0,
      },
    ],
  },

  /* ================= SIATKA I PENDINGI — SZCZEGOLY ================= */
  {
    id: "gridextra",
    title: "Siatka i pendingi — szczegóły",
    icon: "grid",
    desc: "Ustawienia decydujące o tym, GDZIE dokładnie ląduje siatka limitów i kiedy pendingi znikają. Różnica między presetem RUNNER a czempionem KRATA siedzi właśnie tutaj.",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "grid_anchor_absolute",
        label: "SIATKA NA KRACIE ABSOLUTNEJ",
        hint: "Limity stawiane na wielokrotnościach kroku (1/PPM), a nie względem ceny sygnału. Kotwiczenie działa tylko przy włączonym kroku; mnożenie zleceń na poziomie kontroluje osobne pole.",
        type: "bool",
      },
      {
        key: "units_per_level",
        label: "krata mnoży zlecenia NA POZIOMIE",
        hint: "Włączone (domyślnie) = na każdym poziomie kraty ląduje `entry_units` zleceń. Wyłączone = jedno zlecenie na poziom, niezależnie od kotwicy. To ustawienie jest niezależne od przełącznika kroku kraty.",
        type: "bool",
        when: (s) => s.grid_anchor_absolute,
      },
      {
        key: "pending_drop_arm",
        label: "uzbrajaj kasowanie pendingów",
        hint: "Pendingi kasowane dopiero po spełnieniu warunku uzbrojenia, a nie natychmiast.",
        type: "bool",
      },
      {
        key: "ml_licz_wiszace",
        label: "licz margines ZE ZLECENIAMI",
        hint: "Poziom marginesu liczony razem z zamrozonym marginesem zlecen oczekujacych, a nie tylko z otwartych pozycji. Bez tego bramki widza stan sprzed wypelnienia siatki - z tym ticki pod 100 % marginesu spadly ze 176 na 21.",
        type: "bool",
      },
      {
        key: "ml_min_wejscie",
        label: "margines min - NOWY KOSZYK (%)",
        hint: "Ponizej tego poziomu marginesu bot nie otwiera nowego koszyka. 0 = bez bramki. Czterdziesci zyskownych pozycji z SL na progu oplacalnosci nie jest problemem; dwie, ktore rozjezdzaja margines, sa.",
        type: "num",
      },
      {
        key: "ml_min_warstwa",
        label: "margines min - KOLEJNA WARSTWA (%)",
        hint: "Ponizej tego poziomu bot nie doklada kolejnych szczebli siatki do koszyka, ktory juz istnieje. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "ml_min_reentry",
        label: "margines min - POWTORNE WEJSCIE (%)",
        hint: "Ponizej tego poziomu bot nie wchodzi ponownie po trafionym celu. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "ml_min_rearm",
        label: "margines min - PRZEZBROJENIE SIATKI (%)",
        hint: "Ponizej tego poziomu bot nie wystawia ponownie niewypelnionych szczebli. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "ml_min_piramida",
        label: "margines min - PIRAMIDA (%)",
        hint: "Ponizej tego poziomu bot nie doklada do pozycji, ktora jest na plusie. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "ml_min_fast_addon",
        label: "margines min - SZYBKA DOKLADKA (%)",
        hint: "Ponizej tego poziomu bot nie robi szybkiej dokladki po gwaltownym wypelnieniu. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "ml_min_relot_up",
        label: "margines min - PODNIESIENIE LOTA (%)",
        hint: "Ponizej tego poziomu bot nie podnosi wolumenu istniejacych zlecen. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "ml_min_drabina",
        label: "margines min - SZCZEBEL DRABINY (%)",
        hint: "Ponizej tego poziomu bot nie stawia kolejnego szczebla drabinki wejscia rynkowego. 0 = bez bramki.",
        type: "num",
      },
      {
        key: "konto_dzwignia",
        label: "dzwignia konta (0 = z brokera)",
        hint: "Dzwignia uzyta do liczenia marginesu. 0 = bierz z rachunku brokera. Ustaw recznie tylko wtedy, gdy broker jej nie podaje - zla wartosc przesuwa WSZYSTKIE bramki marginesowe.",
        type: "num",
      },
      {
        key: "wiek_od_wypelnienia",
        label: "wiek koszyka OD WYPELNIENIA",
        hint: "Zegar wieku koszyka rusza od pierwszego wypelnienia, a nie od powstania siatki. Siatka limitowa moze czekac dobe i wciaz byc wazna - mierzenie jej wieku od publikacji kasuje setupy, ktore nigdy nie dostaly szansy.",
        type: "bool",
      },
      {
        key: "pending_drop_grace_min",
        label: "okno laski przed skasowaniem siatki (min)",
        hint: "Po zdarzeniu celu osiągniętego bez wejścia siatka czeka tyle minut przed skasowaniem. 0 = kasuj natychmiast. Połącz z ochroną odległości, aby ograniczyć stare zlecenia.",
        type: "num",
      },
      {
        key: "pending_drop_grace_max_dist",
        label: "straz odleglosci okna laski ($)",
        hint: "Okno łaski obowiązuje tylko, dopóki cena jest bliżej strefy niż podana odległość; dalej siatka znika od razu. 0 = bez straży.",
        type: "num",
      },
      {
        key: "pending_drop_keep_n",
        label: "zostaw N najplytszych zlecen",
        hint: "Przy kasowaniu siatki zostaw tyle zleceń najbliższych cenie. 0 = kasuj wszystkie. Jest to kompromis między pełnym usunięciem i zachowaniem całej siatki.",
        type: "num",
      },
      {
        key: "pending_drop_on_target",
        label: "kasuj pendingi po osiągnięciu celu",
        hint: "Po zrealizowaniu celu koszyka niewypełnione limity są usuwane.",
        type: "bool",
      },
      {
        key: "pending_drop_require_zone_touch",
        label: "wymagaj dotknięcia strefy",
        hint: "Bez tego warunkiem jest samo osiągnięcie celu, co przy sygnale z podciągnięcia może być prawdą już w chwili wystawienia siatki i skasować oczekujące zlecenia przed wejściem.",
        type: "bool",
        when: (s) => s.pending_drop_on_target,
      },
      {
        key: "tp_open_extra",
        label: "TP także dla dostawionych pozycji",
        hint: "Pozycje dołożone po starcie koszyka również dostają TP.",
        type: "bool",
      },
      {
        key: "hold_after_tp_hit_min",
        label: "wstrzymaj wybrane reguły wyjścia po TP",
        hint: "Po TP czasowo wstrzymuje wyjścia R-multiple, okrągły poziom, spread i smart-exit. NIE blokuje nowych wejść, SL, harvestu ani wszystkich pozostałych wyjść. 0 = bez tej przerwy.",
        type: "num",
        min: 0,
        step: 1,
        unit: "min",
      },
      {
        key: "basket_target_usd",
        label: "cel koszyka",
        hint: "Zamknij cały koszyk po osiągnięciu tego wyniku. 0 = wyłączone.",
        type: "num",
        min: 0,
        step: 1,
        unit: "$",
      },
      {
        key: "toucher_tp_one_based",
        label: "TP touchera liczony od 1",
        hint: "Zmienia indeksowanie TP dla pozycji z touchera (1 = pierwszy TP zamiast zerowego).",
        type: "bool",
      },
      {
        key: "smart_sl_floor_be_after_rf",
        label: "podłoga SL na BE po RISK FREE",
        hint: "Po ogłoszeniu RISK FREE mądry SL nie zejdzie poniżej progu opłacalności.",
        type: "bool",
      },
    ],
  },

  /* ================= RISK FREE JAKO REGULA ================= */
  {
    id: "riskfreerule",
    title: "RISK FREE jako reguła",
    icon: "shield",
    desc: "Koszyk uwalnia się SAM po osiągnięciu progu zysku — bez czekania na komunikat z kanału. To jest automat, a nie reakcja na wiadomość.",
    category: "management",
    zakres: "preset",
    accent: "var(--long)",
    fields: [
      {
        key: "riskfree_enabled",
        label: "RISK FREE AUTOMATYCZNY",
        hint: "Bez tego cała rodzina poniżej nic nie robi. Domyślnie wyłączone: włączenie zmienia strategię, więc ma być świadomą decyzją.",
        type: "bool",
      },
      {
        key: "riskfree_trigger_usd",
        label: "próg w dolarach",
        hint: "Zysk koszyka, przy którym następuje uwolnienie. 0 = ten warunek nieaktywny. Wystarczy, żeby zadziałał którykolwiek z dwóch progów.",
        type: "num",
        min: 0,
        step: 1,
        unit: "$",
        when: (s) => s.riskfree_enabled,
      },
      {
        key: "riskfree_trigger_r",
        label: "próg w R",
        hint: "Ten sam próg jako wielokrotność ryzyka koszyka (odległość do SL). 0 = nieaktywny. Odporniejszy na zmianę lota niż próg w dolarach.",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "R",
        when: (s) => s.riskfree_enabled,
      },
      {
        key: "riskfree_keep_units",
        label: "ile pozycji zostaje runnerem",
        hint: "Reszta koszyka jest zamykana. Runnery to główne źródło zysku w tym systemie — zero znaczy oddanie całego ogona.",
        type: "num",
        min: 0,
        max: 20,
        step: 1,
        when: (s) => s.riskfree_enabled,
      },
      {
        key: "riskfree_runner_stop",
        label: "STOP RUNNERA",
        hint: "Stop na własnym wejściu warstwy zachowuje geometrię schodkowych wejść. Stop na średniej koszyka może znaleźć się nad wejściem głębszego runnera i zamknąć go zbyt wcześnie.",
        type: "select",
        options: [
          { value: "Be", label: "na średniej ważonej koszyka" },
          { value: "BeOwn", label: "na własnym wejściu warstwy" },
          { value: "TrailGap", label: "start na BE, potem zapadka" },
          { value: "Off", label: "bez stopu" },
        ],
        when: (s) => s.riskfree_enabled,
      },
      {
        key: "riskfree_runner_gap",
        label: "luz zapadki",
        hint: "Tylko dla trybu „start na BE, potem zapadka”. Mniejsza wartość ciaśniej chroni zysk, większa daje runnerowi więcej miejsca; wymaga walidacji na niezależnych oknach.",
        type: "num",
        min: 0,
        step: 1,
        unit: "pkt",
        when: (s) => s.riskfree_enabled && s.riskfree_runner_stop === "TrailGap",
      },
      {
        key: "riskfree_be_offset",
        label: "margines stopu runnera",
        hint: "Przesunięcie stopu względem punktu odniesienia. Dodatnie = stop dalej od ceny (luźniej), ujemne = bliżej.",
        type: "num",
        step: 0.1,
        unit: "pkt",
        when: (s) => s.riskfree_enabled && s.riskfree_runner_stop !== "Off",
      },
      {
        key: "riskfree_runner_target",
        label: "CEL RUNNERA",
        type: "select",
        options: [
          { value: "KeepTp", label: "zostaje przy dotychczasowym" },
          { value: "LastTp", label: "najdalszy cel z drabinki" },
          { value: "NextTp", label: "kolejny nietrafiony" },
          { value: "NoTpTrailOnly", label: "bez TP — tylko trailing" },
        ],
        when: (s) => s.riskfree_enabled,
      },
      {
        key: "runner_max_hold_bez_reguly",
        label: "limit trzymania działa TAKŻE bez reguły RISK FREE",
        hint: "Po włączeniu limit czasu obejmuje także koszyki zabezpieczone poleceniem kanału, nawet gdy autonomiczna reguła RISK FREE jest wyłączona.",
        type: "bool",
      },
      {
        key: "riskfree_runner_max_hold_min",
        label: "domknij runnera po",
        hint: "Minuty liczone od chwili zabezpieczenia koszyka, nie od otwarcia pozycji. 0 wyłącza limit czasu.",
        type: "num",
        min: 0,
        max: 1440,
        step: 15,
        unit: "min",
        // M15: pole musi byc widoczne takze przy wylaczonej regule — inaczej
        // nie da sie ustawic minut dla `runner_max_hold_bez_reguly`.
        when: (s) => s.riskfree_enabled || s.runner_max_hold_bez_reguly,
      },
    ],
  },

  /* ================= PARAMETRY Z SYGNALU ================= */
  {
    id: "adaptive",
    title: "Parametry liczone z sygnału",
    icon: "target",
    desc: "Zamiast jednej stałej wartości dla każdego sygnału — wielkości wyprowadzone z szerokości strefy albo z bieżącej zmienności.",
    category: "management",
    zakres: "preset",
    accent: "var(--ai)",
    fields: [
      {
        key: "adaptive_params",
        label: "PARAMETRY ADAPTACYJNE",
        hint: "Włącznik całej rodziny. Bez niego pola poniżej nie działają, nawet ustawione.",
        type: "bool",
      },
      {
        key: "sl_min_dist_zone_mult",
        label: "SL = mnożnik × szerokość strefy",
        hint: "0 = to źródło nieaktywne. Przy dwóch źródłach naraz wygrywa większa wartość.",
        type: "num",
        min: 0,
        step: 0.1,
        when: (s) => s.adaptive_params,
      },
      {
        key: "sl_min_dist_atr_mult",
        label: "SL = mnożnik × zakres z okna",
        hint: "Zakres H−L z okna zastępczego ATR. 0 = nieaktywne.",
        type: "num",
        min: 0,
        step: 0.1,
        when: (s) => s.adaptive_params,
      },
      {
        key: "sl_min_dist_floor",
        label: "podłoga wyliczonego SL",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.adaptive_params,
      },
      {
        key: "sl_min_dist_cap",
        label: "sufit wyliczonego SL",
        hint: "0 = bez sufitu.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.adaptive_params,
      },
      {
        key: "entry_deep_zone_mult",
        label: "głębokie wejście = mnożnik × strefa",
        type: "num",
        min: 0,
        step: 0.1,
        when: (s) => s.adaptive_params,
      },
      {
        key: "entry_units_zone_ref",
        label: "szerokość odniesienia strefy",
        hint: "Liczba szczebli skaluje się jak szerokość sygnału ÷ ta wartość. 0 = bez skalowania.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
        when: (s) => s.adaptive_params,
      },
      {
        key: "adaptive_atr_window_min",
        label: "okno zastępczego ATR",
        type: "num",
        min: 1,
        step: 5,
        unit: "min",
        when: (s) => s.adaptive_params,
      },
      {
        key: "units_by_hour",
        label: "mnożnik szczebli wg godziny",
        hint: "Format „7-11:0.5,15-17:2” — ten sam zapis przedziałów co filtr sesji, z końcem wyłącznym. Mnożniki godzinowe należy walidować na niezależnych okresach.",
        type: "text",
        wide: true,
        when: (s) => s.adaptive_params,
      },
    ],
  },

  /* ================= JAKOSC SYGNALU I BUDZET ================= */
  {
    id: "signalbudget",
    title: "Budżet transakcji i jakość sygnału",
    icon: "filter",
    desc: "Ile sygnałów wolno wziąć na dobę i które w ogóle są tego warte.",
    category: "management",
    zakres: "preset",
    accent: "var(--warn)",
    fields: [
      {
        key: "daily_signal_budget",
        label: "LIMIT KOSZYKÓW NA DOBĘ",
        hint: "0 = bez limitu. Liczone od granicy doby handlowej serwera, nie od północy lokalnej.",
        type: "num",
        min: 0,
        max: 200,
        step: 1,
      },
      {
        key: "signal_min_rr",
        label: "minimalne R:R sygnału",
        hint: "Liczone przy GORSZEJ krawędzi strefy, więc jest to wartość pesymistyczna. 0 = bez filtra.",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "R",
      },
      {
        key: "signal_min_zone_width",
        label: "najwęższa dopuszczalna strefa",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
      },
      {
        key: "signal_max_zone_width",
        label: "najszersza dopuszczalna strefa",
        hint: "0 = bez górnej granicy.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "$",
      },
      {
        key: "entry_weights_from_rr",
        label: "WAGI WOLUMENU Z R:R SZCZEBLA",
        hint: "Zamiast stałej drabinki „wagi wejść” — wolumen proporcjonalny do własnego R:R każdego szczebla.",
        type: "bool",
      },
      {
        key: "entry_weights_rr_power",
        label: "wykładnik nagięcia",
        hint: "0,5 = pierwiastek (łagodnie), 1 = wprost proporcjonalnie, 2 = kwadrat (ostro).",
        type: "num",
        min: 0.1,
        max: 4,
        step: 0.1,
        when: (s) => s.entry_weights_from_rr,
      },
      {
        key: "entry_weights_rr_cap",
        label: "sufit rozpiętości wag",
        hint: "Ile razy największa waga może przewyższać najmniejszą.",
        type: "num",
        min: 1,
        max: 50,
        step: 0.5,
        unit: "×",
        when: (s) => s.entry_weights_from_rr,
      },
    ],
  },

  /* ================= LACZENIE I PRZEZBRAJANIE KOSZYKOW ================= */
  {
    id: "basketmerge",
    title: "Łączenie i przezbrajanie koszyków",
    icon: "layers",
    desc: "Co zrobić, gdy kanał doprecyzowuje sygnał osobną wiadomością albo gdy cena wraca do strefy.",
    category: "management",
    zakres: "preset",
    accent: "var(--info)",
    fields: [
      {
        key: "merge_same_side",
        label: "ŁĄCZ SYGNAŁY TEJ SAMEJ STRONY",
        hint: "Nowy sygnał zgodny kierunkiem i pokrywający się strefą AKTUALIZUJE żywy koszyk zamiast tworzyć drugi. Bez tego doprecyzowanie strefy przez kanał podwaja ekspozycję.",
        type: "bool",
      },
      {
        key: "merge_window_min",
        label: "okno łączenia",
        hint: "Jak stary może być koszyk, żeby uznać nowy sygnał za jego ciąg dalszy.",
        type: "num",
        min: 0,
        step: 5,
        unit: "min",
        when: (s) => s.merge_same_side,
      },
      {
        key: "merge_min_overlap",
        label: "wymagane pokrycie stref",
        hint: "0–1, liczone od WĘŻSZEJ strefy. 0,5 znaczy „połowa węższej strefy leży w szerszej”.",
        type: "num",
        min: 0,
        max: 1,
        step: 0.05,
        when: (s) => s.merge_same_side,
      },
      {
        key: "basket_realized_broker_only",
        label: "wynik koszyka tylko z potwierdzeń brokera",
        hint: "ON księguje wynik zamknięcia wyłącznie z potwierdzonego ledgeru brokera, także dla partiali. Usuwa podwójne dodanie tego samego zysku po ręcznym zamknięciu i późniejszym sync. Wpływa na rearm_min_basket_profit oraz inne reguły wyniku koszyka; nie dodaje zysku do salda konta. Domyślnie OFF zachowuje legacy do porównań.",
        type: "bool",
      },
      {
        key: "confirmed_exit_retry",
        label: "potwierdź domknięcie koszyka i ponawiaj odmowy",
        hint: "ON zapisuje zamiar wyjścia i kończy koszyk dopiero po potwierdzeniu braku jego pozycji i zleceń u brokera. Odmowa zamknięcia/anulowania powoduje ponowienie nie częściej niż co sekundę, także po restarcie z zapisaną pamięcią. Blokuje nowe wejścia i rearm tego koszyka do zakończenia. Dotyczy zarządzanych koszyków, nie rozszerza uprawnień na cudze pozycje. OFF przywraca historyczny mechanizm bez ponawiania.",
        type: "bool",
      },
      {
        key: "entry_edit_geometry_v2",
        label: "EKSPERYMENT: EDYCJE ŹRÓDŁA I PLANU V2",
        hint: "Domyślnie OFF zachowuje historyczne edycje. V2 rozdziela oryginalny sygnał od planu i potwierdzonego wykonania: kosmetyczna edycja nie ma zmieniać SL, TP, wolumenu ani resetować postępu. Rzeczywista zmiana wymaga walidacji rewizji i potwierdzeń brokera. Brak źródłowego snapshotu lub dowodu cancel/fill wymaga jawnego przeglądu, nie zgadywanego odtworzenia siatki. Wdrożenie i testy nadal w toku; nie jest to certyfikat LIVE ani restartu.",
        type: "bool",
        warn: (s) => s.entry_edit_geometry_v2 ? "EKSPERYMENT W WALIDACJI: pełny cancel/fill, restart i natywny parytet pozostają osobnymi bramkami." : null,
      },
      {
        key: "defer_entry_until_receipts",
        label: "ODROCZ PEŁNY ENTRY DO ROZLICZENIA ZAMKNIĘĆ",
        hint: "Eksperymentalne, domyślnie OFF. Wymaga globalnego close_receipt_reconcile i zweryfikowanej sesji brokera. Odroczenie dotyczy pełnego ENTRY podczas tymczasowej bariery rozliczenia — nie komendy NOW. Po ustąpieniu bariery sygnał musi nadal przejść kontrolę ważności. Kolejka jest tylko w RAM danej sesji: restart jej nie odtwarza. Trwały błąd lub nieznany wynik nie powoduje automatycznego ponawiania wejścia. Ustawienie należy do presetu.",
        type: "bool",
      },
      {
        key: "deferred_entry_max_age_s",
        label: "MAKSYMALNY CZAS OD PIERWSZEGO ODBIORU ENTRY",
        hint: "Sekundy od pierwszego odbioru sygnału przez bota (UTC), nie od publikacji Telegram ani zegara brokera. Edycja nie przedłuża czasu. Wymagana skończona wartość większa od zera. 0 nie oznacza braku limitu ani wyłączenia; niepoprawny limit nie upoważnia do opóźnionego wejścia. Pozostałe reguły ważności nadal obowiązują.",
        type: "num",
        min: 1,
        step: 30,
        unit: "s",
        when: (s) => s.defer_entry_until_receipts,
      },
      {
        key: "rearm_grid_on_return",
        label: "PRZEZBRÓJ SIATKĘ PO POWROCIE CENY",
        hint: "Ponowne wystawienie limitów, gdy cena wraca do strefy po wyjściu z niej.",
        type: "bool",
      },
      {
        key: "rearm_keep_empty_alive",
        label: "zachowaj pusty koszyk do powrotu",
        hint: "Gdy pierwsza fala pozycji i zleceń już zniknęła, nie wygaszaj koszyka po 60 s — zachowaj plan, aby legalny powrót ceny mógł przezbroić siatkę. Wyłączone = zachowanie historyczne.",
        type: "bool",
        when: (s) => s.rearm_grid_on_return,
      },
      {
        key: "rearm_block_after_secured",
        label: "nie przezbrajaj po zabezpieczeniu",
        hint: "Blokuje kolejną falę po RISK FREE / SPP. Chroni już zabezpieczony wynik przed ponownym otwarciem ekspozycji.",
        type: "bool",
        when: (s) => s.rearm_grid_on_return,
      },
      {
        key: "spp_blocks_rearm_when_flat",
        label: "SPP na płaskim koszyku blokuje rearm",
        hint: "Jeżeli jawne SECURING PARTIAL PROFITS / DO NOT ENTER AGAIN przyjdzie już po zamknięciu pozycji, zapamiętaj veto dla przyszłego rearmu bez udawania, że pusty koszyk jest zabezpieczoną pozycją.",
        type: "bool",
        when: (s) => s.rearm_grid_on_return,
      },
      {
        key: "rearm_min_basket_profit",
        label: "minimalny wynik koszyka",
        hint: "Dokładamy tylko do koszyka, który już zarabia — dokładanie do stratnego to uśrednianie w dół.",
        type: "num",
        step: 1,
        unit: "$",
        when: (s) => s.rearm_grid_on_return,
      },
      {
        key: "rearm_max_times",
        label: "ile razy na koszyk",
        hint: "0 = bez limitu.",
        type: "num",
        min: 0,
        max: 20,
        step: 1,
        when: (s) => s.rearm_grid_on_return,
      },
      {
        key: "rearm_min_gap_min",
        label: "najkrótszy odstęp",
        type: "num",
        min: 0,
        step: 5,
        unit: "min",
        when: (s) => s.rearm_grid_on_return,
      },
    ],
  },

  /* ================= WYJSCIE LIMITEM ================= */
  {
    id: "exitlimit",
    title: "Wyjście limitem",
    icon: "logout",
    desc: "Wyjścia uznaniowe (harvest, stagnacja, mądre wyjście) czekają na drugą stronę spreadu zamiast płacić go od razu.",
    category: "management",
    zakres: "preset",
    accent: "var(--long)",
    fields: [
      {
        key: "exit_via_limit",
        label: "WYCHODŹ LIMITEM",
        hint: "Dotyczy WYŁĄCZNIE wyjść uznaniowych. Stop loss i cele idą po rynku zawsze — czekanie na lepszą cenę przy SL to sposób na stratę bez dna.",
        type: "bool",
      },
      {
        key: "exit_limit_offset",
        label: "ponad drugą stronę spreadu",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "pkt",
        when: (s) => s.exit_via_limit,
      },
      {
        key: "exit_limit_wait_s",
        label: "po ilu sekundach po rynku",
        hint: "Wyjście awaryjne, gdy limit się nie wypełnił. Bez tego wyjście może nie nastąpić nigdy.",
        type: "num",
        min: 1,
        step: 5,
        unit: "s",
        when: (s) => s.exit_via_limit,
      },
      {
        key: "exit_limit_min_profit",
        label: "poniżej tego zysku wychodź od razu",
        hint: "Pozycja ledwo na plusie nie ma z czego finansować czekania.",
        type: "num",
        min: 0,
        step: 0.5,
        unit: "pkt",
        when: (s) => s.exit_via_limit,
      },
    ],
  },

  /* ================= FILTR TRENDU WYZSZEGO RZEDU ================= */
  {
    id: "trendfilter",
    title: "Filtr trendu wyższego rzędu",
    icon: "trend",
    desc: "Opcjonalny filtr długiego trendu. Może ograniczyć ekspozycję kierunkową, lecz jego wpływ trzeba zweryfikować poza próbką na własnych danych.",
    category: "management",
    zakres: "preset",
    accent: "var(--short)",
    fields: [
      {
        key: "trend_filter_enabled",
        label: "FILTR TRENDU",
        hint: "Odrzuca albo zmniejsza sygnały idące pod trend liczony w długim oknie. To NIE jest to samo co „filtr reżimu”, który patrzy na nachylenie średniej.",
        type: "bool",
      },
      {
        key: "trend_filter_window_h",
        label: "okno odniesienia",
        hint: "24 = doba, 168 = tydzień.",
        type: "num",
        min: 1,
        max: 720,
        step: 1,
        unit: "h",
        when: (s) => s.trend_filter_enabled,
      },
      {
        key: "trend_filter_drop_pct",
        label: "próg zmiany ceny",
        hint: "Od jakiej zmiany w oknie uznajemy trend za przeciwny sygnałowi. 0,5 = „złoto spadło o pół procent”. 0 = filtr nieaktywny mimo włącznika.",
        type: "num",
        min: 0,
        step: 0.1,
        unit: "%",
        when: (s) => s.trend_filter_enabled,
      },
      {
        key: "trend_filter_mode",
        label: "co z sygnałem pod trend",
        hint: "„Mniejszy rozmiar” jest wariantem RÓWNORZĘDNYM, nie zapasowym: twarda blokada przeciw dominującemu kierunkowi sygnałów może wyciąć większość handlu, a wtedy nie mierzymy już samego filtra.",
        type: "select",
        options: [
          { value: "Shrink", label: "wejdź mniejszym rozmiarem" },
          { value: "Block", label: "nie wchodź wcale" },
        ],
        when: (s) => s.trend_filter_enabled,
      },
      {
        key: "trend_filter_shrink",
        label: "mnożnik rozmiaru",
        hint: "0,5 = połowa jednostek.",
        type: "num",
        min: 0.05,
        max: 1,
        step: 0.05,
        unit: "×",
        when: (s) => s.trend_filter_enabled && s.trend_filter_mode === "Shrink",
      },
      {
        key: "basket_max_age_min",
        label: "TWARDY CZAS ŻYCIA KOSZYKA",
        hint: "Po tylu minutach koszyk jest domykany po rynku razem z niezrealizowanymi limitami — także taki, który nigdy nie doszedł do progu RISK FREE. Uzasadnienie: przewaga sygnału żyje około godziny; koszyk trzymany dłużej to już pozycja kierunkowa, a nie realizacja sygnału. Sensowny zakres 15–180. 0 = wyłączone.",
        type: "num",
        min: 0,
        max: 1440,
        step: 15,
        unit: "min",
      },
    ],
  },

  /* ================= MODEL KOSZTOW BROKERA ================= */
  {
    id: "kosztybrokera",
    title: "Model kosztów brokera",
    icon: "coins",
    desc: "To model kosztów brokera, a nie strategia. Wyłączenie swapu może zawyżać backtest; wpisz aktualne wartości podane przez brokera dla danego rachunku i symbolu.",
    category: "management",
    zakres: "rachunek",
    accent: "var(--warn)",
    fields: [
      {
        key: "swap_enabled",
        label: "NALICZAJ SWAP",
        hint: "Naliczaj punkty swapowe brokera przy każdym rolowaniu otwartej pozycji. Wyłączaj tylko w celowym teście izolującym koszty.",
        type: "bool",
      },
      {
        key: "swap_long_points",
        label: "swap pozycji DŁUGIEJ",
        hint: "Ujemne wartości oznaczają koszt. Skopiuj aktualny swap pozycji długiej ze specyfikacji symbolu u docelowego brokera.",
        type: "num",
        step: 0.01,
        unit: "pkt",
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_short_points",
        label: "swap pozycji KRÓTKIEJ",
        hint: "Dodatnie wartości oznaczają przychód, ujemne koszt. Swap pozycji długiej i krótkiej może istotnie się różnić.",
        type: "num",
        step: 0.01,
        unit: "pkt",
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_point_value",
        label: "wartość punktu swapowego",
        hint: "Wartość gotówkowa jednego punktu swapowego dla jednego lota. Odczytaj ją ze specyfikacji brokera albo kontrolowanego testu demo.",
        type: "num",
        min: 0,
        step: 0.01,
        unit: "$",
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_rollover_weekday",
        // ŚWIADOMIE `num`, a nie `select`: kontrolka wyboru zapisuje wartość
        // jako TEKST, a mostek czyta to pole przez `as_f64()`, które na
        // stringu zwraca `None`. Ustawienie wyglądałoby na zapisane i nigdy
        // nie doszłoby do silnika — czyli dokładnie ta klasa błędu, przez
        // którą pięć przełączników w tym panelu nic nie robiło.
        label: "doba WEJŚCIA w potrójne naliczenie (0 = pon., 3 = czw.)",
        hint: "Pole podaje dobę wejścia w potrójne rolowanie, a nie dobę, której finansowanie dotyczy. Przed ręcznym ustawieniem potwierdź konwencję docelowego brokera.",
        type: "num",
        min: 0,
        max: 6,
        step: 1,
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_rollover_mult",
        label: "mnożnik w tym dniu",
        type: "num",
        min: 1,
        max: 7,
        step: 1,
        unit: "×",
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_pomijaj_weekend",
        label: "nie naliczaj nocy sobotniej i niedzielnej",
        hint: "Potrójne rolowanie zwykle rozlicza weekend z góry. Włącz, aby symulator nie naliczał soboty i niedzieli ponownie.",
        type: "bool",
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_rollover_z_serwera",
        label: "dobę rolowania wylicz z wartości serwera",
        hint: "Podaj surowe SYMBOL_SWAP_ROLLOVER3DAYS z terminala; kod sam przeliczy je na wewnętrzną konwencję dnia tygodnia.",
        type: "bool",
        when: (s) => s.swap_enabled,
      },
      {
        key: "swap_rollover3days_mt5",
        label: "SYMBOL_SWAP_ROLLOVER3DAYS z terminala (0 = niedz.)",
        hint: "Surowa wartość MT5, gdzie tydzień liczy się od niedzieli. Czytana tylko przy włączonym przeliczaniu z serwera.",
        type: "num",
        min: 0,
        max: 6,
        step: 1,
        when: (s) => s.swap_enabled && s.swap_rollover_z_serwera,
      },
      {
        key: "runner_ksiegowanie_v2",
        label: "granica doby: poprawione księgowanie i kolejność",
        hint: "Trzy naprawy pętli backtestu naraz. (1) Zamknięcia z granicy doby — nocne SL/TP z luki i cały EodFlat — nie liczyły się do ŻADNEGO dnia, więc tryb dzienny, czyli główne kryterium wyboru presetu, miał zaniżoną liczbę transakcji. (2) EodFlat trafiał do NOWEGO silnika i podbijał mu serię strat, przez co dzień zaczynał się z pauzą za wczorajsze domknięcie. (3) Na granicy doby broker egzekwował PRZED komunikatami nocnymi (odwrotnie niż na zwykłym ticku), a przy resecie dobowym liczył ten sam tick trzy razy w licznikach poziomu marginesu. Wyłączone = liczby zgodne z całym archiwum.",
        type: "bool",
      },
      {
        key: "msg_kurs_sprzed_luki",
        label: "wiadomość nie widzi kursu zza luki",
        hint: "Wiadomość, która przyszła MIĘDZY tickami, dostawała kurs ticku NASTĘPNEGO — czyli w przerwie dobowej i weekendowej decydowała, znając już cenę otwarcia po luce. To wiedza z przyszłości i zawyża backtest; żywy bot robi odwrotnie. Włączenie podaje takim wiadomościom kwotowanie sprzed przerwy.",
        type: "bool",
      },
      {
        key: "slippage_pending_pts",
        label: "poślizg zleceń OCZEKUJĄCYCH",
        hint: "Oczekiwany poślizg wypełnień zleceń oczekujących, w punktach. Zero pozostaw tylko wtedy, gdy potwierdzają to dane wykonania docelowego brokera; w przeciwnym razie wpisz konserwatywną wartość.",
        type: "num",
        min: 0,
        step: 0.01,
        unit: "pkt",
      },
      {
        key: "stop_out_level_pct",
        label: "POZIOM STOP OUT",
        hint: "Poniżej tego poziomu marginesu broker zamyka pozycje SAM — najbardziej stratną pojedynczo, przeliczając poziom po każdej. Przy siatce wielu małych pozycji to zupełnie inny przebieg niż zamknięcie wszystkiego naraz, a dotyczy jedynego progu bezwzględnego: „konto nie może zostać wyzerowane”.",
        type: "num",
        min: 0,
        max: 100,
        step: 1,
        unit: "%",
      },
      {
        key: "margin_call_level_pct",
        label: "poziom wezwania do uzupełnienia",
        hint: "⚠ NIE JEST JESZCZE MODELOWANE. Pole dociera do silnika, ale silnik go dziś nie czyta — bot NIE przestanie otwierać pozycji przy tym poziomie. Stop out (pole wyżej) działa naprawdę. Zostawiamy kontrolkę, bo wartość jest częścią opisu rachunku, ale zmiana tej liczby nie zmieni dziś zachowania bota.",
        type: "num",
        min: 0,
        max: 500,
        step: 1,
        unit: "%",
      },
    ],
  },

  /* ================= WARSTWA EA (AUTO-EA) ================= */
  {
    id: "ea_layer",
    title: "Warstwa EA (AUTO-EA)",
    icon: "robot",
    desc:
      "EA sterowane sygnałami: wejścia, kierunek i punkty szczególne (TP, SL) dają sygnały traderów, " +
      "a warstwa EA prowadzi pozycje z precyzją profesjonalnego EA i reaguje na komunikaty zarządzające " +
      "kanału (BE, partials, close), gdy trzeba. Brak sygnałów = brak handlu — EA nie otwiera niczego " +
      "samo z siebie. Poniżej jest SZKIELET warstwy (EA-CORE): własny zegar niezależny od strumienia " +
      "kwotowań, maszyna Obrona/Neutral/Agresja z dwustronną histerezą i zapadka. Same osie handlowe " +
      "(rodziny A–G) wylądują TUTAJ jako kolejne pola presetu. Dziś szkielet NIE ZMIENIA ANI JEDNEJ " +
      "LICZBY i to jest jego bramka akceptacji, nie brak.",
    category: "management",
    zakres: "preset",
    accent: "var(--accent)",
    fields: [
      {
        key: "ea_enabled",
        label: "WARSTWA EA — WŁĄCZNIK GŁÓWNY",
        hint:
          "Wyłączona = silnik idzie dzisiejszą ścieżką, ani jednej dodatkowej operacji, rachunek nie jest odczytany ani razu więcej. " +
          "Włączona z samymi zerami poniżej = warstwa pracuje (pulsuje, liczy stan konta, stempluje koszyki, prowadzi maszynę stanu), " +
          "ale NIE ZMIENIA ANI JEDNEJ LICZBY — po to, żeby dało się zmierzyć koszt samego szkieletu osobno od kosztu polityk.",
        type: "bool",
      },
      {
        key: "ea_tick_s",
        label: "zegar zarządzania",
        hint:
          "0 = brak własnego zegara: warstwa budzi się z każdym kwotowaniem, czyli tak jak dzisiejsze reguły. " +
          "Powyżej zera włącza kadencję — puls leci, gdy od poprzedniego minęło tyle sekund, NIEZALEŻNIE od tego, czy obudził go tick, czy zegar. " +
          "Po co: poprzedni bot miał regułę wyjścia zamarzniętą 12 h, bo budziła się wyłącznie ze strumienia — a backtest tej klasy błędów NIE WIDZI. " +
          "Cisza kwotowań jest stanem rynku, nie awarią.",
        type: "num",
        min: 0,
        step: 1,
        unit: "s",
        when: (s) => s.ea_enabled,
      },
      {
        key: "ea_state_src",
        label: "źródło sygnału stanu",
        hint: "Z czego liczy się „jak źle jest teraz”. R = suma pierwotnego ryzyka żywych koszyków, więc miara nie zależy od wielkości konta.",
        type: "select",
        options: [
          { value: "FloatR", label: "FloatR — pływający wynik / R koszyków (zalecane)" },
          { value: "FloatPctEquity", label: "FloatPctEquity — pływający wynik jako % equity" },
        ],
        when: (s) => s.ea_enabled,
      },
      {
        key: "ea_defense_enter",
        label: "wejście w OBRONĘ przy stracie ≥",
        hint:
          "0 = OBRONA NIGDY (nie „obrona przy zerowej stracie”). Obrona jest stanem CAŁEGO PORTFELA: strata nigdy nie podnosi ryzyka całości. " +
          "Jednostka zgodna z polem „źródło sygnału stanu”: R albo % equity.",
        type: "num",
        min: 0,
        step: 0.5,
        when: (s) => s.ea_enabled,
      },
      {
        key: "ea_defense_exit",
        label: "wyjście z obrony przy stracie ≤",
        hint: "Musi być MNIEJ dotkliwe niż próg wejścia — to jest cała histereza. Sygnał drgający między progami nie przełącza stanu.",
        type: "num",
        min: 0,
        step: 0.5,
        when: (s) => s.ea_enabled && s.ea_defense_enter > 0,
      },
      {
        key: "ea_offense_enter",
        label: "wejście w AGRESJĘ przy zysku ≥",
        hint:
          "0 = AGRESJA NIGDY. Agresja jest stanem POJEDYNCZEGO KOSZYKA, nie portfela: zwycięzca biegnie sam i nie licencjonuje kolegów. " +
          "To jest bezpośrednie tłumaczenie kanonu Synergy — wygrana nie jest przepustką dla następnego setupu.",
        type: "num",
        min: 0,
        step: 0.5,
        when: (s) => s.ea_enabled,
      },
      {
        key: "ea_offense_exit",
        label: "wyjście z agresji przy zysku ≤",
        hint: "Drugi bok histerezy, tym razem po stronie zysku.",
        type: "num",
        min: 0,
        step: 0.5,
        when: (s) => s.ea_enabled && s.ea_offense_enter > 0,
      },
      {
        key: "ea_state_dwell_s",
        label: "minimalny czas trwania stanu",
        hint:
          "Anty-migotanie. 0 = bez wymogu. ASYMETRIA JEST WBUDOWANA I NIE PODLEGA USTAWIENIU: zaciskanie (wejście w stan bardziej ostrożny) " +
          "działa NATYCHMIAST, luzowanie wymaga przetrzymania warunku przez ten czas. Ochrona nie czeka na zegar.",
        type: "num",
        min: 0,
        step: 30,
        unit: "s",
        when: (s) => s.ea_enabled && (s.ea_defense_enter > 0 || s.ea_offense_enter > 0),
      },
      {
        key: "ea_state_ratchet",
        label: "zapadka stanu",
        hint:
          "Co się dzieje z koszykiem, który powstał w stanie bardziej ostrożnym, gdy portfel się uspokoi. " +
          "Zapadka = koszyk dożywa w swoim stanie; luz dostają dopiero NOWE koszyki. Kasuje się z zamknięciem koszyka.",
        type: "select",
        options: [
          {
            value: "NieLuzujWKoszyku",
            label: "Nie luzuj w otwartym koszyku (zalecane)",
          },
          { value: "Swobodny", label: "Swobodny — stan działa na wszystko [tylko do pomiaru kosztu zapadki]" },
        ],
        when: (s) => s.ea_enabled,
      },
      {
        key: "ea_state_journal",
        label: "dziennik zmian stanu",
        hint: "Każda zmiana stanu z powodem, wartością sygnału i ŹRÓDŁEM pulsu (tick czy zegar). Nie zmienia żadnej decyzji — bufor w pamięci z twardym sufitem.",
        type: "bool",
        when: (s) => s.ea_enabled,
      },
      {
        key: "ea_dozor_sl",
        label: "DOZÓR SL — dostawiaj brakujące stopy",
        hint:
          "Wyłączony (domyślnie) = warstwa tylko LICZY pozycje bez stop-lossa u brokera; ani jedna modyfikacja nie idzie na rachunek. " +
          "Włączony = pozycja bez SL dostaje SL SWOJEGO KOSZYKA. Dozór NIGDY nie wymyśla poziomu ryzyka: pozycja, której koszyk też nie ma stopu, " +
          "zostaje z incydentem, nie z fantazją. Pozycje zamrożone ręczną edycją i bilety bez koszyka są nietykalne. " +
          "Zamyka klasę błędu „późny fill / restart zostawił pozycję bez stopu”.",
        type: "bool",
        when: (s) => s.ea_enabled,
      },
    ],
  },
];

export const MANAGEMENT_GROUPS = SETTINGS_SCHEMA.filter((g) => g.category === "management");
export const GENERAL_GROUPS = SETTINGS_SCHEMA.filter((g) => g.category === "general");
export const AI_GROUP = SETTINGS_SCHEMA.find((g) => g.category === "ai")!;

/** Klucze objete dedykowana kontrolka — reszta trafia do ZAAWANSOWANE. */
export const COVERED_KEYS = new Set<SettingKey>(
  SETTINGS_SCHEMA.flatMap((g) => g.fields.map((f) => f.key)),
);
