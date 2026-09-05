//! Logowanie kodem QR + hasło dwuetapowe (2FA/SRP).
//!
//! `grammers-client` 0.10 udostępnia logowanie kodem z SMS-a i logowanie bota,
//! ale **nie ma logowania QR**. Poniżej jest ono zbudowane na surowych
//! wywołaniach protokołu, dokładnie wg <https://core.telegram.org/api/qr-login>:
//!
//! ```text
//!   auth.exportLoginToken ──▶ auth.loginToken{expires, token}
//!         │                        │
//!         │                        └─▶ tg://login?token=… ──▶ [KOD QR]
//!         │                                                      │
//!   updateLoginToken ◀────────────── użytkownik skanuje ─────────┘
//!         │
//!         ▼
//!   auth.exportLoginToken ──▶ loginTokenSuccess          → zalogowano
//!                          ├▶ loginTokenMigrateTo{dc}    → auth.importLoginToken w tym DC
//!                          └▶ błąd SESSION_PASSWORD_NEEDED → hasło 2FA (SRP)
//! ```
//!
//! # Dwie rzeczy, które ratują to rozwiązanie w praktyce
//!
//! **Token wygasa po ~30 sekundach.** Kod QR trzeba wtedy przerysować, inaczej
//! użytkownik skanuje martwy obrazek i nic się nie dzieje. Obsługa wygaśnięcia
//! nie jest tu dodatkiem — jest głównym trybem pracy pętli.
//!
//! **Nie polegamy wyłącznie na `updateLoginToken`.** Gdyby ta aktualizacja
//! zaginęła (a przed zalogowaniem strumień aktualizacji jest wątły), logowanie
//! zawisłoby na zawsze. Dlatego wygaśnięcie tokenu również wyzwala ponowne
//! `exportLoginToken` — a jeśli użytkownik zdążył zeskanować, to wywołanie
//! zwróci po prostu `loginTokenSuccess`. Zgubiona aktualizacja opóźnia
//! logowanie o kilkanaście sekund zamiast je zrywać.

use std::sync::Arc;
use std::time::Duration;

use grammers_client::client::{PasswordToken, UpdateStream};
use grammers_client::session::types::PeerInfo;
use grammers_client::session::Session;
use grammers_client::{tl, Client, InvocationError};
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::qr::QrRender;
use crate::session::FileSession;

/// Ile czekamy na aktualizację, zanim i tak odświeżymy token.
/// Serwer daje zwykle 30 s; zapas 2 s chroni przed wyścigiem z wygaśnięciem.
const EXPIRY_MARGIN: Duration = Duration::from_secs(2);

/// Zabezpieczenie przed nieskończonym czekaniem, gdy serwer poda dziwny `expires`.
const MAX_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct QrPrompt {
    /// `tg://login?token=…` — to jest to, co koduje obrazek
    pub url: String,
    pub qr: QrRender,
    /// ile jeszcze ten kod jest ważny
    pub expires_in: Duration,
}

#[derive(Debug, Clone)]
pub struct LoggedIn {
    pub user_id: i64,
    pub name: String,
    pub username: Option<String>,
}

/// Etap logowania. Interfejs użytkownika po prostu przełącza się między nimi.
#[derive(Debug, Clone)]
pub enum LoginStage {
    /// pokaż kod i wołaj [`QrLogin::step`]
    Qr(QrPrompt),
    /// konto ma weryfikację dwuetapową — potrzebne hasło
    PasswordRequired {
        hint: Option<String>,
    },
    /// hasło było błędne; można spróbować jeszcze raz
    PasswordWrong {
        hint: Option<String>,
    },
    Done(LoggedIn),
}

impl LoginStage {
    pub fn is_done(&self) -> bool {
        matches!(self, LoginStage::Done(_))
    }
    pub fn needs_password(&self) -> bool {
        matches!(
            self,
            LoginStage::PasswordRequired { .. } | LoginStage::PasswordWrong { .. }
        )
    }
}

struct Current {
    /// chwila, w której token przestaje być ważny
    deadline: Instant,
}

