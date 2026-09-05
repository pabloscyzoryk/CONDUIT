
use conduit_core::formaty::PulapyGlobalne;
use conduit_core::settings::{
    PendingLifetime, RiskFreeMode, RiskFreeRunnerStop, RiskFreeRunnerTarget, Settings, TrailMode,
};

/// Jedno ostrzeżenie bramki.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Niespojnosc {
    pub kod: &'static str,
    /// Pola, które się kłócą — wypisane po nazwach RDZENIA.
    pub pola: &'static str,
    /// Pełne zdanie do dziennika i do panelu, z powodem i liczbą.
    pub opis: String,
}

impl std::fmt::Display for Niespojnosc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {} — {}", self.kod, self.pola, self.opis)
    }
}

pub fn sprawdz(c: &Settings) -> Vec<Niespojnosc> {
    let mut v = Vec::new();
    let mut zglos = |kod: &'static str, pola: &'static str, opis: String| {
        v.push(Niespojnosc { kod, pola, opis });
    };

    if matches!(
        c.risk_free_mode,
        RiskFreeMode::CloseAllKeepNearest
            | RiskFreeMode::CloseAllKeepBest
            | RiskFreeMode::MoveSlToBeOnly
    ) && matches!(
        c.risk_free_runner_target,
        RiskFreeRunnerTarget::NoTpTrailOnly
    ) && matches!(c.trail_runner_mode, TrailMode::Off)
    {
        zglos(
            "S1-runner-bez-wyjscia",
            "risk_free_runner_target + trail_runner_mode",
            format!(
                "`risk_free_runner_target = NoTpTrailOnly` zdejmuje runnerowi CEL, \
                 a `trail_runner_mode = Off` zdejmuje mu ZAPADKĘ — pozycja zostaje \
                 bez jednego i bez drugiego. `risk_free_trail = {}` tego NIE ratuje: \
                 gałąź trailingu po RISK FREE wymaga `trail_runner_mode` różnego od \
                 `Off`. Wyjściem zostaje wyłącznie SL (przy `risk_free_mode = {:?}`) \
                 albo twardy wiek koszyka (`basket_max_age_min = {}`; 0 = brak).",
                c.risk_free_trail, c.risk_free_mode, c.basket_max_age_min
            ),
        );
    }
    if c.riskfree_enabled
        && matches!(
            c.riskfree_runner_target,
            RiskFreeRunnerTarget::NoTpTrailOnly
        )
        && matches!(c.riskfree_runner_stop, RiskFreeRunnerStop::Off)
        && c.riskfree_runner_max_hold_min <= 0.0
    {
        zglos(
            "S1-runner-bez-wyjscia",
            "riskfree_runner_target + riskfree_runner_stop + riskfree_runner_max_hold_min",
            "reguła RISK FREE daje runnerowi `NoTpTrailOnly` (bez celu) i `Off` \
             (bez stopu), a `riskfree_runner_max_hold_min = 0` znaczy „nigdy nie \
             domykaj po czasie”. Taki runner nie ma ANI JEDNEGO warunku wyjścia \
             — zostaje na rachunku do końca życia koszyka."
                .to_string(),
        );
    }

    // ---------- S2. BE, KTÓREGO NIE DA SIĘ POSTAWIĆ ----------
    //
    // BE-lock uzbraja się przy zysku `be_lock_pts` i stawia stop na
    // `wejście ± be_offset` (engine.rs, gałąź „BE-lock"). Żeby broker taki
    // stop przyjął, musi on leżeć co najmniej `stops_level` od ceny — czyli
    // warunkiem KONIECZNYM jest `be_lock_pts − be_offset >= stops_level`.
    //
    // ⚠ CO SIĘ DZIEJE PRZY PRZECIWNEJ NIERÓWNOŚCI — dokładnie, bo łatwo tu
    // przesadzić. Gałąź siedzi w pętli po pozycjach i próbuje na KAŻDYM
    // ticku, dopóki `pts >= be_lock_pts`. Stop nie stanie w chwili
    // uzbrojenia, ale stanie później — gdy zysk dojdzie do
    // `be_offset + stops_level`. Skutek jest więc taki: PRÓG Z PANELU NIE
    // OBOWIĄZUJE, obowiązuje próg o tyle wyższy; a jeśli cena nigdy tak
    // daleko nie zajdzie, BE nie stanie ani razu. Nierówność jest ostra
    // (`>`), bo `sl_is_valid` przepuszcza stop leżący DOKŁADNIE
    // `stops_level` od ceny (`sl <= bid − stops_level`).
    //
    // ⚠ ZAKRES: pełna wersja tego sprawdzenia („`be_offset` większy niż
    // dystans do OSTATNIEGO celu") wymaga celów z sygnału, których bramka
    // przed startem nie ma. Sprawdzamy tę część, którą da się rozstrzygnąć
    // z samych ustawień; wariant `be_at_tp1` (BE po trafionym TP1) zależy od
    // geometrii konkretnego koszyka i tu nie jest zgłaszany.
    if c.be_lock_pts > 0.0 && c.be_offset > c.be_lock_pts - c.stops_level {
        zglos(
            "S2-be-nieosiagalny",
            "be_offset + be_lock_pts + stops_level",
            format!(
                "BE-lock uzbraja się przy zysku {:.2}, a stop ma stanąć {:.2} nad \
                 wejściem — zostaje {:.2} zapasu przy wymaganych przez brokera {:.2} \
                 (`stops_level`). W chwili uzbrojenia stop leży za blisko ceny \
                 (albo nad nią) i broker odrzuca modyfikację. Gałąź próbuje dalej \
                 na każdym ticku, więc BE stanie dopiero przy zysku {:.2} — o {:.2} \
                 wyżej, niż mówi próg z panelu — a jeśli cena tak daleko nie \
                 zajdzie, nie stanie ani razu.",
                c.be_lock_pts,
                c.be_offset,
                c.be_lock_pts - c.be_offset,
                c.stops_level,
                c.be_offset + c.stops_level,
                c.be_offset + c.stops_level - c.be_lock_pts
            ),
        );
    }

    // ---------- S3. STRAŻ EKSPOZYCJI, KTÓRA NIC NIE ZAMYKA ----------
    //
    // `redukuj_ekspozycje` ma dwa kroki: (a) kasuje leżące szczeble,
    // (b) domyka pozycje — ale (b) TYLKO przy `expo_cap_close`. Gdy próg
    // przekracza sama księga POZYCJI (a tak wygląda każdy wybuch: siatka
    // wypełniła się cała, leżących zleceń już nie ma), krok (a) nie ma czego
    // skasować, licznik `expo_niedosyt` rośnie i funkcja wraca bez działania.
    // Straż jest wtedy miernikiem, nie polisą.
    if c.expo_cap_pct > 0.0 && !c.expo_cap_close {
        zglos(
            "S3-straz-nic-nie-zamyka",
            "expo_cap_pct + expo_cap_close",
            format!(
                "`expo_cap_pct = {}` przy `expo_cap_close = false` potrafi wyłącznie \
                 KASOWAĆ leżące szczeble. Po wypełnieniu całej siatki — czyli \
                 dokładnie wtedy, gdy próg wiąże — nie ma już czego kasować \
                 i straż nie robi nic (rośnie tylko licznik `expo_niedosyt`). \
                 Domykaniem pozycji po poziomie marginesu zajmuje się \
                 `expo_cap_ml_pct` (teraz {}).",
                c.expo_cap_pct, c.expo_cap_ml_pct
            ),
        );
    }

    let siatka_dlugowieczna =
        !matches!(c.pending_lifetime, PendingLifetime::UntilTp1) || c.basket_max_age_min <= 0.0;
    if c.reenter_after_tp && c.reenter_max == 0 && siatka_dlugowieczna {
        zglos(
            "S4-usrednianie-bez-hamulca",
            "reenter_max + reenter_after_tp + pending_lifetime + basket_max_age_min",
            format!(
                "`reenter_max = 0` NIE znaczy „bez re-entry”, tylko BEZ LIMITU \
                 (wyłącznikiem jest `reenter_after_tp`). Przy \
                 `pending_lifetime = {:?}` i `basket_max_age_min = {}` (0 = koszyk \
                 bez twardego wieku) nic nie ogranicza liczby dokładek do jednego \
                 koszyka. Ustaw skończony limit albo twardy wiek koszyka.",
                c.pending_lifetime, c.basket_max_age_min
            ),
        );
    }

    v
}

