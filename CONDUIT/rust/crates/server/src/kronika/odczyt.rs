
use crate::kronika::{Rodzaj, Wpis};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ============================================================
//  PLIKI
// ============================================================

/// Wszystkie pliki należące do jednej kroniki, **w kolejności czytania**.
///
/// Bieżący plik jest zawsze OSTATNI, a obrócone przed nim — rosnąco po nazwie,
/// bo nazwa niesie znacznik czasu obrotu. Dzięki temu sklejenie plików
/// odtwarza jeden ciągły strumień bez sortowania po zawartości.
///
/// Rozpoznaje też stare układy: katalog z plikami na dobę (`kronika-*.jsonl`
/// pierwszej wersji i `wiadomosci-*.jsonl` archiwum Conduita) — bo dane
/// zebrane, zanim powstał jeden plik, są tak samo prawdziwe.
pub fn pliki_kroniki(sciezka: &Path) -> Vec<PathBuf> {
    if sciezka.is_dir() {
        return pliki_w_katalogu(sciezka, None);
    }
    let katalog = sciezka.parent().unwrap_or_else(|| Path::new("."));
    let rdzen = sciezka
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kronika")
        .to_string();
    let mut obrocone = pliki_w_katalogu(katalog, Some(&format!("{rdzen}-")));
    if sciezka.is_file() {
        obrocone.push(sciezka.to_path_buf());
    }
    obrocone
}

fn pliki_w_katalogu(katalog: &Path, przedrostek: Option<&str>) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(katalog) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter(|p| match przedrostek {
            Some(pref) => p
                .file_name()
                .and_then(|x| x.to_str())
                .map(|n| n.starts_with(pref))
                .unwrap_or(false),
            // katalog bez wskazanego pliku: bierzemy oba znane układy historyczne
            None => p
                .file_name()
                .and_then(|x| x.to_str())
                .map(|n| n.starts_with("kronika") || n.starts_with("wiadomosci"))
                .unwrap_or(false),
        })
        .collect();
    out.sort();
    out
}

// ============================================================
//  ROZPOZNANIE ISTNIEJĄCEGO PLIKU
// ============================================================

/// Co wiemy o pliku, ZANIM do niego dopiszemy.
///
/// Powstało dla jednego wymagania: plik kroniki ma żyć POZA katalogiem bota
/// i przeżyć wgranie kolejnej paczki (`VPSREADY`, `VPSREADY2`…). Nowa wersja
/// bota musi więc umieć powiedzieć, **co zastała**, zanim cokolwiek dopisze —
/// inaczej „kontynuuję" i „właśnie założyłem nowy" wyglądają identycznie.
///
/// Świadomie NIE parsuje całego pliku: przy 50 MB kosztowałoby to sekundy
/// przy każdym starcie bota, a odpowiedzi i tak szukamy w trzech miejscach —
/// ile linii (skan bajtowy), czym się zaczyna i czym kończy (dwa `from_str`).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Rozpoznanie {
    pub istnial: bool,
    pub bajtow: u64,
    /// liczba niepustych linii — tanio, bez rozbioru JSON-a
    pub wierszy: u64,
    /// `odebrano_ms` ostatniego czytelnego wiersza; 0 = nie dało się ustalić
    pub ostatni_ms: i64,
    /// czytelny znacznik ostatniego wiersza (pole `odebrano`, jeśli jest)
    pub ostatni_opis: String,
    /// wersja schematu z PIERWSZEGO wiersza (czym plik zakładano)
    pub schemat_pierwszy: u32,
    /// wersja schematu z OSTATNIEGO wiersza (czym pisała poprzednia binarka)
    pub schemat_ostatni: u32,
    /// czy którakolwiek z powyższych wersji jest NOWSZA niż ta, którą umiemy
    /// czytać — wtedy dopisujemy dalej, ale głośno ostrzegamy
    pub obcy_format: bool,
    /// pierwszej albo ostatniej linii nie dało się rozebrać
    pub nieczytelny: bool,
}

