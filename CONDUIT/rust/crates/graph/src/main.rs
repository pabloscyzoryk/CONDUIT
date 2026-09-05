
use anyhow::{Context, Result};
use axum::extract::{Path as SciezkaUrl, Query, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Interfejs — pisany ręcznie, bez kroku budowania. Wbudowany w binarkę,
/// więc `graph.exe` jest jednym plikiem bez katalogu obok.
#[derive(RustEmbed)]
#[folder = "web/"]
struct Zasoby;

const WERSJA: &str = env!("CARGO_PKG_VERSION");

/// Ile plików `*_dane.json` wolno znaleźć w jednym drzewie. `rust/` ma ponad
/// trzysta katalogów `out_*`; bez sufitu wskazanie go jako źródła zamieniłoby
/// start programu w kilkuminutowe skanowanie.
const MAX_PRZEBIEGOW: usize = 400;

/// Jak głęboko schodzimy w podkatalogi przy szukaniu przebiegów.
const GLEBOKOSC: usize = 4;

/// Sufit liczby kubełków w jednej odpowiedzi. Minuty na dwóch miesiącach to
/// 85 000 słupków — nikt tego nie ogląda, a przeglądarka staje.
const MAX_KUBELKOW: usize = 20_000;

/// Ile przebiegów trzymamy wczytanych naraz (każdy to ~2-5 MB w pamięci).
const CACHE: usize = 24;

// ============================================================
//  ARGUMENTY
// ============================================================

struct Argumenty {
    dane: Vec<PathBuf>,
    port: u16,
    bez_przegladarki: bool,
}

const POMOC: &str = "\
graph.exe — CONDUIT GRAPH, analiza wykresów z backtestów

  --dane <katalog>     katalog z przebiegami (`btp --out <katalog>`);
                       można podać wiele razy. Bez tego program szuka
                       katalogu `WYKRESY` obok siebie i wyżej.
  --port <numer>       wymuś port; domyślnie system przydziela wolny,
                       więc nie ma kolizji z conduit.exe ani z drugą kopią
  --bez-przegladarki   nie otwieraj przeglądarki, tylko wypisz adres
  --pomoc, -h          ten opis

Program działa, dopóki otwarte jest okno konsoli (Ctrl+C kończy).

Żeby zejść na poziom MINUTY, przebieg musi być policzony z gęstszym
próbkowaniem krzywej:  btp … --krzywa-ms 60000 --dump-trades
";

fn czytaj_argumenty<I: Iterator<Item = String>>(it: I) -> Result<Option<Argumenty>> {
    let mut a = Argumenty {
        dane: Vec::new(),
        port: 0,
        bez_przegladarki: false,
    };
    let mut it = it.peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pomoc" | "-h" | "--help" | "/?" => {
                println!("{POMOC}");
                return Ok(None);
            }
            "--dane" | "--data" => {
                let v = it.next().context("--dane wymaga ścieżki do katalogu")?;
                a.dane.push(PathBuf::from(v));
            }
            "--port" => {
                let v = it.next().context("--port wymaga numeru")?;
                a.port = v.parse().context("--port musi być liczbą 0-65535")?;
            }
            "--bez-przegladarki" | "--no-browser" => a.bez_przegladarki = true,
            inne => anyhow::bail!("nieznany argument „{inne}” — użyj --pomoc"),
        }
    }
    Ok(Some(a))
}

// ============================================================
//  CZAS — wszystko w UTC, bo silnik liczy dobę w UTC
// ============================================================
//
// `Settings::session_offset()` zwraca 0, więc granica doby w backteście to
// północ UTC, a `dni[].date` to data UTC. Gdybyśmy tutaj użyli strefy
// przeglądarki, słupek „24 lipca" obejmowałby inny materiał niż wiersz
// „2026-07-24" z silnika — i nikt by nie zauważył, bo różnica jest o dwie
// godziny, a nie o rząd wielkości.

const DOBA: i64 = 86_400_000;
const GODZINA: i64 = 3_600_000;
const MINUTA: i64 = 60_000;

/// Dni od epoki → (rok, miesiąc, dzień). Algorytm Hinnanta, ten sam co
/// `fmt_day` w `crates/backtest/src/runner.rs`.
fn data_z_dni(day: i64) -> (i64, u32, u32) {
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
    (y, m as u32, d as u32)
}

/// (rok, miesiąc, dzień) → dni od epoki. Odwrotność `data_z_dni`.
fn dni_z_daty(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn iso(ts: i64) -> String {
    let (y, m, d) = data_z_dni(ts.div_euclid(DOBA));
    let r = ts.rem_euclid(DOBA);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        r / GODZINA,
        (r % GODZINA) / MINUTA,
        (r % MINUTA) / 1000
    )
}

fn data_iso(ts: i64) -> String {
    let (y, m, d) = data_z_dni(ts.div_euclid(DOBA));
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Poziom {
    Miesiac,
    Tydzien,
    Dzien,
    Godzina,
    Minuta,
}

impl Poziom {
    fn z_nazwy(s: &str) -> Option<Poziom> {
        Some(match s {
            "miesiac" | "miesiąc" => Poziom::Miesiac,
            "tydzien" | "tydzień" => Poziom::Tydzien,
            "dzien" | "dzień" => Poziom::Dzien,
            "godzina" => Poziom::Godzina,
            "minuta" => Poziom::Minuta,
            _ => return None,
        })
    }
    fn nazwa(self) -> &'static str {
        match self {
            Poziom::Miesiac => "miesiac",
            Poziom::Tydzien => "tydzien",
            Poziom::Dzien => "dzien",
            Poziom::Godzina => "godzina",
            Poziom::Minuta => "minuta",
        }
    }
    /// Początek kubełka zawierającego `ts`.
    fn granica(self, ts: i64) -> i64 {
        match self {
            Poziom::Miesiac => {
                let (y, m, _) = data_z_dni(ts.div_euclid(DOBA));
                dni_z_daty(y, m, 1) * DOBA
            }
            Poziom::Tydzien => {
                let d = ts.div_euclid(DOBA);
                (d - (d + 3).rem_euclid(7)) * DOBA
            }
            Poziom::Dzien => ts.div_euclid(DOBA) * DOBA,
            Poziom::Godzina => ts.div_euclid(GODZINA) * GODZINA,
            Poziom::Minuta => ts.div_euclid(MINUTA) * MINUTA,
        }
    }
    /// Początek NASTĘPNEGO kubełka po tym, który zaczyna się w `start`.
    fn nastepna(self, start: i64) -> i64 {
        match self {
            Poziom::Miesiac => {
                let (y, m, _) = data_z_dni(start.div_euclid(DOBA));
                let (y2, m2) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                dni_z_daty(y2, m2, 1) * DOBA
            }
            Poziom::Tydzien => start + 7 * DOBA,
            Poziom::Dzien => start + DOBA,
            Poziom::Godzina => start + GODZINA,
            Poziom::Minuta => start + MINUTA,
        }
    }
    fn etykieta(self, start: i64) -> String {
        let (y, m, d) = data_z_dni(start.div_euclid(DOBA));
        let r = start.rem_euclid(DOBA);
        match self {
            Poziom::Miesiac => format!("{y:04}-{m:02}"),
            Poziom::Tydzien => format!("{y:04}-{m:02}-{d:02}"),
            Poziom::Dzien => format!("{y:04}-{m:02}-{d:02}"),
            Poziom::Godzina => format!("{:02}:00", r / GODZINA),
            Poziom::Minuta => format!("{:02}:{:02}", r / GODZINA, (r % GODZINA) / MINUTA),
        }
    }
    /// Poziom o jeden stopień dokładniejszy — dokąd prowadzi kliknięcie.
    fn glebiej(self) -> Option<Poziom> {
        Some(match self {
            Poziom::Miesiac => Poziom::Tydzien,
            Poziom::Tydzien => Poziom::Dzien,
            Poziom::Dzien => Poziom::Godzina,
            Poziom::Godzina => Poziom::Minuta,
            Poziom::Minuta => return None,
        })
    }
}


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Naglowek {
    preset: String,
    od: String,
    #[serde(rename = "do")]
    do_kiedy: String,
    tryb: String,
    saldo_start: f64,
    /// Co ile milisekund silnik próbkował krzywą (`btp --krzywa-ms`).
    /// Starsze pliki tego nie mają — wtedy wyliczamy medianę odstępów.
    krok_ms: i64,
    sygnaly: String,
    format: String,
    metryki: serde_json::Value,
}

