
use std::collections::BTreeMap;

use conduit_core::engine::OdrzuconeWejscie;
use conduit_core::types::{ClosedTrade, Px, Side, XAU_CONTRACT};
use serde::{Deserialize, Serialize};

use crate::data::TickData;
use crate::metrics::Metrics;
use crate::runner::BasketDump;

/// Ile kubełków ma histogram R-multiple. Granice w [`R_GRANICE`].
pub const R_KUBELKOW: usize = 8;
/// Górne granice kubełków R (ostatni jest otwarty w górę).
pub const R_GRANICE: [f64; R_KUBELKOW - 1] = [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0];

/// Nazwy kubełków R do druku — trzymane obok granic, żeby nie rozjechały się
/// z nimi przy pierwszej zmianie.
pub fn r_nazwy() -> Vec<String> {
    let mut v = Vec::with_capacity(R_KUBELKOW);
    v.push(format!("<{:.1}", R_GRANICE[0]));
    for i in 1..R_GRANICE.len() {
        v.push(format!("{:.1}..{:.1}", R_GRANICE[i - 1], R_GRANICE[i]));
    }
    v.push(format!(">{:.1}", R_GRANICE[R_GRANICE.len() - 1]));
    v
}

fn kubelek_r(r: f64) -> usize {
    for (i, g) in R_GRANICE.iter().enumerate() {
        if r < *g {
            return i;
        }
    }
    R_KUBELKOW - 1
}

fn mediana(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) * 0.5
    }
}

fn percentyl(v: &mut Vec<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i]
}

/// Jedna komórka rozkładu: ile sztuk i ile dolarów.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Komorka {
    pub n: u32,
    pub usd: f64,
}

impl Komorka {
    fn dodaj(&mut self, usd: f64) {
        self.n += 1;
        self.usd += usd;
    }
}

/// Okno czasowe (godzina wejścia albo dzień tygodnia).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatOkna {
    pub n: u32,
    pub usd: f64,
    pub win: u32,
    pub win_pct: f64,
}

/// Wynik jednej STRONY rynku.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatStrony {
    pub n: u32,
    pub win: u32,
    pub win_pct: f64,
    pub usd: f64,
    pub profit_factor: f64,
    pub swap: f64,
}

/// KOSZYK JAKO CAŁOŚĆ (E2).
///
/// Transakcja to szczebel, nie sygnał. Preset z ośmioma szczeblami na sygnał
/// pokazuje w statystyce transakcji ośmiokrotnie zawyżoną próbkę i skuteczność
/// policzoną na czymś, czego kanał nigdy nie ogłosił. Tu jednostką jest koszyk,
/// czyli jeden WYKONANY sygnał.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatKoszykow {
    pub total: u32,
    /// koszyki, w których cokolwiek się wypełniło
    pub z_pozycjami: u32,
    /// siatka stała i nic nie weszło — sygnał policzony, handlu nie było
    pub bez_fillu: u32,
    pub win: u32,
    pub loss: u32,
    pub be: u32,
    pub win_pct: f64,
    pub suma_usd: f64,
    pub srednia_usd: f64,
    pub mediana_usd: f64,
    /// rozkład po POWODZIE ŚMIERCI (ostatnie zamknięcie koszyka)
    pub powody: BTreeMap<String, Komorka>,
    /// ile koszyków dotknęło TP1 / TP2 / TP3 (z `tp_touch_ts`, czyli z odczytu
    /// silnika, nie z rekonstrukcji)
    pub tp1: u32,
    pub tp2: u32,
    pub tp3: u32,
    /// mediana czasu sygnał → pierwsze dotknięcie TP1 (minuty)
    pub med_do_tp1_min: f64,
    /// histogram liczby WYPEŁNIONYCH szczebli (indeks = ile szczebli)
    pub fill_hist: Vec<u32>,
    /// średni udział wypełnionych szczebli w planie (0–1)
    pub fill_udzial: f64,
    /// 24 godziny czasu SERWERA (znacznik koszyka jest już w czasie serwera)
    pub godziny: Vec<StatOkna>,
    /// 7 dni tygodnia, indeks 0 = poniedziałek
    pub dni: Vec<StatOkna>,
}

