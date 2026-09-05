
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Wersja formatu. Podbicie = plik ze starszej wersji jest odrzucany
/// z ostrzeżeniem, zamiast być czytany po połowie.
pub const SECRETS_VERSION: u32 = 1;

// ============================================================
//  MASKOWANIE
// ============================================================

/// Zamienia sekret w coś, co wolno wpisać do logu.
///
/// Świadomie NIE pokazuje żadnego fragmentu wartości — nawet ostatnich czterech
/// znaków. `api_hash` ma 32 znaki heksadecymalne; ujawnienie czterech z nich
/// skraca przeszukiwanie o 16 bitów za darmo, a użytkownikowi nie mówi nic
/// więcej niż sama informacja „jest ustawiony i ma tyle znaków".
pub fn mask(secret: &str) -> String {
    if secret.is_empty() {
        return "(brak)".to_string();
    }
    format!("•••••••• ({} znaków)", secret.chars().count())
}

/// Czy sekret jest ustawiony? Do warunków w interfejsie, bez ujawniania treści.
#[inline]
pub fn is_set(secret: &str) -> bool {
    !secret.trim().is_empty()
}

/// Opakowanie, które w `Debug` i `Display` pokazuje wyłącznie maskę.
///
/// Przydaje się tam, gdzie sekret wędruje przez strukturę wypisywaną
/// automatycznie (`tracing::debug!(?cfg)`) — wtedy nie da się go wypisać
/// przez nieuwagę.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Masked(pub String);

impl Masked {
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    #[inline]
    pub fn is_set(&self) -> bool {
        is_set(&self.0)
    }
    #[inline]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<String> for Masked {
    fn from(s: String) -> Self {
        Masked(s)
    }
}

impl From<&str> for Masked {
    fn from(s: &str) -> Self {
        Masked(s.to_string())
    }
}

impl fmt::Debug for Masked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&mask(&self.0))
    }
}

impl fmt::Display for Masked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&mask(&self.0))
    }
}

// ============================================================
//  DOKUMENT
// ============================================================

/// Poświadczenia Telegrama.
///
/// `api_id` jest jawny — to publiczny numer aplikacji, widoczny w każdym
/// kliencie i bezużyteczny bez `api_hash`. Reszta jest tajna.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramSecrets {
    /// z <https://my.telegram.org> → API development tools
    pub api_id: i32,
    pub api_hash: Masked,
    /// Przenośny łańcuch sesji MTProto (base64). RÓWNOWAŻNY ZALOGOWANEMU
    /// URZĄDZENIU: dzięki niemu start po restarcie nie wymaga skanowania kodu.
    pub session_string: Masked,
    /// Kto jest zalogowany — do pokazania w interfejsie. Nie jest sekretem,
    /// ale nie ma powodu, żeby wychodziło poza tę maszynę.
    pub user_id: i64,
    pub user_name: String,
    /// Nazwa z małpą (@handle). Pole nazywa się `handle`, a NIE `username`,
    /// bo `username` i `user_name` dają w camelCase klucze „username"
    /// i „userName" — różne dla serde, ale IDENTYCZNE dla każdego czytnika
    /// JSON-a ignorującego wielkość liter. PowerShell (`ConvertFrom-Json`)
    /// odmawia wtedy wczytania pliku, a to jest plik, który użytkownik
    /// ogląda standardowymi narzędziami Windows.
    pub handle: String,
    /// kiedy sesja została zapisana (ms epoki)
    pub saved_at: i64,
}

impl TelegramSecrets {
    /// Czy da się w ogóle podłączyć do Telegrama (mamy parę api_id/api_hash)?
    pub fn has_credentials(&self) -> bool {
        self.api_id != 0 && self.api_hash.is_set()
    }
    /// Czy da się zalogować bez skanowania kodu?
    pub fn has_session(&self) -> bool {
        self.session_string.is_set()
    }
}

impl fmt::Debug for TelegramSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramSecrets")
            .field("api_id", &self.api_id)
            .field("api_hash", &self.api_hash)
            .field("session_string", &self.session_string)
            .field("user_name", &self.user_name)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmtpSecrets {
    pub password: Masked,
}

impl fmt::Debug for SmtpSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpSecrets")
            .field("password", &self.password)
            .finish()
    }
}

/// Hasło do rachunku MT5 — do logowania HEADLESS przez sidecar
/// (`mt5.initialize(login=…, password=…, server=…)`).
///
/// Ten sam wzorzec co hasło SMTP: pole w panelu jest tylko SKRZYNKĄ
/// PODAWCZĄ — wartość ląduje tutaj, do `settings.json` nigdy nie trafia,
/// a do sidecara jedzie zmienną środowiskową (`CONDUIT_MT5_PASSWORD`),
/// nie argumentem procesu (argumenty widać w liście procesów).
///
/// Puste = sidecar loguje się bez hasła, co działa, gdy terminal ma
/// zapamiętane poświadczenia — najczęstszy przypadek. Hasło jest potrzebne
/// dopiero do PRZEŁĄCZENIA terminala na inne konto bez klikania w GUI.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mt5Secrets {
    pub password: Masked,
}

