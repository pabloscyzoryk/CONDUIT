//! Powiadamiacz — spina politykę, kolejkę, SMTP i dziennik w jedną usługę.
//!
//! Reguła, która rządzi całym plikiem: **pełna treść zdarzenia trafia do
//! dziennika ZAWSZE**, niezależnie od tego, czy mail wyszedł, czekał w
//! kolejce, został scalony z innym, czy kategoria była wyłączona. Poczta bywa
//! zawodna, skrzynka bywa pełna, a hasło aplikacji bywa cofnięte — i wtedy
//! dziennik jest jedynym miejscem, gdzie widać, co się stało. `bot.py` przy
//! zdławionym alercie nie zapisywał NIC (`send_error_email` kończył się
//! `return` przed logiem) i zdarzenie znikało bez śladu.
//!
//! # Przepływ
//!
//! ```text
//!   notify(kategoria, temat, treść)
//!         │
//!         ├─▶ dziennik (zawsze, pełna treść)
//!         │
//!         └─▶ MailPolicy ──┬─ Send        ─▶ MailQueue ─▶ pętla ─▶ SMTP
//!                          ├─ Coalesce    ─▶ czeka na zbiorczy mail
//!                          ├─ RateLimited ─▶ jw.
//!                          └─ Disabled    ─▶ koniec (ale log już jest)
//! ```
//!
//! Wysyłka NIGDY nie dzieje się w wątku wołającego. SMTP potrafi wisieć
//! kilkadziesiąt sekund; gdyby `notify` blokował, alert o utracie połączenia
//! z MT5 zatrzymałby pętlę, która próbuje to połączenie odzyskać.

use crate::mailer::{
    parse_recipients, MailCategories, MailCategory, MailPolicy, MailQueue, MailSecurity,
    MailSender, SmtpTarget, ThrottleConfig,
};
use crate::state::{Shared, StateHandle};
use crate::mailer::i18n::{self as mail_i18n, Language};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

/// Co ile pętla zagląda do kolejki i do zaległości.
pub const TICK: Duration = Duration::from_secs(10);

// ============================================================
//  TRANSPORT SMTP
// ============================================================

/// Wysyłka przez `lettre`. Blokująca — dlatego pętla poczty woła ją przez
/// `spawn_blocking`, a nie wprost z zadania asynchronicznego.
pub struct SmtpMailer;

impl MailSender for SmtpMailer {
    fn send(&self, target: &SmtpTarget, subject: &str, body: &str) -> anyhow::Result<()> {
        use lettre::message::header::ContentType;
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{Message, SmtpTransport, Transport};

        target.is_usable().map_err(|e| anyhow::anyhow!("{e}"))?;

        let from = target.sender();
        let mut msg = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| anyhow::anyhow!("zły adres nadawcy „{from}”: {e}"))?,
            )
            .subject(subject)
            .header(ContentType::TEXT_PLAIN);
        for adres in &target.to {
            msg = msg.to(adres
                .parse()
                .map_err(|e| anyhow::anyhow!("zły adres odbiorcy „{adres}”: {e}"))?);
        }
        let msg = msg.body(body.to_string())?;

        let builder = match target.security {
            MailSecurity::Starttls => SmtpTransport::starttls_relay(&target.host)?,
            MailSecurity::Ssl => SmtpTransport::relay(&target.host)?,
            // Bez szyfrowania hasło poszłoby po sieci jawnym tekstem, więc ten
            // wariant ma sens wyłącznie dla przekaźnika na tej samej maszynie.
            MailSecurity::None => SmtpTransport::builder_dangerous(&target.host),
        };
        let mut builder = builder
            .port(target.port)
            .timeout(Some(Duration::from_secs(30)));
        if !target.user.trim().is_empty() {
            builder = builder.credentials(Credentials::new(
                target.user.clone(),
                target.password.clone(),
            ));
        }
        builder.build().send(&msg)?;
        Ok(())
    }
}

// ============================================================
//  USŁUGA
// ============================================================