/// LEJEK SYGNAŁÓW (E3) — od wiadomości do zrealizowanego handlu.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Lejek {
    /// wiadomości z akcją wejścia, które w ogóle dotarły do bramek
    /// (mianownik WSTECZ: koszyki + odrzucone — zostaje jako odczyt
    /// porównawczy, ale nie widzi sygnałów zgubionych bez jreject)
    pub sygnaly_widziane: u32,
    /// LICZNIK W PRZÓD: każda ŚWIEŻA wiadomość (bez `edit_of` i `reply_to`)
    /// z akcją Entry|MarketOpen, policzona w pętli przebiegu PRZED routingiem
    /// i wszystkimi bramkami. 0 = przebieg sprzed licznika (stare archiwa).
    pub sygnaly_wejsciowe: u32,
    /// UNIKALNE `msg_id` koszyków — sygnały, które OSTATECZNIE weszły.
    /// Osobno od `koszyki`, bo re-arm potrafi założyć kilka koszyków z jednej
    /// wiadomości, a lejek liczy wiadomości, nie koszyki.
    pub koszyki_sygnaly: u32,
    pub odrzucone_sygnaly: u32,
    /// `sygnaly_wejsciowe − koszyki_sygnaly − odrzucone_sygnaly`: sygnały,
    /// które weszły do silnika i wyszły BEZ koszyka i BEZ jreject — dotąd
    /// znikały bez śladu i mianownik wstecz ich nie widział. Ujemne znaczy,
    /// że odrzuty liczą coś spoza licznika w przód (`BrakFormatu` zlicza
    /// WSZYSTKIE wiadomości kanału, nie tylko wejścia) — to też jest
    /// informacja, dlatego bez ucinania do zera.
    pub zgubione_bez_sladu: i64,
    /// odrzucenia wg kodu — te same kubełki co `Metrics::odrzuty`
    pub odrzucone: BTreeMap<String, u32>,
    pub odrzucone_razem: u32,
    pub koszyki: u32,
    pub koszyk_bez_fillu: u32,
    pub koszyk_z_handlem: u32,
    /// CEL WŁAŚCICIELA: 100 %. Koszyki z handlem / sygnały widziane.
    pub wykonanych_pct: f64,
    /// wycena filtrów wg kodu (patrz [`KosztFiltra`])
    pub koszt_filtrow: BTreeMap<String, KosztFiltra>,
    /// suma wyceny wszystkich odrzutów — dodatnia znaczy „filtry kosztowały"
    pub koszt_filtrow_usd: f64,
    /// ile odrzutów udało się wycenić (reszta nie miała SL albo celu)
    pub wycenionych: u32,
}

/// WYCENA JEDNEGO KODU ODRZUTU.
///
/// Model rozmyślnie NAJPROŚTSZY, jak w wyroczni L0: jedna jednostka 0,01 lota
/// wchodzi po cenie z chwili sygnału i biegnie do TP1 albo do SL, cokolwiek
/// tick trafi pierwsze. To NIE jest kontrfaktyczny wynik presetu (siatka,
/// partiale i runnery zmieniłyby wszystko) — to wspólna miarka, którą wolno
/// PORÓWNYWAĆ między kodami: „reżim odrzucał sygnały lepsze niż sesja".
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KosztFiltra {
    pub n: u32,
    /// ile z nich dało się wycenić
    pub wycenione: u32,
    pub tp1: u32,
    pub sl: u32,
    /// ani TP1, ani SL do końca danych
    pub bez_rozstrzygniecia: u32,
    /// suma wyniku hipotetycznego ($ na 0,01 lota)
    pub usd: f64,
}

/// TRANSAKCJE — rozszerzenia per pozycja (E4).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatTransakcji {
    /// histogram R-multiple (`profit / ryzyko`), [`R_KUBELKOW`] kubełków
    pub r_hist: Vec<u32>,
    pub r_nazwy: Vec<String>,
    /// ile transakcji miało policzalne R (był SL na wejściu)
    pub r_n: u32,
    pub r_srednie: f64,
    pub r_mediana: f64,
    pub buy: StatStrony,
    pub sell: StatStrony,
    /// jeden tick przebił SL i TP tej samej pozycji — na tickach MUSI być 0
    pub sl_tp_same_tick: u64,
    /// spread zapłacony przy otwarciach ($)
    pub spread_usd: f64,
    /// swap zapłacony ($; ujemny = koszt)
    pub swap_usd: f64,
    pub hold_p90_min: f64,
}

/// Komplet statystyk Pakietu E dla JEDNEGO przebiegu.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatSygnalow {
    pub lejek: Lejek,
    pub koszyki: StatKoszykow,
    pub transakcje: StatTransakcji,
}

/// Wejście do [`policz`] — wszystko, co przebieg zdążył zebrać.
pub struct Wejscie<'a> {
    pub metrics: &'a Metrics,
    pub koszyki: &'a [BasketDump],
    pub trades: &'a [ClosedTrade],
    pub odrzucone: &'a [OdrzuconeWejscie],
    pub sl_tp_same_tick: u64,
    pub spread_usd: f64,
    /// próg remisu w $ (`Settings::stat_be_prog_usd`)
    pub prog_be: f64,
    /// ticki do wyceny odrzutów; `None` = wycena pominięta
    pub ticks: Option<&'a TickData>,
    /// przesunięcie czasu serwera (`Settings::server_tz_offset_ms`) — godziny
    /// i dni tygodnia liczymy w TYM czasie, bo w nim mówi cały reszta raportu
    pub tz_offset_ms: i64,
    /// licznik W PRZÓD z pętli przebiegu ([`Lejek::sygnaly_wejsciowe`]);
    /// 0 = wołający licznika nie prowadzi (wtedy lejek liczy jak dotąd)
    pub sygnaly_wejsciowe: u32,
}

pub fn policz(w: Wejscie<'_>) -> StatSygnalow {
    StatSygnalow {
        koszyki: policz_koszyki(w.koszyki, w.trades, w.prog_be, w.tz_offset_ms),
        lejek: policz_lejek(
            w.metrics,
            w.koszyki,
            w.odrzucone,
            w.ticks,
            w.sygnaly_wejsciowe,
        ),
        transakcje: policz_transakcje(w.trades, w.sl_tp_same_tick, w.spread_usd),
    }
}

