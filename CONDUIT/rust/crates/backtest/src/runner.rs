//! Przebieg backtestu: ticki + wiadomości → silnik → symulator brokera.
//!
//! Reguły strategii korzystają ze wspólnego Engine. Obserwacja rynku oraz
//! wykonanie brokera mają jawne modele i wymagają oddzielnych testów parytetu;
//! wspólny interfejs sam nie dowodzi identyczności z LIVE.

use crate::data::{ReplayMessage, TickData};
use crate::journal_dump::JournalDump;
use crate::metrics::{compute, DayStat, Metrics};
use crate::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage, SrWarmupBar};
use conduit_core::formaty::{Lancuch, PulapyGlobalne};
use conduit_core::routing::Silniki;
use conduit_core::settings::Settings;
use conduit_core::telegram_ingress::{
    opens_basket, stale_entry_age_minutes, ContentMemory,
};
use conduit_core::types::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// JEDEN FORMAT W PRZEBIEGU: nazwa kanału, nazwa presetu i jego ustawienia.
///
/// Istnieje po to, żeby backtest umiał policzyć **kilka presetów naraz na
/// jednym rachunku** — tak jak bot pracuje na żywo od 03.08.2026. Wcześniej
/// `--preset` opisywał cały strumień i nie dało się zmierzyć portfela dwóch
/// kanałów: ani wspólnego marginesu, ani pułapów, ani tego, czy formaty się
/// wzmacniają, czy wytracają.
#[derive(Debug, Clone)]
pub struct FormatCfg {
    /// nazwa formatu — musi zgadzać się z polem `kanal` sygnału
    pub format: String,
    /// nazwa presetu (do tabel i zrzutów)
    pub preset: String,
    pub settings: Settings,
}

#[derive(Debug, Clone)]
pub struct RunConfig {
    /// początek okna WŁĄCZNIE
    pub from: Ts,
    /// Koniec okna WYŁĄCZNIE. Pętla przebiegu chodzi po `[from, to)` — tick o
    /// znaczniku równym `to` już NIE jest liczony (patrz `index_at` niżej).
    /// `from == to` to okno PUSTE, nie okno jednodniowe.
    pub to: Ts,
    pub start_balance: f64,
    /// Model realizacji LIMIT po lepszej bieżącej cenie; false = legacy.
    /// Dotyczy tylko symulatora. Na live cenę wykonania nadaje broker MT5.
    pub sim_limit_price_improvement: bool,
    /// Simulator-only native pending/SL sequencing profile; default legacy OFF.
    pub sim_new_pending_sl_next_tick: bool,
    /// Explicit broker cash model: None is legacy; Some(0..=8) accrues swap
    /// outside Balance until close, with the declared account-money precision.
    /// This is not a strategy/sizing axis and is independent of NET reporting.
    pub sim_native_swap_cash_digits: Option<u32>,
    /// Feed raw messages through the exact bounded content-dedup ingress used
    /// by the VPS Telegram listener before they reach the parser/Engine.
    /// False preserves the ordinary export backtest bit-for-bit.
    pub live_telegram_ingress: bool,
    /// Explicitly approximate replay: raw rows are grouped into blocks of N
    /// and represented by causal BID/ASK extrema.  `1` is the exact legacy
    /// path.  This is a search/screening control, never a strategy axis.
    pub quick_tick_stride: usize,
    /// Ustawienia PRZEBIEGU JEDNOFORMATOWEGO. Gdy `formaty` nie jest puste,
    /// to pole opisuje wyłącznie RACHUNEK (pola z `wielosilnik::POLA_RACHUNKU`)
    /// i nie steruje handlem.
    pub settings: Settings,
    /// FORMATY HANDLUJĄCE. Puste = dokładnie ścieżka sprzed 03.08.2026:
    /// jeden silnik, slot 0, zerowe pułapy, zerowe obce obciążenie i **bez
    /// widoku brokera**. Na tym stoi bramka parytetu.
    pub formaty: Vec<FormatCfg>,
    /// Pułapy obowiązujące PONAD presetami (`limit skuteczny = min(preset, pułap)`).
    /// Same zera = rządzą wyłącznie presety.
    pub pulapy: PulapyGlobalne,
    /// Tryb „każdy dzień osobno": co dobę resetujemy konto do kwoty startowej.
    /// Odpowiada pytaniu „ile zarobiłbym każdego dnia, zaczynając od $200",
    /// w odróżnieniu od trybu z compoundingiem.
    pub daily_reset: bool,
    /// nazwa źródła sygnałów (kanał)
    pub source_name: String,
    /// zapisuj krzywą equity co N ms (0 = co zamknięcie transakcji)
    pub curve_interval_ms: i64,
    /// ROZGRZEWKA HISTORII RYNKU: ile godzin ceny SPRZED `from` wsypać do
    /// silnika, zanim zacznie handlować. **0 = zimny start** i to jest
    /// zachowanie sprzed 03.08.2026, na którym stoi bramka parytetu.
    ///
    /// # Po co to jest
    ///
    /// `Engine::regime_ok` przy niepełnej historii PRZEPUSZCZA WSZYSTKO, a
    /// `price_hist` rośnie o jeden punkt na godzinę. Przy `regime_ma_hours =
    /// 72` znaczy to, że silnik startujący na zimno przez pierwsze 72 godziny
    /// handlowe nie filtruje reżimu w ogóle.
    ///
    /// W backteście liczonym od `--from` jest to WIERNY model bota włączonego
    /// tego dnia — i dopóki bot produkcyjny naprawdę tak startował, była to
    /// właściwa domyślna. Od chwili, gdy `live.rs` dostał rozgrzewkę
    /// (`rozgrzej_historie`), wierny model wymaga jej także tutaj — inaczej
    /// LOTTO-SURVIVAL mierzy bota, którego już nie wydajemy, i systematycznie
    /// ZANIŻA przeżywalność.
    ///
    /// Wycena szkody z `ZADANIA 31072026.txt` (ten sam preset, te same dni):
    /// 22.07 okno ciepłe **0,00 $ / 0 transakcji**, zimny start **−135,48 $ /
    /// 13 transakcji**; 23.07 odpowiednio **0,00** i **−48,64 / 14**. Dwa dni,
    /// które preset miał przesiedzieć, kosztowały **−184 $**.
    ///
    /// Historia budowana jest **z tych samych ticków**, którymi przebieg
    /// potem gra, i tą samą regułą co `on_tick` (punkt, gdy minęła godzina;
    /// wartość = `mid`). Dzięki temu nie jest przybliżeniem, tylko dokładnie
    /// tym, co silnik miałby, gdyby ruszył wcześniej.
    pub rozgrzewka_h: usize,
    /// DRABINKA ŁAŃCUCHÓW PO SALDZIE. **Pusta = ścieżka dotychczasowa,
    /// dosłownie: żaden nowy kod się nie wykonuje.** Na tym stoi bramka
    /// parytetu. Niepusta lista wyklucza `formaty` i `pulapy` — szczeble
    /// noszą własne nogi i własne pułapy. Patrz [`SzczebelCfg`].
    pub drabinka: Vec<SzczebelCfg>,
    /// Histereza schodzenia W DÓŁ, w procentach progu: schodzimy dopiero,
    /// gdy saldo < prog · (1 − h/100). `0` = bez histerezy (domyślnie
    /// w backteście; na żywo wartość ustala użytkownik).
    pub drabinka_histereza_pct: f64,
    /// Historyczny kredyt odejmowany od progów Drabinki (model legacy).
    /// `credit_balance_separate` ON ignoruje tę kwotę: raw MT5 Balance
    /// już wyklucza Credit. Próg nadal nie zależy od pływającego PnL/Equity.
    ///
    /// `0.0` = brak bonusu (domyślnie) — wtedy próg widzi surowe saldo i kod
    /// jest co do centa tym sprzed 04.08.2026.
    pub drabinka_kredyt: f64,
    /// PŁASKA DOBA BEZ RESETU SALDA — „dzień po dniu, ale zyski i straty
    /// przechodzą dalej".
    ///
    /// Robi DOKŁADNIE pierwszą połowę tego, co `daily_reset`: na granicy doby
    /// domyka wszystko (`CloseReason::EodFlat`) i wymienia silnik, ale
    /// **saldo zostaje**. Odpowiednik na żywo to `eod_flat_hour` — czyli nie
    /// nowy mechanizm, tylko przełącznik, który silnik już ma.
    ///
    /// `false` = ścieżka dotychczasowa co do centa.
    pub flat_na_dobie: bool,
    /// Dokąd zrzucić dziennik zdarzeń (`.jsonl`). `None` = nie zapisuj.
    ///
    /// Dziennik powstaje tylko wtedy, gdy preset ma `journal_enabled` —
    /// sama ścieżka go nie włącza. Dzięki temu sweep po stu presetach nie
    /// zaczyna nagle produkować stu plików po kilkaset megabajtów.
    pub journal_path: Option<std::path::PathBuf>,
    /// TRYB AUTO-EA W BACKTEŚCIE — flaga [`Engine::tryb_auto_ea`] na każdym
    /// silniku zespołu.
    ///
    /// # Po co osobne pole, skoro flagi dziś nikt nie czyta
    ///
    /// Bo **punkt (c) potrójnego kontraktu zera** („tryb AUTO-EA bez osi ⇒
    /// co do centa jak AUTO") jest inaczej NIESPRAWDZALNY na korpusie.
    /// Do 25.08.2026 flagę ustawiała wyłącznie `live.rs`, więc jedyny dowód,
    /// jaki dało się przedstawić, był dowodem na atrapie brokera — a to jest
    /// dokładnie ta klasa dowodu, o której wiemy, że nie wystarcza
    /// (`sim_clock_strict` nie widział zamrożonego `rev_exit`).
    ///
    /// `false` = ścieżka dotychczasowa, dosłownie: ani jedno przypisanie się
    /// nie wykonuje. Bramka parytetu liczy się przy tej wartości.
    pub auto_ea: bool,
    /// KONFIGURACJA EA tego przebiegu (surowy JSON z pola `ea` presetu).
    ///
    /// `None` = nie ruszaj warstwy: rdzeń weźmie konfigurację ze zmiennej
    /// `CONDUIT_EA_BETA` albo zostanie wyłączony. Dzięki temu przebieg bez
    /// tego pola jest identyczny co do bitu z przebiegiem sprzed zmiany.
    pub ea_konfig: Option<String>,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            from: 0,
            to: i64::MAX,
            start_balance: 200.0,
            sim_limit_price_improvement: false,
            sim_new_pending_sl_next_tick: false,
            sim_native_swap_cash_digits: None,
            live_telegram_ingress: false,
            quick_tick_stride: 1,
            settings: Settings::default(),
            formaty: Vec::new(),
            pulapy: PulapyGlobalne::default(),
            daily_reset: false,
            source_name: "ATFX VIP SIGNALS".into(),
            curve_interval_ms: 60_000,
            // ZIMNY START — zachowanie sprzed 03.08.2026. Bramka parytetu
            // liczy się przy tej wartości i tak ma zostać.
            rozgrzewka_h: 0,
            journal_path: None,
            drabinka: Vec::new(),
            drabinka_histereza_pct: 0.0,
            drabinka_kredyt: 0.0,
            flat_na_dobie: false,
            // Tryb AUTO-EA WYŁĄCZONY — ścieżka parytetu. Włączenie ma dziś
            // zmieniać dokładnie nic (kontrakt zera, punkt c), ale domyślną
            // zostaje `false`, żeby dowód „nic nie zmienia" był dowodem,
            // a nie definicją.
            auto_ea: false,
            ea_konfig: None,
        }
    }
}

/// JEDEN SZCZEBEL DRABINKI ŁAŃCUCHÓW (`RunConfig::drabinka`).
///
/// Drabinka FFS-1C (zamówienie użytkownika 04.08.2026): konto zaczyna na
/// najniższym szczeblu i wspina się po progach SALDA — `0 → ZENONLY5,
/// 500 → ZENONLY3, 1000 → SENTINEL-0, 1500 → SENTINEL-0A`. Progi liczą się
/// po **BALANCE, nie equity**: pływający wynik otwartych pozycji nie ma prawa
/// przełączać łańcuchów, bo cofnąłby się razem z ceną i drabinka trzepotałaby
/// przy każdym oddechu rynku.
#[derive(Debug, Clone)]
pub struct SzczebelCfg {
    /// próg BALANCE, od którego ten szczebel obowiązuje (pierwszy zwykle 0)
    pub prog: f64,
    /// nazwa łańcucha — do tabel i dziennika
    pub nazwa: String,
    /// nogi łańcucha: format → preset (ustawienia SUROWE z pliku presetu;
    /// scalenie z polami rachunku robi runner, tak samo jak przy `formaty`)
    pub formaty: Vec<FormatCfg>,
    /// pułapy łańcucha tego szczebla — każdy szczebel nosi własne
    pub pulapy: PulapyGlobalne,
}

/// Co zrobił JEDEN SZCZEBEL drabinki w całym przebiegu.
///
/// Zysk przypisujemy szczeblowi AKTYWNEMU W CHWILI ZAMKNIĘCIA transakcji —
/// to jest ta sama zasada co rozliczanie koszyka domkniętego zbiorczo:
/// liczy się historia brokera, nie życzenie silnika. Koszyk otwarty na
/// szczeblu A i zamknięty na B liczy się do B, bo to B nim zarządzał,
/// gdy pieniądze stały się faktem.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatSzczebla {
    pub nazwa: String,
    pub prog: f64,
    /// suma zrealizowanych transakcji zamkniętych, gdy ten szczebel był aktywny
    pub zysk: f64,
    pub trejdy: u32,
    /// ile razy drabinka WESZŁA na ten szczebel (start przebiegu = wejście)
    pub wejscia: u32,
}

/// Jedno przełączenie szczebla — do dziennika przebiegu.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Przelaczenie {
    pub ts: Ts,
    pub z: String,
    pub na: String,
    /// saldo, które wywołało przełączenie
    pub balance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasketDump {
    pub id: u32,
    pub msg_id: i64,
    pub side: String,
    pub zone_lo: f64,
    pub zone_hi: f64,
    pub sl: Option<f64>,
    pub tps: Vec<f64>,
    pub created_ts: i64,
    pub tp_stage: usize,

    // ---------- ODCZYT, nie rekonstrukcja ----------
    //
    // Poniższe pola istnieją po to, żeby dało się PORÓWNAĆ rekonstrukcję
    // zdarzeń z biegu ekstremów z tym, co silnik naprawdę zobaczył. To jest
    // test, którego dotąd nikt nie mógł wykonać — a rekonstrukcja myli się
    // dokładnie tam, gdzie rynek przeskakuje poziom.
    /// chwila pierwszego dotknięcia każdego celu (0 = nie dotknięty)
    #[serde(default)]
    pub tp_touch_ts: Vec<i64>,
    /// CENA w tej chwili — nie poziom celu
    #[serde(default)]
    pub tp_touch_px: Vec<f64>,
    #[serde(default)]
    pub sl_touch_ts: i64,
    #[serde(default)]
    pub sl_touch_px: f64,
    /// warstwy siatki: (poziom, cena zlecenia, chwila wypełnienia, cena
    /// wypełnienia, czy anulowana)
    #[serde(default)]
    pub warstwy: Vec<WarstwaDump>,

    // ---------- OŚ PRĘDKOŚCI ----------
    //
    // Zrzut bez tych pól nie pozwalał odpowiedzieć na jedyne pytanie, które
    // się liczy: „czy koszyk szybki zarobił więcej niż wolny". Wynik koszyka
    // (`pl`) liczymy z historii brokera, nie z `realized` — koszyk domknięty
    // zbiorczo na koniec doby nigdy nie dostaje kredytu w `realized`, bo
    // silnik jest wymieniany zanim zdąży odebrać zamknięcia od brokera.
    /// numer odcinka przebiegu (rośnie przy każdym resecie dobowym) — bez
    /// niego identyfikatory koszyków z różnych dni nakładają się na siebie
    #[serde(default)]
    pub seg: u32,
    /// wynik koszyka w $ — suma zamknięć z historii brokera
    #[serde(default)]
    pub pl: f64,
    /// ile transakcji złożyło się na `pl`
    #[serde(default)]
    pub n_trades: u32,
    /// pierwsze otwarcie i ostatnie zamknięcie pozycji koszyka
    #[serde(default)]
    pub first_open_ts: i64,
    #[serde(default)]
    pub last_close_ts: i64,
    /// suma zaksięgowana przez sam silnik (dla porównania z `pl`)
    #[serde(default)]
    pub realized: f64,
    #[serde(default)]
    pub reentries: u32,
    #[serde(default)]
    pub rearms: u32,
    #[serde(default)]
    pub last_rearm_ts: i64,
    #[serde(default)]
    pub had_positions: bool,
    #[serde(default)]
    pub secured: bool,
    #[serde(default)]
    pub peak_pl_usd: f64,
    /// strefa z sygnału PRZED offsetami — do pomiaru wyjścia ze strefy
    #[serde(default)]
    pub entry_lo: f64,
    #[serde(default)]
    pub entry_hi: f64,
    #[serde(default)]
    pub state: String,
}

/// Jedna warstwa siatki w zrzucie — odczyt z realizacji zlecenia.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarstwaDump {
    pub poziom: i32,
    pub cena_zlecenia: f64,
    pub wolumen: f64,
    /// 0 = nigdy się nie wypełniła
    pub fill_ts: i64,
    pub fill_px: f64,
    pub anulowana: bool,
    pub toucher: bool,
    /// Czy szczebel kiedykolwiek się zrealizował.
    ///
    /// Osobne od `fill_ts != 0`: szczebel wypełniony i już zamknięty ma
    /// `filled = true`, a rekonstrukcja z samego `fill_ts` myli go ze
    /// szczeblem, który dopiero czeka.
    #[serde(default)]
    pub filled: bool,
}

