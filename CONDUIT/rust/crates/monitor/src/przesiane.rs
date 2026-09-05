//! Przeglądarka presetów JUŻ POLICZONYCH w trwającym przemiataniu.
//!
//! Przemiatanie na trzy tysiące konfiguracji trwa kilkanaście godzin i przez
//! ten czas okno pokazywało wyłącznie pasek postępu. Tymczasem plik
//! `wyniki_czastkowe.json` — dopisywany po KAŻDYM przebiegu — zawiera komplet
//! statystyk wszystkiego, co już policzone. Ten moduł czyta go i porządkuje,
//! żeby dało się zobaczyć czempiona bez czekania na koniec.
//!
//! # Dlaczego surowy JSON, a nie `conduit_backtest::Metrics`
//!
//! Zależność idzie w drugą stronę: to backtest zależy od monitora. Poza tym
//! typowany odczyt znaczyłby, że dopisanie metryki w backteście psuje okno
//! przy niezgodnym pliku. Czytamy mapę `nazwa → obiekt` i sięgamy po pola po
//! nazwie: brak pola daje „—", nigdy błąd.

use std::path::Path;

/// Kierunek, w którym liczba jest LEPSZA.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lepiej {
    Wiecej,
    Mniej,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Dolary,
    Procent,
    Liczba,
    Sztuki,
}

/// Po czym porządkujemy listę presetów.
pub struct Kryterium {
    /// napis w rozwijanym wyborze
    pub etykieta: &'static str,
    /// klucz w `wyniki_czastkowe.json`; pusty = ocena łączna (patrz [`ocena_laczna`])
    pub pole: &'static str,
    pub lepiej: Lepiej,
    /// jak sformatować wartość obok nazwy presetu
    pub format: Format,
}

/// Kryteria w kolejności, w jakiej pokazuje je okno. PIERWSZE jest domyślne.
///
/// „Wszystkie statystyki" stoi na końcu, bo to jedyna pozycja, która nie
/// odczytuje jednego pola, tylko składa rangi — patrz [`ocena_laczna`].
pub const KRYTERIA: &[Kryterium] = &[
    Kryterium {
        etykieta: "zysk końcowy",
        pole: "total_profit",
        lepiej: Lepiej::Wiecej,
        format: Format::Dolary,
    },
    Kryterium {
        etykieta: "% dodatnich dni rynkowych (equity)",
        pole: "positive_market_days_pct",
        lepiej: Lepiej::Wiecej,
        format: Format::Procent,
    },
    Kryterium {
        etykieta: "% dodatnich dni z zamknięciami (legacy)",
        pole: "win_days_pct",
        lepiej: Lepiej::Wiecej,
        format: Format::Procent,
    },
    Kryterium {
        etykieta: "profit factor",
        pole: "profit_factor",
        lepiej: Lepiej::Wiecej,
        format: Format::Liczba,
    },
    Kryterium {
        etykieta: "najniższe obsunięcie",
        pole: "max_dd_pct",
        lepiej: Lepiej::Mniej,
        format: Format::Procent,
    },
    Kryterium {
        etykieta: "najgorszy dzień rynkowy (equity)",
        pole: "worst_market_day",
        lepiej: Lepiej::Wiecej,
        format: Format::Dolary,
    },
    Kryterium {
        etykieta: "najgorszy dzień z zamknięciami (legacy)",
        pole: "worst_day",
        lepiej: Lepiej::Wiecej,
        format: Format::Dolary,
    },
    Kryterium {
        etykieta: "mediana dnia",
        pole: "median_day",
        lepiej: Lepiej::Wiecej,
        format: Format::Dolary,
    },
    Kryterium {
        etykieta: "skuteczność bez BE",
        pole: "win_rate_bez_be",
        lepiej: Lepiej::Wiecej,
        format: Format::Procent,
    },
    Kryterium {
        etykieta: "najniższe equity",
        pole: "min_equity",
        lepiej: Lepiej::Wiecej,
        format: Format::Dolary,
    },
    Kryterium {
        etykieta: "Sharpe",
        pole: "sharpe",
        lepiej: Lepiej::Wiecej,
        format: Format::Liczba,
    },
    Kryterium {
        etykieta: "Calmar",
        pole: "calmar",
        lepiej: Lepiej::Wiecej,
        format: Format::Liczba,
    },
    Kryterium {
        etykieta: "recovery factor",
        pole: "recovery_factor",
        lepiej: Lepiej::Wiecej,
        format: Format::Liczba,
    },
    Kryterium {
        etykieta: "seria stratnych dni",
        pole: "max_losing_streak_days",
        lepiej: Lepiej::Mniej,
        format: Format::Sztuki,
    },
    Kryterium {
        etykieta: "zysk miesięcznie",
        pole: "monthly_profit",
        lepiej: Lepiej::Wiecej,
        format: Format::Dolary,
    },
    Kryterium {
        etykieta: "transakcje",
        pole: "trades",
        lepiej: Lepiej::Wiecej,
        format: Format::Sztuki,
    },
    Kryterium {
        etykieta: "liczba koszyków",
        pole: "baskets",
        lepiej: Lepiej::Wiecej,
        format: Format::Sztuki,
    },
    Kryterium {
        etykieta: "% wykorzystanych sygnałów",
        pole: "signal_use_pct",
        lepiej: Lepiej::Wiecej,
        format: Format::Procent,
    },
    Kryterium {
        etykieta: "WSZYSTKIE STATYSTYKI",
        pole: "",
        lepiej: Lepiej::Wiecej,
        format: Format::Liczba,
    },
];

