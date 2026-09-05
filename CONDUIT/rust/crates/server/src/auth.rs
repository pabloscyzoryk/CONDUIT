//! Logowanie do Telegrama — KONTRAKT, nie implementacja.
//!
//! Klienta MTProto pisze inny agent (`crates/telegram`). Tutaj definiujemy
//! wyłącznie:
//!  * automat stanu, który widzi interfejs (`AuthState`),
//!  * cechę `TelegramAuth` — jedyny punkt wpięcia prawdziwego klienta,
//!  * generator kodu QR (prawdziwy QR z URL-a logowania, nie ozdoba).
//!
//! Dzięki temu ekran logowania w React można napisać RAZ i podłączyć
//! prawdziwy Telegram bez dotykania UI.
//!
//! Kolejność wywołań z UI:
//! ```text
//!   GET  /api/auth/state      -> stage = needCredentials  (pierwsze uruchomienie)
//!   POST /api/auth/credentials { apiId, apiHash }
//!   POST /api/auth/qr/start   -> AuthState { stage: waitingScan, qrSvg, tokenUrl, expiresAt }
//!   GET  /api/auth/state      -> polling co ~1 s (albo push przez WS, sekcja `auth`)
//!        · stage = waitingPassword  -> POST /api/auth/2fa { password }
//!        · stage = confirming       -> użytkownik potwierdza na telefonie
//!        · stage = loggedIn         -> UI wchodzi do terminala
//!        · stage = expired          -> UI wywołuje /qr/start ponownie
//!   POST /api/auth/logout
//! ```
//!
//! # Dlaczego `needCredentials` jest osobnym etapem
//!
//! Kodu QR NIE DA SIĘ wygenerować bez pary `api_id`/`api_hash`: token wydaje
//! serwer Telegrama w odpowiedzi na `auth.exportLoginToken`, a to wywołanie
//! wymaga poświadczeń aplikacji. Ekran, który pokazuje kod QR przed ich
//! podaniem, może pokazać wyłącznie atrapę — a atrapa kodu logowania jest
//! gorsza od pustego miejsca, bo użytkownik traci czas na skanowanie czegoś,
//! co nigdy nie zadziała.
//!
//! Po udanym logowaniu łańcuch sesji ląduje w `secrets.json` i kolejne starty
//! omijają cały ten automat: `state()` od razu zwraca `loggedIn`.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Etap logowania. Nazwy w camelCase — trafiają wprost do TypeScriptu.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum AuthStage {
    /// nikt nie jest zalogowany, nie ma aktywnego tokenu
    #[default]
    LoggedOut,
    /// BRAK `api_id`/`api_hash` — bez nich nie da się wydać tokenu QR.
    /// UI pokazuje formularz z odnośnikiem do <https://my.telegram.org>.
    NeedCredentials,
    /// mamy poświadczenia, klient się podnosi (albo wznawia zapisaną sesję)
    Connecting,
    /// token wygenerowany, czekamy na zeskanowanie kodu
    WaitingScan,
    /// kod zeskanowany, konto ma hasło chmury (2FA)
    WaitingPassword,
    /// czekamy na potwierdzenie „to ja" na telefonie
    Confirming,
    /// token QR wygasł — trzeba wygenerować nowy
    Expired,
    LoggedIn,
    Error,
}

/// Pełny stan logowania widziany przez UI.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    #[serde(flatten)]
    pub stage: AuthStage,
    /// URL `tg://login?token=...` zakodowany w kodzie QR
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    /// gotowy do wstawienia SVG kodu QR (bez zależności po stronie przeglądarki)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_svg: Option<String>,
    /// kiedy token wygasa (ms epoki) — UI odlicza i sam odświeża
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// podpowiedź do hasła 2FA z serwera Telegrama
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_hint: Option<String>,
    /// nazwa urządzenia / użytkownika po zeskanowaniu
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// komunikat błędu (np. „hasło nieprawidłowe")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    // ---------- poświadczenia aplikacji ----------
    /// `api_id` — PUBLICZNY numer aplikacji, wolno go pokazać.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_id: Option<i32>,
    /// Czy `api_hash` jest zapisany. Sam hash NIGDY nie wychodzi z serwera.
    #[serde(default)]
    pub api_hash_set: bool,
    /// Maska hasza do pokazania w ustawieniach („•••••••• (32 znaków)").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_hash_masked: Option<String>,
    /// Czy w `secrets.json` leży łańcuch sesji, czyli czy kolejny start
    /// zaloguje się bez skanowania kodu.
    #[serde(default)]
    pub session_saved: bool,
}

