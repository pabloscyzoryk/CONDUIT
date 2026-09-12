
use serde::{Deserialize, Serialize};

// ============================================================
//  SLOT — rozłączne numery koszyków między silnikami
// ============================================================

/// Ile numerów koszyków przypada na jeden slot.
///
/// Numer koszyka to `slot * KROK_SLOTU + kolejny`, czyli slot czyta się
/// z góry numeru: `B100001` to pierwszy koszyk slotu 1, `B200003` to trzeci
/// koszyk slotu 2.
///
/// **Dlaczego 100 000, a nie mniej.** Duży krok pozostawia wieloletni zapas
/// numerów dla każdego slotu i zapobiega wejściu jednego slotu w zakres
/// drugiego podczas długiej, ciągłej pracy.
///
/// **Dlaczego nie więcej.** Numer wchodzi do komentarza MT5, gdzie mamy
/// twarde 29 znaków (`conduit_mt5::comment::MAX_COMMENT`). Przy 100 000
/// najdłuższy realny numer ma 7 cyfr.
pub const KROK_SLOTU: u32 = 100_000;

/// Ile slotów w ogóle rozdajemy. Powyżej tej liczby numer koszyka robi się
/// niepotrzebnie długi, a i tak nikt nie prowadzi 40 kanałów sygnałowych.
pub const MAX_SLOTOW: u32 = 40;

pub const SLOT_STARY: u32 = 0;

/// Pierwszy numer koszyka w danym slocie.
///
/// Slot 0 zaczyna od 1 (tak jak przed zmianą), slot `k` od `k * KROK_SLOTU`.
#[inline]
pub fn baza_slotu(slot: u32) -> u32 {
    slot.saturating_mul(KROK_SLOTU)
}

/// Z numeru koszyka na slot. To jest jedyna droga w tę stronę.
#[inline]
pub fn slot_koszyka(id: u32) -> u32 {
    id / KROK_SLOTU
}

/// Pierwszy WOLNY numer w slocie — czyli od czego ma ruszyć licznik silnika.
#[inline]
pub fn pierwszy_numer(slot: u32) -> u32 {
    if slot == SLOT_STARY {
        1
    } else {
        baza_slotu(slot) + 1
    }
}

/// Stały slot wyliczony z NAZWY formatu.
///
/// # Dlaczego wyliczany, a nie nadawany po kolei
///
/// Slot musi być ten sam po restarcie, bo koszyk `B200001` leżący na rachunku
/// znaczy „drugi slot" i nic więcej — bez stałego przypisania po dołożeniu
/// nowego formatu w panelu pozycje przeskoczyłyby do innego silnika. Numer
/// kolejny z listy tego nie daje: lista się zmienia.
///
/// Funkcja skrótu to FNV-1a — nie dlatego, że potrzebujemy kryptografii, tylko
/// dlatego, że jest krótka, deterministyczna i nie zależy od wersji biblioteki
/// standardowej (`DefaultHasher` NIE daje gwarancji stałości między wydaniami
/// Rusta, więc tam ten sam format mógłby po aktualizacji kompilatora dostać
/// inny slot).
///
/// Kolizje są możliwe (39 kubełków) i **nie są zamiatane pod dywan**:
/// [`sloty_formatow`] wykrywa je i rozstrzyga deterministycznie, a warstwa
/// żywa zapisuje o tym ostrzeżenie.
pub fn slot_z_nazwy(nazwa: &str) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in nazwa.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    1 + (h % (MAX_SLOTOW as u64 - 1)) as u32
}

/// Rozdaje sloty KOMPLETOWI formatów, rozstrzygając ewentualne kolizje.
///
/// Zwraca pary `(format, slot)` oraz listę formatów, którym trzeba było slot
/// przesunąć — ta lista ma trafić do dziennika panelu, bo przesunięty slot
/// znaczy, że numeracja koszyków tego formatu zmieni się przy dołożeniu albo
/// usunięciu innego formatu.
///
/// Kolejność wejścia nie ma znaczenia: sortujemy po nazwie, więc dwa wywołania
/// na tym samym zbiorze zawsze dają ten sam wynik.
pub fn sloty_formatow(formaty: &[String]) -> (Vec<(String, u32)>, Vec<String>) {
    let mut nazwy: Vec<String> = formaty.to_vec();
    nazwy.sort();
    nazwy.dedup();

    let mut zajete: Vec<u32> = Vec::new();
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut przesuniete: Vec<String> = Vec::new();

    for n in nazwy {
        let chciany = slot_z_nazwy(&n);
        let mut slot = chciany;
        // liniowe sondowanie w górę, z zawinięciem — deterministyczne
        let mut kroki = 0;
        while zajete.contains(&slot) && kroki < MAX_SLOTOW {
            slot = if slot + 1 >= MAX_SLOTOW { 1 } else { slot + 1 };
            kroki += 1;
        }
        if slot != chciany {
            przesuniete.push(n.clone());
        }
        zajete.push(slot);
        out.push((n, slot));
    }
    (out, przesuniete)
}