// ============================================================
//  E2 — KOSZYKI
// ============================================================

fn policz_koszyki(
    koszyki: &[BasketDump],
    trades: &[ClosedTrade],
    prog_be: f64,
    tz: i64,
) -> StatKoszykow {
    // POWÓD ŚMIERCI = powód zamknięcia OSTATNIEJ pozycji koszyka.
    //
    // Odtwarzanie go ze stanu zrzutu („dotknięty stop albo etap celu")
    // kłamało dokładnie tam, gdzie boli: koszyk, który zebrał trzy cele
    // i dopiero potem oddał resztkę na stopie, wyglądał jak koszyk zabity
    // stopem — z dodatnim wynikiem obok. Historia brokera zna prawdę.
    //
    // KLUCZ TO (id, chwila ostatniego zamknięcia), nie samo id: w trybie
    // dziennym silnik jest wymieniany i id koszyków POWTARZAJĄ SIĘ co dobę,
    // więc mapa po samym id przypisywała każdemu segmentowi powód z tego
    // dnia, który zamknął się ostatni. `last_close_ts` zrzutu pochodzi
    // z tego samego wycinka historii co `pl`, więc trafia we własny dzień.
    let mut ostatni: std::collections::HashMap<(u32, i64), String> =
        std::collections::HashMap::new();
    for t in trades {
        let Some(id) = t.basket else { continue };
        // przy kilku zamknięciach w tej samej milisekundzie wygrywa ostatnie
        // z listy — dokładnie tak, jak rozstrzygało `close_ts >= w.0`
        ostatni.insert((id, t.close_ts), format!("{:?}", t.reason));
    }

    let mut s = StatKoszykow {
        total: koszyki.len() as u32,
        fill_hist: Vec::new(),
        godziny: vec![StatOkna::default(); 24],
        dni: vec![StatOkna::default(); 7],
        ..Default::default()
    };
    let prog = prog_be.abs();
    let mut wyniki: Vec<f64> = Vec::with_capacity(koszyki.len());
    let mut do_tp1: Vec<f64> = Vec::new();
    let mut udzialy: Vec<f64> = Vec::new();

    for k in koszyki {
        // „Miał pozycje" bierzemy z LICZBY TRANSAKCJI, nie z flagi `had_positions`:
        // flaga mówi o stanie silnika w chwili zrzutu, a nas interesuje, czy
        // z koszyka wyszedł jakikolwiek zrealizowany handel.
        // FILL-RATE WARSTW liczymy dla KAŻDEGO koszyka, także tego, w którym
        // nic nie weszło — zero wypełnionych szczebli jest tu odpowiedzią,
        // nie brakiem danych. Pominięcie takich koszyków zawyżałoby udział
        // wypełnień dokładnie o tę część planu, która nigdy nie zagrała.
        let plan = k.warstwy.len();
        if plan > 0 {
            let wypelnione = k
                .warstwy
                .iter()
                .filter(|x| x.filled || x.fill_ts > 0)
                .count();
            if s.fill_hist.len() <= wypelnione {
                s.fill_hist.resize(wypelnione + 1, 0);
            }
            s.fill_hist[wypelnione] += 1;
            udzialy.push(wypelnione as f64 / plan as f64);
        }

        // ETAPY CELÓW — z odczytu silnika (`tp_touch_ts`), nie z rekonstrukcji
        // po cenach: rekonstrukcja myli się dokładnie tam, gdzie rynek
        // przeskakuje poziom.
        // PRZED odsianiem koszyków bez handlu: „TP dotknięty bez wejścia"
        // to sedno sporu o pokrycie — liczony dopiero po `continue` był
        // niewidzialny i statystyka dotknięć opisywała tylko koszyki z fillem.
        let dotkniete = |i: usize| k.tp_touch_ts.get(i).copied().unwrap_or(0) > 0;
        if dotkniete(0) {
            s.tp1 += 1;
            if k.created_ts > 0 && k.tp_touch_ts[0] > k.created_ts {
                do_tp1.push((k.tp_touch_ts[0] - k.created_ts) as f64 / 60_000.0);
            }
        }
        if dotkniete(1) {
            s.tp2 += 1;
        }
        if dotkniete(2) {
            s.tp3 += 1;
        }

        let handlowal = k.n_trades > 0;
        if handlowal {
            s.z_pozycjami += 1;
        } else {
            s.bez_fillu += 1;
            continue;
        }

        s.suma_usd += k.pl;
        wyniki.push(k.pl);
        if k.pl.abs() <= prog {
            s.be += 1;
        } else if k.pl > 0.0 {
            s.win += 1;
        } else {
            s.loss += 1;
        }

        let powod = ostatni
            .get(&(k.id, k.last_close_ts))
            .cloned()
            .unwrap_or_else(|| "?".into());
        s.powody.entry(powod).or_default().dodaj(k.pl);

        // OKNA CZASOWE — chwila POWSTANIA koszyka, czyli sygnału.
        if k.created_ts > 0 {
            let lok = k.created_ts + tz;
            let godz = ((lok / 3_600_000) % 24).rem_euclid(24) as usize;
            let dzien = (((lok / 86_400_000) + 3).rem_euclid(7)) as usize;
            for cel in [&mut s.godziny[godz], &mut s.dni[dzien]] {
                cel.n += 1;
                cel.usd += k.pl;
                if k.pl > prog {
                    cel.win += 1;
                }
            }
        }
    }

    let n = s.win + s.loss + s.be;
    if n > 0 {
        s.win_pct = s.win as f64 / n as f64 * 100.0;
        s.srednia_usd = s.suma_usd / n as f64;
    }
    s.mediana_usd = mediana(&mut wyniki);
    s.med_do_tp1_min = mediana(&mut do_tp1);
    if !udzialy.is_empty() {
        s.fill_udzial = udzialy.iter().sum::<f64>() / udzialy.len() as f64;
    }
    for o in s.godziny.iter_mut().chain(s.dni.iter_mut()) {
        if o.n > 0 {
            o.win_pct = o.win as f64 / o.n as f64 * 100.0;
        }
    }
    s
}

