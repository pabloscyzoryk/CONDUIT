
pub mod api;
pub mod eksport;
pub mod odczyt;
pub mod zapis;

use serde::{Deserialize, Serialize};

pub use api::{KanalInfo, Stan, TematInfo, Tryb};
pub use eksport::{eksportuj, PodsumowanieEksportu};
pub use odczyt::{
    czytaj, pliki_kroniki, rozpoznaj, statystyki, statystyki_z, Przerwa, Rozklad, Rozpoznanie,
    StatKanalu, Statystyki,
};
pub use zapis::{Decyzja, Kronika, Liczniki, Przychodzace};

/// Wersja schematu wiersza. Czytnik sprawdza ją, zanim cokolwiek policzy.
pub const SCHEMAT: u32 = 2;

/// Domyślna nazwa jednego, ciągłego pliku kroniki.
pub const PLIK_DOMYSLNY: &str = "kronika.jsonl";

pub const PLIK_PULPITU: &str = "conduit_kronika.jsonl";

/// Ścieżka domyślna: `%USERPROFILE%\Desktop\conduit_kronika.jsonl`.
///
/// `None`, gdy pulpitu nie ma (konto usługowe, serwer bez profilu, inny
/// system) — wołający ma wtedy zostać przy katalogu bota, a nie tworzyć
/// katalog „Desktop" w losowym miejscu.
///
/// OneDrive bierzemy pod uwagę świadomie: na Windows 11 z przekierowanym
/// profilem `%USERPROFILE%\Desktop` **nie istnieje**, a prawdziwy pulpit
/// leży w `%OneDrive%\Desktop`. Bez tego domyślna ścieżka wskazywałaby
/// katalog, którego użytkownik nigdy nie zobaczy.
pub fn pulpit() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    let kandydaci = [
        std::env::var_os("OneDrive").map(std::path::PathBuf::from),
        std::env::var_os("OneDriveConsumer").map(std::path::PathBuf::from),
        std::env::var_os("USERPROFILE").map(std::path::PathBuf::from),
        std::env::var_os("HOME").map(std::path::PathBuf::from),
    ];
    for k in kandydaci.into_iter().flatten() {
        for nazwa in ["Desktop", "Pulpit"] {
            let d = k.join(nazwa);
            if d.is_dir() {
                return Some(d);
            }
        }
    }
    None
}

/// Pełna domyślna ścieżka pliku kroniki na pulpicie.
pub fn sciezka_pulpitu() -> Option<std::path::PathBuf> {
    pulpit().map(|d| d.join(PLIK_PULPITU))
}

// ============================================================
//  WIERSZ
// ============================================================

/// Co się stało.
///
/// `Start` i `Stop` to **znaczniki sesji rejestratora**, nie wiadomości.
/// Bez nich dziura w pliku jest dwuznaczna: cisza na kanałach wygląda
/// identycznie jak wyłączony rejestrator, a to jest różnica między
/// „nic się nie działo" a „nie wiemy, co się działo".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rodzaj {
    /// pierwsza wersja wiadomości — tak, jak przyszła
    #[serde(alias = "received")]
    Nowa,
    /// autor poprawił treść; `edit_of` wskazuje oryginał
    #[serde(alias = "edited")]
    Edycja,
    /// wiadomość zniknęła z kanału
    #[serde(alias = "deleted")]
    Skasowana,
    /// rejestrator ruszył
    Start,
    /// rejestrator został zatrzymany uprzejmie
    Stop,
}

impl Rodzaj {
    pub fn as_str(self) -> &'static str {
        match self {
            Rodzaj::Nowa => "nowa",
            Rodzaj::Edycja => "edycja",
            Rodzaj::Skasowana => "skasowana",
            Rodzaj::Start => "start",
            Rodzaj::Stop => "stop",
        }
    }

    /// Czy ten wiersz opisuje wiadomość (a nie stan rejestratora).
    pub fn to_wiadomosc(self) -> bool {
        matches!(self, Rodzaj::Nowa | Rodzaj::Edycja | Rodzaj::Skasowana)
    }
}

