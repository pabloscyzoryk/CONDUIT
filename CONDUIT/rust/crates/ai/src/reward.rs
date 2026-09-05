//! Funkcja nagrody.
//!
//! Postać, którą wybraliśmy, i uzasadnienie każdego składnika:
//!
//! ```text
//!   f_okna = ( PnL − K_dd·MaxDD − K_risk·MaxOtwarteRyzyko − K_hold·lot_godziny ) / kapitał
//!   f      = W_mean·średnia(f_okna) + W_min·min(f_okna)
//!   jeśli konto wyzerowane albo przebita podłoga:  f_okna = −K_blow + PnL/kapitał
//! ```
//!
//! * **PnL** — cel właściwy. Wyrażony jako zwrot z kapitału, żeby funkcja miała
//!   ten sam sens przy koncie 200 $ i 20 000 $.
//! * **−K_dd·MaxDD** — bez tego optymalizator wybiera strategię „trzymaj stratę,
//!   aż wróci". Kara liczona od SZCZYTU equity, tak samo jak w raportowaniu, więc
//!   to, co maksymalizujemy, jest tym, co potem oglądamy w wyniku. `K_dd = 0.5`
//!   to kompromis: przy 1.0 wygrywają strategie prawie nic nie robiące.
//! * **−K_risk·MaxOtwarteRyzyko** — to jest zabezpieczenie przed KONKRETNĄ
//!   sztuczką, którą dane historyczne mogą premiować. Szeroki stop-loss może
//!   wyglądać dobrze, gdy rzadko zostaje zrealizowany: zrealizowane obsunięcie
//!   pozostaje małe, choć konto stoi pod dużym, jeszcze niewywołanym ryzykiem.
//!   Sam drawdown tego nie widzi.
//!   Karzemy więc szczytową sumę `|wejście − SL| × 100 × wolumen` po wszystkich
//!   otwartych pozycjach; pozycja bez SL wchodzi z ryzykiem zastępczym równym
//!   wielokrotności ATR, żeby zdjęcie stopa niczego nie ukryło.
//!   Waga jest CELOWO mała (0.05): ryzyko jest przede wszystkim OGRANICZENIEM w
//!   [`crate::safety`] (twardy limit % kapitału na wejście i łącznie), a nie karą
//!   do negocjacji. Kara tej wielkości tylko przechyla remisy; to ogranicznik
//!   decyduje, czego nie wolno.
//! * **−K_hold·lot_godziny** — syntetyczny koszt utrzymania pozycji, naliczany
//!   proporcjonalnie do wolumenu i czasu (jak swap). Premiuje SZYBKI zysk, ale —
//!   w odróżnieniu od dyskontowania zysku w czasie — karze też POWOLNĄ STRATĘ.
//!   Dyskonto samego zysku miałoby patologiczny efekt uboczny: obniżałoby karę za
//!   długie trzymanie stratnych pozycji, czyli uczyło dokładnie odwrotnie.
//!   Kalibracja jest tu delikatna i była już dwa razy zła. 2.0 $/lot/h przykrywało
//!   cały PnL; 0.3 $/lot/h nadal karało strategię typu „runner" za to, co stanowi
//!   jej mechanizm — GLEBIA trzyma pozycje długo Z ZAŁOŻENIA, a kara sprawiała, że
//!   jej ocena wychodziła UJEMNA mimo +260 $ zysku. Przy 0.05 $/lot/h kara jest
//!   rzędu 1–2 $ na okno i faktycznie tylko przechyla remisy.
//! * **−K_blow** — wyzerowanie konta ma być gorsze niż jakakolwiek strata, jaką
//!   da się osiągnąć „normalnie". Dziesięciokrotność kapitału stawia je poza
//!   zasięgiem, więc żaden kompromis ryzyko/zysk go nie opłaci.
//! * **W_min** — 25 % wagi na KWARTYL DOLNY okien (nie na minimum!). Chodzi o to,
//!   żeby model nie żył z jednym koszmarnym reżimem. Minimum okazało się jednak
//!   zbyt ostre: przy 14 oknach to skrajna statystyka pozycyjna i z wagą 0.4
//!   sprawiała, że BEZCZYNNOŚĆ (ocena dokładnie 0) biła strategię zarabiającą
//!   +83.87 $. Kwartyl mierzy ten sam zły reżim, nie będąc zakładnikiem jednego
//!   pechowego okna.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RewardWeights {
    /// waga maksymalnego obsunięcia
    pub k_dd: f64,
    /// waga szczytowego OTWARTEGO ryzyka (Σ |wejście − SL| × 100 × wolumen)
    pub k_risk: f64,
    /// ryzyko zastępcze pozycji bez SL, w jednostkach ATR
    pub no_sl_atr: f64,
    /// koszt utrzymania: $ za 1 lot przez 1 godzinę
    pub k_hold: f64,
    /// kara za wyzerowanie konta, w jednostkach zwrotu z kapitału
    pub k_blow: f64,
    pub w_mean: f64,
    pub w_min: f64,
    /// Minimalna aktywność osobnika jako ułamek liczby transakcji linii bazowej
    /// W TYM SAMYM oknie. Poniżej progu osobnik jest odrzucany.
    ///
    /// To jest bezpośrednie wycięcie płaskiej doliny „nic nie rób", w której ES
    /// osiadał w każdym dotychczasowym przebiegu. Bezczynność ma ocenę dokładnie
    /// zero i jest lokalnym minimum otoczonym samymi gorszymi sąsiadami, więc
    /// gradient z niej nie wyprowadza. Próg aktywności sprawia, że tego punktu w
    /// ogóle nie ma w przestrzeni dopuszczalnej.
    pub min_activity_frac: f64,
}

