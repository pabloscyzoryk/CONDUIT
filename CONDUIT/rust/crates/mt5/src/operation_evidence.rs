//! Whitelisted operational evidence. Never serializes account pins or free-form payloads.
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    LocalValidation,
    LocalReceiptPending,
    LocalReceiptReview,
    RemoteRefusal,
    ConfirmationPending,
    TransportUnknown,
    Acknowledged,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub command: &'static str,
    pub request: Value,
    pub status: &'static str,
    pub retcode: Option<i64>,
    pub acknowledgement: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationEvidence {
    /// Monotonic within this bridge session; the runtime provenance identifies the session.
    pub sequence: u64,
    pub operation: &'static str,
    pub outcome: Outcome,
    pub attempts: Vec<Attempt>,
}

fn fields(value: &Value, keys: &[&str]) -> Value {
    let mut safe = Map::new();
    for key in keys {
        if let Some(v) = value.get(*key) {
            if v.is_number() || v.is_null() { safe.insert((*key).into(), v.clone()); }
        }
    }
    Value::Object(safe)
}

impl OperationEvidence {
    pub fn new(sequence: u64, operation: &'static str) -> Self {
        Self { sequence, operation, outcome: Outcome::LocalValidation, attempts: Vec::new() }
    }
    pub fn dispatched(&mut self, command: &'static str, request: &Value) {
        self.attempts.push(Attempt {
            command,
            request: fields(request, &["volume", "price", "sl", "tp", "ticket", "deviation"]),
            status: "dispatched", retcode: None, acknowledgement: Value::Null,
        });
    }
    pub fn response(&mut self, status: &'static str, value: Option<&Value>, retcode: Option<i64>, outcome: Outcome) {
        self.outcome = outcome;
        if let Some(attempt) = self.attempts.last_mut() {
            attempt.status = status;
            attempt.retcode = retcode;
            attempt.acknowledgement = value.map(|v| fields(v,
                &["retcode", "deal", "order", "position", "position_identifier", "volume", "price"]))
                .unwrap_or(Value::Null);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn evidence_excludes_account_pins_credentials_comments_and_unknown_response_fields() {
        let mut e = OperationEvidence::new(1, "open_market");
        e.dispatched("open_market", &json!({"volume":0.01,"sl":3990.,"comment":"SECRET_SENTINEL",
            "_expected_account":{"login":123},"password":"SECRET_SENTINEL"}));
        e.response("acknowledged", Some(&json!({"retcode":10009,"deal":8,"volume":0.01,
            "comment":"SECRET_SENTINEL","account":123})), Some(10009), Outcome::Acknowledged);
        let v = serde_json::to_value(e).unwrap();
        assert_eq!(v["attempts"][0]["request"],json!({"volume":0.01,"sl":3990.}));
        assert_eq!(v["attempts"][0]["acknowledgement"],json!({"retcode":10009,"deal":8,"volume":0.01}));
        assert!(!v.to_string().contains("SECRET_SENTINEL"));
    }
}