/// Zrzut jednego koszyka wraz z wynikiem policzonym z historii brokera.
///
/// Wydzielone z `run_with_progress`, bo zrzut trzeba pobrać w DWÓCH
/// miejscach: przy każdym resecie dobowym (inaczej silnik znika razem
/// z koszykami dnia) i na końcu przebiegu.
fn zrzuc_koszyki(
    baskets: &[conduit_core::types::Basket],
    hist: &[ClosedTrade],
    seg: u32,
) -> Vec<BasketDump> {
    let mut agg: HashMap<u32, (f64, u32, i64, i64)> = HashMap::new();
    for t in hist {
        let Some(bid) = t.basket else { continue };
        let e = agg.entry(bid).or_insert((0.0, 0, i64::MAX, 0));
        e.0 += t.profit;
        e.1 += 1;
        e.2 = e.2.min(t.open_ts);
        e.3 = e.3.max(t.close_ts);
    }
    baskets
        .iter()
        .map(|b| {
            let (pl, n, fo, lc) = agg.get(&b.id).copied().unwrap_or((0.0, 0, 0, 0));
            BasketDump {
                id: b.id,
                msg_id: b.msg_id,
                side: format!("{:?}", b.side),
                zone_lo: b.zone_lo,
                zone_hi: b.zone_hi,
                sl: b.sl,
                tps: b.tps.clone(),
                created_ts: b.created_ts,
                tp_stage: b.tp_stage,
                tp_touch_ts: b.tp_touch_ts.clone(),
                tp_touch_px: b.tp_touch_px.clone(),
                sl_touch_ts: b.sl_touch_ts,
                sl_touch_px: b.sl_touch_px,
                warstwy: b
                    .levels
                    .iter()
                    .map(|g| WarstwaDump {
                        poziom: g.level,
                        cena_zlecenia: g.price,
                        wolumen: g.volume,
                        fill_ts: g.fill_ts,
                        fill_px: g.fill_px,
                        anulowana: g.cancelled,
                        toucher: g.is_toucher,
                        filled: g.filled,
                    })
                    .collect(),
                seg,
                pl,
                n_trades: n,
                first_open_ts: if fo == i64::MAX { 0 } else { fo },
                last_close_ts: lc,
                realized: b.realized,
                reentries: b.reentries,
                rearms: b.rearms,
                last_rearm_ts: b.last_rearm_ts,
                had_positions: b.had_positions,
                secured: b.secured,
                peak_pl_usd: b.peak_pl_usd,
                entry_lo: b.entry_lo,
                entry_hi: b.entry_hi,
                state: format!("{:?}", b.state),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Present only for deliberately approximate screening runs.  Absence is
    /// the backwards-compatible exact result contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approximation: Option<ApproximationInfo>,
    /// Non-rankable run: a canonical-cost receipt/configuration was incomplete.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub cost_reconciliation_required: Option<String>,
    /// Invalid/unavailable exact S/R warmup is NOT a zero-profit ranked result.
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub sr_warmup_reconciliation_required: Option<String>,
    /// An explicitly selected broker execution profile exceeds its verified scope.
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub sim_execution_reconciliation_required: Option<String>,
    /// An uninitialized/unsupported continuation is not a ranked zero result.
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub continuation_reconciliation_required: Option<String>,
    /// Explicit model-only identity; never certifies MT5 restart or account state.
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub continuation_scope: Option<String>,
    pub metrics: Metrics,
    pub equity_curve: Vec<(Ts, f64)>,
    /// KRZYWA SALDA — osobno od equity.
    ///
    /// `equity` to saldo PLUS wynik otwartych pozycji, więc sama w sobie nie
    /// pokazuje, ile zostało już zaksięgowane, a ile jeszcze wisi na rynku.
    /// Różnica między tymi dwiema liniami to dokładnie pływający wynik —
    /// i to ona mówi, czy głębokie obsunięcie było stratą zamkniętą,
    /// czy tylko przejściowym zanurzeniem otwartego koszyka.
    ///
    /// Próbkowana w tych samych chwilach co `equity_curve`, więc indeksy
    /// obu wektorów odpowiadają sobie jeden do jednego.
    ///
    /// `serde(default)` jest OBOWIĄZKOWE, nie kosmetyczne: bez niego każdy
    /// wcześniej zapisany `wyniki_*.json` przestaje się wczytywać. Dokładnie
    /// tak straciliśmy kiedyś 207 plików archiwum naraz — pole dodane bez
    /// wartości domyślnej unieważnia CAŁE archiwum, a błąd wygląda jak
    /// uszkodzony plik, nie jak zmiana schematu. Pilnuje tego test
    /// `stary_wynik_bez_pola_cancelled_wczytuje_sie`.
    #[serde(default)]
    pub balance_curve: Vec<(Ts, f64)>,
    pub daily: Vec<DayStat>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub trades: Vec<ClosedTrade>,
    /// zrzut koszyków do audytu: (id, msg_id, side, zone_lo, zone_hi, sl, tps, created_ts)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub baskets_dump: Vec<BasketDump>,
    pub ticks_processed: u64,
    pub elapsed_ms: u64,
    /// ile linii dziennika zapisano (0 = dziennik wyłączony)
    #[serde(default)]
    pub journal_lines: u64,
    /// Suma naliczonych punktów swapowych w całym przebiegu ($).
    ///
    /// Wyprowadzone na wierzch, bo swap jest kosztem NIEWIDOCZNYM w rozkładzie
    /// powodów zamknięć — a przy 92,8 % udziale kupna to koszt asymetryczny.
    /// Bez tej liczby nie da się odróżnić „swap nic nie zmienia" od „swap
    /// w ogóle się nie nalicza".
    #[serde(default)]
    pub swap_paid: f64,
    /// ile razy broker wykonał stop out (zamknięcie pojedynczej pozycji)
    #[serde(default)]
    pub stop_outs: u64,
    /// Czy przebieg został PRZERWANY przez sygnalizator postępu.
    /// Wynik jest wtedy policzony z tego, co zdążyło się wydarzyć — wolno go
    /// pokazać jako „częściowy", ale nie wolno porównywać z pełnymi przebiegami.
    #[serde(default)]
    pub cancelled: bool,
    /// ROZBICIE NA FORMATY — puste przy przebiegu jednoformatowym.
    ///
    /// Bez tego „portfel dwóch kanałów zarobił X" jest liczbą, z której nie da
    /// się wyciągnąć żadnej decyzji: nie wiadomo, czy drugi format dokłada, czy
    /// wisi na marginesie pierwszego.
    #[serde(default)]
    pub formaty: Vec<StatFormatu>,
    /// SYGNAŁY, KTÓRE NIE TRAFIŁY DO ŻADNEGO SILNIKA, po nazwie kanału.
    ///
    /// Osobny licznik, a nie cicha strata: zbiór z pięcioma kanałami puszczony
    /// przez łańcuch o dwóch formatach wygląda dokładnie tak samo jak zbiór
    /// dwukanałowy — różnicę widać wyłącznie tutaj.
    #[serde(default)]
    pub bez_trasy: BTreeMap<String, u64>,
    /// ROZBICIE NA SZCZEBLE DRABINKI — puste bez `--drabinka`.
    /// `serde(default)` obowiązkowe: bez niego stare `wyniki_*.json`
    /// przestają się wczytywać (patrz `balance_curve`).
    #[serde(default)]
    pub szczeble: Vec<StatSzczebla>,
    /// Dziennik przełączeń drabinki, w kolejności zdarzeń.
    #[serde(default)]
    pub przelaczenia: Vec<Przelaczenie>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApproximationInfo {
    pub schema: String,
    pub method: String,
    pub requested_stride: usize,
    pub raw_rows: u64,
    pub observed_rows: u64,
    pub observed_pct: f64,
    pub message_and_day_boundaries_forced: bool,
    pub extrema_preserved: bool,
    pub coronation_eligible: bool,
    pub warning: String,
}

impl RunResult {
    /// Shared rejection gate for every ranking/reporting adapter. A result
    /// with incomplete execution/cost/state proof is diagnostic data, never a
    /// scored zero-profit candidate. This does not change serialization or
    /// the separately labelled user-cancelled/partial-result contract.
    pub fn reconciliation_hold(&self) -> Option<(&'static str, &str)> {
        [
            ("COST", self.cost_reconciliation_required.as_deref()),
            ("SR WARMUP", self.sr_warmup_reconciliation_required.as_deref()),
            ("SIM EXECUTION", self.sim_execution_reconciliation_required.as_deref()),
            ("CONTINUATION", self.continuation_reconciliation_required.as_deref()),
        ].into_iter().find_map(|(kind, reason)| reason.map(|reason| (kind, reason)))
    }


    /// A quick result can be ranked only inside its screening stage.  Any
    /// release/coronation adapter should require this predicate in addition
    /// to the ordinary reconciliation gate.
    pub fn coronation_eligible(&self) -> bool {
        !self.cancelled && self.approximation.is_none() && self.reconciliation_hold().is_none()
    }
}

/// Co zrobił JEDEN format w przebiegu wieloformatowym.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatFormatu {
    pub format: String,
    pub preset: String,
    /// slot numeracji koszyków — po nim rozpoznaje się transakcje tego formatu
    pub slot: u32,
    /// suma zrealizowanych transakcji z koszyków tego slotu
    pub zysk: f64,
    pub trejdy: u32,
    /// ile sygnałów silnik PRZYJĄŁ (nie ile dostał wiadomości)
    pub sygnaly: u64,
    pub wiadomosci: u64,
    pub koszyki: u32,
}

/// Sygnalizator postępu długiego przebiegu.
///
/// Dostaje LICZBĘ TICKÓW OD POPRZEDNIEGO WYWOŁANIA (przyrost, nie sumę) —
/// dzięki temu wywołujący może po prostu dodawać ją do wspólnego licznika,
/// nawet gdy kilkanaście przebiegów liczy się równolegle na różnych rdzeniach.
/// Zwrócenie `false` PRZERYWA przebieg.
pub type ProgressFn<'a> = &'a (dyn Fn(u64) -> bool + Sync + Send);

/// Co ile ticków pytamy sygnalizator. 262 144 ticka to ułamek sekundy pracy
/// silnika, a jednocześnie rzadko dość, żeby wywołanie przez wskaźnik nie
/// zjadło zysku z mapowania pliku w pamięć.
const PROGRESS_EVERY: usize = 1 << 18;

enum ReplayTickIndices {
    Exact(std::ops::Range<usize>),
    Quick { indices: std::sync::Arc<Vec<usize>>, pos: usize },
}

impl Iterator for ReplayTickIndices {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Exact(range) => range.next(),
            Self::Quick { indices, pos } => {
                let value = indices.get(*pos).copied();
                *pos += usize::from(value.is_some());
                value
            }
        }
    }
}

fn approximation_info(stride: usize, raw_rows: usize, observed_rows: usize) -> Option<ApproximationInfo> {
    (stride > 1).then(|| ApproximationInfo {
        schema: "conduit.quick-backtest.v1".into(),
        method: "causal-block-extrema: first/last + min/max BID/ASK; split at message/day boundaries".into(),
        requested_stride: stride,
        raw_rows: raw_rows as u64,
        observed_rows: observed_rows as u64,
        observed_pct: if raw_rows == 0 { 0.0 } else { observed_rows as f64 / raw_rows as f64 * 100.0 },
        message_and_day_boundaries_forced: true,
        extrema_preserved: true,
        coronation_eligible: false,
        warning: "APPROXIMATE SCREENING ONLY — rerun finalists with quick_tick_stride=1 before any crown/release decision".into(),
    })
}

/// Uruchamia pełny przebieg.
pub fn run(ticks: &TickData, messages: &[ReplayMessage], cfg: &RunConfig) -> RunResult {
    run_with_progress(ticks, messages, cfg, None)
}

/// To samo co [`run`], ale z raportowaniem postępu i możliwością przerwania.
///
/// Osobna funkcja zamiast pola w `RunConfig`, bo `RunConfig` jest `Clone`
/// i wędruje między wątkami — domknięcie z licznikiem nie ma tam czego szukać.
/// `run` deleguje tutaj z `None`, więc istnieje DOKŁADNIE JEDNA implementacja
/// pętli: nie da się poprawić błędu w jednej i zapomnieć o drugiej.
pub fn run_with_progress(
    ticks: &TickData,
    messages: &[ReplayMessage],
    cfg: &RunConfig,
    progress: Option<ProgressFn>,
) -> RunResult {
    let t_start = std::time::Instant::now();

    if cfg.sim_native_swap_cash_digits.is_some_and(|digits| digits > 8) {
        return rejected_sim_execution(cfg, "native swap cash digits must be in 0..=8".into());
    }

    if let Err(reason)=crate::continuation::static_run_supported(cfg) {
        return rejected_continuation(cfg,reason);
    }

    // Inspect future as well as initial chain legs: ON must not silently start
    // later under a different TF/history configuration that has no V2 proof.
    if cfg.drabinka.iter().flat_map(|s| &s.formaty).any(|f| {
        let c=&f.settings;
        c.sr_warmup_exact_ticks && c.trail_sr_enabled && (c.trail_sr_min_prominence_atr>0.0
            || c.trail_sr_offset_atr_mult>0.0 || c.trail_sr_offset_spread_mult>0.0)
    }) {
        return rejected_sr_warmup(cfg,"SR V2: dynamic chain/config changes are not certified".into());
    }

    let i0 = ticks.index_at(cfg.from);
    let i1 = ticks.index_at(cfg.to).min(ticks.len());
    if i1 <= i0 {
        return RunResult {
            approximation: approximation_info(cfg.quick_tick_stride, 0, 0),
            cost_reconciliation_required: None,
            sr_warmup_reconciliation_required: None,
            sim_execution_reconciliation_required: None,
            continuation_reconciliation_required: None,
            continuation_scope: None,
            metrics: Metrics {
                start_balance: cfg.start_balance,
                ..Default::default()
            },
            equity_curve: Vec::new(),
            balance_curve: Vec::new(),
            daily: Vec::new(),
            trades: Vec::new(),
            baskets_dump: Vec::new(),
            ticks_processed: 0,
            swap_paid: 0.0,
            stop_outs: 0,
            elapsed_ms: 0,
            journal_lines: 0,
            cancelled: false,
            formaty: Vec::new(),
            bez_trasy: BTreeMap::new(),
            szczeble: Vec::new(),
            przelaczenia: Vec::new(),
        };
    }

    // Znaczniki ticków są już w czasie serwera, więc dobę liczymy z offsetem 0.
    let tz = cfg.settings.session_offset();
    // Jedna lista pól brokera dla całego backtestu — patrz
    // `SimBroker::z_ustawien`. Trzy kopie tej listy kosztowały nas Z-1:
    // tryb okien gubił walidację 10016 i przywracał szczeble-widma.
    //
    // Broker opisuje RACHUNEK, więc przy wielu formatach bierze pola z
    // `cfg.settings` — tego samego kompletu, który `zbuduj_zespol` wstrzyknął
    // każdemu presetowi jako `POLA_RACHUNKU`. Dwa presety z różnym poślizgiem
    // opisywałyby dwa różne konta.
    let mut broker = SimBroker::z_ustawien(cfg.start_balance, &cfg.settings);
    let mut cost_reconciliation_required: Option<String> = None;
    let check_cost_faults = cfg.settings.closed_profit_net_costs
        || cfg.sim_native_swap_cash_digits.is_some()
        || cfg.formaty.iter().any(|f| f.settings.closed_profit_net_costs);
    let mut continuation_reconciliation_required: Option<String> = None;
    // SimBroker's offline credit is constant for a run. It supports broker
    // margin, but it is not trading PnL and must not inflate reporting curves.
    let reporting_credit = if broker.credit_balance_separate && broker.credit.is_finite() && broker.credit > 0.0 {
        broker.credit
    } else { 0.0 };
    broker.limit_price_improvement = cfg.sim_limit_price_improvement;
    broker.defer_new_pending_sl = cfg.sim_new_pending_sl_next_tick;
    broker.price_digits = ticks.price_digits();
    if let Some(digits) = cfg.sim_native_swap_cash_digits {
        if let Err(reason) = broker.set_native_swap_cash_digits(Some(digits)) {
            return rejected_sim_execution(cfg, reason);
        }
    }
    // ---------- DRABINKA ŁAŃCUCHÓW ----------
    // `drabinka_on == false` = dosłownie ścieżka dotychczasowa: `zbuduj_zespol`
    // i ani jedna nowa gałąź w pętli się nie wykonuje. Na tym stoi parytet.
    let drabinka_on = !cfg.drabinka.is_empty();
    debug_assert!(
        !drabinka_on || cfg.formaty.is_empty(),
        "--drabinka wyklucza --preset-format: szczeble noszą własne nogi"
    );
    // bieżący szczebel + flaga „format ma nogę na bieżącym szczeblu"
    let mut szczebel: usize = 0;
    let mut aktywne: Vec<bool> = Vec::new();
    let mut zespol = if drabinka_on {
        let (z, akt, rung) = zbuduj_zespol_drabinki(cfg);
        szczebel = rung;
        aktywne = akt;
        z
    } else {
        zbuduj_zespol(cfg)
    };
    // JEDEN SILNIK = widok brokera jest tożsamościowy (przepuszcza wszystko),
    // więc go NIE budujemy: i dla parytetu, i dlatego, że filtr niefiltrujący
    // kosztowałby dwie pętle po pozycjach na każdym z 54 mln ticków.
    let pojedynczy = zespol.lista.len() == 1;
    // CZY W OGÓLE ROUTOWAĆ. To jest INNE pytanie niż „ilu jest silników":
    // jeden format wybrany z wielokanałowego zbioru musi dostać wyłącznie
    // swoje wiadomości (i policzyć resztę jako pominiętą), a przebieg
    // klasyczny (`--preset`) bierze cały strumień jak przed 03.08.2026.
    let routuj = !cfg.formaty.is_empty() || drabinka_on;
    // rozliczenie zysku per szczebel: znacznik historii brokera przy ostatnim
    // przełączeniu + statystyki każdego szczebla
    let mut szczeble_stat: Vec<StatSzczebla> = cfg
        .drabinka
        .iter()
        .map(|s| StatSzczebla {
            nazwa: s.nazwa.clone(),
            prog: s.prog,
            ..Default::default()
        })
        .collect();
    if drabinka_on {
        szczeble_stat[szczebel].wejscia = 1;
    }
    let mut szczebel_mark = 0usize;
    let mut przelaczenia: Vec<Przelaczenie> = Vec::new();
    // nowe sygnały formatu, który na bieżącym szczeblu nie ma nogi
    let mut poza_szczeblem: BTreeMap<String, u64> = BTreeMap::new();
    for s in zespol.lista.iter_mut() {
        s.engine.set_run_id("bt");
    }
    // ---------- ROZGRZEWKA HISTORII RYNKU ----------
    // Patrz `RunConfig::rozgrzewka_h`. Przy 0 (domyślnie) ta gałąź nie robi
    // NIC — dosłownie: nie dotyka silników, nie czyta ticków. Na tym stoi
    // bramka parytetu.
    if cfg.rozgrzewka_h > 0 {
        let (ph, vh) = historia_z_tickow(ticks, cfg.from, cfg.rozgrzewka_h, &cfg.settings);
        // Live odbudowuje dynamiczne S/R z DOMKNIETYCH świec M1. Samo
        // `set_market_history` zasila wyłącznie filtr reżimu/zmienność i nie
        // dotyka `StanSr`, więc dotychczas cropped backtest zaczynał S/R od
        // zera, choć pełne okno miało już gotową strukturę. Agregujemy raz,
        // bo ticki rynku są wspólne dla wszystkich formatów; każdy silnik
        // niezależnie sprawdza wymagane pokrycie swojego presetu.
        let sr_bars = swiece_m1_sr_z_tickow(ticks, cfg.from, cfg.rozgrzewka_h);
        let exact_active = zespol.lista.iter().any(|s| s.engine.sr_warmup_exact_active());
        if exact_active && drabinka_on {
            return rejected_sr_warmup(cfg, "SR V2: dynamic chain/config changes are not certified".into());
        }
        let exact_snapshot = if exact_active {
            match crate::sr_warmup::snapshot_from_tickdata(ticks,
                poczatek_rozgrzewki(cfg.from, cfg.rozgrzewka_h).max(0), cfg.from) {
                Ok(snapshot) => Some(snapshot),
                Err(reason) => return rejected_sr_warmup(cfg, reason),
            }
        } else { None };
        for s in zespol.lista.iter_mut() {
            s.engine.set_market_history(ph.clone(), vh.clone());
            if s.engine.sr_warmup_exact_active() {
                let snapshot = exact_snapshot.as_ref().expect("active V2 producer");
                if let Err(reason) = rozgrzej_dynamiczne_sr_v2(&mut s.engine, snapshot) {
                    return rejected_sr_warmup(cfg, reason);
                }
            } else {
                rozgrzej_dynamiczne_sr(&mut s.engine, &sr_bars);
            }
        }
    } else if zespol.lista.iter().any(|s| s.engine.sr_warmup_exact_active()) {
        return rejected_sr_warmup(cfg, "SR V2 requires an explicit nonzero warmup range".into());
    }
    // The model starts empty; bind only after warmup and before any signal.
    let continuation_scope=match crate::continuation::initialize_fresh(&mut zespol,&mut broker) {
        Ok(scope)=>scope,
        Err(reason)=>return rejected_continuation(cfg,reason),
    };
    // sygnały, dla których żaden silnik nie handluje ich kanałem
    let mut bez_trasy: BTreeMap<String, u64> = BTreeMap::new();
    // LICZNIKI PER FORMAT ZBIERANE PRZEZ CAŁY PRZEBIEG.
    //
    // W trybie dziennym silniki są WYMIENIANE co dobę, więc `stats.signals`
    // na końcu opisuje wyłącznie ostatni dzień — a ten bywa pusty. Bez tych
    // sum rozbicie na formaty pokazywałoby „2 koszyki" przy dwustu
    // transakcjach i wyglądałoby na usterkę routingu.
    let mut narosle: Vec<(u64, u64, u32)> = vec![(0, 0, 0); zespol.lista.len()];
    // archiwum koszyków zbierane przez wszystkie odcinki przebiegu
    let mut arch: Vec<BasketDump> = Vec::new();
    let mut hist_mark = 0usize;
    let mut seg = 0u32;

    // ---------- dziennik zdarzeń ----------
    // Zdarzenia zabieramy z silnika porcjami i zapisujemy strumieniowo.
    // Bufor w rdzeniu ma sufit, więc nieodebrane zdarzenia by przepadły —
    // dlatego opróżniamy go regularnie, a nie na końcu przebiegu.
    let mut dump = match (&cfg.journal_path, cfg.settings.journal_enabled) {
        (Some(p), true) => match JournalDump::create(p, cfg.settings.journal_text_mirror) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("dziennik: nie udało się otworzyć {}: {e}", p.display());
                None
            }
        },
        _ => None,
    };
    if dump.is_none() {
        // Bez odbiorcy dziennik jest tylko kosztem: wyłączamy go w silniku,
        // żeby sweep po stu presetach nie płacił za zdarzenia, których nikt
        // nigdy nie przeczyta.
        for s in zespol.lista.iter_mut() {
            s.engine.journal.cfg.enabled = false;
        }
    }
    let jtz = cfg.settings.server_tz_offset_ms;

    // Wiadomości w oknie, przesunięte o DWIE rzeczy naraz:
    //  * `msg_offset()` — różnicę zegarów (Telegram w UTC, ticki w czasie
    //    serwera). Bez tego silnik dostaje sygnał razem ze strumieniem cen
    //    sprzed trzech godzin i wchodzi po kursie z przeszłości, znając już
    //    strefę — czysty look-ahead;
    //  * `exec_latency_ms` — modelowane opóźnienie od wiadomości do zlecenia.
    let lat = cfg.settings.msg_offset() + cfg.settings.exec_latency_ms;
    let mut msgs: Vec<&ReplayMessage> = messages
        .iter()
        .filter(|m| m.ts + lat >= cfg.from && m.ts + lat < cfg.to)
        .collect();
    msgs.sort_by_key(|m| m.ts);
    let mut mi = 0usize;

    // QUICK BACKTEST: force every effective message-arrival row and every
    // first row of a broker day.  The extrema selector splits at those rows,
    // which is the key causal guarantee: a freshly created order cannot use a
    // low/high from the earlier part of its raw N-row block.
    let (tick_indices, quick_selected_total) = if cfg.quick_tick_stride > 1 {
        let mut forced = Vec::with_capacity(msgs.len() + 128);
        for m in &msgs {
            let index = ticks.index_at(m.ts.saturating_add(lat));
            if index >= i0 && index < i1 { forced.push(index); }
        }
        let first_day = day_of(ticks.ts(i0), tz);
        let last_day = day_of(ticks.ts(i1 - 1), tz);
        for day in (first_day + 1)..=last_day {
            let boundary = day.saturating_mul(86_400_000).saturating_sub(tz);
            let index = ticks.index_at(boundary);
            if index >= i0 && index < i1 { forced.push(index); }
        }
        forced.sort_unstable();
        forced.dedup();
        let indices = ticks.quick_extrema_indices(i0, i1, cfg.quick_tick_stride, &forced);
        let count = indices.len();
        (ReplayTickIndices::Quick { indices, pos: 0 }, count)
    } else {
        (ReplayTickIndices::Exact(i0..i1), i1 - i0)
    };

    let source = SourceKey::new(-100_1874_553_201, None);
    // The ordinary export backtest deliberately bypasses this.  A
    // live-backtest enables it and therefore receives Telegram redeliveries
    // through the same bounded, byte-exact gate as `app::live` on the VPS.
    let mut live_telegram_ingress = ContentMemory::new();

    let mut equity_curve: Vec<(Ts, f64)> = Vec::with_capacity(4096);
    let mut balance_curve: Vec<(Ts, f64)> = Vec::with_capacity(4096);
    let mut daily: Vec<DayStat> = Vec::new();
    let mut all_trades: Vec<ClosedTrade> = Vec::new();

    let mut cur_day = i64::MIN;
    let mut day_start_eq = cfg.start_balance;
    let mut day_peak_eq = cfg.start_balance;
    let mut day_dd: f64 = 0.0;
    let mut day_trades = 0u32;
    let mut day_signals = 0u32;
    let mut last_curve_ts = 0i64;
    let mut max_open_risk: f64 = 0.0;
    // Ryzyko względem equity W TYM MOMENCIE. Przy compoundingu wartość
    // bezwzględna rośnie razem z kontem, więc odnoszenie jej do kapitału
    // STARTOWEGO daje bezsensowne setki tysięcy procent.
    let mut max_open_risk_rel: f64 = 0.0;
    let mut max_floating_loss: f64 = 0.0;
    let mut max_open_pos: u32 = 0;
    let mut signals_taken_prev = 0u64;
    // LEJEK W PRZÓD (audyt poz. 20): świeże wiadomości z akcją wejścia
    // (Entry|MarketOpen), liczone w pętli PRZED routingiem i wszystkimi
    // bramkami. Licznik jest własnością PRZEBIEGU (zmienna pętli), więc
    // wymiana dobowa ani przełączenie szczebla drabinki go nie zerują —
    // mianownik lejka, którego dotąd nie było: 308 z 890 sygnałów OMEGA-X2
    // wychodziło z silnika bez koszyka i bez jreject, niepoliczone nigdzie.
    let mut sygnaly_wejsciowe: u64 = 0;
    // ARCHIWUM COMPOUNDINGU (audyt poz. 21): silnik wycina martwe koszyki
    // (>500 szt., starsze niż 7 dni) ZANIM przebieg bez resetu dobowego
    // zrobi jedyny zrzut na końcu — O91 widział 105 z 807 koszyków. Drenaż
    // dobowy do mapy po id trzyma każdy koszyk w stanie z ostatniego dnia
    // jego życia; wpis odświeża się, dopóki koszyk żyje w silniku.
    let mut arch_comp: BTreeMap<u32, BasketDump> = BTreeMap::new();
    // ostatni indeks zgłoszony sygnalizatorowi — raportujemy PRZYROSTY
    let mut last_reported = i0;
    let mut cancelled = false;
    // dokąd realnie doszliśmy; bez przerwania równa się `i1`, więc wynik
    // przebiegu do końca jest CO DO BITU taki sam jak przed dodaniem hooka
    let mut end_idx = i1;
    // D3: kwotowanie z POPRZEDNIEGO obrotu pętli — jedyna cena, którą
    // wiadomość z przerwy między tickami mogła znać naprawdę. Czytane
    // wyłącznie przy `msg_kurs_sprzed_luki`; `None` na pierwszym ticku.
    let mut q_sprzed: Option<Quote> = None;
    // WIERNA KOLEJNOŚĆ ZDARZEŃ BROKERA. W terminalu tick najpierw realizuje
    // zlecenia, SL i TP, które istniały PRZED tym tickiem; dopiero potem EA
    // oraz Conduit mogą zareagować na wiadomość. Historyczna ścieżka runnera
    // robiła odwrotnie i pozwalała zleceniu utworzonemu z wiadomości zużyć
    // tick, który już doprowadził do jej obsługi. Oś jest wspólna z live i
    // domyślnie wyłączona, więc stare wyniki pozostają odtwarzalne 1:1.
    let causal_tick_before_messages = cfg.settings.live_tick_order_strict;

    let mut quick_observed = 0usize;
    for i in tick_indices {
        quick_observed += 1;
        // ---------- postęp i przerwanie ----------
        if progress.is_some() && i - last_reported >= PROGRESS_EVERY {
            let delta = (i - last_reported) as u64;
            last_reported = i;
            // `unwrap` bezpieczne: warunek wyżej sprawdził `is_some`
            if !(progress.unwrap())(delta) {
                cancelled = true;
                end_idx = i + 1;
                break;
            }
        }

        let q = ticks.quote(i);

        // ---------- granica doby ----------
        let day = day_of(q.ts, tz);
        if day != cur_day {
            // NOC KOSZTUJE (S-3). Zanim policzymy wynik doby i cokolwiek
            // zamkniemy, broker musi zobaczyć PIERWSZY kurs nowej doby:
            //  * nalicza punkty swapowe za przekroczoną północ,
            //  * odświeża cenę, więc zamknięcie idzie po kursie NOWEGO dnia,
            //    a nie po ostatnim kursie poprzedniego.
            // Bez tego tryb „dzień po dniu" nie płacił ani swapu, ani luki
            // nocnej — a to unieważnia KAŻDY pomiar strategii trzymającej
            // przez północ, w tym runnery po RISK FREE.
            //
            // Legacy repeats broker execution below. Under B15 this same
            // source-row id is executed once, including across the day loop;
            // its new SL must not become eligible until another physical row.
            //
            // D4 (`runner_ksiegowanie_v2`): pełny `on_quote` robił tu jednak
            // DWIE rzeczy ponad to, o co chodziło — egzekwował PRZED
            // komunikatami nocnymi (odwrotnie niż na zwykłym ticku) i przy
            // `daily_reset` liczył ten sam tick trzy razy w licznikach
            // poziomu marginesu. `mark` daje sam swap i cenę, czyli dokładnie
            // to, czego wymaga „noc kosztuje".
            //
            // D5: zamknięcia z TEGO bloku (nocne SL/TP z luki oraz `EodFlat`
            // niżej) nie trafiały do `DayStat::trades` ŻADNEGO dnia: pierwsze
            // padały przed `daily.push`, drugie po nim, a licznik zaraz potem
            // wracał do zera. Zapamiętujemy więc długość historii PRZED całym
            // blokiem i doliczamy przyrost do dnia ZAMYKANEGO.
            let hist_granica = broker.history.len();
            let zamykamy_dobe = cur_day != i64::MIN;
            if cur_day != i64::MIN {
                if cfg.settings.runner_ksiegowanie_v2 {
                    broker.mark(q);
                } else {
                    broker.on_tape_quote(q, i);
                }
            }
            if cur_day != i64::MIN {
                daily.push(DayStat {
                    day: cur_day,
                    date: fmt_day(cur_day),
                    start_equity: day_start_eq,
                    end_equity: broker.equity(),
                    profit: broker.equity() - day_start_eq,
                    max_dd: day_dd,
                    trades: day_trades,
                    signals: day_signals,
                });
            }
            cur_day = day;

            if (cfg.daily_reset || cfg.flat_na_dobie) && !daily.is_empty() {
                // zamknij wszystko i (przy `daily_reset`) wróć do kwoty startowej — badamy każdy
                // dzień niezależnie, tak jak przy codziennym starcie od $200
                //
                // Przy wielu formatach KAŻDY silnik zamyka WYŁĄCZNIE swoje.
                // Gdyby pierwszy zamknął cudze, drugi zobaczyłby zniknięcie
                // pozycji, których nigdy nie otwierał, i zaliczyłby cudze
                // straty do własnej serii.
                if pojedynczy {
                    zespol.lista[0].engine.close_everything(
                        &mut broker,
                        q.ts,
                        CloseReason::EodFlat,
                    );
                } else {
                    zespol.kazdy(&mut broker, |e, w| {
                        e.close_everything(w, q.ts, CloseReason::EodFlat)
                    });
                }
                // D4: drugie przejście tego samego ticku przez pełny
                // `on_quote`. Po `close_everything` nie ma już czego wypełniać
                // ani zamykać, a liczniki poziomu marginesu dostawały drugą
                // próbkę z tej samej chwili.
                if cfg.settings.runner_ksiegowanie_v2 {
                    broker.mark(q);
                } else {
                    broker.on_tape_quote(q, i);
                }
                // D6: EODFLAT MUSI ZOSTAĆ W SILNIKU, KTÓRY GO ZLECIŁ.
                //
                // `close_everything` wkłada zamknięcia do kolejki brokera,
                // a kolejkę czyta dopiero `on_tick`. Bez drenażu odbierał je
                // ŚWIEŻY silnik w pierwszym ticku nowej doby: podbijał sobie
                // `loss_streak` cudzymi stratami i przy `streak_pause_n > 0`
                // ustawiał pauzę na poranek. W trybie dziennym — czyli
                // w głównym kryterium wyboru presetu — znaczyło to, że dzień
                // zaczynał się z karą za wczorajsze domknięcie.
                //
                // Transakcje NIE GINĄ: `broker.history` ma je już zapisane,
                // drenujemy wyłącznie poczekalnię dla silnika.
                if cfg.settings.runner_ksiegowanie_v2 {
                    let _ = conduit_core::broker::Broker::drain_closed(&mut broker);
                }
                if cfg.daily_reset {
                    broker.balance = cfg.start_balance;
                    // bez tego jeden wyzerowany dzień blokowałby resztę przebiegu
                    broker.blown = false;
                }
                // Resetujemy KONTO, nie WIEDZĘ O RYNKU. Historia ceny jest
                // własnością rynku, a nie salda: bot na żywo nie zapomina
                // o północy, jak wyglądały ostatnie 72 h. Bez przeniesienia
                // tych buforów filtr reżimu (okno godzinowe) nigdy nie zbierał
                // dość próbek i w trybie „dzień po dniu" był MARTWY — a to
                // właśnie ten tryb jest głównym kryterium wyboru presetu.
                // DZIENNIK PRZED WYMIANĄ SILNIKA. Bufor zdarzeń jest polem
                // silnika, więc `engine = Engine::new(...)` wyrzucał razem
                // z nim wszystko, czego jeszcze nie zapisano na dysk — a zapis
                // następuje dopiero po uzbieraniu 4096 zdarzeń, czego jeden
                // dzień handlowy nie osiąga. Efekt: `--journal` w trybie
                // „dzień po dniu" dawał plik o zerowej długości, czyli raport
                // udający, że bot nie zrobił nic.
                if let Some(d) = dump.as_mut() {
                    for s in zespol.lista.iter_mut() {
                        let mut evs = s.engine.drain_journal();
                        let _ = d.push(&mut evs, jtz);
                    }
                }
                // ZRZUT KOSZYKÓW PRZED WYMIANĄ SILNIKA. Bez tego zrzut
                // z trybu „dzień po dniu" zawierał zawsze koszyki wyłącznie
                // z ostatniej doby — a przy pustej ostatniej dobie ZERO
                // koszyków, czyli plik udający, że bot nie handlował.
                for s in zespol.lista.iter() {
                    arch.extend(zrzuc_koszyki(
                        &s.engine.baskets,
                        &broker.history[hist_mark..],
                        seg,
                    ));
                }
                hist_mark = broker.history.len();
                seg += 1;
                for (idx, s) in zespol.lista.iter_mut().enumerate() {
                    let engine = &mut s.engine;
                    narosle[idx].0 += engine.stats.signals;
                    narosle[idx].1 += engine.stats.messages;
                    narosle[idx].2 += engine.baskets.len() as u32;
                    let hist = engine.market_history();
                    // DIAGNOSTYKA: liczniki odrzuceń są własnością PRZEBIEGU,
                    // nie doby. Bez przeniesienia zrzut pokazywałby wyłącznie
                    // ostatni dzień — a ten bywa pusty.
                    let odrz = std::mem::take(&mut engine.odrzuty);
                    // Rejestr odrzuconych WEJŚĆ (Pakiet E3) jedzie tą samą
                    // drogą i z tego samego powodu: wycena filtrów ma dotyczyć
                    // całego przebiegu, nie ostatniej doby.
                    let odrz_w = std::mem::take(&mut engine.odrzucone_wejscia);
                    // HISTORIA REŻIMU idzie przez reset razem z historią ceny.
                    // Bramka reżimu piramidy czyta udział koszyków-przelotów
                    // z ostatnich N ocen; koszyki giną przy wymianie silnika,
                    // więc bez przeniesienia tej listy bramka w trybie
                    // „dzień po dniu" jest MARTWA — a jej sygnaturą jest wynik
                    // identyczny co do centa z wariantem niebramkowanym.
                    let rezim = std::mem::take(&mut engine.regime_hist);
                    // PROFIL ZMIENNOŚCI PO GODZINACH przechodzi przez reset
                    // z dokładnie tego samego powodu co historia reżimu:
                    // kubełek godzinowy potrzebuje trzech ZAMKNIĘTYCH godzin,
                    // a jedna doba daje ich dokładnie jedną. Bez tego
                    // odsezonowanie w trybie dziennym byłoby martwe.
                    // Przy `vol_size_mode = Off` to jest przeniesienie
                    // wartości domyślnej — czyli nic.
                    let zmien = engine.stan_zmiennosci();
                    // STAN TRAILINGU S/R (agregator 1M + potwierdzone swingi)
                    // przechodzi przez reset z tego samego powodu co profil
                    // zmienności: to WIEDZA O RYNKU, nie stan konta. Bez tego
                    // oś S/R w trybie dziennym zaczynałaby dobę od pustej
                    // struktury — po cichu słabsza niż w compoundingu.
                    // Przy `trail_sr_enabled = false` przenosimy wartość
                    // domyślną — czyli nic.
                    let sr = engine.stan_sr();
                    // Liczniki PENDING-RELOT też są własnością przebiegu, nie doby.
                    // Bez przeniesienia raport z trybu dziennego pokazywał same
                    // zera i wyglądało to na „relot nie działa", choć wynik był
                    // inny niż w bazie.
                    let rl = (
                        engine.stats.relot_up_zdarzen,
                        engine.stats.relot_down_zdarzen,
                        engine.stats.relot_up_lotow,
                        engine.stats.relot_down_lotow,
                        engine.stats.relot_down_bez_spadku,
                        engine.stats.relot_up_ponad_plan,
                        engine.stats.relot_prob,
                        engine.stats.relot_udane,
                        engine.stats.relot_odmowy,
                        engine.stats.relot_plan_pusty,
                        engine.stats.relot_plan_ok,
                        engine.stats.relot_rozjazd_lotow,
                        engine.stats.relot_szczebli,
                        engine.stats.relot_ksztalt_odmowa,
                    );
                    // Ekspozycja: licznik jest wlasnoscia PRZEBIEGU, nie doby
                    // ani szczebla drabinki — te same powody co przy relocie.
                    let ex = (
                        engine.stats.expo_max_pct,
                        engine.stats.expo_zdarzen,
                        engine.stats.expo_pend_skasowane,
                        engine.stats.expo_lotow,
                        engine.stats.expo_poz_domkniete,
                        engine.stats.expo_niedosyt,
                    );
                    // Ustawienia bierzemy Z SILNIKA, nie z `cfg`: przy wielu
                    // formatach każdy ma własne i wspólny komplet by je zrównał.
                    let ust = engine.cfg.clone();
                    // ...a razem z nimi WRACA SLOT I PUŁAP. `Engine::new`
                    // numeruje koszyki od 1, więc bez tego po pierwszej
                    // północy dwa formaty produkowałyby ten sam numer `B1`.
                    let (slot, pulapy) = (engine.slot(), engine.pulapy.clone());
                    // ...i TRYB. `tryb_auto_ea` jest własnością PRZEBIEGU,
                    // nie doby: wymiana silnika o północy nie ma prawa
                    // przełączyć bota z AUTO-EA na AUTO w połowie pomiaru.
                    // (Sam `EaRdzen` celowo NIE przechodzi — wymiana silnika
                    // jest modelem restartu, a N19 każe odtworzyć stan.)
                    let auto_ea = engine.tryb_auto_ea;
                    *engine = Engine::new(ust, cfg.start_balance);
                    engine.przypisz_slot(slot);
                    engine.pulapy = pulapy;
                    engine.tryb_auto_ea = auto_ea;
                    engine.odrzuty = odrz;
                    engine.odrzucone_wejscia = odrz_w;
                    engine.stats.relot_up_zdarzen = rl.0;
                    engine.stats.relot_down_zdarzen = rl.1;
                    engine.stats.relot_up_lotow = rl.2;
                    engine.stats.relot_down_lotow = rl.3;
                    engine.stats.relot_down_bez_spadku = rl.4;
                    engine.stats.relot_up_ponad_plan = rl.5;
                    engine.stats.relot_prob = rl.6;
                    engine.stats.relot_udane = rl.7;
                    engine.stats.relot_odmowy = rl.8;
                    engine.stats.relot_plan_pusty = rl.9;
                    engine.stats.relot_plan_ok = rl.10;
                    engine.stats.relot_rozjazd_lotow = rl.11;
                    engine.stats.relot_szczebli = rl.12;
                    engine.stats.relot_ksztalt_odmowa = rl.13;
                    engine.stats.expo_max_pct = ex.0;
                    engine.stats.expo_zdarzen = ex.1;
                    engine.stats.expo_pend_skasowane = ex.2;
                    engine.stats.expo_lotow = ex.3;
                    engine.stats.expo_poz_domkniete = ex.4;
                    engine.stats.expo_niedosyt = ex.5;
                    engine.regime_hist = rezim;
                    engine.set_stan_zmiennosci(zmien);
                    engine.set_stan_sr(sr);
                    engine.set_run_id("bt");
                    engine.journal.cfg.enabled = dump.is_some() && engine.cfg.journal_enabled;
                    engine.set_market_history(hist.0, hist.1);
                }
            }
            // DRENAŻ ARCHIWUM W COMPOUNDINGU (poz. 21). Ścieżki z wymianą
            // dobową zrzucają koszyki do `arch` wyżej; bez resetu jedyny
            // zrzut był na końcu przebiegu — a `expire_old_baskets` w silniku
            // wycina martwe koszyki starsze niż 7 dni przy >500 sztukach,
            // więc zrzut widział tylko ogon (O91: 105 zamiast 807). Wpis pod
            // tym samym id nadpisuje wczorajszy stan świeższym; koszyk
            // wycięty w środku przebiegu zostaje w mapie taki, jaki był
            // w ostatnim dniu życia. Statystyka, nie handel: silnika ani
            // brokera nie dotykamy.
            if !(cfg.daily_reset || cfg.flat_na_dobie) && zamykamy_dobe {
                for s in zespol.lista.iter() {
                    for d in zrzuc_koszyki(&s.engine.baskets, &broker.history, seg) {
                        arch_comp.insert(d.id, d);
                    }
                }
            }
            day_start_eq = broker.equity();
            day_peak_eq = day_start_eq;
            day_dd = 0.0;
            day_trades = 0;
            day_signals = 0;
            // LICZNIK ODNIESIENIA MUSI WRÓCIĆ RAZEM Z SILNIKIEM.
            //
            // W trybie dziennym silnik jest WYMIENIANY (`Engine::new` wyżej),
            // więc `engine.stats.signals` startuje od zera — a `signals_taken_prev`
            // zostawał z narosłą wartością z poprzednich dni. Warunek
            // `signals > signals_taken_prev` nie był już nigdy prawdziwy
            // i `day_signals` zatrzymywał się po pierwszym dniu.
            //
            // Objaw zmierzony 01.08.2026: pełne okno pokazywało 9 dni z sygnałami
            // przy 50 dniach Z TRANSAKCJAMI. Handel bez sygnału jest niemożliwy,
            // więc to licznik kłamał, nie silnik — ale każda analiza „ile sygnałów
            // przypada na dzień bez transakcji" była przez to bezwartościowa.
            signals_taken_prev = przyjete_sygnaly(&zespol);

            // ---------- DRABINKA + RESET DOBOWY ----------
            // Semantyka POMIARU „dzień z drabinką", zdefiniowana jawnie:
            // reset dobowy wraca do kwoty startowej ORAZ do szczebla tej
            // kwoty. Drabinka może wspiąć się WEWNĄTRZ dnia (dzień +250 od
            // 300 $ przekracza próg 500 i przełącza łańcuch), ale o północy
            // wszystko wraca do bazy. To NIE jest pomiar „drabinki
            // wielodniowej" — od niego jest compounding.
            //
            // Warunek `cfg.daily_reset && !daily.is_empty()` MUSI być lustrem
            // warunku resetu wyżej: w compoundingu saldo przechodzi przez
            // północ, więc i szczebel przechodzi — wymuszenie powrotu do bazy
            // każdej doby zamieniłoby compounding z drabinką w tryb dzienny
            // z opóźnieniem i nikt by tego nie zauważył w tabeli.
            if drabinka_on && (cfg.daily_reset || cfg.flat_na_dobie) && !daily.is_empty() {
                // Przy `flat_na_dobie` saldo ROŚNIE, więc szczebel liczy się
                // z bieżącego salda; przy `daily_reset` saldo wraca do
                // startowego i szczebel jest stały przez cały przebieg.
                let saldo_progu = if cfg.daily_reset {
                    cfg.start_balance
                } else {
                    broker.balance
                };
                let cel = szczebel_dla_salda(&cfg.drabinka, saldo_dla_drabinki(cfg, saldo_progu));
                if cel != szczebel {
                    for t in broker.history[szczebel_mark..].iter() {
                        szczeble_stat[szczebel].zysk += t.profit;
                        szczeble_stat[szczebel].trejdy += 1;
                    }
                    szczebel_mark = broker.history.len();
                    przelaczenia.push(Przelaczenie {
                        ts: q.ts,
                        z: cfg.drabinka[szczebel].nazwa.clone(),
                        na: cfg.drabinka[cel].nazwa.clone(),
                        balance: cfg.start_balance,
                    });
                    przelacz_szczebel(
                        &mut zespol,
                        &mut aktywne,
                        cfg,
                        cel,
                        cfg.start_balance,
                        &mut narosle,
                        &mut dump,
                        jtz,
                    );
                    szczebel = cel;
                    szczeble_stat[cel].wejscia += 1;
                    signals_taken_prev = przyjete_sygnaly(&zespol);
                }
            }

            // ---------- D5: TRANSAKCJE Z GRANICY DOBY ----------
            //
            // Wszystko, co zamknęło się w tym bloku — nocne SL/TP z luki oraz
            // `EodFlat` — należy do dnia, KTÓRY WŁAŚNIE ZAMKNĘLIŚMY, a nie do
            // nowego (tam licznik i tak startuje od zera) i nie donikąd
            // (tak było). Doliczamy przyrost historii do ostatniego wpisu.
            //
            // Świadomie tylko `trades`: `end_equity` liczy się przed
            // `EodFlat`, jak dotąd, i zmiana tamtego byłaby osobną decyzją
            // o tym, czy dzień kończy się przed domknięciem, czy po nim.
            if cfg.settings.runner_ksiegowanie_v2 && zamykamy_dobe {
                let z_granicy = broker.history.len() - hist_granica;
                if let Some(d) = daily.last_mut() {
                    d.trades += z_granicy as u32;
                }
            }
        }

        // D5b: ZAMKNIĘCIA WYWOŁANE KOMUNIKATEM TEŻ SIĘ NIE LICZYŁY.
        //
        // Znalezione przy mierzeniu wpływu D5 (OMEGA-X2, 18.08): suma
        // `DayStat::trades` po wszystkich dniach dawała 206 przy 282
        // transakcjach przebiegu — a granica doby tłumaczyła tylko 23 z tych
        // 76. Reszta to zamknięcia, które wykonał sam KOMUNIKAT: „TP1 HIT",
        // „SL HIT", „CLOSE", bank przy RISK FREE. `hist_przed` brany jest
        // niżej, PO pętli wiadomości, więc każde z nich przepadało — mimo że
        // to najczęstsza droga wyjścia z pozycji w tym bocie.
        //
        // Ta sama wada co D5 i ten sam skutek (zaniżony licznik transakcji
        // dnia w trybie dziennym), więc siedzi pod tą samą osią.
        let hist_wiad = broker.history.len();

        // W trybie przyczynowym rynek wykonuje się PRZED wiadomościami.
        // Zapamiętujemy osobno przyrost historii, aby stary licznik dni
        // (`runner_ksiegowanie_v2 = false`) nadal pomijał zamknięcia wywołane
        // komunikatem, dokładnie jak przed zmianą. Przy v2 policzymy niżej
        // cały zakres od `hist_wiad`, niezależnie od kolejności.
        let mut strict_tick_trades = 0usize;
        if causal_tick_before_messages {
            let hist_przed_tickiem = broker.history.len();
            let (_filled, _closed) = broker.on_tape_quote(q, i);
            if pojedynczy {
                zespol.lista[0].engine.on_tick_received(&mut broker, &q, q.ts - cfg.settings.server_tz_offset_ms);
            } else {
                zespol.przelicz_obce(&broker, None);
                zespol.kazdy(&mut broker, |e, w| e.on_tick_received(w, &q, q.ts - cfg.settings.server_tz_offset_ms));
            }
            strict_tick_trades = broker.history.len() - hist_przed_tickiem;
        }

        // ---------- wiadomości, których czas już nadszedł ----------
        while mi < msgs.len() && msgs[mi].ts + lat <= q.ts {
            let m = msgs[mi];
            mi += 1;
            if cfg.live_telegram_ingress {
                // Explicit NON_HISTORICAL benchmark control: model only the
                // Telegram listener's volatile ingress memory being lost on a
                // process restart.  It is never forwarded to parser/Engine.
                if m.kanal == "__CONDUIT_CONTROL__"
                    && m.text == "__CONDUIT_LIVEBACKTEST_INGRESS_RESTART__"
                {
                    live_telegram_ingress = ContentMemory::new();
                    continue;
                }
                let ingress_message = IncomingMessage {
                    ts: q.ts,
                    source: source.clone(),
                    source_name: cfg.source_name.clone(),
                    msg_id: m.msg_id,
                    reply_to: m.reply_to,
                    edit_of: m.edit_of,
                    text: m.text.clone(),
                };
                if live_telegram_ingress.duplikat_tresci(&ingress_message) {
                    continue;
                }
                // Production evaluates age at receipt, before replacing the
                // Telegram timestamp with the latest market tick.  The raw
                // replay stores both facts separately, so this is the same
                // five-minute opening gate as `app/live.rs`.  Ordinary export
                // replays have no publication timestamp and remain untouched.
                if let Some(published_at) = m.telegram_published_ts {
                    let stale = stale_entry_age_minutes(m.ts, published_at, 5.0)
                        .filter(|_| opens_basket(&m.text, m.edit_of));
                    if stale.is_some() {
                        continue;
                    }
                }
            }
            // LEJEK W PRZÓD (poz. 20): licznik mianownika, zanim wiadomość
            // zobaczy routing, szczebel drabinki czy jakąkolwiek bramkę
            // silnika. Tylko wiadomość ŚWIEŻA — edycja i odpowiedź nie
            // zakładają nowego koszyka, więc liczyłyby ten sam sygnał
            // drugi raz. Samo `parse` jest czyste: nie dotyka ani brokera,
            // ani silników, więc handel zostaje co do centa.
            if m.edit_of.is_none()
                && m.reply_to.is_none()
                && conduit_core::parser::parse(&m.text).iter().any(|s| {
                    matches!(
                        s,
                        conduit_core::parser::Signal::Entry(_)
                            | conduit_core::parser::Signal::MarketOpen { .. }
                    )
                })
            {
                sygnaly_wejsciowe += 1;
            }
            // broker musi znać cenę PRZED obsługą wiadomości
            //
            // D3: KTÓRĄ cenę. Wiadomość, która przyszła MIĘDZY tickami,
            // dostawała kurs ticku NASTĘPNEGO — czyli w przerwie dobowej
            // i weekendowej podejmowała decyzję, znając już cenę otwarcia
            // PO LUCE. To jest wiedza z przyszłości i systematycznie zawyża
            // backtest; na żywo `live.rs` robi odwrotnie (`im.ts` = ostatni
            // znany znacznik). Przy osi podajemy kwotowanie z poprzedniego
            // obrotu pętli — pierwsza wiadomość przebiegu nie ma poprzednika,
            // więc dostaje bieżące, jak dotąd.
            broker.q = if cfg.settings.msg_kurs_sprzed_luki {
                q_sprzed.unwrap_or(q)
            } else {
                q
            };
            let im = IncomingMessage {
                ts: q.ts,
                source: source.clone(),
                source_name: cfg.source_name.clone(),
                msg_id: m.msg_id,
                reply_to: m.reply_to,
                edit_of: m.edit_of,
                text: m.text.clone(),
            };
            if !routuj {
                // Przebieg klasyczny (`--preset`) bierze CAŁY strumień,
                // niezależnie od pola `kanal` — tak samo, jak brał go przed
                // 03.08.2026. To jest warunek parytetu.
                zespol.lista[0].engine.on_message_received(&mut broker, &im, m.ts + lat - cfg.settings.server_tz_offset_ms);
            } else {
                // ROUTING PO KANALE. Nazwa formatu z sygnału musi trafić na
                // silnik o tej nazwie; brak trafienia to POLICZONA strata,
                // nie cisza (`bez_trasy` ląduje w metrykach i na ekranie).
                match zespol.indeks_formatu(&m.kanal) {
                    Some(i) => {
                        // DRABINKA: format bez nogi na bieżącym szczeblu =
                        // ZARZĄDZAJ-NIE-OTWIERAJ. Świeża wiadomość z wejściem
                        // (nowy sygnał) jest blokowana i POLICZONA; edycje
                        // i odpowiedzi przechodzą, bo odnoszą się do rozmowy
                        // istniejącego koszyka (RISK FREE, korekty TP, OUT).
                        // Ten sam kontrakt co adopcja po restarcie na żywo:
                        // stare pozycje dożywają końca pod swoim silnikiem,
                        // nowych ekspozycji format nie bierze.
                        if drabinka_on && !aktywne[i] {
                            let nowy_sygnal = m.edit_of.is_none()
                                && m.reply_to.is_none()
                                && conduit_core::parser::parse(&im.text).iter().any(|s| {
                                    matches!(
                                        s,
                                        conduit_core::parser::Signal::Entry(_)
                                            | conduit_core::parser::Signal::MarketOpen { .. }
                                    )
                                });
                            if nowy_sygnal {
                                *poza_szczeblem
                                    .entry(zespol.lista[i].format.clone())
                                    .or_insert(0) += 1;
                                continue;
                            }
                        }
                        if pojedynczy {
                            zespol.lista[0].engine.on_message_received(&mut broker, &im, m.ts + lat - cfg.settings.server_tz_offset_ms);
                        } else {
                            // Kierunek sygnału trzeba znać PRZED bramką, żeby
                            // pułap „nie otwieraj przeciwnie do innego formatu"
                            // miał czego pilnować. Ta sama kolejność co
                            // w `live.rs::skieruj`.
                            let strona = conduit_core::parser::parse(&im.text)
                                .into_iter()
                                .find_map(|s| match s {
                                    conduit_core::parser::Signal::Entry(e) => Some(e.side),
                                    conduit_core::parser::Signal::MarketOpen { side } => Some(side),
                                    _ => None,
                                });
                            zespol.przelicz_obce(&broker, strona);
                            zespol.z_widokiem(i, &mut broker, |e, w| e.on_message_received(w, &im, m.ts + lat - cfg.settings.server_tz_offset_ms));
                        }
                    }
                    None => {
                        let klucz = if m.kanal.is_empty() {
                            "(bez kanału)".to_string()
                        } else {
                            m.kanal.clone()
                        };
                        *bez_trasy.entry(klucz).or_insert(0) += 1;
                    }
                }
            }
            let teraz = przyjete_sygnaly(&zespol);
            if teraz > signals_taken_prev {
                day_signals += (teraz - signals_taken_prev) as u32;
                signals_taken_prev = teraz;
            }
        }

        // ---------- symulacja brokera i zarządzanie ----------
        //
        // Licznik transakcji dnia bierze się z PRZYROSTU HISTORII, a nie ze
        // zwrotu `on_quote` (S-5). Tamten zwraca wyłącznie zamknięcia
        // BROKERSKIE (stop, cel, stop out), a wyjścia silnikowe — żniwo,
        // stagnacja, OUT AT ENTRY, wygaśnięcie, bank RISK FREE,
        // `close_everything` — dzieją się w `engine.on_tick` i nie były
        // liczone. Skutek: dni z samymi wyjściami silnikowymi wyglądały na
        // bezczynne i WYPADAŁY z rozkładu „% dni na plusie", czyli z kolumny,
        // po której rankujemy presety.
        // D5b: przy osi licznik startuje SPRZED pętli wiadomości, więc obejmuje
        // także wyjścia zlecone komunikatem.
        if causal_tick_before_messages {
            day_trades += if cfg.settings.runner_ksiegowanie_v2 {
                (broker.history.len() - hist_wiad) as u32
            } else {
                strict_tick_trades as u32
            };
        } else {
            let hist_przed = if cfg.settings.runner_ksiegowanie_v2 {
                hist_wiad
            } else {
                broker.history.len()
            };
            let (_filled, _closed) = broker.on_tape_quote(q, i);
            if pojedynczy {
                zespol.lista[0].engine.on_tick_received(&mut broker, &q, q.ts - cfg.settings.server_tz_offset_ms);
            } else {
                // Obce obciążenie odświeżamy RAZ na obrót pętli, przed turą
                // silników — dokładnie tak jak `live.rs`. Bez tego pułapy
                // widziałyby wyłącznie własny silnik i nie chroniłyby przed niczym.
                zespol.przelicz_obce(&broker, None);
                zespol.kazdy(&mut broker, |e, w| e.on_tick_received(w, &q, q.ts - cfg.settings.server_tz_offset_ms));
            }
            day_trades += (broker.history.len() - hist_przed) as u32;
        }

        if check_cost_faults {
            let fault=broker.cost_reconciliation_required().map(str::to_owned)
                .or_else(||zespol.lista.iter().find_map(|s|s.engine.cost_reconciliation_required.clone()));
            if fault.is_some() {
                cost_reconciliation_required=fault;
                // This quote was observed, but the suffix after it was not.
                // Keep HOLD; only its diagnostic extent/counters change.
                end_idx=i+1;
                break;
            }
        }
        if cfg.settings.restore_strategy_continuation {
            if let Some(reason)=crate::continuation::review_reason(&zespol) {
                continuation_reconciliation_required=Some(reason);
                end_idx=i+1;
                break;
            }
        }

        // ---------- DRABINKA: czy saldo przekroczyło próg ----------
        // Saldo zmienia się wyłącznie przy rozliczeniu transakcji, więc
        // sprawdzenie raz na tick wystarcza; koszt to dwa porównania.
        // Progi liczą się po BALANCE, nie equity — pływający wynik nie ma
        // prawa trzepotać łańcuchami.
        if drabinka_on {
            // ŚRODKI WŁASNE, nie surowe saldo: bonus kredytowy jest poduszką
            // marginesową, a nie dorobkiem — próg ma widzieć to samo, co
            // `podstawa_lota()`.
            let bal = saldo_dla_drabinki(cfg, broker.balance);
            let mut cel = szczebel_dla_salda(&cfg.drabinka, bal);
            if cel < szczebel {
                // HISTEREZA schodzenia: dopiero saldo < prog · (1 − h/100).
                // Przy h = 0 (domyślnie w backteście) schodzimy od razu.
                let prog = cfg.drabinka[szczebel].prog * (1.0 - cfg.drabinka_histereza_pct / 100.0);
                if bal >= prog {
                    cel = szczebel;
                }
            }
            if cel != szczebel {
                // rozlicz transakcje zamknięte na schodzącym szczeblu
                for t in broker.history[szczebel_mark..].iter() {
                    szczeble_stat[szczebel].zysk += t.profit;
                    szczeble_stat[szczebel].trejdy += 1;
                }
                szczebel_mark = broker.history.len();
                przelaczenia.push(Przelaczenie {
                    ts: q.ts,
                    z: cfg.drabinka[szczebel].nazwa.clone(),
                    na: cfg.drabinka[cel].nazwa.clone(),
                    balance: bal,
                });
                przelacz_szczebel(
                    &mut zespol,
                    &mut aktywne,
                    cfg,
                    cel,
                    bal,
                    &mut narosle,
                    &mut dump,
                    jtz,
                );
                szczebel = cel;
                szczeble_stat[cel].wejscia += 1;
                signals_taken_prev = przyjete_sygnaly(&zespol);
            }
        }

        // ---------- pomiar OTWARTEGO RYZYKA ----------
        // Liczone rzadko (co ~1 s), bo to pętla po pozycjach.
        if q.ts - last_curve_ts >= 1000 {
            let mut risk = 0.0;
            let mut floating = 0.0;
            for p in broker.positions() {
                if let Some(sl) = p.sl {
                    risk += (p.open_price - sl).abs() * XAU_CONTRACT * p.volume;
                }
                floating += p.profit_usd(&q);
            }
            if risk > max_open_risk {
                max_open_risk = risk;
            }
            let eq_now = broker.equity().max(1.0);
            let rel = risk / eq_now * 100.0;
            if rel > max_open_risk_rel {
                max_open_risk_rel = rel;
            }
            if floating < max_floating_loss {
                max_floating_loss = floating;
            }
            let n = broker.positions().len() as u32;
            if n > max_open_pos {
                max_open_pos = n;
            }
        }

        // ---------- krzywa equity ----------
        let eq = broker.equity();
        if eq > day_peak_eq {
            day_peak_eq = eq;
        }
        let d = day_peak_eq - eq;
        if d > day_dd {
            day_dd = d;
        }
        if cfg.curve_interval_ms > 0 && q.ts - last_curve_ts >= cfg.curve_interval_ms {
            last_curve_ts = q.ts;
            equity_curve.push((q.ts, eq));
            balance_curve.push((q.ts, broker.balance));
        }

        if let Some(d) = dump.as_mut() {
            for s in zespol.lista.iter_mut() {
                if s.engine.journal.len() >= 4096 {
                    let mut evs = s.engine.drain_journal();
                    let _ = d.push(&mut evs, jtz);
                }
            }
        }

        // WYZEROWANE KONTO (S-4). W trybie „każdy dzień osobno" nie wolno
        // urywać CAŁEGO przebiegu — dzień ma zostać zaksięgowany jako strata
        // i następny startuje od kwoty początkowej. Urwanie zamieniało jeden
        // zły dzień w brak wszystkich kolejnych, czyli w cichy, korzystny
        // dla wyniku obcięcie próby.
        // D3: ten kurs jest od następnego obrotu „ceną sprzed luki".
        // Zapis jest bezwarunkowy i kosztuje jedno przypisanie — gałąź
        // czytająca i tak istnieje tylko przy włączonej osi.
        q_sprzed = Some(q);

        if broker.blown && !cfg.daily_reset {
            break;
        }
    }

    // domknięcie ostatniej doby
    if cur_day != i64::MIN {
        daily.push(DayStat {
            day: cur_day,
            date: fmt_day(cur_day),
            start_equity: day_start_eq,
            end_equity: broker.equity(),
            profit: broker.equity() - day_start_eq,
            max_dd: day_dd,
            trades: day_trades,
            signals: day_signals,
        });
    }
    equity_curve.push((ticks.ts(end_idx.saturating_sub(1)), broker.equity()));
    balance_curve.push((ticks.ts(end_idx.saturating_sub(1)), broker.balance));

    // domknięcie licznika postępu: reszta ticków, która nie złożyła się na
    // pełną porcję. Bez tego globalny pasek postępu zatrzymywałby się tuż
    // przed setką i nigdy jej nie dobijał.
    if let Some(cb) = progress {
        if end_idx > last_reported {
            cb((end_idx - last_reported) as u64);
        }
    }

    // pełna historia transakcji (kolejkę `drain_closed` konsumuje silnik)
    all_trades.extend(broker.history.iter().cloned());

    // ON only: report the complete own-equity path (realized + floating).
    // Do NOT use closed trades alone: positions may remain open at the end.
    // Preserve raw broker equity explicitly in Metrics instead of changing its
    // meaning silently. OFF / credit=0 does not modify even one curve value.
    let reporting_min_equity = if reporting_credit > 0.0 {
        for (_, equity) in &mut equity_curve { *equity -= reporting_credit; }
        for day in &mut daily {
            day.start_equity -= reporting_credit;
            day.end_equity -= reporting_credit;
            // The previously computed E delta and absolute daily DD already
            // cancel a constant credit; do not round/recompute either one.
        }
        broker.min_equity - reporting_credit
    } else { broker.min_equity };
    // w trybie dziennego resetu suma zysków dni jest właściwą miarą wyniku
    let mut metrics = compute(
        cfg.start_balance,
        &equity_curve,
        &daily,
        &all_trades,
        reporting_min_equity,
        broker.blown,
        cfg.settings.stat_be_prog_usd,
    );
    if reporting_credit > 0.0 {
        metrics.reporting_equity_basis = Some("own_equity_excluding_constant_credit".into());
        metrics.initial_credit = Some(reporting_credit);
        metrics.raw_broker_end_equity = Some(broker.equity());
        metrics.raw_broker_min_equity = Some(broker.min_equity);
        metrics.end_balance = broker.balance;
        // daily_reset below intentionally replaces end_balance/end_equity by
        // the existing aggregate of independent days; raw_* stays final day.
    }
    if cfg.daily_reset {
        let total: f64 = daily.iter().map(|d| d.profit).sum();
        metrics.total_profit = total;
        metrics.end_equity = cfg.start_balance + total;
        metrics.end_balance = metrics.end_equity;
        metrics.return_pct = total / cfg.start_balance * 100.0;
        // obsunięcie mierzymy jako najgorszy dzień, bo konto startuje od nowa
        metrics.max_dd_abs = daily.iter().map(|d| d.max_dd).fold(0.0, f64::max);
        metrics.max_dd_pct = metrics.max_dd_abs / cfg.start_balance * 100.0;

        // POLA POCHODNE MUSZĄ IŚĆ ZA `total_profit`, NIE PRZED NIM.
        //
        // `compute` liczy je z `end_equity - start_balance`, a przy resecie
        // dobowym saldo wraca co dzień do kwoty startowej, więc ta różnica jest
        // ≈ 0. Do 18.08.2026 zostawały tu wartości policzone z tego zera:
        // `expectancy` wychodziło 0,0 przy 130 transakcjach i zysku 3 681,18 $
        // (powinno 28,32), a `recovery_factor` był zaniżony tak samo.
        // `trades`, `win_rate` i `profit_factor` były poprawne, bo nie zależą
        // od `total_profit` — stąd defekt wyglądał na kaprys jednego pola.
        if metrics.trades > 0 {
            metrics.expectancy = metrics.total_profit / metrics.trades as f64;
        }
        metrics.recovery_factor = if metrics.max_dd_abs > 0.0 {
            metrics.total_profit / metrics.max_dd_abs
        } else {
            0.0
        };

        // CALMAR zostaje ZEROWANY, a nie przeliczany. Opiera się na CAGR, czyli
        // na złożonym wzroście kapitału — a przy resecie dobowym kapitał się nie
        // składa, bo każdy dzień startuje od tej samej kwoty. Liczba wyszłaby
        // poprawna arytmetycznie i bez sensu merytorycznie; zero mówi wprost
        // „ta miara nie stosuje się do tego trybu".
        metrics.calmar = 0.0;
    }
    metrics.max_open_risk = max_open_risk;
    metrics.max_open_risk_pct = max_open_risk_rel;
    metrics.max_floating_loss = max_floating_loss;
    metrics.max_open_positions = max_open_pos;
    // ---------- liczniki silnika: SUMA po wszystkich formatach ----------
    // Przy jednym silniku suma jednoelementowa daje dokładnie to, co dawało
    // odczytanie pola wprost — stąd parytet.
    // NAROSŁE + BIEŻĄCE (poz. 21): w trybie dziennym silniki są wymieniane
    // co dobę, więc same bieżące liczniki opisywały wyłącznie ostatni dzień
    // udający cały przebieg (runner:1333 w audycie). W compoundingu bez
    // drabinki `narosle` zostaje zerowe i suma równa się bieżącym co do sztuki.
    metrics.signals_seen = (narosle.iter().map(|n| n.1).sum::<u64>()
        + zespol
            .lista
            .iter()
            .map(|s| s.engine.stats.messages)
            .sum::<u64>()) as u32;
    metrics.signals_taken =
        (narosle.iter().map(|n| n.0).sum::<u64>() + przyjete_sygnaly(&zespol)) as u32;
    metrics.baskets = narosle.iter().map(|n| n.2).sum::<u32>()
        + zespol
            .lista
            .iter()
            .map(|s| s.engine.baskets.len())
            .sum::<usize>() as u32;
    metrics.rejected_stops = broker.rejected_stops;
    metrics.market_instead_of_limit = broker.market_instead_of_limit;
    metrics.rejected_no_money = broker.rejected_no_money;
    metrics.stop_outs = broker.stop_outs;
    metrics.min_margin_level = broker.min_margin_level;
    metrics.ml_pod_200 = broker.ml_pod_200;
    metrics.ml_pod_150 = broker.ml_pod_150;
    metrics.ml_pod_100 = broker.ml_pod_100;
    metrics.max_open_volume = broker.max_open_volume;
    metrics.max_open_margin = broker.max_open_margin;
    {
        // Rozkład liczymy na WSZYSTKICH zleceniach, które przestały leżeć —
        // wypełnionych i skasowanych razem. Liczenie samych wypełnionych
        // zaniżyłoby ogon (najdłużej leżą te, które nigdy się nie wypełniły),
        // a samych skasowanych — zawyżyło.
        let mut v: Vec<i64> = broker.zycie_pend_fill.clone();
        v.extend_from_slice(&broker.zycie_pend_anul);
        v.sort_unstable();
        if !v.is_empty() {
            let idx = |f: f64| -> f64 {
                let i = ((v.len() as f64 - 1.0) * f).round() as usize;
                v[i.min(v.len() - 1)] as f64 / 60_000.0
            };
            metrics.pend_zycie_med_min = idx(0.5);
            metrics.pend_zycie_p90_min = idx(0.9);
        }
        metrics.pend_fill_n = broker.zycie_pend_fill.len() as u64;
        metrics.pend_anul_n = broker.zycie_pend_anul.len() as u64;
        metrics.stop_out_ts = broker.stop_out_ts;
        metrics.bal_przy_stopoucie = broker.bal_przy_stopoucie;
        metrics.eq_przy_stopoucie = broker.eq_przy_stopoucie;
    }
    metrics.rejected_pending_stops = broker.rejected_pending_stops;
    if cfg.quick_tick_stride > 1 && quick_observed > 0 {
        // These three legacy fields count quote observations, not elapsed
        // time.  A quick tape has fewer observations by construction.  Scale
        // them back to raw-row equivalents so the existing ranking penalty
        // does not reward a larger N merely for looking less often.  Since
        // extrema are deliberately overrepresented this estimate is usually
        // conservative; exact N=1 remains required for finalists.
        let raw = end_idx.saturating_sub(i0) as u128;
        let observed = quick_observed as u128;
        let scale = |value: u64| -> u64 {
            ((value as u128).saturating_mul(raw).saturating_add(observed / 2) / observed)
                .min(u64::MAX as u128) as u64
        };
        metrics.ml_pod_200 = scale(metrics.ml_pod_200);
        metrics.ml_pod_150 = scale(metrics.ml_pod_150);
        metrics.ml_pod_100 = scale(metrics.ml_pod_100);
    }
    // ---------- DIAGNOSTYKA WARSTWY EA-CORE (N15 / N10 / N18 / N19) ----------
    //
    // Ten sam wzorzec co diagnostyka zmienności niżej: **na stderr, nie do
    // `metrics`**. To jest wynik POMIARU, po którym nic się nie rankuje —
    // a `Metrics` jest schematem archiwum (dopisanie pola bez `serde(default)`
    // unieważniło już raz 207 plików).
    //
    // Przy `ea_enabled = false` nie drukuje się ani jedna linijka i nie liczy
    // ani jedna suma. Parytet obejmuje też brak hałasu w wyjściu — dokładnie
    // dlatego pętla stoi pod warunkiem, a nie warunek w środku pętli.
    //
    // ⚠ Przy `--daily-reset` i przy drabince silniki są WYMIENIANE, a razem
    // z nimi ginie `EaRdzen` (to jest zamierzone: wymiana silnika modeluje
    // restart, a N19 każe odtworzyć stan). Liczniki opisują wtedy ostatni
    // odcinek, nie cały przebieg — i linijka mówi to wprost, zamiast podawać
    // sumę, która wygląda na całość.
    if zespol.lista.iter().any(|s| s.engine.cfg.ea_enabled) {
        let odcinkowe = cfg.daily_reset || cfg.flat_na_dobie || !cfg.drabinka.is_empty();
        for s in zespol.lista.iter() {
            if !s.engine.cfg.ea_enabled {
                continue;
            }
            let ea = &s.engine.ea;
            let bl = ea.bilans();
            let fmt = if s.format.is_empty() {
                "-"
            } else {
                s.format.as_str()
            };
            eprintln!(
                "[EA-CORE {fmt}] auto_ea={} ea_tick_s={} | pulsy tick={} zegar={} | stan={} gotowy={} stemple={}{}",
                s.engine.tryb_auto_ea,
                s.engine.cfg.ea_tick_s,
                ea.pulsy_tick,
                ea.pulsy_zegar,
                ea.stan().kod(),
                ea.gotowy(),
                ea.stemple().len(),
                if odcinkowe { "  (liczniki z OSTATNIEGO odcinka — silnik był wymieniany)" } else { "" },
            );
            eprintln!(
                "[EA-CORE {fmt}] N18 bilans: rozpatrzone={} obsluzone={} pominiete={} zgubione_bez_sladu={} domyka_sie={}",
                bl.rozpatrzone,
                bl.obsluzone,
                bl.pominiete,
                bl.zgubione_bez_sladu(),
                bl.domyka_sie(),
            );
            eprintln!(
                "[EA-CORE {fmt}] N15 pozycje bez SL: widziane={} dostawione={} (dozor {})",
                ea.widziane_bez_sl,
                ea.dostawione_sl,
                if s.engine.cfg.ea_dozor_sl {
                    "WLACZONY"
                } else {
                    "wylaczony"
                },
            );
            // N10: raportujemy KOSZYKI (miara akceptacji niezmiennika) i dopiero
            // za nimi zdarzenia. Odwrotna kolejność już raz wprowadziła w błąd:
            // liczba zdarzeń jest funkcją `ea_tick_s`, więc sama w sobie nie
            // mówi nic o handlu.
            eprintln!(
                "[EA-CORE {fmt}] N10 zapadka: koszykow_ponad_stemplem={} (zdarzen={}) max_nadwyzka={:.2} $ ({:.1} %) | regresja_etapu: koszykow={} (zdarzen={})",
                ea.koszyki_zapadka,
                bl.ile(conduit_core::ea::KodPominiecia::ZapadkaZlamana),
                ea.zapadka_max_nadwyzka_usd,
                ea.zapadka_max_nadwyzka_pct,
                ea.koszyki_regresja,
                bl.ile(conduit_core::ea::KodPominiecia::RegresjaEtapu),
            );
            let kody: Vec<String> = conduit_core::ea::KodPominiecia::WSZYSTKIE
                .iter()
                .filter(|k| bl.ile(**k) > 0)
                .map(|k| format!("{}={}", k.kod(), bl.ile(*k)))
                .collect();
            eprintln!(
                "[EA-CORE {fmt}] kody pominiec: {} | zmian stanu w dzienniku: {}",
                if kody.is_empty() {
                    "brak".to_string()
                } else {
                    kody.join(" ")
                },
                ea.dziennik().len(),
            );
        }
    }
    // ---------- DIAGNOSTYKA RODZINY „ROZMIAR STEROWANY ZMIENNOŚCIĄ" ----------
    //
    // Na stderr, nie do `metrics`: to jest wynik POMIARU, a nie liczba,
    // po której cokolwiek się rankuje. Przy `Off` nie drukuje się nic i nic
    // się nie liczy — parytet obejmuje też brak hałasu w wyjściu.
    for s in zespol.lista.iter() {
        if s.engine.cfg.vol_size_mode == conduit_core::settings::VolSizeMode::Off {
            continue;
        }
        let z = s.engine.stan_zmiennosci();
        let (profil, ile) = s.engine.profil_godzinowy();
        let sr = if z.ile_policzono > 0 {
            z.suma_mult / z.ile_policzono as f64
        } else {
            1.0
        };
        eprintln!(
            "[ZMIENNOSC {}] tryb={:?} target={} okno={} odsezonuj={} | ocen={} mult sr={:.3} min={:.3} max={:.3} | ocen z odsezonowaniem={}",
            s.format,
            s.engine.cfg.vol_size_mode,
            s.engine.cfg.vol_size_target,
            s.engine.cfg.vol_size_percentile_okno,
            s.engine.cfg.vol_size_odsezonuj,
            z.ile_policzono,
            sr,
            if z.min_mult == f64::MAX { 1.0 } else { z.min_mult },
            if z.max_mult == f64::MIN { 1.0 } else { z.max_mult },
            z.ile_odsezonowano,
        );
        let p: Vec<String> = profil.iter().map(|x| format!("{x:.2}")).collect();
        let n: Vec<String> = ile.iter().map(|x| x.to_string()).collect();
        eprintln!(
            "[ZMIENNOSC {}] profil godzinowy (mnoznik):  {}",
            s.format,
            p.join(" ")
        );
        eprintln!(
            "[ZMIENNOSC {}] profil godzinowy (n godzin): {}",
            s.format,
            n.join(" ")
        );
    }
    for s in zespol.lista.iter() {
        for (k, v) in s.engine.odrzuty.iter() {
            *metrics.odrzuty.entry(k.clone()).or_insert(0) += *v;
        }
        metrics.relot_up_zdarzen += s.engine.stats.relot_up_zdarzen;
        metrics.relot_down_zdarzen += s.engine.stats.relot_down_zdarzen;
        metrics.relot_up_lotow += s.engine.stats.relot_up_lotow;
        metrics.relot_down_lotow += s.engine.stats.relot_down_lotow;
        metrics.relot_down_bez_spadku += s.engine.stats.relot_down_bez_spadku;
        metrics.relot_up_ponad_plan += s.engine.stats.relot_up_ponad_plan;
        metrics.relot_prob += s.engine.stats.relot_prob;
        metrics.relot_udane += s.engine.stats.relot_udane;
        metrics.relot_odmowy += s.engine.stats.relot_odmowy;
        metrics.relot_plan_pusty += s.engine.stats.relot_plan_pusty;
        metrics.relot_plan_ok += s.engine.stats.relot_plan_ok;
        metrics.relot_rozjazd_lotow += s.engine.stats.relot_rozjazd_lotow;
        metrics.relot_szczebli += s.engine.stats.relot_szczebli;
        metrics.relot_ksztalt_odmowa += s.engine.stats.relot_ksztalt_odmowa;
        // Ekspozycja: MAKSIMUM, nie suma — to jedna wielkość rachunku, a nie
        // licznik zdarzeń. Sumowanie po silnikach dałoby liczbę bez sensu
        // fizycznego (dwa razy 300 % to nadal 300 %, nie 600 %).
        if s.engine.stats.expo_max_pct > metrics.expo_max_pct {
            metrics.expo_max_pct = s.engine.stats.expo_max_pct;
        }
        metrics.expo_zdarzen += s.engine.stats.expo_zdarzen;
        metrics.expo_pend_skasowane += s.engine.stats.expo_pend_skasowane;
        metrics.expo_lotow += s.engine.stats.expo_lotow;
        metrics.expo_poz_domkniete += s.engine.stats.expo_poz_domkniete;
        metrics.expo_niedosyt += s.engine.stats.expo_niedosyt;
    }
    // Sygnały, które nie miały dokąd pójść, są CZĘŚCIĄ diagnostyki przebiegu.
    for (k, v) in bez_trasy.iter() {
        *metrics
            .odrzuty
            .entry(format!("BrakFormatu:{k}"))
            .or_insert(0) += *v;
    }
    // Nowe sygnały formatu bez nogi na bieżącym szczeblu — POLICZONE, nie cisza.
    for (k, v) in poza_szczeblem.iter() {
        *metrics
            .odrzuty
            .entry(format!("SzczebelBezNogi:{k}"))
            .or_insert(0) += *v;
    }
    // Wiadomości z `ts + lat` PO OSTATNIM TICKU okna: pętla nigdy do nich nie
    // doszła, więc nie zostały ani wydane, ani nigdzie policzone (audyt:
    // runner:629-633). Licznik diagnostyczny — nie wchodzą do lejka, bo nie
    // były też w `sygnaly_wejsciowe` (liczonym przy wydaniu). Wpis tylko przy
    // niezerowej liczbie, żeby typowy wydruk odrzutów się nie zmienił.
    {
        let ostatni_ts = ticks.ts(i1 - 1);
        let po_ostatnim = msgs.iter().filter(|m| m.ts + lat > ostatni_ts).count() as u64;
        if po_ostatnim > 0 {
            *metrics.odrzuty.entry("PoOstatnimTicku".into()).or_insert(0) += po_ostatnim;
        }
    }
    // domknięcie rozliczenia drabinki: transakcje od ostatniego przełączenia
    // należą do szczebla, który kończył przebieg
    if drabinka_on {
        for t in broker.history[szczebel_mark..].iter() {
            szczeble_stat[szczebel].zysk += t.profit;
            szczeble_stat[szczebel].trejdy += 1;
        }
    }

    let journal_lines = match dump {
        Some(mut d) => {
            for s in zespol.lista.iter_mut() {
                let mut evs = s.engine.drain_journal();
                let _ = d.push(&mut evs, jtz);
            }
            d.finish().unwrap_or(0)
        }
        None => 0,
    };

    let mut baskets_dump = arch;
    if cfg.daily_reset || cfg.flat_na_dobie {
        for s in zespol.lista.iter() {
            baskets_dump.extend(zrzuc_koszyki(
                &s.engine.baskets,
                &broker.history[hist_mark.min(broker.history.len())..],
                seg,
            ));
        }
    } else {
        // COMPOUNDING (poz. 21): stan końcowy nadpisuje wpisy drenażu
        // dobowego, a koszyki wycięte przez silnik w trakcie przebiegu
        // zostają w mapie ze stanem z ostatniego dnia życia. Bez przycinania
        // w silniku mapa == dotychczasowy zrzut co do sztuki (id rosną
        // z numeracją koszyków, więc i kolejność jest ta sama).
        for s in zespol.lista.iter() {
            for d in zrzuc_koszyki(
                &s.engine.baskets,
                &broker.history[hist_mark.min(broker.history.len())..],
                seg,
            ) {
                arch_comp.insert(d.id, d);
            }
        }
        baskets_dump.extend(arch_comp.into_values());
    }

    // ---------- rozbicie na formaty ----------
    //
    // Zysk formatu liczymy z HISTORII BROKERA po slocie numeru koszyka, a nie
    // ze statystyk silnika: koszyk domknięty zbiorczo na koniec doby nigdy nie
    // dostaje kredytu w `realized`, bo silnik jest wymieniany zanim zdąży
    // odebrać zamknięcia. Dokładnie z tego powodu `zrzuc_koszyki` też liczy
    // z historii.
    let formaty: Vec<StatFormatu> = if !routuj {
        Vec::new()
    } else {
        zespol
            .lista
            .iter()
            .enumerate()
            .map(|(idx, s)| {
                let mut zysk = 0.0;
                let mut trejdy = 0u32;
                for t in all_trades.iter() {
                    let slot = t
                        .basket
                        .map(conduit_core::wielosilnik::slot_koszyka)
                        .unwrap_or(conduit_core::wielosilnik::SLOT_STARY);
                    if slot == s.slot {
                        zysk += t.profit;
                        trejdy += 1;
                    }
                }
                StatFormatu {
                    format: s.format.clone(),
                    preset: s.preset.clone(),
                    slot: s.slot,
                    zysk,
                    trejdy,
                    sygnaly: narosle[idx].0 + s.engine.stats.signals,
                    wiadomosci: narosle[idx].1 + s.engine.stats.messages,
                    koszyki: narosle[idx].2 + s.engine.baskets.len() as u32,
                }
            })
            .collect()
    };

    // ---- STATYSTYKI PAKIETU E (warstwa pomiarowa, nic nie wraca do silnika) ----
    //
    // Rejestr odrzuconych wejść zbieramy ze WSZYSTKICH nóg: przy jednym
    // formacie to dosłownie ten jeden silnik, więc ścieżka jednonoga jest
    // dokładnie taka, jak była.
    let odrzucone_wejscia: Vec<conduit_core::engine::OdrzuconeWejscie> = zespol
        .lista
        .iter()
        .flat_map(|s| s.engine.odrzucone_wejscia.iter().cloned())
        .collect();
    metrics.stat_sygnalow = crate::statystyki::policz(crate::statystyki::Wejscie {
        metrics: &metrics,
        koszyki: &baskets_dump,
        trades: &all_trades,
        odrzucone: &odrzucone_wejscia,
        sl_tp_same_tick: broker.sl_tp_same_tick,
        spread_usd: broker.spread_paid_usd,
        prog_be: cfg.settings.stat_be_prog_usd,
        ticks: Some(ticks),
        tz_offset_ms: cfg.settings.server_tz_offset_ms,
        sygnaly_wejsciowe: sygnaly_wejsciowe.min(u32::MAX as u64) as u32,
    });

    RunResult {
        approximation: approximation_info(
            cfg.quick_tick_stride,
            end_idx.saturating_sub(i0),
            quick_observed.min(quick_selected_total),
        ),
        cost_reconciliation_required: cost_reconciliation_required
            .or_else(||broker.cost_reconciliation_required().map(str::to_owned)),
        sr_warmup_reconciliation_required: None,
        sim_execution_reconciliation_required: None,
        continuation_reconciliation_required: continuation_reconciliation_required
            .or_else(||crate::continuation::review_reason(&zespol)),
        continuation_scope,
        metrics,
        baskets_dump,
        equity_curve,
        balance_curve,
        daily,
        trades: all_trades,
        ticks_processed: (end_idx - i0) as u64,
        elapsed_ms: t_start.elapsed().as_millis() as u64,
        journal_lines,
        swap_paid: broker.swap_total,
        stop_outs: broker.stop_outs,
        cancelled,
        formaty,
        bez_trasy,
        szczeble: if drabinka_on {
            szczeble_stat
        } else {
            Vec::new()
        },
        przelaczenia,
    }
}

