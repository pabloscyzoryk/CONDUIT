//! Presentation language for the native monitor. Protocol keys and user names
//! remain unchanged. Only the GUI initializes or changes the current language.
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    En,
    Pl,
}

impl Language {
    pub fn parse(value: &str) -> Option<Self> {
        match value
            .trim_start_matches('\u{feff}')
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "en" | "en-us" | "en-gb" => Some(Self::En),
            "pl" | "pl-pl" => Some(Self::Pl),
            _ => None,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }
    pub fn text<'a>(self, pl: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::En => en,
            Self::Pl => pl,
        }
    }
    pub fn number(self, value: f64, digits: usize) -> String {
        let polish = crate::pl_liczba(value, digits);
        match self {
            Self::Pl => polish,
            Self::En => polish.replace(',', "."),
        }
    }
    pub fn large(self, value: f64) -> String {
        if self == Self::Pl {
            return crate::pl_duza(value);
        }
        let abs = value.abs();
        if abs >= 1e9 {
            format!("{} B", self.number(value / 1e9, 2))
        } else if abs >= 1e6 {
            format!("{} M", self.number(value / 1e6, 1))
        } else if abs >= 1e4 {
            format!("{} k", self.number(value / 1e3, 1))
        } else {
            self.number(value, 0)
        }
    }
    pub fn tasks(self, count: usize) -> String {
        if self == Self::En {
            return format!("{count} {}", if count == 1 { "task" } else { "tasks" });
        }
        let form = if count == 1 {
            "zadanie"
        } else if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) {
            "zadania"
        } else {
            "zadań"
        };
        format!("{count} {form}")
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);
pub fn current() -> Language {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Language::Pl
    } else {
        Language::En
    }
}
pub fn set(language: Language) {
    CURRENT.store(
        if language == Language::Pl { 1 } else { 0 },
        Ordering::Relaxed,
    );
}
pub fn text<'a>(pl: &'a str, en: &'a str) -> &'a str {
    current().text(pl, en)
}
pub fn number(value: f64, digits: usize) -> String {
    current().number(value, digits)
}
pub fn large(value: f64) -> String {
    current().large(value)
}

pub fn choose(
    cli: Option<&str>,
    environment: Option<&str>,
    saved: Option<&str>,
) -> Result<Language, &'static str> {
    if let Some(value) = cli {
        return Language::parse(value).ok_or("--language requires en or pl");
    }
    Ok(environment
        .and_then(Language::parse)
        .or_else(|| saved.and_then(Language::parse))
        .unwrap_or(Language::En))
}

pub fn initialize(args: &[String], directory: &Path) -> Result<Language, &'static str> {
    let mut cli = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--language" {
            cli = Some(
                args.get(index + 1)
                    .ok_or("--language requires en or pl")?
                    .as_str(),
            );
        } else if let Some(value) = arg.strip_prefix("--language=") {
            cli = Some(value);
        }
    }
    let env = std::env::var("CONDUIT_LANGUAGE").ok();
    let saved = std::fs::read_to_string(directory.join("language.txt")).ok();
    let language = choose(cli, env.as_deref(), saved.as_deref())?;
    set(language);
    Ok(language)
}

pub fn save(directory: &Path, language: Language) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join("language.txt.tmp");
    std::fs::write(&temporary, format!("{}\n", language.code()))?;
    std::fs::rename(temporary, directory.join("language.txt"))
}

pub fn unit(value: &str) -> &str {
    unit_in(current(), value)
}

pub fn unit_in(language: Language, value: &str) -> &str {
    if language == Language::Pl {
        return value;
    }
    match value {
        "ticków" => "ticks",
        "ticków/s" => "ticks/s",
        "ocen" => "evaluations",
        "ocen/s" => "evaluations/s",
        "presetów" => "presets",
        "presetów/s" => "presets/s",
        "konfiguracji" => "configurations",
        "konfiguracji/s" => "configurations/s",
        "przebiegów" => "runs",
        "przebiegów/s" => "runs/s",
        "pokoleń" => "generations",
        "pokoleń/s" => "generations/s",
        "trening" => "training",
        _ => value,
    }
}

