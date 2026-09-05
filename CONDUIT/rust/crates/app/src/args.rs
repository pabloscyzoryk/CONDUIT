
use std::net::IpAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Args {
    pub host: IpAddr,
    pub port: u16,
    /// katalog z `settings.json`, `presets/`, `backup_memory/`
    pub data_dir: Option<PathBuf>,
    /// katalog ze zbudowanym interfejsem (nadpisuje zasoby wbudowane)
    pub web_dir: Option<PathBuf>,
    pub headless: bool,
    /// otwórz stronę w domyślnej przeglądarce po starcie
    pub open: bool,
    /// tryb demo: bot gra na wirtualnym brokerze (ten sam silnik co backtest)
    pub demo: bool,
    /// otwórz okno od razu na widoku „Laboratorium" (backtesty i trening AI)
    pub lab: bool,
    pub start_balance: f64,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            host: [127, 0, 0, 1].into(),
            port: 8787,
            data_dir: None,
            web_dir: None,
            headless: false,
            open: false,
            demo: false,
            lab: false,
            start_balance: 2000.0,
        }
    }
}

pub enum Parsed {
    Run(Box<Args>),
    Help,
    Version,
}

pub fn parse<I: IntoIterator<Item = String>>(it: I) -> Result<Parsed, String> {
    let mut a = Args::default();
    let mut args = it.into_iter().skip(1).peekable();

    while let Some(arg) = args.next() {
        let mut wartosc = |nazwa: &str| -> Result<String, String> {
            args.next()
                .ok_or_else(|| format!("opcja {nazwa} wymaga wartości"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-V" | "--version" => return Ok(Parsed::Version),
            "--headless" => a.headless = true,
            "--open" => a.open = true,
            "--demo" => a.demo = true,
            "--lab" => a.lab = true,
            "--host" => {
                let v = wartosc("--host")?;
                a.host = v.parse().map_err(|_| format!("zły adres: {v}"))?;
            }
            "--port" => {
                let v = wartosc("--port")?;
                a.port = v.parse().map_err(|_| format!("zły port: {v}"))?;
            }
            "--data-dir" => a.data_dir = Some(PathBuf::from(wartosc("--data-dir")?)),
            "--web-dir" => a.web_dir = Some(PathBuf::from(wartosc("--web-dir")?)),
            "--balance" => {
                let v = wartosc("--balance")?;
                a.start_balance = v.parse().map_err(|_| format!("zła kwota: {v}"))?;
            }
            inne => return Err(format!("nieznana opcja: {inne}")),
        }
    }
    Ok(Parsed::Run(Box::new(a)))
}

pub const HELP: &str = "\
CONDUIT — Telegram → MetaTrader 5

  conduit.exe [OPCJE]

Bez opcji: uruchamia serwer i otwiera okno natywne.

  --headless           tylko serwer, bez okna (VPS, usługa Windows)
  --host <adres>       adres nasłuchu           (domyślnie 127.0.0.1)
  --port <numer>       port                     (domyślnie 8787)
  --data-dir <kat>     katalog konfiguracji     (domyślnie obok exe)
  --web-dir <kat>      interfejs z dysku zamiast wbudowanego
  --open               otwórz stronę w przeglądarce po starcie
  --demo               TRYB DEMO: bot gra na wirtualnym brokerze, w czasie
                       rzeczywistym, tym samym silnikiem co backtest. Ceny
                       z pliku ticków albo z generatora; konfiguracja
                       w demo.json i w widoku „Tryb demo”
  --lab                otwórz okno na widoku „Laboratorium” — backtesty
                       i trening modelu AI z postępem na żywo
  --balance <kwota>    saldo startowe, gdy nie ma zapisanego stanu
  -h, --help           ta pomoc
  -V, --version        wersja

Pliki czytane z katalogu konfiguracji:
  settings.json  smtp.json  channels.json  presets/*.json
Stan zapisywany do:
  backup_memory/latest.json (+ kopie rotacyjne)
";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &[&str]) -> Result<Args, String> {
        let mut v = vec!["conduit.exe".to_string()];
        v.extend(s.iter().map(|x| x.to_string()));
        match parse(v)? {
            Parsed::Run(a) => Ok(*a),
            _ => Err("pomoc/wersja".into()),
        }
    }

    #[test]
    fn domyslne_wartosci() {
        let a = p(&[]).unwrap();
        assert_eq!(a.port, 8787);
        assert!(!a.headless);
        assert_eq!(a.host.to_string(), "127.0.0.1");
    }

    #[test]
    fn tryb_headless_i_port() {
        let a = p(&["--headless", "--port", "9000"]).unwrap();
        assert!(a.headless);
        assert_eq!(a.port, 9000);
    }

    #[test]
    fn katalogi_sa_sciezkami() {
        let a = p(&["--data-dir", "C:/dane", "--web-dir", "../dist"]).unwrap();
        assert_eq!(a.data_dir.unwrap().to_string_lossy(), "C:/dane");
        assert_eq!(a.web_dir.unwrap().to_string_lossy(), "../dist");
    }

    #[test]
    fn zla_opcja_konczy_sie_bledem_a_nie_cichym_pominieciem() {
        assert!(p(&["--nie-ma-takiej"]).is_err());
        assert!(p(&["--port"]).is_err());
        assert!(p(&["--port", "abc"]).is_err());
    }

    #[test]
    fn tryb_laboratorium_jest_flaga_bez_wartosci() {
        assert!(p(&["--lab"]).unwrap().lab);
        assert!(
            !p(&[]).unwrap().lab,
            "domyślnie okno otwiera się na pulpicie"
        );
        // da się łączyć z resztą — np. laboratorium w przeglądarce
        let a = p(&["--lab", "--open", "--port", "8790"]).unwrap();
        assert!(a.lab && a.open && a.port == 8790);
    }

    #[test]
    fn pomoc_jest_rozpoznawana() {
        let v = vec!["conduit.exe".into(), "--help".into()];
        assert!(matches!(parse(v), Ok(Parsed::Help)));
    }
}
