
use crate::ui;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `RRRR-MM-DD_GG-MM-SS` z czasu w ms — bez zależności od formatera dat.
pub fn stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let day = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

#[cfg(test)]
mod stamp_tests {
    use super::stamp;

    #[test]
    fn znacznik_ma_date_i_godzine() {
        // 2026-07-27 11:58:07 UTC
        assert_eq!(stamp(1_785_153_487_000), "2026-07-27_11-58-07");
        assert_eq!(stamp(0), "1970-01-01_00-00-00");
    }

    #[test]
    fn kolejne_eksporty_nie_nadpisuja_sie() {
        assert_ne!(stamp(1_785_153_487_000), stamp(1_785_153_488_000));
    }
}

/// Wersja formatu `backup_memory`. Podbijamy przy każdej niekompatybilnej
/// zmianie; starsze pliki są wtedy odrzucane z ostrzeżeniem, a nie
/// wczytywane „na chybił trafił".
pub const BACKUP_VERSION: u32 = 1;

/// Ile kopii rotacyjnych trzymamy.
pub const BACKUP_KEEP: usize = 20;

// ============================================================
//  DOKUMENTY KONFIGURACJI
// ============================================================

/// `settings.json` — wszystko, co użytkownik ustawia w panelu.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDoc {
    #[serde(default)]
    pub mode: ui::TradingMode,
    #[serde(default)]
    pub preset_id: String,
    #[serde(default)]
    pub lot: ui::LotConfig,
    #[serde(default)]
    pub favorites: Vec<String>,
    /// JĘZYK INTERFEJSU — na POZIOMIE GŁÓWNYM dokumentu (obok `favorites`),
    /// nie w `settings{}`: to preferencja panelu, nie pole silnika, więc nie
    /// ma prawa liczyć się do „ustawień", zdejmować etykiety presetu ani
    /// wchodzić do żadnego porównania konfiguracji handlu (JEZYKI.md §5).
    #[serde(default = "jezyk_domyslny")]
    pub language: String,
    /// Surowy dokument 149 kluczy z panelu React. Trzymamy go w oryginale,
    /// żeby dodanie nowego ustawienia w UI nie wymagało zmiany w Ruście.
    #[serde(default)]
    pub settings: serde_json::Value,
}

impl Default for SettingsDoc {
    fn default() -> Self {
        SettingsDoc {
            mode: ui::TradingMode::Auto,
            preset_id: String::new(),
            lot: ui::LotConfig::default(),
            favorites: Vec::new(),
            language: jezyk_domyslny(),
            settings: serde_json::Value::Object(Default::default()),
        }
    }
}

fn jezyk_domyslny() -> String {
    "en".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Paczka {
    /// nazwa wysyłki, np. „VPSREADY"
    #[serde(default)]
    pub nazwa: String,
    /// kiedy złożona — tekst do wyświetlenia, nie do porównywania
    #[serde(default)]
    pub zbudowano: String,
    /// łańcuch, który MA być aktywny po wgraniu
    #[serde(default)]
    pub aktywny_lancuch: String,
    /// nogi handlujące: format → preset
    #[serde(default)]
    pub presety_nog: std::collections::BTreeMap<String, String>,
}

/// `smtp.json`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpDoc {
    #[serde(default)]
    pub email: ui::EmailConfig,
    #[serde(default)]
    pub notify: ui::NotifyConfig,
}

/// `channels.json`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsDoc {
    #[serde(default)]
    pub bindings: BTreeMap<String, ui::ChannelBinding>,
}

// ============================================================
//  PAMIĘĆ STANU
// ============================================================

/// Migawka stanu bota zapisywana do `backup_memory/`.
///
/// Zakres jest wybrany pod JEDEN cel: po restarcie bot ma wiedzieć, co
/// prowadził, zanim jeszcze odezwie się broker. Pozycje i zlecenia to
/// **ostatni znany stan do rekoncyliacji** — źródłem prawdy po starcie
/// zawsze pozostaje broker; gdyby było odwrotnie, restart po ręcznym
/// zamknięciu pozycji w terminalu wskrzeszałby duchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupMemory {
    pub version: u32,
    pub saved_at: i64,
    pub session_start: i64,
    pub balance: f64,
    pub stats: ui::Stats,
    pub halt: ui::HaltState,
    pub risk_override: ui::RiskOverride,
    #[serde(default)]
    pub baskets: Vec<ui::Basket>,
    #[serde(default)]
    pub positions: Vec<ui::Position>,
    #[serde(default)]
    pub pendings: Vec<ui::PendingOrder>,
    #[serde(default)]
    pub closed: Vec<ui::ClosedPosition>,
    #[serde(default)]
    pub pending_history: Vec<ui::PendingHistoryItem>,
    /// ostatnie wiadomości — dzięki nim wiązanie „odpowiedź → koszyk"
    /// przeżywa restart; bez tego pierwsze „TP1 HIT" po restarcie
    /// nie trafiłoby do żadnego koszyka
    #[serde(default)]
    pub messages: Vec<ui::ChatMessage>,
    #[serde(default)]
    pub next_basket_id: u32,
    /// Stan DRABINKI ŁAŃCUCHÓW TRYBÓW AUTO/MANUAL/AI — szczebel, na którym
    /// stała, i konfiguracja.
    ///
    /// Persystowany po to, żeby restart w środku drabinki nie oscylował:
    /// bez `biezacy_prog` histereza nie ma punktu odniesienia i konto tuż
    /// pod progiem przełączałoby łańcuch przy każdym starcie.
    #[serde(default)]
    pub drabinka: ui::DrabinkaLancuchow,
    /// To samo dla DRABINKI TRYBU AUTO-EA („SKYNET-1", projekt EA-2c).
    ///
    /// Osobne pole, bo osobny jest szczebel bieżący: gdyby obie drabinki
    /// dzieliły `biezacy_prog`, przełączenie trybu podawałoby histerezie
    /// punkt odniesienia z cudzej drogi. Kontrakt zera: pamięć sprzed EA-2c
    /// tego klucza nie ma → drabinka EA wstaje pusta i wyłączona.
    #[serde(default = "ui::drabinka_ea_domyslna")]
    pub drabinka_ea: ui::DrabinkaLancuchow,
}

impl BackupMemory {
    pub fn from_snapshot(s: &ui::UiSnapshot, now: i64) -> Self {
        BackupMemory {
            version: BACKUP_VERSION,
            saved_at: now,
            session_start: s.stats.session_start,
            balance: s.balance,
            stats: s.stats.clone(),
            halt: s.halt.clone(),
            risk_override: s.risk_override.clone(),
            baskets: s.baskets.clone(),
            positions: s.positions.clone(),
            pendings: s.pendings.clone(),
            closed: s.closed.iter().take(500).cloned().collect(),
            pending_history: s.pending_history.iter().take(500).cloned().collect(),
            messages: s.messages.iter().rev().take(200).rev().cloned().collect(),
            next_basket_id: s.baskets.iter().map(|b| b.id + 1).max().unwrap_or(1),
            drabinka: s.drabinka.clone(),
            drabinka_ea: s.drabinka_ea.clone(),
        }
    }

