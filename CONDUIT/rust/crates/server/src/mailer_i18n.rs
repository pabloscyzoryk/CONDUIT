//! Presentation at the email boundary. Engine state, journal text, user subject
//! templates and the original Telegram message are never rewritten.
use super::{MailCategory, SubjectVars};
use regex::{Captures, Regex};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Pl,
    En,
}

impl Language {
    pub fn from_app(value: &str) -> Self {
        if value == "en" { Self::En } else { Self::Pl }
    }
    pub fn choose<'a>(self, pl: &'a str, en: &'a str) -> &'a str {
        if self == Self::En { en } else { pl }
    }
    pub fn category(self, category: MailCategory) -> &'static str {
        if self == Self::Pl { return category.label(); }
        match category {
            MailCategory::Lifecycle => "bot start/stop",
            MailCategory::Mt5Connection => "MT5 connection",
            MailCategory::Mt5RecoveryFailed => "MT5 recovery failed",
            MailCategory::Drawdown => "drawdown limit",
            MailCategory::OrderError => "order error",
            MailCategory::Summary => "summary",
            MailCategory::SignalUnreadable => "unreadable signal",
            MailCategory::Test => "test",
        }
    }
}

static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([^{}]*)\}").unwrap());
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
struct Rule { regex: Regex, names: Vec<String>, target: String, specificity: usize }

fn identity(spec: &str, auto: &mut usize) -> String {
    let name = spec.split(':').next().unwrap_or("");
    if !name.is_empty() { return name.to_string(); }
    let value = auto.to_string(); *auto += 1; value
}

fn compile_rules(reverse: bool) -> Vec<Rule> {
    let pairs: Vec<(String, String)> = serde_json::from_str(include_str!("engine_translations.json"))
        .expect("validated presentation dictionary");
    let mut rules = Vec::new();
    for (pl, en) in pairs {
        let (source, target) = if reverse { (en, pl) } else { (pl, en) };
        let mut pattern = String::from("(?s)^");
        let mut names = Vec::new();
        let mut end = 0; let mut auto = 0;
        for capture in TOKEN.captures_iter(&source) {
            let whole = capture.get(0).unwrap();
            let literal = regex::escape(&source[end..whole.start()]);
            pattern.push_str(&SPACE.replace_all(&literal, r"\s+"));
            pattern.push_str("(.*?)");
            names.push(identity(&capture[1], &mut auto));
            end = whole.end();
        }
        pattern.push_str(&SPACE.replace_all(&regex::escape(&source[end..]), r"\s+"));
        pattern.push('$');
        let specificity = TOKEN.replace_all(&source, "").len();
        let mut target_auto = 0;
        let complete = TOKEN.captures_iter(&target)
            .all(|capture| names.contains(&identity(&capture[1], &mut target_auto)));
        if specificity >= 5 && complete {
            rules.push(Rule { regex: Regex::new(&pattern).expect("valid escaped dictionary template"), names, target, specificity });
        }
    }
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.specificity));
    rules
}
static RULES_EN: LazyLock<Vec<Rule>> = LazyLock::new(|| compile_rules(false));
static RULES_PL: LazyLock<Vec<Rule>> = LazyLock::new(|| compile_rules(true));

pub fn text(language: Language, source: &str) -> String {
    translate(source, 0, language)
}

fn translate(source: &str, depth: usize, language: Language) -> String {
    if depth > 4 { return source.to_string(); }
    let rules = if language == Language::En { &RULES_EN } else { &RULES_PL };
    for rule in rules.iter() {
        let Some(captures) = rule.regex.captures(source) else { continue; };
        let values: HashMap<&str, &str> = rule.names.iter().enumerate()
            .map(|(i, name)| (name.as_str(), captures.get(i + 1).map(|value| value.as_str()).unwrap_or("")))
            .collect();
        let mut auto = 0;
        return TOKEN.replace_all(&rule.target, |capture: &Captures<'_>| {
            let name = identity(&capture[1], &mut auto);
            match values.get(name.as_str()) {
                Some(value) if matches!(name.as_str(), "e" | "powod" | "opis" | "zostaje" | "category" | "r" | "reason" | "p" | "przyczyna" | "b" | "opis_stopu" | "naglowek") => translate(value, depth + 1, language),
                Some(value) => value.to_string(),
                None => capture[0].to_string(),
            }
        }).into_owned();
    }
    if let Some(rest) = source.strip_prefix("  • ") { return format!("  • {}", translate(rest, depth + 1, language)); }
    if source.contains('\n') { return source.split('\n').map(|line| translate(line, depth + 1, language)).collect::<Vec<_>>().join("\n"); }
    source.to_string()
}