/// Ile sygnałów PRZYJĘŁY wszystkie silniki razem.
///
/// Przy jednym silniku to dosłownie `engine.stats.signals`, więc licznik dnia
/// i metryka końcowa zachowują się dokładnie jak przed wprowadzeniem formatów.
#[inline]
fn przyjete_sygnaly(z: &Silniki) -> u64 {
    z.lista.iter().map(|s| s.engine.stats.signals).sum()
}

/// Wspólny, konserwatywny początek skanu rozgrzewki. Zapas pokrywa weekendy
/// i przerwy dobowe; końcowe bufory same odrzucają nadmiarową historię.
#[inline]
fn poczatek_rozgrzewki(od: Ts, godzin: usize) -> Ts {
    let zapas_h = (godzin as i64).saturating_mul(2).saturating_add(120);
    od.saturating_sub(zapas_h.saturating_mul(3_600_000))
}

/// Buduje domknięte świece M1 dla rozgrzewki dynamicznego S/R.
///
/// Jest to tickowa odpowiedź na `MarketData::candles(symbol, "M1", ...)`
/// używane przez live:
///
/// * `ts` = początek minuty w zegarze ticków/serwera brokera,
/// * OHLC struktury liczymy po `mid`, dokładnie jak `Engine::sr_na_ticku`,
/// * `spread` = spread OSTATNIEGO ticka tej minuty,
/// * minuta zawierająca `od` jest pomijana, jeśli nie zdążyła się domknąć,
/// * minuta bez ticka nie tworzy sztucznej świecy.
///
/// Ostatnie dwa punkty są istotne dla przyczynowości: rozgrzewka nie może
/// znać high/low ani close formującej się świecy. Przy `godzin == 0` funkcja
/// wraca PRZED jakimkolwiek odczytem `TickData`; zimny start pozostaje więc
/// także kosztowo ścieżką legacy.
pub fn swiece_m1_sr_z_tickow(ticks: &TickData, od: Ts, godzin: usize) -> Vec<SrWarmupBar> {
    if godzin == 0 {
        return Vec::new();
    }

    const M1_MS: i64 = 60_000;
    // `od` może wypaść w środku minuty. Domknięte są wyłącznie kubełki
    // kończące się nie później niż początek tej formującej się minuty.
    let koniec = od.div_euclid(M1_MS).saturating_mul(M1_MS);
    let surowy_poczatek = poczatek_rozgrzewki(od, godzin);
    // MT5 zwraca pełną pierwszą świecę M1, więc skan także zaczynamy od jej
    // początku (najwyżej 59 999 ms ponad konserwatywny zapas).
    let poczatek = surowy_poczatek.div_euclid(M1_MS).saturating_mul(M1_MS);
    if koniec <= poczatek {
        return Vec::new();
    }

    let i0 = ticks.index_at(poczatek);
    let i1 = ticks.index_at(koniec).min(ticks.len());
    if i1 <= i0 {
        return Vec::new();
    }

    let mut out: Vec<SrWarmupBar> = Vec::new();
    let mut otwarta: Option<SrWarmupBar> = None;
    for i in i0..i1 {
        let q = ticks.quote(i);
        let mid = q.mid();
        let spread = q.spread();
        if !mid.is_finite() || !spread.is_finite() || spread < 0.0 {
            continue;
        }
        let ts = q.ts.div_euclid(M1_MS).saturating_mul(M1_MS);
        match otwarta.as_mut() {
            None => {
                otwarta = Some(SrWarmupBar {
                    ts,
                    high: mid,
                    low: mid,
                    close: mid,
                    spread,
                });
            }
            Some(bar) if bar.ts == ts => {
                bar.high = bar.high.max(mid);
                bar.low = bar.low.min(mid);
                bar.close = mid;
                bar.spread = spread;
            }
            Some(bar) if bar.ts < ts => {
                out.push(*bar);
                *bar = SrWarmupBar {
                    ts,
                    high: mid,
                    low: mid,
                    close: mid,
                    spread,
                };
            }
            // `TickData::index_at` wymaga porządku rosnącego. Gdy uszkodzony
            // plik mimo to zawiera cofnięty rekord, nie wolno nim przepisać
            // close już późniejszej świecy.
            Some(_) => {}
        }
    }
    if let Some(bar) = otwarta {
        if bar.ts.saturating_add(M1_MS) <= koniec {
            out.push(bar);
        }
    }
    out
}