impl fmt::Debug for Mt5Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mt5Secrets")
            .field("password", &self.password)
            .finish()
    }
}

/// Całość `secrets.json`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsDoc {
    pub version: u32,
    #[serde(default)]
    pub telegram: TelegramSecrets,
    #[serde(default)]
    pub smtp: SmtpSecrets,
    #[serde(default)]
    pub mt5: Mt5Secrets,
}

impl Default for SecretsDoc {
    fn default() -> Self {
        SecretsDoc {
            version: SECRETS_VERSION,
            telegram: TelegramSecrets::default(),
            smtp: SmtpSecrets::default(),
            mt5: Mt5Secrets::default(),
        }
    }
}

impl fmt::Debug for SecretsDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretsDoc")
            .field("version", &self.version)
            .field("telegram", &self.telegram)
            .field("smtp", &self.smtp)
            .field("mt5", &self.mt5)
            .finish()
    }
}

impl SecretsDoc {
    /// Podsumowanie BEZPIECZNE do wysłania do interfejsu i do logu.
    ///
    /// To jest jedyna droga, którą stan poświadczeń opuszcza serwer.
    pub fn public_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "telegram": {
                "apiId": self.telegram.api_id,
                "apiHashSet": self.telegram.api_hash.is_set(),
                "apiHashMasked": mask(self.telegram.api_hash.as_str()),
                "sessionSet": self.telegram.has_session(),
                "userName": self.telegram.user_name,
                "handle": self.telegram.handle,
                "savedAt": self.telegram.saved_at,
            },
            "smtp": {
                "passwordSet": self.smtp.password.is_set(),
                "passwordMasked": mask(self.smtp.password.as_str()),
            },
            "mt5": {
                "passwordSet": self.mt5.password.is_set(),
                "passwordMasked": mask(self.mt5.password.as_str()),
            },
        })
    }
}

// ============================================================
//  UPRAWNIENIA PLIKU
// ============================================================

/// Zawęża prawa do pliku do samego właściciela.
///
/// Zwraca `Err` tylko wtedy, gdy dało się stwierdzić porażkę — brak `icacls`
/// albo nietypowy system plików kończy się ostrzeżeniem, a nie przerwaniem
/// startu bota. Odmowa uruchomienia z powodu ACL byłaby gorsza od samego
/// problemu: bot i tak trzyma te dane, tylko przestałby handlować.
pub fn restrict_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(path)?.permissions();
        p.set_mode(0o600);
        std::fs::set_permissions(path, p)?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        // Zrywamy dziedziczenie (`/inheritance:r`) i zostawiamy JEDEN wpis:
        // pełne prawa dla właściciela. Bez zerwania dziedziczenia wpisy
        // z katalogu nadrzędnego (często „Users: odczyt") zostają w mocy
        // i całe ćwiczenie jest pozorne.
        let konto = konto_windows();
        let out = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{konto}:F"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => anyhow::bail!("icacls zakończył się kodem {}", o.status),
            Err(e) => anyhow::bail!("nie udało się uruchomić icacls: {e}"),
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