/// Osie, z których składa się ocena łączna, wraz z kierunkiem.
///
/// Zysk jest tu obecny, ale z taką samą wagą jak reszta. O to chodzi w tym
/// kryterium: preset zarabiający milion z jednym dniem, który zjada połowę
/// konta, ma przegrać z presetem zarabiającym mniej, ale wszędzie równym.
const OSIE_LACZNE: &[(&str, Lepiej)] = &[
    ("total_profit", Lepiej::Wiecej),
    ("positive_market_days_pct", Lepiej::Wiecej),
    ("profit_factor", Lepiej::Wiecej),
    ("max_dd_pct", Lepiej::Mniej),
    ("worst_market_day", Lepiej::Wiecej),
    ("median_day", Lepiej::Wiecej),
    ("win_rate_bez_be", Lepiej::Wiecej),
    ("sharpe", Lepiej::Wiecej),
    ("max_losing_streak_days", Lepiej::Mniej),
    ("min_equity", Lepiej::Wiecej),
    ("baskets", Lepiej::Wiecej),
    ("signal_use_pct", Lepiej::Wiecej),
];

/// Jeden policzony preset.
pub struct Wynik {
    pub nazwa: String,
    pola: serde_json::Map<String, serde_json::Value>,
    /// ocena łączna, 0…1; liczona raz przy wczytaniu
    laczna: f64,
}

impl Wynik {
    /// Liczba spod klucza; `None`, gdy pola nie ma albo nie jest liczbą.
    pub fn liczba(&self, klucz: &str) -> Option<f64> {
        if klucz == "signal_use_pct" {
            let seen = self.liczba("signals_seen")?;
            let taken = self.liczba("signals_taken")?;
            return (seen > 0.0).then_some(taken / seen * 100.0);
        }
        self.pola
            .get(klucz)
            .and_then(|v| v.as_f64())
            .filter(|x| x.is_finite())
    }

