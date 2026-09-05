
use crate::coalesce::Section;
use crate::ui;
use serde::{Deserialize, Serialize};

// ============================================================
//  SERWER → KLIENT
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
// `rename_all` dotyczy WARIANTÓW; pola wewnątrz wariantów zmienia dopiero
// `rename_all_fields`. Bez tego drugiego `req_id` szedłby po drucie jako
// `req_id`, a interfejs szuka `reqId`.
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServerMsg {
    Snapshot {
        state: Box<ui::UiSnapshot>,
    },
    Delta {
        rev: u64,
        patch: Box<DeltaPatch>,
    },
    Event {
        event: Box<ui::UiEvent>,
    },
    Ack {
        req_id: u64,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Pong {
        ts: i64,
        server_time: i64,
    },
}

/// Zawartość delty. Każde pole jest opcjonalne — wysyłamy tylko brudne sekcje.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quotes: Option<std::collections::BTreeMap<String, ui::Quote>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positions: Option<Vec<ui::Position>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pendings: Option<Vec<ui::PendingOrder>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baskets: Option<Vec<ui::Basket>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<Vec<ui::ClosedPosition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_history: Option<Vec<ui::PendingHistoryItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<ui::Stats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<Vec<ui::LogEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<ui::ChatMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lot: Option<ui::LotConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<crate::auth::AuthState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection: Option<ui::ConnectionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halt: Option<ui::HaltState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_override: Option<ui::RiskOverride>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sims: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bindings: Option<std::collections::BTreeMap<String, ui::ChannelBinding>>,
    /// Słownik formatów i zbiór łańcuchów. Jadą sekcją `Settings`, bo zmiana
    /// łańcucha jest zmianą KONFIGURACJI handlu, a nie zdarzeniem rynkowym —
    /// osobna sekcja tylko rozdrobniłaby to, co i tak zmienia się razem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formaty: Option<Vec<conduit_core::formaty::Format>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lancuchy: Option<conduit_core::formaty::Lancuchy>,
    /// Wskaźnik łańcucha trybu AUTO-EA (projekt EA-2). Jedzie razem
    /// z `lancuchy` — to jedna decyzja rozpisana na dwa pola, a delta bez
    /// wskaźnika kazałaby panelowi zgadywać, czy wskazanie zniknęło.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aktywny_ea: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorites: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<ui::EmailConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify: Option<ui::NotifyConfig>,
    // Bez pominięcia pustego pola KAŻDA delta niosła `"plan": null` — nawet
    // taka, która miała być pusta. Poza marnowaniem łącza łamało to kontrakt
    // „delta zawiera wyłącznie brudne sekcje", na którym stoi cały mechanizm
    // scalania po stronie panelu.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drabinka: Option<ui::DrabinkaLancuchow>,
    /// Drabinka trybu AUTO-EA („SKYNET-1", projekt EA-2c). Osobne pole, bo
    /// osobny stan — panel pokazuje dokładnie jedną z nich, tę od bieżącego
    /// trybu, i musi dostać obie, żeby przełączenie trybu nie czekało na
    /// kolejną deltę z drugą drabinką.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drabinka_ea: Option<ui::DrabinkaLancuchow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ui::TradingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lab: Option<crate::lab::LabState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo: Option<crate::demo::DemoState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scalanie: Option<ui::PostepScalania>,
    /// Podsumowanie pozycji spoza bota. Jedzie z sekcją `Positions`, bo zmienia
    /// się dokładnie wtedy, co ona.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreign: Option<ui::ForeignSummary>,
}