impl Rozpoznanie {
    /// Jedno zdanie do dziennika. To jest cała treść wymagania „zamelduj,
    /// że kontynuujesz".
    pub fn zdanie(&self, sciezka: &Path) -> String {
        // Plik o zerowej długości to nie jest „istniejąca kronika": tak
        // wygląda świeżo utworzony plik po sprawdzeniu zapisywalności.
        if !self.istnial || self.bajtow == 0 {
            return format!("kronika: zakładam nowy plik {}", sciezka.display());
        }
        let ostatni = if self.ostatni_opis.is_empty() {
            "nieznanej daty".to_string()
        } else {
            self.ostatni_opis.clone()
        };
        let mut s = format!(
            "kronika: kontynuuję istniejący plik {}, {} wpisów, ostatni z {ostatni}",
            sciezka.display(),
            self.wierszy
        );
        if self.obcy_format {
            s.push_str(&format!(
                " · ⚠ FORMAT NOWSZY NIŻ ZNANY (schemat {} / {}, umiem {}) — dopisuję dalej, \
                 ale odczyt i statystyka mogą nie widzieć wszystkiego",
                self.schemat_pierwszy,
                self.schemat_ostatni,
                crate::kronika::SCHEMAT
            ));
        } else if self.nieczytelny {
            s.push_str(
                " · ⚠ skrajnych wierszy nie dało się rozebrać (urwany zapis?) — dopisuję dalej",
            );
        }
        s
    }
}

/// Ogląda plik, nie ruszając go.
///
/// Nie zwraca `Result`: brak pliku to normalny, spodziewany stan („świeży
/// start"), a nie awaria. Błąd odczytu daje `nieczytelny`, bo i tak jedyną
/// sensowną reakcją jest dopisanie dalej z ostrzeżeniem — porzucenie zapisu
/// kosztowałoby strumień, którego nie da się nadrobić wstecz.
pub fn rozpoznaj(sciezka: &Path) -> Rozpoznanie {
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

    let mut r = Rozpoznanie::default();
    let Ok(meta) = std::fs::metadata(sciezka) else {
        return r;
    };
    if !meta.is_file() {
        return r;
    }
    r.istnial = true;
    r.bajtow = meta.len();
    if r.bajtow == 0 {
        return r;
    }
    let Ok(mut f) = std::fs::File::open(sciezka) else {
        r.nieczytelny = true;
        return r;
    };

    // --- ile wierszy: skan bajtowy, bez budowania łańcuchów ---
    {
        let mut br = BufReader::with_capacity(1 << 20, &f);
        let mut bufor = [0u8; 1 << 16];
        let mut ostatni_bajt = b'\n';
        loop {
            match br.read(&mut bufor) {
                Ok(0) => break,
                Ok(n) => {
                    r.wierszy += bufor[..n].iter().filter(|b| **b == b'\n').count() as u64;
                    ostatni_bajt = bufor[n - 1];
                }
                Err(_) => {
                    r.nieczytelny = true;
                    break;
                }
            }
        }
        // ostatnia linia bez znaku końca też jest linią
        if ostatni_bajt != b'\n' {
            r.wierszy += 1;
        }
    }

    // --- pierwszy wiersz ---
    let _ = f.seek(SeekFrom::Start(0));
    let mut pierwsza = String::new();
    {
        let mut br = BufReader::new(&f);
        let _ = br.read_line(&mut pierwsza);
    }
    match serde_json::from_str::<crate::kronika::Wpis>(pierwsza.trim()) {
        Ok(w) => r.schemat_pierwszy = w.v,
        Err(_) => r.nieczytelny = true,
    }

    // --- ostatni wiersz: ogon pliku, nie całość ---
    const OGON: u64 = 64 * 1024;
    let od = r.bajtow.saturating_sub(OGON);
    let _ = f.seek(SeekFrom::Start(od));
    let mut ogon = String::new();
    if f.read_to_string(&mut ogon).is_err() {
        // ogon mógł trafić w środek znaku UTF-8 — to nie jest awaria pliku
        ogon.clear();
    }
    let ostatnia = ogon
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    match serde_json::from_str::<crate::kronika::Wpis>(ostatnia.trim()) {
        Ok(mut w) => {
            w.znormalizuj();
            r.schemat_ostatni = w.v;
            r.ostatni_ms = w.odebrano_ms;
            r.ostatni_opis = if w.odebrano.is_empty() {
                crate::kronika::czas_iso(w.odebrano_ms, 0)
            } else {
                w.odebrano.clone()
            };
        }
        Err(_) => r.nieczytelny = true,
    }

    r.obcy_format =
        r.schemat_pierwszy > crate::kronika::SCHEMAT || r.schemat_ostatni > crate::kronika::SCHEMAT;
    r
}

/// Wynik odczytu. `uszkodzonych` nie jest szczegółem: urwana linia po zaniku
/// zasilania jest normalna, ale jeżeli jest ich tysiąc, to statystyka opisuje
/// co innego, niż się wydaje.
#[derive(Debug, Default)]
pub struct Odczyt {
    pub wpisy: Vec<Wpis>,
    pub uszkodzonych: u64,
    pub plikow: usize,
    pub bajtow: u64,
}