/// Translate the explanatory prefix; keep the entire source-message block
/// byte-for-byte, even if it happens to contain one of our diagnostic phrases.
pub fn body(language: Language, source: &str) -> String {
    for (pl, en) in [("\nTREŚĆ:\n", "\nSOURCE MESSAGE:\n"), ("\nTreść wiadomości:\n", "\nMessage text:\n")] {
        for marker in [pl, en] {
            if let Some((prefix, raw)) = source.split_once(marker) {
                return format!("{}{}{}", translate(prefix, 0, language), language.choose(pl, en), raw);
            }
        }
    }
    translate(source, 0, language)
}

pub fn variables(snap: &crate::ui::UiSnapshot, cat: MailCategory, subject: &str, now: i64) -> SubjectVars {
    let language = Language::from_app(&snap.language);
    let mut vars = super::zmienne_tematu(snap, cat, subject, now);
    for item in &mut vars.items {
        item.label = text(language, &item.label);
        match item.name.as_str() {
            "kategoria" => item.value = language.category(cat).to_string(),
            "zdarzenie" => item.value = text(language, subject),
            "preset" if snap.preset_id.trim().is_empty() => item.value = language.choose("(ustawienia własne)", "(custom settings)").into(),
            _ => {},
        }
    }
    vars
}

pub fn system_subject(language: Language, cat: MailCategory, subject: &str) -> String {
    format!("{} {} — {}", crate::notify::TAG, language.category(cat), text(language, subject))
}

/// The exact same subject renderer is used for previews and actual deliveries.
/// Custom template literals remain user-owned; only built-in variables change.
pub fn subject(snap: &crate::ui::UiSnapshot, cat: MailCategory, event: &str, template: &str, now: i64) -> String {
    let language = Language::from_app(&snap.language);
    let fallback = || system_subject(language, cat, event);
    if template.trim().is_empty() { return fallback(); }
    let rendered = super::render_subject(template, &variables(snap, cat, event, now));
    let one_line = rendered.replace(['\n', '\r'], " ").trim().to_string();
    if one_line.is_empty() { fallback() } else { one_line }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_language_default_and_unknown_values_remain_polish() {
        assert_eq!(Language::default(), Language::Pl);
        assert_eq!(Language::from_app(""), Language::Pl);
        assert_eq!(Language::from_app("de"), Language::Pl);
        assert_eq!(text(Language::Pl, "Sygnał odrzucony"), "Sygnał odrzucony");
    }
    #[test]
    fn shared_templates_translate_without_losing_broker_values() {
        let pairs: Vec<(String, String)> = serde_json::from_str(include_str!("engine_translations.json")).unwrap();
        for (pl, en) in pairs {
            let source = TOKEN.replace_all(&pl, "7301").into_owned();
            let expected = TOKEN.replace_all(&en, "7301").into_owned();
            assert_eq!(text(Language::En, &source), expected, "{pl}");
        }
        assert_eq!(text(Language::En, "koszyk B15: broker odrzucił zmianę SL — INVALID_STOPS"), "basket B15: broker rejected the SL change — INVALID_STOPS");
    }
    #[test]
    fn raw_telegram_blocks_are_never_translated() {
        let raw = "Sygnał odrzucony\nBUY GOLD 2400\nSL 2390\nTP 2420";
        let source = format!("Sygnał ODROCZONY — nie wykonany\nTREŚĆ:\n{raw}");
        let translated = body(Language::En, &source);
        assert_eq!(translated, format!("DEFERRED signal — not executed\nSOURCE MESSAGE:\n{raw}"));
        assert_eq!(body(Language::Pl, &source), source);
    }
    #[test]
    fn english_engine_profit_budget_diagnostic_is_presented_in_polish_when_selected() {
        assert_eq!(text(Language::Pl, "PROFIT BUDGET: MissingStop; new order withheld"),
            "PROFIT BUDGET: MissingStop; nowe zlecenie wstrzymane");
        assert_eq!(text(Language::En, "PROFIT BUDGET: MissingStop; new order withheld"),
            "PROFIT BUDGET: MissingStop; new order withheld");
    }
    #[test]
    fn custom_subject_literals_and_account_values_remain_unchanged() {
        let mut snap = crate::ui::UiSnapshot::empty(0);
        snap.language = "en".into();
        snap.connection.account.login = 7301;
        let rendered = subject(&snap, MailCategory::Summary, "Podsumowanie dnia", "Mój własny temat ${login}: ${kategoria} / ${zdarzenie}", 0);
        assert_eq!(rendered, "Mój własny temat 7301: summary / Daily summary");
        assert_eq!(subject(&snap, MailCategory::Summary, "Podsumowanie dnia", "\n  ", 0), "[CONDUIT] summary — Daily summary");
    }
}