impl AuthState {
    pub fn logged_out() -> Self {
        AuthState {
            stage: AuthStage::LoggedOut,
            ..Default::default()
        }
    }

    /// Pierwsze uruchomienie: nie ma o co pytać Telegrama, dopóki nie ma
    /// poświadczeń aplikacji.
    pub fn need_credentials() -> Self {
        AuthState {
            stage: AuthStage::NeedCredentials,
            ..Default::default()
        }
    }

    /// Dokłada opis poświadczeń do dowolnego etapu — dzięki temu ekran
    /// logowania zawsze wie, czy formularz api_id/api_hash ma być wypełniony.
    pub fn with_credentials(mut self, api_id: i32, api_hash: &str, session_saved: bool) -> Self {
        self.api_id = (api_id != 0).then_some(api_id);
        self.api_hash_set = crate::secrets::is_set(api_hash);
        self.api_hash_masked = Some(crate::secrets::mask(api_hash));
        self.session_saved = session_saved;
        self
    }

    pub fn error(msg: impl Into<String>) -> Self {
        AuthState {
            stage: AuthStage::Error,
            error: Some(msg.into()),
            ..Default::default()
        }
    }

    pub fn logged_in(user: impl Into<String>) -> Self {
        AuthState {
            stage: AuthStage::LoggedIn,
            user: Some(user.into()),
            ..Default::default()
        }
    }

    #[inline]
    pub fn is_logged_in(&self) -> bool {
        self.stage == AuthStage::LoggedIn
    }
}

/// Renderuje PRAWDZIWY kod QR jako SVG.
///
/// Kod niesie dokładnie ten URL, który poda klient MTProto — więc telefon
/// naprawdę go zeskanuje. To nie jest atrapa z prototypu React.
pub fn qr_svg(payload: &str, size: u32) -> anyhow::Result<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(payload.as_bytes())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(size, size)
        .quiet_zone(true)
        // kolory jako `currentColor`/`transparent`, żeby SVG dopasował się
        // do motywu i palety interfejsu bez przeliczania po stronie serwera
        .dark_color(svg::Color("currentColor"))
        .light_color(svg::Color("transparent"))
        .build())
}