// ============================================================
//  OBCE OBCIĄŻENIE — co robią POZOSTAŁE silniki
// ============================================================

/// Fakty o pozostałych silnikach na tym samym rachunku.
///
/// Silnik nie widzi kolegów — widzi tylko własne koszyki i (przez widok
/// brokera) własne pozycje. Pułapy globalne dotyczą jednak CAŁEGO rachunku,
/// więc brakującą połowę obrazu trzeba mu podać z zewnątrz. Robi to warstwa
/// żywa raz na obrót pętli.
///
/// **Same zera = jestem jedynym silnikiem.** Tak wygląda backtest, trening AI
/// i handel jednym formatem — czyli wszystko, co mierzy bramka parytetu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ObceObciazenie {
    /// Unresolved submitted rearm in another account slot. Recomputed by routing;
    /// never a persistent halt and never a restriction on protective exits.
    pub rearm_entry_hold: bool,
    /// otwarte pozycje pozostałych silników (i zlecenia, gdy preset je liczy)
    ///
    /// Osobne pole, choć broker widzi wszystko: silnik dostaje WIDOK
    /// odfiltrowany po slocie, więc `Broker::positions()` mówi mu wyłącznie
    /// o jego własnych. To jest zamierzone — limit presetu ma opisywać ten
    /// format — a pułap łańcucha potrzebuje brakującej reszty.
    pub pozycje: u32,
    /// żywe koszyki pozostałych silników
    pub koszyki: u32,
    pub zrealizowane_dzis: f64,
    /// wolumen BUY i SELL pozostałych silników (loty)
    pub loty_buy: f64,
    pub loty_sell: f64,
    /// czy inny silnik stoi już po stronie przeciwnej do rozważanej
    /// (wypełniane przy sprawdzaniu konkretnego sygnału)
    pub przeciwny_kierunek: bool,
}

// ============================================================
//  USTAWIENIA RACHUNKU KONTRA USTAWIENIA HANDLU
// ============================================================