impl Default for RewardWeights {
    fn default() -> Self {
        RewardWeights {
            k_dd: 0.25,
            k_risk: 0.05,
            no_sl_atr: 40.0,
            k_hold: 0.05,
            k_blow: 10.0,
            w_mean: 0.75,
            w_min: 0.25,
            min_activity_frac: 0.30,
        }
    }
}

/// Wynik jednego przebiegu przez okno danych.
#[derive(Clone, Debug, Default)]
pub struct WindowOutcome {
    pub start_balance: f64,
    pub end_equity: f64,
    pub max_dd_abs: f64,
    /// szczytowa suma ryzyka otwartych pozycji w trakcie przebiegu
    pub max_open_risk: f64,
    /// najniższe equity, jakie wystąpiło w oknie — margines do zera
    pub min_equity: f64,
    /// wynik każdej DOBY w oknie, w dolarach
    pub daily_pnl: Vec<f64>,
    /// liczba transakcji zamkniętych w każdej dobie — pozwala odróżnić dobę
    /// stratną od doby, w której po prostu nic się nie działo (weekend, święto)
    pub daily_trades: Vec<u32>,
    /// wynik każdej doby jako % equity NA POCZĄTKU TEJ DOBY
    ///
    /// Odniesienie do equity bieżącego, nie startowego. Przy compoundingu
    /// dzielenie przez kapitał startowy zamienia stratę 15 % w „750 %" i
    /// wyrzuca każdy rosnący wariant z rankingu.
    pub daily_ret_pct: Vec<f64>,
    /// Σ wolumen × godziny trzymania po zamkniętych transakcjach
    pub lot_hours: f64,
    pub trades: u32,
    pub wins: u32,
    pub gross_win: f64,
    pub gross_loss: f64,
    pub blown: bool,
    pub floor_hit: bool,
    pub decisions: u64,
    pub actions: u64,
    // --- diagnostyka przebiegu (nie wchodzi do nagrody) ---
    /// ile wiadomości silnik uznał za sygnał do działania
    pub signals: u64,
    /// ile koszyków powstało
    pub baskets: u32,
    /// ile zleceń broker odrzucił z powodu SL/TP
    pub rejected_stops: u64,
    /// ile limitów zostało zafillowanych
    pub filled_pendings: u64,
    /// zlecenia wysłane do brokera: rynkowe ok/błąd, oczekujące ok/błąd
    pub orders: [u64; 4],
    /// rozbicie akcji modelu — patrz [`ACTION_NAMES`]
    pub acts: [u64; 7],
    /// zysk każdej zamkniętej transakcji — do analizy ryzyka ruiny
    pub profits: Vec<f64>,
    pub repositions: u64,
    pub repos_units: u64,
    pub depth_sum: f64,
}