/// Czyta całą kronikę i porządkuje po chwili ODEBRANIA.
///
/// Sortowanie jest **stabilne** i idzie po parze `(odebrano_ms, seq)`: dwa
/// zdarzenia z tej samej milisekundy zachowują kolejność, w jakiej naprawdę
/// weszły. Bez `seq` sortowanie potrafiłoby postawić edycję przed wiadomością,
/// którą poprawia — i backtest zobaczyłby drabinkę przed sygnałem.
pub fn czytaj(sciezka: &Path) -> Odczyt {
    let mut o = Odczyt::default();
    for p in pliki_kroniki(sciezka) {
        o.plikow += 1;
        o.bajtow += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let Ok(tresc) = std::fs::read_to_string(&p) else {
            continue;
        };
        for linia in tresc.lines() {
            if linia.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Wpis>(linia) {
                Ok(mut w) => {
                    w.znormalizuj();
                    o.wpisy.push(w);
                }
                Err(_) => o.uszkodzonych += 1,
            }
        }
    }
    o.wpisy.sort_by_key(|w| (w.odebrano_ms, w.seq));
    o
}

// ============================================================
//  STATYSTYKA
// ============================================================

/// Rozkład czasu — mediana na pierwszym miejscu, średnia obok dla porównania.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Rozklad {
    pub n: u64,
    pub min_s: f64,
    pub p50_s: f64,
    pub p90_s: f64,
    pub max_s: f64,
    pub srednia_s: f64,
}

impl Rozklad {
    fn z_probek(mut v: Vec<f64>) -> Rozklad {
        if v.is_empty() {
            return Rozklad::default();
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        let kwantyl = |q: f64| -> f64 {
            // indeks metodą „najbliższej rangi" — bez interpolacji, bo dla
            // kilkuset próbek różnica jest poniżej sekundy, a wynik zostaje
            // PRAWDZIWĄ zaobserwowaną wartością, a nie średnią dwóch
            let i = ((q * n as f64).ceil() as usize).clamp(1, n) - 1;
            v[i]
        };
        Rozklad {
            n: n as u64,
            min_s: v[0],
            p50_s: kwantyl(0.5),
            p90_s: kwantyl(0.9),
            max_s: v[n - 1],
            srednia_s: v.iter().sum::<f64>() / n as f64,
        }
    }
}

/// Statystyka jednego źródła (kanał albo temat forum).
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatKanalu {
    pub chat_id: i64,
    pub chat: String,
    pub temat: Option<i64>,
    /// czy bot nasłuchiwał tego źródła (choć raz w zebranym okresie)
    pub nasluchiwany: bool,
    pub wiadomosci: u64,
    pub edytowanych: u64,
    pub procent_edytowanych: f64,
    pub edycji: u64,
    pub skasowanych: u64,
    /// mediana sekund do PIERWSZEJ edycji
    pub p50_pierwsza_s: f64,
    pub ostatnia_ms: i64,
}

/// Przerwa w nagrywaniu — czas, o którym kronika NIC nie wie.
///
/// Wyznaczana ze znaczników sesji. To jest różnica między „w kanale było
/// cicho" a „rejestrator nie działał", której z samych wiadomości nie da się
/// odczytać.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Przerwa {
    pub od_ms: i64,
    pub do_ms: i64,
    pub sekund: i64,
    /// `true`, gdy przed przerwą NIE BYŁO znacznika `stop` — czyli rejestrator
    /// nie zakończył pracy uprzejmie (awaria, zabity proces, zanik zasilania)
    pub nagle: bool,
}

/// Komplet liczb pokazywanych na pierwszym ekranie.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Statystyki {
    pub wpisow: u64,
    pub plikow: usize,
    pub bajtow: u64,
    pub uszkodzonych: u64,

    /// unikalne wiadomości (para kanał + numer), które widzieliśmy jako nowe
    pub wiadomosci: u64,
    pub nowych: u64,
    pub edycji: u64,
    pub skasowanych: u64,

    /// TO JEST GŁÓWNA LICZBA CAŁEGO NARZĘDZIA
    pub edytowanych: u64,
    pub procent_edytowanych: f64,
    /// ile sekund od odebrania do PIERWSZEJ edycji (tego eksport nie umie)
    pub do_pierwszej: Rozklad,
    /// ile sekund do OSTATNIEJ edycji (to jedyne, co pokazuje eksport)
    pub do_ostatniej: Rozklad,
    /// najwięcej poprawek jednej wiadomości
    pub max_edycji_jednej: u32,
    /// ile wiadomości poprawiono więcej niż raz — czyli ile razy eksport
    /// z Telegrama ukryłby wersję pośrednią
    pub wielokrotnie_edytowanych: u64,

    pub bez_oryginalu: u64,

    pub edycji_po_odpowiedzi: u64,
    /// ile sekund po pierwszej odpowiedzi przyszła taka edycja
    pub po_odpowiedzi: Rozklad,

    pub od_ms: i64,
    pub do_ms: i64,
    pub zrodel: usize,
    pub wg_zrodla: Vec<StatKanalu>,
    pub przerwy: Vec<Przerwa>,
    /// łączny czas przerw w nagrywaniu
    pub przerw_sekund: i64,
}