/// PUNKT WPIĘCIA klienta Telegrama.
///
/// Implementacja żyje w `crates/telegram`. Metody są synchroniczne i muszą
/// wracać natychmiast — prawdziwy klient trzyma własne zadanie w tle
/// i tylko publikuje przez nie bieżący stan.
pub trait TelegramAuth: Send + Sync {
    /// Bounded asynchronous read-only preview; no historical message injection.
    /// Default refusal preserves existing auth implementations and OFF behavior.
    fn history_import_preview(
        &self,
        _request: crate::history_import::HistoryFetchRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<crate::history_import::HistoryPreviewResponse, String>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Err("experimental history preview is unavailable".into()) })
    }
    /// Rozpocznij (lub odśwież) logowanie kodem QR.
    fn start_qr(&self) -> anyhow::Result<AuthState>;
    /// Bieżący stan — wołane przez polling i przez pętlę push WS.
    fn state(&self) -> AuthState;
    /// Hasło weryfikacji dwuetapowej.
    fn submit_password(&self, password: &str) -> anyhow::Result<AuthState>;
    /// Zerwij sesję.
    fn logout(&self) -> AuthState;

    /// Zapisuje `api_id`/`api_hash` z <https://my.telegram.org> i podnosi
    /// klienta. Wołane RAZ, przed pierwszym kodem QR; przy kolejnych startach
    /// dane są już w `secrets.json`.
    ///
    /// Domyślna implementacja odmawia — dzięki temu dołożenie metody nie psuje
    /// zaślepek i atrap, które o poświadczeniach nic nie wiedzą.
    fn set_credentials(&self, _api_id: i32, _api_hash: &str) -> anyhow::Result<AuthState> {
        anyhow::bail!("ten klient Telegrama nie przyjmuje poświadczeń")
    }

    /// Czy da się już wołać [`TelegramAuth::start_qr`]?
    fn has_credentials(&self) -> bool {
        false
    }

    /// Kasuje zapisane poświadczenia I sesję. Ostrzejsze niż `logout`:
    /// po tym trzeba wpisać api_id/api_hash od nowa.
    fn forget_credentials(&self) -> AuthState {
        self.logout()
    }

    /// LISTA CZATÓW KONTA — do wyboru źródła sygnału w panelu.
    ///
    /// Bez niej ekran „Kanały" mógłby pokazać wyłącznie wymyśloną listę
    /// z prototypu, a wtedy nie da się zaznaczyć PRAWDZIWEGO kanału: bot
    /// porównuje `chat_id` przychodzącej wiadomości z zapisanym powiązaniem,
    /// więc zaznaczenie fikcyjnego identyfikatora oznacza „nie handluj nigdy".
    ///
    /// Zwraca pustą listę, gdy klient nie jest zalogowany albo gdy tej binarki
    /// nie zbudowano z klientem MTProto — wołający ma wtedy powiedzieć wprost
    /// „zaloguj się", a nie podstawiać atrapę.
    fn list_channels(&self) -> anyhow::Result<Vec<ChannelInfo>> {
        Ok(Vec::new())
    }

    /// MINIATURA ZDJĘCIA PROFILOWEGO czatu — ścieżka do pliku na dysku.
    ///
    /// Zwraca `Ok(None)`, gdy czat zdjęcia nie ma (albo klienta MTProto w tej
    /// binarce nie ma) — i to jest normalna odpowiedź, nie awaria: interfejs
    /// zostawia wtedy literkę. Implementacja ma trzymać pobrane pliki
    /// w pamięci podręcznej na dysku; wołający NIE jest zobowiązany do
    /// ograniczania częstotliwości, bo endpoint obrazka odpytuje przeglądarka.
    fn channel_photo(&self, _chat_id: i64) -> anyhow::Result<Option<std::path::PathBuf>> {
        Ok(None)
    }
}

/// Czat konta widziany przez panel. Pola dobrane pod ekran wyboru kanałów —
/// nic, czego nie widać na liście, tutaj nie trafia.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    /// identyfikator w konwencji Bot API — DOKŁADNIE ten, który przychodzi
    /// w `SourceKey` odebranej wiadomości
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// `user` | `group` | `channel`
    pub kind: String,
    pub is_forum: bool,
    /// Czy czat MA na Telegramie zdjęcie profilowe.
    ///
    /// Osobne pole, żeby interfejs nie strzelał w pustkę: bez niego każdy
    /// z 200 kanałów zamawiałby obrazek i połowa dostawałaby 404.
    #[serde(default)]
    pub has_photo: bool,
    /// Znacznik wersji zdjęcia (`photo_id` z Telegrama) — jako TEKST, bo
    /// przekracza zakres bezpiecznych liczb JavaScriptu. Doklejany do adresu
    /// obrazka, żeby podmiana zdjęcia przebiła pamięć podręczną przeglądarki.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_version: Option<String>,
    /// tematy forum; pusta lista dla zwykłych czatów
    #[serde(default)]
    pub topics: Vec<ChannelTopic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTopic {
    pub id: i64,
    pub title: String,
    pub closed: bool,
}

/// Zaślepka używana tam, gdzie klienta MTProto NIE MA: w testach serwera
/// i w binarce zbudowanej bez `crates/telegram`.
///
/// UCZCIWIE: to nie loguje do Telegrama i **nie pokazuje kodu QR**. Zatrzymuje
/// się na `needCredentials` z komunikatem wprost. Wcześniejsza wersja rysowała
/// prawdziwy obrazek QR z zastępczym adresem — i to był błąd: użytkownik
/// skanował telefonem kod, który nie mógł zadziałać, bo token logowania wydaje
/// serwer Telegrama, a nie my. Puste miejsce z wyjaśnieniem jest uczciwsze niż
/// obrazek, który wygląda na działający.
pub struct UnconfiguredAuth {
    state: Mutex<AuthState>,
}