impl DeltaPatch {
    /// Czy delta niesie cokolwiek? Pustych ramek nie wysyłamy.
    pub fn is_empty(&self) -> bool {
        self.quotes.is_none()
            && self.positions.is_none()
            && self.pendings.is_none()
            && self.baskets.is_none()
            && self.closed.is_none()
            && self.pending_history.is_none()
            && self.balance.is_none()
            && self.stats.is_none()
            && self.logs.is_none()
            && self.messages.is_none()
            && self.settings.is_none()
            && self.lot.is_none()
            && self.preset_id.is_none()
            && self.auth.is_none()
            && self.connection.is_none()
            && self.halt.is_none()
            && self.risk_override.is_none()
            && self.sims.is_none()
            && self.bindings.is_none()
            && self.favorites.is_none()
            && self.aktywny_ea.is_none()
            && self.email.is_none()
            && self.notify.is_none()
            && self.drabinka.is_none()
            && self.drabinka_ea.is_none()
            && self.language.is_none()
            && self.mode.is_none()
            && self.lab.is_none()
            && self.demo.is_none()
            && self.scalanie.is_none()
            && self.foreign.is_none()
    }

    /// Buduje deltę z pełnego stanu, biorąc tylko wskazane sekcje.
    pub fn build(snap: &ui::UiSnapshot, sections: crate::coalesce::Sections) -> DeltaPatch {
        let mut p = DeltaPatch::default();
        if sections.contains(Section::Quotes) {
            p.quotes = Some(snap.quotes.clone());
        }
        if sections.contains(Section::Positions) {
            p.positions = Some(snap.positions.clone());
            p.balance = Some(snap.balance);
            p.foreign = Some(snap.foreign.clone());
        }
        if sections.contains(Section::Pendings) {
            p.pendings = Some(snap.pendings.clone());
            p.foreign = Some(snap.foreign.clone());
        }
        if sections.contains(Section::Baskets) {
            p.baskets = Some(snap.baskets.clone());
        }
        if sections.contains(Section::Closed) {
            p.closed = Some(snap.closed.clone());
            p.balance = Some(snap.balance);
        }
        if sections.contains(Section::PendingHistory) {
            p.pending_history = Some(snap.pending_history.clone());
        }
        if sections.contains(Section::Stats) {
            p.stats = Some(snap.stats.clone());
            p.balance = Some(snap.balance);
        }
        if sections.contains(Section::Logs) {
            p.logs = Some(snap.logs.clone());
        }
        if sections.contains(Section::Messages) {
            p.messages = Some(snap.messages.clone());
        }
        if sections.contains(Section::Settings) {
            p.settings = Some(snap.settings.clone());
            p.lot = Some(snap.lot.clone());
            p.preset_id = Some(snap.preset_id.clone());
            // `redacted()`, a nie `clone()`: delta leci tą samą drogą co
            // migawka i musi być tak samo pozbawiona hasła
            p.email = Some(snap.email.redacted());
            p.notify = Some(snap.notify.clone());
            p.drabinka = Some(snap.drabinka.clone());
            p.drabinka_ea = Some(snap.drabinka_ea.clone());
            p.favorites = Some(snap.favorites.clone());
            p.language = Some(snap.language.clone());
            p.formaty = Some(snap.formaty.clone());
            p.lancuchy = Some(snap.lancuchy.clone());
            p.aktywny_ea = Some(snap.aktywny_ea.clone());
        }
        if sections.contains(Section::Auth) {
            p.auth = Some(snap.auth.clone());
        }
        if sections.contains(Section::Connection) {
            p.connection = Some(snap.connection.clone());
        }
        if sections.contains(Section::Halt) {
            p.halt = Some(snap.halt.clone());
            p.risk_override = Some(snap.risk_override.clone());
        }
        if sections.contains(Section::Sims) {
            p.sims = Some(snap.sims.clone());
        }
        if sections.contains(Section::Bindings) {
            p.bindings = Some(snap.bindings.clone());
        }
        if sections.contains(Section::Mode) {
            p.mode = Some(snap.mode);
        }
        if sections.contains(Section::Lab) {
            p.lab = Some(snap.lab.clone());
        }
        if sections.contains(Section::Demo) {
            p.demo = Some(snap.demo.clone());
        }
        if sections.contains(Section::Scalanie) {
            p.scalanie = Some(snap.scalanie.clone());
        }
        p
    }
}