    /// Nakłada zapamiętany stan na świeży snapshot.
    pub fn apply_to(&self, s: &mut ui::UiSnapshot) {
        s.balance = self.balance;
        s.stats = self.stats.clone();
        s.halt = self.halt.clone();
        s.risk_override = self.risk_override.clone();
        s.drabinka = self.drabinka.clone();
        s.drabinka_ea = self.drabinka_ea.clone();
        // SKUTECZNOŚĆ PRZELICZA SIĘ PO WZNOWIENIU, nie wraca z pliku: `enabled`
        // zapisane w pamięci opisuje tryb, w którym bot był GASZONY, a wstaje
        // w tym, który stoi w `settings.json`. Bez tego drabinka nie-swojego
        // trybu ożywałaby na jeden przebieg pętli po restarcie.
        ui::przelicz_izolacje_drabinek(s);
        s.baskets = self.baskets.clone();
        s.positions = self.positions.clone();
        s.pendings = self.pendings.clone();
        s.closed = self.closed.clone();
        s.pending_history = self.pending_history.clone();
        s.messages = self.messages.clone();
    }
}

// ============================================================
//  KATALOG ROBOCZY
// ============================================================

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Workspace { root: root.into() }
    }

    pub fn next_to_exe() -> Result<Self> {
        let exe = std::env::current_exe().context("nie udało się ustalić ścieżki programu")?;
        let dir = exe
            .parent()
            .context("program nie ma katalogu nadrzędnego")?
            .to_path_buf();
        Ok(Workspace::new(dir))
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    pub fn smtp_path(&self) -> PathBuf {
        self.root.join("smtp.json")
    }
    /// Poświadczenia. ŚWIADOMIE osobny plik od `settings.json` — ten pierwszy
    /// wolno pokazać komukolwiek, tego drugiego nikomu.
    pub fn secrets_path(&self) -> PathBuf {
        self.root.join("secrets.json")
    }
    pub fn channels_path(&self) -> PathBuf {
        self.root.join("channels.json")
    }
    /// ŁAŃCUCHY (`format → preset` + pułapy globalne).
    ///
    /// Osobny plik od `settings.json` świadomie: łańcuch rozstrzyga, ILE
    /// silników pracuje i czym, więc jest opisem konfiguracji CAŁEGO bota,
    /// a nie jednym z trzystu pól strategii. Wrzucenie go do dokumentu
    /// ustawień znaczyłoby, że wgranie cudzego presetu przestawia rachunek
    /// na inne kanały.
    pub fn lancuchy_path(&self) -> PathBuf {
        self.root.join("lancuchy.json")
    }
    /// Konfiguracja trybu demo. Świadomie OSOBNY plik od `settings.json`:
    /// to nie jest ustawienie strategii, tylko opis stanowiska testowego
    /// (skąd dane, jak szybko, od kiedy) — preset strategii nie ma prawa
    /// nieść ze sobą ścieżek do plików na czyimś dysku.
    pub fn demo_path(&self) -> PathBuf {
        self.root.join("demo.json")
    }
    pub fn presets_dir(&self) -> PathBuf {
        self.root.join("presets")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Dziennik zdarzeń (`.jsonl` + lustro `.log`), jeden plik na dobę
    /// handlową serwera. Świadomie OSOBNY katalog od `logs/`: tamte pliki są
    /// zrzutami panelu do czytania, a te są danymi wejściowymi analizatora
    /// i nie wolno ich mieszać z niczym, co nie ma schematu.
    pub fn journal_dir(&self) -> PathBuf {
        self.logs_dir().join("journal")
    }

    /// ARCHIWUM WIADOMOŚCI z kanałów (`.jsonl`, jeden plik na dobę).
    ///
    /// Świadomie OSOBNY katalog od `journal/`: tam mieszka rozumowanie
    /// silnika, tutaj surowe wejście — treść wiadomości wraz z każdą edycją.
    /// Zmieszanie ich znaczyłoby, że nie da się już powiedzieć, co było daną,
    /// a co wnioskiem z niej wyciągniętym.
    pub fn archive_dir(&self) -> PathBuf {
        self.logs_dir().join("wiadomosci")
    }

    /// Opcje zapisu KRONIKI. Świadomie OSOBNY plik od `settings.json`:
    /// to nie jest ustawienie strategii, tylko opis rejestratora — wgranie
    /// cudzego presetu nie ma prawa przestawić nikomu tego, co się zapisuje
    /// ani gdzie ten plik leży.
    pub fn kronika_path(&self) -> PathBuf {
        self.root.join("kronika.json")
    }

    pub fn load_kronika(&self) -> crate::kronika::Ustawienia {
        if let Some(u) = read_json::<crate::kronika::Ustawienia>(&self.kronika_path()) {
            return u;
        }
        crate::kronika::Ustawienia {
            plik: self.domyslny_plik_kroniki(),
            ..Default::default()
        }
    }

    /// Ścieżka kroniki dla instalacji, która nie ma jeszcze `kronika.json`.
    /// Wydzielona, żeby dało się ją pokazać w panelu („przywróć domyślną")
    /// i przetestować bez tworzenia całego magazynu.
    pub fn domyslny_plik_kroniki(&self) -> String {
        self.domyslny_plik_kroniki_z(crate::kronika::sciezka_pulpitu().as_deref())
    }

    /// Sama decyzja, bez pytania systemu o pulpit — żeby dało się ją
    /// przetestować, nie dotykając prawdziwego pulpitu użytkownika.
    pub fn domyslny_plik_kroniki_z(&self, pulpit: Option<&std::path::Path>) -> String {
        let stary = self.logs_dir().join(crate::kronika::PLIK_DOMYSLNY);
        match pulpit {
            Some(p) if p.is_file() || !stary.is_file() => p.display().to_string(),
            // pulpitu nie ma ALBO dane leżą już w katalogu bota — w obu
            // wypadkach zostajemy przy `logs/`, bo osierocenie zebranego
            // archiwum jest gorsze niż niewygodna ścieżka
            _ => "logs/kronika.jsonl".to_string(),
        }
    }

    pub fn save_kronika(&self, u: &crate::kronika::Ustawienia) -> Result<()> {
        write_json_atomic(&self.kronika_path(), u)
    }

    /// Wyniki laboratorium: jeden podkatalog na zadanie + punkt kontrolny
    /// treningu. Świadomie OBOK programu, a nie w `%TEMP%` — te pliki chce się
    /// otworzyć, obejrzeć i skopiować, a nie stracić przy sprzątaniu dysku.
    pub fn lab_dir(&self) -> PathBuf {
        self.root.join("lab")
    }

    /// Scala dziennik do jednego pliku analitycznego.
    ///
    /// Nazwa zawiera DATĘ I GODZINĘ scalenia, więc kolejne eksporty nie
    /// nadpisują się nawzajem — przy diagnozowaniu problemu chce się mieć
    /// obok siebie zrzut sprzed zmiany i po niej.
    pub fn merge_logs(&self, lines: &[String], now_ms: i64) -> Result<PathBuf> {
        std::fs::create_dir_all(self.logs_dir())?;
        let name = format!("alllogs_{}.txt", stamp(now_ms));
        let path = self.logs_dir().join(name);
        let mut buf = String::with_capacity(lines.len() * 96);
        for l in lines {
            buf.push_str(l);
            buf.push('\n');
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, buf.as_bytes())?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.root.join("backup_memory")
    }
    pub fn backup_latest(&self) -> PathBuf {
        self.backup_dir().join("latest.json")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.presets_dir())?;
        std::fs::create_dir_all(self.backup_dir())?;
        Ok(())
    }

    // ---------- konfiguracja ----------

    pub fn load_settings(&self) -> SettingsDoc {
        self.load_settings_checked().0
    }

    pub fn load_settings_checked(&self) -> (SettingsDoc, Option<String>) {
        match read_json_raport::<SettingsDoc>(&self.settings_path()) {
            Ok(v) => (v, None),
            Err(BladWczytania::Brak) => {
                let bak = self.settings_path().with_extension("bak");
                match read_json_raport::<SettingsDoc>(&bak) {
                    Ok(v) => {
                        let ile = v.settings.as_object().map(|o| o.len()).unwrap_or(0);
                        (
                            v,
                            Some(format!(
                                "Pliku settings.json NIE BYŁO, ale obok leżała kopia \
                                 settings.bak — wczytano ją ({ile} ustawień). Sprawdź, czym \
                                 bot gra, ZANIM zaczniesz handlować: kopia pochodzi sprzed \
                                 ostatniego zapisu, więc mogła zgubić najnowszą zmianę."
                            )),
                        )
                    }
                    Err(_) => (SettingsDoc::default(), None),
                }
            }
            Err(BladWczytania::Uszkodzony(e)) => {
                // ZABEZPIECZENIE TREŚCI: nieczytelny plik zostaje odłożony
                // na bok, ZANIM pierwszy zapis nadpisze go domyślnymi.
                // Bez tego jedna literówka w Notatniku kasuje 267 ustawień
                // bezpowrotnie — a zapis nastąpi już przy pierwszej zmianie
                // czegokolwiek w panelu.
                let kopia = self.ratuj_uszkodzony(&self.settings_path());
                let opis = match kopia {
                    Some(k) => format!("{e} (kopia zachowana: {k})"),
                    None => e,
                };
                (SettingsDoc::default(), Some(opis))
            }
        }
    }

    /// Odkłada nieczytelny plik obok, żeby nie zginął przy pierwszym zapisie.
    fn ratuj_uszkodzony(&self, p: &Path) -> Option<String> {
        let stempel = {
            use chrono::Local;
            Local::now().format("%Y%m%d-%H%M%S").to_string()
        };
        let cel = p.with_extension(format!("uszkodzony-{stempel}.json"));
        match std::fs::copy(p, &cel) {
            Ok(_) => Some(cel.file_name()?.to_string_lossy().to_string()),
            Err(e) => {
                tracing::error!(blad = %e, "nie udało się zachować kopii uszkodzonego pliku");
                None
            }
        }
    }
    pub fn save_settings(&self, d: &SettingsDoc) -> Result<()> {
        write_json_atomic(&self.settings_path(), d)
    }

    pub fn load_smtp(&self) -> SmtpDoc {
        read_json(&self.smtp_path()).unwrap_or_default()
    }
    pub fn save_smtp(&self, d: &SmtpDoc) -> Result<()> {
        write_json_atomic(&self.smtp_path(), d)
    }

    // ---------- poświadczenia ----------

    /// Wczytuje `secrets.json`. Brak pliku = pusty dokument (pierwsze
    /// uruchomienie), niezgodna wersja = pusty dokument z ostrzeżeniem.
    ///
    /// Uszkodzony plik NIE przerywa startu: bot ma wtedy poprosić o dane
    /// logowania jeszcze raz, a nie odmówić uruchomienia.
    pub fn load_secrets(&self) -> crate::secrets::SecretsDoc {
        use crate::secrets::{SecretsDoc, SECRETS_VERSION};
        let Some(d) = read_json::<SecretsDoc>(&self.secrets_path()) else {
            return SecretsDoc::default();
        };
        if d.version != SECRETS_VERSION {
            tracing::warn!(
                wersja = d.version,
                oczekiwana = SECRETS_VERSION,
                "secrets.json w nieobsługiwanym formacie — zaczynam od pustych poświadczeń"
            );
            return SecretsDoc::default();
        }
        d
    }

    /// Zapisuje `secrets.json` i NATYCHMIAST zawęża prawa do pliku.
    ///
    /// Kolejność ma znaczenie: gdyby prawa ustawiać przed zapisem, `rename`
    /// z pliku tymczasowego i tak podstawiłby plik z prawami domyślnymi.
    pub fn save_secrets(&self, d: &crate::secrets::SecretsDoc) -> Result<()> {
        let path = self.secrets_path();
        write_json_atomic(&path, d)?;
        if let Err(e) = crate::secrets::restrict_permissions(&path) {
            // Ostrzeżenie, nie błąd: dane są już zapisane, a odmowa działania
            // z powodu ACL zatrzymałaby bota bez poprawy bezpieczeństwa.
            tracing::warn!(
                plik = %path.display(),
                blad = %e,
                "nie udało się zawęzić uprawnień do secrets.json — sprawdź je ręcznie"
            );
        }
        Ok(())
    }

    pub fn load_demo(&self) -> crate::demo::DemoConfig {
        read_json(&self.demo_path()).unwrap_or_default()
    }
    pub fn save_demo(&self, d: &crate::demo::DemoConfig) -> Result<()> {
        write_json_atomic(&self.demo_path(), d)
    }

    /// Wczytuje `channels.json` RAZEM Z MIGRACJĄ starego kształtu.
    ///
    /// Migruje `serde` (`ui::ChannelBinding`), a tutaj wychodzi jedynie LISTA
    /// OSTRZEŻEŃ dla dziennika panelu: kanał albo temat, który miał więcej niż
    /// jeden format, stracił wszystkie poza pierwszym. Cisza jest tu zakazana —
    /// użytkownik zaznaczył coś, co przestało obowiązywać.
    pub fn load_channels_z_ostrzezeniami(&self) -> (ChannelsDoc, Vec<String>) {
        let surowy: Option<serde_json::Value> = read_json(&self.channels_path());
        let mut ostrzezenia = Vec::new();
        if let Some(v) = &surowy {
            for (id, b) in v
                .get("bindings")
                .and_then(|x| x.as_object())
                .into_iter()
                .flatten()
            {
                let mut zglos = |gdzie: String, x: &serde_json::Value| {
                    if let Some(a) = x.as_array() {
                        if a.len() > 1 {
                            ostrzezenia.push(format!(
                                "{gdzie}: było {} formatów ({}), a kanał może mieć DOKŁADNIE JEDEN                                  — zostaje pierwszy",
                                a.len(),
                                a.iter()
                                    .filter_map(|y| y.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                    }
                };
                if let Some(x) = b.get("formats") {
                    zglos(format!("kanał {id}"), x);
                }
                if let Some(t) = b.get("topics").and_then(|x| x.as_object()) {
                    for (tid, x) in t {
                        zglos(format!("kanał {id}, temat {tid}"), x);
                    }
                }
            }
        }
        // CISZA JEST TU ZAKAZANA. `.ok()` zamieniało każdy błąd parsowania na
        // pusty dokument — bot budził się bez ANI JEDNEGO obserwowanego kanału
        // i milczał. Na rachunku, który stoi tydzień bez opieki, to jest
        // najgorszy tryb awarii: wygląda identycznie jak „kanały jeszcze nie
        // przyszły" i nie zostawia po sobie śladu.
        let doc = match surowy {
            None => ChannelsDoc::default(),
            Some(v) => match serde_json::from_value::<ChannelsDoc>(v) {
                Ok(d) => d,
                Err(e) => {
                    ostrzezenia.push(format!(
                        "channels.json NIE DAŁ SIĘ WCZYTAĆ ({e}) — bot startuje BEZ ani jednego                          obserwowanego kanału i nie weźmie żadnego sygnału. Plik został                          nietknięty; popraw go albo skonfiguruj kanały od nowa w panelu."
                    ));
                    ChannelsDoc::default()
                }
            },
        };
        (doc, ostrzezenia)
    }

    pub fn load_channels(&self) -> ChannelsDoc {
        self.load_channels_z_ostrzezeniami().0
    }
    pub fn save_channels(&self, d: &ChannelsDoc) -> Result<()> {
        write_json_atomic(&self.channels_path(), d)
    }

    pub fn load_lancuchy(&self) -> conduit_core::formaty::Lancuchy {
        let Some(mut z) = read_json::<conduit_core::formaty::Lancuchy>(&self.lancuchy_path())
        else {
            return Default::default();
        };
        let mut lista = conduit_core::formaty::lancuchy_wbudowane();
        for wlasny in z.lista.into_iter() {
            match lista.iter().position(|w| w.nazwa == wlasny.nazwa) {
                Some(i) => lista[i] = wlasny,
                None => lista.push(wlasny),
            }
        }
        z.lista = lista;
        z
    }

    /// WSKAŹNIK AKTYWNEGO ŁAŃCUCHA DLA TRYBU **AUTO-EA** (projekt EA-2).
    ///
    /// # Dlaczego osobne pole, a nie drugie `Lancuchy`
    ///
    /// Tryb AUTO-EA prowadzi INNY skład niż AUTO — ta sama lista łańcuchów,
    /// inny wskazany. Gdyby wskaźnik był jeden, przełączenie trybu na chwilę
    /// przestawiałoby też skład drugiego trybu, a powrót nie miałby dokąd
    /// wrócić. Dwa wskaźniki nad JEDNĄ listą to najmniejsza zmiana, która
    /// to rozstrzyga.
    ///
    /// # Kontrakt zera
    ///
    /// Pole jest OPCJONALNE w `lancuchy.json` (`aktywnyEa`). Plik sprzed
    /// tej wersji nie ma go wcale i wtedy AUTO-EA czyta zwykłe `aktywny` —
    /// czyli zachowuje się CO DO BITU jak dotąd. Pusty łańcuch znaków znaczy
    /// dokładnie to samo co brak klucza.
    ///
    /// # Dlaczego czytane surowym JSON-em
    ///
    /// `conduit_core::formaty::Lancuchy` należy do rdzenia i nie zna trybów
    /// panelu (`TradingMode` mieszka w serwerze). Wskaźnik trybu jest więc
    /// polem SERWERA dopisywanym do tego samego dokumentu — rdzeń go nie
    /// widzi i widzieć nie musi, bo to warstwa wyżej rozstrzyga, którym
    /// łańcuchem gra się w którym trybie.
    pub fn load_aktywny_ea(&self) -> String {
        let Some(v) = read_json::<serde_json::Value>(&self.lancuchy_path()) else {
            return String::new();
        };
        // camelCase jak w całym protokole; snake_case przyjmujemy z litości
        // dla pliku napisanego ręcznie.
        v.get("aktywnyEa")
            .or_else(|| v.get("aktywny_ea"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Zapis zbioru łańcuchów RAZEM ze wskaźnikiem trybu AUTO-EA.
    ///
    /// Pusty `aktywny_ea` NIE trafia do pliku — dokument bez tego klucza jest
    /// stanem domyślnym (fallback na `aktywny`), więc zapisywanie pustki
    /// tylko zaśmiecałoby plik polem bez znaczenia.
    pub fn save_lancuchy(
        &self,
        d: &conduit_core::formaty::Lancuchy,
        aktywny_ea: &str,
    ) -> Result<()> {
        let mut v = serde_json::to_value(d)?;
        if let Some(o) = v.as_object_mut() {
            if aktywny_ea.is_empty() {
                o.remove("aktywnyEa");
            } else {
                o.insert(
                    "aktywnyEa".into(),
                    serde_json::Value::String(aktywny_ea.into()),
                );
            }
        }
        write_json_atomic(&self.lancuchy_path(), &v)
    }

    /// PIECZĘĆ PACZKI — patrz [`Paczka`]. Brak pliku = brak sprawdzenia.
    pub fn paczka_path(&self) -> PathBuf {
        self.root.join("PACZKA.json")
    }
    pub fn load_paczka(&self) -> Option<Paczka> {
        read_json::<Paczka>(&self.paczka_path())
    }

    /// Wczytuje wszystkie presety z `presets/`. Plik, którego nie da się
    /// sparsować, jest POMIJANY z ostrzeżeniem — jeden zepsuty preset nie
    /// może uniemożliwić startu bota.
    pub fn load_presets(&self) -> Vec<conduit_core::Preset> {
        let dir = self.presets_dir();
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => return out,
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        files.sort();
        // Tożsamością presetu jest jego NAZWA (tak szuka go `ApplyPreset`), a nie
        // nazwa pliku. Dwa pliki o tej samej nazwie presetu — a tak wygląda
        // paczka, w której czempion leży dodatkowo jako `00-CHAMPION.json` —
        // dawały w panelu dwa identyczne wiersze i dwa razy tę samą pozycję na
        // liście wyboru. Zostawiamy PIERWSZY po posortowaniu, czyli ten
        // z prefiksem porządkującym.
        let mut widziane: std::collections::HashSet<String> = std::collections::HashSet::new();
        for f in files {
            if let Ok(txt) = std::fs::read_to_string(&f) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                    let obce = conduit_core::nieznane_pola_ustawien(&v);
                    if !obce.is_empty() {
                        static POWIEDZIANE: std::sync::OnceLock<
                            std::sync::Mutex<std::collections::HashSet<String>>,
                        > = std::sync::OnceLock::new();
                        let klucz = format!("{}|{}", f.display(), obce.join(","));
                        let pierwszy = POWIEDZIANE
                            .get_or_init(Default::default)
                            .lock()
                            .map(|mut z| z.insert(klucz))
                            .unwrap_or(false);
                        if pierwszy {
                            tracing::warn!(
                                plik = %f.display(),
                                pola = %obce.join(", "),
                                "preset ma pola, ktorych ta binarka nie zna -- sa POMIJANE; przebuduj conduit.exe z biezacego silnika"
                            );
                        }
                    }
                }
            }
            match read_json::<conduit_core::Preset>(&f) {
                Some(p) => {
                    let klucz = p.name.to_lowercase();
                    if !widziane.insert(klucz) {
                        tracing::info!(
                            plik = %f.display(),
                            preset = %p.name,
                            "pominięto duplikat presetu (ta sama nazwa co wcześniejszy plik)"
                        );
                        continue;
                    }
                    out.push(p);
                }
                None => tracing::warn!(plik = %f.display(), "pominięto uszkodzony preset"),
            }
        }
        out
    }

    pub fn save_preset(&self, p: &conduit_core::Preset) -> Result<()> {
        std::fs::create_dir_all(self.presets_dir())?;
        let name = sanitize_file_name(&p.name);
        write_json_atomic(&self.presets_dir().join(format!("{name}.json")), p)
    }

    // ---------- modele AI ----------

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    /// Ścieżka modelu o danym identyfikatorze (nazwa pliku bez `.json`).
    ///
    /// Zwraca `None` dla identyfikatora, który próbowałby wyjść poza katalog
    /// `models/` — to jedyna bariera między adresem z sieci a systemem plików.
    pub fn model_path(&self, id: &str) -> Option<PathBuf> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return None;
        }
        Some(self.models_dir().join(format!("{id}.json")))
    }

    /// Wczytuje modele AI z `models/` jako SUROWY JSON.
    ///
    /// Świadomie nie parsujemy do `conduit_ai::Model`: serwer ma pokazać to, co
    /// leży na dysku, także model wytrenowany na innym zestawie cech niż
    /// bieżący kod. Odmowa wczytania należy do tego, kto model wpuszcza do gry
    /// (`Model::validate`), a nie do przeglądarki modeli.
    ///
    /// Zwraca pary `(identyfikator, dokument)`; identyfikator to nazwa pliku
    /// bez rozszerzenia, bo to on trafia do adresu `/api/models/{id}`.
    pub fn load_models(&self) -> Vec<(String, serde_json::Value)> {
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(self.models_dir()) {
            Ok(r) => r,
            Err(_) => return out,
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        files.sort();
        for f in files {
            let id = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            match read_json::<serde_json::Value>(&f) {
                Some(v) => out.push((id, v)),
                None => tracing::warn!(plik = %f.display(), "pominięto uszkodzony model AI"),
            }
        }
        out
    }

    // ---------- pamięć stanu ----------

    /// Zapisuje migawkę: najpierw kopia rotacyjna, potem `latest.json`.
    pub fn save_backup(&self, m: &BackupMemory) -> Result<PathBuf> {
        let dir = self.backup_dir();
        std::fs::create_dir_all(&dir)?;
        let stamp = stamp_from_ms(m.saved_at);
        let rotated = dir.join(format!("{stamp}.json"));
        write_json_atomic(&rotated, m)?;
        write_json_atomic(&self.backup_latest(), m)?;
        self.prune_backups(BACKUP_KEEP);
        Ok(rotated)
    }

    /// Wczytuje ostatni zapis. Jeśli `latest.json` jest uszkodzony,
    /// schodzi do najnowszej sprawnej kopii rotacyjnej — dlatego w ogóle
    /// trzymamy rotację.
    pub fn load_backup(&self) -> Option<BackupMemory> {
        if let Some(m) = read_json::<BackupMemory>(&self.backup_latest()) {
            if m.version == BACKUP_VERSION {
                return Some(m);
            }
            tracing::warn!(
                wersja = m.version,
                oczekiwana = BACKUP_VERSION,
                "backup_memory w starym formacie — pomijam"
            );
        }
        for p in self.rotated_backups().into_iter().rev() {
            if let Some(m) = read_json::<BackupMemory>(&p) {
                if m.version == BACKUP_VERSION {
                    tracing::warn!(plik = %p.display(), "latest.json nieczytelny — wznowiono z kopii");
                    return Some(m);
                }
            }
        }
        None
    }

    /// Kopie rotacyjne posortowane rosnąco po nazwie (czyli po czasie).
    pub fn rotated_backups(&self) -> Vec<PathBuf> {
        let dir = self.backup_dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut v: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().map(|x| x == "json").unwrap_or(false)
                    && p.file_name().map(|n| n != "latest.json").unwrap_or(false)
            })
            .collect();
        v.sort();
        v
    }

    fn prune_backups(&self, keep: usize) {
        let list = self.rotated_backups();
        if list.len() <= keep {
            return;
        }
        for p in &list[..list.len() - keep] {
            let _ = std::fs::remove_file(p);
        }
    }
}

// ============================================================
//  POMOCNICZE
// ============================================================

/// Dlaczego wczytanie się nie udało. **Brak pliku to NIE to samo co plik
/// nieczytelny** — pierwsze jest normalnym pierwszym uruchomieniem, drugie
/// znaczy, że ktoś właśnie stracił konfigurację i o tym nie wie.
#[derive(Debug, Clone, PartialEq)]
pub enum BladWczytania {
    /// pliku nie ma — pierwsze uruchomienie, wszystko w porządku
    Brak,
    /// plik jest, ale nie da się go odczytać albo sparsować
    Uszkodzony(String),
}

/// Wczytuje dokument JSON, **zjadając BOM**, i mówi, CO poszło nie tak.
///
/// # BOM
///
/// `EF BB BF` na początku pliku to nie uszkodzenie, tylko inny sposób zapisu
/// tego samego JSON-a — dokłada go Notatnik, `Set-Content -Encoding UTF8`
/// w PowerShellu i pół edytorów na Windows. `serde_json` przewraca się na nim
/// z komunikatem `expected value at line 1 column 1`, po którym cała
/// konfiguracja szła do kosza i bot startował z wartościami domyślnymi.
/// Użytkownik, który poprawił jedno pole w Notatniku, dostawał po cichu
/// zupełnie innego bota.
fn read_json_raport<T: for<'de> Deserialize<'de>>(p: &Path) -> Result<T, BladWczytania> {
    if !p.exists() {
        return Err(BladWczytania::Brak);
    }
    let raw = std::fs::read_to_string(p)
        .map_err(|e| BladWczytania::Uszkodzony(format!("nie da się odczytać pliku: {e}")))?;
    let czysty = raw.trim_start_matches('\u{feff}');
    serde_json::from_str(czysty).map_err(|e| BladWczytania::Uszkodzony(e.to_string()))
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    match read_json_raport(p) {
        Ok(v) => Some(v),
        Err(BladWczytania::Brak) => None,
        Err(BladWczytania::Uszkodzony(e)) => {
            tracing::error!(plik = %p.display(), blad = %e, "PLIK NIECZYTELNY — użyto wartości domyślnych");
            None
        }
    }
}

/// Write and flush the replacement before rename. A failed replacement MUST
/// leave the old destination intact; never remove it to make rename succeed.
/// This is not an exactly-once journal or a guarantee against storage failure.
pub fn write_json_atomic<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    use std::io::Write;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("tmp");
    let data = serde_json::to_vec_pretty(v)?;
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("otwarcie {}", tmp.display()))?;
        file.write_all(&data).with_context(|| format!("zapis {}", tmp.display()))?;
        file.sync_all().with_context(|| format!("utrwalenie {}", tmp.display()))?;
    }

    // KOPIA POPRZEDNIEJ WERSJI — ostatnia deska ratunku, gdyby podmiana padła
    // w najgorszym możliwym momencie. Tania: jeden plik, nadpisywany.
    if p.exists() {
        let _ = std::fs::copy(p, p.with_extension("bak"));
    }

    std::fs::rename(&tmp, p).with_context(|| format!(
        "podmiana {} nie powiodła się; poprzedni plik nie został usunięty, nowy zapis pozostaje w {}",
        p.display(), tmp.display()))
}