/// Krótka nazwa źródła sygnałów do etykiety. `signals.json` to historyczny
/// zbiór ATFX i tak go podpisujemy; reszta idzie po przyrostku pliku.
fn nazwa_zrodla(n: &Naglowek) -> String {
    if !n.format.is_empty() && n.format != "?" {
        return n.format.clone();
    }
    let s = n.sygnaly.trim_end_matches(".json");
    match s {
        "" => "?".into(),
        "signals" => "ATFX".into(),
        inne => inne.trim_start_matches("signals_").to_uppercase(),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Dzien {
    day: i64,
    date: String,
    start_equity: f64,
    end_equity: f64,
    profit: f64,
    max_dd: f64,
    trades: u32,
    signals: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Transakcja {
    ticket: u64,
    side: String,
    volume: f64,
    open_price: f64,
    close_price: f64,
    open_ts: i64,
    close_ts: i64,
    profit: f64,
    commission: f64,
    swap: f64,
    reason: String,
    basket: Option<u32>,
}

/// Pełny plik `*_dane.json`. Osobna struktura od `Naglowek`, bo listy
/// przebiegów nie wolno budować przez wczytanie wszystkich krzywych —
/// szesnaście plików po półtora megabajta to sekundy czekania na starcie.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct PlikDanych {
    preset: String,
    od: String,
    #[serde(rename = "do")]
    do_kiedy: String,
    tryb: String,
    saldo_start: f64,
    krok_ms: i64,
    sygnaly: String,
    format: String,
    metryki: serde_json::Value,
    krzywa: Vec<(i64, f64)>,
    saldo: Vec<(i64, f64)>,
    dni: Vec<Dzien>,
}

// ============================================================
//  PRZEBIEG W PAMIĘCI
// ============================================================

struct Przebieg {
    naglowek: Naglowek,
    /// TRYB DZIENNY (`btp --daily-reset`) — każda doba liczona osobno od kwoty
    /// startowej. To nie jest kosmetyka: o północy silnik zamyka wszystko
    /// i USTAWIA `balance = start_balance`, więc krzywa spada z 1 700 $ na
    /// 200 $ bez żadnej straty. Każda różnica equity policzona PRZEZ granicę
    /// doby jest w tym trybie fikcją i musi być liczona inaczej.
    dzienny: bool,
    /// znaczniki czasu (rosnąco) — wspólne dla `eq` i `sal`
    t: Vec<i64>,
    eq: Vec<f64>,
    sal: Vec<f64>,
    /// Szczyt kroczący equity, policzony RAZ przy wczytaniu. W trybie dziennym
    /// zerowany na granicy doby — inaczej „obsunięcie od szczytu" pokazywałoby
    /// −94 % dla przebiegu, który nie stracił ani centa (spadek 1 700 → 200 to
    /// przelew na start następnego dnia, nie strata).
    szczyt: Vec<f64>,
    dni: Vec<Dzien>,
    /// indeks `dni` po numerze doby — poziom dnia zestawia liczby z silnika
    /// z liczbami z krzywej
    dni_wg_doby: HashMap<i64, usize>,
    /// transakcje posortowane po `close_ts`
    transakcje: Vec<Transakcja>,
    /// te same transakcje, kolejność po `open_ts` (indeksy do `transakcje`)
    wg_otwarcia: Vec<u32>,
}

impl Przebieg {
    fn zakres(&self) -> (i64, i64) {
        if self.t.is_empty() {
            (0, 0)
        } else {
            (self.t[0], self.t[self.t.len() - 1])
        }
    }
}

/// Mediana odstępów między próbkami. Odpowiada na pytanie „czy ten przebieg
/// w ogóle da się oglądać co minutę", bez zaufania do pola w pliku (starsze
/// pliki go nie mają, a nowsze mogły powstać przy innym `--krzywa-ms`).
fn krok_z_danych(t: &[i64]) -> i64 {
    if t.len() < 3 {
        return 0;
    }
    let n = t.len().min(4001);
    let mut d: Vec<i64> = (1..n).map(|i| t[i] - t[i - 1]).collect();
    d.sort_unstable();
    d[d.len() / 2]
}

fn wczytaj(plik: &Path) -> Result<Przebieg> {
    let raw = std::fs::read_to_string(plik)
        .with_context(|| format!("nie mogę odczytać {}", plik.display()))?;
    let d: PlikDanych = serde_json::from_str(&raw)
        .with_context(|| format!("uszkodzony plik {}", plik.display()))?;

    let t: Vec<i64> = d.krzywa.iter().map(|(t, _)| *t).collect();
    let eq: Vec<f64> = d.krzywa.iter().map(|(_, v)| *v).collect();

    // Saldo bierzemy TYLKO wtedy, gdy jest próbkowane w tych samych chwilach
    // co equity. Inaczej kursor pokazywałby equity z jednej chwili, a saldo
    // z sąsiedniej — i „wynik pływający" (różnica) byłby wymyślony.
    let sal: Vec<f64> = if d.saldo.len() == d.krzywa.len()
        && d.saldo.iter().zip(d.krzywa.iter()).all(|(a, b)| a.0 == b.0)
    {
        d.saldo.iter().map(|(_, v)| *v).collect()
    } else {
        eq.clone()
    };

    let dzienny = d.tryb == "daily";
    let mut szczyt = Vec::with_capacity(eq.len());
    let mut s = f64::NEG_INFINITY;
    let mut doba = i64::MIN;
    for (i, v) in eq.iter().enumerate() {
        if dzienny {
            let d2 = t[i].div_euclid(DOBA);
            if d2 != doba {
                doba = d2;
                s = f64::NEG_INFINITY;
            }
        }
        if *v > s {
            s = *v;
        }
        szczyt.push(s);
    }

    let mut dni_wg_doby = HashMap::new();
    for (i, dz) in d.dni.iter().enumerate() {
        dni_wg_doby.insert(dz.day, i);
    }

    // `transakcje.json` leży obok i powstaje tylko przy `--dump-trades`
    // dla przebiegu JEDNEGO presetu (`bt.rs` pilnuje `results.len() == 1`),
    // więc nie ma ryzyka, że dokleimy transakcje innej konfiguracji.
    let mut transakcje: Vec<Transakcja> = plik
        .parent()
        .map(|k| k.join("transakcje.json"))
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    transakcje.sort_by_key(|x| x.close_ts);
    let mut wg_otwarcia: Vec<u32> = (0..transakcje.len() as u32).collect();
    wg_otwarcia.sort_by_key(|i| transakcje[*i as usize].open_ts);

    let krok = if d.krok_ms > 0 {
        d.krok_ms
    } else {
        krok_z_danych(&t)
    };

    Ok(Przebieg {
        naglowek: Naglowek {
            preset: d.preset,
            od: d.od,
            do_kiedy: d.do_kiedy,
            tryb: d.tryb,
            saldo_start: d.saldo_start,
            krok_ms: krok,
            sygnaly: d.sygnaly,
            format: d.format,
            metryki: d.metryki,
        },
        dzienny,
        t,
        eq,
        sal,
        szczyt,
        dni: d.dni,
        dni_wg_doby,
        transakcje,
        wg_otwarcia,
    })
}

// ============================================================
//  SZUKANIE PRZEBIEGÓW
// ============================================================

#[derive(Clone, Serialize)]
struct Zrodlo {
    id: String,
    /// nazwa pokazywana na liście — katalog + preset + tryb
    etykieta: String,
    katalog: String,
    plik: String,
    bajty: u64,
    #[serde(skip)]
    sciezka: PathBuf,
}

fn skanuj(korzen: &Path, znalezione: &mut Vec<PathBuf>, glebokosc: usize) {
    if znalezione.len() >= MAX_PRZEBIEGOW || glebokosc == 0 {
        return;
    }
    let rd = match std::fs::read_dir(korzen) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut pliki = Vec::new();
    let mut katalogi = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            katalogi.push(p);
        } else if p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with("_dane.json"))
            .unwrap_or(false)
        {
            pliki.push(p);
        }
    }
    pliki.sort();
    katalogi.sort();
    for p in pliki {
        if znalezione.len() >= MAX_PRZEBIEGOW {
            return;
        }
        znalezione.push(p);
    }
    for k in katalogi {
        skanuj(&k, znalezione, glebokosc - 1);
    }
}

/// Identyfikator w adresie URL. Powstaje z NAZWY, ale ścieżki nigdy z niego
/// nie odtwarzamy — plik znajdujemy przez wyszukanie w rejestrze. To jest
/// cała ochrona przed wyjściem poza katalog: adres nie trafia do `Path::join`.
fn zrob_id(sciezka: &Path, korzen: Option<&Path>) -> String {
    let wzgl = korzen
        .and_then(|k| sciezka.strip_prefix(k).ok())
        .unwrap_or(sciezka)
        .to_string_lossy()
        .to_string();
    let mut s: String = wzgl
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.len() > 120 {
        s = s[s.len() - 120..].to_string();
    }
    s
}

