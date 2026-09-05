
pub mod budzet;
pub mod cechy;
pub mod cien;
pub mod glebokosc;
pub mod margines;
pub mod oczy;
pub mod polityka;
pub mod portfel;
pub mod przebieg;
pub mod rama;
pub mod we;
pub mod wejscie;

// ============================================================================
//  DROBNA STATYSTYKA — wspólna dla raportów E0
// ============================================================================
//
// Własna, a nie z biblioteki, z jednego powodu: raporty E0 mają być
// odtwarzalne co do ostatniej cyfry między przebiegami i między maszynami.
// Percentyl „metodą najbliższej rangi" nie interpoluje, więc nie zależy od
// kolejności sumowania ani od trybu zaokrąglania.

pub fn percentyl(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p = p.clamp(0.0, 100.0);
    let idx = ((p / 100.0) * (v.len() as f64 - 1.0)).round() as usize;
    v[idx.min(v.len() - 1)]
}

/// Mediana — [`percentyl`] przy 50 %.
pub fn mediana(v: &mut [f64]) -> f64 {
    percentyl(v, 50.0)
}

/// Średnia arytmetyczna. Pusta próbka → `f64::NAN`.
pub fn srednia(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

#[cfg(test)]
mod higiena {
    /// GRANICA 1 — RDZEŃ DECYZYJNY NIE ZNA BROKERA, ZEGARA ANI LOSOWOŚCI.
    ///
    /// Skan źródeł, nie przegląd kodu.
    ///
    /// # Zakres na etapie E0
    ///
    /// Skanowane są WYŁĄCZNIE pliki rdzenia decyzyjnego. `we.rs` i binarki
    /// obserwacyjne są z niego wyjęte świadomie: ich zadaniem jest czytać
    /// zrzuty z dysku, więc `File::open` i `HashMap` są tam poprawne. Zakres
    /// rośnie razem z crate'em — w E2, gdy powstaną `wejscie`, `domena`,
    /// `straz`, `macierz`, `arbiter`, `polityka` i `ksiega`, wszystkie
    /// dochodzą do tej listy. Lista jest WYLICZONA, a nie „wszystko oprócz",
    /// żeby nowy plik rdzenia trzeba było do niej dopisać ŚWIADOMIE.
    const RDZEN: &[&str] = &["margines.rs", "rama.rs", "budzet.rs"];

    /// `HashMap`/`HashSet`/`sort_by(` są tu nie dlatego, że są złe, tylko
    /// dlatego, że są znanymi drogami do niedeterminizmu kolejności —
    /// a determinizm jest w rdzeniu doktryną, nie preferencją.
    const ZAKAZANE: &[&str] = &[
        "Broker",
        "open_market",
        "place_pending",
        "modify_position",
        "modify_pending",
        "close_position",
        "close_partial",
        "cancel_pending",
        "SystemTime",
        "Instant",
        "rand::",
        "thread_rng",
        "env::var",
        "File::open",
        "HashMap",
        "HashSet",
        "sort_by(",
        "partial_cmp",
    ];

    #[test]
    fn higiena_rdzenia_skan_zrodel() {
        let katalog = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut trafienia: Vec<String> = Vec::new();
        for nazwa in RDZEN {
            let p = katalog.join(nazwa);
            let Ok(tresc) = std::fs::read_to_string(&p) else {
                continue; // plik jeszcze nie powstał — dopisany do listy z wyprzedzeniem
            };
            for (nr, linia) in tresc.lines().enumerate() {
                // Komentarze WOLNO — cały sens tego crate'a opisuje się przez
                // nazwanie tego, czego w nim nie ma.
                if linia.trim_start().starts_with("//") {
                    continue;
                }
                for z in ZAKAZANE {
                    if linia.contains(z) {
                        trafienia.push(format!("{nazwa}:{}: „{z}” w: {}", nr + 1, linia.trim()));
                    }
                }
            }
        }
        assert!(
            trafienia.is_empty(),
            "rdzeń mózgu dotknął warstwy, której nie wolno mu znać:\n{}",
            trafienia.join("\n")
        );
    }

    #[test]
    fn percentyl_najblizszej_rangi_bez_interpolacji() {
        let mut v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(super::percentyl(&mut v, 0.0), 1.0);
        assert_eq!(super::percentyl(&mut v, 50.0), 3.0);
        assert_eq!(super::percentyl(&mut v, 100.0), 5.0);
        assert_eq!(super::mediana(&mut v), 3.0);
        assert!((super::srednia(&v) - 3.0).abs() < 1e-12);
        assert!(super::percentyl(&mut Vec::new(), 50.0).is_nan());
        assert!(super::srednia(&[]).is_nan());
    }
}