pub struct QrLogin {
    client: Client,
    session: Arc<FileSession>,
    api_id: i32,
    api_hash: String,
    current: Option<Current>,
    password: Option<PasswordToken>,
}

impl QrLogin {
    pub fn new(
        client: Client,
        session: Arc<FileSession>,
        api_id: i32,
        api_hash: impl Into<String>,
    ) -> Self {
        QrLogin {
            client,
            session,
            api_id,
            api_hash: api_hash.into(),
            current: None,
            password: None,
        }
    }

    /// Pierwszy krok: wygenerowanie tokenu i kodu QR.
    pub async fn start(&mut self) -> anyhow::Result<LoginStage> {
        self.export().await
    }

    /// Czeka na zeskanowanie kodu.
    ///
    /// Kończy się, gdy: użytkownik zeskanował (→ `Done` albo
    /// `PasswordRequired`), albo token wygasł (→ nowy `Qr` do pokazania).
    /// Interfejs woła to w pętli, dopóki nie dostanie `Done`.
    pub async fn step(&mut self, updates: &mut UpdateStream) -> anyhow::Result<LoginStage> {
        let deadline = self
            .current
            .as_ref()
            .map(|c| c.deadline)
            .unwrap_or_else(|| Instant::now() + MAX_WAIT);

        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                debug!("QR: token wygasł — odświeżam");
                return self.export().await;
            }
            tokio::select! {
                _ = tokio::time::sleep(left) => {
                    debug!("QR: minął czas ważności tokenu");
                    return self.export().await;
                }
                u = updates.next() => {
                    match u {
                        Ok(u) => {
                            if matches!(u.raw(), tl::enums::Update::LoginToken) {
                                info!("QR: kod zeskanowany — domykam logowanie");
                                return self.export().await;
                            }
                            // każda inna aktualizacja przed zalogowaniem jest bez znaczenia
                        }
                        Err(e) => {
                            warn!(%e, "QR: strumień aktualizacji przerwany — czekam na wygaśnięcie tokenu");
                            tokio::time::sleep_until(deadline).await;
                            return self.export().await;
                        }
                    }
                }
            }
        }
    }

    /// Podaje hasło weryfikacji dwuetapowej.
    ///
    /// Hasło NIE jest wysyłane w postaci jawnej — `grammers` liczy dowód SRP
    /// (`auth.checkPassword`), więc serwer nigdy go nie widzi.
    pub async fn submit_password(&mut self, password: &str) -> anyhow::Result<LoginStage> {
        let token = self
            .password
            .take()
            .ok_or_else(|| anyhow::anyhow!("hasło podane bez wcześniejszego żądania"))?;
        match self.client.check_password(token, password.trim()).await {
            Ok(user) => {
                let li = LoggedIn {
                    user_id: user.id().bare_id_unchecked(),
                    name: user.first_name().unwrap_or_default().to_string(),
                    username: user.username().map(|s| s.to_string()),
                };
                info!(user = %li.name, "Telegram: zalogowano (2FA)");
                self.persist();
                Ok(LoginStage::Done(li))
            }
            Err(grammers_client::SignInError::InvalidPassword(t)) => {
                let hint = t.hint().map(|s| s.to_string());
                // token wraca do puli — użytkownik może spróbować jeszcze raz
                self.password = Some(t);
                Ok(LoginStage::PasswordWrong { hint })
            }
            Err(e) => Err(anyhow::anyhow!("weryfikacja hasła nieudana: {e}")),
        }
    }

    // ============================================================

    /// Jedno wywołanie `auth.exportLoginToken` wraz z obsługą wszystkich
    /// trzech odpowiedzi i błędu 2FA.
    async fn export(&mut self) -> anyhow::Result<LoginStage> {
        let req = tl::functions::auth::ExportLoginToken {
            api_id: self.api_id,
            api_hash: self.api_hash.clone(),
            except_ids: Vec::new(),
        };
        let res = match self.client.invoke(&req).await {
            Ok(r) => r,
            Err(e) if is_password_needed(&e) => return self.ask_password().await,
            Err(e) => return Err(anyhow::anyhow!("auth.exportLoginToken: {e}")),
        };
        self.handle_token(res).await
    }

    async fn handle_token(
        &mut self,
        res: tl::enums::auth::LoginToken,
    ) -> anyhow::Result<LoginStage> {
        use tl::enums::auth::LoginToken as LT;
        match res {
            LT::Token(t) => {
                let ttl = Duration::from_secs((t.expires as i64 - now_secs()).clamp(1, 300) as u64);
                let wait = ttl
                    .saturating_sub(EXPIRY_MARGIN)
                    .max(Duration::from_secs(1));
                self.current = Some(Current {
                    deadline: Instant::now() + wait,
                });
                let qr = QrRender::from_token(&t.token)?;
                debug!(ttl_s = ttl.as_secs(), "QR: nowy token");
                Ok(LoginStage::Qr(QrPrompt {
                    url: qr.url.clone(),
                    qr,
                    expires_in: ttl,
                }))
            }

            // Konto mieszka w innym datacentrum. Trzeba tam przenieść dom
            // sesji i dokończyć logowanie POD TYM adresem — token z DC „A"
            // nie znaczy nic w DC „B".
            LT::MigrateTo(m) => {
                info!(
                    dc = m.dc_id,
                    "QR: przenoszę sesję do właściwego datacentrum"
                );
                self.session
                    .set_home_dc_id(m.dc_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("zapis domowego DC: {e}"))?;
                let res = match self
                    .client
                    .invoke_in_dc(
                        m.dc_id,
                        &tl::functions::auth::ImportLoginToken { token: m.token },
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) if is_password_needed(&e) => return self.ask_password().await,
                    Err(e) => return Err(anyhow::anyhow!("auth.importLoginToken: {e}")),
                };
                match res {
                    LT::Success(s) => self.finish(s.authorization).await,
                    other => {
                        // dwa przeniesienia pod rząd to znak, że coś jest nie tak
                        Err(anyhow::anyhow!(
                            "po przeniesieniu DC oczekiwano sukcesu, jest {other:?}"
                        ))
                    }
                }
            }

            LT::Success(s) => self.finish(s.authorization).await,
        }
    }

    async fn ask_password(&mut self) -> anyhow::Result<LoginStage> {
        let pw: tl::types::account::Password = self
            .client
            .invoke(&tl::functions::account::GetPassword {})
            .await
            .map_err(|e| anyhow::anyhow!("account.getPassword: {e}"))?
            .into();
        let token = PasswordToken::new(pw);
        let hint = token.hint().map(|s| s.to_string());
        info!("QR: konto chronione hasłem dwuetapowym");
        self.password = Some(token);
        Ok(LoginStage::PasswordRequired { hint })
    }

    /// Domknięcie logowania.
    ///
    /// `grammers` robi to samo w `Client::complete_login`, ale ta metoda jest
    /// prywatna i dostępna tylko dla wbudowanych ścieżek logowania. Dlatego
    /// zapisujemy „siebie" do sesji tutaj — bez tego wpisu `stream_updates`
    /// nie wie, że jest zalogowany, i nie nadrabia zaległych aktualizacji.
    async fn finish(&mut self, auth: tl::enums::auth::Authorization) -> anyhow::Result<LoginStage> {
        // `SignUpRequired` znaczy, że numer nie ma jeszcze konta. Aplikacje
        // spoza oficjalnych NIE MOGĄ zakładać kont — trzeba to zrobić
        // w telefonie i wrócić tutaj.
        let tl::enums::auth::Authorization::Authorization(a) = auth else {
            anyhow::bail!(
                "to konto jeszcze nie istnieje — załóż je oficjalną aplikacją Telegrama, \
                 aplikacje zewnętrzne nie mogą rejestrować nowych numerów"
            );
        };
        let tl::enums::User::User(u) = a.user else {
            anyhow::bail!("Telegram zwrócił pustego użytkownika");
        };
        let li = LoggedIn {
            user_id: u.id,
            name: u.first_name.clone().unwrap_or_default(),
            username: u.username.clone(),
        };

        self.session
            .cache_peer(&PeerInfo::User {
                id: u.id,
                auth: u
                    .access_hash
                    .map(grammers_client::session::types::PeerAuth::from_hash),
                bot: Some(u.bot),
                is_self: Some(true),
            })
            .await
            .map_err(|e| anyhow::anyhow!("zapis użytkownika do sesji: {e}"))?;

        // punkt startowy dla strumienia aktualizacji; brak stanu tylko oznacza,
        // że pierwsze aktualizacje przyjdą bez nadrabiania zaległości
        if let Ok(tl::enums::updates::State::State(st)) = self
            .client
            .invoke(&tl::functions::updates::GetState {})
            .await
        {
            let _ = self
                .session
                .set_update_state(grammers_client::session::types::UpdateState::All(
                    grammers_client::session::types::UpdatesState {
                        pts: st.pts,
                        qts: st.qts,
                        date: st.date,
                        seq: st.seq,
                        channels: Vec::new(),
                    },
                ))
                .await;
        }

        info!(user = %li.name, id = li.user_id, "Telegram: zalogowano kodem QR");
        self.current = None;
        self.persist();
        Ok(LoginStage::Done(li))
    }

    /// Zapisuje sesję natychmiast po zalogowaniu.
    ///
    /// Logowanie jest drogie (kilka prób pod rząd = blokada na godziny), więc
    /// utrata świeżo zdobytego klucza autoryzacyjnego jest znacznie gorsza niż
    /// jeden zapis na dysk w niewygodnym momencie.
    fn persist(&self) {
        if let Err(e) = self.session.save() {
            warn!(%e, "Telegram: NIE UDAŁO SIĘ zapisać sesji — po restarcie trzeba będzie logować się ponownie");
        }
    }
}