/// Nazwy pozycji w tablicy `acts`, w kolejności.
pub const ACTION_NAMES: [&str; 7] = [
    "zamknięcia",
    "częściowe",
    "SL",
    "TP",
    "TP zdjęte",
    "limity+",
    "limity−",
];

impl WindowOutcome {
    #[inline]
    pub fn pnl(&self) -> f64 {
        self.end_equity - self.start_balance
    }

    #[inline]
    pub fn profit_factor(&self) -> f64 {
        if self.gross_loss > 1e-9 {
            self.gross_win / self.gross_loss
        } else if self.gross_win > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    }

    #[inline]
    pub fn win_rate(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.wins as f64 / self.trades as f64
        }
    }

    #[inline]
    pub fn max_dd_pct(&self) -> f64 {
        self.max_dd_abs / self.start_balance.max(1.0) * 100.0
    }

    pub fn fitness(&self, w: &RewardWeights) -> f64 {
        let cap = self.start_balance.max(1.0);
        let pnl = self.pnl();
        if self.blown || self.floor_hit {
            return -w.k_blow + pnl / cap;
        }
        (pnl - w.k_dd * self.max_dd_abs - w.k_risk * self.max_open_risk - w.k_hold * self.lot_hours)
            / cap
    }
}

/// **Oś rankingu: MAR × margines do zera × skala zysku.**
///
/// Zmiana filozofii wobec poprzedniej wersji, i to zmiana zasadnicza:
/// **obsunięcie samo w sobie nie jest szkodą — szkodą jest realna strata.**
/// Konto, które z 200 $ robi 600 $ przy obsunięciu 400 $, jest lepsze od konta,
/// które robi 180 $ przy obsunięciu 20 $, mimo dwudziestokrotnie większego
/// obsunięcia. Poprzednia funkcja celu karała obsunięcie wprost i dlatego
/// systematycznie wybierała bezczynność.
///
/// Trzy czynniki, każdy o innym zadaniu:
/// * **MAR** = zysk / maksymalne obsunięcie — oś właściwa,
/// * **margines do zera** = `min_equity / kapitał`, przycięty do [0.05, 1.0] —
///   konto, które ocalało cudem, nie ma prawa wygrać z takim, które nigdy nie
///   zeszło nisko,
/// * **skala zysku** = `sqrt(zysk / kapitał)` — bez tego wygrywa wariant
///   zarabiający grosze przy obsunięciu bliskim zeru, bo MAR leci wtedy w niebo.
///
/// Strata jest ZAWSZE poniżej każdego zysku, a wyzerowanie konta dyskwalifikuje
/// bezwarunkowo. Żadnych kar za obsunięcie, ryzyko czy czas trzymania —
/// bezpieczeństwa pilnuje warstwa wykonania, nie funkcja celu.
pub fn aggregate_mar(
    outs: &[WindowOutcome],
    base: Option<&[WindowOutcome]>,
    w: &RewardWeights,
) -> f64 {
    if outs.is_empty() {
        return f64::NEG_INFINITY;
    }
    let cap = outs[0].start_balance.max(1.0);

    // dyskwalifikacja: wyzerowane konto albo przebita podłoga
    if outs.iter().any(|o| o.blown || o.floor_hit) {
        return f64::NEG_INFINITY;
    }

    // bramka aktywności — bezczynność i tak wypada ujemnie, ale niech wypada JAWNIE
    if let Some(b) = base {
        if b.len() == outs.len() {
            let t_m: u32 = outs.iter().map(|o| o.trades).sum();
            let t_b: u32 = b.iter().map(|o| o.trades).sum();
            if (t_m as f64) < t_b as f64 * w.min_activity_frac {
                return -1000.0 + t_m as f64 * 1e-6;
            }
        }
    }

    let profit: f64 = outs.iter().map(|o| o.pnl()).sum();
    // strata zawsze poniżej każdego zysku — osobna, monotoniczna gałąź
    if profit <= 0.0 {
        return -1000.0 + profit / cap;
    }

    let maxdd = outs.iter().map(|o| o.max_dd_abs).fold(0.0f64, f64::max);
    let mar = profit / maxdd.max(cap * 0.005);
    let margines = (outs
        .iter()
        .map(|o| o.min_equity)
        .fold(f64::INFINITY, f64::min)
        / cap)
        .clamp(0.05, 1.0);
    let skala = (profit / cap).sqrt();
    mar * margines * skala
}