// ============================================================
//  E3 — LEJEK I WYCENA FILTRÓW
// ============================================================

fn policz_lejek(
    m: &Metrics,
    koszyki: &[BasketDump],
    odrzucone: &[OdrzuconeWejscie],
    ticks: Option<&TickData>,
    sygnaly_wejsciowe: u32,
) -> Lejek {
    let mut l = Lejek {
        koszyki: koszyki.len() as u32,
        koszyk_z_handlem: koszyki.iter().filter(|k| k.n_trades > 0).count() as u32,
        ..Default::default()
    };
    l.koszyk_bez_fillu = l.koszyki - l.koszyk_z_handlem;

    // ODRZUCONE LICZYMY Z REJESTRU WEJŚĆ, nie z `Metrics::odrzuty`.
    //
    // `odrzuty` to licznik WSZYSTKICH odrzuconych AKCJI: siedzą w nim także
    // komunikaty zarządzające (hamulce, sieroty edycji, budżet ryzyka), które
    // nigdy nie miały być wejściem. Lejek ma opisywać drogę SYGNAŁU, więc
    // mianownik musi liczyć wyłącznie wiadomości, które o wejście prosiły.
    for o in odrzucone {
        *l.odrzucone.entry(o.kod.clone()).or_insert(0) += 1;
        l.odrzucone_razem += 1;
    }
    // Wyjątek: sygnał BEZ TRASY (kanał, którego łańcuch nie obsługuje) i
    // sygnał na szczeblu bez nogi nie docierają do żadnej bramki, więc nie ma
    // ich w rejestrze — a są stratą lejka jak każda inna. Klucze mają prefiks
    // nadany w `runner`, więc rozpoznajemy je bez zgadywania.
    for (k, v) in &m.odrzuty {
        if k.starts_with("BrakFormatu:") || k.starts_with("SzczebelBezNogi:") {
            *l.odrzucone.entry(k.clone()).or_insert(0) += *v as u32;
            l.odrzucone_razem += *v as u32;
        }
    }

    // MIANOWNIK WSTECZ: każda wiadomość z wejściem albo stała się koszykiem,
    // albo odpadła na bramce. Świadomie NIE `signals_taken` — ono liczy
    // przyjęte AKCJE (także „TP1 HIT"), więc dawało mianownik kilkanaście
    // razy za duży. Z definicji nie widzi jednak sygnału, który wszedł do
    // silnika i wyszedł bez jreject i bez koszyka — dlatego niżej licznik
    // W PRZÓD z pętli przebiegu przejmuje rolę mianownika, gdy tylko jest.
    l.sygnaly_widziane = l.koszyki + l.odrzucone_razem;
    l.sygnaly_wejsciowe = sygnaly_wejsciowe;
    if sygnaly_wejsciowe > 0 {
        let kosz_msg: std::collections::HashSet<i64> = koszyki.iter().map(|k| k.msg_id).collect();
        let odrz_msg: std::collections::HashSet<i64> = odrzucone
            .iter()
            .map(|o| o.msg_id)
            .filter(|id| !kosz_msg.contains(id))
            .collect();
        // BrakFormatu/SzczebelBezNogi są liczone w runnerze już na poziomie
        // wiadomości, więc wchodzą bez dedupe.
        let bez_trasy: u32 = m
            .odrzuty
            .iter()
            .filter(|(k, _)| k.starts_with("BrakFormatu:") || k.starts_with("SzczebelBezNogi:"))
            .map(|(_, v)| *v as u32)
            .sum();
        l.koszyki_sygnaly = kosz_msg.len() as u32;
        l.odrzucone_sygnaly = odrz_msg.len() as u32 + bez_trasy;
        // różnica ze znakiem: ujemna zdradza odrzuty spoza licznika w przód
        // (np. `BrakFormatu` liczy wszystkie wiadomości kanału, nie wejścia)
        l.zgubione_bez_sladu =
            sygnaly_wejsciowe as i64 - l.koszyki_sygnaly as i64 - l.odrzucone_sygnaly as i64;
    }
    let mianownik = if sygnaly_wejsciowe > 0 {
        sygnaly_wejsciowe
    } else {
        l.sygnaly_widziane
    };
    if mianownik > 0 {
        l.wykonanych_pct = l.koszyk_z_handlem as f64 / mianownik as f64 * 100.0;
    }

    for o in odrzucone {
        let wpis = l.koszt_filtrow.entry(o.kod.clone()).or_default();
        wpis.n += 1;
        let Some(td) = ticks else { continue };
        match wycen(o, td) {
            Wycena::Tp1(usd) => {
                wpis.wycenione += 1;
                wpis.tp1 += 1;
                wpis.usd += usd;
                l.wycenionych += 1;
                l.koszt_filtrow_usd += usd;
            }
            Wycena::Sl(usd) => {
                wpis.wycenione += 1;
                wpis.sl += 1;
                wpis.usd += usd;
                l.wycenionych += 1;
                l.koszt_filtrow_usd += usd;
            }
            Wycena::Brak => wpis.bez_rozstrzygniecia += 1,
        }
    }
    l
}

