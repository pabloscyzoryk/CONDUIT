
use conduit_server::coalesce::{Section, Sections};
use conduit_server::{alllogs, ui, ServerConfig};

fn stan(tag: &str) -> conduit_server::StateHandle {
    let mut dir = std::env::temp_dir();
    dir.push(format!("conduit-alllogs-{tag}-{}", std::process::id()));
    let cfg = ServerConfig {
        workspace: dir,
        ..Default::default()
    };
    conduit_server::bootstrap(&cfg, conduit_server::default_auth()).unwrap()
}

#[test]
fn sekcja2_drukuje_ostrzezenia_bramki_i_martwe_pulapy() {
    let st = stan("bramka");
    st.update(
        Sections::one(Section::Settings),
        |s: &mut ui::UiSnapshot| {
            // S3: straż ekspozycji bez zgody na domykanie pozycji.
            s.settings["expo_cap_pct"] = serde_json::json!(80.0);
            s.settings["expo_cap_close"] = serde_json::json!(false);
            // D27: pułap, którego silnik nie czyta, na AKTYWNYM łańcuchu.
            let aktywny = s.lancuchy.aktywny.clone();
            if let Some(l) = s.lancuchy.lista.iter_mut().find(|l| l.nazwa == aktywny) {
                l.pulapy.cel_dnia_zamyka = true;
            }
        },
    );

    let txt = alllogs::zbuduj(&st);
    assert!(
        txt.contains("USTAWIENIA WEWNĘTRZNIE SPRZECZNE"),
        "brak podsekcji w sekcji 2"
    );
    assert!(
        txt.contains("S3-straz-nic-nie-zamyka"),
        "ostrzeżenie o ustawieniach nie dojechało"
    );
    assert!(
        txt.contains("D27-pulap-martwy") && txt.contains("cel_dnia_zamyka"),
        "martwy pułap aktywnego łańcucha nie dojechał — sprawdź klucz `lancuchy` w migawce"
    );
}

/// Zrzut ze ZDROWEJ konfiguracji ma pokazywać „(brak)", a nie znikającą
/// podsekcję: czytający musi widzieć, że sprawdzenie się ODBYŁO.
#[test]
fn sekcja2_pokazuje_brak_gdy_nie_ma_o_czym_mowic() {
    let st = stan("cisza");
    let txt = alllogs::zbuduj(&st);
    let ogon: String = txt
        .lines()
        .skip_while(|l| !l.contains("USTAWIENIA WEWNĘTRZNIE SPRZECZNE"))
        .take(4)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!ogon.is_empty(), "brak podsekcji w sekcji 2");
    assert!(
        ogon.contains("(brak)"),
        "zdrowa konfiguracja, a podsekcja nie mówi „(brak)”:\n{ogon}"
    );
}