    /// Czy przebieg wyzerował konto. Taki wynik nie jest „najlepszy" w żadnym
    /// ujęciu, choćby po drodze pokazał najwyższy zysk na świecie.
    pub fn wysadzony(&self) -> bool {
        self.pola
            .get("blown")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Ile transakcji zawarł przebieg.
    pub fn transakcje(&self) -> u64 {
        self.liczba("trades").unwrap_or(0.0).max(0.0) as u64
    }

    /// Przebieg, który NIE ZAWARŁ ANI JEDNEJ TRANSAKCJI.
    ///
    /// Taki wynik wygrywa każdą oś ryzyka: obsunięcie zero, najgorszy dzień
    /// zero, seria strat zero. To nie jest strategia bez ryzyka, tylko brak
    /// strategii, i na czele listy byłby zwykłym kłamstwem — dlatego ląduje
    /// na końcu razem z wysadzonymi kontami. Trafia się to często: w pliku z
    /// 801 presetów takich było 25, a jeden z nich prowadził w rankingu
    /// „najniższe obsunięcie".
    pub fn bez_handlu(&self) -> bool {
        self.transakcje() == 0
    }

    /// Wynik, którego nie wolno postawić na czele listy.
    pub fn zdyskwalifikowany(&self) -> bool {
        self.wysadzony() || self.bez_handlu()
    }

    /// Krótkie wyjaśnienie dyskwalifikacji do pokazania obok nazwy; pusty
    /// łańcuch = wynik jest w porządku.
    pub fn powod_dyskwalifikacji(&self) -> &'static str {
        if self.wysadzony() {
            "KONTO NA ZERO"
        } else if self.bez_handlu() {
            "BEZ TRANSAKCJI"
        } else {
            ""
        }
    }

    /// Wartość według kryterium, gotowa do porównania.
    pub fn wg(&self, k: &Kryterium) -> Option<f64> {
        if k.pole.is_empty() {
            Some(self.laczna)
        } else {
            self.liczba(k.pole)
        }
    }

    /// Wartość jako napis do pokazania obok nazwy.
    pub fn napis(&self, k: &Kryterium) -> String {
        self.napis_w_jezyku(k, crate::language::Language::Pl)
    }

    pub fn napis_w_jezyku(&self, k: &Kryterium, language: crate::language::Language) -> String {
        match self.wg(k) {
            None => "—".into(),
            Some(v) => match k.format {
                Format::Dolary => format!("{} $", language.number(v, 2)),
                Format::Procent => format!("{v:.1} %"),
                Format::Liczba => format!("{v:.2}"),
                Format::Sztuki => format!("{}", v.round() as i64),
            },
        }
    }

    /// Wszystkie pola proste do tabelki.
    ///
    /// Pola, po których wolno sortować ([`KRYTERIA`]), idą PIERWSZE i w tej
    /// samej kolejności co w rozwijanym wyborze. Reszta leci alfabetycznie za
    /// nimi. Kolejność z pliku byłaby tu bezużyteczna: `serde_json` bez cechy
    /// `preserve_order` układa klucze alfabetycznie, więc `total_profit`
    /// lądowałby gdzieś w ogonie, za `avg_hold_min` i `bes`.
    pub fn wiersze(&self) -> Vec<(String, String)> {
        self.wiersze_w_jezyku(crate::language::Language::Pl)
    }