/// POLA, KTÓRE OPISUJĄ RACHUNEK — wspólne dla wszystkich formatów.
///
/// # Dlaczego ta lista musi istnieć
///
/// Każdy format ma własny preset, a preset to pełny komplet [`crate::Settings`].
/// Gdyby brać go w całości, dwa presety kłóciłyby się o rzeczy, których na
/// jednym rachunku nie da się mieć w dwóch wersjach: dźwignię, koszt swapu,
/// poziom stop-out, opóźnienie realizacji, symbol i magic, adres serwera
/// pocztowego, poziom szczegółowości dziennika. Wygrywałby ten preset, który
/// akurat wczytano jako ostatni — czyli rozjazd bez śladu w logach.
///
/// Dlatego przy składaniu ustawień dla formatu bierzemy **preset formatu**,
/// a POTEM nadpisujemy te pola wartościami z dokumentu panelu, który opisuje
/// rachunek. Wszystko, czego tu nie ma, jest ustawieniem HANDLU i należy do
/// presetu formatu.
///
/// Nazwy są kluczami serde struktury [`crate::Settings`] — test
/// `kazde_pole_ustawien_ma_przypisana_warstwe` pilnuje, żeby każdy klucz
/// istniał i żeby nowe pole nie przeszło niezauważone.
pub const POLA_RACHUNKU: &[&str] = &[
    // One broker receipt/ownership contract for every format on this account.
    // Never allow a leg preset to override the global reconciliation policy.
    "close_receipt_reconcile",
    // One closed-profit convention for all legs; never silently switched by a preset.
    "closed_profit_net_costs",
    // Recovery is account/runtime policy, never a strategy preset override.
    "restore_strategy_continuation",
    // Final broker volume lattice/limit contract is common to all legs.
    "order_volume_contract_v2",
    // ---------- KOSZTY I WARUNKI BROKERA ----------
    // Jeden rachunek = jedna prowizja, jeden swap, jeden poślizg. Dwa presety
    // z różnymi kosztami opisywałyby dwa różne konta.
    "commission_per_lot",
    "swap_enabled",
    "swap_long_points",
    "swap_short_points",
    "swap_point_value",
    "swap_rollover_mult",
    "swap_rollover_weekday",
    // Pakiet D1/D1b: jak liczyć noce weekendowe i skąd brać dobę potrójnego
    // rolowania. To są własności KONTRAKTU U BROKERA, nie strategii — dwie
    // nogi z różną odpowiedzią opisywałyby dwa różne rachunki.
    "swap_pomijaj_weekend",
    "swap_rollover_z_serwera",
    "swap_rollover3days_mt5",
    // Pakiet D5/D6/D4 i D3: wierność SAMEJ PĘTLI backtestu — kolejność
    // zdarzeń na granicy doby i to, jaki kurs widzi wiadomość z przerwy.
    // Opisują świat, w którym mierzymy, a nie sposób handlu; dwie nogi
    // z różną odpowiedzią mierzyłyby dwa różne światy na jednym przebiegu.
    // Ta sama klasa co `sim_margin_at_market` niżej.
    "runner_ksiegowanie_v2",
    "msg_kurs_sprzed_luki",
    "slippage_pts",
    "slippage_pending_pts",
    "stops_level",
    // ---------- CZAS I OPÓŹNIENIA ----------
    "exec_latency_ms",
    "msg_clock_offset_ms",
    "server_tz_offset_ms",
    // ---------- BEZPIECZNIKI SAMEGO RACHUNKU ----------
    // Stop-out i wezwanie do uzupełnienia depozytu ogłasza BROKER, dla całego
    // konta. Format nie ma tu nic do powiedzenia.
    "stop_out_level_pct",
    "margin_call_level_pct",
    "expo_cap_pct",
    // ---------- WIERNOŚĆ SYMULATORA ----------
    // Opisują to, jak zachowuje się broker, a nie jak handlujemy. Rozjazd
    // między formatami znaczyłby, że jeden z nich mierzy inny świat.
    "sim_margin_check_on_fill",
    "sim_validate_pending_stops",
    "sim_margin_at_market",
    "expo_cap_ml_pct",
    "expo_cap_close",
    "expo_cap_s",
    // ---------- TERMINAL MT5 ----------
    "mt5_autostart",
    "mt5_watchdog",
    "mt5_health_interval_s",
    "mt5_restart_after",
    "mt5_retry_attempts",
    "mt5_retry_delay_s",
    "mt5_terminal_path",
    // ---------- DZIENNIK ----------
    // Jeden plik dziennika na proces, więc jeden poziom szczegółowości.
    "journal_enabled",
    "journal_min_level",
    "journal_text_mirror",
    "journal_retention_days",
    "journal_excursions",
    "journal_snapshots",
    "journal_buffer_cap",
    "ai_enabled",
    "ai_model",
    "ai_decision_interval_s",
    "ai_replaces_management",
    // PODSTAWA LOTA (`lot_base`: saldo / equity / mniejsze z dwóch) należy
    // tutaj z tego samego powodu co kredyt: mówi, OD CZEGO liczy się wolumen,
    // a to jest własność rachunku. Dwie nogi z różną podstawą liczyłyby lot
    // z dwóch różnych kwot na jednym koncie.
    "lot_base",
    // ---------- KREDYT BONUSOWY ----------
    // Bonus daje BROKER, jednemu rachunkowi, jedną kwotą. Dwa formaty z dwiema
    // różnymi wartościami opisywałyby dwa różne konta — a jest jedno. Gdyby te
    // pola były per format, jeden silnik liczyłby lot od 300 $, a drugi od
    // 600 $ na tym samym saldzie, i nikt by nie wiedział, dlaczego wolumeny
    // się nie zgadzają.
    "odlicz_kredyt",
    "credit_balance_separate",
    "kredyt_reczny",
    // ---------- DŹWIGNIA RACHUNKU ----------
    //
    // Mówi, jaką dźwignię przyjąć do liczenia marginesu, gdy broker jej nie
    // podaje. Rachunek jest jeden i dźwignia jest jedna — dwie nogi z różnymi
    // wartościami liczyłyby margines tego samego konta dwoma różnymi wzorami.
    "konto_dzwignia",
];