/// Nazwa konta w postaci, którą rozumie `icacls`.
#[cfg(windows)]
fn konto_windows() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        // `%USERNAME%` bywa puste w usłudze systemowej — wtedy zostaje
        // wbudowana nazwa właściciela pliku, którą icacls też akceptuje
        return "CREATOR OWNER".to_string();
    }
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => user,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maska_nie_ujawnia_ani_jednego_znaku() {
        let hash = "0123456789abcdef0123456789abcdef";
        let m = mask(hash);
        assert!(!m.contains("0123"), "maska ujawnia początek: {m}");
        assert!(!m.contains("cdef"), "maska ujawnia koniec: {m}");
        // ale mówi, że coś tam jest i ile tego jest
        assert!(m.contains("32"), "maska musi podać długość: {m}");
        assert_eq!(mask(""), "(brak)");
    }

    #[test]
    fn maska_liczy_znaki_a_nie_bajty() {
        // hasło z polskimi znakami ma mniej znaków niż bajtów w UTF-8
        assert_eq!(mask("zażółć"), "•••••••• (6 znaków)");
    }

    #[test]
    fn debug_sekretu_pokazuje_maske_a_nie_tresc() {
        let s = Masked::from("tajne-haslo-smtp");
        let d = format!("{s:?}");
        assert!(!d.contains("tajne"), "Debug ujawnia sekret: {d}");
        assert!(d.contains('•'), "{d}");
        // i to samo przez Display
        assert!(!format!("{s}").contains("tajne"));
    }

    #[test]
    fn debug_calego_dokumentu_nie_wypisuje_sekretow() {
        let doc = SecretsDoc {
            version: SECRETS_VERSION,
            telegram: TelegramSecrets {
                api_id: 1234567,
                api_hash: "abcdef0123456789abcdef0123456789".into(),
                session_string: "BARDZO-DLUGI-LANCUCH-SESJI".into(),
                user_id: 42,
                user_name: "Demo User".into(),
                handle: "demo_user".into(),
                saved_at: 1_700_000_000_000,
            },
            smtp: SmtpSecrets {
                password: "haslo-aplikacji".into(),
            },
            mt5: Default::default(),
        };
        let d = format!("{doc:?}");
        for tajne in ["abcdef0123456789", "BARDZO-DLUGI", "haslo-aplikacji"] {
            assert!(!d.contains(tajne), "Debug ujawnia „{tajne}”: {d}");
        }
        // api_id ma prawo być widoczny — jest publiczny
        assert!(d.contains("1234567"), "{d}");
    }

    #[test]
    fn podsumowanie_publiczne_nie_niesie_sekretow() {
        let doc = SecretsDoc {
            version: SECRETS_VERSION,
            telegram: TelegramSecrets {
                api_id: 987,
                api_hash: "sekretny-hash".into(),
                session_string: "sekretna-sesja".into(),
                user_name: "Demo User".into(),
                ..Default::default()
            },
            smtp: SmtpSecrets {
                password: "sekretne-haslo".into(),
            },
            mt5: Default::default(),
        };
        let j = doc.public_summary();
        let tekst = serde_json::to_string(&j).unwrap();
        for tajne in ["sekretny-hash", "sekretna-sesja", "sekretne-haslo"] {
            assert!(
                !tekst.contains(tajne),
                "podsumowanie ujawnia „{tajne}”: {tekst}"
            );
        }
        // ale mówi, CO jest ustawione — bez tego UI nie wie, czy prosić
        assert_eq!(j["telegram"]["apiId"], 987);
        assert_eq!(j["telegram"]["apiHashSet"], true);
        assert_eq!(j["telegram"]["sessionSet"], true);
        assert_eq!(j["smtp"]["passwordSet"], true);
        assert_eq!(j["telegram"]["userName"], "Demo User");
    }

    #[test]
    fn brak_poswiadczen_jest_rozpoznawany() {
        let t = TelegramSecrets::default();
        assert!(!t.has_credentials());
        assert!(!t.has_session());

        let t = TelegramSecrets {
            api_id: 5,
            ..Default::default()
        };
        assert!(!t.has_credentials(), "sam api_id nie wystarcza");

        let t = TelegramSecrets {
            api_id: 5,
            api_hash: "x".into(),
            ..Default::default()
        };
        assert!(t.has_credentials());

        // same białe znaki to nadal brak
        let t = TelegramSecrets {
            api_id: 5,
            api_hash: "   ".into(),
            ..Default::default()
        };
        assert!(!t.has_credentials());
    }

    #[test]
    fn dokument_przechodzi_przez_json_bez_strat() {
        let doc = SecretsDoc {
            version: SECRETS_VERSION,
            telegram: TelegramSecrets {
                api_id: 1234567,
                api_hash: "abc".into(),
                session_string: "sesja".into(),
                user_id: 7,
                user_name: "Demo User".into(),
                handle: "demo_user".into(),
                saved_at: 99,
            },
            smtp: SmtpSecrets {
                password: "p".into(),
            },
            mt5: Default::default(),
        };
        let raw = serde_json::to_string(&doc).unwrap();
        let back: SecretsDoc = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, doc);
        // sekrety w PLIKU są jawne (to plik z sekretami) — maskowanie
        // dotyczy logów i odpowiedzi po sieci, nie zapisu na dysk
        assert!(raw.contains("sesja"));
    }

    #[test]
    fn klucze_pliku_sa_rozne_takze_bez_wielkosci_liter() {
        // REGRESJA znaleziona przy realnym uruchomieniu: pola `user_name`
        // i `username` dawały w camelCase klucze „userName" i „username" —
        // dla serde różne, dla PowerShellowego `ConvertFrom-Json` te same.
        // Efekt: użytkownik nie mógł otworzyć własnego secrets.json
        // standardowym narzędziem Windows („duplicated keys").
        let raw = serde_json::to_string(&SecretsDoc::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();

        fn sprawdz(v: &serde_json::Value, sciezka: &str) {
            let Some(o) = v.as_object() else { return };
            let mut male: Vec<String> = o.keys().map(|k| k.to_lowercase()).collect();
            male.sort();
            let przed = male.len();
            male.dedup();
            assert_eq!(
                przed,
                male.len(),
                "w „{sciezka}” są klucze różniące się tylko wielkością liter: {:?}",
                o.keys().collect::<Vec<_>>()
            );
            for (k, x) in o {
                sprawdz(x, &format!("{sciezka}/{k}"));
            }
        }
        sprawdz(&v, "");

        // to samo dla podsumowania wysyłanego do UI
        sprawdz(&SecretsDoc::default().public_summary(), "public_summary");
    }

    #[test]
    fn brakujace_pola_wypelniaja_sie_domyslnymi() {
        // plik zapisany starszą wersją programu nie może wywrócić startu
        let doc: SecretsDoc = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(doc.version, 1);
        assert!(!doc.telegram.has_credentials());
        assert!(!doc.smtp.password.is_set());
    }
}
