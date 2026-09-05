//! `loganaliza` — analizator dziennika zdarzeń bota.
//!
//! ```text
//!   loganaliza <plik.jsonl | katalog> [--json raport.json] [--csv-dni dni.csv]
//!              [--csv-krzywa equity.csv] [--dzien YYYY-MM-DD] [--cicho]
//! ```
//!
//! Bez argumentów szuka dziennika w `logs/journal` obok programu.
//!
//! Odpowiada na cztery pytania, na które log poprzedniego bota odpowiedzieć
//! nie potrafił: ile zarobiono danego dnia, jak wyglądała krzywa kapitału,
//! jak rozkładają się powody zamknięć — i **ile pieniędzy zostawiono na
//! stole**, z podziałem na reguły zarządzania, które to zrobiły.

use conduit_backtest::loganaliza::{tekst, wczytaj_sciezke_dnia, Report};
use std::path::PathBuf;

fn pomoc() {
    println!(
        "loganaliza — analiza dziennika zdarzeń (.jsonl)\n\
         \n\
         UŻYCIE:\n\
         \x20 loganaliza <plik.jsonl | katalog> [opcje]\n\
         \n\
         OPCJE:\n\
         \x20 --json <plik>        zapisz pełny raport w JSON (zapis atomowy)\n\
         \x20 --csv-dni <plik>     wynik dzienny w CSV\n\
         \x20 --csv-krzywa <plik>  krzywa kapitału w CSV\n\
         \x20 --dzien <YYYY-MM-DD> licz tylko tę dobę handlową\n\
         \x20 --cicho              nie drukuj tabel na ekran\n"
    );
}

fn zapisz(p: &std::path::Path, tresc: &str) -> std::io::Result<()> {
    // ten sam schemat co w serwerze: zapisz obok, potem podmień nazwę
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, tresc.as_bytes())?;
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
    std::fs::rename(&tmp, p)
}

fn csv_dni(r: &Report) -> String {
    let mut s = String::from(
        "dzien;netto;transakcje;trafne;stratne;wolumen;max_obsuniecie;equity_koniec;na_stole;wiadomosci;odrzucenia\n",
    );
    for d in &r.days {
        s.push_str(&format!(
            "{};{:.2};{};{};{};{:.2};{:.2};{:.2};{:.2};{};{}\n",
            d.day,
            d.realized,
            d.trades,
            d.wins,
            d.losses,
            d.volume,
            d.max_dd,
            d.equity_close,
            d.left_on_table,
            d.signals,
            d.rejects
        ));
    }
    s
}

fn csv_krzywa(r: &Report) -> String {
    let mut s = String::from("ts_broker_ms;equity\n");
    for (t, e) in &r.equity_curve {
        s.push_str(&format!("{t};{e:.2}\n"));
    }
    s
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        pomoc();
        return;
    }

    let mut sciezka: Option<PathBuf> = None;
    let mut out_json: Option<PathBuf> = None;
    let mut out_dni: Option<PathBuf> = None;
    let mut out_krzywa: Option<PathBuf> = None;
    let mut tylko_dzien: Option<String> = None;
    let mut cicho = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                i += 1;
                out_json = args.get(i).map(PathBuf::from);
            }
            "--csv-dni" => {
                i += 1;
                out_dni = args.get(i).map(PathBuf::from);
            }
            "--csv-krzywa" => {
                i += 1;
                out_krzywa = args.get(i).map(PathBuf::from);
            }
            "--dzien" => {
                i += 1;
                tylko_dzien = args.get(i).cloned();
            }
            "--cicho" => cicho = true,
            x if !x.starts_with("--") => sciezka = Some(PathBuf::from(x)),
            x => {
                eprintln!("nieznana opcja: {x}");
                pomoc();
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let p = sciezka.unwrap_or_else(|| PathBuf::from("logs/journal"));
    if !p.exists() {
        eprintln!(
            "nie ma czego analizować: {} nie istnieje.\n\
             Wskaż plik .jsonl albo katalog z dziennikiem.",
            p.display()
        );
        std::process::exit(1);
    }

    // Filtr doby działa PRZY WCZYTYWANIU, żeby sumy, obsunięcia i „na stole"
    // dotyczyły tego samego dnia co tabela — a nie całego pliku.
    let a = match wczytaj_sciezke_dnia(&p, tylko_dzien.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("błąd odczytu {}: {e}", p.display());
            std::process::exit(1);
        }
    };
    let r = a.raport();

    if let Some(d) = &tylko_dzien {
        if r.days.is_empty() {
            eprintln!("w dzienniku nie ma doby {d}");
            std::process::exit(1);
        }
    }

    if !cicho {
        print!("{}", tekst(&r));
    }

    if let Some(o) = out_json {
        let tresc = serde_json::to_string_pretty(&r).unwrap_or_default();
        match zapisz(&o, &tresc) {
            Ok(()) => eprintln!("raport JSON → {}", o.display()),
            Err(e) => eprintln!("nie udało się zapisać {}: {e}", o.display()),
        }
    }
    if let Some(o) = out_dni {
        match zapisz(&o, &csv_dni(&r)) {
            Ok(()) => eprintln!("wynik dzienny → {}", o.display()),
            Err(e) => eprintln!("nie udało się zapisać {}: {e}", o.display()),
        }
    }
    if let Some(o) = out_krzywa {
        match zapisz(&o, &csv_krzywa(&r)) {
            Ok(()) => eprintln!("krzywa kapitału → {}", o.display()),
            Err(e) => eprintln!("nie udało się zapisać {}: {e}", o.display()),
        }
    }
}