    pub fn wiersze_w_jezyku(&self, language: crate::language::Language) -> Vec<(String, String)> {
        let mut w = Vec::new();
        let mut kolejnosc: Vec<&str> = KRYTERIA
            .iter()
            .map(|k| k.pole)
            .filter(|p| !p.is_empty())
            .collect();
        const SOURCE_COUNTS: [&str; 3] = [
            "known_entry_sources",
            "known_full_entry_sources",
            "entry_sources_first_seen_as_edit",
        ];
        kolejnosc.extend(SOURCE_COUNTS);
        for k in self.pola.keys() {
            // Data jest pokazywana w TYM SAMYM wierszu co wartość
            // `worst_day`, a nie jako oderwana metryka na końcu tabeli.
            if k != "worst_day_date"
                && k != "worst_market_day_date"
                && !kolejnosc.contains(&k.as_str())
            {
                kolejnosc.push(k.as_str());
            }
        }
        let pary: Vec<(&String, &serde_json::Value)> = kolejnosc
            .iter()
            .filter_map(|k| self.pola.get_key_value(*k))
            .collect();
        for (k, v) in pary {
            if k == "odrzuty" {
                if let Some(rejections) = v.as_object() {
                    for (reason, count) in rejections {
                        if let Some(count) = count.as_u64() {
                            w.push((
                                format!(
                                    "{} · {}",
                                    language.text("odrzucone", "rejected"),
                                    crate::language::label(language, reason)
                                ),
                                language.number(count as f64, 0),
                            ));
                        }
                    }
                }
                continue;
            }
            let mut txt = match v {
                serde_json::Value::Bool(b) => (if *b {
                    language.text("tak", "yes")
                } else {
                    language.text("nie", "no")
                })
                .to_string(),
                serde_json::Value::Number(n) => {
                    let f = n.as_f64().unwrap_or(0.0);
                    if !f.is_finite() {
                        "—".to_string()
                    } else if f.abs() >= 1e9 {
                        // Calmar przy obsunięciu bliskim zera wychodzi w
                        // kwadrylionach i rozpycha wiersz na pół ekranu.
                        // Taka liczba nie niesie nic poza „dzielone przez
                        // prawie zero", więc skracamy ją do wykładnika.
                        format!("{f:.2e}")
                    } else if f.fract() == 0.0 {
                        language.number(f, 0)
                    } else {
                        language.number(f, 2)
                    }
                }
                serde_json::Value::String(s) => s.clone(),
                // zagnieżdżone obiekty (np. `stat_sygnalow`) pomijamy — karta
                // ma być czytelna, a nie kompletna; od kompletu jest plik
                _ => continue,
            };
            if k.as_str() == "worst_day" || k.as_str() == "worst_market_day" {
                let date_key = if k == "worst_market_day" {
                    "worst_market_day_date"
                } else {
                    "worst_day_date"
                };
                if let Some(date) = self.pola.get(date_key).and_then(|v| v.as_str()) {
                    if !date.is_empty() {
                        txt.push_str("  ·  ");
                        txt.push_str(date);
                    }
                }
            }
            let label = KRYTERIA
                .iter()
                .find(|item| item.pole == k)
                .map(|item| item.etykieta)
                .unwrap_or(k);
            w.push((crate::language::label(language, label).to_string(), txt));
        }
        for key in SOURCE_COUNTS {
            if self.liczba(key).is_none() {
                w.push((crate::language::label(language, key).into(), "—".into()));
            }
        }
        if let Some(usage) = self.liczba("signal_use_pct") {
            w.push((
                crate::language::label(language, "% wykorzystanych sygnałów").into(),
                format!("{} %", language.number(usage, 2)),
            ));
        }
        w
    }
}

/// Komplet wyników jednego przemiatania, gotowy do pokazania.
#[derive(Default)]
pub struct Przesiane {
    pub wyniki: Vec<Wynik>,
    /// `true` means that the file is a quick-sweep screening artefact, not
    /// an exact backtest.  The rows remain sortable inside that screening
    /// stage, but must never be presented as coronation/release candidates.
    pub approximate: bool,
    /// Requested quick block size. `None` for the historical exact map.
    pub quick_tick_stride: Option<usize>,
    /// Explicit release gate copied from the wrapper. Exact legacy maps are
    /// eligible here; the ordinary reconciliation gates still apply later.
    pub coronation_eligible: bool,
    /// Human-readable warning supplied by the producer.
    pub warning: Option<String>,
    /// Znacznik czasu pliku przy ostatnim odczycie — po nim poznajemy, że
    /// doszło coś nowego i trzeba przeliczyć rangi.
    pub stempel: Option<std::time::SystemTime>,
}

/// Wczytuje `wyniki_czastkowe.json`, a gdy go nie ma — komplet `wyniki_*.json`.
///
/// Zwraca `None`, gdy w katalogu nie ma NIC do pokazania. Plik uszkodzony albo
/// przyłapany w połowie zapisu też daje `None`, nigdy błąd: okno ma wtedy po
/// prostu nie rysować przeglądarki i spróbować za sekundę.
pub fn wczytaj(katalog: &Path) -> Option<Przesiane> {
    let sciezka = sciezka_wynikow(katalog)?;
    let stempel = std::fs::metadata(&sciezka).and_then(|m| m.modified()).ok();
    let txt = std::fs::read_to_string(&sciezka).ok()?;
    let mut root: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&txt).ok()?;

    // Quick results deliberately use a wrapper so that a release adapter
    // expecting the historical flat map cannot consume them accidentally.
    // Unwrap only an explicitly labelled approximate document: a perfectly
    // legal legacy preset named `results` must remain a normal preset.
    let approximate = root
        .get("approximate")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let (mapa, quick_tick_stride, coronation_eligible, warning) = if approximate {
        let stride = root
            .get("quick_tick_stride")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| usize::try_from(n).ok());
        let eligible = root
            .get("coronation_eligible")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let warning = root
            .get("warning")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let results = root.remove("results")?.as_object()?.clone();
        (results, stride, eligible, warning)
    } else {
        (root, None, true, None)
    };

    let mut wyniki: Vec<Wynik> = mapa
        .into_iter()
        .filter_map(|(nazwa, v)| match v {
            serde_json::Value::Object(pola) => Some(Wynik {
                nazwa,
                pola,
                laczna: 0.0,
            }),
            _ => None,
        })
        .collect();
    if wyniki.is_empty() {
        return None;
    }
    ocena_laczna(&mut wyniki);
    Some(Przesiane {
        wyniki,
        approximate,
        quick_tick_stride,
        coronation_eligible,
        warning,
        stempel,
    })
}

