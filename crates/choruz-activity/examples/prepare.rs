use choruz_activity::{TelemetryEvent, sanitize_value, validate_batch};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut events: Vec<TelemetryEvent> = serde_json::from_str(
        r#"[{"eventId":"example-1","schemaVersion":1,"traceId":"trace-1",
        "spanId":"span-1","sessionId":"session-1","name":"button.click",
        "ts":"2026-09-28T00:00:00Z","data":{"component":"send","token":"example-secret"}}]"#,
    )?;
    for event in &mut events {
        event.data = event.data.take().map(sanitize_value);
    }
    validate_batch(&events)?;
    println!("{}", serde_json::to_string(&events)?);
    Ok(())
}