/// Wspólna z live bramka pełnego pokrycia dynamicznego S/R.
///
/// Sam `Engine::rozgrzej_sr_z_m1` potrafi policzyć ATR już po kilku barach,
/// lecz live wymaga także całego horyzontu struktury i zapasu na potwierdzenie
/// fractala. Backtest musi stosować tę samą regułę; inaczej cropped okno może
/// uznać S/R za gotowe wcześniej niż bot po restarcie.
fn rozgrzej_dynamiczne_sr(engine: &mut Engine, bars: &[SrWarmupBar]) {
    let c = &engine.cfg;
    let dynamiczne = c.trail_sr_enabled
        && (c.trail_sr_min_prominence_atr > 0.0
            || c.trail_sr_offset_atr_mult > 0.0
            || c.trail_sr_offset_spread_mult > 0.0);
    if !dynamiczne {
        return;
    }
    let zapas_barow =
        2 * c.trail_sr_fractal_n.max(1) as i64 + c.trail_sr_atr_period.max(1) as i64 + 3;
    let wymagane_ms = c.trail_sr_struct_window_h.max(1) as i64 * 3_600_000
        + zapas_barow * c.trail_sr_tf_min.max(1) as i64 * 60_000;
    let pokrycie_ok = match (bars.first(), bars.last()) {
        (Some(a), Some(z)) => z.ts.saturating_sub(a.ts) >= wymagane_ms,
        _ => false,
    };
    if pokrycie_ok {
        let _ = engine.rozgrzej_sr_z_m1(bars);
    } else {
        // Jak live: nie zostawiamy przypadkiem starego/połowicznego stanu.
        let _ = engine.rozgrzej_sr_z_m1(&[]);
    }
}