pub fn label(language: Language, value: &str) -> &str {
    match value {
        "known_entry_sources" => return language.text("znane źródła wejść", "known entry sources"),
        "known_full_entry_sources" => {
            return language.text("znane pełne źródła wejść", "known full entry sources")
        }
        "entry_sources_first_seen_as_edit" => {
            return language.text(
                "wejścia po raz pierwszy widziane jako edycja",
                "entry sources first seen as edits",
            )
        }
        "EditOrphan" => {
            return language.text("edycja bez znanego wejścia", "edit without a known entry")
        }
        _ => {}
    }
    if language == Language::Pl {
        return value;
    }
    match value {
        "T-100: aktywność ukończonych" => "T-100: completed-run activity",
        "T-100: rozgrzewka" => "T-100: warmup",
        "T-100: ocena lidera" => "T-100: leader assessment",
        "zegar zdarzeń" => "event clock",
        "zakres mierzony" => "measured range",
        "zdarzenia eksportu" => "export events",
        "pełne dane ticków" => "full tick history",
        "rozgrzewka rynku/SR" => "market/SR warmup",
        "kapitał startowy" => "starting capital",
        "zysk końcowy" => "net profit",
        "% dodatnich dni rynkowych (equity)" => "% positive market days (equity)",
        "% dodatnich dni z zamknięciami (legacy)" => "% positive days with closes (legacy)",
        "najniższe obsunięcie" => "lowest drawdown",
        "najgorszy dzień rynkowy (equity)" => "worst market day (equity)",
        "najgorszy dzień z zamknięciami (legacy)" => "worst day with closes (legacy)",
        "mediana dnia" => "median day",
        "skuteczność bez BE" => "win rate excluding BE",
        "najniższe equity" => "minimum equity",
        "seria stratnych dni" => "consecutive losing days",
        "zysk miesięcznie" => "monthly profit",
        "transakcje" => "trades",
        "liczba koszyków" => "basket count",
        "% wykorzystanych sygnałów" => "% signals used",
        "WSZYSTKIE STATYSTYKI" => "ALL STATISTICS",
        "KONTO NA ZERO" => "ACCOUNT DEPLETED",
        "BEZ TRANSAKCJI" => "NO TRADES",
        "pokolenie" => "generation",
        "ocena najlepszego" => "best score",
        "mediana" => "median",
        "najlepszy" => "best",
        "najlepszy z ukończonych" => "best completed",
        "wynik tego presetu" => "this preset's result",
        "ryzyko tego presetu" => "this preset's risk",
        "średnia" => "mean",
        "przebiegi" => "runs",
        "pozostało" => "remaining",
        "zysk" => "profit",
        "saldo" => "balance",
        "sygnały" => "signals",
        "koszyki" => "baskets",
        "wykorzystanie sygnałów" => "signal use",
        "obsunięcie" => "drawdown",
        "depozyt" => "deposit",
        "wątki" => "threads",
        "czas" => "time",
        "nieudane" => "failed",
        "ukończone" => "completed",
        "puste polecenie" => "empty command",
        "nie umiem odczytać programu z tego polecenia" => {
            "cannot identify the executable in this command"
        }
        "okno zamknięto w trakcie — nie wiem, czy to zadanie doszło do końca" => {
            "The window closed during execution — completion is unknown"
        }
        _ => value,
    }
}

/// Translate only the known producer envelope, never a custom preset name.
pub fn progress_text(language: Language, value: &str) -> String {
    if language == Language::Pl {
        return value.to_owned();
    }
    if let Some(rest) = value.strip_prefix("aktualnie liczone: ") {
        if let Some((name, tail)) = rest.rsplit_once(" · najdalej z ") {
            return format!(
                "currently running: {name} · furthest of {}",
                tail.replace(" naraz (najsłabszy ", " parallel (least advanced ")
            );
        }
        return format!("currently running: {rest}");
    }
    if let Some(rest) = value.strip_prefix("gotowe ") {
        let (status, name) = rest
            .split_once(" — ")
            .map(|(s, n)| (s, Some(n)))
            .unwrap_or((rest, None));
        // Only numeric completion counts belong to this envelope.
        if status
            .split_whitespace()
            .next()
            .is_some_and(|n| n.contains('/') && n.chars().all(|c| c.is_ascii_digit() || c == '/'))
        {
            let status = status
                .replace(" · liczone ", " · running ")
                .replace(" naraz", " in parallel")
                .replace(" · aktualnie liczone", " · currently running");
            return match name {
                Some(n) => format!("completed {status} — {n}"),
                None => format!("completed {status}"),
            };
        }
    }
    value.to_owned()
}