/// Ładna nazwa przebiegu: `KATALOG · PRESET · tryb`. Sam preset nie wystarcza,
/// bo tych samych `HYPER-2` jest w projekcie kilkanaście — różnią się saldem
/// startowym i oknem, a to widać po katalogu.
fn zrob_etykiete(sciezka: &Path) -> String {
    let katalog = sciezka
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let plik = sciezka
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let rdzen = plik.trim_end_matches("_dane.json").to_string();
    if katalog.is_empty() {
        rdzen
    } else {
        format!("{katalog} · {rdzen}")
    }
}

// ============================================================
//  PRZERZEDZANIE
// ============================================================

/// Wybiera z zakresu `[a, b)` co najwyżej ~`cel` indeksów, ZACHOWUJĄC
/// skrajności: w każdym kubełku zostaje pierwszy, minimum, maksimum i ostatni.
///
/// Zwykłe „co n-ty punkt" gubi całe obsunięcia — czyli dokładnie to, czego
/// się szuka na wykresie kapitału. Dlatego kubełków jest `cel / 3`: cztery
/// punkty na kubełek po odrzuceniu powtórzeń dają w praktyce około `cel`.
fn przerzedz(eq: &[f64], a: usize, b: usize, cel: usize) -> Vec<usize> {
    let n = b.saturating_sub(a);
    if n == 0 {
        return Vec::new();
    }
    if n <= cel || cel < 8 {
        return (a..b).collect();
    }
    let kubelkow = (cel / 3).max(1);
    let mut out: Vec<usize> = Vec::with_capacity(cel + 8);
    for k in 0..kubelkow {
        let ka = a + n * k / kubelkow;
        let kb = a + n * (k + 1) / kubelkow;
        if kb <= ka {
            continue;
        }
        let mut lo = ka;
        let mut hi = ka;
        for i in ka..kb {
            if eq[i] < eq[lo] {
                lo = i;
            }
            if eq[i] > eq[hi] {
                hi = i;
            }
        }
        let mut czworka = [ka, lo, hi, kb - 1];
        czworka.sort_unstable();
        for i in czworka {
            if out.last() != Some(&i) {
                out.push(i);
            }
        }
    }
    out
}

/// Pierwszy indeks, dla którego `t[i] >= v`.
fn dolna_granica(t: &[i64], v: i64) -> usize {
    t.partition_point(|x| *x < v)
}

// ============================================================
//  KUBEŁKI
// ============================================================

#[derive(Debug, Clone, Default, Serialize)]
struct Kubelek {
    t: i64,
    #[serde(rename = "do")]
    do_kiedy: i64,
    etykieta: String,
    /// equity na wejściu w kubełek (ostatnia próbka PRZED nim, a na granicy
    /// doby — `start_equity` z silnika)
    otwarcie: f64,
    zamkniecie: f64,
    /// WYNIK KUBEŁKA — liczba, którą pokazujemy.
    /// Kubełki obejmujące całe doby (dzień, tydzień, miesiąc) biorą ją
    /// z SUMY dni policzonych przez silnik; krótsze — z różnicy equity.
    profit: f64,
    /// Ta sama wielkość policzona z samej krzywej. Trzymamy obie, żeby dało
    /// się zobaczyć rozjazd, zamiast wybierać ładniejszą liczbę.
    profit_krzywa: f64,
    /// `silnik` albo `krzywa` — skąd wzięło się `profit`
    zrodlo_wyniku: &'static str,
    min: f64,
    max: f64,
    saldo_otw: f64,
    saldo_zam: f64,
    /// najgłębsze zanurzenie od lokalnego szczytu WEWNĄTRZ kubełka, w $
    obsuniecie: f64,
    /// najgorsze odchylenie od szczytu CAŁEGO przebiegu, w %
    dd_pct: f64,
    /// ile próbek krzywej wpadło w kubełek; 0 = brak danych, nie „zero zysku"
    punktow: u32,
    /// transakcje ZAMKNIĘTE w kubełku
    transakcje: u32,
    wygrane: u32,
    zrealizowany: f64,
    wolumen: f64,
    /// pozycje OTWARTE w kubełku
    otwarc: u32,
    // --- tylko poziom dnia: liczby prosto z silnika, do zestawienia ---
    #[serde(skip_serializing_if = "Option::is_none")]
    silnik_profit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    silnik_dd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    silnik_trades: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sygnaly: Option<u32>,
}

/// Zakres indeksów transakcji o `close_ts` w `[od, do)`.
fn zakres_zamkniec(tr: &[Transakcja], od: i64, az: i64) -> (usize, usize) {
    let a = tr.partition_point(|x| x.close_ts < od);
    let b = tr.partition_point(|x| x.close_ts < az);
    (a, b)
}

/// Czy kubełki tego poziomu obejmują CAŁE doby. Jeśli tak, wynik bierzemy
/// z `dni[]` policzonych przez silnik — bo tylko one są poprawne w trybie
/// dziennym, gdzie o północy saldo wraca do kwoty startowej.
fn cale_doby(poziom: Poziom) -> bool {
    matches!(poziom, Poziom::Dzien | Poziom::Tydzien | Poziom::Miesiac)
}

fn kubelki(p: &Przebieg, poziom: Poziom, od: i64, az: i64) -> (Vec<Kubelek>, bool) {
    let mut out = Vec::new();
    if p.t.is_empty() || az <= od {
        return (out, false);
    }
    let start = poziom.granica(od);
    let mut i = dolna_granica(&p.t, start);
    let mut poprz_eq = if i > 0 {
        p.eq[i - 1]
    } else {
        p.naglowek.saldo_start
    };
    let mut poprz_sal = if i > 0 {
        p.sal[i - 1]
    } else {
        p.naglowek.saldo_start
    };
    // Wejście w kubełek zaczynający dobę bierzemy z silnika, nie z ostatniej
    // próbki poprzedniej doby. W trybie dziennym te dwie liczby dzieli cały
    // wynik dnia (1 700 $ wobec 200 $), a w trybie łączonym różnią się
    // o ruch ceny między ostatnią próbką a pierwszym kursem nowej doby.
    let start_doby = |ts: i64, zapas_eq: f64| -> Option<f64> {
        if ts.rem_euclid(DOBA) != 0 {
            return None;
        }
        p.dni_wg_doby
            .get(&ts.div_euclid(DOBA))
            .map(|k| p.dni[*k].start_equity)
            .or({
                if p.dzienny {
                    Some(p.naglowek.saldo_start)
                } else {
                    Some(zapas_eq)
                }
            })
    };

    // otwarcia pozycji — osobna oś czasu, więc osobny wskaźnik
    let mut oi = p
        .wg_otwarcia
        .partition_point(|k| p.transakcje[*k as usize].open_ts < start);

    let mut b0 = start;
    let mut obciete = false;
    while b0 < az {
        if out.len() >= MAX_KUBELKOW {
            obciete = true;
            break;
        }
        let b1 = poziom.nastepna(b0);
        if let Some(v) = start_doby(b0, poprz_eq) {
            poprz_eq = v;
            if p.dzienny {
                poprz_sal = v;
            }
        }
        let mut k = Kubelek {
            t: b0,
            do_kiedy: b1,
            etykieta: poziom.etykieta(b0),
            otwarcie: poprz_eq,
            zamkniecie: poprz_eq,
            min: poprz_eq,
            max: poprz_eq,
            saldo_otw: poprz_sal,
            saldo_zam: poprz_sal,
            zrodlo_wyniku: "krzywa",
            ..Default::default()
        };
        let mut szczyt_lok = poprz_eq;
        let mut doba_lok = b0.div_euclid(DOBA);
        while i < p.t.len() && p.t[i] < b1 {
            let v = p.eq[i];
            // W trybie dziennym przekroczenie północy WEWNĄTRZ kubełka
            // (tydzień, miesiąc) resetuje odniesienie: spadek z 1 700 na 200
            // to przelew, nie obsunięcie.
            if p.dzienny {
                let d2 = p.t[i].div_euclid(DOBA);
                if d2 != doba_lok {
                    doba_lok = d2;
                    szczyt_lok = v;
                }
            }
            if v < k.min {
                k.min = v;
            }
            if v > k.max {
                k.max = v;
            }
            if v > szczyt_lok {
                szczyt_lok = v;
            }
            let zanurzenie = szczyt_lok - v;
            if zanurzenie > k.obsuniecie {
                k.obsuniecie = zanurzenie;
            }
            let s = p.szczyt[i];
            if s > 0.0 {
                let d = (v - s) / s * 100.0;
                if d < k.dd_pct {
                    k.dd_pct = d;
                }
            }
            k.punktow += 1;
            i += 1;
        }
        if k.punktow > 0 {
            k.zamkniecie = p.eq[i - 1];
            k.saldo_zam = p.sal[i - 1];
        }
        k.profit_krzywa = k.zamkniecie - k.otwarcie;
        k.profit = k.profit_krzywa;

        let (ta, tb) = zakres_zamkniec(&p.transakcje, b0, b1);
        for tr in &p.transakcje[ta..tb] {
            k.transakcje += 1;
            if tr.profit > 0.0 {
                k.wygrane += 1;
            }
            k.zrealizowany += tr.profit;
            k.wolumen += tr.volume;
        }
        while oi < p.wg_otwarcia.len() {
            let tr = &p.transakcje[p.wg_otwarcia[oi] as usize];
            if tr.open_ts >= b1 {
                break;
            }
            if tr.open_ts >= b0 {
                k.otwarc += 1;
            }
            oi += 1;
        }

        // Kubełek obejmujący całe doby: wynik z SUMY dni silnika. Jedyna
        // liczba poprawna w obu trybach — w dziennym różnica equity przez
        // północ jest fikcją, w łączonym obie drogi dają to samo co do centa
        // (sprawdzone: 318 083,90 z krzywej i z silnika na tym samym oknie).
        if cale_doby(poziom) {
            let mut suma = 0.0;
            let mut dd = 0.0f64;
            let mut trades = 0u32;
            let mut sygnaly = 0u32;
            let mut znalezione = false;
            for d in b0.div_euclid(DOBA)..b1.div_euclid(DOBA) {
                if let Some(idx) = p.dni_wg_doby.get(&d) {
                    let dz = &p.dni[*idx];
                    suma += dz.profit;
                    dd = dd.max(dz.max_dd);
                    trades += dz.trades;
                    sygnaly += dz.signals;
                    znalezione = true;
                }
            }
            if znalezione {
                k.profit = suma;
                k.zrodlo_wyniku = "silnik";
                k.silnik_profit = Some(suma);
                k.silnik_dd = Some(dd);
                k.silnik_trades = Some(trades);
                k.sygnaly = Some(sygnaly);
                // w trybie dziennym „zamknięcie" ma być wynikiem doby liczonym
                // od kwoty startowej, a nie stanem konta z ostatniej próbki
                if p.dzienny && poziom != Poziom::Dzien {
                    k.otwarcie = p.naglowek.saldo_start;
                    k.zamkniecie = p.naglowek.saldo_start + suma;
                }
            }
        }

        poprz_eq = k.zamkniecie;
        poprz_sal = k.saldo_zam;
        out.push(k);
        b0 = b1;
    }
    (out, obciete)
}