fn rozgrzej_dynamiczne_sr_v2(engine: &mut Engine,
    snapshot: &conduit_core::engine::SrWarmupSnapshotV2) -> Result<(), String> {
    let c = &engine.cfg;
    let reserve = 2 * c.trail_sr_fractal_n.max(1) as i64 + c.trail_sr_atr_period.max(1) as i64 + 3;
    let required = c.trail_sr_struct_window_h.max(1) as i64 * 3_600_000
        + reserve * c.trail_sr_tf_min.max(1) as i64 * 60_000;
    let coverage = snapshot.minutes.first().zip(snapshot.minutes.last())
        .map_or(0, |(a,z)| z.last_tick_ts.saturating_sub(a.first_tick_ts));
    if coverage < required { return Err(format!("SR V2 insufficient prefix: {coverage} ms < {required} ms")); }
    match engine.rozgrzej_sr_v2(snapshot, &snapshot.context, snapshot.cutoff_exclusive)? {
        conduit_core::engine::SrWarmupAppliedV2::Applied { dynamic_ready: true, .. } => Ok(()),
        _ => Err("SR V2 prefix does not initialize dynamic state".into()),
    }
}

fn rejected_sr_warmup(cfg: &RunConfig, reason: String) -> RunResult {
    RunResult {
        approximation: approximation_info(cfg.quick_tick_stride, 0, 0),
        cost_reconciliation_required: None, sr_warmup_reconciliation_required: Some(reason),
        sim_execution_reconciliation_required: None,
        continuation_reconciliation_required: None,
        continuation_scope: None,
        metrics: Metrics { start_balance: cfg.start_balance, ..Default::default() },
        equity_curve: Vec::new(), balance_curve: Vec::new(), daily: Vec::new(),
        trades: Vec::new(), baskets_dump: Vec::new(), ticks_processed: 0, elapsed_ms: 0,
        journal_lines: 0, swap_paid: 0.0, stop_outs: 0, cancelled: false,
        formaty: Vec::new(), bez_trasy: BTreeMap::new(), szczeble: Vec::new(), przelaczenia: Vec::new(),
    }
}

