fn main() {
    let kat = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("uzycie: sprawdz_ustawienia <katalog_roboczy_bota>");
        std::process::exit(2);
    });
    let ws = conduit_server::store::Workspace::new(std::path::Path::new(&kat));
    let (doc, blad) = ws.load_settings_checked();
    let ile = doc.settings.as_object().map(|o| o.len()).unwrap_or(0);
    println!("katalog        : {kat}");
    println!("ustawien        : {ile}");
    println!("preset_id       : {}", doc.preset_id);
    println!("tryb            : {:?}", doc.mode);
    match blad {
        None => println!("WYNIK           : WCZYTANE BEZ ZASTRZEZEN (halt by NIE padl)"),
        Some(e) => println!("WYNIK           : ZASTRZEZENIE -> HALT\n  powod: {e}"),
    }
}