#[derive(Default)]
struct Slad {
    pierwsze_ms: i64,
    pierwsza_edycja_ms: Option<i64>,
    ostatnia_edycja_ms: Option<i64>,
    edycji: u32,
    chat_id: i64,
    temat: Option<i64>,
    mial_nowa: bool,
}

/// Liczy statystykę z pliku (albo katalogu) kroniki.
pub fn statystyki(sciezka: &Path) -> Statystyki {
    statystyki_z(std::slice::from_ref(&sciezka.to_path_buf()))
}

pub fn statystyki_z(zrodla: &[PathBuf]) -> Statystyki {
    let mut wpisy = Vec::new();
    let (mut plikow, mut bajtow, mut uszkodzonych) = (0usize, 0u64, 0u64);
    for z in zrodla {
        let o = czytaj(z);
        plikow += o.plikow;
        bajtow += o.bajtow;
        uszkodzonych += o.uszkodzonych;
        wpisy.extend(o.wpisy);
    }
    // Sklejone źródła trzeba uporządkować RAZEM — inaczej wiersze z archiwum
    // stałyby w bloku przed kroniką i przerwy wyszłyby fikcyjne.
    wpisy.sort_by_key(|w| (w.odebrano_ms, w.seq));
    let mut s = policz(&wpisy);
    s.plikow = plikow;
    s.bajtow = bajtow;
    s.uszkodzonych = uszkodzonych;
    s
}