/// Konfiguracja wyliczona ze stanu (ustawienia + sekret z `secrets.json`).
#[derive(Debug, Clone)]
pub struct MailSetup {
    pub enabled: bool,
    pub target: SmtpTarget,
    pub throttle: ThrottleConfig,
    pub categories: MailCategories,
    /// Szablon tematu ze zmiennymi `${...}`. Puste = temat systemowy.
    pub subject_tpl: String,
}

/// Prefiks tematu systemowego — jak w `bot.py`, żeby filtry w skrzynce
/// użytkownika dalej działały po przesiadce na CONDUIT.
pub const TAG: &str = "[CONDUIT]";

/// Temat systemowy: `[CONDUIT] kategoria — zdarzenie`.
///
/// Publiczna funkcja, a nie literał w dwóch miejscach: dokładnie ten sam
/// łańcuch pokazuje podgląd w panelu przy pustym szablonie
/// (`rest::email_subject`). Rozjazd tych dwóch napisów oznaczałby, że panel
/// obiecuje inny temat, niż wychodzi w mailu.
pub fn temat_systemowy(cat: MailCategory, subject: &str) -> String {
    format!("{TAG} {} — {}", cat.label(), subject)
}

pub struct Notifier {
    policy: Mutex<MailPolicy>,
    queue: MailQueue,
    sender: Arc<dyn MailSender>,
    setup: Mutex<MailSetup>,
}

impl Notifier {
    pub fn new(
        setup: MailSetup,
        sender: Arc<dyn MailSender>,
        queue_path: Option<std::path::PathBuf>,
    ) -> Self {
        let policy = MailPolicy::new(setup.throttle.clone(), setup.categories.clone());
        Notifier {
            policy: Mutex::new(policy),
            queue: MailQueue::new(queue_path),
            sender,
            setup: Mutex::new(setup),
        }
    }