/// Agregacja WZGLĘDEM LINII BAZOWEJ — poprzedni tryb oceny (zachowany do porównań).
///
/// `nagroda = f(model) − f(preset na tym samym oknie)`. Różnica wobec liczenia
/// od zera jest zasadnicza: bezczynność przestaje być punktem neutralnym i staje
/// się jawnie ujemna, bo oddaje cały wynik linii bazowej. Wcześniej model, który
/// nic nie robił, dostawał 0.0 i wygrywał z każdą strategią obciążoną karami,
/// zanim ta zdążyła cokolwiek zarobić.
///
/// Osobnik zbyt bierny (mniej niż `min_activity_frac` transakcji linii bazowej)
/// dostaje ocenę karną, a nie „prawie zero" — inaczej bezczynność wracałaby
/// tylnymi drzwiami jako bezpieczny remis.
pub fn aggregate_vs(outs: &[WindowOutcome], base: &[WindowOutcome], w: &RewardWeights) -> f64 {
    if outs.is_empty() || base.len() != outs.len() {
        return -w.k_blow;
    }
    let cap = outs[0].start_balance.max(1.0);
    let mut f: Vec<f64> = Vec::with_capacity(outs.len());
    for (o, b) in outs.iter().zip(base.iter()) {
        if o.blown || o.floor_hit {
            f.push(-w.k_blow);
            continue;
        }
        let prog = (b.trades as f64 * w.min_activity_frac).ceil() as u32;
        if o.trades < prog {
            // zbyt bierny: oddajemy cały wynik bazy i jeszcze dokładamy
            f.push(-(b.pnl().abs() + cap * 0.05) / cap);
            continue;
        }
        f.push(o.fitness(w) - b.fitness(w));
    }
    let mean = f.iter().sum::<f64>() / f.len() as f64;
    f.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q25 = f[(f.len() as f64 * 0.25).floor() as usize];
    w.w_mean * mean + w.w_min * q25
}

/// Agregacja wielu okien w jedną liczbę.
///
/// Człon „ogonowy" to **kwartyl dolny**, a nie minimum. To była realna pułapka:
/// przy 14 oknach minimum jest skrajną statystyką pozycyjną, o której decyduje
/// jedno pechowe okno. Z wagą 0.4 wystarczało, żeby strategia zarabiająca
/// +83.87 $ na części uczącej dostała ocenę −0.027, czyli GORSZĄ niż „nie rób
/// nic" (dokładnie 0.0 w każdym oknie). Optymalizator postępował poprawnie —
/// to funkcja celu była źle postawiona i nagradzała bezczynność.
///
/// Kwartyl dolny mierzy to samo, o co chodziło (jak wygląda ZŁY reżim), ale nie
/// jest zakładnikiem jednego okna.
pub fn aggregate(outs: &[WindowOutcome], w: &RewardWeights) -> f64 {
    if outs.is_empty() {
        return -w.k_blow;
    }
    let mut f: Vec<f64> = outs.iter().map(|o| o.fitness(w)).collect();
    let mean = f.iter().sum::<f64>() / f.len() as f64;
    f.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q25 = f[(f.len() as f64 * 0.25).floor() as usize];
    w.w_mean * mean + w.w_min * q25
}