fn rejected_sim_execution(cfg: &RunConfig, reason: String) -> RunResult {
    let mut result = rejected_sr_warmup(cfg, String::new());
    result.sr_warmup_reconciliation_required = None;
    result.sim_execution_reconciliation_required = Some(reason);
    result
}

fn rejected_continuation(cfg:&RunConfig,reason:String)->RunResult {
    let mut result=rejected_sr_warmup(cfg,String::new());
    result.sr_warmup_reconciliation_required=None;
    result.continuation_reconciliation_required=Some(reason);result
}

/// Buduje historię rynku z ticków SPRZED `od`, dokładnie tak, jak zrobiłby to
/// `Engine::on_tick`, gdyby silnik ruszył `godzin` wcześniej.
///
/// `price_hist` zawiera jeden punkt `mid` na godzinę dla filtra reżimu, a
/// `vol_hist` próbkę co 5 s dla mnożnika zmienności i reversal-exit. Reguły
/// przycinania odpowiadają tym w silniku: 30 dni dla ceny i podwójne okno
/// aktywnej reguły dla zmienności.
pub fn historia_z_tickow(
    ticks: &TickData,
    od: Ts,
    godzin: usize,
    s: &Settings,
) -> (Vec<(Ts, Px)>, Vec<(Ts, Px)>) {
    // ⚠ OKNO KALENDARZOWE ≠ GODZINY HANDLOWE. To był no-op, nie teoria.
    //
    // `price_hist` dostaje jeden punkt na godzinę **Z TICKÓW**, a złoto nie ma
    // ticków w weekend ani w przerwie dobowej. Cofnięcie się o dokładnie
    // `godzin * 3_600_000` (czyli 72 h = 3 dni kalendarzowe) daje przy starcie
    // w poniedziałek okno Piątek→Poniedziałek, w którym ticki są tylko z
    // piątku — czyli **~24 punkty zamiast 72**.
    //
    // A `regime_ok` przy `price_hist.len() < n` **przepuszcza wszystko**:
    //
    // ```
    // if self.price_hist.len() < n.max(2) { return true; }
    // ```
    //
    // Skutek: rozgrzewka formalnie działała (bufor był wypełniany), ale filtr
    // i tak milczał, więc wynik wychodził **co do centa taki sam jak przy
    // zimnym starcie**. Dokładnie to zgłosił zespół Fable jako „`--rozgrzewka-h`
    // jest no-opem" i mieli rację.
    //
    // Cofamy się więc o zapas: `godzin * 2 + 120 h`. Weekend zjada 48 h na
    // każde 7 dni, przerwy dobowe po ~1 h — podwojenie z okładem pokrywa oba
    // przy każdym dniu tygodnia. Nadmiarowe punkty NIE szkodzą: `regime_ok`
    // bierze `price_hist[len - n..]`, czyli ostatnie `n`, a `on_tick` i tak
    // przycina bufor do 30 dni.
    //
    // NIE „naprawiamy" tego przez poluzowanie warunku w `regime_ok` — ten
    // warunek jest wspólny z backtestem liczonym od `--from` i jego zmiana
    // ruszyłaby każdy dotychczasowy pomiar.
    //
    // ZERO GODZIN = ZIMNY START, TAKŻE TUTAJ. Wołający (`run_with_progress`)
    // i tak nie wchodzi tu przy `rozgrzewka_h == 0`, ale funkcja jest
    // publiczna i sama musi trzymać umowę: bez tej gałęzi zapas `+120 h`
    // wczytywałby historię nawet przy jawnym „bez rozgrzewki" — i ktoś,
    // kto woła ją wprost (test, przyszłe narzędzie), dostałby ciepły silnik
    // tam, gdzie zamówił zimny. Złapane przez
    // `rozgrzewka_nie_jest_noopem::zero_godzin_dalej_znaczy_zimny_start`.
    if godzin == 0 {
        return (Vec::new(), Vec::new());
    }
    let poczatek = poczatek_rozgrzewki(od, godzin);
    let i0 = ticks.index_at(poczatek);
    let i1 = ticks.index_at(od).min(ticks.len());

    let mut price: Vec<(Ts, Px)> = Vec::new();
    let mut vol: Vec<(Ts, Px)> = Vec::new();
    if i1 <= i0 {
        return (price, vol);
    }

    // Okno bufora zmienności — jak w `on_tick`.
    let okno_vol = (s.vol_window_min.max(s.rev_exit_window_min).max(60.0) * 2.0) * 60_000.0;

    for i in i0..i1 {
        let q = ticks.quote(i);
        let ts = q.ts;
        let mid = q.mid();
        if price
            .last()
            .map(|(t, _)| ts - *t >= 3_600_000)
            .unwrap_or(true)
        {
            price.push((ts, mid));
            if price.len() > 24 * 30 {
                price.remove(0);
            }
        }
        if vol.last().map(|(t, _)| ts - *t >= 5_000).unwrap_or(true) {
            vol.push((ts, mid));
            let horyzont = ts - okno_vol as i64;
            if vol.len() > 64 {
                vol.retain(|(t, _)| *t >= horyzont);
            }
        }
    }
    (price, vol)
}