    /// Podmienia konfigurację w locie — po zapisaniu ustawień z panelu.
    pub fn reconfigure(&self, setup: MailSetup) {
        self.policy
            .lock()
            .set_config(setup.throttle.clone(), setup.categories.clone());
        *self.setup.lock() = setup;
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn pending_coalesced(&self) -> u32 {
        self.policy.lock().pending_total()
    }

    fn temat_i_jezyk(&self, st: &Shared, cat: MailCategory, event: &str) -> (String, Language) {
        let tpl = self.setup.lock().subject_tpl.clone();
        st.read(|snap| (
            mail_i18n::subject(snap, cat, event, &tpl, crate::now_ms()),
            Language::from_app(&snap.language),
        ))
    }

    /// ZGŁOSZENIE ZDARZENIA. Nie blokuje i nie wysyła — tylko loguje
    /// i (być może) kolejkuje.
    pub fn notify(&self, st: &Shared, cat: MailCategory, subject: &str, body: &str) {
        let now = crate::now_ms();

        // 1. DZIENNIK — zawsze i w całości, zanim cokolwiek może pójść nie tak
        let poziom = match cat {
            MailCategory::Drawdown | MailCategory::Mt5RecoveryFailed => "error",
            MailCategory::Mt5Connection | MailCategory::OrderError => "warn",
            _ => "info",
        };
        st.log("email", poziom, subject.to_string(), body.to_string());

        // 2. POCZTA
        //
        // Uchwyt do `setup` bierzemy i oddajemy w jednym wyrażeniu: `temat`
        // sięga po ten sam zamek (po szablon), a `parking_lot::Mutex` nie jest
        // wznawialny — zagnieżdżenie zakleszczyłoby pierwsze powiadomienie.
        let wlaczona = self.setup.lock().enabled;
        if !wlaczona {
            return;
        }
        let (temat, language) = self.temat_i_jezyk(st, cat, subject);

        let decyzja = self.policy.lock().offer(cat, subject, now);
        use crate::mailer::Decision;
        match decyzja {
            Decision::Send => {
                self.queue.push(cat, temat, mail_i18n::body(language, body), now);
            }
            Decision::Coalesce { pending } | Decision::RateLimited { pending } => {
                tracing::debug!(
                    kategoria = cat.key(),
                    czeka = pending,
                    "powiadomienie scalone — wyjdzie zbiorczo"
                );
            }
            Decision::Disabled => {
                tracing::debug!(
                    kategoria = cat.key(),
                    "kategoria maili wyłączona w ustawieniach"
                );
            }
        }
    }

    /// Mail testowy z przycisku w ustawieniach.
    ///
    /// Jedyna droga, która wysyła SYNCHRONICZNIE i zwraca błąd wprost:
    /// użytkownik nacisnął przycisk po to, żeby zobaczyć, czy konfiguracja
    /// działa. Odpowiedź „dodano do kolejki" nie odpowiada na to pytanie.
    pub fn send_test(&self, st: &Shared) -> anyhow::Result<String> {
        let setup = self.setup.lock().clone();
        setup
            .target
            .is_usable()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // Mail testowy leci Z TYM SAMYM tematem, co produkcyjny — inaczej
        // przycisk „wyślij mail testowy" nie sprawdzałby tego, co użytkownik
        // właśnie wpisał, a to jest jedyny powód, dla którego się go naciska.
        let (temat, language) = self.temat_i_jezyk(st, MailCategory::Test, "mail testowy");
        let tresc = format!(
            "To jest wiadomość testowa z CONDUIT.\n\n\
             Serwer SMTP: {host}:{port} ({sec})\n\
             Nadawca:     {from}\n\
             Odbiorcy:    {to}\n\
             Czas:        {czas}\n\n\
             Jeśli ją widzisz, powiadomienia e-mail działają.",
            host = setup.target.host,
            port = setup.target.port,
            sec = match setup.target.security {
                MailSecurity::Starttls => "STARTTLS",
                MailSecurity::Ssl => "SSL/TLS",
                MailSecurity::None => language.choose("bez szyfrowania", "unencrypted"),
            },
            from = setup.target.sender(),
            to = setup.target.to.join(", "),
            czas = crate::store::stamp(crate::now_ms()),
        );

        let tresc = mail_i18n::body(language, &tresc);
        match self.sender.send(&setup.target, &temat, &tresc) {
            Ok(()) => {
                let ile = setup.target.to.len();
                st.log(
                    "email",
                    "success",
                    "Mail testowy wysłany",
                    format!("do {} adresatów: {}", ile, setup.target.to.join(", ")),
                );
                Ok(format!("Wysłano do: {}", setup.target.to.join(", ")))
            }
            Err(e) => {
                // Komunikat błędu SMTP potrafi zawierać login, ale NIGDY hasło
                // — lettre go nie wypisuje. Mimo to przycinamy, żeby nie wlać
                // do dziennika kilobajta odpowiedzi serwera.
                let opis: String = e.to_string().chars().take(400).collect();
                st.log("email", "error", "Mail testowy NIE wyszedł", opis.clone());
                Err(anyhow::anyhow!(opis))
            }
        }
    }

    /// Jeden obrót pętli: wypuszcza zaległe zbiorcze i próbuje wysłać
    /// najstarszą wiadomość z kolejki.
    ///
    /// Wysyła NAJWYŻEJ JEDNĄ wiadomość na obrót. Serwery poczty karzą za
    /// serie połączeń, a przy odzyskaniu sieci kolejka i tak rozejdzie się
    /// w kilkanaście sekund.
    pub fn tick(&self, st: &Shared) {
        let now = crate::now_ms();

        // zaległe zbiorcze → do kolejki
        let due = self.policy.lock().take_due(now);
        for (cat, ile, body) in due {
            let (temat, language) = self.temat_i_jezyk(st, cat, &format!("{ile} zdarzeń"));
            let konto = st.read(|s| {
                if s.connection.account.login != 0 {
                    format!(
                        "Rachunek {} · {} · saldo {:.2}\n\n",
                        s.connection.account.login, s.connection.account.server, s.stats.balance
                    )
                } else {
                    String::new()
                }
            });
            let tresc = format!(
                "{konto}W ostatnim oknie dławienia wystąpiło {ile} zdarzeń kategorii „{}”:\n\n{body}",
                cat.label()
            );
            st.log(
                "email",
                "info",
                format!("Zbiorcze powiadomienie: {ile} × {}", cat.label()),
                tresc.clone(),
            );
            self.queue.push(cat, temat, mail_i18n::body(language, &tresc), now);
        }

        let (wlaczona, target) = {
            let s = self.setup.lock();
            (s.enabled, s.target.clone())
        };
        if !wlaczona {
            return;
        }
        let Some(m) = self.queue.next_due(now) else {
            return;
        };

        if let Err(e) = target.is_usable() {
            // Konfiguracja niekompletna: nie ma sensu palić prób. Wiadomość
            // zostaje w kolejce i wyjdzie, gdy użytkownik uzupełni dane.
            tracing::debug!(powod = %e, "wysyłka wstrzymana — konfiguracja SMTP niekompletna");
            return;
        }

        match self.sender.send(&target, &m.subject, &m.body) {
            Ok(()) => {
                self.queue.remove(m.id);
                tracing::info!(temat = %m.subject, "e-mail wysłany");
            }
            Err(e) => {
                let opis: String = e.to_string().chars().take(300).collect();
                let zostaje = self.queue.defer(m.id, now, &opis);
                if zostaje {
                    tracing::warn!(temat = %m.subject, blad = %opis, "wysyłka nieudana — ponowię");
                } else {
                    st.log(
                        "email",
                        "error",
                        format!("Porzucono powiadomienie: {}", m.subject),
                        format!(
                            "Nie udało się wysłać po {} próbach. Ostatni błąd: {opis}\n\n\
                             Treść zdarzenia (zachowana w dzienniku):\n{}",
                            crate::mailer::MAX_PROB,
                            m.body
                        ),
                    );
                }
            }
        }
    }
}

impl std::fmt::Debug for Notifier {
    /// Hasło SMTP jest w `setup.target.password` — dlatego `Debug` jest ręczny.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.setup.lock();
        f.debug_struct("Notifier")
            .field("wlaczony", &s.enabled)
            .field("serwer", &format!("{}:{}", s.target.host, s.target.port))
            .field("odbiorcow", &s.target.to.len())
            .field("haslo", &crate::secrets::mask(&s.target.password))
            .field("w_kolejce", &self.queue.len())
            .finish()
    }
}