enum Wycena {
    Tp1(f64),
    Sl(f64),
    Brak,
}

/// Jedna jednostka 0,01 lota — najmniejsza, jaką broker w ogóle przyjmie.
const WYCENA_LOT: f64 = 0.01;

/// Hipotetyczne wykonanie odrzuconego sygnału: wejście po cenie z chwili
/// sygnału, wyjście na TP1 albo SL, cokolwiek tick trafi pierwsze.
///
/// Wejście bierzemy po BLIŻSZEJ krawędzi strefy (dla BUY po `hi`, dla SELL
/// po `lo`) tylko wtedy, gdy rynek tam stoi; w przeciwnym razie po cenie
/// rynkowej. To jest ta sama zasada co w silniku: sygnał limitowy, którego
/// strefy rynek nigdy nie dotknął, nie ma prawa liczyć się jako wykonany —
/// wtedy zwracamy [`Wycena::Brak`].
fn wycen(o: &OdrzuconeWejscie, td: &TickData) -> Wycena {
    let (Some(sl), Some(tp1)) = (o.sl, o.tp1) else {
        return Wycena::Brak;
    };
    let start = td.index_at(o.ts);
    if start >= td.len() {
        return Wycena::Brak;
    }
    let (lo, hi) = (o.lo.min(o.hi), o.lo.max(o.hi));

    // 1. czekamy, aż rynek dotknie strefy (albo od razu w niej stoi)
    let mut i = start;
    let mut wejscie: Option<(usize, Px)> = None;
    while i < td.len() {
        let (bid, ask) = (td.bid(i), td.ask(i));
        let cena = match o.side {
            Side::Buy => ask,
            Side::Sell => bid,
        };
        if cena >= lo && cena <= hi {
            wejscie = Some((i, cena));
            break;
        }
        i += 1;
        // Sygnał, którego strefy rynek nie dotknął przez dobę, nie wszedłby
        // także w silniku — trzymanie go dłużej mierzyłoby inną strategię.
        if td.ts(i.min(td.len() - 1)) - o.ts > 86_400_000 {
            return Wycena::Brak;
        }
    }
    let Some((i0, px)) = wejscie else {
        return Wycena::Brak;
    };

    // 2. bieg do pierwszego z dwóch poziomów
    let mut i = i0 + 1;
    while i < td.len() {
        let wyjscie = match o.side {
            Side::Buy => td.bid(i),
            Side::Sell => td.ask(i),
        };
        let (sl_hit, tp_hit) = match o.side {
            Side::Buy => (wyjscie <= sl, wyjscie >= tp1),
            Side::Sell => (wyjscie >= sl, wyjscie <= tp1),
        };
        // SL ma pierwszeństwo — dokładnie jak w symulatorze
        if sl_hit {
            return Wycena::Sl(zysk(o.side, px, sl));
        }
        if tp_hit {
            return Wycena::Tp1(zysk(o.side, px, tp1));
        }
        i += 1;
    }
    Wycena::Brak
}

fn zysk(side: Side, wejscie: Px, wyjscie: Px) -> f64 {
    let kierunek = match side {
        Side::Buy => 1.0,
        Side::Sell => -1.0,
    };
    (wyjscie - wejscie) * kierunek * XAU_CONTRACT * WYCENA_LOT
}

// ============================================================
//  E4 — TRANSAKCJE
// ============================================================