/// Wstrzykuje konfigurację EA z presetu do rdzenia silnika.
///
/// Cisza przy złym JSON-ie byłaby tą samą klasą błędu, która 24.08.2026
/// puściła 20 zleceń z niezwiązanego presetu — więc krzyczymy na `stderr`
/// i zostawiamy warstwę wyłączoną, zamiast po cichu grać czymś innym,
/// niż napisano w pliku.
fn wstrzyknij_ea(e: &mut Engine, cfg: &RunConfig) {
    let txt = match cfg.ea_konfig.as_deref() {
        Some(t) if !t.trim().is_empty() => t,
        _ => return,
    };
    match serde_json::from_str::<conduit_core::ea::KonfigBety>(txt) {
        Ok(k) => e.ea.wstrzyknij_bete(k),
        Err(err) => eprintln!(
            "⚠ pole `ea` presetu nie da się sparsować ({err}) — warstwa EA ZOSTAJE WYŁĄCZONA"
        ),
    }
}

/// Buduje jeden silnik legacy albo zespół formatów współdzielący rachunek.
/// Pola rachunku są wspólne, zaś taktyka pochodzi z ustawień danego formatu.
fn zbuduj_zespol(cfg: &RunConfig) -> Silniki {
    if cfg.formaty.is_empty() {
        let mut e = Engine::new(cfg.settings.clone(), cfg.start_balance);
        wstrzyknij_ea(&mut e, cfg);
        // Pułapy działają także przy JEDNYM presecie — inaczej `--pulapy`
        // z jednym `--preset-format` byłoby cicho ignorowane. Domyślne zera
        // niczego nie zmieniają (`sufit_u32(0, p) == p`), więc parytet stoi.
        e.pulapy = cfg.pulapy.clone();
        e.tryb_auto_ea = cfg.auto_ea;
        let l = Lancuch {
            nazwa: "BACKTEST".into(),
            pulapy: cfg.pulapy.clone(),
            ..Default::default()
        };
        return Silniki::pojedynczy(e, String::new(), String::new(), l, true);
    }
    let mut l = Lancuch {
        nazwa: "BACKTEST".into(),
        pulapy: cfg.pulapy.clone(),
        ..Default::default()
    };
    let mut presety: BTreeMap<String, Settings> = BTreeMap::new();
    for f in cfg.formaty.iter() {
        // Klucz presetu to NAZWA FORMATU, nie nazwa pliku: dwa formaty wolno
        // puścić na tym samym presecie (`HYPER-X1` na obu), a mapa po nazwie
        // pliku sklejałaby je w jeden wpis i drugi format zostałby bez silnika.
        l.presety.insert(f.format.clone(), f.format.clone());
        presety.insert(f.format.clone(), f.settings.clone());
    }
    let (mut s, braki) = Silniki::zbuduj(&l, &presety, &cfg.settings, cfg.start_balance);
    // Wielosilnik: KAŻDA noga dostaje tę samą konfigurację EA. Warstwa jest
    // decyzją o rachunku, nie o kanale — jeden mózg na jedno konto.
    for sl in s.lista.iter_mut() {
        wstrzyknij_ea(&mut sl.engine, cfg);
    }
    for b in braki {
        // Nieosiągalne (mapę budujemy wiersz w wiersz), ale cisza jest zakazana.
        eprintln!("routing backtestu: {}", b.opis());
    }
    // Nazwy presetów do raportu — `zbuduj` wpisał tam nazwy formatów.
    for si in s.lista.iter_mut() {
        if let Some(f) = cfg.formaty.iter().find(|f| f.format == si.format) {
            si.preset = f.preset.clone();
        }
        si.engine.tryb_auto_ea = cfg.auto_ea;
    }
    s
}

/// Najwyższy szczebel, którego próg jest osiągnięty przez SALDO.
/// Szczeble muszą być posortowane rosnąco po progu — pilnuje tego bt.rs.
fn szczebel_dla_salda(drabinka: &[SzczebelCfg], saldo: f64) -> usize {
    let mut r = 0;
    for (i, s) in drabinka.iter().enumerate() {
        if saldo >= s.prog {
            r = i;
        }
    }
    r
}

/// Zespół silników dla drabinki: UNIA formatów WSZYSTKICH szczebli.
///
/// Silnik każdego formatu istnieje przez cały przebieg — przełączenie
/// szczebla wymienia mu ustawienia (z adopcją koszyków), ale nie zmienia
/// slotu ani indeksu w zespole. Dzięki temu:
/// * numeracja koszyków `B<slot>…` jest stabilna między szczeblami,
/// * `narosle`/`aktywne` indeksują się raz na cały przebieg,
/// * format, który wypadł z nogi przy zejściu, dalej ma silnik do
///   ZARZĄDZANIA swoimi otwartymi koszykami (kontrakt: zero sierot).
///
/// Formaty nieaktywne na szczeblu startowym dostają ustawienia swojej nogi
/// z NAJNIŻSZEGO szczebla, który je zna — to placeholder: routing blokuje im
/// nowe sygnały, a koszyków jeszcze nie mają, więc ustawienia nie grają.
fn zbuduj_zespol_drabinki(cfg: &RunConfig) -> (Silniki, Vec<bool>, usize) {
    let rung = szczebel_dla_salda(&cfg.drabinka, saldo_dla_drabinki(cfg, cfg.start_balance));
    let mut l = Lancuch {
        nazwa: cfg.drabinka[rung].nazwa.clone(),
        pulapy: cfg.drabinka[rung].pulapy.clone(),
        ..Default::default()
    };
    // klucz presetu = nazwa FORMATU (jak w `zbuduj_zespol`): dwa formaty
    // wolno puścić na tym samym presecie bez sklejenia w jeden wpis
    let mut presety: BTreeMap<String, Settings> = BTreeMap::new();
    for sz in cfg.drabinka.iter() {
        for f in sz.formaty.iter() {
            if !presety.contains_key(&f.format) {
                l.presety.insert(f.format.clone(), f.format.clone());
                presety.insert(f.format.clone(), f.settings.clone());
            }
        }
    }
    // szczebel startowy nadpisuje placeholder swoją nogą
    for f in cfg.drabinka[rung].formaty.iter() {
        presety.insert(f.format.clone(), f.settings.clone());
    }
    let (mut s, braki) = Silniki::zbuduj(&l, &presety, &cfg.settings, cfg.start_balance);
    for b in braki {
        eprintln!("drabinka: {}", b.opis());
    }
    let mut aktywne = Vec::with_capacity(s.lista.len());
    for si in s.lista.iter_mut() {
        match cfg.drabinka[rung]
            .formaty
            .iter()
            .find(|f| f.format == si.format)
        {
            Some(f) => {
                si.preset = f.preset.clone();
                aktywne.push(true);
            }
            None => {
                if let Some(f) = cfg
                    .drabinka
                    .iter()
                    .flat_map(|sz| sz.formaty.iter())
                    .find(|f| f.format == si.format)
                {
                    si.preset = f.preset.clone();
                }
                aktywne.push(false);
            }
        }
    }
    (s, aktywne, rung)
}

/// Model MT5 już wyklucza Credit z Balance. OFF zachowuje dawny parametr CLI.
fn saldo_dla_drabinki(cfg: &RunConfig, balance: f64) -> f64 {
    if cfg.settings.credit_balance_separate { cfg.settings.saldo_wlasne(balance, 0.0) }
    else { balance - cfg.drabinka_kredyt }
}

#[cfg(test)]
mod credit_balance_separate_drabinka_tests {
    use super::*;
    #[test]
    fn credit_balance_separate_drabinka_keeps_raw_balance_and_legacy_param() {
        let mut cfg=RunConfig::default();cfg.drabinka_kredyt=300.0;
        assert_eq!(saldo_dla_drabinki(&cfg,159.8),159.8-300.0);
        cfg.settings.credit_balance_separate=true;
        for base in [conduit_core::settings::PodstawaLota::Balance,conduit_core::settings::PodstawaLota::Equity,conduit_core::settings::PodstawaLota::MinOfBoth] {
            cfg.settings.lot_base=base;
            assert_eq!(saldo_dla_drabinki(&cfg,159.8),159.8);
            assert_eq!(saldo_dla_drabinki(&cfg,-50.0),-50.0);
        }
        cfg.settings.credit_balance_separate=false;
        assert_eq!(saldo_dla_drabinki(&cfg,159.8),159.8-300.0);
    }
}

/// Przełącza zespół na szczebel `nowy` — Z ADOPCJĄ koszyków.
///
/// Kontrakt (ten sam co restart na żywo, `Engine::adopt_baskets`):
/// * koszyki zachowują ORYGINALNE numery, licznik numeracji podnosi się sam
///   ponad najwyższy adoptowany — zero podwójnych numerów;
/// * koszyki przechodzą pod ustawienia NOWEJ nogi formatu (to nowy preset
///   nimi odtąd zarządza) — przechodzą WSZYSTKIE, także domknięte, żeby
///   zrzut końcowy był kompletny, a numeracja monotoniczna;
/// * format bez nogi na nowym szczeblu zatrzymuje silnik i koszyki
///   (zarządzaj-nie-otwieraj — nowe sygnały blokuje routing), zero sierot;
/// * historia rynku, liczniki odrzutów, historia reżimu i liczniki relotu
///   przechodzą jak przy wymianie dobowej — to własność PRZEBIEGU, nie
///   szczebla.
#[allow(clippy::too_many_arguments)]
fn przelacz_szczebel(
    zespol: &mut Silniki,
    aktywne: &mut [bool],
    cfg: &RunConfig,
    nowy: usize,
    saldo: f64,
    narosle: &mut [(u64, u64, u32)],
    dump: &mut Option<JournalDump>,
    jtz: i64,
) {
    let sz = &cfg.drabinka[nowy];
    for (idx, s) in zespol.lista.iter_mut().enumerate() {
        match sz.formaty.iter().find(|f| f.format == s.format) {
            Some(f) => {
                let engine = &mut s.engine;
                // dziennik przed wymianą silnika — bufor jest polem silnika
                // i przepadłby razem z nim (ta sama pułapka co przy resecie
                // dobowym)
                if let Some(d) = dump.as_mut() {
                    let mut evs = engine.drain_journal();
                    let _ = d.push(&mut evs, jtz);
                }
                narosle[idx].0 += engine.stats.signals;
                narosle[idx].1 += engine.stats.messages;
                // koszyki NIE wchodzą do narosłych: przechodzą do nowego
                // silnika w całości i policzą się w rozbiciu na końcu —
                // dopisanie ich tutaj liczyłoby je drugi raz
                let hist = engine.market_history();
                let odrz = std::mem::take(&mut engine.odrzuty);
                // …i rejestr odrzuconych wejść (Pakiet E3) — patrz bliźniacze
                // miejsce w resecie dobowym wyżej.
                let odrz_w = std::mem::take(&mut engine.odrzucone_wejscia);
                let rezim = std::mem::take(&mut engine.regime_hist);
                // Profil zmienności po godzinach — patrz bliźniacze miejsce
                // w resecie dobowym wyżej.
                let zmien = engine.stan_zmiennosci();
                // Stan trailingu S/R — patrz bliźniacze miejsce w resecie
                // dobowym wyżej (wiedza o rynku, nie stan konta).
                let sr = engine.stan_sr();
                let rl = (
                    engine.stats.relot_up_zdarzen,
                    engine.stats.relot_down_zdarzen,
                    engine.stats.relot_up_lotow,
                    engine.stats.relot_down_lotow,
                    engine.stats.relot_down_bez_spadku,
                    engine.stats.relot_up_ponad_plan,
                    engine.stats.relot_prob,
                    engine.stats.relot_udane,
                    engine.stats.relot_odmowy,
                    engine.stats.relot_plan_pusty,
                    engine.stats.relot_plan_ok,
                    engine.stats.relot_rozjazd_lotow,
                    engine.stats.relot_szczebli,
                    engine.stats.relot_ksztalt_odmowa,
                );
                // Ekspozycja: licznik jest wlasnoscia PRZEBIEGU, nie doby
                // ani szczebla drabinki — te same powody co przy relocie.
                let ex = (
                    engine.stats.expo_max_pct,
                    engine.stats.expo_zdarzen,
                    engine.stats.expo_pend_skasowane,
                    engine.stats.expo_lotow,
                    engine.stats.expo_poz_domkniete,
                    engine.stats.expo_niedosyt,
                );
                let kosz = std::mem::take(&mut engine.baskets);
                let slot = engine.slot();
                // tryb przebiegu przeżywa zmianę szczebla drabinki — patrz
                // ta sama linijka przy wymianie dobowej
                let auto_ea = engine.tryb_auto_ea;
                let ust = conduit_core::wielosilnik::ustawienia_formatu(&f.settings, &cfg.settings);
                // saldo bieżące, nie startowe: nowy silnik ma liczyć lot od
                // stanu konta, który zastał — dokładnie jak bot włączony dziś
                *engine = Engine::new(ust, saldo);
                engine.przypisz_slot(slot);
                engine.pulapy = sz.pulapy.clone();
                engine.tryb_auto_ea = auto_ea;
                engine.odrzuty = odrz;
                engine.odrzucone_wejscia = odrz_w;
                engine.stats.relot_up_zdarzen = rl.0;
                engine.stats.relot_down_zdarzen = rl.1;
                engine.stats.relot_up_lotow = rl.2;
                engine.stats.relot_down_lotow = rl.3;
                engine.stats.relot_down_bez_spadku = rl.4;
                engine.stats.relot_up_ponad_plan = rl.5;
                engine.stats.relot_prob = rl.6;
                engine.stats.relot_udane = rl.7;
                engine.stats.relot_odmowy = rl.8;
                engine.stats.relot_plan_pusty = rl.9;
                engine.stats.relot_plan_ok = rl.10;
                engine.stats.relot_rozjazd_lotow = rl.11;
                engine.stats.relot_szczebli = rl.12;
                engine.stats.relot_ksztalt_odmowa = rl.13;
                engine.stats.expo_max_pct = ex.0;
                engine.stats.expo_zdarzen = ex.1;
                engine.stats.expo_pend_skasowane = ex.2;
                engine.stats.expo_lotow = ex.3;
                engine.stats.expo_poz_domkniete = ex.4;
                engine.stats.expo_niedosyt = ex.5;
                engine.regime_hist = rezim;
                engine.set_stan_zmiennosci(zmien);
                engine.set_stan_sr(sr);
                engine.set_run_id("bt");
                engine.journal.cfg.enabled = dump.is_some() && engine.cfg.journal_enabled;
                engine.set_market_history(hist.0, hist.1);
                engine.adopt_baskets(kosz);
                s.preset = f.preset.clone();
                aktywne[idx] = true;
            }
            None => {
                // zarządzaj-nie-otwieraj: silnik i koszyki zostają, pułapy
                // przechodzą na bieżący szczebel (rachunek jest jeden)
                s.engine.pulapy = sz.pulapy.clone();
                aktywne[idx] = false;
            }
        }
    }
}

/// WSPÓLNA ścieżka budowy drabinki dla `bt` i `lotto` — jedna implementacja,
/// żeby sweep progów w LOTTO liczył DOKŁADNIE tę samą drabinkę, którą mierzy
/// backtest i którą dostaje sędzia.
/// JEDEN SZCZEBEL Z GOŁEGO PRESETU — bez łańcucha wbudowanego.
///
/// Format nogi bierzemy z pola `format` presetu (routing sygnałów po `kanal`
/// jest ten sam co przy `--preset-format`), a pułapy globalne zostają PUSTE.
/// To jest celowe: goły preset nie ma nad sobą łańcucha, więc jedynym sufitem
/// są jego własne `max_open_baskets` / `max_open_positions`. Pułap łańcucha
/// przycina limit do `min(preset, pułap)` — wstawienie tu czegokolwiek innego
/// niż zero cicho zmieniłoby to, co użytkownik napisał w pliku.
fn szczebel_z_presetu(
    prog: f64,
    plik: &std::path::Path,
    zrodlo: &str,
) -> anyhow::Result<SzczebelCfg> {
    let p: conduit_core::settings::Preset = std::fs::read_to_string(plik)
        .map_err(anyhow::Error::from)
        .and_then(|t| serde_json::from_str(&t).map_err(anyhow::Error::from))
        .map_err(|e| {
            anyhow::anyhow!("--drabinka: szczebel „{zrodlo}” ({}): {e}", plik.display())
        })?;
    if p.format.trim().is_empty() {
        anyhow::bail!(
            "--drabinka: preset {} nie ma pola `format` — nie wiadomo, który kanał ma grać",
            plik.display()
        );
    }
    let nazwa = if p.name.trim().is_empty() {
        plik.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| zrodlo.into())
    } else {
        p.name.clone()
    };
    Ok(SzczebelCfg {
        prog,
        nazwa,
        formaty: vec![FormatCfg {
            format: p.format.clone(),
            preset: p.name,
            settings: p.settings,
        }],
        pulapy: PulapyGlobalne::default(),
    })
}