// ============================================================
//  BUDOWANIE KONFIGURACJI ZE STANU
// ============================================================

/// Składa konfigurację poczty z dokumentu ustawień i pliku sekretów.
///
/// Hasło przychodzi WYŁĄCZNIE z `secrets.json`. Pole `pass` w `EmailConfig`
/// jest kanałem jednokierunkowym: UI może nim ustawić nowe hasło, ale nigdy
/// go stamtąd nie odczyta (serwer wysyła puste).
pub fn setup_from_state(st: &Shared) -> MailSetup {
    let (email, kategorie, throttle) = st.read(|s| {
        (
            s.email.clone(),
            s.email.categories.clone(),
            s.email.throttle.clone(),
        )
    });
    let haslo = st.workspace.load_secrets().smtp.password.into_inner();
    MailSetup {
        enabled: email.enabled,
        target: SmtpTarget {
            host: email.host.clone(),
            port: email.port,
            security: email.security,
            user: email.user.clone(),
            password: haslo,
            from: email.from.clone(),
            to: parse_recipients(&email.to),
        },
        throttle,
        categories: kategorie,
        subject_tpl: email.subject.clone(),
    }
}

/// Pętla poczty. Startowana raz, obok pętli delt i kopii zapasowej.
pub async fn mail_loop(st: StateHandle, notifier: Arc<Notifier>) {
    let mut co = tokio::time::interval(TICK);
    co.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        co.tick().await;
        let n = Arc::clone(&notifier);
        let s = st.clone();
        // SMTP blokuje — nie wolno mu zatrzymać środowiska asynchronicznego
        if let Err(e) = tokio::task::spawn_blocking(move || n.tick(&s)).await {
            tracing::warn!(blad = %e, "pętla poczty: zadanie wysyłki padło");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailer::MailCategory;

    /// Atrapa wysyłki: liczy próby i potrafi udawać awarię sieci.
    #[derive(Default)]
    struct Atrapa {
        wyslane: Mutex<Vec<(String, String)>>,
        proby: std::sync::atomic::AtomicU32,
        psuj: std::sync::atomic::AtomicBool,
    }

    impl Atrapa {
        fn proby(&self) -> u32 {
            self.proby.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn psuj(&self, tak: bool) {
            self.psuj.store(tak, std::sync::atomic::Ordering::Relaxed);
        }
        fn wyslane(&self) -> Vec<(String, String)> {
            self.wyslane.lock().clone()
        }
    }

    impl MailSender for Atrapa {
        fn send(&self, _t: &SmtpTarget, subject: &str, body: &str) -> anyhow::Result<()> {
            self.proby
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.psuj.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("sieć nie odpowiada");
            }
            self.wyslane
                .lock()
                .push((subject.to_string(), body.to_string()));
            Ok(())
        }
    }

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-notify-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        let st = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        st.update(crate::coalesce::Sections::one(crate::coalesce::Section::Settings), |snap| snap.language = "pl".into());
        st
    }

    fn setup() -> MailSetup {
        MailSetup {
            enabled: true,
            target: SmtpTarget {
                host: "smtp.example.com".into(),
                port: 587,
                security: MailSecurity::Starttls,
                user: "bot@example.com".into(),
                password: "tajne".into(),
                from: String::new(),
                to: vec!["ja@example.com".into()],
            },
            throttle: ThrottleConfig {
                window_min: 10.0,
                max_per_hour: 12,
            },
            categories: MailCategories::default(),
            subject_tpl: String::new(),
        }
    }

    #[test]
    fn english_delivery_uses_app_language_and_preserves_original_journal() {
        let st = stan("language-en");
        st.update(crate::coalesce::Sections::one(crate::coalesce::Section::Settings), |snap| snap.language = "en".into());
        let fake = Arc::new(Atrapa::default());
        let n = Notifier::new(setup(), fake.clone() as Arc<dyn MailSender>, None);
        let subject = "Niepełne potwierdzenie zamknięcia — blokada nowych wejść";
        let body = "Odczyt stanu brokera zakończony; tymczasowa bramka wejść zdjęta.";
        n.notify(&st, MailCategory::OrderError, subject, body);
        n.tick(&st);
        let sent = fake.wyslane();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "[CONDUIT] order error — Incomplete close confirmation — new entries blocked");
        assert_eq!(sent[0].1, "Broker state read completed; temporary entry gate removed.");
        assert!(st.read(|snap| snap.logs.iter().any(|entry| entry.title == subject && entry.content == body)));
    }

    #[test]
    fn test_email_is_localized_and_language_switch_applies_to_next_event() {
        let st = stan("language-test");
        let fake = Arc::new(Atrapa::default());
        let n = Notifier::new(setup(), fake.clone() as Arc<dyn MailSender>, None);
        st.update(crate::coalesce::Sections::one(crate::coalesce::Section::Settings), |snap| snap.language = "en".into());
        n.send_test(&st).unwrap();
        st.update(crate::coalesce::Sections::one(crate::coalesce::Section::Settings), |snap| snap.language = "pl".into());
        n.send_test(&st).unwrap();
        let sent = fake.wyslane();
        assert_eq!(sent[0].0, "[CONDUIT] test — test email");
        assert!(sent[0].1.starts_with("This is a test message from CONDUIT."));
        assert!(sent[0].1.contains("SMTP server: smtp.example.com:587 (STARTTLS)"));
        assert!(sent[1].1.starts_with("To jest wiadomość testowa z CONDUIT."));
        assert!(sent.iter().all(|(_, body)| !body.contains("tajne")));
    }

    #[test]
    fn zdarzenie_trafia_do_dziennika_nawet_gdy_poczta_wylaczona() {
        // TO JEST SEDNO: bot.py przy zdławionym alercie nie zapisywał NIC
        let st = stan("log");
        let n = Notifier::new(
            MailSetup {
                enabled: false,
                ..setup()
            },
            Arc::new(Atrapa::default()),
            None,
        );
        n.notify(
            &st,
            MailCategory::Drawdown,
            "MAX DD 40%",
            "equity spadło o 40% od szczytu",
        );

        let logi = st.read(|s| s.logs.clone());
        let wpis = logi
            .iter()
            .find(|l| l.title.contains("MAX DD"))
            .expect("wpis w dzienniku");
        assert_eq!(wpis.category, "email");
        assert_eq!(wpis.level, "error");
        assert!(
            wpis.content.contains("40%"),
            "pełna treść musi być w logu: {}",
            wpis.content
        );
        assert_eq!(
            n.queue_len(),
            0,
            "przy wyłączonej poczcie nic nie kolejkujemy"
        );
    }

    #[test]
    fn zdlawione_zdarzenie_tez_jest_w_dzienniku() {
        let st = stan("dlaw");
        let n = Notifier::new(setup(), Arc::new(Atrapa::default()), None);
        for i in 0..5 {
            n.notify(
                &st,
                MailCategory::OrderError,
                &format!("odrzucone zlecenie {i}"),
                "retcode 10016",
            );
        }
        let logi = st.read(|s| s.logs.clone());
        let ile = logi
            .iter()
            .filter(|l| l.title.starts_with("odrzucone zlecenie"))
            .count();
        assert_eq!(ile, 5, "każde zdarzenie ma swój wpis, także zdławione");
        assert_eq!(n.queue_len(), 1, "ale mail wychodzi jeden");
        assert_eq!(n.pending_coalesced(), 4);
    }

    #[test]
    fn kolejka_wysyla_po_ustaniu_awarii_sieci() {
        let st = stan("siec");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(setup(), atrapa.clone() as Arc<dyn MailSender>, None);

        atrapa.psuj(true);
        n.notify(
            &st,
            MailCategory::Mt5Connection,
            "utracono MT5",
            "terminal zniknął",
        );
        assert_eq!(n.queue_len(), 1);

        n.tick(&st);
        assert_eq!(atrapa.proby(), 1);
        assert_eq!(n.queue_len(), 1, "nieudana wysyłka zostaje w kolejce");
        assert!(atrapa.wyslane().is_empty());

        // ponowienie jest ODŁOŻONE — kolejny tick nie próbuje od razu
        n.tick(&st);
        assert_eq!(atrapa.proby(), 1, "ponowienie bez odstępu to zapętlenie");

        atrapa.psuj(false);
        // symulujemy upływ czasu, cofając termin ponowienia
        std::thread::sleep(std::time::Duration::from_millis(20));
        for _ in 0..30 {
            n.tick(&st);
            if n.queue_len() == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
        assert_eq!(n.queue_len(), 0, "po powrocie sieci alert musi dojść");
        let w = atrapa.wyslane();
        assert_eq!(w.len(), 1);
        assert!(w[0].0.contains("połączenie z MT5"), "{}", w[0].0);
        assert!(w[0].1.contains("terminal zniknął"));
    }

    #[test]
    fn mail_testowy_zwraca_blad_wprost_a_nie_kolejkuje() {
        let st = stan("test");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(setup(), atrapa.clone() as Arc<dyn MailSender>, None);

        atrapa.psuj(true);
        let e = n.send_test(&st).unwrap_err();
        assert!(e.to_string().contains("sieć"), "{e}");
        assert_eq!(
            n.queue_len(),
            0,
            "test nie ląduje w kolejce — ma odpowiedzieć od razu"
        );

        atrapa.psuj(false);
        let ok = n.send_test(&st).unwrap();
        assert!(ok.contains("ja@example.com"), "{ok}");
        assert_eq!(atrapa.wyslane().len(), 1);
    }

    #[test]
    fn mail_testowy_bez_odbiorcy_nie_probuje_sieci() {
        let st = stan("bezodb");
        let atrapa = Arc::new(Atrapa::default());
        let mut s = setup();
        s.target.to.clear();
        let n = Notifier::new(s, atrapa.clone() as Arc<dyn MailSender>, None);
        assert!(n.send_test(&st).is_err());
        assert_eq!(
            atrapa.proby(),
            0,
            "brak odbiorcy wykrywamy przed połączeniem"
        );
    }

    #[test]
    fn zbiorczy_mail_wychodzi_po_zamknieciu_okna() {
        let st = stan("zbior");
        let atrapa = Arc::new(Atrapa::default());
        // okno zerowe: pierwszy mail idzie, kolejne wpadają w limit godzinowy
        let mut s = setup();
        s.throttle = ThrottleConfig {
            window_min: 0.0,
            max_per_hour: 1,
        };
        let n = Notifier::new(s, atrapa.clone() as Arc<dyn MailSender>, None);

        n.notify(&st, MailCategory::OrderError, "błąd A", "treść A");
        n.notify(&st, MailCategory::OrderError, "błąd B", "treść B");
        assert_eq!(n.pending_coalesced(), 1);

        n.tick(&st);
        // pierwszy poszedł; zaległy czeka na odnowienie limitu godzinowego
        assert_eq!(atrapa.wyslane().len(), 1);
        assert_eq!(n.pending_coalesced(), 1);
    }

    #[test]
    fn debug_powiadamiacza_nie_wypisuje_hasla() {
        let n = Notifier::new(setup(), Arc::new(Atrapa::default()), None);
        let d = format!("{n:?}");
        assert!(!d.contains("tajne"), "hasło SMTP wyciekło do Debug: {d}");
        assert!(d.contains("smtp.example.com:587"), "{d}");
    }

    // ---------------- własny temat maila ----------------

    #[test]
    fn pusty_szablon_zostawia_temat_systemowy() {
        // REGRESJA: cała reszta programu (filtry w skrzynce użytkownika) stoi
        // na prefiksie `[CONDUIT]`. Dodanie pola „własny temat" nie ma prawa
        // zmienić zachowania nikomu, kto go nie wypełnił.
        let st = stan("temat-pusty");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(setup(), atrapa.clone() as Arc<dyn MailSender>, None);
        n.notify(&st, MailCategory::Drawdown, "MAX DD 40%", "equity −40%");
        n.tick(&st);
        let w = atrapa.wyslane();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].0, "[CONDUIT] limit obsunięcia — MAX DD 40%");
    }

    #[test]
    fn wlasny_szablon_wchodzi_do_kolejki_z_podstawionymi_wartosciami() {
        let st = stan("temat-wlasny");
        st.update(
            crate::coalesce::Sections::one(crate::coalesce::Section::Settings),
            |s| {
                s.stats.balance = 237.65;
                s.connection.user.name = "Demo User".into();
            },
        );

        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(
            MailSetup {
                subject_tpl: "Aktualny balans: ${balance} (${user}) ${balance}".into(),
                ..setup()
            },
            atrapa.clone() as Arc<dyn MailSender>,
            None,
        );
        n.notify(
            &st,
            MailCategory::Mt5Connection,
            "utracono MT5",
            "terminal zniknął",
        );
        n.tick(&st);

        let w = atrapa.wyslane();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].0, "Aktualny balans: $237.65 (Demo User) $237.65");
        // treść zdarzenia zostaje nietknięta — szablon dotyczy TYLKO tematu
        assert!(w[0].1.contains("terminal zniknął"));
    }

    #[test]
    fn szablon_z_literowka_pokazuje_ja_w_temacie() {
        // Cichy zanik `${blans}` zostawiłby użytkownika z tematem „Saldo: "
        // i bez żadnej wskazówki, gdzie leży błąd.
        let st = stan("temat-literowka");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(
            MailSetup {
                subject_tpl: "Saldo: ${blans}".into(),
                ..setup()
            },
            atrapa.clone() as Arc<dyn MailSender>,
            None,
        );
        n.notify(&st, MailCategory::Lifecycle, "start", "");
        n.tick(&st);
        assert_eq!(atrapa.wyslane()[0].0, "Saldo: ${blans}");
    }

    #[test]
    fn szablon_obowiazuje_takze_mail_testowy() {
        // Przycisk „wyślij mail testowy" ma sprawdzać TO, co użytkownik
        // właśnie wpisał. Systemowy temat w mailu testowym przy własnym
        // szablonie w produkcji byłby diagnostyką, która diagnozuje co innego.
        let st = stan("temat-test");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(
            MailSetup {
                subject_tpl: "BOT ${kategoria}: ${zdarzenie}".into(),
                ..setup()
            },
            atrapa.clone() as Arc<dyn MailSender>,
            None,
        );

        n.send_test(&st).unwrap();
        assert_eq!(atrapa.wyslane()[0].0, "BOT test: mail testowy");

        // ta sama droga dla powiadomienia z pętli handlowej
        n.notify(
            &st,
            MailCategory::OrderError,
            "invalid stops",
            "retcode 10016",
        );
        n.tick(&st);
        let tematy: Vec<String> = atrapa.wyslane().iter().map(|x| x.0.clone()).collect();
        assert!(
            tematy
                .iter()
                .any(|t| t == "BOT błąd zlecenia: invalid stops"),
            "{tematy:?}"
        );
    }

    #[test]
    fn zmiana_szablonu_w_locie_dziala_bez_restartu() {
        let st = stan("temat-locie");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(setup(), atrapa.clone() as Arc<dyn MailSender>, None);
        n.reconfigure(MailSetup {
            subject_tpl: "NOWY ${zdarzenie}".into(),
            ..setup()
        });
        n.notify(&st, MailCategory::Lifecycle, "start bota", "");
        n.tick(&st);
        assert_eq!(atrapa.wyslane()[0].0, "NOWY start bota");
    }

    #[test]
    fn temat_z_zalamaniem_wiersza_jest_prostowany() {
        // Nagłówek `Subject:` z `\n` to w najlepszym razie ucięty temat,
        // w najgorszym odrzucona wiadomość przez serwer poczty.
        let st = stan("temat-nowa-linia");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(
            MailSetup {
                subject_tpl: "  linia\ndruga  ".into(),
                ..setup()
            },
            atrapa.clone() as Arc<dyn MailSender>,
            None,
        );
        n.notify(&st, MailCategory::Lifecycle, "start", "");
        n.tick(&st);
        assert_eq!(atrapa.wyslane()[0].0, "linia druga");
    }

    #[test]
    fn temat_przezywa_restart_bo_lezy_w_smtp_json() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-temat-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        {
            let st = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
            let e = crate::ui::EmailConfig {
                enabled: true,
                to: "ja@example.com".into(),
                host: "smtp.example.com".into(),
                subject: "Balans ${balance} · ${data}".into(),
                ..Default::default()
            };
            crate::commands::apply(&st, &crate::proto::Command::SetEmail { email: e }).unwrap();
        }
        let st2 = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        assert_eq!(
            st2.read(|s| s.email.subject.clone()),
            "Balans ${balance} · ${data}"
        );
        assert_eq!(
            setup_from_state(&st2).subject_tpl,
            "Balans ${balance} · ${data}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn zmiana_ustawien_dziala_w_locie() {
        let st = stan("recfg");
        let atrapa = Arc::new(Atrapa::default());
        let n = Notifier::new(
            MailSetup {
                enabled: false,
                ..setup()
            },
            atrapa.clone() as Arc<dyn MailSender>,
            None,
        );
        n.notify(&st, MailCategory::Lifecycle, "start", "");
        assert_eq!(n.queue_len(), 0);

        n.reconfigure(setup());
        n.notify(&st, MailCategory::Lifecycle, "start 2", "");
        assert_eq!(
            n.queue_len(),
            1,
            "po włączeniu poczty zdarzenia mają się kolejkować"
        );
    }
}