/// D27 — CZTERY MARTWE PUŁAPY GLOBALNE ŁAŃCUCHA.
///
/// `PulapyGlobalne` niesie 16 pól. Dwanaście z nich silnik czyta
/// (`engine.rs`: `blokuj_przeciwne_kierunki`, podłoga i obsunięcie, limity
/// pozycji/koszyków/lotów, cel dnia, limit straty dnia). CZTERY nie mają
/// w całym `rust/crates` ani jednego odczytu poza własną definicją — a panel
/// wystawia je jako zwykłe, działające kontrolki z żywym `onChange`:
///
/// | pole | co obiecuje | stan |
/// |---|---|---|
/// | `max_ryzyko_pct` | łączne ryzyko otwarte jako % equity | brak odczytu |
/// | `cel_dnia_zamyka` | „po celu dnia zamknij wszystko" | brak odczytu |
/// | `pauza_po_stratach_n` | pauza po serii stratnych koszyków | brak odczytu |
/// | `pauza_po_stratach_min` | długość tej pauzy | brak odczytu |
///
/// Dlaczego to nie jest łapane niczym innym: [`Settings::martwe_ustawienia`]
/// obejmuje wyłącznie pola `Settings`, a `PulapyGlobalne` to osobna struktura
/// łańcucha. Backtest przyjmuje te pola w `--pulapy`, więc przemiatanie po
/// nich daje wynik identyczny co do dolara — czyli klasyczna sygnatura pola
/// zbramkowanego, tyle że tutaj nie ma żadnej bramki, jest po prostu brak
/// kodu.
///
/// **Ta funkcja niczego nie naprawia.** Podłączenie `cel_dnia_zamyka`
/// i `max_ryzyko_pct` wymaga zmian w `engine.rs`, a tego pliku ta bramka nie
/// dotyka. Do czasu wdrożenia jedyną uczciwą rzeczą jest powiedzieć wprost,
/// że kontrolka nie działa — zamiast pozwalać właścicielowi liczyć na ochronę,
/// której nie ma.
pub fn sprawdz_pulapy(p: &PulapyGlobalne) -> Vec<Niespojnosc> {
    let mut v = Vec::new();
    let mut zglos = |pola: &'static str, opis: String| {
        v.push(Niespojnosc {
            kod: "D27-pulap-martwy",
            pola,
            opis,
        });
    };

    if p.max_ryzyko_pct > 0.0 {
        zglos(
            "pulapy.max_ryzyko_pct",
            format!(
                "`max_ryzyko_pct = {}` NIE DZIAŁA — silnik nie ma dla tego pola ani \
                 jednego odczytu. Łączne ryzyko otwarte nie jest pilnowane wcale. \
                 Zamiennik działający dziś: `max_portfolio_risk_pct` w presecie.",
                p.max_ryzyko_pct
            ),
        );
    }
    if p.cel_dnia_zamyka {
        zglos(
            "pulapy.cel_dnia_zamyka",
            "`cel_dnia_zamyka = true` NIE DZIAŁA — silnik nie ma dla tego pola ani \
             jednego odczytu. Po osiągnięciu celu dnia łańcuch wyłącznie PRZESTAJE \
             OTWIERAĆ (bramka wejścia); otwarte koszyki jadą dalej i mogą oddać cały \
             dzisiejszy zysk. Zamiennik działający dziś: `day_target_close` w presecie \
             — ale on liczy cel PER PRESET, nie dla całego łańcucha."
                .to_string(),
        );
    }
    if p.pauza_po_stratach_n > 0 || p.pauza_po_stratach_min > 0.0 {
        zglos(
            "pulapy.pauza_po_stratach_n / pulapy.pauza_po_stratach_min",
            format!(
                "`pauza_po_stratach_n = {}` / `pauza_po_stratach_min = {}` NIE DZIAŁAJĄ \
                 — silnik nie ma dla tych pól ani jednego odczytu, więc seria strat \
                 liczona ŁĄCZNIE przez formaty nie zatrzymuje niczego. Zamiennik \
                 działający dziś: `streak_pause_n`/`streak_pause_min` w presecie \
                 (liczy serię w obrębie jednego presetu, nie całego łańcucha).",
                p.pauza_po_stratach_n, p.pauza_po_stratach_min
            ),
        );
    }

    v
}