impl Default for UnconfiguredAuth {
    fn default() -> Self {
        Self::new()
    }
}

/// Komunikat powtarzany wszędzie, gdzie zaślepka odmawia — jedno miejsce,
/// żeby interfejs mógł go rozpoznać po treści.
pub const BRAK_KLIENTA: &str =
    "Klient MTProto nie jest wbudowany w tę binarkę — logowanie do Telegrama niedostępne.";

impl UnconfiguredAuth {
    pub fn new() -> Self {
        UnconfiguredAuth {
            state: Mutex::new(AuthState {
                stage: AuthStage::NeedCredentials,
                error: Some(BRAK_KLIENTA.into()),
                ..Default::default()
            }),
        }
    }
}

impl TelegramAuth for UnconfiguredAuth {
    fn start_qr(&self) -> anyhow::Result<AuthState> {
        anyhow::bail!(BRAK_KLIENTA)
    }

    fn state(&self) -> AuthState {
        self.state.lock().clone()
    }

    fn submit_password(&self, _password: &str) -> anyhow::Result<AuthState> {
        anyhow::bail!(BRAK_KLIENTA)
    }

    fn logout(&self) -> AuthState {
        let st = AuthState {
            stage: AuthStage::NeedCredentials,
            error: Some(BRAK_KLIENTA.into()),
            ..Default::default()
        };
        *self.state.lock() = st.clone();
        st
    }

    fn set_credentials(&self, _api_id: i32, _api_hash: &str) -> anyhow::Result<AuthState> {
        anyhow::bail!(BRAK_KLIENTA)
    }
}

pub type SharedAuth = Arc<dyn TelegramAuth>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_jest_prawdziwym_svg() {
        let svg = qr_svg("tg://login?token=ABC", 200).unwrap();
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(svg.contains("currentColor"));
    }

    #[test]
    fn stan_serializuje_sie_plasko() {
        let st = AuthState {
            stage: AuthStage::WaitingPassword,
            password_hint: Some("kot".into()),
            ..Default::default()
        };
        let j = serde_json::to_value(&st).unwrap();
        assert_eq!(j["stage"], "waitingPassword");
        assert_eq!(j["passwordHint"], "kot");
        // pola puste nie zaśmiecają protokołu
        assert!(j.get("qrSvg").is_none());
    }

    #[test]
    fn zaslepka_nie_udaje_zalogowania_i_nie_rysuje_atrapy_qr() {
        let a = UnconfiguredAuth::new();
        let st = a.state();
        assert_eq!(st.stage, AuthStage::NeedCredentials);
        assert!(!st.is_logged_in());
        // najważniejsze: ŻADNEGO kodu QR, którego nie da się zeskanować
        assert!(
            st.qr_svg.is_none(),
            "zaślepka nie może pokazywać atrapy kodu QR"
        );
        assert!(st.token_url.is_none());
        assert!(a.start_qr().is_err());
        assert!(a.submit_password("cokolwiek").is_err());
        assert!(a.set_credentials(123, "abc").is_err());
        assert!(!a.has_credentials());
    }

    #[test]
    fn opis_poswiadczen_nie_niesie_hasza() {
        let st = AuthState::need_credentials().with_credentials(
            1234567,
            "0123456789abcdef0123456789abcdef",
            true,
        );
        let j = serde_json::to_value(&st).unwrap();
        let tekst = serde_json::to_string(&j).unwrap();
        assert!(
            !tekst.contains("0123456789abcdef"),
            "api_hash wyciekł do UI: {tekst}"
        );
        assert_eq!(j["apiId"], 1234567);
        assert_eq!(j["apiHashSet"], true);
        assert_eq!(
            j["sessionSet"],
            serde_json::Value::Null,
            "pole nazywa się sessionSaved"
        );
        assert_eq!(j["sessionSaved"], true);
    }

    #[test]
    fn brak_poswiadczen_nie_udaje_ze_sa() {
        let st = AuthState::need_credentials().with_credentials(0, "", false);
        assert!(st.api_id.is_none());
        assert!(!st.api_hash_set);
        assert!(!st.session_saved);
    }
}
