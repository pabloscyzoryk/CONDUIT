
use std::path::PathBuf;

fn korzen() -> PathBuf {
    // crates/mt5 → crates → rust → korzeń repozytorium
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

#[test]
fn obie_kopie_sidecara_sa_identyczne() {
    let zrodlo = korzen().join("rust/crates/mt5/sidecar/mt5_sidecar.py");
    let pakiet = korzen().join("PACKAGE/mt5_sidecar.py");

    let a = std::fs::read_to_string(&zrodlo)
        .unwrap_or_else(|e| panic!("brak źródła sidecara {}: {e}", zrodlo.display()));

    // Brak `PACKAGE/` nie jest błędem: drzewo bywa rozpakowane bez folderu
    // wydania (np. w CI liczącym wyłącznie bramkę parytetu). Testujemy
    // ROZJAZD, a nie obecność.
    let Ok(b) = std::fs::read_to_string(&pakiet) else {
        eprintln!("PACKAGE/mt5_sidecar.py nie istnieje — pomijam porównanie");
        return;
    };

    if a == b {
        return;
    }

    // Pokaż PIERWSZĄ różniącą się linię: „pliki się różnią" bez wskazania
    // miejsca zmusza do ręcznego diffa i zwykle kończy się zignorowaniem.
    let mut opis = String::new();
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            opis = format!(
                "pierwsza różnica w linii {}:\n  źródło:  {la}\n  PACKAGE: {lb}",
                i + 1
            );
            break;
        }
    }
    if opis.is_empty() {
        opis = format!(
            "jeden plik jest dłuższy: źródło {} linii, PACKAGE {} linii",
            a.lines().count(),
            b.lines().count()
        );
    }

    panic!(
        "KOPIE SIDECARA SIĘ ROZJECHAŁY.\n{opis}\n\n\
         Napraw kopiując źródło do wydania:\n  \
         cp rust/crates/mt5/sidecar/mt5_sidecar.py PACKAGE/mt5_sidecar.py\n\
         (nie odwrotnie — źródłem prawdy jest drzewo, nie folder wydania)"
    );
}

#[test]
fn obie_kopie_czytaja_account_credit() {
    for wzgledna in [
        "rust/crates/mt5/sidecar/mt5_sidecar.py",
        "PACKAGE/mt5_sidecar.py",
    ] {
        let p = korzen().join(wzgledna);
        let Ok(t) = std::fs::read_to_string(&p) else {
            continue;
        };
        assert!(
            t.contains("\"credit\""),
            "{wzgledna} nie zwraca pola `credit` — bot zobaczy konto z bonusem \
             jako większe, niż jest, i policzy lot od cudzych pieniędzy"
        );
    }
}