/// Czy to jest odmowa „potrzebne hasło dwuetapowe"?
fn is_password_needed(e: &InvocationError) -> bool {
    e.is("SESSION_PASSWORD_NEEDED")
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etapy_rozpoznaja_sie_nawzajem() {
        let done = LoginStage::Done(LoggedIn {
            user_id: 1,
            name: "Demo User".into(),
            username: None,
        });
        assert!(done.is_done());
        assert!(!done.needs_password());

        let pw = LoginStage::PasswordRequired {
            hint: Some("kot".into()),
        };
        assert!(pw.needs_password());
        assert!(!pw.is_done());

        let wrong = LoginStage::PasswordWrong { hint: None };
        assert!(
            wrong.needs_password(),
            "po błędnym haśle nadal czekamy na hasło"
        );
        assert!(!wrong.is_done());
    }

    #[test]
    fn kod_qr_powstaje_z_tokenu_o_realnej_dlugosci() {
        // Telegram wydaje tokeny 32-bajtowe
        let token: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(7)).collect();
        let qr = QrRender::from_token(&token).unwrap();
        assert!(qr.url.starts_with("tg://login?token="));
        assert_eq!(crate::qr::token_from_url(&qr.url).unwrap(), token);
        assert!(!qr.to_svg(4).is_empty());
        assert!(!qr.to_ascii(crate::qr::AsciiStyle::Ansi).is_empty());
    }

    #[test]
    fn czas_zycia_tokenu_jest_przycinany_do_rozsadnego_zakresu() {
        // serwer bywa niezsynchronizowany; ujemny albo absurdalny `expires`
        // nie może dać ani natychmiastowej pętli, ani czekania w nieskończoność
        let clamp = |expires: i64, now: i64| (expires - now).clamp(1, 300);
        assert_eq!(clamp(0, 1_000), 1, "przeszłość → minimum");
        assert_eq!(clamp(1_030, 1_000), 30, "typowe 30 s");
        assert_eq!(clamp(999_999, 1_000), 300, "absurd → maksimum");
    }

    #[test]
    fn margines_nie_daje_zerowego_czekania() {
        // gdyby ttl było mniejsze od marginesu, `step` kręciłby się w kółko
        // wołając exportLoginToken bez przerwy
        for ttl_s in [1u64, 2, 3, 30] {
            let ttl = Duration::from_secs(ttl_s);
            let wait = ttl
                .saturating_sub(EXPIRY_MARGIN)
                .max(Duration::from_secs(1));
            assert!(wait >= Duration::from_secs(1), "ttl {ttl_s}s dał {wait:?}");
        }
    }
}