/// Jeden wiersz kroniki — dokładnie to, co zobaczyliśmy, bez interpretacji.
///
/// Aliasy pól nie są ozdobą: czytnik przyjmuje **trzy** historyczne kształty
/// zapisu, żeby statystyka liczyła się także z danych zebranych, zanim ta
/// biblioteka powstała:
///
/// | źródło                                  | nazwy pól                          |
/// |-----------------------------------------|------------------------------------|
/// | ta wersja                               | `odebrano_ms`, `rodzaj`, `chat`…   |
/// | pierwsza `kronika.exe`                  | to samo, ale `ts_telegram` w **sekundach** |
/// | archiwum Conduita (`wiadomosci-*.jsonl`)| `received_at_ms`, `event`, `source_name`… |
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wpis {
    #[serde(default = "domyslny_schemat")]
    pub v: u32,
    /// numer w obrębie uruchomienia — porządkuje zdarzenia z tej samej milisekundy
    #[serde(default)]
    pub seq: u64,
    #[serde(alias = "event")]
    pub rodzaj: Rodzaj,

    /// **Nasz** czas odbioru. Jedyny znacznik, którego Telegram nie przepisuje,
    /// i jedyny, po którym wolno sortować: to on odtwarza kolejność, w jakiej
    /// bot naprawdę zobaczył zdarzenia.
    #[serde(alias = "received_at_ms")]
    pub odebrano_ms: i64,
    #[serde(default, alias = "received_at")]
    pub odebrano: String,

    /// Znacznik od Telegrama w milisekundach (`date` dla nowej, `edit_date`
    /// dla edycji). Przy edycji potrafi być STARSZY niż `odebrano_ms` — i to
    /// jest dokładnie powód, dla którego oba pola muszą istnieć osobno.
    #[serde(default, alias = "msg_ts_ms")]
    pub ts_telegram_ms: i64,

    #[serde(default)]
    pub chat_id: i64,
    #[serde(default, alias = "source_name")]
    pub chat: String,
    /// temat forum — osobne źródło, własne koszyki, własny format
    #[serde(default, alias = "topic_id", skip_serializing_if = "Option::is_none")]
    pub temat: Option<i64>,
    #[serde(default)]
    pub msg_id: i64,
    /// KLUCZOWE dla kanałów typu ATFX: zarządzają pozycją, ODPOWIADAJĄC na
    /// wiadomość z sygnałem. Bez tego numeru „TP1 HIT" nie ma do czego przypiąć.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_of: Option<i64>,

    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub znakow: usize,

    #[serde(default, alias = "monitored")]
    pub nasluchiwany: bool,
    /// Format przypisany źródłu w chwili odbioru — część odtworzenia decyzji.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Czy parser rdzenia cokolwiek w tym zobaczył. Podpowiedź dla interfejsu;
    /// prawdą pozostaje `text`, bo tekst da się przetworzyć ponownie, a wniosek nie.
    #[serde(default)]
    pub rozpoznane: bool,

    /// Opis przy znaczniku sesji (`start`/`stop`) — wersja, konfiguracja, powód.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uwaga: Option<String>,

    /// Znacznik Telegrama w SEKUNDACH — pole wyłącznie do odczytu starych
    /// plików pierwszej `kronika.exe`. Nowe wiersze go nie zapisują.
    #[serde(default, skip_serializing)]
    pub ts_telegram: Option<i64>,
}

fn domyslny_schemat() -> u32 {
    SCHEMAT
}

impl Wpis {
    /// Klucz wiadomości: kanał + numer. `msg_id` jest unikalny w obrębie czatu,
    /// nie globalnie — para bez `chat_id` skleiłaby dwa różne kanały w jeden.
    pub fn klucz(&self) -> (i64, i64) {
        (self.chat_id, self.msg_id)
    }