fn policz_transakcje(trades: &[ClosedTrade], sl_tp: u64, spread: f64) -> StatTransakcji {
    let mut s = StatTransakcji {
        r_hist: vec![0; R_KUBELKOW],
        r_nazwy: r_nazwy(),
        sl_tp_same_tick: sl_tp,
        spread_usd: spread,
        ..Default::default()
    };
    let mut r_wartosci: Vec<f64> = Vec::new();
    let mut holds: Vec<f64> = Vec::with_capacity(trades.len());
    let (mut buy_g, mut buy_s) = (0.0, 0.0);
    let (mut sell_g, mut sell_s) = (0.0, 0.0);

    for t in trades {
        s.swap_usd += t.swap;
        holds.push((t.close_ts - t.open_ts) as f64 / 60_000.0);

        let strona = match t.side {
            Side::Buy => &mut s.buy,
            Side::Sell => &mut s.sell,
        };
        strona.n += 1;
        strona.usd += t.profit;
        strona.swap += t.swap;
        if t.profit > 0.0 {
            strona.win += 1;
        }
        match t.side {
            Side::Buy => {
                if t.profit > 0.0 {
                    buy_g += t.profit
                } else {
                    buy_s += -t.profit
                }
            }
            Side::Sell => {
                if t.profit > 0.0 {
                    sell_g += t.profit
                } else {
                    sell_s += -t.profit
                }
            }
        }
    }

    // R-MULTIPLE. `ClosedTrade` nie niesie stopu, więc ryzyko odtwarzamy
    // z transakcji zamkniętych STOPEM: dla nich |wejście − wyjście| JEST
    // zrealizowanym ryzykiem, a mediana tej wielkości daje jednostkę R dla
    // całej reszty. To jest przybliżenie i tak ma być czytane — dokładne R
    // per transakcja wymagałoby przeniesienia SL do `ClosedTrade`, czyli
    // zmiany w rdzeniu, na którą Pakiet E nie ma zgody.
    let mut ryzyka: Vec<f64> = trades
        .iter()
        .filter(|t| matches!(t.reason, conduit_core::types::CloseReason::Sl))
        .map(|t| (t.close_price - t.open_price).abs() * XAU_CONTRACT * t.volume)
        .filter(|x| *x > 0.0)
        .collect();
    let jednostka = mediana(&mut ryzyka);
    if jednostka > 0.0 {
        for t in trades {
            let r = t.profit / jednostka;
            s.r_hist[kubelek_r(r)] += 1;
            r_wartosci.push(r);
        }
        s.r_n = r_wartosci.len() as u32;
        s.r_srednie = r_wartosci.iter().sum::<f64>() / r_wartosci.len().max(1) as f64;
        s.r_mediana = mediana(&mut r_wartosci.clone());
    }

    for (strona, g, l) in [(&mut s.buy, buy_g, buy_s), (&mut s.sell, sell_g, sell_s)] {
        if strona.n > 0 {
            strona.win_pct = strona.win as f64 / strona.n as f64 * 100.0;
        }
        strona.profit_factor = if l > 0.0 { g / l } else { 0.0 };
    }
    s.hold_p90_min = percentyl(&mut holds, 0.9);
    s
}

#[cfg(test)]
mod testy {
    use super::*;
    use conduit_core::types::CloseReason;

    fn trejd(side: Side, profit: f64, reason: CloseReason) -> ClosedTrade {
        ClosedTrade {
            profit_basis: None, cost_receipt: None,
            ticket: 1,
            side,
            volume: 0.01,
            open_price: 4000.0,
            close_price: 4000.0 + profit,
            open_ts: 0,
            close_ts: 600_000,
            profit,
            commission: 0.0,
            swap: -0.5,
            reason,
            basket: Some(1),
        }
    }

    fn odrzut(kod: &str) -> OdrzuconeWejscie {
        OdrzuconeWejscie {
            ts: 1_755_000_000_000,
            msg_id: 1,
            kod: kod.into(),
            side: Side::Buy,
            lo: 3995.0,
            hi: 4000.0,
            sl: Some(3990.0),
            tp1: Some(4010.0),
        }
    }

    fn koszyk(id: u32, pl: f64, n_trades: u32) -> BasketDump {
        BasketDump {
            id,
            msg_id: id as i64,
            side: "Buy".into(),
            zone_lo: 3995.0,
            zone_hi: 4000.0,
            sl: Some(3990.0),
            tps: vec![4010.0, 4020.0, 4030.0],
            created_ts: 1_755_000_000_000,
            tp_stage: 0,
            tp_touch_ts: vec![0, 0, 0],
            tp_touch_px: vec![0.0; 3],
            sl_touch_ts: 0,
            sl_touch_px: 0.0,
            warstwy: Vec::new(),
            seg: 0,
            pl,
            n_trades,
            first_open_ts: 0,
            last_close_ts: 0,
            realized: pl,
            reentries: 0,
            rearms: 0,
            last_rearm_ts: 0,
            had_positions: n_trades > 0,
            secured: false,
            peak_pl_usd: 0.0,
            entry_lo: 3995.0,
            entry_hi: 4000.0,
            state: "Done".into(),
        }
    }

    /// E2: trzy koszyki — wygrany, przegrany i dokładnie na zero — trafiają
    /// w trzy różne kubełki, a czwarty (bez wypełnienia) w ogóle nie liczy
    /// się do wyniku, tylko do `bez_fillu`.
    #[test]
    fn koszyki_trafiaja_we_wlasciwe_kubelki() {
        let k = vec![
            koszyk(1, 40.0, 3),
            koszyk(2, -25.0, 2),
            koszyk(3, 0.0, 1),
            koszyk(4, 0.0, 0),
        ];
        let s = policz_koszyki(&k, &[], 0.0, 0);
        assert_eq!((s.total, s.z_pozycjami, s.bez_fillu), (4, 3, 1));
        assert_eq!((s.win, s.loss, s.be), (1, 1, 1));
        assert_eq!(
            s.win + s.loss + s.be,
            s.z_pozycjami,
            "suma kubełków = koszyki z handlem"
        );
        assert!((s.suma_usd - 15.0).abs() < 1e-9);
        assert!((s.mediana_usd - 0.0).abs() < 1e-9);
    }