fn sanitize_file_name(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "preset".into()
    } else {
        out
    }
}

/// `2026-07-27_143012` — nazwa sortowalna leksykograficznie, czyli
/// sortowanie po nazwie = sortowanie po czasie.
fn stamp_from_ms(ms: i64) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_millis_opt(ms).single() {
        Some(dt) => dt
            .format("%Y-%m-%d_%H%M%S%.3f")
            .to_string()
            .replace('.', "-"),
        None => format!("{ms}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prosty katalog tymczasowy bez dodatkowej zależności.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let mut p = std::env::temp_dir();
            let uniq = format!(
                "conduit-test-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            p.push(uniq);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn przykladowy_snapshot() -> ui::UiSnapshot {
        let mut s = ui::UiSnapshot::empty(1_700_000_000_000);
        s.balance = 2137.42;
        s.stats.equity = 2200.0;
        s.stats.messages = 17;
        s.halt = ui::HaltState::nowy(ui::KlasaHaltu::Ryzyko, "MAX DD 12%");
        s.positions.push(ui::Position {
            ticket: 991,
            symbol: "XAUUSD".into(),
            direction: ui::Direction::Buy,
            volume: 0.02,
            open_price: 4118.5,
            open_time: 1_700_000_000_000,
            sl: Some(4110.0),
            tp: Some(4130.0),
            vsl: None,
            profit: 3.4,
            swap: 0.0,
            commission: 0.0,
            comment: "B1".into(),
            magic: Some(770_077),
            basket_id: Some(1),
            level: 0,
            frozen: false,
            peak_pts: 1.2,
            runner: false,
            toucher: false,
            last_peak_time: 1_700_000_000_000,
            // Pole dołożone razem z podglądem CAŁEGO rachunku (pozycje spoza
            // bota). Bez niego moduł testowy przestał się kompilować i cały
            // zestaw testów serwera nie dawał się uruchomić.
            source: ui::Origin::Bot,
        });
        s.messages.push(ui::ChatMessage {
            id: "m1".into(),
            time: 1_700_000_000_000,
            channel_id: -100123,
            channel_name: "ATFX VIP".into(),
            topic_id: None,
            topic_name: None,
            format: Some("ATFX".into()),
            text: "BUY LIMITS GOLD @ 4118/4112".into(),
            types: vec!["ENTRY".into()],
            basket_id: Some(1),
            edited: false,
            pending_action: None,
            parsed: None,
        });
        s
    }

    #[test]
    fn bom_nie_kasuje_konfiguracji() {
        // REGRESJA: `Set-Content -Encoding UTF8` w PowerShellu i Notatnik
        // dokładają BOM `EF BB BF`. `serde_json` przewracał się na nim
        // („expected value at line 1 column 1"), `load_settings` oddawało
        // wartości domyślne i bot handlował konfiguracją, której nikt nie
        // wybrał — a jedynym śladem była linijka WARN na stdout.
        let t = Tmp::new("bom");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();

        let mut doc = SettingsDoc::default();
        doc.preset_id = "KRATA".into();
        doc.settings = serde_json::json!({ "entry_units": 3, "trail_mode": "lock_pct" });
        ws.save_settings(&doc).unwrap();

        // dokładamy BOM dokładnie tak, jak robi to Windows
        let sciezka = ws.settings_path();
        let tresc = std::fs::read_to_string(&sciezka).unwrap();
        std::fs::write(&sciezka, format!("\u{feff}{tresc}")).unwrap();

        let (wczytane, blad) = ws.load_settings_checked();
        assert!(
            blad.is_none(),
            "BOM to nie usterka pliku, tylko inny zapis tego samego JSON-a"
        );
        assert_eq!(wczytane.preset_id, "KRATA", "preset musi przeżyć BOM");
        assert_eq!(wczytane.settings["entry_units"], serde_json::json!(3));
    }

    #[test]
    fn brak_pliku_to_nie_to_samo_co_plik_uszkodzony() {
        let t = Tmp::new("uszkodzony");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();

        // 1. pliku nie ma — pierwsze uruchomienie, ŻADNEGO błędu
        let (_, blad) = ws.load_settings_checked();
        assert!(
            blad.is_none(),
            "brak pliku to normalne pierwsze uruchomienie"
        );

        // 2. plik jest, ale nieczytelny — TO JEST AWARIA
        std::fs::write(ws.settings_path(), "{ to nie jest json").unwrap();
        let (dom, blad) = ws.load_settings_checked();
        assert!(
            blad.is_some(),
            "nieczytelny plik MUSI zgłosić powód, a nie udawać braku"
        );
        assert_eq!(dom.preset_id, "", "przy awarii wracamy do domyślnych");

        // 3. i uszkodzony plik NIE MOŻE zginąć przy pierwszym zapisie
        let kopie: Vec<_> = std::fs::read_dir(&t.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("uszkodzony"))
            .collect();
        assert_eq!(
            kopie.len(),
            1,
            "kopia uszkodzonego settings.json musi zostać zachowana"
        );
    }

    #[test]
    fn domyslna_sciezka_kroniki_nie_gubi_zebranych_danych() {
        let t = Tmp::new("domyslna-kronika");
        let ws = Workspace::new(t.0.clone());
        let mut pulpit = std::env::temp_dir();
        pulpit.push(format!(
            "conduit-pulpit-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        std::fs::create_dir_all(&pulpit).unwrap();
        let na_pulpicie = pulpit.join(crate::kronika::PLIK_PULPITU);

        // 1. czysto: nowy plik na pulpicie
        assert_eq!(
            ws.domyslny_plik_kroniki_z(Some(&na_pulpicie)),
            na_pulpicie.display().to_string(),
            "pierwsze uruchomienie ma iść na pulpit"
        );

        // 2. dane leżą JUŻ w katalogu bota, na pulpicie nie ma nic — nie
        //    wolno ich osierocić, zakładając nowy plik gdzie indziej
        std::fs::create_dir_all(ws.logs_dir()).unwrap();
        std::fs::write(ws.logs_dir().join("kronika.jsonl"), b"{}\n").unwrap();
        assert_eq!(
            ws.domyslny_plik_kroniki_z(Some(&na_pulpicie)),
            "logs/kronika.jsonl",
            "istniejące archiwum w katalogu bota ma pierwszeństwo przed pustym pulpitem"
        );

        // 3. plik na pulpicie ISTNIEJE — to jest ta ciągłość, po którą cała
        //    zmiana powstała: nowa paczka dopisuje do niego, nie do swojego
        std::fs::write(&na_pulpicie, b"{}\n").unwrap();
        assert_eq!(
            ws.domyslny_plik_kroniki_z(Some(&na_pulpicie)),
            na_pulpicie.display().to_string(),
            "istniejący plik na pulpicie wygrywa ZAWSZE — to on przeżywa aktualizacje"
        );

        // 4. brak pulpitu (konto usługowe, serwer bez profilu) — ścieżka
        //    musi istnieć mimo wszystko
        assert_eq!(ws.domyslny_plik_kroniki_z(None), "logs/kronika.jsonl");

        let _ = std::fs::remove_dir_all(pulpit);
    }

    /// `kronika.json` na dysku ZAWSZE wygrywa z domyślną — inaczej wybór
    /// użytkownika znikałby przy każdej aktualizacji bota.
    #[test]
    fn zapisane_opcje_kroniki_biją_domyslna_sciezke() {
        let t = Tmp::new("kronika-opcje");
        let ws = Workspace::new(t.0.clone());
        let wlasna = crate::kronika::Ustawienia {
            plik: "D:/archiwum/moja_kronika.jsonl".into(),
            ..Default::default()
        };
        ws.save_kronika(&wlasna).unwrap();
        assert_eq!(ws.load_kronika().plik, "D:/archiwum/moja_kronika.jsonl");
    }

    #[test]
    fn zapis_i_odczyt_backup_memory_zachowuje_stan() {
        let t = Tmp::new("backup");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();

        let snap = przykladowy_snapshot();
        let mem = BackupMemory::from_snapshot(&snap, 1_700_000_123_456);
        ws.save_backup(&mem).unwrap();

        let wczytane = ws.load_backup().expect("backup musi się wczytać");
        assert_eq!(wczytane.version, BACKUP_VERSION);
        assert_eq!(wczytane.balance, 2137.42);
        assert_eq!(wczytane.halt.reason, "MAX DD 12%");
        assert_eq!(wczytane.positions.len(), 1);
        assert_eq!(wczytane.positions[0].ticket, 991);
        assert_eq!(wczytane.messages.len(), 1);
        assert_eq!(wczytane.next_basket_id, 1);

        // i nakłada się z powrotem na świeży stan
        let mut fresh = ui::UiSnapshot::empty(0);
        wczytane.apply_to(&mut fresh);
        assert_eq!(fresh.balance, 2137.42);
        assert!(fresh.halt.active);
        assert_eq!(fresh.positions.len(), 1);
    }

    #[test]
    fn uszkodzony_latest_schodzi_do_kopii_rotacyjnej() {
        let t = Tmp::new("rot");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();

        let snap = przykladowy_snapshot();
        ws.save_backup(&BackupMemory::from_snapshot(&snap, 1_700_000_000_000))
            .unwrap();

        // ktoś/coś ucina plik w połowie zapisu
        std::fs::write(ws.backup_latest(), b"{\"version\":1,\"saved_at\":").unwrap();

        let m = ws.load_backup().expect("musi wznowić z kopii rotacyjnej");
        assert_eq!(m.balance, 2137.42);
    }

    #[test]
    fn stary_format_jest_odrzucany_a_nie_zgadywany() {
        let t = Tmp::new("wersja");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        let mut mem = BackupMemory::from_snapshot(&przykladowy_snapshot(), 1);
        mem.version = 999;
        write_json_atomic(&ws.backup_latest(), &mem).unwrap();
        assert!(ws.load_backup().is_none());
    }

    #[test]
    fn rotacja_przycina_stare_kopie() {
        let t = Tmp::new("prune");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        let snap = przykladowy_snapshot();
        for i in 0..(BACKUP_KEEP + 7) {
            ws.save_backup(&BackupMemory::from_snapshot(
                &snap,
                1_700_000_000_000 + i as i64 * 1000,
            ))
            .unwrap();
        }
        assert_eq!(ws.rotated_backups().len(), BACKUP_KEEP);
    }

    #[test]
    fn poswiadczenia_przechodza_zapis_i_odczyt_w_obie_strony() {
        let t = Tmp::new("sekrety");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();

        // brak pliku = puste poświadczenia, a nie błąd
        assert!(!ws.load_secrets().telegram.has_credentials());

        let mut d = ws.load_secrets();
        d.telegram.api_id = 2_040_123;
        d.telegram.api_hash = "0123456789abcdef0123456789abcdef".into();
        d.telegram.session_string = "BASE64-LANCUCH-SESJI".into();
        d.telegram.user_id = 777;
        d.telegram.user_name = "Demo User".into();
        d.telegram.handle = "demo_user".into();
        d.telegram.saved_at = 1_700_000_000_000;
        d.smtp.password = "haslo-aplikacji".into();
        ws.save_secrets(&d).unwrap();

        let back = ws.load_secrets();
        assert_eq!(back, d, "cały dokument musi wrócić bez strat");
        assert!(back.telegram.has_credentials());
        assert!(back.telegram.has_session());
        assert_eq!(back.smtp.password.as_str(), "haslo-aplikacji");

        // zapis atomowy nie zostawia śmieci
        assert!(!ws.secrets_path().with_extension("tmp").exists());
    }

    #[test]
    fn podsumowanie_poswiadczen_nie_niesie_zadnej_wartosci() {
        // To jest jedyna postać, w jakiej poświadczenia opuszczają serwer.
        let t = Tmp::new("sekrety-pub");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        let mut d = ws.load_secrets();
        d.telegram.api_id = 42;
        d.telegram.api_hash = "0123456789abcdef0123456789abcdef".into();
        d.telegram.session_string = "TAJNA-SESJA".into();
        d.smtp.password = "TAJNE-HASLO".into();
        ws.save_secrets(&d).unwrap();

        let j = serde_json::to_string(&ws.load_secrets().public_summary()).unwrap();
        for tajne in ["0123456789abcdef", "TAJNA-SESJA", "TAJNE-HASLO"] {
            assert!(!j.contains(tajne), "podsumowanie ujawnia „{tajne}”: {j}");
        }
        // ale mówi, co jest ustawione — bez tego UI nie wie, czy pytać
        assert!(j.contains("\"apiId\":42"));
        assert!(j.contains("\"apiHashSet\":true"));
        assert!(j.contains("\"sessionSet\":true"));
        assert!(j.contains("\"passwordSet\":true"));
    }

    #[test]
    fn uszkodzony_plik_sekretow_nie_blokuje_startu() {
        // Odmowa uruchomienia byłaby gorsza od problemu: bot ma poprosić
        // o dane logowania jeszcze raz, a nie odmówić pracy.
        let t = Tmp::new("sekrety-zle");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        std::fs::write(ws.secrets_path(), b"{ to nie jest json").unwrap();
        assert!(!ws.load_secrets().telegram.has_credentials());

        // to samo dla obcej wersji formatu
        std::fs::write(ws.secrets_path(), br#"{"version":99}"#).unwrap();
        assert!(!ws.load_secrets().telegram.has_credentials());
    }

    #[test]
    fn brak_plikow_daje_wartosci_domyslne_a_nie_blad() {
        let t = Tmp::new("puste");
        let ws = Workspace::new(&t.0);
        assert!(ws.load_backup().is_none());
        assert_eq!(ws.load_settings().preset_id, "");
        assert!(ws.load_channels().bindings.is_empty());
        assert!(ws.load_presets().is_empty());
    }

    #[test]
    fn konfiguracja_przezywa_zapis_i_odczyt() {
        let t = Tmp::new("cfg");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();

        let d = SettingsDoc {
            mode: ui::TradingMode::Ai,
            preset_id: "MONTE-CARLO".into(),
            lot: ui::LotConfig {
                mode: "percent".into(),
                fixed: 0.01,
                percent: 0.125,
            },
            settings: serde_json::json!({ "trail_mode": "tiered", "max_dd_pct": 60 }),
            ..Default::default()
        };
        ws.save_settings(&d).unwrap();

        let back = ws.load_settings();
        assert_eq!(back.mode, ui::TradingMode::Ai);
        assert_eq!(back.preset_id, "MONTE-CARLO");
        assert_eq!(back.lot.percent, 0.125);
        assert_eq!(back.settings["trail_mode"], "tiered");
    }

    #[test]
    fn tryb_auto_ea_ma_staly_zapis_a_stare_wartosci_bez_zmian() {
        use ui::TradingMode as M;
        for (m, txt) in [
            (M::Manual, "\"MANUAL\""),
            (M::Auto, "\"AUTO\""),
            (M::AutoEa, "\"AUTO-EA\""),
            (M::Ai, "\"AI\""),
        ] {
            assert_eq!(serde_json::to_string(&m).unwrap(), txt);
            assert_eq!(serde_json::from_str::<M>(txt).unwrap(), m);
        }
        assert_eq!(M::default(), M::Auto, "default trybu musi zostać AUTO");

        let t = Tmp::new("autoea");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        ws.save_settings(&SettingsDoc {
            mode: M::AutoEa,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(ws.load_settings().mode, M::AutoEa);
    }

    /// Nieznany string w `mode` NIE panikuje — działa dotychczasowy fallback:
    /// dokument liczy się jako uszkodzony, wraca default (AUTO) z powodem,
    /// a oryginał zostaje odłożony obok (żadna ścieżka nie traci pliku).
    #[test]
    fn nieznany_tryb_nie_panikuje_tylko_wraca_default_z_powodem() {
        let t = Tmp::new("trybobcy");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        std::fs::write(ws.settings_path(), br#"{"mode":"TURBO-9000"}"#).unwrap();
        let (doc, powod) = ws.load_settings_checked();
        assert_eq!(doc.mode, ui::TradingMode::Auto);
        assert!(
            powod.is_some(),
            "nieczytelny tryb musi mieć jawny powód, nie ciszę"
        );
    }

    #[test]
    fn zepsuty_preset_nie_blokuje_reszty() {
        let t = Tmp::new("presety");
        let ws = Workspace::new(&t.0);
        ws.ensure_dirs().unwrap();
        ws.save_preset(&conduit_core::Preset {
            name: "T4-MAX".into(),
            description: "test".into(),
            settings: conduit_core::Settings::default(),
            // Preset bez pliku = ustawienia domyślne, a te powstały pod kanał ATFX.
            format: "ATFX".into(),
            ea: None,
        })
        .unwrap();
        std::fs::write(ws.presets_dir().join("zepsuty.json"), b"{ to nie jest json").unwrap();

        let p = ws.load_presets();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "T4-MAX");
    }

    #[test]
    fn modele_ai_czytaja_sie_z_katalogu_a_zepsuty_nie_blokuje_reszty() {
        let t = Tmp::new("modele");
        let ws = Workspace::new(&t.0);
        std::fs::create_dir_all(ws.models_dir()).unwrap();
        std::fs::write(
            ws.models_dir().join("atfx_manager_v1.json"),
            br#"{"format":1,"name":"atfx","policy":{"pos":{"dims":[60,48,11]}}}"#,
        )
        .unwrap();
        std::fs::write(ws.models_dir().join("zepsuty.json"), b"{ to nie jest json").unwrap();
        std::fs::write(ws.models_dir().join("notatka.txt"), b"nie model").unwrap();

        let m = ws.load_models();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, "atfx_manager_v1");
        assert_eq!(m[0].1["format"], 1);

        // identyfikator z adresu nie może wyjść poza `models/`
        assert!(ws.model_path("atfx_manager_v1").is_some());
        assert!(ws.model_path("../settings").is_none());
        assert!(ws.model_path("a/b").is_none());
        assert!(ws.model_path("").is_none());
    }

    #[test]
    fn zapis_atomowy_nie_zostawia_pliku_tmp() {
        let t = Tmp::new("atom");
        let f = t.0.join("x.json");
        write_json_atomic(&f, &serde_json::json!({"a":1})).unwrap();
        write_json_atomic(&f, &serde_json::json!({"a":2})).unwrap();
        assert!(!f.with_extension("tmp").exists());
        let v: serde_json::Value = read_json(&f).unwrap();
        assert_eq!(v["a"], 2);
    }

    #[test]
    #[cfg(windows)]
    fn zapis_atomowy_blocked_replacement_preserves_old_and_new_data() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = Tmp::new("locked-replace");
        let p = t.0.join("settings.json");
        write_json_atomic(&p, &serde_json::json!({"version":"old"})).unwrap();
        // Permit reading/copying, but not replacement/deletion while held.
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(&p).unwrap();
        assert!(write_json_atomic(&p, &serde_json::json!({"version":"new"})).is_err());
        let old: serde_json::Value = read_json(&p).unwrap();
        let new: serde_json::Value = read_json(&p.with_extension("tmp")).unwrap();
        assert_eq!(old["version"], "old");
        assert_eq!(new["version"], "new");
        drop(held);
        write_json_atomic(&p, &serde_json::json!({"version":"new"})).unwrap();
        assert!(!p.with_extension("tmp").exists());
    }
}