// ============================================================
//  STAN SERWERA
// ============================================================

struct Stan {
    rejestr: Mutex<Vec<Zrodlo>>,
    cache: Mutex<Vec<(String, Arc<Przebieg>)>>,
    korzenie: Vec<PathBuf>,
    sprawdzone: Vec<PathBuf>,
}

type Uchwyt = Arc<Stan>;

impl Stan {
    fn dodaj_korzen(&self, korzen: &Path) -> usize {
        let mut znalezione = Vec::new();
        if korzen.is_file() {
            znalezione.push(korzen.to_path_buf());
        } else {
            skanuj(korzen, &mut znalezione, GLEBOKOSC);
        }
        let mut rej = self.rejestr.lock().unwrap();
        let mut dodano = 0;
        for p in znalezione {
            if rej.iter().any(|z| z.sciezka == p) {
                continue;
            }
            let mut id = zrob_id(&p, Some(korzen));
            if rej.iter().any(|z| z.id == id) {
                id = format!("{id}_{}", rej.len());
            }
            rej.push(Zrodlo {
                id,
                etykieta: zrob_etykiete(&p),
                katalog: p
                    .parent()
                    .map(|x| x.display().to_string())
                    .unwrap_or_default(),
                bajty: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                plik: p.display().to_string(),
                sciezka: p,
            });
            dodano += 1;
        }
        dodano
    }

    fn sciezka(&self, id: &str) -> Option<PathBuf> {
        self.rejestr
            .lock()
            .unwrap()
            .iter()
            .find(|z| z.id == id)
            .map(|z| z.sciezka.clone())
    }

    /// Wczytany przebieg z pamięci podręcznej — plik czytamy i parsujemy RAZ.
    fn przebieg(&self, id: &str) -> Result<Arc<Przebieg>> {
        if let Some((_, p)) = self.cache.lock().unwrap().iter().find(|(k, _)| k == id) {
            return Ok(p.clone());
        }
        let sciezka = self.sciezka(id).context("nie ma takiego przebiegu")?;
        let p = Arc::new(wczytaj(&sciezka)?);
        let mut c = self.cache.lock().unwrap();
        c.push((id.to_string(), p.clone()));
        if c.len() > CACHE {
            c.remove(0);
        }
        Ok(p)
    }
}

fn blad(kod: StatusCode, tresc: impl Into<String>) -> Response {
    (kod, Json(serde_json::json!({ "error": tresc.into() }))).into_response()
}

// ============================================================
//  TRASY
// ============================================================

async fn zdrowie() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "app": "conduit", "tool": "graph", "version": WERSJA }))
}

async fn info(State(st): State<Uchwyt>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "wersja": WERSJA,
        "korzenie": st.korzenie.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "szukano": st.sprawdzone.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "liczba": st.rejestr.lock().unwrap().len(),
        "limit": MAX_PRZEBIEGOW,
    }))
}

/// Lista przebiegów. Czytamy z każdego pliku TYLKO nagłówek — `serde` mija
/// tablice `krzywa`/`saldo`/`dni` bez alokacji, więc szesnaście plików po
/// półtora megabajta kosztuje ułamek sekundy, a nie kilkanaście.
async fn lista(State(st): State<Uchwyt>) -> Json<Vec<serde_json::Value>> {
    let zrodla = st.rejestr.lock().unwrap().clone();
    let mut out = Vec::with_capacity(zrodla.len());
    for z in zrodla {
        let n: Naglowek = std::fs::read_to_string(&z.sciezka)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        out.push(serde_json::json!({
            "id": z.id,
            "etykieta": z.etykieta,
            "katalog": z.katalog,
            "plik": z.plik,
            "bajty": z.bajty,
            "preset": n.preset,
            "od": n.od,
            "do": n.do_kiedy,
            "tryb": n.tryb,
            "saldo_start": n.saldo_start,
            "zrodlo": nazwa_zrodla(&n),
            "sygnaly": n.sygnaly,
            "metryki": n.metryki,
            "ma_transakcje": z.sciezka.parent().map(|k| k.join("transakcje.json").is_file()).unwrap_or(false),
        }));
    }
    Json(out)
}

#[derive(Deserialize)]
struct Skan {
    sciezka: String,
}

/// Dodanie katalogu z poziomu interfejsu — użytkownik wybiera przebiegi
/// w aplikacji, nie w wierszu poleceń.
async fn dodaj(State(st): State<Uchwyt>, Query(q): Query<Skan>) -> Response {
    let p = PathBuf::from(&q.sciezka);
    if !p.exists() {
        return blad(
            StatusCode::NOT_FOUND,
            format!("nie ma ścieżki „{}”", q.sciezka),
        );
    }
    let n = st.dodaj_korzen(&p);
    Json(serde_json::json!({ "dodano": n, "razem": st.rejestr.lock().unwrap().len() }))
        .into_response()
}