/// Liczy statystykę z gotowej listy wierszy — wydzielone, żeby dało się
/// przetestować bez dotykania dysku.
pub fn policz(wpisy: &[Wpis]) -> Statystyki {
    let mut s = Statystyki {
        wpisow: wpisy.len() as u64,
        ..Default::default()
    };
    let mut slady: HashMap<(i64, i64), Slad> = HashMap::new();
    // Kiedy PIERWSZY RAZ ktoś odpowiedział na daną wiadomość. Potrzebne, żeby
    // wyłapać edycje dopisywane po fakcie — patrz `edycji_po_odpowiedzi`.
    let mut pierwsza_odpowiedz: HashMap<(i64, i64), i64> = HashMap::new();
    // Kolejność źródeł ma być stabilna (kolejność pierwszego pojawienia),
    // bo lista w interfejsie nie może podskakiwać przy każdym odświeżeniu.
    let mut kolejnosc_zrodel: Vec<(i64, Option<i64>)> = Vec::new();
    let mut zrodla: HashMap<(i64, Option<i64>), StatKanalu> = HashMap::new();
    let mut ostatnia_aktywnosc = 0i64;
    let mut zamkniete_uprzejmie = true;

    for w in wpisy {
        if !w.rodzaj.to_wiadomosc() {
            match w.rodzaj {
                Rodzaj::Start => {
                    if ostatnia_aktywnosc > 0 && w.odebrano_ms > ostatnia_aktywnosc {
                        let sek = (w.odebrano_ms - ostatnia_aktywnosc) / 1000;
                        s.przerwy.push(Przerwa {
                            od_ms: ostatnia_aktywnosc,
                            do_ms: w.odebrano_ms,
                            sekund: sek,
                            nagle: !zamkniete_uprzejmie,
                        });
                        s.przerw_sekund += sek;
                    }
                    zamkniete_uprzejmie = false;
                }
                Rodzaj::Stop => zamkniete_uprzejmie = true,
                _ => {}
            }
            ostatnia_aktywnosc = w.odebrano_ms.max(ostatnia_aktywnosc);
            continue;
        }

        if s.od_ms == 0 || w.odebrano_ms < s.od_ms {
            s.od_ms = w.odebrano_ms;
        }
        s.do_ms = s.do_ms.max(w.odebrano_ms);
        ostatnia_aktywnosc = ostatnia_aktywnosc.max(w.odebrano_ms);

        if let Some(r) = w.reply_to {
            pierwsza_odpowiedz
                .entry((w.chat_id, r))
                .and_modify(|t| *t = (*t).min(w.odebrano_ms))
                .or_insert(w.odebrano_ms);
        }

        let klucz_z = (w.chat_id, w.temat);
        if !zrodla.contains_key(&klucz_z) {
            kolejnosc_zrodel.push(klucz_z);
            zrodla.insert(
                klucz_z,
                StatKanalu {
                    chat_id: w.chat_id,
                    chat: w.chat.clone(),
                    temat: w.temat,
                    nasluchiwany: false,
                    wiadomosci: 0,
                    edytowanych: 0,
                    procent_edytowanych: 0.0,
                    edycji: 0,
                    skasowanych: 0,
                    p50_pierwsza_s: 0.0,
                    ostatnia_ms: 0,
                },
            );
        }
        let z = zrodla.get_mut(&klucz_z).expect("wstawione linijkę wyżej");
        z.nasluchiwany |= w.nasluchiwany;
        z.ostatnia_ms = z.ostatnia_ms.max(w.odebrano_ms);
        // Nazwa kanału bywa pusta w najstarszych wierszach — bierzemy pierwszą
        // niepustą, jaka się trafi, zamiast pokazywać goły identyfikator.
        if z.chat.is_empty() && !w.chat.is_empty() {
            z.chat = w.chat.clone();
        }

        match w.rodzaj {
            Rodzaj::Nowa => {
                s.nowych += 1;
                z.wiadomosci += 1;
                let sl = slady.entry(w.klucz()).or_insert_with(|| Slad {
                    pierwsze_ms: w.odebrano_ms,
                    chat_id: w.chat_id,
                    temat: w.temat,
                    ..Default::default()
                });
                // Wersja pierwsza jest punktem odniesienia dla wszystkich
                // odstępów — także wtedy, gdy ślad założyła wcześniejsza edycja
                // (plik sklejony z dwóch źródeł potrafi mieć taką kolejność).
                if !sl.mial_nowa {
                    sl.mial_nowa = true;
                    sl.pierwsze_ms = w.odebrano_ms;
                }
            }
            Rodzaj::Edycja => {
                s.edycji += 1;
                z.edycji += 1;
                // Edycja wiadomości, której NOWEJ wersji nie mamy (rejestrator
                // ruszył później niż kanał), zakłada ślad z czasem tej edycji —
                // inaczej zniknęłaby z liczb zupełnie. Odstęp jest wtedy zerowy
                // i nie zafałszuje rozkładu w górę.
                let sl = slady.entry(w.klucz()).or_insert_with(|| Slad {
                    pierwsze_ms: w.odebrano_ms,
                    chat_id: w.chat_id,
                    temat: w.temat,
                    ..Default::default()
                });
                sl.edycji += 1;
                if sl.pierwsza_edycja_ms.is_none() {
                    sl.pierwsza_edycja_ms = Some(w.odebrano_ms);
                }
                sl.ostatnia_edycja_ms = Some(w.odebrano_ms);
            }
            Rodzaj::Skasowana => {
                s.skasowanych += 1;
                z.skasowanych += 1;
            }
            _ => {}
        }
    }

    // Rejestrator, który nie zdążył zapisać `stop`, zostawia otwartą sesję —
    // to nie jest przerwa, tylko „koniec danych". Nie dopisujemy jej.

    let mut pierwsze: Vec<f64> = Vec::new();
    let mut ostatnie: Vec<f64> = Vec::new();
    let mut po_odpowiedzi: Vec<f64> = Vec::new();
    let mut pierwsze_wg_zrodla: HashMap<(i64, Option<i64>), Vec<f64>> = HashMap::new();

    for (klucz, sl) in slady.iter() {
        if !sl.mial_nowa {
            s.bez_oryginalu += 1;
            continue;
        }
        s.wiadomosci += 1;
        if sl.edycji == 0 {
            continue;
        }
        s.edytowanych += 1;
        if sl.edycji > 1 {
            s.wielokrotnie_edytowanych += 1;
        }
        s.max_edycji_jednej = s.max_edycji_jednej.max(sl.edycji);
        if let Some(t) = sl.pierwsza_edycja_ms {
            let sek = (t - sl.pierwsze_ms).max(0) as f64 / 1000.0;
            pierwsze.push(sek);
            pierwsze_wg_zrodla
                .entry((sl.chat_id, sl.temat))
                .or_default()
                .push(sek);
        }
        if let Some(t) = sl.ostatnia_edycja_ms {
            ostatnie.push((t - sl.pierwsze_ms).max(0) as f64 / 1000.0);
            // Edycja PO odpowiedzi: w eksporcie z Telegrama ta wiadomość
            // dostanie znacznik nowszy niż komunikaty, które na nią
            // odpowiadały — i odtworzony strumień będzie niewykonalny.
            if let Some(&o) = pierwsza_odpowiedz.get(klucz) {
                if t > o {
                    s.edycji_po_odpowiedzi += 1;
                    po_odpowiedzi.push((t - o) as f64 / 1000.0);
                }
            }
        }
        if let Some(z) = zrodla.get_mut(&(sl.chat_id, sl.temat)) {
            z.edytowanych += 1;
        }
    }

    s.procent_edytowanych = if s.wiadomosci > 0 {
        s.edytowanych as f64 * 100.0 / s.wiadomosci as f64
    } else {
        0.0
    };
    s.do_pierwszej = Rozklad::z_probek(pierwsze);
    s.do_ostatniej = Rozklad::z_probek(ostatnie);
    s.po_odpowiedzi = Rozklad::z_probek(po_odpowiedzi);

    for k in &kolejnosc_zrodel {
        if let Some(mut z) = zrodla.remove(k) {
            z.procent_edytowanych = if z.wiadomosci > 0 {
                z.edytowanych as f64 * 100.0 / z.wiadomosci as f64
            } else {
                0.0
            };
            z.p50_pierwsza_s = pierwsze_wg_zrodla
                .remove(k)
                .map(|v| Rozklad::z_probek(v).p50_s)
                .unwrap_or(0.0);
            s.wg_zrodla.push(z);
        }
    }
    // Najgadatliwsze źródło na górze — to ono decyduje o wyniku całości.
    s.wg_zrodla.sort_by(|a, b| b.wiadomosci.cmp(&a.wiadomosci));
    s.zrodel = s.wg_zrodla.len();
    s
}