/// Sumaryczny obraz wielu okien — do raportowania, nie do optymalizacji.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub pnl: f64,
    pub max_dd_abs: f64,
    pub max_dd_pct: f64,
    pub max_open_risk: f64,
    pub trades: u32,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub blown: usize,
    pub windows: usize,
    pub positive_windows: usize,
    pub fitness: f64,
    pub actions: u64,
    pub signals: u64,
    pub baskets: u32,
    pub rejected_stops: u64,
    pub filled_pendings: u64,
    pub orders: [u64; 4],
    pub acts: [u64; 7],
    pub profits: Vec<f64>,
    pub repositions: u64,
    pub repos_units: u64,
    pub depth_sum: f64,
    /// PnL każdego okna osobno — podstawa przedziału ufności
    pub window_pnl: Vec<f64>,
    /// najniższe equity w całym zbiorze, jako ułamek kapitału startowego
    pub min_equity_ratio: f64,
    /// liczba okien (przy oknach dziennych: dni) na minusie
    pub losing_windows: usize,
    /// najgorsze okno jako % kapitału startowego
    pub worst_window_pct: f64,
    pub mar: f64,
}

pub fn summarize(outs: &[WindowOutcome], w: &RewardWeights) -> Summary {
    let mut s = Summary {
        windows: outs.len(),
        ..Default::default()
    };
    let mut gw = 0.0;
    let mut gl = 0.0;
    for o in outs {
        s.pnl += o.pnl();
        s.max_dd_abs = s.max_dd_abs.max(o.max_dd_abs);
        s.max_dd_pct = s.max_dd_pct.max(o.max_dd_pct());
        s.max_open_risk = s.max_open_risk.max(o.max_open_risk);
        s.trades += o.trades;
        gw += o.gross_win;
        gl += o.gross_loss;
        if o.blown || o.floor_hit {
            s.blown += 1;
        }
        if o.pnl() > 0.0 {
            s.positive_windows += 1;
        }
        s.actions += o.actions;
        s.signals += o.signals;
        s.baskets += o.baskets;
        s.rejected_stops += o.rejected_stops;
        s.filled_pendings += o.filled_pendings;
        for k in 0..4 {
            s.orders[k] += o.orders[k];
        }
        for k in 0..7 {
            s.acts[k] += o.acts[k];
        }
        s.profits.extend_from_slice(&o.profits);
        s.window_pnl.push(o.pnl());
        s.repositions += o.repositions;
        s.repos_units += o.repos_units;
        s.depth_sum += o.depth_sum;
    }
    // --- miary REALNEJ szkody ---
    let cap = outs
        .first()
        .map(|o| o.start_balance)
        .unwrap_or(1.0)
        .max(1.0);
    s.min_equity_ratio = outs
        .iter()
        .map(|o| o.min_equity)
        .fold(f64::INFINITY, f64::min)
        / cap;
    s.losing_windows = outs.iter().filter(|o| o.pnl() < 0.0).count();
    s.worst_window_pct = outs.iter().map(|o| o.pnl()).fold(f64::INFINITY, f64::min) / cap * 100.0;
    s.mar = if s.max_dd_abs > 1e-9 {
        s.pnl / s.max_dd_abs
    } else {
        0.0
    };
    let wins: u32 = outs.iter().map(|o| o.wins).sum();
    s.win_rate = if s.trades > 0 {
        wins as f64 / s.trades as f64
    } else {
        0.0
    };
    s.profit_factor = if gl > 1e-9 {
        gw / gl
    } else if gw > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    s.fitness = aggregate(outs, w);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(pnl: f64, dd: f64) -> WindowOutcome {
        WindowOutcome {
            start_balance: 1000.0,
            end_equity: 1000.0 + pnl,
            max_dd_abs: dd,
            ..Default::default()
        }
    }

    #[test]
    fn drawdown_obniza_ocene() {
        let w = RewardWeights::default();
        assert!(out(100.0, 10.0).fitness(&w) > out(100.0, 200.0).fitness(&w));
    }

    #[test]
    fn wyzerowanie_konta_jest_gorsze_od_kazdej_straty() {
        let w = RewardWeights::default();
        let mut blown = out(-400.0, 400.0);
        blown.blown = true;
        // najgorszy „normalny" wynik, jaki podłoga equity w ogóle dopuszcza
        let bad = out(-400.0, 400.0);
        assert!(blown.fitness(&w) < bad.fitness(&w) - 5.0);
    }

    #[test]
    fn szeroki_stop_nie_jest_darmowy() {
        // Dwa przebiegi o IDENTYCZNYM wyniku i identycznym zrealizowanym
        // obsunięciu. Różni je tylko to, że drugi trzymał ogromne otwarte
        // ryzyko, którego historia akurat nie zrealizowała. Funkcja nagrody
        // musi go ukarać — inaczej model nauczy się rozszerzać stopy.
        let w = RewardWeights::default();
        let waski = WindowOutcome {
            max_open_risk: 20.0,
            ..out(80.0, 15.0)
        };
        let szeroki = WindowOutcome {
            max_open_risk: 400.0,
            ..out(80.0, 15.0)
        };
        assert!(waski.fitness(&w) > szeroki.fitness(&w));
        // Kara jest CELOWO niewielka — otwarte ryzyko ogranicza przede wszystkim
        // warstwa bezpieczeństwa (twardy limit % kapitału), a nagroda tylko
        // przechyla remisy. Sprawdzamy więc kierunek i dokładną wielkość, a nie
        // to, czy kara dominuje.
        let roznica = waski.fitness(&w) - szeroki.fitness(&w);
        assert!(
            (roznica - w.k_risk * (400.0 - 20.0) / 1000.0).abs() < 1e-9,
            "różnica {roznica}"
        );
    }

    #[test]
    fn dlugie_trzymanie_kosztuje() {
        let w = RewardWeights::default();
        let fast = out(50.0, 5.0);
        let mut slow = out(50.0, 5.0);
        slow.lot_hours = 0.01 * 200.0; // 0.01 lota przez 200 h
        assert!(fast.fitness(&w) > slow.fitness(&w));
    }

    #[test]
    fn agregacja_karze_zly_ogon() {
        let w = RewardWeights::default();
        let rowny = vec![
            out(50.0, 10.0),
            out(50.0, 10.0),
            out(50.0, 10.0),
            out(50.0, 10.0),
        ];
        let rozstrzelony = vec![
            out(200.0, 10.0),
            out(50.0, 10.0),
            out(-50.0, 120.0),
            out(-50.0, 120.0),
        ];
        assert!(aggregate(&rowny, &w) > aggregate(&rozstrzelony, &w));
    }

    #[test]
    fn zyskowna_strategia_bije_bezczynnosc() {
        // REGRESJA: z wagą 0.4 na MINIMUM strategia zarabiająca przegrywała
        // z „nie rób nic", bo wystarczało jedno złe okno na czternaście.
        let w = RewardWeights::default();
        let mut zyskowna: Vec<WindowOutcome> = (0..13).map(|_| out(6.0, 4.0)).collect();
        zyskowna.push(out(-16.0, 20.0)); // jedno paskudne okno
        let bezczynnosc: Vec<WindowOutcome> = (0..14).map(|_| out(0.0, 0.0)).collect();
        let a = aggregate(&zyskowna, &w);
        let b = aggregate(&bezczynnosc, &w);
        assert!(a > b, "zyskowna {a:+.4} musi bić bezczynność {b:+.4}");
    }
}