    /// Nazwa źródła do pokazania: kanał albo „kanał · temat N".
    pub fn zrodlo(&self) -> String {
        match self.temat {
            Some(t) => format!("{} · temat {t}", self.chat),
            None => self.chat.clone(),
        }
    }

    /// Domyka rozbieżności między historycznymi kształtami zapisu.
    pub(crate) fn znormalizuj(&mut self) {
        if self.ts_telegram_ms == 0 {
            if let Some(s) = self.ts_telegram {
                self.ts_telegram_ms = s * 1000;
            }
        }
        if self.znakow == 0 && !self.text.is_empty() {
            self.znakow = self.text.chars().count();
        }
    }
}

// ============================================================
//  OPCJE ZAPISU
// ============================================================

/// Które źródła nagrywać.
///
/// Domyślnie **wszystkie**, i to nie jest lenistwo: kanał pominięty przy
/// zapisie jest stracony na zawsze, a włączenie go jutro nie odtworzy tego,
/// co mówił wczoraj. Wybór istnieje dlatego, że konto z dwustoma czatami
/// prywatnymi produkuje szum, którego nikt nigdy nie przeczyta — ale to
/// użytkownik ma podjąć tę decyzję świadomie, a nie program za niego.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tryb", rename_all = "lowercase")]
pub enum Zrodla {
    /// nagrywaj wszystko, co widzi sesja
    Wszystkie,
    /// nagrywaj wyłącznie wymienione kanały (i tematy)
    Wybrane { lista: Vec<Zrodlo> },
}

impl Default for Zrodla {
    fn default() -> Self {
        Zrodla::Wszystkie
    }
}

/// Jedno wybrane źródło. `temat: None` znaczy **cały kanał**, razem ze
/// wszystkimi tematami — inaczej wybór forum wymagałby wyklikania listy,
/// która rośnie sama, gdy autor doda nowy temat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zrodlo {
    pub chat_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temat: Option<i64>,
}

impl Zrodla {
    pub fn pasuje(&self, chat_id: i64, temat: Option<i64>) -> bool {
        match self {
            Zrodla::Wszystkie => true,
            Zrodla::Wybrane { lista } => lista
                .iter()
                .any(|z| z.chat_id == chat_id && (z.temat.is_none() || z.temat == temat)),
        }
    }