#[cfg(test)]
mod testy {
    use super::*;
    use crate::kronika::SCHEMAT;

    fn w(ms: i64, seq: u64, rodzaj: Rodzaj, msg_id: i64, chat_id: i64) -> Wpis {
        Wpis {
            v: SCHEMAT,
            seq,
            rodzaj,
            odebrano_ms: ms,
            odebrano: String::new(),
            ts_telegram_ms: ms,
            chat_id,
            chat: format!("KANAL{chat_id}"),
            temat: None,
            msg_id,
            reply_to: None,
            edit_of: if rodzaj == Rodzaj::Edycja {
                Some(msg_id)
            } else {
                None
            },
            text: "x".into(),
            znakow: 1,
            nasluchiwany: true,
            format: None,
            rozpoznane: true,
            uwaga: None,
            ts_telegram: None,
        }
    }

    const S: i64 = 1000;

    #[test]
    fn procent_edytowanych_i_mediana_do_pierwszej_edycji() {
        // 4 wiadomości, 3 edytowane; odstępy do pierwszej edycji: 10 s, 20 s, 300 s
        let t = 1_785_000_000_000;
        let wpisy = vec![
            w(t, 1, Rodzaj::Nowa, 1, -100),
            w(t + 10 * S, 2, Rodzaj::Edycja, 1, -100),
            w(t + 100 * S, 3, Rodzaj::Nowa, 2, -100),
            w(t + 120 * S, 4, Rodzaj::Edycja, 2, -100),
            w(t + 200 * S, 5, Rodzaj::Nowa, 3, -100),
            w(t + 500 * S, 6, Rodzaj::Edycja, 3, -100),
            w(t + 600 * S, 7, Rodzaj::Nowa, 4, -100),
        ];
        let s = policz(&wpisy);
        assert_eq!(s.wiadomosci, 4);
        assert_eq!(s.edytowanych, 3);
        assert!((s.procent_edytowanych - 75.0).abs() < 1e-9);
        assert_eq!(s.do_pierwszej.n, 3);
        assert_eq!(s.do_pierwszej.min_s, 10.0);
        assert_eq!(s.do_pierwszej.p50_s, 20.0, "mediana, nie średnia");
        assert_eq!(s.do_pierwszej.max_s, 300.0);
        // średnia (110 s) jest 5,5x większa od mediany — dokładnie ten długi
        // ogon, dla którego podajemy obie liczby
        assert!(s.do_pierwszej.srednia_s > 100.0);
    }

    #[test]
    fn pierwsza_edycja_to_nie_to_samo_co_ostatnia() {
        // TO JEST CAŁY POWÓD ISTNIENIA NARZĘDZIA: eksport z Telegrama zna tylko
        // ostatnią edycję (300 s), a decyzję bot podjął po pierwszej (12 s).
        let t = 1_785_000_000_000;
        let wpisy = vec![
            w(t, 1, Rodzaj::Nowa, 1, -100),
            w(t + 12 * S, 2, Rodzaj::Edycja, 1, -100),
            w(t + 300 * S, 3, Rodzaj::Edycja, 1, -100),
        ];
        let s = policz(&wpisy);
        assert_eq!(s.edytowanych, 1);
        assert_eq!(s.edycji, 2);
        assert_eq!(s.do_pierwszej.p50_s, 12.0);
        assert_eq!(s.do_ostatniej.p50_s, 300.0);
        assert_eq!(
            s.wielokrotnie_edytowanych, 1,
            "eksport ukryłby wersję pośrednią"
        );
        assert_eq!(s.max_edycji_jednej, 2);
    }