    /// Próg BE przesuwa granicę remisu — i tylko ją.
    #[test]
    fn prog_be_szerzej_lapie_remisy() {
        let k = vec![koszyk(1, 0.03, 1), koszyk(2, 40.0, 1)];
        let waski = policz_koszyki(&k, &[], 0.0, 0);
        assert_eq!((waski.win, waski.be), (2, 0));
        let szeroki = policz_koszyki(&k, &[], 0.05, 0);
        assert_eq!((szeroki.win, szeroki.be), (1, 1));
        assert!(
            (szeroki.suma_usd - waski.suma_usd).abs() < 1e-9,
            "suma $ się nie zmienia"
        );
    }

    /// E4: rozbicie BUY/SELL zgadza się z sumą, a remis nie jest wygraną.
    #[test]
    fn rozbicie_stron_sumuje_sie_do_calosci() {
        let t = vec![
            trejd(Side::Buy, 10.0, CloseReason::Tp),
            trejd(Side::Buy, -5.0, CloseReason::Sl),
            trejd(Side::Sell, 0.0, CloseReason::OutAtEntry),
        ];
        let s = policz_transakcje(&t, 0, 1.25);
        assert_eq!(s.buy.n + s.sell.n, 3);
        assert_eq!((s.buy.win, s.sell.win), (1, 0));
        assert!((s.buy.usd - 5.0).abs() < 1e-9);
        assert!((s.buy.profit_factor - 2.0).abs() < 1e-9);
        assert!(
            (s.swap_usd + 1.5).abs() < 1e-9,
            "swap sumuje się po wszystkich"
        );
        assert_eq!(s.spread_usd, 1.25);
        // jednostka R = mediana ryzyka z transakcji zamkniętych stopem = 5 $
        assert_eq!(s.r_hist.iter().sum::<u32>(), 3);
        assert_eq!(s.r_n, 3);
    }

    /// Bez ani jednej transakcji zamkniętej stopem nie ma z czego policzyć R —
    /// i wtedy histogram ma zostać PUSTY, a nie wypełniony zgadywaniem.
    #[test]
    fn brak_stopow_zostawia_histogram_r_pusty() {
        let t = vec![trejd(Side::Buy, 10.0, CloseReason::Tp)];
        let s = policz_transakcje(&t, 0, 0.0);
        assert_eq!(s.r_n, 0);
        assert_eq!(s.r_hist.iter().sum::<u32>(), 0);
        assert_eq!(s.r_hist.len(), R_KUBELKOW, "kubełki istnieją, tylko puste");
    }

    /// Granice kubełków R i ich nazwy nie mogą się rozjechać.
    #[test]
    fn kubelki_r_maja_tyle_nazw_ile_kubelkow() {
        assert_eq!(r_nazwy().len(), R_KUBELKOW);
        assert_eq!(kubelek_r(-9.0), 0);
        assert_eq!(kubelek_r(9.0), R_KUBELKOW - 1);
        assert_eq!(kubelek_r(-1.5), 1);
    }

    /// E3: mianownik lejka to sygnały PRZYJĘTE plus odrzucone, a `%wykonanych`
    /// liczy się od koszyków, które naprawdę zahandlowały.
    #[test]
    fn lejek_domyka_sie_na_mianowniku() {
        let mut m = Metrics::default();
        m.signals_taken = 8;
        m.odrzuty.insert("SessionClosed".into(), 2);
        let k = vec![koszyk(1, 10.0, 1), koszyk(2, 0.0, 0)];
        let odrzucone = vec![
            odrzut("SessionClosed"),
            odrzut("SessionClosed"),
            odrzut("RegimeFilter"),
        ];
        let l = policz_lejek(&m, &k, &odrzucone, None, 0);
        // 2 koszyki + 3 odrzucone WEJŚCIA. `Metrics::odrzuty` (tu: 2 sztuki
        // pod SessionClosed) NIE jest mianownikiem — liczy wszystkie akcje.
        assert_eq!(l.odrzucone_razem, 3);
        assert_eq!(l.sygnaly_widziane, 5);
        // bez licznika w przód (0 = stare archiwum) zgubionych nie zgadujemy
        assert_eq!(l.zgubione_bez_sladu, 0);
        assert_eq!(l.odrzucone.get("SessionClosed"), Some(&2));
        assert_eq!(
            (l.koszyki, l.koszyk_z_handlem, l.koszyk_bez_fillu),
            (2, 1, 1)
        );
        assert!((l.wykonanych_pct - 20.0).abs() < 1e-9);
        // bez ticków kubełki kodów istnieją, ale nic nie jest wycenione
        assert_eq!(l.koszt_filtrow.len(), 2);
        assert_eq!(l.koszt_filtrow["SessionClosed"].n, 2);
        assert_eq!(l.wycenionych, 0);
        assert_eq!(l.koszt_filtrow_usd, 0.0);
    }