/// Który plik czytać: cząstkowy ma pierwszeństwo, bo jest świeższy.
fn sciezka_wynikow(katalog: &Path) -> Option<std::path::PathBuf> {
    let czastkowe = katalog.join("wyniki_czastkowe.json");
    if czastkowe.is_file() {
        return Some(czastkowe);
    }
    // Przebieg skończony zapisuje `wyniki_<tag>.json`. Bierzemy NAJŚWIEŻSZY,
    // bo w jednym katalogu potrafi ich leżeć kilka.
    let mut kandydaci: Vec<(std::time::SystemTime, std::path::PathBuf)> =
        std::fs::read_dir(katalog)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("wyniki_") && n.ends_with(".json"))
                    .unwrap_or(false)
            })
            .filter_map(|p| {
                let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
                Some((t, p))
            })
            .collect();
    kandydaci.sort_by_key(|(t, _)| *t);
    kandydaci.pop().map(|(_, p)| p)
}

/// Ocena łączna: średnia RANG na osiach z [`OSIE_LACZNE`].
///
/// Rangi, nie wartości. Uśrednianie surowych liczb dałoby dyktaturę zysku —
/// on jeden chodzi w setkach tysięcy, a profit factor w jednościach, więc
/// każda inna oś zniknęłaby w zaokrągleniu. Ranga zrównuje jednostki: na
/// każdej osi najlepszy dostaje 1,0, najgorszy 0,0, reszta liniowo pomiędzy.
///
/// Oś, na której wszyscy są równi, jest pomijana — niczego nie rozstrzyga, a
/// dorzucona do średniej tylko rozmywałaby różnice na osiach, które coś mówią.
fn ocena_laczna(wyniki: &mut [Wynik]) {
    let n = wyniki.len();
    if n == 0 {
        return;
    }
    let mut suma = vec![0.0f64; n];
    let mut ile_osi = 0usize;

    for (pole, lepiej) in OSIE_LACZNE {
        // A legacy-only file keeps its historical ranking. If any market-day
        // result is present, absence stays unknown rather than borrowing a
        // different denominator from closed-trade days.
        let legacy = match *pole {
            "positive_market_days_pct" => Some("win_days_pct"),
            "worst_market_day" => Some("worst_day"),
            _ => None,
        };
        let pole = match legacy {
            Some(legacy) if !wyniki.iter().any(|w| w.liczba(pole).is_some()) => legacy,
            _ => *pole,
        };
        let wartosci: Vec<Option<f64>> = wyniki.iter().map(|w| w.liczba(pole)).collect();
        let obecne: Vec<f64> = wartosci.iter().flatten().copied().collect();
        if obecne.len() < 2 {
            continue;
        }
        let min = obecne.iter().copied().fold(f64::INFINITY, f64::min);
        let max = obecne.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if (max - min).abs() < 1e-12 {
            continue;
        }
        ile_osi += 1;
        for (i, v) in wartosci.iter().enumerate() {
            // brak wartości = najgorsza ranga; nieznane nie może udawać dobrego
            let r = match v {
                None => 0.0,
                Some(x) => {
                    let u = (x - min) / (max - min);
                    if *lepiej == Lepiej::Wiecej {
                        u
                    } else {
                        1.0 - u
                    }
                }
            };
            suma[i] += r;
        }
    }
    if ile_osi == 0 {
        return;
    }
    for (i, w) in wyniki.iter_mut().enumerate() {
        w.laczna = suma[i] / ile_osi as f64;
    }
}