    #[test]
    fn edycja_po_odpowiedzi_lamie_przyczynowosc_i_jest_policzona() {
        let t = 1_785_000_000_000;
        let mut odpowiedz = w(t + 100 * S, 2, Rodzaj::Nowa, 2, -100);
        odpowiedz.reply_to = Some(1);
        let wpisy = vec![
            w(t, 1, Rodzaj::Nowa, 1, -100),
            odpowiedz,
            w(t + 900 * S, 3, Rodzaj::Edycja, 1, -100),
        ];
        let s = policz(&wpisy);
        assert_eq!(s.edycji_po_odpowiedzi, 1);
        assert_eq!(
            s.po_odpowiedzi.p50_s, 800.0,
            "800 s PO pierwszej odpowiedzi"
        );
    }

    #[test]
    fn edycja_przed_odpowiedzia_przyczynowosci_nie_lamie() {
        let t = 1_785_000_000_000;
        let mut odpowiedz = w(t + 900 * S, 3, Rodzaj::Nowa, 2, -100);
        odpowiedz.reply_to = Some(1);
        let s = policz(&[
            w(t, 1, Rodzaj::Nowa, 1, -100),
            w(t + 100 * S, 2, Rodzaj::Edycja, 1, -100),
            odpowiedz,
        ]);
        assert_eq!(s.edycji_po_odpowiedzi, 0);
        assert_eq!(s.edytowanych, 1, "zwykła edycja nadal się liczy");
    }

    #[test]
    fn ten_sam_numer_w_dwoch_kanalach_to_dwie_wiadomosci() {
        // `msg_id` jest unikalny w obrębie czatu, nie globalnie. Klucz bez
        // `chat_id` skleiłby dwa kanały w jeden i zaniżył liczbę wiadomości.
        let t = 1_785_000_000_000;
        let wpisy = vec![
            w(t, 1, Rodzaj::Nowa, 7, -100),
            w(t + S, 2, Rodzaj::Nowa, 7, -200),
            w(t + 2 * S, 3, Rodzaj::Edycja, 7, -200),
        ];
        let s = policz(&wpisy);
        assert_eq!(s.wiadomosci, 2);
        assert_eq!(s.edytowanych, 1);
        assert_eq!(s.zrodel, 2);
    }

    #[test]
    fn temat_forum_jest_osobnym_zrodlem() {
        let t = 1_785_000_000_000;
        let mut a = w(t, 1, Rodzaj::Nowa, 1, -100);
        a.temat = Some(7);
        let mut b = w(t + S, 2, Rodzaj::Nowa, 2, -100);
        b.temat = Some(9);
        let s = policz(&[a, b]);
        assert_eq!(s.zrodel, 2, "dwa tematy tego samego kanału to dwa źródła");
    }

    #[test]
    fn przerwa_w_nagrywaniu_jest_widoczna_i_odrozniona_od_ciszy() {
        let t = 1_785_000_000_000;
        let mut start1 = w(t, 1, Rodzaj::Start, 0, 0);
        start1.rodzaj = Rodzaj::Start;
        let mut stop = w(t + 60 * S, 3, Rodzaj::Stop, 0, 0);
        stop.rodzaj = Rodzaj::Stop;
        let mut start2 = w(t + 3660 * S, 4, Rodzaj::Start, 0, 0);
        start2.rodzaj = Rodzaj::Start;

        let s = policz(&[
            start1,
            w(t + 10 * S, 2, Rodzaj::Nowa, 1, -100),
            stop,
            start2,
        ]);
        assert_eq!(s.przerwy.len(), 1);
        assert_eq!(s.przerwy[0].sekund, 3600);
        assert!(
            !s.przerwy[0].nagle,
            "po znaczniku stop przerwa jest planowa"
        );
        assert_eq!(s.przerw_sekund, 3600);
        assert_eq!(s.wiadomosci, 1, "znaczniki sesji NIE są wiadomościami");
    }

    #[test]
    fn brak_znacznika_stop_znaczy_awarie() {
        let t = 1_785_000_000_000;
        let start1 = w(t, 1, Rodzaj::Start, 0, 0);
        let start2 = w(t + 600 * S, 3, Rodzaj::Start, 0, 0);
        let s = policz(&[start1, w(t + 10 * S, 2, Rodzaj::Nowa, 1, -100), start2]);
        assert_eq!(s.przerwy.len(), 1);
        assert!(
            s.przerwy[0].nagle,
            "brak `stop` = proces zginął, nie został zamknięty"
        );
    }