/// Metadane jednego przebiegu: dni z silnika, metryki, zakres czasu.
/// Krzywa NIE wchodzi — po nią idzie się do `/seria`, dla widocznego okna.
async fn przebieg(State(st): State<Uchwyt>, SciezkaUrl(id): SciezkaUrl<String>) -> Response {
    let p = match st.przebieg(&id) {
        Ok(p) => p,
        Err(e) => return blad(StatusCode::NOT_FOUND, e.to_string()),
    };
    let (t0, t1) = p.zakres();
    Json(serde_json::json!({
        "id": id,
        "preset": p.naglowek.preset,
        "od": p.naglowek.od,
        "do": p.naglowek.do_kiedy,
        "tryb": p.naglowek.tryb,
        "saldo_start": p.naglowek.saldo_start,
        "krok_ms": p.naglowek.krok_ms,
        "zrodlo": nazwa_zrodla(&p.naglowek),
        "sygnaly": p.naglowek.sygnaly,
        "metryki": p.naglowek.metryki,
        "t0": t0,
        "t1": t1,
        "punktow": p.t.len(),
        "dni": p.dni,
        "transakcji": p.transakcje.len(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct Okno {
    od: Option<i64>,
    #[serde(rename = "do")]
    do_kiedy: Option<i64>,
    cel: Option<usize>,
}

/// Przerzedzona seria dla WIDOCZNEGO okna. Tablice równoległe zamiast
/// tablicy par — o jedną trzecią mniej znaków w odpowiedzi.
async fn seria(
    State(st): State<Uchwyt>,
    SciezkaUrl(id): SciezkaUrl<String>,
    Query(q): Query<Okno>,
) -> Response {
    let p = match st.przebieg(&id) {
        Ok(p) => p,
        Err(e) => return blad(StatusCode::NOT_FOUND, e.to_string()),
    };
    let (t0, t1) = p.zakres();
    let od = q.od.unwrap_or(t0);
    let az = q.do_kiedy.unwrap_or(t1);
    let cel = q.cel.unwrap_or(4000).clamp(8, 60_000);

    // Po jednej próbce z każdej strony okna, żeby linia dochodziła do krawędzi
    // wykresu, zamiast urywać się kilkanaście sekund przed nią.
    let a = dolna_granica(&p.t, od).saturating_sub(1);
    let b = (dolna_granica(&p.t, az) + 1).min(p.t.len());
    let idx = przerzedz(&p.eq, a, b, cel);

    let mut tt = Vec::with_capacity(idx.len());
    let mut ee = Vec::with_capacity(idx.len());
    let mut ss = Vec::with_capacity(idx.len());
    let mut dd = Vec::with_capacity(idx.len());
    for i in idx {
        tt.push(p.t[i]);
        ee.push(p.eq[i]);
        ss.push(p.sal[i]);
        let s = p.szczyt[i];
        dd.push(if s > 0.0 {
            (p.eq[i] - s) / s * 100.0
        } else {
            0.0
        });
    }
    Json(serde_json::json!({
        "t": tt, "eq": ee, "sal": ss, "dd": dd,
        "pelnych": b.saturating_sub(a),
        "zwroconych": tt.len(),
        "krok_ms": p.naglowek.krok_ms,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct OknoPoziom {
    poziom: String,
    od: Option<i64>,
    #[serde(rename = "do")]
    do_kiedy: Option<i64>,
}

async fn slupki(
    State(st): State<Uchwyt>,
    SciezkaUrl(id): SciezkaUrl<String>,
    Query(q): Query<OknoPoziom>,
) -> Response {
    let p = match st.przebieg(&id) {
        Ok(p) => p,
        Err(e) => return blad(StatusCode::NOT_FOUND, e.to_string()),
    };
    let poziom = match Poziom::z_nazwy(&q.poziom) {
        Some(x) => x,
        None => {
            return blad(
                StatusCode::BAD_REQUEST,
                format!("nieznany poziom „{}”", q.poziom),
            )
        }
    };
    let (t0, t1) = p.zakres();
    let od = q.od.unwrap_or(t0);
    let az = q.do_kiedy.unwrap_or(t1 + 1);
    let (k, obciete) = kubelki(&p, poziom, od, az);
    let suma: f64 = k.iter().map(|x| x.profit).sum();
    let silnik: f64 = k.iter().filter_map(|x| x.silnik_profit).sum();
    Json(serde_json::json!({
        "poziom": poziom.nazwa(),
        "glebiej": poziom.glebiej().map(|x| x.nazwa()),
        "od": od, "do": az,
        "kubelki": k,
        "suma": suma,
        "suma_silnika": silnik,
        "obciete": obciete,
        "krok_ms": p.naglowek.krok_ms,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct OknoTrans {
    od: Option<i64>,
    #[serde(rename = "do")]
    do_kiedy: Option<i64>,
    limit: Option<usize>,
}

/// Transakcje przecinające okno — wchodzi każda, która była w tym czasie
/// OTWARTA albo się w nim domknęła. Filtrowanie po samym `close_ts` gubiłoby
/// pozycje trzymane przez badaną minutę, czyli te najciekawsze.
async fn transakcje(
    State(st): State<Uchwyt>,
    SciezkaUrl(id): SciezkaUrl<String>,
    Query(q): Query<OknoTrans>,
) -> Response {
    let p = match st.przebieg(&id) {
        Ok(p) => p,
        Err(e) => return blad(StatusCode::NOT_FOUND, e.to_string()),
    };
    let (t0, t1) = p.zakres();
    let od = q.od.unwrap_or(t0);
    let az = q.do_kiedy.unwrap_or(t1 + 1);
    let limit = q.limit.unwrap_or(4000).min(50_000);
    let wybrane: Vec<&Transakcja> = p
        .transakcje
        .iter()
        .filter(|t| t.close_ts >= od && t.open_ts < az)
        .take(limit)
        .collect();
    Json(serde_json::json!({
        "transakcje": wybrane,
        "wszystkich": p.transakcje.len(),
        "obciete": wybrane.len() >= limit,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct Eksport {
    co: Option<String>,
    poziom: Option<String>,
    od: Option<i64>,
    #[serde(rename = "do")]
    do_kiedy: Option<i64>,
    /// `pl` (średnik + przecinek dziesiętny, dla Excela) albo `iso`
    format: Option<String>,
}

/// Zamiana liczby na tekst w wybranej konwencji. Bez tego polski Excel
/// wsadza „318083.9" do jednej komórki jako TEKST i wykres z tego nie wyjdzie.
fn licz(v: f64, przecinek: bool) -> String {
    let s = format!("{v:.4}");
    if przecinek {
        s.replace('.', ",")
    } else {
        s
    }
}

async fn csv(
    State(st): State<Uchwyt>,
    SciezkaUrl(id): SciezkaUrl<String>,
    Query(q): Query<Eksport>,
) -> Response {
    let p = match st.przebieg(&id) {
        Ok(p) => p,
        Err(e) => return blad(StatusCode::NOT_FOUND, e.to_string()),
    };
    let (t0, t1) = p.zakres();
    let od = q.od.unwrap_or(t0);
    let az = q.do_kiedy.unwrap_or(t1 + 1);
    let pl = q.format.as_deref().unwrap_or("pl") == "pl";
    let sep = if pl { ';' } else { ',' };
    let co = q.co.as_deref().unwrap_or("krzywa");

    let mut s = String::new();
    if pl {
        // Excel czyta tę linię i sam ustawia separator kolumn.
        s.push_str("sep=;\n");
    }
    match co {
        "krzywa" => {
            s.push_str(&format!(
                "czas{sep}epoka_ms{sep}equity{sep}saldo{sep}plywajace{sep}szczyt{sep}od_szczytu_pct\n"
            ));
            let a = dolna_granica(&p.t, od);
            let b = dolna_granica(&p.t, az);
            for i in a..b {
                let sz = p.szczyt[i];
                let d = if sz > 0.0 {
                    (p.eq[i] - sz) / sz * 100.0
                } else {
                    0.0
                };
                s.push_str(&format!(
                    "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}\n",
                    iso(p.t[i]),
                    p.t[i],
                    licz(p.eq[i], pl),
                    licz(p.sal[i], pl),
                    licz(p.eq[i] - p.sal[i], pl),
                    licz(sz, pl),
                    licz(d, pl)
                ));
            }
        }
        "slupki" => {
            let poziom = match Poziom::z_nazwy(q.poziom.as_deref().unwrap_or("dzien")) {
                Some(x) => x,
                None => return blad(StatusCode::BAD_REQUEST, "nieznany poziom"),
            };
            let (k, _) = kubelki(&p, poziom, od, az);
            s.push_str(&format!(
                "etykieta{sep}od{sep}do{sep}otwarcie{sep}zamkniecie{sep}wynik{sep}min{sep}max{sep}\
                 obsuniecie{sep}od_szczytu_pct{sep}saldo_otw{sep}saldo_zam{sep}probek{sep}\
                 transakcji{sep}wygranych{sep}zrealizowany{sep}wolumen{sep}otwarc{sep}\
                 silnik_wynik{sep}sygnalow\n"
            ));
            for x in k {
                s.push_str(&format!(
                    "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}\
                     {}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}\n",
                    x.etykieta,
                    iso(x.t),
                    iso(x.do_kiedy),
                    licz(x.otwarcie, pl),
                    licz(x.zamkniecie, pl),
                    licz(x.profit, pl),
                    licz(x.min, pl),
                    licz(x.max, pl),
                    licz(x.obsuniecie, pl),
                    licz(x.dd_pct, pl),
                    licz(x.saldo_otw, pl),
                    licz(x.saldo_zam, pl),
                    x.punktow,
                    x.transakcje,
                    x.wygrane,
                    licz(x.zrealizowany, pl),
                    licz(x.wolumen, pl),
                    x.otwarc,
                    x.silnik_profit.map(|v| licz(v, pl)).unwrap_or_default(),
                    x.sygnaly.map(|v| v.to_string()).unwrap_or_default(),
                ));
            }
        }
        "transakcje" => {
            s.push_str(&format!(
                "ticket{sep}strona{sep}wolumen{sep}otwarcie{sep}cena_otw{sep}zamkniecie{sep}\
                 cena_zam{sep}wynik{sep}prowizja{sep}swap{sep}powod{sep}koszyk\n"
            ));
            for t in p
                .transakcje
                .iter()
                .filter(|t| t.close_ts >= od && t.open_ts < az)
            {
                s.push_str(&format!(
                    "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}\n",
                    t.ticket,
                    t.side,
                    licz(t.volume, pl),
                    iso(t.open_ts),
                    licz(t.open_price, pl),
                    iso(t.close_ts),
                    licz(t.close_price, pl),
                    licz(t.profit, pl),
                    licz(t.commission, pl),
                    licz(t.swap, pl),
                    t.reason,
                    t.basket.map(|b| b.to_string()).unwrap_or_default(),
                ));
            }
        }
        inne => return blad(StatusCode::BAD_REQUEST, format!("nieznany zakres „{inne}”")),
    }

    let nazwa = format!("conduit-graph_{}_{}_{}.csv", id, co, data_iso(od));
    let mut res = Response::new(axum::body::Body::from(s.into_bytes()));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{nazwa}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    res
}

// ============================================================
//  STATYKI
// ============================================================

/// Blokuje wyjście poza zasoby (`..`, ścieżki bezwzględne, dyski).
fn bezpieczna(rel: &str) -> Option<String> {
    let mut czesci = Vec::new();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\\') {
            return None;
        }
        czesci.push(part);
    }
    Some(czesci.join("/"))
}

async fn statyki(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let rel = if path.is_empty() {
        "index.html".to_string()
    } else {
        path.to_string()
    };
    let rel = match bezpieczna(&rel) {
        Some(r) if !r.is_empty() => r,
        _ => "index.html".to_string(),
    };
    if let Some(f) = Zasoby::get(&rel) {
        return z_naglowkami(f.data.to_vec(), &rel);
    }
    let wyglada_na_plik = rel
        .rsplit('/')
        .next()
        .map(|s| s.contains('.'))
        .unwrap_or(false);
    if !wyglada_na_plik {
        if let Some(f) = Zasoby::get("index.html") {
            return z_naglowkami(f.data.to_vec(), "index.html");
        }
    }
    (StatusCode::NOT_FOUND, "nie znaleziono").into_response()
}

fn z_naglowkami(dane: Vec<u8>, rel: &str) -> Response {
    let mime = mime_guess::from_path(rel).first_or_octet_stream();
    let mut res = Response::new(axum::body::Body::from(dane));
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime.essence_str()) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}

// ============================================================
//  START
// ============================================================

/// Gdzie szukać przebiegów, po kolei — tak jak `wizualizacja.exe` szuka modeli:
/// jawne wskazanie wygrywa i wtedy TYLKO ono, bez cichego zjeżdżania gdzie
/// indziej. Bez wskazania: `WYKRESY` obok programu i w górę drzewa, potem
/// katalog bieżący.
fn kandydaci(katalog_exe: &Path, cwd: Option<&Path>) -> Vec<PathBuf> {
    let mut k = vec![katalog_exe.join("WYKRESY")];
    let mut p = katalog_exe.to_path_buf();
    for _ in 0..3 {
        if let Some(nad) = p.parent().map(|x| x.to_path_buf()) {
            k.push(nad.join("WYKRESY"));
            p = nad;
        }
    }
    if let Some(c) = cwd {
        let w = c.join("WYKRESY");
        if !k.contains(&w) {
            k.push(w);
        }
    }
    k
}

fn main() -> Result<()> {
    let args = match czytaj_argumenty(std::env::args().skip(1))? {
        Some(a) => a,
        None => return Ok(()),
    };

    let exe = std::env::current_exe().context("nie znam własnej ścieżki")?;
    let katalog_exe = exe.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();

    let cwd = std::env::current_dir().ok();
    let (korzenie, sprawdzone) = if args.dane.is_empty() {
        let mut k = kandydaci(&katalog_exe, cwd.as_deref());
        let mut istniejace: Vec<PathBuf> = k.iter().filter(|p| p.is_dir()).cloned().collect();
        // Katalog BIEŻĄCY jako źródło dopiero wtedy, gdy nie ma żadnego
        // `WYKRESY`. Inaczej uruchomienie z katalogu domowego kazałoby
        // przeczesać całe drzewo w poszukiwaniu `*_dane.json`.
        if istniejace.is_empty() {
            if let Some(c) = cwd.as_ref() {
                k.push(c.clone());
                if c.is_dir() {
                    istniejace.push(c.clone());
                }
            }
        }
        (istniejace, k)
    } else {
        (args.dane.clone(), args.dane.clone())
    };

    if !interfejs_wbudowany() {
        anyhow::bail!("binarka nie zawiera interfejsu — brakuje crates/graph/web/index.html");
    }

    let stan: Uchwyt = Arc::new(Stan {
        rejestr: Mutex::new(Vec::new()),
        cache: Mutex::new(Vec::new()),
        korzenie: korzenie.clone(),
        sprawdzone: sprawdzone.clone(),
    });
    for k in &korzenie {
        stan.dodaj_korzen(k);
    }
    let znaleziono = stan.rejestr.lock().unwrap().len();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("nie udało się uruchomić środowiska asynchronicznego")?;

    rt.block_on(async move {
        let api = Router::new()
            .route("/health", get(zdrowie))
            .route("/graph/info", get(info))
            .route("/graph/przebiegi", get(lista))
            .route("/graph/dodaj", get(dodaj))
            .route("/graph/p/{id}", get(przebieg))
            .route("/graph/p/{id}/seria", get(seria))
            .route("/graph/p/{id}/slupki", get(slupki))
            .route("/graph/p/{id}/transakcje", get(transakcje))
            .route("/graph/p/{id}/csv", get(csv));
        let app = Router::new()
            .nest("/api", api)
            .fallback(statyki)
            .with_state(stan);

        let adres = SocketAddr::from(([127, 0, 0, 1], args.port));
        let listener = tokio::net::TcpListener::bind(adres)
            .await
            .with_context(|| format!("nie mogę zająć {adres}"))?;
        let lokalny = listener.local_addr().context("nie znam własnego portu")?;
        let url = format!("http://127.0.0.1:{}/", lokalny.port());

        println!("┌──────────────────────────────────────────────");
        println!("│ CONDUIT GRAPH · analiza wykresów  v{WERSJA}");
        if korzenie.is_empty() {
            println!("│ NIE ZNALAZŁEM katalogu z przebiegami. Szukałem w:");
            for k in &sprawdzone {
                println!("│   · {}", k.display());
            }
            println!("│ Wskaż katalog: graph.exe --dane <ścieżka>");
            println!("│ (można też dodać go z poziomu strony)");
        } else {
            for k in &korzenie {
                println!("│ źródło        : {}", k.display());
            }
            println!("│ przebiegów    : {znaleziono}");
        }
        println!("│ adres         : {url}");
        println!("│ zamknięcie    : Ctrl+C albo zamknij to okno");
        println!("└──────────────────────────────────────────────");

        if !args.bez_przegladarki {
            if let Err(e) = open::that_detached(url.as_str()) {
                println!("Nie udało się otworzyć przeglądarki ({e}). Wklej adres: {url}");
            }
        }

        let serwer = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("serwer padł: {e}");
            }
        });
        tokio::select! {
            _ = tokio::signal::ctrl_c() => println!("Kończę."),
            _ = serwer => {}
        }
        Ok::<(), anyhow::Error>(())
    })
}

fn interfejs_wbudowany() -> bool {
    Zasoby::get("index.html")
        .map(|f| f.data.len() > 200)
        .unwrap_or(false)
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;

    fn arg(v: &[&str]) -> Result<Option<Argumenty>> {
        czytaj_argumenty(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn argumenty_zbieraja_wiele_katalogow() {
        let a = arg(&["--dane", "A", "--dane", "B", "--port", "1234"])
            .unwrap()
            .unwrap();
        assert_eq!(a.dane.len(), 2);
        assert_eq!(a.port, 1234);
        assert!(!a.bez_przegladarki);
        assert!(arg(&["--nieznany"]).is_err());
    }

    #[test]
    fn data_tam_i_z_powrotem() {
        for d in [0i64, 1, 19_000, 20_605, 25_000, -1, -365] {
            let (y, m, dd) = data_z_dni(d);
            assert_eq!(dni_z_daty(y, m, dd), d, "rozjazd na dniu {d}");
        }
        // 2026-06-01 to dzień 20605 — tak twierdzi silnik w `dni[].day`
        assert_eq!(data_z_dni(20_605), (2026, 6, 1));
        assert_eq!(
            iso(20_605 * DOBA + 14 * GODZINA + 37 * MINUTA + 5_000),
            "2026-06-01 14:37:05"
        );
    }

    #[test]
    fn granice_kubelkow_sa_wyrownane() {
        // 2026-07-24 14:37:05
        let ts = dni_z_daty(2026, 7, 24) * DOBA + 14 * GODZINA + 37 * MINUTA + 5_000;
        assert_eq!(Poziom::Minuta.granica(ts), ts - 5_000);
        assert_eq!(Poziom::Godzina.granica(ts), ts - 37 * MINUTA - 5_000);
        assert_eq!(Poziom::Dzien.granica(ts), dni_z_daty(2026, 7, 24) * DOBA);
        assert_eq!(Poziom::Miesiac.granica(ts), dni_z_daty(2026, 7, 1) * DOBA);
        assert_eq!(Poziom::Tydzien.granica(ts), dni_z_daty(2026, 7, 20) * DOBA);
    }

    #[test]
    fn miesiac_przeskakuje_przez_grudzien() {
        let grudzien = dni_z_daty(2026, 12, 3) * DOBA;
        assert_eq!(
            Poziom::Miesiac.nastepna(Poziom::Miesiac.granica(grudzien)),
            dni_z_daty(2027, 1, 1) * DOBA
        );
    }

    #[test]
    fn przerzedzanie_zachowuje_szczyt_i_dno() {
        // 10 000 punktów szumu z JEDNYM głębokim dnem i JEDNYM szczytem —
        // dokładnie ten przypadek, w którym „co n-ty punkt" kłamie
        let mut eq: Vec<f64> = (0..10_000).map(|i| 100.0 + (i % 7) as f64).collect();
        eq[4_321] = -500.0;
        eq[7_777] = 9_999.0;
        let idx = przerzedz(&eq, 0, eq.len(), 300);
        assert!(
            idx.len() <= 400,
            "przerzedzenie nie zmieściło się w celu: {}",
            idx.len()
        );
        assert!(
            idx.contains(&4_321),
            "zgubione DNO — to jest ten błąd, którego szukamy"
        );
        assert!(idx.contains(&7_777), "zgubiony SZCZYT");
        assert_eq!(idx[0], 0);
        assert_eq!(*idx.last().unwrap(), 9_999);
        // rosnąco i bez powtórzeń
        assert!(idx.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn przerzedzanie_krotkiej_serii_nic_nie_rusza() {
        let eq: Vec<f64> = (0..50).map(|i| i as f64).collect();
        assert_eq!(przerzedz(&eq, 0, 50, 4000), (0..50).collect::<Vec<_>>());
    }

    fn probka() -> Przebieg {
        probka_trybu(false)
    }

    /// Przebieg testowy: dwie doby po 24 próbki (co godzinę), equity rośnie
    /// o 1 $ na godzinę, w drugiej dobie spada.
    ///
    /// W wariancie DZIENNYM druga doba startuje od nowa od 100 $ — dokładnie
    /// tak, jak robi silnik przy `--daily-reset` (`broker.balance = start`).
    fn probka_trybu(dzienny: bool) -> Przebieg {
        let d0 = dni_z_daty(2026, 6, 1) * DOBA;
        let mut t = Vec::new();
        let mut eq = Vec::new();
        for h in 0..48 {
            t.push(d0 + h * GODZINA + 30 * MINUTA);
            eq.push(if h < 24 {
                100.0 + h as f64
            } else if dzienny {
                100.0 - (h - 24) as f64
            } else {
                124.0 - (h - 24) as f64
            });
        }
        let sal = eq.clone();
        let mut szczyt = Vec::new();
        let mut s = f64::NEG_INFINITY;
        let mut doba = i64::MIN;
        for (i, v) in eq.iter().enumerate() {
            if dzienny {
                let d2 = t[i].div_euclid(DOBA);
                if d2 != doba {
                    doba = d2;
                    s = f64::NEG_INFINITY;
                }
            }
            if *v > s {
                s = *v;
            }
            szczyt.push(s);
        }
        let dni = vec![
            Dzien {
                day: d0 / DOBA,
                date: "2026-06-01".into(),
                start_equity: 100.0,
                end_equity: 123.0,
                profit: 23.0,
                max_dd: 0.0,
                trades: 3,
                signals: 9,
            },
            Dzien {
                day: d0 / DOBA + 1,
                date: "2026-06-02".into(),
                start_equity: if dzienny { 100.0 } else { 123.0 },
                end_equity: if dzienny { 77.0 } else { 101.0 },
                profit: if dzienny { -23.0 } else { -22.0 },
                max_dd: 23.0,
                trades: 2,
                signals: 4,
            },
        ];
        let mut dni_wg_doby = HashMap::new();
        dni_wg_doby.insert(d0 / DOBA, 0);
        dni_wg_doby.insert(d0 / DOBA + 1, 1);
        let transakcje = vec![
            Transakcja {
                ticket: 1,
                side: "Buy".into(),
                volume: 0.01,
                open_ts: d0 + GODZINA,
                close_ts: d0 + 2 * GODZINA,
                profit: 5.0,
                reason: "Tp".into(),
                ..Default::default()
            },
            Transakcja {
                ticket: 2,
                side: "Sell".into(),
                volume: 0.02,
                open_ts: d0 + 2 * GODZINA,
                close_ts: d0 + 30 * GODZINA,
                profit: -3.0,
                reason: "Sl".into(),
                ..Default::default()
            },
        ];
        let mut wg_otwarcia: Vec<u32> = (0..transakcje.len() as u32).collect();
        wg_otwarcia.sort_by_key(|i| transakcje[*i as usize].open_ts);
        Przebieg {
            naglowek: Naglowek {
                saldo_start: 100.0,
                krok_ms: GODZINA,
                tryb: if dzienny {
                    "daily".into()
                } else {
                    "compound".into()
                },
                ..Default::default()
            },
            dzienny,
            t,
            eq,
            sal,
            szczyt,
            dni,
            dni_wg_doby,
            transakcje,
            wg_otwarcia,
        }
    }

    #[test]
    fn kubelki_dnia_niosa_liczby_z_silnika_obok_liczb_z_krzywej() {
        let p = probka();
        let (t0, t1) = p.zakres();
        let (k, obciete) = kubelki(&p, Poziom::Dzien, t0, t1 + 1);
        assert!(!obciete);
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].etykieta, "2026-06-01");
        // z krzywej: 100 → 123
        assert!((k[0].profit - 23.0).abs() < 1e-9);
        // z silnika: to samo, ale liczone niezależnie — i pokazujemy OBIE
        assert_eq!(k[0].silnik_profit, Some(23.0));
        assert_eq!(k[0].sygnaly, Some(9));
        assert_eq!(k[0].punktow, 24);
        // transakcje: jedna domknięta pierwszego dnia, jedna drugiego
        assert_eq!(k[0].transakcje, 1);
        assert_eq!(k[1].transakcje, 1);
        assert_eq!(k[0].otwarc, 2, "obie pozycje OTWARTO pierwszego dnia");
    }

    #[test]
    fn godziny_sumuja_sie_do_dnia() {
        let p = probka();
        let d0 = dni_z_daty(2026, 6, 1) * DOBA;
        let (g, _) = kubelki(&p, Poziom::Godzina, d0, d0 + DOBA);
        assert_eq!(g.len(), 24);
        let suma: f64 = g.iter().map(|x| x.profit).sum();
        let (d, _) = kubelki(&p, Poziom::Dzien, d0, d0 + DOBA);
        assert!(
            (suma - d[0].profit).abs() < 1e-9,
            "godziny nie teleskopują się do doby"
        );
        // pierwsza godzina doby: brak próbki przed nią → otwarcie z salda startowego
        assert_eq!(g[0].punktow, 1);
    }

    #[test]
    fn kubelek_bez_probek_niesie_zero_a_nie_dziure() {
        let p = probka();
        let d0 = dni_z_daty(2026, 6, 1) * DOBA;
        // minuty pierwszej godziny: próbka jest tylko o 00:30
        let (m, _) = kubelki(&p, Poziom::Minuta, d0, d0 + GODZINA);
        assert_eq!(m.len(), 60);
        assert_eq!(m[0].punktow, 0);
        assert_eq!(m[30].punktow, 1);
        // pusty kubełek nie udaje straty: equity przeniesione, wynik zero
        assert_eq!(m[0].profit, 0.0);
        assert!((m[0].zamkniecie - 100.0).abs() < 1e-9);
        assert!((m[31].zamkniecie - m[30].zamkniecie).abs() < 1e-9);
    }

    #[test]
    fn tryb_dzienny_nie_liczy_wyniku_przez_polnoc() {
        let p = probka_trybu(true);
        let (t0, t1) = p.zakres();
        let (d, _) = kubelki(&p, Poziom::Dzien, t0, t1 + 1);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].zrodlo_wyniku, "silnik");
        assert!((d[0].profit - 23.0).abs() < 1e-9);
        assert!((d[1].profit + 23.0).abs() < 1e-9);
        // wejście w dobę drugą to kwota startowa, nie 123 $ z wczoraj
        assert!((d[1].otwarcie - 100.0).abs() < 1e-9);

        let (w, _) = kubelki(&p, Poziom::Tydzien, t0, t1 + 1);
        assert_eq!(w.len(), 1);
        assert!((w[0].profit - 0.0).abs() < 1e-9, "suma dni 23 + (-23) = 0");
        // a z samej krzywej wyszłoby to samo zero, ale Z INNEGO POWODU —
        // i przy innych danych rozjechałoby się o cały wynik tygodnia
        assert_eq!(w[0].zrodlo_wyniku, "silnik");
    }

    #[test]
    fn tryb_dzienny_nie_udaje_obsuniecia_na_przelewie_o_polnocy() {
        let p = probka_trybu(true);
        let (t0, t1) = p.zakres();
        // tydzień obejmuje obie doby; spadek 123 → 100 o północy to przelew
        let (w, _) = kubelki(&p, Poziom::Tydzien, t0, t1 + 1);
        assert!(
            w[0].obsuniecie <= 23.0 + 1e-9,
            "obsunięcie {} zawiera przelew o północy",
            w[0].obsuniecie
        );
        // szczyt kroczący zerowany na dobę → dno drugiego dnia to −23 %, nie −37 %
        assert!(
            w[0].dd_pct > -24.0,
            "dd_pct {} liczone przez północ",
            w[0].dd_pct
        );
    }

    #[test]
    fn tryb_laczony_bierze_wynik_dnia_z_silnika_i_zgadza_sie_z_krzywa() {
        let p = probka();
        let (t0, t1) = p.zakres();
        let (d, _) = kubelki(&p, Poziom::Dzien, t0, t1 + 1);
        for k in &d {
            assert_eq!(k.zrodlo_wyniku, "silnik");
            // w trybie łączonym obie drogi muszą dać to samo — jeśli kiedyś
            // przestaną, to jest sygnał o błędzie, a nie o zaokrągleniu
            assert!(
                (k.profit - k.profit_krzywa).abs() < 1e-9,
                "silnik {} vs krzywa {}",
                k.profit,
                k.profit_krzywa
            );
        }
    }

    #[test]
    fn obsuniecie_w_kubelku_liczy_sie_od_wejscia_a_nie_od_pierwszej_probki() {
        // equity: wchodzimy z 124 (koniec doby 1), doba 2 spada do 101
        let p = probka();
        let d1 = dni_z_daty(2026, 6, 2) * DOBA;
        let (d, _) = kubelki(&p, Poziom::Dzien, d1, d1 + DOBA);
        assert!(
            d[0].obsuniecie > 20.0,
            "obsunięcie {} za małe",
            d[0].obsuniecie
        );
        assert!(d[0].dd_pct < -15.0);
    }

    #[test]
    fn sufit_kubelkow_zglasza_obciecie_zamiast_zawiesic_przegladarke() {
        let p = probka();
        let d0 = dni_z_daty(2026, 6, 1) * DOBA;
        // minuty przez 100 dni to 144 000 kubełków
        let (k, obciete) = kubelki(&p, Poziom::Minuta, d0, d0 + 100 * DOBA);
        assert!(obciete);
        assert_eq!(k.len(), MAX_KUBELKOW);
    }

    #[test]
    fn krok_z_danych_to_mediana_a_nie_srednia() {
        // jedna wielka dziura (weekend) nie może udawać, że próbkujemy co dobę
        let mut t: Vec<i64> = (0..100).map(|i| i * MINUTA).collect();
        t.push(100 * MINUTA + 3 * DOBA);
        assert_eq!(krok_z_danych(&t), MINUTA);
    }

    #[test]
    fn nie_da_sie_wyjsc_poza_zasoby() {
        assert!(bezpieczna("../../secret.txt").is_none());
        assert!(bezpieczna("..\\secret.txt").is_none());
        assert!(bezpieczna("C:/windows/win.ini").is_none());
        assert_eq!(bezpieczna("app.js").as_deref(), Some("app.js"));
    }

    #[test]
    fn identyfikator_nie_przenosi_separatorow_sciezki() {
        let id = zrob_id(
            Path::new("C:/x/WYKRESY/../etc/HYPER-2_daily_dane.json"),
            None,
        );
        assert!(!id.contains('/') && !id.contains('\\') && !id.contains(':'));
        assert!(!id.contains(".."));
    }

    #[test]
    fn etykieta_niesie_katalog_bo_presetow_o_tej_nazwie_sa_dziesiatki() {
        let e = zrob_etykiete(Path::new(
            "C:/x/WYKRESY/HYPER-2_300/HYPER-2_compound_dane.json",
        ));
        assert_eq!(e, "HYPER-2_300 · HYPER-2_compound");
    }

    #[test]
    fn zrodlo_sygnalow_rozroznia_przebiegi_o_tym_samym_presecie() {
        let z = |f: &str, s: &str| {
            nazwa_zrodla(&Naglowek {
                format: f.into(),
                sygnaly: s.into(),
                ..Default::default()
            })
        };
        // pole `format` z presetu ma pierwszeństwo
        assert_eq!(z("NOVA", "signals_nova.json"), "NOVA");
        assert_eq!(z("", "signals_pulsex.json"), "PULSEX");
        assert_eq!(z("", "signals_example.json"), "EXAMPLE");
        // historyczny `signals.json` to ATFX i tylko na nim był mierzony
        assert_eq!(z("", "signals.json"), "ATFX");
        // brak jednego i drugiego = „nie wiem", a nie ciche założenie ATFX
        assert_eq!(z("", ""), "?");
    }

    #[test]
    fn szukanie_idzie_w_gore_od_exe_i_nie_bierze_katalogu_biezacego() {
        // `LAB\graph.exe` obok `WYKRESY` — dwuklik ma trafić w przebiegi
        let k = kandydaci(Path::new("C:/projekt/LAB"), Some(Path::new("C:/gdzies")));
        assert_eq!(k[0], PathBuf::from("C:/projekt/LAB/WYKRESY"));
        assert_eq!(k[1], PathBuf::from("C:/projekt/WYKRESY"));
        // katalog bieżący WCHODZI tylko jako `WYKRESY` w nim, nigdy sam
        assert!(k.contains(&PathBuf::from("C:/gdzies/WYKRESY")));
        assert!(
            !k.contains(&PathBuf::from("C:/gdzies")),
            "sam katalog bieżący jako źródło = przeczesywanie całego drzewa"
        );
    }

    #[test]
    fn liczba_po_polsku_ma_przecinek() {
        assert_eq!(licz(1234.5, true), "1234,5000");
        assert_eq!(licz(1234.5, false), "1234.5000");
    }

    #[test]
    fn saldo_o_innych_znacznikach_czasu_jest_odrzucane() {
        // Gdyby saldo przyszło próbkowane inaczej niż equity, „wynik pływający"
        // (różnica) byłby wymyślony. Wolimy pokazać saldo = equity.
        let dir = std::env::temp_dir().join(format!("cg_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("X_compound_dane.json");
        std::fs::write(
            &p,
            r#"{"preset":"X","saldo_start":100,
            "krzywa":[[1000,100.0],[2000,110.0]],
            "saldo":[[1500,100.0],[2500,105.0]],"dni":[],"metryki":{}}"#,
        )
        .unwrap();
        let w = wczytaj(&p).unwrap();
        assert_eq!(w.sal, w.eq);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wczytanie_liczy_szczyt_kroczacy_raz() {
        let dir = std::env::temp_dir().join(format!("cg_test2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Y_daily_dane.json");
        std::fs::write(
            &p,
            r#"{"preset":"Y","saldo_start":100,
            "krzywa":[[1000,100.0],[2000,150.0],[3000,120.0],[4000,140.0]],
            "saldo":[[1000,100.0],[2000,150.0],[3000,120.0],[4000,140.0]],
            "dni":[],"metryki":{}}"#,
        )
        .unwrap();
        let w = wczytaj(&p).unwrap();
        assert_eq!(w.szczyt, vec![100.0, 150.0, 150.0, 150.0]);
        assert_eq!(w.naglowek.krok_ms, 1000);
        std::fs::remove_dir_all(&dir).ok();
    }
}