pub fn statistic_value(language: Language, key: &str, value: &str) -> String {
    if language == Language::Pl {
        return value.to_owned();
    }
    match key {
        "T-100: aktywność ukończonych" => value
            .replace(" z zamknięciami · ", " with closes · ")
            .replace(" bez zamknięć", " without closes"),
        "T-100: rozgrzewka" if value == "zimny start · pełne BID M1/M5/M15 · zwykle co najmniej 60 min" =>
            "cold start · complete BID M1/M5/M15 · usually at least 60 min".into(),
        "T-100: ocena lidera" if value == "wynik roboczy — aktywność i walidacja oceniane po zakończeniu" =>
            "provisional result — activity and validation assessed after completion".into(),
        "zegar zdarzeń" if value == "czas pliku po CLI · offset lub opóźnienie różne między presetami" =>
            "file time after CLI adjustment · offset or latency differs between presets".into(),
        "zegar zdarzeń" if value.starts_with("czas wykonania jak ticki · CLI ") => value
            .replacen("czas wykonania jak ticki · CLI ", "execution time in tick clock · CLI ", 1)
            .replace(" · opóźnienie ", " · latency "),
        "zakres mierzony" => value.replace(" (ostatni tick)", " (last tick)"),
        "zdarzenia eksportu" if value == "brak zdarzeń w mierzonym zakresie" =>
            "no events in the measured range".into(),
        "zdarzenia eksportu" => value.replace(" zdarzeń", " events"),
        "rozgrzewka rynku/SR" if value == "0 h — zimny start" => "0 h — cold start".into(),
        "rozgrzewka rynku/SR" => value
            .replace(" h historii · skan ", " h of history · scan ")
            .replace(" … start · bez handlu", " … start · no trading"),
        "najlepszy z ukończonych" => {
            if let Some(rest) = value.strip_suffix(" gotowych") {
                if let Some((name, counts)) = rest.rsplit_once(" · ") {
                    return format!("{name} · {counts} completed");
                }
            }
            value.to_owned()
        }
        "wynik tego presetu" => value.replace(" zamknięć", " closes").replace(',', "."),
        "ryzyko tego presetu" => value
            .replace("najgorszy dzień ", "worst day ")
            .replace(" · dno ", " · minimum equity ")
            .replace(" · dni+ ", " · positive days ")
            .replace(',', "."),
        "przebiegi" => value.replace(" naraz", " in parallel"),
        _ => value.to_owned(),
    }
}