// ============================================================
//  STATYSTYKA: PRZEDZIAŁ UFNOŚCI I RYZYKO RUINY
// ============================================================

/// Deterministyczny generator do bootstrapu (xorshift64*).
///
/// Własny, bo bootstrap ma dawać ten sam przedział przy tym samym wejściu —
/// przedział ufności, który zmienia się między uruchomieniami, jest gorszy niż
/// żaden.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    #[inline]
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    #[inline]
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Przedział ufności 95 % dla SUMY wyników okien, metodą bootstrapu.
///
/// Losujemy okna ze zwracaniem, bo okna są jednostką niezależną — nie
/// pojedyncze transakcje, które w obrębie okna są ze sobą skorelowane przez
/// wspólny stan konta i wspólny reżim rynku. Bootstrap po transakcjach dawałby
/// przedział sztucznie wąski.
pub fn bootstrap_ci(window_pnl: &[f64], draws: usize, seed: u64) -> (f64, f64) {
    if window_pnl.len() < 2 {
        return (f64::NAN, f64::NAN);
    }
    let n = window_pnl.len();
    let mut rng = Rng::new(seed);
    let mut sums: Vec<f64> = Vec::with_capacity(draws);
    for _ in 0..draws {
        let mut s = 0.0;
        for _ in 0..n {
            s += window_pnl[rng.below(n)];
        }
        sums.push(s);
    }
    sums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (sums[draws / 40], sums[draws - 1 - draws / 40])
}