/// Kolejność indeksów od najlepszego.
///
/// Wyniki zdyskwalifikowane — konto na zero albo zero transakcji — lądują NA
/// KOŃCU niezależnie od kryterium, a brak wartości tuż przed nimi.
pub fn kolejnosc(wyniki: &[Wynik], k: &Kryterium) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..wyniki.len()).collect();
    idx.sort_by(|&a, &b| {
        let (wa, wb) = (wyniki[a].zdyskwalifikowany(), wyniki[b].zdyskwalifikowany());
        if wa != wb {
            return wa.cmp(&wb); // false (=0) przed true (=1)
        }
        let (va, vb) = (wyniki[a].wg(k), wyniki[b].wg(k));
        match (va, vb) {
            (None, None) => wyniki[a].nazwa.cmp(&wyniki[b].nazwa),
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(x), Some(y)) => {
                let porz = if k.lepiej == Lepiej::Wiecej {
                    y.partial_cmp(&x)
                } else {
                    x.partial_cmp(&y)
                };
                porz.unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| wyniki[a].nazwa.cmp(&wyniki[b].nazwa))
            }
        }
    });
    idx
}

#[cfg(test)]
mod testy {
    use super::*;

    fn temp_dir(suffix: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("conduit-przesiane-{}-{suffix}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn w(nazwa: &str, zysk: f64, dni: f64, wysadzony: bool) -> Wynik {
        let mut pola = serde_json::Map::new();
        pola.insert("total_profit".into(), serde_json::json!(zysk));
        pola.insert("win_days_pct".into(), serde_json::json!(dni));
        pola.insert("blown".into(), serde_json::json!(wysadzony));
        // domyślnie przebieg HANDLOWAŁ — inaczej każdy byłby zdyskwalifikowany
        pola.insert("trades".into(), serde_json::json!(100));
        Wynik {
            nazwa: nazwa.into(),
            pola,
            laczna: 0.0,
        }
    }

    fn kryt(pole: &'static str) -> Kryterium {
        Kryterium {
            etykieta: "",
            pole,
            lepiej: Lepiej::Wiecej,
            format: Format::Dolary,
        }
    }

    #[test]
    fn sortuje_po_zysku_malejaco() {
        let v = vec![
            w("a", 10.0, 50.0, false),
            w("b", 30.0, 50.0, false),
            w("c", 20.0, 50.0, false),
        ];
        let k = kolejnosc(&v, &kryt("total_profit"));
        assert_eq!(v[k[0]].nazwa, "b");
        assert_eq!(v[k[2]].nazwa, "a");
    }

    #[test]
    fn wyniki_rynkowe_nie_pozyczaja_mianownika_legacy() {
        let mut a = w("rynkowy-80", 1.0, 100.0, false);
        a.pola
            .insert("positive_market_days_pct".into(), serde_json::json!(80.0));
        a.pola
            .insert("worst_market_day".into(), serde_json::json!(-20.0));
        let mut b = w("rynkowy-90", 1.0, 10.0, false);
        b.pola
            .insert("positive_market_days_pct".into(), serde_json::json!(90.0));
        b.pola
            .insert("worst_market_day".into(), serde_json::json!(-10.0));
        let legacy = w("legacy-100", 1.0, 100.0, false);
        assert_eq!(legacy.liczba("positive_market_days_pct"), None);
        let mut v = vec![a, b, legacy];
        ocena_laczna(&mut v);
        assert!(v[1].laczna > v[0].laczna);
        assert!(v[1].laczna > v[2].laczna);
    }

    #[test]
    fn wykorzystanie_sygnalow_wymaga_obu_licznikow_i_dodatniego_mianownika() {
        let mut row = w("pokrycie", 1.0, 80.0, false);
        assert_eq!(row.liczba("signal_use_pct"), None);
        row.pola
            .insert("signals_taken".into(), serde_json::json!(120));
        assert_eq!(row.liczba("signal_use_pct"), None);
        row.pola
            .insert("signals_seen".into(), serde_json::json!(200));
        assert_eq!(row.liczba("signal_use_pct"), Some(60.0));
        row.pola.insert("signals_seen".into(), serde_json::json!(0));
        assert_eq!(row.liczba("signal_use_pct"), None);
    }

    #[test]
    fn archived_source_counts_are_unknown_and_nested_rejections_are_visible() {
        let mut row = w("history", 1.0, 80.0, false);
        let before = row.wiersze_w_jezyku(crate::language::Language::En);
        assert!(before.contains(&("known entry sources".into(), "—".into())));
        row.pola
            .insert("known_entry_sources".into(), serde_json::json!(100));
        row.pola.insert(
            "entry_sources_first_seen_as_edit".into(),
            serde_json::json!(7),
        );
        row.pola.insert(
            "odrzuty".into(),
            serde_json::json!({"EditOrphan":7,"BudgetStop":4}),
        );
        let after = row.wiersze_w_jezyku(crate::language::Language::En);
        assert!(after.contains(&("known entry sources".into(), "100".into())));
        assert!(after.contains(&("known full entry sources".into(), "—".into())));
        assert!(after.contains(&("rejected · edit without a known entry".into(), "7".into())));
        assert!(after.contains(&("rejected · BudgetStop".into(), "4".into())));
        assert!(!after.contains(&("known entry sources".into(), "—".into())));
    }

    /// Sedno reguły: przebieg z najwyższym zyskiem, ale wyzerowanym kontem,
    /// nie może wyjść na czoło. Konto na zero jest końcem gry, nie wynikiem.
    #[test]
    fn wysadzony_zawsze_na_koncu() {
        let v = vec![w("trup", 999.0, 90.0, true), w("zywy", 1.0, 10.0, false)];
        let k = kolejnosc(&v, &kryt("total_profit"));
        assert_eq!(v[k[0]].nazwa, "zywy");
        assert_eq!(v[k[1]].nazwa, "trup");
    }

    /// Przebieg bez transakcji ma zerowe obsunięcie i wygrałby każdą oś
    /// ryzyka. Nie jest strategią bez ryzyka, tylko brakiem strategii.
    #[test]
    fn zero_transakcji_nie_wygrywa_ryzyka() {
        let mut pusty = w("pusty", 0.0, 0.0, false);
        pusty.pola.insert("trades".into(), serde_json::json!(0));
        pusty
            .pola
            .insert("max_dd_pct".into(), serde_json::json!(0.0));
        let mut prawdziwy = w("prawdziwy", 5000.0, 70.0, false);
        prawdziwy
            .pola
            .insert("trades".into(), serde_json::json!(900));
        prawdziwy
            .pola
            .insert("max_dd_pct".into(), serde_json::json!(31.0));
        let v = vec![pusty, prawdziwy];
        let k = Kryterium {
            etykieta: "",
            pole: "max_dd_pct",
            lepiej: Lepiej::Mniej,
            format: Format::Procent,
        };
        let kol = kolejnosc(&v, &k);
        assert_eq!(
            v[kol[0]].nazwa, "prawdziwy",
            "pusty przebieg wyszedł na czoło"
        );
    }

    #[test]
    fn brak_pola_ustepuje_wartosci() {
        let mut bez = w("bez", 0.0, 0.0, false);
        bez.pola.remove("total_profit");
        let v = vec![bez, w("z", -500.0, 0.0, false)];
        let k = kolejnosc(&v, &kryt("total_profit"));
        assert_eq!(v[k[0]].nazwa, "z", "nawet strata bije brak pomiaru");
    }

    /// Ocena łączna ma NIE być dyktaturą zysku.
    ///
    /// Skrajny zarabia więcej, ale przegrywa na dwóch pozostałych osiach i to
    /// ma przesądzić. Uwaga na czytanie tego testu: przy DWÓCH osiach, gdzie
    /// każdy wygrywa jedną, wychodzi remis — i słusznie, bo to naprawdę jest
    /// remis. Dopiero trzecia oś rozstrzyga, a rozstrzyga na korzyść tego,
    /// który jest równy, nie tego, który ma większą liczbę na czele.
    #[test]
    fn zysk_nie_dominuje_oceny_lacznej() {
        let mut skrajny = w("skrajny", 1_000_000.0, 10.0, false);
        skrajny
            .pola
            .insert("worst_day".into(), serde_json::json!(-50_000.0));
        let mut rowny = w("rowny", 900_000.0, 100.0, false);
        rowny
            .pola
            .insert("worst_day".into(), serde_json::json!(-100.0));
        let mut v = vec![skrajny, rowny];
        ocena_laczna(&mut v);
        assert!(
            v[1].laczna > v[0].laczna,
            "równy {:.3} miał pobić skrajnego {:.3}",
            v[1].laczna,
            v[0].laczna
        );
    }

    /// Remis jest remisem: dwie osie, każdy wygrywa jedną.
    #[test]
    fn dwie_osie_po_jednej_daja_remis() {
        let mut v = vec![
            w("zyskowny", 1_000_000.0, 10.0, false),
            w("rowny", 900_000.0, 100.0, false),
        ];
        ocena_laczna(&mut v);
        assert!(
            (v[0].laczna - v[1].laczna).abs() < 1e-9,
            "{} vs {}",
            v[0].laczna,
            v[1].laczna
        );
    }

    #[test]
    fn mniej_znaczy_lepiej_dziala() {
        let mut a = w("plytki", 0.0, 0.0, false);
        a.pola.insert("max_dd_pct".into(), serde_json::json!(5.0));
        let mut b = w("gleboki", 0.0, 0.0, false);
        b.pola.insert("max_dd_pct".into(), serde_json::json!(80.0));
        let v = vec![b, a];
        let k = Kryterium {
            etykieta: "",
            pole: "max_dd_pct",
            lepiej: Lepiej::Mniej,
            format: Format::Procent,
        };
        let kol = kolejnosc(&v, &k);
        assert_eq!(v[kol[0]].nazwa, "plytki");
    }

    #[test]
    fn wczytuje_legacy_flat_map_bez_zmiany_kontraktu() {
        let dir = temp_dir("legacy");
        std::fs::write(
            dir.join("wyniki_czastkowe.json"),
            r#"{"alpha":{"total_profit":12.5,"trades":4,"blown":false}}"#,
        )
        .unwrap();

        let result = wczytaj(&dir).expect("legacy result");
        assert_eq!(result.wyniki.len(), 1);
        assert_eq!(result.wyniki[0].nazwa, "alpha");
        assert_eq!(result.wyniki[0].liczba("total_profit"), Some(12.5));
        assert!(!result.approximate);
        assert_eq!(result.quick_tick_stride, None);
        assert!(result.coronation_eligible);
        assert!(result.warning.is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wczytuje_quick_wrapper_i_nie_robi_falszywego_presetu_results() {
        let dir = temp_dir("quick-wrapper");
        std::fs::write(
            dir.join("wyniki_czastkowe.json"),
            r#"{
                "schema":"conduit.quick-sweep-partial.v1",
                "approximate":true,
                "coronation_eligible":false,
                "quick_tick_stride":20,
                "warning":"APPROXIMATE SCREENING ONLY",
                "results":{
                    "alpha":{"total_profit":12.5,"trades":4,"blown":false},
                    "beta":{"total_profit":7.0,"trades":3,"blown":false}
                }
            }"#,
        )
        .unwrap();

        let result = wczytaj(&dir).expect("quick result");
        assert_eq!(result.wyniki.len(), 2);
        assert!(result.wyniki.iter().any(|row| row.nazwa == "alpha"));
        assert!(result.wyniki.iter().any(|row| row.nazwa == "beta"));
        assert!(!result.wyniki.iter().any(|row| row.nazwa == "results"));
        assert!(result.approximate);
        assert_eq!(result.quick_tick_stride, Some(20));
        assert!(!result.coronation_eligible);
        assert_eq!(
            result.warning.as_deref(),
            Some("APPROXIMATE SCREENING ONLY")
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