#[macro_export]
macro_rules! localized_format {
    ($pl:literal, $en:literal $(, $args:expr)* $(,)?) => {
        if $crate::language::current() == $crate::language::Language::En {
            format!($en $(, $args)*)
        } else { format!($pl $(, $args)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t100_clock_and_range_catalog_preserves_counts_offsets_and_time_domain() {
        let cases = [
            ("T-100: aktywność ukończonych", "T-100: completed-run activity", "11 z zamknięciami · 0 bez zamknięć", "11 with closes · 0 without closes"),
            ("T-100: rozgrzewka", "T-100: warmup", "zimny start · pełne BID M1/M5/M15 · zwykle co najmniej 60 min", "cold start · complete BID M1/M5/M15 · usually at least 60 min"),
            ("T-100: ocena lidera", "T-100: leader assessment", "wynik roboczy — aktywność i walidacja oceniane po zakończeniu", "provisional result — activity and validation assessed after completion"),
            ("zegar zdarzeń", "event clock", "czas wykonania jak ticki · CLI -60 min · preset +180.000 min · opóźnienie 250 ms", "execution time in tick clock · CLI -60 min · preset +180.000 min · latency 250 ms"),
            ("zegar zdarzeń", "event clock", "czas pliku po CLI · offset lub opóźnienie różne między presetami", "file time after CLI adjustment · offset or latency differs between presets"),
            ("zakres mierzony", "measured range", "22.06.2026 00:00 … 05.09.2026 21:59 (ostatni tick)", "22.06.2026 00:00 … 05.09.2026 21:59 (last tick)"),
            ("zdarzenia eksportu", "export events", "22.06.2026 03:00 … 04.09.2026 17:45 · 123 zdarzeń", "22.06.2026 03:00 … 04.09.2026 17:45 · 123 events"),
            ("zdarzenia eksportu", "export events", "brak zdarzeń w mierzonym zakresie", "no events in the measured range"),
            ("rozgrzewka rynku/SR", "market/SR warmup", "0 h — zimny start", "0 h — cold start"),
            ("rozgrzewka rynku/SR", "market/SR warmup", "72 h historii · skan 11.06.2026 00:00 … start · bez handlu", "72 h of history · scan 11.06.2026 00:00 … start · no trading"),
            ("pełne dane ticków", "full tick history", "01.06.2026 00:00 … 05.09.2026 21:59", "01.06.2026 00:00 … 05.09.2026 21:59"),
            ("kapitał startowy", "starting capital", "600 $", "600 $"),
        ];
        let producer = include_str!("../../backtest/src/bin/bt.rs");
        for (pl_key, en_key, pl_value, en_value) in cases {
            assert!(producer.contains(&format!("\"{pl_key}\"")), "review current producer key: {pl_key}");
            assert_eq!(label(Language::En, pl_key), en_key);
            assert_eq!(statistic_value(Language::En, pl_key, pl_value), en_value);
            assert_eq!(label(Language::Pl, pl_key), pl_key);
            assert_eq!(statistic_value(Language::Pl, pl_key, pl_value), pl_value);
        }
        assert_eq!(statistic_value(Language::En, "custom strategy", "72 h historii · skan custom"), "72 h historii · skan custom");
        assert_eq!(statistic_value(Language::En, "T-100: rozgrzewka", "vendor custom status"), "vendor custom status");
    }
    #[test]
    fn current_and_completed_result_envelopes_preserve_preset_names() {
        assert_eq!(
            progress_text(
                Language::En,
                "gotowe 2/10 · liczone 4 naraz — custom, gotowe"
            ),
            "completed 2/10 · running 4 in parallel — custom, gotowe"
        );
        assert_eq!(
            progress_text(Language::En, "aktualnie liczone: custom, gotowe"),
            "currently running: custom, gotowe"
        );
        assert_eq!(
            statistic_value(
                Language::En,
                "najlepszy z ukończonych",
                "custom, gotowe · 2/10 gotowych"
            ),
            "custom, gotowe · 2/10 completed"
        );
        assert_eq!(
            progress_text(Language::En, "custom gotowe 2/10"),
            "custom gotowe 2/10"
        );
        assert_eq!(progress_text(Language::Pl, "gotowe 2/10"), "gotowe 2/10");
    }
    #[test]
    fn explicit_language_precedence_and_invalid_inputs() {
        assert_eq!(choose(None, None, None), Ok(Language::En));
        assert_eq!(choose(None, None, Some("pl\n")), Ok(Language::Pl));
        assert_eq!(choose(None, Some("en"), Some("pl")), Ok(Language::En));
        assert_eq!(choose(Some("pl"), Some("en"), Some("en")), Ok(Language::Pl));
        assert!(choose(Some("invalid"), Some("pl"), None).is_err());
        assert_eq!(choose(None, Some("invalid"), Some("pl")), Ok(Language::Pl));
    }
    #[test]
    fn localized_numbers_and_plural_forms_preserve_values() {
        assert_eq!(Language::En.number(1234.5, 1), "1\u{202f}234.5");
        assert_eq!(Language::Pl.number(1234.5, 1), "1\u{202f}234,5");
        assert_eq!(Language::En.tasks(1), "1 task");
        assert_eq!(Language::En.tasks(21), "21 tasks");
        assert_eq!(Language::Pl.tasks(22), "22 zadania");
        assert_eq!(Language::Pl.tasks(12), "12 zadań");
    }
    #[test]
    fn language_file_replaces_only_its_own_preference_and_custom_text_is_preserved() {
        let dir = std::env::temp_dir().join(format!(
            "conduit-monitor-language-{}-{}",
            std::process::id(),
            crate::teraz_ms()
        ));
        save(&dir, Language::Pl).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("language.txt")).unwrap(),
            "pl\n"
        );
        save(&dir, Language::En).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("language.txt")).unwrap(),
            "en\n"
        );
        assert_eq!(
            label(Language::En, "custom strategy α"),
            "custom strategy α"
        );
        assert_eq!(unit_in(Language::En, "custom units/s"), "custom units/s");
        std::fs::remove_file(dir.join("language.txt")).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
    #[test]
    fn localizing_presentation_does_not_modify_the_progress_protocol() {
        let state = crate::Postep {
            nazwa: "custom task α".into(),
            rodzaj: crate::TRENING.into(),
            jednostka_szybkosci: "ocen/s".into(),
            statystyki: crate::Statystyki(vec![("status_walidacji".into(), "holdout".into())]),
            ..Default::default()
        };
        let before = serde_json::to_string(&state).unwrap();
        assert_eq!(
            crate::badanie::walidacja_w_jezyku(&state.statystyki, Language::En),
            "Holdout · reserved historical data"
        );
        assert_eq!(
            unit_in(Language::En, &state.jednostka_szybkosci),
            "evaluations/s"
        );
        assert_eq!(serde_json::to_string(&state).unwrap(), before);
        assert_eq!(state.nazwa, "custom task α");
    }
}