/// Ryzyko ruiny przy zadanym kapitale startowym.
///
/// Losujemy CIĄGI transakcji ze zwracaniem z rozkładu zaobserwowanego poza
/// próbą i sprawdzamy, czy equity kiedykolwiek spadnie do podłogi. Zysk
/// transakcji nie skaluje się z kapitałem (lot jest stały 0.01), więc większy
/// kapitał wprost obniża ryzyko — i o to w tym pytaniu chodzi.
///
/// To jest oszacowanie OPTYMISTYCZNE: losowanie ze zwracaniem gubi
/// autokorelację serii strat, a realne serie bywają dłuższe niż losowe.
pub fn ruin_probability(
    profits: &[f64],
    capital: f64,
    floor_pct: f64,
    paths: usize,
    seed: u64,
) -> f64 {
    if profits.is_empty() {
        return 0.0;
    }
    let floor = capital * floor_pct / 100.0;
    let mut rng = Rng::new(seed);
    let mut ruined = 0usize;
    for _ in 0..paths {
        let mut eq = capital;
        for _ in 0..profits.len() {
            eq += profits[rng.below(profits.len())];
            if eq <= floor {
                ruined += 1;
                break;
            }
        }
    }
    ruined as f64 / paths as f64
}

/// Najmniejszy kapitał z listy, przy którym ryzyko ruiny spada poniżej progu.
pub fn min_capital_for(
    profits: &[f64],
    targets: &[f64],
    max_ruin: f64,
    floor_pct: f64,
    seed: u64,
) -> Option<(f64, f64)> {
    for &c in targets {
        let r = ruin_probability(profits, c, floor_pct, 20_000, seed);
        if r < max_ruin {
            return Some((c, r));
        }
    }
    None
}

#[cfg(test)]
mod stat_tests {
    use super::*;

    #[test]
    fn bootstrap_jest_deterministyczny_i_obejmuje_srednia() {
        let w = vec![10.0, -5.0, 20.0, -2.0, 7.0, 1.0, -9.0, 12.0];
        let a = bootstrap_ci(&w, 4000, 1);
        let b = bootstrap_ci(&w, 4000, 1);
        assert_eq!(a, b, "ten sam wejściowy zestaw musi dać ten sam przedział");
        let suma: f64 = w.iter().sum();
        assert!(
            a.0 <= suma && suma <= a.1,
            "przedział {a:?} nie obejmuje sumy {suma}"
        );
        assert!(a.0 < a.1);
    }

    #[test]
    fn ryzyko_ruiny_maleje_z_kapitalem() {
        // rozkład wyraźnie stratny
        let p: Vec<f64> = (0..200)
            .map(|i| if i % 3 == 0 { 6.0 } else { -4.0 })
            .collect();
        let r200 = ruin_probability(&p, 200.0, 60.0, 5_000, 7);
        let r5000 = ruin_probability(&p, 5000.0, 60.0, 5_000, 7);
        assert!(r200 > r5000, "ruina {r200} vs {r5000}");
        assert!(r200 > 0.5, "stratny rozkład na małym koncie musi rujnować");
    }

    #[test]
    fn dodatni_rozklad_nie_rujnuje_duzego_konta() {
        let p: Vec<f64> = (0..200)
            .map(|i| if i % 4 == 0 { -3.0 } else { 2.0 })
            .collect();
        assert!(ruin_probability(&p, 5000.0, 60.0, 5_000, 3) < 0.01);
    }
}