// ============================================================
//  KLIENT → SERWER
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClientMsg {
    /// Pusta lista = wszystkie sekcje. Przydatne dla lekkich klientów
    /// (np. widget zasobnika, który chce tylko statystyki).
    Subscribe {
        #[serde(default)]
        sections: Vec<Section>,
    },
    /// Identyfikator korelacji nazywa się `reqId`, a NIE `id`.
    /// To nie jest kosmetyka: `cmd` jest wpłaszczane (`flatten`), a wśród
    /// komend są takie z własnym polem `id` (preset, koszyk, symulacja).
    /// Przy nazwie `id` oba pola trafiłyby na ten sam klucz JSON i komendy
    /// z identyfikatorem stałyby się niemożliwe do wysłania.
    Command {
        req_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_session: Option<String>,
        #[serde(flatten)]
        cmd: Command,
    },
    SettingsPatch {
        req_id: u64,
        patch: serde_json::Value,
    },
    Ping {
        ts: i64,
    },
}

/// Akcje użytkownika. Odpowiadają 1:1 metodom z `AppContextValue` w React,
/// żeby warstwa transportu nie musiała niczego tłumaczyć.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Command {
    // --- konfiguracja (obsługiwana w całości przez serwer) ---
    SetMode {
        mode: ui::TradingMode,
    },
    /// `values` to katalog presetów wbudowany w interfejs. Serwer najpierw
    /// szuka presetu o tej nazwie w `presets/` (te są nadrzędne, bo pisze je
    /// użytkownik), a dopiero gdy go nie ma — bierze wartości z UI. Dzięki
    /// temu galeria presetów działa od pierwszego uruchomienia, zanim
    /// ktokolwiek utworzy plik na dysku.
    ApplyPreset {
        id: String,
        #[serde(default)]
        values: Option<serde_json::Value>,
    },
    ResetSettings,
    SetLot {
        lot: ui::LotConfig,
    },
    SetBinding {
        channel_id: i64,
        patch: serde_json::Value,
    },
    /// PEŁNY ZAPIS ZBIORU ŁAŃCUCHÓW — tworzenie, zmiana nazwy, usunięcie,
    /// edycja pułapów. Jeden zapis = jeden stan, więc panel nie musi wysyłać
    /// serii poleceń, których połowa może się nie udać.
    SetLancuchy {
        lancuchy: conduit_core::formaty::Lancuchy,
        /// WSKAŹNIK TRYBU AUTO-EA (projekt EA-2). Brak pola = „nie ruszaj":
        /// panel w trybach nie-EA nigdy go nie wysyła, więc zapis z tamtej
        /// strony NIE MOŻE po cichu skasować wskazania warstwy EA.
        #[serde(default)]
        aktywny_ea: Option<String>,
    },
    SetAktywnyLancuch {
        nazwa: String,
    },
    SetEmail {
        email: ui::EmailConfig,
    },
    /// Przycisk „wyślij mail testowy" w ustawieniach. Odpowiedź `ack` niesie
    /// wynik PRAWDZIWEJ próby wysyłki, nie „przyjęto do kolejki" — inaczej
    /// przycisk diagnostyczny nie diagnozowałby niczego.
    SendTestEmail,
    SendTestNotify,
    SetNotify {
        notify: ui::NotifyConfig,
    },
    SetDrabinka {
        drabinka: ui::DrabinkaLancuchow,
        #[serde(default)]
        tryb: Option<ui::TradingMode>,
    },
    ToggleFavorite {
        symbol: String,
    },
    ClearLogs,

    // --- handel (przekazywane do środowiska uruchomieniowego) ---
    OpenOrder {
        kind: String,
        direction: ui::Direction,
        volume: f64,
        #[serde(default)]
        price: Option<f64>,
        #[serde(default)]
        sl: Option<f64>,
        #[serde(default)]
        tp: Option<f64>,
    },
    ClosePosition {
        ticket: u64,
    },
    /// zamknięcie części pozycji (wolumen w lotach)
    ClosePartial {
        ticket: u64,
        volume: f64,
    },
    ModifyPosition {
        ticket: u64,
        sl: Option<f64>,
        tp: Option<f64>,
    },
    CloseBulk {
        which: String,
    },
    DeletePending {
        ticket: u64,
    },
    ModifyPending {
        ticket: u64,
        price: f64,
        sl: Option<f64>,
        tp: Option<f64>,
    },
    DeleteAllPendings,
    CloseBasket {
        id: u32,
    },
    UpdateBasket {
        id: u32,
        patch: serde_json::Value,
    },
    ResumeTrading,
    RearmGuard,

    SimulateMessage {
        text: String,
        /// `chat_id` docelowego kanału; brak = pseudokanał panelu (`0`)
        #[serde(default)]
        channel_id: Option<i64>,
        /// temat forum — dla silnika to OSOBNE źródło (własne koszyki)
        #[serde(default)]
        topic_id: Option<i64>,
        /// własny numer wiadomości; brak = numer z zakresu panelu (ujemny)
        #[serde(default)]
        msg_id: Option<i64>,
        /// numer wiadomości, NA KTÓRĄ odpowiadamy
        #[serde(default)]
        reply_to: Option<i64>,
        /// numer wiadomości, KTÓRĄ edytujemy — wtedy to nie jest nowa
        /// wiadomość, tylko podmiana treści tamtej
        #[serde(default)]
        edit_of: Option<i64>,
    },
    ExecuteMessage {
        id: String,
    },
    DismissMessage {
        id: String,
    },

    // --- symulacje ---
    /// Nowa instancja symulacji. `mode` to WŁASNY tryb handlu instancji
    /// (AUTO / AUTO-EA / AI): jej silniki dostają flagę `tryb_auto_ea`
    /// niezależnie od trybu głównego bota, więc bot może grać AUTO-EA,
    /// a symulacja obok AUTO — i na odwrót (patrz [`crate::symulacje`]).
    ///
    /// **Kontrakt zera:** `None` (stare panele nie wysyłają pola) =
    /// dziedziczenie trybu głównego bota, czyli zachowanie sprzed zmiany.
    /// MANUAL nie ma w symulacji sensu (nie ma komu klikać „Wykonaj")
    /// i jest odrzucany przy dodawaniu instancji.
    AddSim {
        preset: String,
        name: String,
        balance: f64,
        lot: f64,
        #[serde(default)]
        mode: Option<ui::TradingMode>,
    },
    RemoveSim {
        id: String,
    },
    ResetSim {
        id: String,
    },
}