    #[test]
    fn edycja_bez_oryginalu_nie_psuje_glownej_liczby() {
        let t = 1_785_000_000_000;
        let s = policz(&[w(t, 1, Rodzaj::Edycja, 1, -100)]);
        assert_eq!(s.wiadomosci, 0, "bez oryginału nie ma czego mierzyć");
        assert_eq!(s.edytowanych, 0);
        assert_eq!(
            s.bez_oryginalu, 1,
            "ale liczba musi być WIDOCZNA, nie przemilczana"
        );
        assert_eq!(s.edycji, 1, "samo zdarzenie edycji nadal się liczy");
        assert_eq!(s.do_pierwszej.n, 0, "żadnego zera w rozkładzie");
    }

    #[test]
    fn oryginal_po_edycji_w_sklejonym_pliku_nadal_daje_pomiar() {
        let t = 1_785_000_000_000;
        let s = policz(&[
            w(t + 30 * S, 2, Rodzaj::Edycja, 1, -100),
            w(t, 1, Rodzaj::Nowa, 1, -100),
        ]);
        assert_eq!(s.wiadomosci, 1);
        assert_eq!(s.edytowanych, 1);
        assert_eq!(s.bez_oryginalu, 0);
        assert_eq!(s.do_pierwszej.p50_s, 30.0);
    }

    #[test]
    fn pusty_zbior_nie_wybucha() {
        let s = policz(&[]);
        assert_eq!(s.wiadomosci, 0);
        assert_eq!(s.procent_edytowanych, 0.0);
        assert_eq!(s.do_pierwszej.n, 0);
    }

    #[test]
    fn kolejnosc_czytania_stawia_biezacy_plik_na_koncu() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "kronika-pliki-{}-{}",
            std::process::id(),
            crate::kronika::teraz_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for n in [
            "kronika-20260801-120000.jsonl",
            "kronika-20260802-120000.jsonl",
            "kronika.jsonl",
        ] {
            std::fs::write(dir.join(n), "").unwrap();
        }
        let p = pliki_kroniki(&dir.join("kronika.jsonl"));
        assert_eq!(p.len(), 3);
        assert!(p[0].to_string_lossy().contains("20260801"));
        assert!(p[1].to_string_lossy().contains("20260802"));
        assert_eq!(
            p[2],
            dir.join("kronika.jsonl"),
            "bieżący plik czytamy OSTATNI"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn czyta_katalog_starego_archiwum_conduita() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "kronika-stare-{}-{}",
            std::process::id(),
            crate::kronika::teraz_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("wiadomosci-2026-07-29.jsonl"),
            "{\"v\":1,\"seq\":1,\"event\":\"received\",\"received_at_ms\":1785000000000,\
             \"msg_ts_ms\":1785000000000,\"chat_id\":-100,\"msg_id\":1,\"source_name\":\"ATFX\",\
             \"text\":\"BUY\",\"monitored\":true}\n\
             {\"v\":1,\"seq\":2,\"event\":\"edited\",\"received_at_ms\":1785000030000,\
             \"msg_ts_ms\":1785000000000,\"chat_id\":-100,\"msg_id\":1,\"edit_of\":1,\
             \"source_name\":\"ATFX\",\"text\":\"BUY TP1\",\"monitored\":true}\n",
        )
        .unwrap();
        let s = statystyki(&dir);
        assert_eq!(s.wiadomosci, 1);
        assert_eq!(s.edytowanych, 1);
        assert_eq!(s.do_pierwszej.p50_s, 30.0);
        assert_eq!(s.wg_zrodla[0].chat, "ATFX");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn uszkodzona_linia_nie_przerywa_odczytu_ale_jest_policzona() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "kronika-urwane-{}-{}",
            std::process::id(),
            crate::kronika::teraz_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("kronika.jsonl"),
            "{\"rodzaj\":\"nowa\",\"odebrano_ms\":1,\"chat_id\":-1,\"msg_id\":1}\n\
             {\"rodzaj\":\"nowa\",\"odeb\n\
             {\"rodzaj\":\"nowa\",\"odebrano_ms\":3,\"chat_id\":-1,\"msg_id\":2}\n",
        )
        .unwrap();
        let s = statystyki(&dir.join("kronika.jsonl"));
        assert_eq!(
            s.wiadomosci, 2,
            "tydzień pracy nie może przepaść przez jedną urwaną linię"
        );
        assert_eq!(s.uszkodzonych, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