    /// Sygnał BEZ TRASY nie dociera do żadnej bramki, więc nie ma go
    /// w rejestrze — a jest stratą lejka i musi się w nim znaleźć.
    #[test]
    fn brak_trasy_wchodzi_do_lejka_mimo_braku_rejestru() {
        let mut m = Metrics::default();
        m.odrzuty.insert("BrakFormatu:ZEN".into(), 4);
        m.odrzuty.insert("HamulecStop".into(), 9);
        let l = policz_lejek(&m, &[], &[], None, 0);
        assert_eq!(
            l.odrzucone_razem, 4,
            "hamulec zarządzania to nie odrzucony SYGNAŁ"
        );
        assert_eq!(l.sygnaly_widziane, 4);
    }

    /// Poz. 20: licznik w przód dokłada wiersz zgubionych na poziomie
    /// WIADOMOŚCI. Rejestr odrzutów jest per próba (edycje ponawiają), więc:
    /// koszyk wygrywa nad odrzutem tej samej wiadomości, dwie próby jednej
    /// wiadomości liczą się raz, a reszta mianownika to zgubione.
    #[test]
    fn lejek_w_przod_liczy_zgubione() {
        let m = Metrics::default();
        // koszyki z wiadomości 1 i 2 (helper: msg_id == id)
        let k = vec![koszyk(1, 10.0, 1), koszyk(2, 0.0, 0)];
        // wiadomość 1: odrzucona, ale PÓŹNIEJ weszła (koszyk) — nie liczy się
        // wiadomość 10: odrzucona DWA razy (dwie próby po edycji) — liczy się raz
        // wiadomość 11: odrzucona raz
        let mut o1 = odrzut("SessionClosed");
        o1.msg_id = 1;
        let mut o10a = odrzut("SessionClosed");
        o10a.msg_id = 10;
        let mut o10b = odrzut("RegimeFilter");
        o10b.msg_id = 10;
        let mut o11 = odrzut("RegimeFilter");
        o11.msg_id = 11;
        let odrzucone = vec![o1, o10a, o10b, o11];
        let l = policz_lejek(&m, &k, &odrzucone, None, 6);
        assert_eq!(l.sygnaly_wejsciowe, 6);
        assert_eq!(l.koszyki_sygnaly, 2);
        assert_eq!(
            l.odrzucone_sygnaly, 2,
            "msg 10 raz, msg 11 raz, msg 1 to koszyk"
        );
        assert_eq!(l.zgubione_bez_sladu, 2, "6 wejść − 2 koszyki − 2 odrzucone");
        // suma lejka domyka się na liczniku w przód
        assert_eq!(
            l.koszyki_sygnaly as i64 + l.odrzucone_sygnaly as i64 + l.zgubione_bez_sladu,
            l.sygnaly_wejsciowe as i64,
        );
        // % wykonanych liczy się od pełnego mianownika: 1 z 6
        assert!((l.wykonanych_pct - 100.0 / 6.0).abs() < 1e-9);
        // surowe liczniki prób zostają nietknięte (kubełki = Metrics::odrzuty)
        assert_eq!(l.odrzucone_razem, 4);
        assert_eq!(
            l.sygnaly_widziane, 6,
            "2 koszyki + 4 próby — mianownik wstecz"
        );
    }

    /// Powody śmierci nie mieszają dni: w trybie dziennym id koszyka powtarza
    /// się co dobę, więc klucz musi trzymać chwilę ostatniego zamknięcia
    /// segmentu. Na samym id oba koszyki dostałyby powód z późniejszego dnia.
    #[test]
    fn powody_nie_mieszaja_segmentow() {
        let mut k1 = koszyk(7, -10.0, 1);
        k1.seg = 0;
        k1.last_close_ts = 1_000;
        let mut k2 = koszyk(7, 25.0, 1);
        k2.seg = 1;
        k2.last_close_ts = 2_000;
        let mut t1 = trejd(Side::Buy, -10.0, CloseReason::Sl);
        t1.basket = Some(7);
        t1.close_ts = 1_000;
        let mut t2 = trejd(Side::Buy, 25.0, CloseReason::Tp);
        t2.basket = Some(7);
        t2.close_ts = 2_000;
        let s = policz_koszyki(&[k1, k2], &[t1, t2], 0.0, 0);
        assert_eq!(
            s.powody.get("Sl").map(|k| k.n),
            Some(1),
            "dzień 1 umarł na stopie"
        );
        assert_eq!(
            s.powody.get("Tp").map(|k| k.n),
            Some(1),
            "dzień 2 umarł na celu"
        );
    }

    /// Dotknięcie TP liczy się także dla koszyka BEZ wejścia — „TP bez
    /// wejścia" to sedno sporu o pokrycie i nie wolno go odsiewać razem
    /// z koszykami bez fillu.
    #[test]
    fn tp_touch_liczy_sie_bez_fillu() {
        let mut k = koszyk(1, 0.0, 0);
        k.tp_touch_ts = vec![k.created_ts + 60_000, 0, 0];
        let s = policz_koszyki(&[k], &[], 0.0, 0);
        assert_eq!((s.total, s.bez_fillu, s.tp1), (1, 1, 1));
        assert!(
            (s.med_do_tp1_min - 1.0).abs() < 1e-9,
            "minuta od sygnału do TP1"
        );
    }
}
