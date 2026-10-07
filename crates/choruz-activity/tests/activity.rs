use choruz_activity::{TelemetryEvent, redact_sensitive_text, sanitize_value, validate_batch};
use serde_json::{Value, json};

fn event() -> Value {
    json!({
        "eventId":"owned-1", "schemaVersion":1, "traceId":"trace:1",
        "spanId":"span_1", "sessionId":"session.1", "name":"button.click",
        "ts":"2026-09-28T00:00:00Z", "durationMs":0,
        "data":{"component":"send", "result":"success"}
    })
}

#[test]
fn wire_contract_rejects_unknown_fields_and_invalid_batches() {
    let valid: TelemetryEvent = serde_json::from_value(event()).unwrap();
    validate_batch(&[valid]).unwrap();
    assert!(validate_batch(&[]).is_err());
    for count in [100, 101] {
        let events: Vec<TelemetryEvent> = (0..count)
            .map(|_| serde_json::from_value(event()).unwrap())
            .collect();
        assert_eq!(validate_batch(&events).is_ok(), count == 100);
    }
    let mut unknown = event();
    unknown["principalId"] = json!("untrusted-actor");
    assert!(serde_json::from_value::<TelemetryEvent>(unknown).is_err());
    for (field, value) in [
        ("schemaVersion", json!(2)),
        ("eventId", json!("")),
        ("traceId", json!("space forbidden")),
        ("spanId", json!("x".repeat(129))),
        ("sessionId", json!("non-ascii-ñ")),
        ("name", json!("bad/name")),
        ("durationMs", json!(-1)),
        ("data", json!([])),
        ("data", json!({"text":"x".repeat(16_384)})),
    ] {
        let mut input = event();
        input[field] = value;
        let parsed = serde_json::from_value(input).unwrap();
        assert!(validate_batch(&[parsed]).is_err(), "accepted {field}");
    }
}

#[test]
fn redaction_preserves_observations_but_removes_nested_sensitive_values() {
    assert_eq!(
        sanitize_value(json!({
            "component":"send", "count":2, "ok":true,
            "entries":[{"accessToken":"secret-value", "workspacePath":"/private"},
                {"private":true,"text":"private text","result":"failed"},
                {"privacy":"private","preview":"private preview"}],
            "message":"request Bearer abc, password: xyz) done"
        })),
        json!({
            "component":"send", "count":2, "ok":true,
            "entries":[{"accessToken":"[REDACTED]", "workspacePath":"[REDACTED]"},
                {"private":true,"text":"[REDACTED]","result":"failed"},
                {"privacy":"private","preview":"[REDACTED]"}],
            "message":"request Bearer [REDACTED], password:[REDACTED]) done"
        })
    );
    assert_eq!(
        redact_sensitive_text("token=abc next"),
        "token=[REDACTED] next"
    );
}

#[test]
fn sanitize_then_validate_matches_ingest_order_without_mutating_identity() {
    let mut input = event();
    input["data"] = json!({"credential":"x".repeat(20_000),"component":"send"});
    let mut event: TelemetryEvent = serde_json::from_value(input).unwrap();
    assert!(validate_batch(std::slice::from_ref(&event)).is_err());
    event.data = event.data.map(sanitize_value);
    validate_batch(std::slice::from_ref(&event)).unwrap();
    assert_eq!(event.event_id, "owned-1");
    assert_eq!(
        event.data.unwrap(),
        json!({"credential":"[REDACTED]","component":"send"})
    );
}