impl Command {
    /// Czy komenda dotyczy handlu (wymaga podłączonego brokera)?
    pub fn needs_runtime(&self) -> bool {
        matches!(
            self,
            Command::OpenOrder { .. }
                | Command::ClosePosition { .. }
                | Command::ClosePartial { .. }
                | Command::ModifyPosition { .. }
                | Command::CloseBulk { .. }
                | Command::DeletePending { .. }
                | Command::ModifyPending { .. }
                | Command::DeleteAllPendings
                | Command::CloseBasket { .. }
                | Command::UpdateBasket { .. }
                | Command::ResumeTrading
                | Command::RearmGuard
                | Command::SimulateMessage { .. }
                | Command::ExecuteMessage { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coalesce::Sections;

    fn parse(s: &str) -> ClientMsg {
        serde_json::from_str(s).expect("poprawny komunikat klienta")
    }

    #[test]
    fn subscribe_bez_sekcji_jest_poprawne() {
        match parse(r#"{"type":"subscribe"}"#) {
            ClientMsg::Subscribe { sections } => assert!(sections.is_empty()),
            other => panic!("zły wariant: {other:?}"),
        }
    }

    #[test]
    fn subscribe_z_lista_sekcji() {
        match parse(r#"{"type":"subscribe","sections":["positions","stats"]}"#) {
            ClientMsg::Subscribe { sections } => {
                assert_eq!(sections, vec![Section::Positions, Section::Stats]);
            }
            other => panic!("zły wariant: {other:?}"),
        }
    }

    #[test]
    fn komenda_ma_plaski_ksztalt() {
        let m = parse(r#"{"type":"command","reqId":7,"cmd":"closePosition","ticket":12345}"#);
        match m {
            ClientMsg::Command {
                req_id,
                cmd: Command::ClosePosition { ticket },
                account_session,
            } => {
                assert_eq!(req_id, 7);
                assert_eq!(ticket, 12345);
                assert!(account_session.is_none());
            }
            other => panic!("zły wariant: {other:?}"),
        }
    }

    /// REGRESJA: przy nazwie `id` dla identyfikatora korelacji komendy
    /// z własnym polem `id` były NIEMOŻLIWE do wysłania — `flatten` sadzał
    /// oba pola na ten sam klucz JSON.
    #[test]
    fn komendy_z_wlasnym_polem_id_dzialaja() {
        match parse(r#"{"type":"command","reqId":3,"cmd":"applyPreset","id":"T4-MAX"}"#) {
            ClientMsg::Command {
                req_id,
                cmd: Command::ApplyPreset { id, values },
                ..
            } => {
                assert_eq!(req_id, 3);
                assert_eq!(id, "T4-MAX");
                assert!(values.is_none());
            }
            other => panic!("zły wariant: {other:?}"),
        }
        match parse(r#"{"type":"command","reqId":4,"cmd":"closeBasket","id":12}"#) {
            ClientMsg::Command {
                req_id,
                cmd: Command::CloseBasket { id },
                ..
            } => {
                assert_eq!(req_id, 4);
                assert_eq!(id, 12);
            }
            other => panic!("zły wariant: {other:?}"),
        }
        match parse(r#"{"type":"command","reqId":5,"cmd":"executeMessage","id":"m17"}"#) {
            ClientMsg::Command {
                cmd: Command::ExecuteMessage { id },
                ..
            } => assert_eq!(id, "m17"),
            other => panic!("zły wariant: {other:?}"),
        }
    }

    /// REGRESJA: `rename_all` na enumie zmienia tylko WARIANTY. Bez
    /// `rename_all_fields` pola wielowyrazowe (`channel_id`, `req_id`)
    /// wychodziły po drucie w snake_case i interfejs ich nie trafiał.
    #[test]
    fn pola_wielowyrazowe_ida_w_camelcase() {
        let m = parse(
            r#"{"type":"command","reqId":8,"cmd":"setBinding","channelId":-1001234,"patch":{"monitored":true}}"#,
        );
        match m {
            ClientMsg::Command {
                req_id,
                cmd: Command::SetBinding { channel_id, patch },
                ..
            } => {
                assert_eq!(req_id, 8);
                assert_eq!(channel_id, -1001234);
                assert_eq!(patch["monitored"], true);
            }
            other => panic!("zły wariant: {other:?}"),
        }

        let j = serde_json::to_value(ServerMsg::Pong {
            ts: 5,
            server_time: 9,
        })
        .unwrap();
        assert_eq!(
            j["serverTime"], 9,
            "pole `server_time` musi wyjść jako `serverTime`: {j}"
        );
    }

    #[test]
    fn preset_moze_przyjsc_z_wartosciami_z_ui() {
        match parse(
            r#"{"type":"command","reqId":9,"cmd":"applyPreset","id":"MONTE-CARLO","values":{"max_dd_pct":60}}"#,
        ) {
            ClientMsg::Command {
                cmd: Command::ApplyPreset { id, values },
                ..
            } => {
                assert_eq!(id, "MONTE-CARLO");
                assert_eq!(values.unwrap()["max_dd_pct"], 60);
            }
            other => panic!("zły wariant: {other:?}"),
        }
    }

    #[test]
    fn account_session_is_optional_flat_and_camel_case() {
        let old = parse(r#"{"type":"command","reqId":2,"cmd":"closePosition","ticket":7}"#);
        assert!(matches!(old, ClientMsg::Command { account_session: None, .. }));
        let new = parse(r#"{"type":"command","reqId":3,"accountSession":"synthetic-B-2","cmd":"closeBasket","id":7}"#);
        match new {
            ClientMsg::Command { account_session, cmd: Command::CloseBasket { id }, .. } => {
                assert_eq!(account_session.as_deref(), Some("synthetic-B-2"));
                assert_eq!(id, 7);
            }
            _ => panic!("wrong scoped command"),
        }
        let j=serde_json::to_value(ClientMsg::Command {req_id:9, account_session:Some("synthetic-A-3".into()), cmd:Command::ClosePosition {ticket:7}}).unwrap();
        assert_eq!(j["accountSession"],"synthetic-A-3");
        assert!(j.get("account_session").is_none());
    }

    #[test]
    fn add_sim_bez_trybu_i_z_trybem() {
        match parse(
            r#"{"type":"command","reqId":10,"cmd":"addSim","preset":"HYPER-2","name":"","balance":200.0,"lot":0.01}"#,
        ) {
            ClientMsg::Command {
                cmd: Command::AddSim { preset, mode, .. },
                ..
            } => {
                assert_eq!(preset, "HYPER-2");
                assert!(
                    mode.is_none(),
                    "stary komunikat = dziedziczenie trybu, nie błąd"
                );
            }
            other => panic!("zły wariant: {other:?}"),
        }
        match parse(
            r#"{"type":"command","reqId":11,"cmd":"addSim","preset":"HYPER-2","name":"ea","balance":200.0,"lot":0.01,"mode":"AUTO-EA"}"#,
        ) {
            ClientMsg::Command {
                cmd: Command::AddSim { mode, .. },
                ..
            } => {
                assert_eq!(mode, Some(ui::TradingMode::AutoEa));
            }
            other => panic!("zły wariant: {other:?}"),
        }
    }

    #[test]
    fn komenda_otwarcia_z_polami_opcjonalnymi() {
        let m = parse(
            r#"{"type":"command","reqId":1,"cmd":"openOrder","direction":"BUY","kind":"market","volume":0.05}"#,
        );
        match m {
            ClientMsg::Command {
                cmd:
                    Command::OpenOrder {
                        kind,
                        direction,
                        volume,
                        price,
                        sl,
                        tp,
                    },
                ..
            } => {
                assert_eq!(kind, "market");
                assert_eq!(direction, ui::Direction::Buy);
                assert_eq!(volume, 0.05);
                assert!(price.is_none() && sl.is_none() && tp.is_none());
            }
            other => panic!("zły wariant: {other:?}"),
        }
    }

    #[test]
    fn tryb_pracy_jest_tekstem_z_ui() {
        let m = parse(r#"{"type":"command","reqId":2,"cmd":"setMode","mode":"AI"}"#);
        match m {
            ClientMsg::Command {
                cmd: Command::SetMode { mode },
                ..
            } => {
                assert_eq!(mode, ui::TradingMode::Ai)
            }
            other => panic!("zły wariant: {other:?}"),
        }

        let m = parse(r#"{"type":"command","reqId":3,"cmd":"setMode","mode":"AUTO-EA"}"#);
        match m {
            ClientMsg::Command {
                cmd: Command::SetMode { mode },
                ..
            } => {
                assert_eq!(mode, ui::TradingMode::AutoEa)
            }
            other => panic!("zły wariant: {other:?}"),
        }
    }

    #[test]
    fn snapshot_i_delta_maja_pole_type() {
        let snap = ServerMsg::Snapshot {
            state: Box::new(ui::UiSnapshot::empty(0)),
        };
        let j = serde_json::to_value(&snap).unwrap();
        assert_eq!(j["type"], "snapshot");
        assert!(j["state"].is_object());

        let d = ServerMsg::Delta {
            rev: 3,
            patch: Box::new(DeltaPatch::default()),
        };
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["type"], "delta");
        assert_eq!(j["rev"], 3);
        // pusta delta nie ma żadnych kluczy sekcji
        assert_eq!(j["patch"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn delta_niesie_tylko_brudne_sekcje() {
        let mut snap = ui::UiSnapshot::empty(0);
        snap.balance = 1234.0;
        snap.logs.push(ui::LogEntry {
            id: 1,
            t: 0,
            category: "events".into(),
            title: "test".into(),
            content: String::new(),
            level: "info".into(),
        });

        let patch = DeltaPatch::build(&snap, Sections::one(Section::Logs));
        assert!(patch.logs.is_some());
        assert!(patch.positions.is_none());
        assert!(patch.stats.is_none());
        assert!(!patch.is_empty());

        let j = serde_json::to_value(&patch).unwrap();
        assert_eq!(
            j.as_object().unwrap().len(),
            1,
            "delta zawiera nadmiarowe pola: {j}"
        );
    }

    #[test]
    fn delta_niesie_postep_scalania() {
        let mut snap = ui::UiSnapshot::empty(0);
        snap.scalanie = ui::PostepScalania {
            aktywne: true,
            faza: "trwa".into(),
            etap: "kronika".into(),
            postep: 0.42,
            zrobione: 42,
            wszystkich: 100,
            predkosc: "12.5 MB/s".into(),
            eta_ms: 5_000,
            czas_ms: 3_000,
            ..Default::default()
        };
        let patch = DeltaPatch::build(&snap, Sections::one(Section::Scalanie));
        let s = patch
            .scalanie
            .as_ref()
            .expect("postęp MUSI jechać w delcie");
        assert_eq!(s.postep, 0.42);
        assert_eq!(s.predkosc, "12.5 MB/s");
        assert!(
            !patch.is_empty(),
            "delta z samym postępem nie może uchodzić za pustą"
        );

        // …i wyłącznie ona: postęp tyka kilka razy na sekundę, więc doklejenie
        // czegokolwiek innego znaczyłoby przepisywanie tego przez łącze.
        let j = serde_json::to_value(&patch).unwrap();
        assert_eq!(
            j.as_object().unwrap().len(),
            1,
            "delta zawiera nadmiarowe pola: {j}"
        );
        assert_eq!(
            j["scalanie"]["etaMs"], 5_000,
            "kontrakt z panelem jest w camelCase"
        );

        // sekcja nieoznaczona jako brudna nie ma prawa nieść postępu
        let inna = DeltaPatch::build(&snap, Sections::one(Section::Logs));
        assert!(inna.scalanie.is_none());
    }

    #[test]
    fn delta_pozycji_dokłada_balans() {
        let snap = ui::UiSnapshot::empty(0);
        let patch = DeltaPatch::build(&snap, Sections::one(Section::Positions));
        assert!(patch.positions.is_some());
        assert!(
            patch.balance.is_some(),
            "UI liczy zysk względem balansu — musi przyjść razem"
        );
    }

    #[test]
    fn ack_bez_bledu_nie_wysyla_pola_error() {
        let j = serde_json::to_value(ServerMsg::Ack {
            req_id: 1,
            ok: true,
            error: None,
        })
        .unwrap();
        assert_eq!(j["type"], "ack");
        assert_eq!(j["reqId"], 1);
        assert!(j.get("error").is_none());
    }

    #[test]
    fn zdarzenie_ma_rozpoznawalny_rodzaj() {
        let ev = ServerMsg::Event {
            event: Box::new(ui::UiEvent::Toast {
                level: "warn".into(),
                title: "Uwaga".into(),
                text: "tekst".into(),
            }),
        };
        let j = serde_json::to_value(&ev).unwrap();
        assert_eq!(j["type"], "event");
        assert_eq!(j["event"]["kind"], "toast");
    }

    #[test]
    fn komendy_handlowe_wymagaja_srodowiska() {
        assert!(Command::ClosePosition { ticket: 1 }.needs_runtime());
        assert!(!Command::SetMode {
            mode: ui::TradingMode::Auto
        }
        .needs_runtime());
        assert!(!Command::ClearLogs.needs_runtime());
    }
}