/// Buduje szczeble drabinki z zapisu `PROG=SZCZEBEL,PROG=SZCZEBEL,…`.
///
/// SZCZEBEL wolno podać na trzy sposoby:
/// * `NAZWA` — łańcuch z `formaty.rs::lancuchy_wbudowane()` (postać pierwotna,
///   nietknięta: łańcuch niesie swoje nogi i swoje pułapy, presety nóg ładują
///   się z `presety_dir` PO NAZWIE z łańcucha, brak pliku to błąd twardy);
/// * `preset:NAZWA` — POJEDYNCZY preset `presety_dir/NAZWA.json`;
/// * `plik:ŚCIEŻKA` — POJEDYNCZY preset spod podanej ścieżki.
///
/// Dwie ostatnie postacie istnieją, bo drabinka jest narzędziem pomiarowym:
/// bez nich każdy nowy zestaw szczebli wymagał dopisania łańcucha do kodu
/// i przebudowy binarki, a pomiar 18.08.2026 musiał w tym celu przepisywać
/// pole `kanal` w korpusie na „ZEN” (jedyne łańcuchy bez pułapów globalnych).
/// Kolejność sprawdzania jest istotna: nazwa łańcucha wygrywa, więc każda
/// wcześniejsza specyfikacja znaczy DOKŁADNIE to samo co przedtem.
pub fn zbuduj_drabinke(
    spec: &str,
    presety_dir: &std::path::Path,
) -> anyhow::Result<Vec<SzczebelCfg>> {
    let wbudowane = conduit_core::formaty::lancuchy_wbudowane();
    let mut out: Vec<SzczebelCfg> = Vec::new();
    for czlon in spec.split(',') {
        let czlon = czlon.trim();
        if czlon.is_empty() {
            continue;
        }
        let (prog_s, nazwa) = czlon.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("--drabinka: oczekuję PROG=SZCZEBEL, dostałem „{czlon}”")
        })?;
        let prog: f64 = prog_s
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("--drabinka: próg „{prog_s}” nie jest liczbą"))?;
        let nazwa = nazwa.trim();
        // GOŁY PRESET — dwie postaci, jedna ścieżka kodu. Sprawdzane PRZED
        // łańcuchami, ale po prefiksie, więc kolizja nazw jest niemożliwa.
        if let Some(sciezka) = nazwa.strip_prefix("plik:") {
            out.push(szczebel_z_presetu(
                prog,
                std::path::Path::new(sciezka.trim()),
                nazwa,
            )?);
            continue;
        }
        if let Some(p_nazwa) = nazwa.strip_prefix("preset:") {
            let plik = presety_dir.join(format!("{}.json", p_nazwa.trim()));
            out.push(szczebel_z_presetu(prog, &plik, nazwa)?);
            continue;
        }
        let l = wbudowane.iter().find(|l| l.nazwa == nazwa).ok_or_else(|| {
            anyhow::anyhow!(
                "--drabinka: łańcuch „{nazwa}” nie istnieje wśród wbudowanych ({}). \
                     Pojedynczy preset podaje się jako „preset:NAZWA” (z --presety-dir) \
                     albo „plik:ŚCIEŻKA”",
                wbudowane
                    .iter()
                    .map(|x| x.nazwa.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let mut formaty: Vec<FormatCfg> = Vec::new();
        for (format, preset) in l.presety.iter().filter(|(_, p)| !p.is_empty()) {
            let plik = presety_dir.join(format!("{preset}.json"));
            let p: conduit_core::settings::Preset = std::fs::read_to_string(&plik)
                .map_err(anyhow::Error::from)
                .and_then(|t| serde_json::from_str(&t).map_err(anyhow::Error::from))
                .map_err(|e| {
                    anyhow::anyhow!("--drabinka: łańcuch „{nazwa}”, noga {format}→{preset}: {e}")
                })?;
            if p.name != *preset {
                eprintln!(
                    "⚠ drabinka: plik {} nosi nazwę presetu „{}”, łańcuch woła „{}” — \
                     jadę na zawartości pliku, ale to wygląda na podmieniony plik",
                    plik.display(),
                    p.name,
                    preset
                );
            }
            formaty.push(FormatCfg {
                format: format.clone(),
                preset: preset.clone(),
                settings: p.settings,
            });
        }
        if formaty.is_empty() {
            anyhow::bail!("--drabinka: łańcuch „{nazwa}” nie ma ani jednej handlującej nogi");
        }
        out.push(SzczebelCfg {
            prog,
            nazwa: nazwa.to_string(),
            formaty,
            pulapy: l.pulapy.clone(),
        });
    }
    if out.len() < 2 {
        anyhow::bail!(
            "--drabinka: podaj co najmniej dwa szczeble (jeden = zwykły --preset-format)"
        );
    }
    // rosnąco po progu + progi bez powtórek — drabinka z dwoma szczeblami na
    // tym samym progu nie ma jednoznacznego stanu
    for w in out.windows(2) {
        if w[1].prog <= w[0].prog {
            anyhow::bail!(
                "--drabinka: progi muszą rosnąć ({} po {} nie rośnie)",
                w[1].prog,
                w[0].prog
            );
        }
    }
    Ok(out)
}

fn fmt_day(day: i64) -> String {
    // dni od epoki → data (algorytm Hinnanta)
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zapisuje minimalny plik ticków w formacie `CDTK` (nagłówek 64 B,
    /// rekordy 16 B: `i64` znacznik, `f32` bid, `f32` ask).
    fn zapisz_ticki(sciezka: &std::path::Path, ticki: &[(Ts, f32, f32)]) {
        let mut buf = vec![0u8; 64];
        buf[0..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
        buf[8..16].copy_from_slice(&(ticki.len() as u64).to_le_bytes());
        for (ts, bid, ask) in ticki {
            buf.extend_from_slice(&ts.to_le_bytes());
            buf.extend_from_slice(&bid.to_le_bytes());
            buf.extend_from_slice(&ask.to_le_bytes());
        }
        std::fs::write(sciezka, buf).unwrap();
    }

    #[test]
    fn quick_n1_is_exact_path_and_n_gt_1_is_non_coronation() {
        let path=std::env::temp_dir().join(format!("conduit_quick_runner_{}.bin",std::process::id()));
        let t0=1_700_000_000_000i64;
        let rows:Vec<_>=(0..24).map(|i|{
            let bid=4000.0 + ((i*7)%11) as f32 - 5.0;
            (t0+i*1000,bid,bid+0.2)
        }).collect();
        zapisz_ticki(&path,&rows);
        let ticks=TickData::open(&path).unwrap();
        let base=RunConfig{from:t0,to:t0+24_000,..Default::default()};
        let mut explicit=base.clone();explicit.quick_tick_stride=1;
        let a=run(&ticks,&[],&base);let b=run(&ticks,&[],&explicit);
        assert_eq!(serde_json::to_value(&a.metrics).unwrap(),serde_json::to_value(&b.metrics).unwrap());
        assert_eq!(a.equity_curve,b.equity_curve);
        assert_eq!(a.balance_curve,b.balance_curve);
        assert_eq!(serde_json::to_value(&a.daily).unwrap(),serde_json::to_value(&b.daily).unwrap());
        assert_eq!(serde_json::to_value(&a.trades).unwrap(),serde_json::to_value(&b.trades).unwrap());
        assert_eq!(a.approximation,None);
        assert!(a.coronation_eligible());

        let mut quick=base;quick.quick_tick_stride=8;
        let q=run(&ticks,&[],&quick);
        let info=q.approximation.as_ref().expect("N>1 must self-label");
        assert_eq!(info.requested_stride,8);
        assert_eq!(info.raw_rows,24);
        assert!(info.observed_rows < info.raw_rows && info.extrema_preserved);
        assert!(!info.coronation_eligible && !q.coronation_eligible());
        drop(ticks);std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn credit_balance_separate_no_trades_credit_is_not_profit_and_zero_parity() {
        let path=std::env::temp_dir().join(format!("conduit_credit_report_{}.bin",std::process::id()));
        let t0=1_700_000_000_000i64;
        zapisz_ticki(&path,&[(t0,4000.0,4000.2),(t0+86_400_000,4000.0,4000.2)]);
        let ticks=TickData::open(&path).unwrap();
        let mut cfg=RunConfig {from:t0,to:t0+86_400_001,start_balance:600.0,..Default::default()};
        cfg.settings.credit_balance_separate=true;cfg.settings.kredyt_reczny=300.0;
        for reset in [false,true] {
            cfg.daily_reset=reset;
            let result=run(&ticks,&[],&cfg);
            let m=&result.metrics;
            assert_eq!(m.total_profit,0.0);assert_eq!(m.trades,0);
            assert_eq!(m.start_balance,600.0);assert_eq!(m.end_balance,600.0);
            assert_eq!(m.end_equity,600.0);assert_eq!(m.min_equity,600.0);
            assert_eq!(m.max_dd_abs,0.0);assert_eq!(m.return_pct,0.0);
            assert_eq!(m.raw_broker_end_equity,Some(900.0));
            assert_eq!(m.raw_broker_min_equity,Some(900.0));
            assert_eq!(m.initial_credit,Some(300.0));
            assert!(result.equity_curve.iter().all(|(_,e)|*e==600.0));
            assert!(result.daily.iter().all(|d|d.profit==0.0&&d.start_equity==600.0&&d.end_equity==600.0));
        }
        cfg.daily_reset=false;cfg.settings.kredyt_reczny=0.0;
        let on=run(&ticks,&[],&cfg);cfg.settings.credit_balance_separate=false;
        let off=run(&ticks,&[],&cfg);
        assert_eq!(serde_json::to_value(&on.metrics).unwrap(),serde_json::to_value(&off.metrics).unwrap());
        assert_eq!(on.equity_curve,off.equity_curve);
        assert!(!serde_json::to_string(&off.metrics).unwrap().contains("initial_credit"));
        drop(ticks);std::fs::remove_file(path).unwrap();
    }

    /// REGRESJA: zegar wiadomości musi być wyrównany do zegara ticków.
    ///
    /// `signals.json` niesie czas Telegrama (UTC), a `ticks.bin` czas serwera
    /// brokera (UTC+3, patrz `Settings::server_tz_offset_ms`). Gdy runner nie
    /// doda tej różnicy, silnik dostaje sygnał razem ze strumieniem cen sprzed
    /// trzech godzin i może wejść po kursie z przeszłości, znając już strefę i
    /// cele — czyli po prostu zerka w przyszłość.
    ///
    /// Konstrukcja testu: cena przez pierwsze trzy godziny stoi na 4000, potem
    /// skacze do 3000. Strefa wejścia to 2995–3005. Przy poprawnym wyrównaniu
    /// wiadomość dociera dopiero, gdy cena jest przy 3000, więc limit ma prawo
    /// się wypełnić. Przy braku wyrównania wiadomość trafia do silnika przy
    /// cenie 4000 — a to właśnie sytuacja, która wcześniej drukowała zysk.
    ///
    /// Test przypina SAMO PRZESUNIĘCIE, a nie wynik handlu: sprawdza, o ile
    /// runner opóźnia wiadomość względem jej znacznika.
    #[test]
    fn wiadomosci_wyrownane_do_zegara_tickow() {
        let s = Settings::default();
        assert_eq!(
            s.msg_offset(),
            3 * 3_600_000,
            "domyślnie zegar wiadomości przesuwamy o strefę serwera"
        );
        assert_eq!(
            s.session_offset(),
            0,
            "znacznik ticka JEST już czasem serwera — doby nie wolno przesuwać drugi raz"
        );

        let dir = std::env::temp_dir().join("conduit_test_zegar");
        std::fs::create_dir_all(&dir).unwrap();
        let plik = dir.join("ticks.bin");

        // 6 godzin ticków co minutę. Przez pierwsze trzy godziny cena stoi na
        // 4000 (daleko nad strefą). Potem zjeżdża na 3010 i powoli schodzi do
        // 2990 — czyli limit kupna w strefie 2995–3005 ma się czym wypełnić.
        let t0: Ts = 1_775_000_000_000;
        let ticki: Vec<(Ts, f32, f32)> = (0..360)
            .map(|i| {
                let ts = t0 + i as i64 * 60_000;
                let px: f32 = if i < 180 {
                    4000.0
                } else {
                    3010.0 - (i - 180) as f32 * 0.15
                };
                (ts, px, px + 0.30)
            })
            .collect();
        zapisz_ticki(&plik, &ticki);
        let dane = TickData::open(&plik).unwrap();

        // Wiadomość nadana w chwili t0 czasu TELEGRAMA (UTC).
        let msgs = vec![ReplayMessage {
            ts: t0,
            telegram_published_ts: None,
            msg_id: 1,
            reply_to: None,
            edit_of: None,
            text: "BUY GOLD @ 3005/2995

TP 3020
SL 2990"
                .into(),
            kanal: String::new(),
        }];

        let mut cfg = RunConfig {
            from: t0,
            to: t0 + 6 * 3_600_000,
            ..Default::default()
        };
        cfg.settings.exec_latency_ms = 0;
        cfg.settings.skip_if_sl_breached = false;

        let wynik = run(&dane, &msgs, &cfg);

        // Sedno: sygnał został przyjęty dopiero po przesunięciu o 3 h, czyli
        // gdy cena była już przy strefie 2995–3005, a nie przy 4000.
        assert!(
            !wynik.trades.is_empty(),
            "limit powinien się wypełnić po wyrównaniu zegara — bez przesunięcia \
             wiadomość dociera przy cenie 4000 i strefa 2995–3005 nigdy nie zadziała"
        );
        let pierwsze_wejscie = wynik
            .trades
            .iter()
            .map(|t| t.open_price)
            .fold(f64::INFINITY, f64::min);
        if pierwsze_wejscie.is_finite() {
            assert!(
                pierwsze_wejscie < 3100.0,
                "wejście po {pierwsze_wejscie:.2} — silnik zobaczył ceny sprzed \
                 wyrównania zegara (spodziewane ~3000, nie ~4000)"
            );
        }

        let _ = std::fs::remove_file(&plik);
    }

    /// REGRESJA: wiadomość nie może utworzyć zlecenia, które wypełni się
    /// tickiem już wykorzystanym do jej dostarczenia. To dokładnie kontrakt
    /// terminala MT5: najpierw broker rozgrywa istniejące zlecenia, potem EA
    /// i Conduit dostają zdarzenie.
    #[test]
    fn strict_tick_order_nie_wypelnia_zlecenia_wstecz() {
        let dir =
            std::env::temp_dir().join(format!("conduit_test_causal_order_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plik = dir.join("ticks.bin");
        let t0: Ts = 1_775_100_000_000;
        zapisz_ticki(
            &plik,
            &[
                (t0, 4010.0, 4010.2),
                (t0 + 1_000, 3999.0, 3999.2),
                (t0 + 2_000, 3999.0, 3999.2),
            ],
        );
        let dane = TickData::open(&plik).unwrap();
        let msgs = vec![ReplayMessage {
            ts: t0 + 1_000,
            telegram_published_ts: None,
            msg_id: 77,
            reply_to: None,
            edit_of: None,
            text: "BUY LIMITS GOLD @ 4001/4000 AREA\nTP 4010\nSL 3990".into(),
            kanal: String::new(),
        }];

        let mut legacy = RunConfig {
            from: t0,
            to: t0 + 3_000,
            ..Default::default()
        };
        legacy.settings.server_tz_offset_ms = 0;
        legacy.settings.exec_latency_ms = 0;
        legacy.settings.msg_kurs_sprzed_luki = true;
        legacy.settings.skip_if_sl_breached = false;

        let stary = run(&dane, &msgs, &legacy);
        let mut strict = legacy.clone();
        strict.settings.live_tick_order_strict = true;
        let nowy = run(&dane, &msgs, &strict);

        let pierwszy_fill = |r: &RunResult| {
            r.baskets_dump
                .iter()
                .flat_map(|b| b.warstwy.iter())
                .filter(|w| w.fill_ts > 0)
                .map(|w| w.fill_ts)
                .min()
                .expect("co najmniej jedna warstwa powinna się wypełnić")
        };
        assert_eq!(
            pierwszy_fill(&stary),
            t0 + 1_000,
            "legacy przypina fill do bieżącego ticka"
        );
        assert_eq!(
            pierwszy_fill(&nowy),
            t0 + 2_000,
            "strict: zlecenie powstałe po ticku może wejść dopiero na następnym ticku"
        );

        let _ = std::fs::remove_file(&plik);
        let _ = std::fs::remove_dir(&dir);
    }

    /// The switch is a true pre-Engine LIVE ingress gate. A reconnect can
    /// redeliver the same Telegram NEW with the same id and text; ordinary
    /// replay keeps its historical behavior, while live replay routes it once.
    #[test]
    fn live_telegram_ingress_drops_exact_redelivery_before_engine() {
        let dir = std::env::temp_dir().join(format!(
            "conduit_test_live_ingress_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ticks.bin");
        let t0: Ts = 1_775_200_000_000;
        zapisz_ticki(
            &path,
            &[
                (t0, 4000.0, 4000.2),
                (t0 + 1_000, 4000.0, 4000.2),
                (t0 + 2_000, 4000.0, 4000.2),
            ],
        );
        let ticks = TickData::open(&path).unwrap();
        let message = ReplayMessage {
            ts: t0 + 500,
            telegram_published_ts: Some(t0 + 400),
            msg_id: 9191,
            reply_to: None,
            edit_of: None,
            text: "BUY LIMITS GOLD @ 3999/3998 AREA\nTP 4010\nSL 3990".into(),
            kanal: "Synergy".into(),
        };
        let messages = vec![message.clone(), message];
        let mut cfg = RunConfig {
            from: t0,
            to: t0 + 3_000,
            ..Default::default()
        };
        cfg.settings.server_tz_offset_ms = 0;
        cfg.settings.exec_latency_ms = 0;
        cfg.settings.msg_kurs_sprzed_luki = true;

        let ordinary = run(&ticks, &messages, &cfg);
        assert_eq!(ordinary.metrics.stat_sygnalow.lejek.sygnaly_wejsciowe, 2);

        cfg.live_telegram_ingress = true;
        let live = run(&ticks, &messages, &cfg);
        assert_eq!(live.metrics.stat_sygnalow.lejek.sygnaly_wejsciowe, 1);

        drop(ticks);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn formatowanie_daty() {
        // 2026-04-01 = 20544 dni od epoki
        let d = (chrono_days(2026, 4, 1)) as i64;
        assert_eq!(fmt_day(d), "2026-04-01");
        assert_eq!(fmt_day(chrono_days(2026, 12, 31) as i64), "2026-12-31");
        assert_eq!(fmt_day(0), "1970-01-01");
    }

    fn chrono_days(y: i64, m: i64, d: i64) -> i64 {
        let yy = if m <= 2 { y - 1 } else { y };
        let era = if yy >= 0 { yy } else { yy - 399 } / 400;
        let yoe = yy - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Pusty zakres nie może zawiesić ani wywołać sygnalizatora.
    #[test]
    fn pusty_zakres_nie_wola_sygnalizatora() {
        // `TickData` wymaga pliku, więc tu sprawdzamy tylko kontrakt typu:
        // domyślnie `cancelled` jest fałszem i nie ma czego raportować.
        let r = RunResult {
            approximation: None,
            cost_reconciliation_required: None,
            sr_warmup_reconciliation_required: None,
            sim_execution_reconciliation_required: None,
            continuation_reconciliation_required: None,
            continuation_scope: None,
            metrics: Metrics::default(),
            equity_curve: Vec::new(),
            balance_curve: Vec::new(),
            daily: Vec::new(),
            trades: Vec::new(),
            baskets_dump: Vec::new(),
            ticks_processed: 0,
            swap_paid: 0.0,
            stop_outs: 0,
            elapsed_ms: 0,
            journal_lines: 0,
            cancelled: false,
            formaty: Vec::new(),
            bez_trasy: Default::default(),
            szczeble: Vec::new(),
            przelaczenia: Vec::new(),
        };
        assert!(!r.cancelled);
    }

    /// REGRESJA: wynik zserializowany bez pola `cancelled` (starsze pliki
    /// `wyniki_*.json`) MUSI się dalej wczytywać.
    #[test]
    fn stary_wynik_bez_pola_cancelled_wczytuje_sie() {
        let json = r#"{
            "metrics": {"start_balance":200.0,"end_balance":0.0,"end_equity":0.0,"total_profit":0.0,
              "return_pct":0.0,"monthly_profit":0.0,"days":0,"trading_days":0,"avg_per_day":0.0,
              "median_day":0.0,"best_day":0.0,"worst_day":0.0,"win_days":0,"win_days_pct":0.0,
              "max_losing_streak_days":0,"max_dd_abs":0.0,"max_dd_pct":0.0,"max_daily_dd":0.0,
              "min_equity":0.0,"max_floating_loss":0.0,"max_open_risk":0.0,"max_open_risk_pct":0.0,
              "max_open_positions":0,"blown":false,"recovery_factor":0.0,"calmar":0.0,
              "ulcer_index":0.0,"trades":0,"wins":0,"losses":0,"win_rate":0.0,"profit_factor":0.0,
              "expectancy":0.0,"avg_win":0.0,"avg_loss":0.0,"largest_win":0.0,"largest_loss":0.0,
              "avg_hold_min":0.0,"median_hold_min":0.0,"max_consecutive_losses":0,"sharpe":0.0,
              "sortino":0.0,"median_time_to_profit":0.0,"signals_seen":0,"signals_taken":0,
              "baskets":0,"rejected_stops":0,"market_instead_of_limit":0},
            "equity_curve": [], "daily": [], "ticks_processed": 5, "elapsed_ms": 1
        }"#;
        let r: RunResult = serde_json::from_str(json).expect("stary format musi się wczytać");
        assert!(!r.cancelled);
        assert_eq!(r.ticks_processed, 5);
    }
}