// day_trail_basis is strategy-owned, like the existing day trail percentages.
// Profit budget arm/keep/deploy are per-strategy risk axes, like day trail.
// Nested t100 is strategy configuration owned by its preset, not account overlay.
pub const LICZBA_POL_USTAWIEN: usize = 521;

/// Składa ustawienia dla JEDNEGO formatu.
///
/// Bierze `preset` formatu i nadpisuje w nim [`POLA_RACHUNKU`] wartościami
/// z `rachunek` — czyli z tego kompletu, który opisuje konto (dziś: dokument
/// panelu przepuszczony przez `settings_map::core_from_ui`).
///
/// # Dlaczego przez JSON, a nie polem po polu
///
/// [`crate::Settings`] ma ponad trzysta pól. Ręczne przepisywanie trzydziestu
/// z nich byłoby listą, która rozjeżdża się z [`POLA_RACHUNKU`] przy pierwszej
/// zmianie nazwy — a rozjazd byłby cichy. Tędy lista nazw jest JEDYNYM
/// źródłem prawdy i literówka wychodzi w teście, nie na rachunku.
///
/// Wołane przy zmianie ustawień (nie częściej niż raz na 2 s), więc koszt
/// serializacji nie ma znaczenia.
pub fn ustawienia_formatu(preset: &crate::Settings, rachunek: &crate::Settings) -> crate::Settings {
    let (cfg, blad) = ustawienia_formatu_z_diagnoza(preset, rachunek);
    if let Some(b) = blad {
        // Rdzeń nie ma dziennika (journal żyje w silniku), a cisza była tu
        // groźniejsza niż stderr: noga grała dźwignią/swapem/stop-outem
        // z PLIKU PRESETU zamiast z konta i nie zostawał po tym żaden ślad
        // (audyt wielosilnik.rs:620).
        eprintln!(
            "ustawienia_formatu: scalenie pól rachunku NIE weszło, noga gra \
             SAMYM presetem (dźwignia/swap/stop-out z pliku, nie z konta): {b}"
        );
    }
    cfg
}