    pub fn ile(&self) -> Option<usize> {
        match self {
            Zrodla::Wszystkie => None,
            Zrodla::Wybrane { lista } => Some(lista.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tryb", rename_all = "lowercase")]
pub enum Fsync {
    /// po każdej linii — bezpieczne, domyślne
    Kazda,
    /// co `n` linii; przy awarii tracisz do `n` ostatnich wiadomości
    Co { n: u32 },
    /// nigdy — najszybsze i jedyne, które potrafi zgubić godziny pracy
    Nigdy,
}

impl Default for Fsync {
    fn default() -> Self {
        Fsync::Kazda
    }
}

impl Fsync {
    /// Zdanie do pokazania w interfejsie — z ceną, nie tylko z nazwą.
    pub fn ryzyko(self) -> &'static str {
        match self {
            Fsync::Kazda => "Zanik zasilania kosztuje najwyżej ostatnią wiadomość.",
            Fsync::Co { n } => {
                let _ = n;
                "Zanik zasilania kosztuje do N ostatnich wiadomości."
            }
            Fsync::Nigdy => {
                "Zanik zasilania kosztuje WSZYSTKO, co system trzyma w buforze — \
                 realnie godziny sygnałów."
            }
        }
    }
}

/// Komplet opcji zapisu. Leży we własnym pliku `kronika.json`.
///
/// ŚWIADOMIE osobno od `settings.json`: to nie jest ustawienie strategii,
/// tylko opis rejestratora. Wgranie cudzego presetu nie ma prawa przestawić
/// nikomu tego, co i gdzie się zapisuje.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ustawienia {
    /// czy w ogóle zapisywać
    pub wlaczona: bool,
    /// JEDEN ciągły plik. Ścieżka względna liczy się od katalogu roboczego.
    pub plik: String,
    pub zrodla: Zrodla,
    /// zapisywać wiadomości, których parser NIE rozpoznał (domyślnie TAK —
    /// selekcja przy zapisie jest nieodwracalna, a formaty się zmieniają)
    pub nierozpoznane: bool,
    /// zapisywać wiadomości bez treści tekstowej (zdjęcia, załączniki,
    /// komunikaty usługowe). Sam strumień bota niesie tylko tekst, więc taki
    /// wiersz jest pusty — ale jego ISTNIENIE mówi, że coś się w kanale stało.
    pub puste: bool,
    pub fsync: Fsync,
    /// obrót pliku po przekroczeniu rozmiaru (MB). 0 = jeden plik bez końca.
    pub obrot_mb: u64,
    /// ile obróconych plików trzymać. 0 = wszystkie.
    pub trzymaj_plikow: u32,
    /// dopisywać znaczniki `start`/`stop` (patrz [`Rodzaj`])
    pub znaczniki_sesji: bool,
    /// przesunięcie strefy w godzinach — do renderowania `odebrano`
    pub strefa_h: f64,
}

impl Default for Ustawienia {
    fn default() -> Self {
        Ustawienia {
            wlaczona: true,
            plik: PLIK_DOMYSLNY.to_string(),
            zrodla: Zrodla::Wszystkie,
            // OBIE wartości domyślne na `true` z tego samego powodu:
            // czego nie zapiszemy dzisiaj, tego nie będzie jutro.
            nierozpoznane: true,
            puste: true,
            fsync: Fsync::Kazda,
            obrot_mb: 0,
            trzymaj_plikow: 0,
            znaczniki_sesji: true,
            strefa_h: 3.0,
        }
    }
}

impl Ustawienia {
    pub fn strefa_ms(&self) -> i64 {
        (self.strefa_h * 3_600_000.0) as i64
    }

    /// Ścieżka pliku rozwinięta względem katalogu roboczego.
    pub fn sciezka(&self, korzen: &std::path::Path) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(&self.plik);
        if p.is_absolute() {
            p
        } else {
            korzen.join(p)
        }
    }

    /// Sprawdza to, co da się sprawdzić bez dotykania dysku.
    pub fn sprawdz(&self) -> Result<(), String> {
        if self.plik.trim().is_empty() {
            return Err("ścieżka pliku nie może być pusta".into());
        }
        if self.plik.contains("..") {
            return Err("ścieżka nie może zawierać „..”".into());
        }
        if let Zrodla::Wybrane { lista } = &self.zrodla {
            if lista.is_empty() {
                return Err(
                    "wybrano tryb „tylko zaznaczone”, ale nie zaznaczono ani jednego kanału — \
                     kronika nie zapisałaby niczego"
                        .into(),
                );
            }
        }
        if let Fsync::Co { n } = self.fsync {
            if n == 0 {
                return Err("„co N linii” wymaga N większego od zera".into());
            }
        }
        Ok(())
    }
}

// ============================================================
//  CZAS
// ============================================================

pub fn teraz_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// ISO 8601 w zadanej strefie — czytelne dla człowieka i sortowalne.
pub fn czas_iso(ms: i64, strefa_ms: i64) -> String {
    use chrono::{FixedOffset, TimeZone};
    let strefa = FixedOffset::east_opt((strefa_ms / 1000) as i32)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("offset zerowy jest zawsze poprawny"));
    match strefa.timestamp_millis_opt(ms).single() {
        Some(t) => t.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        None => ms.to_string(),
    }
}

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn wybrane_zrodla_obejmuja_caly_kanal_gdy_temat_pusty() {
        let z = Zrodla::Wybrane {
            lista: vec![Zrodlo {
                chat_id: -100,
                temat: None,
            }],
        };
        assert!(z.pasuje(-100, None));
        assert!(
            z.pasuje(-100, Some(42)),
            "brak tematu = CAŁY kanał, razem z tematami"
        );
        assert!(!z.pasuje(-200, None));
    }

    #[test]
    fn wybrany_temat_nie_wpuszcza_reszty_kanalu() {
        let z = Zrodla::Wybrane {
            lista: vec![Zrodlo {
                chat_id: -100,
                temat: Some(42),
            }],
        };
        assert!(z.pasuje(-100, Some(42)));
        assert!(!z.pasuje(-100, Some(7)));
        assert!(!z.pasuje(-100, None));
    }

    #[test]
    fn wszystkie_zrodla_wpuszczaja_cokolwiek() {
        assert!(Zrodla::Wszystkie.pasuje(1, None));
        assert!(Zrodla::Wszystkie.pasuje(-999, Some(3)));
    }

    #[test]
    fn domyslne_ustawienia_nic_nie_gubia() {
        let u = Ustawienia::default();
        assert!(u.wlaczona);
        assert!(u.nierozpoznane, "domyślnie zapisujemy TAKŻE nierozpoznane");
        assert!(u.puste);
        assert_eq!(
            u.fsync,
            Fsync::Kazda,
            "fsync po każdej linii MUSI być domyślny"
        );
        assert_eq!(u.obrot_mb, 0, "domyślnie JEDEN ciągły plik");
        assert!(matches!(u.zrodla, Zrodla::Wszystkie));
    }

    #[test]
    fn pusty_wybor_kanalow_jest_bledem_a_nie_cisza() {
        let mut u = Ustawienia::default();
        u.zrodla = Zrodla::Wybrane { lista: vec![] };
        assert!(u.sprawdz().is_err(), "zapis do nikąd musi być odrzucony");
    }

    #[test]
    fn czyta_stary_ksztalt_archiwum_conduita() {
        // wiersz z `logs/wiadomosci/wiadomosci-*.jsonl` sprzed tej biblioteki
        let stary = r#"{"v":1,"seq":3,"event":"edited","received_at":"2026-07-29T10:00:00.000+03:00",
            "received_at_ms":1785000000000,"msg_ts_ms":1784999000000,"chat_id":-100,"topic_id":7,
            "msg_id":500,"reply_to":499,"edit_of":500,"source_name":"ATFX","text":"RISK FREE","monitored":true}"#;
        let mut w: Wpis = serde_json::from_str(stary).unwrap();
        w.znormalizuj();
        assert_eq!(w.rodzaj, Rodzaj::Edycja);
        assert_eq!(w.chat, "ATFX");
        assert_eq!(w.temat, Some(7));
        assert_eq!(w.nasluchiwany, true);
        assert_eq!(w.ts_telegram_ms, 1784999000000);
        assert_eq!(
            w.znakow, 9,
            "długość doliczana przy odczycie starego wiersza"
        );
    }

    #[test]
    fn czyta_stary_ksztalt_pierwszej_kroniki_z_sekundami() {
        let stary = r#"{"odebrano_ms":1785000000000,"odebrano":"2026-07-29 10:00:00.000",
            "rodzaj":"nowa","chat_id":-100,"chat":"SYNERGY","temat":null,"msg_id":9,
            "reply_to":null,"ts_telegram":1784999000,"text":"BUY","znakow":3}"#;
        let mut w: Wpis = serde_json::from_str(stary).unwrap();
        w.znormalizuj();
        assert_eq!(w.rodzaj, Rodzaj::Nowa);
        assert_eq!(
            w.ts_telegram_ms, 1784999000000,
            "sekundy przeliczone na milisekundy"
        );
        assert_eq!(w.v, SCHEMAT, "brak pola wersji = najstarszy kształt");
    }

    #[test]
    fn znacznik_sesji_nie_jest_wiadomoscia() {
        assert!(!Rodzaj::Start.to_wiadomosc());
        assert!(!Rodzaj::Stop.to_wiadomosc());
        assert!(Rodzaj::Nowa.to_wiadomosc());
        assert!(Rodzaj::Edycja.to_wiadomosc());
        assert!(Rodzaj::Skasowana.to_wiadomosc());
    }
}