/// Wygodne opakowanie dla wywołujących, którzy mają dokument PANELU
/// (`settings.json`), a nie gotowe [`Settings`].
pub fn sprawdz_dokument(doc: &serde_json::Value) -> Vec<Niespojnosc> {
    sprawdz(&crate::settings_map::core_from_ui(doc))
}

pub fn opis_do_dziennika(lista: &[Niespojnosc]) -> String {
    let mut s = String::new();
    for (i, n) in lista.iter().enumerate() {
        s.push_str(&format!("{}. {}\n", i + 1, n));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kody(v: &[Niespojnosc]) -> Vec<&str> {
        v.iter().map(|n| n.kod).collect()
    }

    /// KONTRAKT ZERA BRAMKI: domyślne ustawienia nie mogą zapalić ani jednego
    /// ostrzeżenia. Bramka, która krzyczy na czystej konfiguracji, zostanie
    /// wyłączona po tygodniu i nie złapie już niczego.
    #[test]
    fn domyslne_ustawienia_sa_spojne() {
        assert!(sprawdz(&Settings::default()).is_empty());
        assert!(sprawdz_pulapy(&PulapyGlobalne::default()).is_empty());
    }

    /// S1 — runner bez celu i bez zapadki (rodzina starsza).
    #[test]
    fn s1_runner_bez_tp_przy_wylaczonym_trailingu() {
        let mut c = Settings::default();
        c.risk_free_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        c.trail_runner_mode = TrailMode::Off;
        assert_eq!(kody(&sprawdz(&c)), vec!["S1-runner-bez-wyjscia"]);

        // każdy człon z osobna jest normalną, sensowną konfiguracją
        let mut sam_cel = Settings::default();
        sam_cel.risk_free_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        assert!(sprawdz(&sam_cel).is_empty());
        let mut sama_zapadka = Settings::default();
        sama_zapadka.trail_runner_mode = TrailMode::Off;
        assert!(sprawdz(&sama_zapadka).is_empty());

        // `risk_free_trail` wygląda na ratunek i nim NIE JEST — gałąź w silniku
        // wymaga `trail_runner_mode != Off`. Dokładnie ta trójka stoi w całej
        // rodzinie OMEGA-X, więc gdyby bramka na nią milczała, nie złapałaby
        // najczęstszego wystąpienia tego przypadku.
        c.risk_free_trail = true;
        assert_eq!(kody(&sprawdz(&c)), vec!["S1-runner-bez-wyjscia"]);

        // ścieżka reakcji na KOMUNIKAT wyłączona — nie ma kto zrobić runnera
        c.risk_free_mode = RiskFreeMode::Ignore;
        assert!(sprawdz(&c).is_empty());

        // …ani przy trybach, które nie zostawiają NIKOGO: `keepers` jest wtedy
        // pusta, więc żadna pozycja nie dostaje `NoTpTrailOnly`.
        c.risk_free_mode = RiskFreeMode::CloseEverything;
        assert!(sprawdz(&c).is_empty());
        c.risk_free_mode = RiskFreeMode::CloseProfitableOnly;
        assert!(sprawdz(&c).is_empty());

        // ZERO RUNNERÓW NIE JEST WYŁĄCZNIKIEM: `keep_n = risk_free_runners.max(1)`,
        // czyli 0 zostawia jednego. Gdyby bramka tu zamilkła, przepuściłaby
        // runnera bez wyjścia — a to jest cały sens tej reguły.
        c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        c.risk_free_runners = 0;
        assert_eq!(
            kody(&sprawdz(&c)),
            vec!["S1-runner-bez-wyjscia"],
            "`risk_free_runners = 0` znaczy JEDEN runner, nie zero"
        );
    }

    /// S1 — rodzina nowsza (`riskfree_*`): bez celu, bez stopu, bez zegara.
    /// Zegar różny od zera JEST wyjściem, więc wtedy nie ostrzegamy.
    #[test]
    fn s1_regula_riskfree_bez_zadnego_warunku_wyjscia() {
        let mut c = Settings::default();
        c.riskfree_enabled = true;
        c.riskfree_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        c.riskfree_runner_stop = RiskFreeRunnerStop::Off;
        c.riskfree_runner_max_hold_min = 0.0;
        assert_eq!(kody(&sprawdz(&c)), vec!["S1-runner-bez-wyjscia"]);

        c.riskfree_runner_max_hold_min = 60.0;
        assert!(
            sprawdz(&c).is_empty(),
            "zegar 60 min to jest warunek wyjścia"
        );

        c.riskfree_runner_max_hold_min = 0.0;
        c.riskfree_enabled = false;
        assert!(
            sprawdz(&c).is_empty(),
            "reguła wyłączona nie ma czego prowadzić"
        );
    }

    /// S2 — BE, którego broker nigdy nie przyjmie.
    #[test]
    fn s2_be_offset_wiekszy_niz_prog_uzbrojenia() {
        let mut c = Settings::default();
        c.stops_level = 0.0;
        c.be_lock_pts = 2.0;
        c.be_offset = 3.0; // stop NAD ceną w chwili uzbrojenia
        assert_eq!(kody(&sprawdz(&c)), vec!["S2-be-nieosiagalny"]);

        c.be_offset = 0.5; // 1,50 zapasu przy zerowym wymaganiu brokera
        assert!(sprawdz(&c).is_empty());

        // ten sam zapas, ale broker wymaga więcej, niż go zostało
        c.stops_level = 2.0;
        assert_eq!(kody(&sprawdz(&c)), vec!["S2-be-nieosiagalny"]);

        // GRANICA: zapas DOKŁADNIE równy wymaganiu brokera jest legalny
        // (`sl_is_valid`: `sl <= bid − stops_level`), więc bramka ma milczeć.
        // Nierówność nieostra dawała tu fałszywy alarm.
        c.stops_level = 0.5;
        c.be_offset = 1.5; // 2,0 − 1,5 = 0,5 = stops_level
        assert!(
            sprawdz(&c).is_empty(),
            "zapas równy stops_level to jeszcze NIE sprzeczność"
        );

        // wyłączony BE-lock nie ma czego zgłaszać, choćby offset był absurdalny
        c.be_lock_pts = 0.0;
        c.be_offset = 100.0;
        assert!(sprawdz(&c).is_empty());
    }

    /// S3 — straż ekspozycji bez zgody na domykanie.
    #[test]
    fn s3_straz_ekspozycji_ktora_nic_nie_zamyka() {
        let mut c = Settings::default();
        c.expo_cap_pct = 300.0;
        c.expo_cap_close = false;
        assert_eq!(kody(&sprawdz(&c)), vec!["S3-straz-nic-nie-zamyka"]);

        c.expo_cap_close = true;
        assert!(sprawdz(&c).is_empty());

        // sam własny stop-out po poziomie marginesu domyka POZYCJE bez tej
        // zgody (osobna gałąź w `redukuj_ekspozycje`) — nie ma czego zgłaszać
        let mut d = Settings::default();
        d.expo_cap_ml_pct = 80.0;
        assert!(sprawdz(&d).is_empty());
    }

    /// S4 — `reenter_max = 0` to BEZ LIMITU, nie „wyłączone".
    #[test]
    fn s4_usrednianie_bez_hamulca() {
        let mut c = Settings::default();
        c.reenter_after_tp = true;
        c.reenter_max = 0;
        c.pending_lifetime = PendingLifetime::UntilTp2;
        assert_eq!(kody(&sprawdz(&c)), vec!["S4-usrednianie-bez-hamulca"]);

        // limit dokładek = hamulec, o który chodzi
        c.reenter_max = 3;
        assert!(sprawdz(&c).is_empty());

        // bez re-entry pole `reenter_max` nie ma czego ograniczać
        c.reenter_max = 0;
        c.reenter_after_tp = false;
        assert!(sprawdz(&c).is_empty());

        // krótka siatka + twardy wiek koszyka: hamulcem jest czas
        c.reenter_after_tp = true;
        c.pending_lifetime = PendingLifetime::UntilTp1;
        c.basket_max_age_min = 60.0;
        assert!(sprawdz(&c).is_empty());
    }

    /// Cztery przypadki są NIEZALEŻNE — konfiguracja chora na wszystko
    /// zgłasza wszystko, w stałej kolejności.
    #[test]
    fn wszystkie_cztery_naraz_i_zawsze_w_tej_samej_kolejnosci() {
        let mut c = Settings::default();
        c.risk_free_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        c.trail_runner_mode = TrailMode::Off;
        c.be_lock_pts = 2.0;
        c.be_offset = 3.0;
        c.stops_level = 0.0;
        c.expo_cap_pct = 300.0;
        c.reenter_after_tp = true;
        c.reenter_max = 0;
        c.pending_lifetime = PendingLifetime::Never;

        let v = sprawdz(&c);
        assert_eq!(
            kody(&v),
            vec![
                "S1-runner-bez-wyjscia",
                "S2-be-nieosiagalny",
                "S3-straz-nic-nie-zamyka",
                "S4-usrednianie-bez-hamulca",
            ]
        );
        // ten sam wynik przy powtórzeniu — wpis w dzienniku ma być porównywalny
        assert_eq!(sprawdz(&c), v);
    }

    /// D27 — cztery martwe pułapy łańcucha.
    #[test]
    fn d27_martwe_pulapy_zglaszane_po_nazwach() {
        let mut p = PulapyGlobalne::default();
        p.max_ryzyko_pct = 20.0;
        p.cel_dnia_zamyka = true;
        p.pauza_po_stratach_n = 3;
        p.pauza_po_stratach_min = 90.0;

        let v = sprawdz_pulapy(&p);
        assert_eq!(v.len(), 3, "pauza to jedna para pól: {v:?}");
        assert!(v.iter().all(|n| n.kod == "D27-pulap-martwy"));
        let tekst = opis_do_dziennika(&v);
        for pole in ["max_ryzyko_pct", "cel_dnia_zamyka", "pauza_po_stratach_n"] {
            assert!(tekst.contains(pole), "brak `{pole}` w opisie: {tekst}");
        }

        // pułapy, które silnik CZYTA, nie mają prawa się tu pojawić
        let mut zywe = PulapyGlobalne::default();
        zywe.max_pozycji = 10;
        zywe.max_koszykow = 3;
        zywe.max_lotow = 1.0;
        zywe.max_dd_pct = 40.0;
        zywe.podloga_equity_usd = 50.0;
        zywe.cel_dnia_usd = 100.0;
        zywe.limit_straty_dnia_usd = 50.0;
        zywe.blokuj_przeciwne_kierunki = true;
        assert!(sprawdz_pulapy(&zywe).is_empty());
    }

    /// Bramka musi dawać ten sam werdykt na dokumencie PANELU, co na gotowych
    /// ustawieniach — inaczej ostrzegałaby o czymś innym, niż pojedzie na
    /// rachunek (dokładnie ta klasa błędu, przez którą `preset_to_ui` musiał
    /// w ogóle powstać).
    #[test]
    fn dokument_panelu_daje_ten_sam_werdykt() {
        let doc = serde_json::json!({
            "expo_cap_pct": 300.0,
            "expo_cap_close": false,
        });
        let z_dokumentu = sprawdz_dokument(&doc);
        assert_eq!(kody(&z_dokumentu), vec!["S3-straz-nic-nie-zamyka"]);
        assert_eq!(
            z_dokumentu,
            sprawdz(&crate::settings_map::core_from_ui(&doc))
        );
    }
}