/// Jak [`ustawienia_formatu`], ale z diagnozą zamiast samego stderr.
///
/// `Some(opis)` znaczy, że scalenie się NIE powiodło i zwrócony komplet to
/// SAM preset — wszystkie [`POLA_RACHUNKU`] pochodzą wtedy z pliku nogi,
/// nie z konta. Wołający, który ma własny dziennik, powinien to zapisać.
pub fn ustawienia_formatu_z_diagnoza(
    preset: &crate::Settings,
    rachunek: &crate::Settings,
) -> (crate::Settings, Option<String>) {
    // Nieosiągalne przy poprawnym `Serialize`, ale cisza jest zakazana:
    // wolę oddać preset bez nadpisania niż udawać, że się udało.
    let mut p = match serde_json::to_value(preset) {
        Ok(v) => v,
        Err(e) => return (preset.clone(), Some(format!("serializacja presetu: {e}"))),
    };
    let r = match serde_json::to_value(rachunek) {
        Ok(v) => v,
        Err(e) => return (preset.clone(), Some(format!("serializacja rachunku: {e}"))),
    };
    let Some(ro) = r.as_object() else {
        return (
            preset.clone(),
            Some("rachunek nie serializuje się do obiektu".into()),
        );
    };
    let Some(po) = p.as_object_mut() else {
        return (
            preset.clone(),
            Some("preset nie serializuje się do obiektu".into()),
        );
    };
    for k in POLA_RACHUNKU {
        if let Some(v) = ro.get(*k) {
            po.insert((*k).to_string(), v.clone());
        }
    }
    match serde_json::from_value(p) {
        Ok(s) => (s, None),
        Err(e) => (
            preset.clone(),
            Some(format!("deserializacja po scaleniu: {e}")),
        ),
    }
}

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn t100_is_preset_strategy_not_account_overlay() {
        let mut preset=crate::Settings::default();preset.t100.enabled=true;preset.t100.risk_pct=2.5;
        let mut account=crate::Settings::default();account.t100.risk_pct=0.5;
        assert!(!POLA_RACHUNKU.contains(&"t100"));
        assert_eq!(ustawienia_formatu(&preset,&account).t100,preset.t100);
    }

    #[test]
    fn slot_zero_zachowuje_sie_jak_przed_zmiana() {
        // Cała gwarancja parytetu stoi na tej jednej równości: silnik bez
        // przypisanego slotu numeruje koszyki dokładnie tak, jak numerował
        // przed wprowadzeniem wielu silników.
        assert_eq!(pierwszy_numer(SLOT_STARY), 1);
        assert_eq!(slot_koszyka(1), SLOT_STARY);
        assert_eq!(slot_koszyka(99_999), SLOT_STARY);
    }

    #[test]
    fn numery_slotow_sa_rozlaczne() {
        for a in 1..MAX_SLOTOW {
            for b in 1..MAX_SLOTOW {
                if a == b {
                    continue;
                }
                // najwyższy numer slotu `a` leży poniżej najniższego slotu `a+1`
                let ostatni_a = baza_slotu(a) + KROK_SLOTU - 1;
                let pierwszy_b = pierwszy_numer(b);
                assert!(
                    slot_koszyka(ostatni_a) != slot_koszyka(pierwszy_b),
                    "sloty {a} i {b} dzielą numerację"
                );
            }
        }
        assert_eq!(slot_koszyka(pierwszy_numer(1)), 1);
        assert_eq!(slot_koszyka(pierwszy_numer(7)), 7);
        assert_eq!(slot_koszyka(baza_slotu(7) + KROK_SLOTU - 1), 7);
    }

    #[test]
    fn slot_z_nazwy_jest_staly_i_w_zakresie() {
        for n in ["ATFX", "Synergy", "Cokolwiek", "", "ą"] {
            let s = slot_z_nazwy(n);
            assert!(
                (1..MAX_SLOTOW).contains(&s),
                "slot {s} dla {n:?} poza zakresem"
            );
            assert_eq!(s, slot_z_nazwy(n), "ta sama nazwa musi dawać ten sam slot");
        }
        // ATFX i Synergy nie mogą wpaść na siebie — to jest para, z którą
        // startujemy, więc kolizja tutaj byłaby widoczna od pierwszego dnia.
        assert_ne!(slot_z_nazwy("ATFX"), slot_z_nazwy("Synergy"));
    }

    #[test]
    fn rozdanie_slotow_nie_zalezy_od_kolejnosci_wejscia() {
        let a = sloty_formatow(&["ATFX".into(), "Synergy".into()]).0;
        let b = sloty_formatow(&["Synergy".into(), "ATFX".into()]).0;
        assert_eq!(a, b);
        // powtórka tej samej nazwy nie zjada drugiego slotu
        let (c, _) = sloty_formatow(&["ATFX".into(), "ATFX".into()]);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn kolizja_slotow_jest_rozstrzygana_i_zglaszana() {
        // Szukamy dwóch nazw o tym samym slocie — przy 39 kubełkach para
        // znajduje się od ręki. Test nie zakłada, KTÓRE to nazwy, żeby nie
        // przywiązywać się do konkretnej funkcji skrótu.
        let mut nazwy: Vec<String> = Vec::new();
        for i in 0..200 {
            nazwy.push(format!("F{i}"));
        }
        let (pary, przesuniete) = sloty_formatow(&nazwy[..MAX_SLOTOW as usize - 1]);
        let mut sloty: Vec<u32> = pary.iter().map(|(_, s)| *s).collect();
        sloty.sort_unstable();
        let ile = sloty.len();
        sloty.dedup();
        assert_eq!(
            sloty.len(),
            ile,
            "sloty muszą być rozłączne mimo kolizji skrótów"
        );
        assert!(
            !przesuniete.is_empty(),
            "przy 39 nazwach kolizja musi wystąpić i zostać zgłoszona"
        );
    }

    /// LISTA POLA_RACHUNKU MUSI OPISYWAĆ ISTNIEJĄCE POLA.
    ///
    /// Literówka w nazwie znaczyłaby, że pole rachunku po cichu wraca do bycia
    /// polem presetu — a wtedy dwa formaty dostają dwie różne dźwignie na
    /// jednym koncie i nikt się o tym nie dowie.
    #[test]
    fn pola_rachunku_istnieja_w_ustawieniach() {
        let v = serde_json::to_value(crate::Settings::default()).unwrap();
        let o = v.as_object().expect("Settings serializuje się do obiektu");
        for k in POLA_RACHUNKU {
            assert!(
                o.contains_key(*k),
                "POLA_RACHUNKU wskazuje nieistniejące pole `{k}`"
            );
        }
    }

    #[test]
    fn canonical_net_account_owner_does_not_silently_enable_strategy_dependency() {
        assert!(POLA_RACHUNKU.contains(&"closed_profit_net_costs"));
        for enabled in [false,true] {
            let mut preset=crate::Settings::default();
            let mut account=crate::Settings::default();
            preset.closed_profit_net_costs=!enabled;
            preset.basket_realized_broker_only=false;
            account.closed_profit_net_costs=enabled;
            account.basket_realized_broker_only=true;
            let merged=ustawienia_formatu(&preset,&account);
            assert_eq!(merged.closed_profit_net_costs,enabled);
            assert!(!merged.basket_realized_broker_only,"invalid dependency needs an explicit guard, not silent strategy mutation");
        }
    }

    #[test]
    fn entry_edit_and_exact_sr_warmup_are_strategy_owned() {
        for enabled in [false,true] {
            let mut preset=crate::Settings::default();let mut account=crate::Settings::default();
            preset.entry_edit_geometry_v2=enabled;preset.sr_warmup_exact_ticks=enabled;
            account.entry_edit_geometry_v2=!enabled;account.sr_warmup_exact_ticks=!enabled;
            let merged=ustawienia_formatu(&preset,&account);
            assert_eq!(merged.entry_edit_geometry_v2,enabled);
            assert_eq!(merged.sr_warmup_exact_ticks,enabled);
        }
        assert!(!POLA_RACHUNKU.contains(&"entry_edit_geometry_v2"));
        assert!(!POLA_RACHUNKU.contains(&"sr_warmup_exact_ticks"));
    }

    /// KAŻDE NOWE POLE USTAWIEŃ WYMAGA ŚWIADOMEJ DECYZJI O WARSTWIE.
    ///
    /// Test celowo pilnuje LICZBY pól, a nie ich spisu: spis 306 nazw byłby
    /// nieczytelny i nikt by go nie czytał, a licznik wywala się dokładnie
    /// wtedy, gdy ktoś dokłada pole — czyli w jedynym momencie, w którym
    /// pytanie „rachunek czy handel" ma sens.
    #[test]
    fn nowe_pole_ustawien_wymaga_decyzji_o_warstwie() {
        let v = serde_json::to_value(crate::Settings::default()).unwrap();
        let ile = v.as_object().unwrap().len();
        assert_eq!(
            ile, LICZBA_POL_USTAWIEN,
            "liczba pól Settings zmieniła się z {LICZBA_POL_USTAWIEN} na {ile}.\n\
             Rozstrzygnij, do której warstwy należy nowe pole:\n\
             • opisuje RACHUNEK (broker, terminal, zegar, dziennik, karta lota) \
             -> dopisz do POLA_RACHUNKU,\n\
             • opisuje HANDEL danym formatem (strefy, cele, trailing, filtry) \
             -> nic nie rób, to jest domyślna warstwa,\n\
             a potem popraw LICZBA_POL_USTAWIEN."
        );
    }

    /// Pola rachunku nie mogą się dublować — powtórka znaczyłaby, że ktoś
    /// dopisał to samo w dwóch sekcjach i jedna z nich jest nieaktualna.
    #[test]
    fn lista_pol_rachunku_nie_ma_powtorek() {
        let mut v: Vec<&str> = POLA_RACHUNKU.to_vec();
        let ile = v.len();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), ile, "POLA_RACHUNKU zawiera powtórzone nazwy");
    }

    /// SEDNO PODZIAŁU: pole rachunku idzie z konta, pole handlu z presetu.
    #[test]
    fn ustawienia_formatu_biora_rachunek_z_konta_a_handel_z_presetu() {
        let mut preset = crate::Settings::default();
        let mut rachunek = crate::Settings::default();

        // RACHUNEK — musi wygrać wartość z konta
        preset.commission_per_lot = 7.0;
        rachunek.commission_per_lot = 0.0;
        preset.exec_latency_ms = 5000;
        rachunek.exec_latency_ms = 120;

        // HANDEL — musi wygrać wartość z presetu
        preset.max_open_positions = 9;
        rachunek.max_open_positions = 3;
        preset.trail_start = 42.0;
        rachunek.trail_start = 1.0;

        let s = ustawienia_formatu(&preset, &rachunek);
        assert_eq!(s.commission_per_lot, 0.0, "koszt brokera opisuje RACHUNEK");
        assert_eq!(s.exec_latency_ms, 120, "opóźnienie opisuje RACHUNEK");
        assert_eq!(
            s.max_open_positions, 9,
            "limit pozycji opisuje HANDEL formatem"
        );
        assert_eq!(s.trail_start, 42.0, "trailing opisuje HANDEL formatem");
    }

    #[test]
    fn karta_lota_nie_nadpisuje_lota_presetu() {
        let mut preset = crate::Settings::default();
        let mut rachunek = crate::Settings::default();

        // preset nogi: FRESHQUEEN-5 (procent 0,5 z sufitem 0,01)
        preset.lot_mode_percent = true;
        preset.lot_percent = 0.5;
        preset.lot_fixed = 0.01;
        preset.lot_max = 0.01;
        preset.lot_min = 0.01;
        preset.lot_scale_step = 0.0;
        // dokument panelu: HYPER-2 + karta lota ustawiona na coś innego
        rachunek.lot_mode_percent = false;
        rachunek.lot_percent = 5.0;
        rachunek.lot_fixed = 1.0;
        rachunek.lot_max = 100.0;
        rachunek.lot_scale_step = 250.0;

        let s = ustawienia_formatu(&preset, &rachunek);
        assert!(s.lot_mode_percent, "tryb lota należy do PRESETU nogi");
        assert_eq!(s.lot_percent, 0.5, "procent lota należy do PRESETU nogi");
        assert_eq!(s.lot_fixed, 0.01, "stały lot należy do PRESETU nogi");
        assert_eq!(s.lot_max, 0.01, "sufit lota należy do PRESETU nogi");
        assert_eq!(
            s.lot_scale_step, 0.0,
            "auto-skalowanie należy do PRESETU nogi"
        );

        for k in [
            "lot_mode_percent",
            "lot_fixed",
            "lot_percent",
            "lot_min",
            "lot_max",
        ] {
            assert!(
                !POLA_RACHUNKU.contains(&k),
                "`{k}` wróciło do POLA_RACHUNKU — dokument panelu znów nadpisze lot nogi"
            );
        }
    }

    /// Diagnoza scalenia: zdrowy komplet NIE zgłasza błędu, a wynik jest
    /// identyczny z tym, co oddaje wersja bez diagnozy — obie ścieżki muszą
    /// zostać jedną funkcją, inaczej rozjadą się przy pierwszej poprawce.
    #[test]
    fn diagnoza_scalenia_milczy_na_zdrowych_ustawieniach() {
        let mut preset = crate::Settings::default();
        let mut rachunek = crate::Settings::default();
        preset.commission_per_lot = 7.0;
        rachunek.commission_per_lot = 0.0;

        let (s, blad) = ustawienia_formatu_z_diagnoza(&preset, &rachunek);
        assert!(
            blad.is_none(),
            "zdrowe ustawienia nie mają prawa zgłosić błędu: {blad:?}"
        );
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::to_value(&ustawienia_formatu(&preset, &rachunek)).unwrap(),
            "wersja z diagnozą i bez muszą oddawać ten sam komplet"
        );
    }

    /// Ten sam komplet po obu stronach musi wyjść bez zmiany — inaczej sama
    /// runda przez JSON gubiłaby pola i pojedynczy format handlowałby innymi
    /// ustawieniami niż preset, który wskazał użytkownik.
    #[test]
    fn skladanie_nie_gubi_ani_jednego_pola() {
        let p = crate::Settings::default();
        let s = ustawienia_formatu(&p, &p);
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::to_value(&p).unwrap(),
            "runda przez JSON zmieniła ustawienia"
        );
    }
}
