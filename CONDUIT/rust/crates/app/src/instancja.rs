
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

/// Ile czekamy na odpowiedź instancji, która już działa. Rozmowa idzie po
/// pętli zwrotnej, więc sekunda to i tak wielokrotność potrzebnego czasu;
/// dłuższe czekanie zamieniłoby „program się nie uruchomił" w „program się
/// zawiesił", a to jest zmiana na gorsze.
const CZAS_ODPOWIEDZI: Duration = Duration::from_millis(1200);

/// Co zastaliśmy pod adresem, na którym mamy nasłuchiwać.
#[derive(Debug)]
pub enum Zastane {
    /// port wolny — startujemy normalnie
    Wolny,
    /// port trzyma DRUGI CONDUIT; w środku jego adres i wersja
    Nasz { url: String, wersja: String },
    /// port trzyma coś obcego (albo coś, co nie chce rozmawiać)
    Obcy { powod: String },
}

/// Sprawdza, czy da się zająć adres, a jeśli nie — kto go trzyma.
pub fn sprawdz(bind: SocketAddr) -> Zastane {
    // Najpierw najtańszy test: czy w ogóle da się zająć adres. Gniazdo
    // zwalniamy od razu — to tylko pytanie, nie rezerwacja. Wyścig z innym
    // procesem jest teoretycznie możliwy, ale kończy się dokładnie tym samym
    // błędem, co dziś, i to na tej samej linijce co zawsze.
    match TcpListener::bind(bind) {
        Ok(l) => {
            drop(l);
            return Zastane::Wolny;
        }
        Err(e) if e.kind() != std::io::ErrorKind::AddrInUse => {
            return Zastane::Obcy {
                powod: format!("{e}"),
            };
        }
        Err(_) => {}
    }

    // Adres nasłuchu bywa „dowolny" (0.0.0.0) — wtedy pytamy pętlę zwrotną,
    // bo pod 0.0.0.0 nie da się połączyć.
    let cel = if bind.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), bind.port())
    } else {
        bind
    };

    match zapytaj_health(cel) {
        Ok(tresc) if tresc.contains("\"app\":\"conduit\"") => Zastane::Nasz {
            url: format!("http://{cel}"),
            wersja: wytnij(&tresc, "\"version\":\"").unwrap_or_else(|| "?".into()),
        },
        Ok(_) => Zastane::Obcy {
            powod: "odpowiada serwer HTTP, ale to nie jest CONDUIT".into(),
        },
        Err(e) => Zastane::Obcy { powod: e },
    }
}

/// Pierwszy wolny port w okolicy — do podpowiedzi, gdy port trzyma ktoś obcy.
pub fn wolny_port_obok(bind: SocketAddr) -> Option<u16> {
    (bind.port() + 1..bind.port().saturating_add(20))
        .find(|p| TcpListener::bind(SocketAddr::new(bind.ip(), *p)).is_ok())
}

fn zapytaj_health(addr: SocketAddr) -> Result<String, String> {
    let mut s = TcpStream::connect_timeout(&addr, CZAS_ODPOWIEDZI)
        .map_err(|e| format!("nie udało się połączyć: {e}"))?;
    let _ = s.set_read_timeout(Some(CZAS_ODPOWIEDZI));
    let _ = s.set_write_timeout(Some(CZAS_ODPOWIEDZI));
    let zadanie = format!(
        "GET /api/health HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: conduit-start\r\nConnection: close\r\n\r\n"
    );
    s.write_all(zadanie.as_bytes())
        .map_err(|e| format!("nie udało się wysłać pytania: {e}"))?;
    let mut bufor = Vec::new();
    // Odpowiedź `/api/health` to kilkadziesiąt bajtów. Limit jest po to, żeby
    // rozmowa z czymś, co gada bez końca, nie zawiesiła startu programu.
    let mut kawalek = [0u8; 2048];
    loop {
        match s.read(&mut kawalek) {
            Ok(0) => break,
            Ok(n) => {
                bufor.extend_from_slice(&kawalek[..n]);
                if bufor.len() > 16 * 1024 {
                    break;
                }
            }
            Err(e) => {
                if bufor.is_empty() {
                    return Err(format!("brak odpowiedzi: {e}"));
                }
                break;
            }
        }
    }
    Ok(String::from_utf8_lossy(&bufor).to_string())
}

/// Wycina wartość napisową spod klucza JSON, bez wciągania parsera.
fn wytnij(tresc: &str, klucz: &str) -> Option<String> {
    let i = tresc.find(klucz)? + klucz.len();
    let reszta = &tresc[i..];
    let j = reszta.find('"')?;
    Some(reszta[..j].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wolny_port_jest_rozpoznawany() {
        // port 0 = „przydziel dowolny wolny" — zawsze da się zająć
        let a: SocketAddr = ([127, 0, 0, 1], 0).into();
        assert!(matches!(sprawdz(a), Zastane::Wolny));
    }

    #[test]
    fn zajety_port_bez_health_to_obcy() {
        let l = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let a = l.local_addr().unwrap();
        // nasłuchuje, ale nie odpowiada na HTTP — czyli nie jest nasz
        match sprawdz(a) {
            Zastane::Obcy { .. } => {}
            inne => panic!("zły wynik: {inne:?}"),
        }
    }

    #[test]
    fn wersja_wyciagana_z_odpowiedzi() {
        let t = r#"{"app":"conduit","ok":true,"version":"1.0.0"}"#;
        assert_eq!(wytnij(t, "\"version\":\"").as_deref(), Some("1.0.0"));
        assert_eq!(wytnij(t, "\"nie-ma\":\""), None);
    }

    #[test]
    fn podpowiedz_portu_omija_zajety() {
        let l = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let a = l.local_addr().unwrap();
        let p = wolny_port_obok(a).expect("w okolicy musi być wolny port");
        assert!(p > a.port());
    }
}
