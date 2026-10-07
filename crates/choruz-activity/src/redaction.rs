/// Recursively redact known sensitive keys and secret markers. Explicit private
/// objects omit content fields. This is defensive filtering, not a publication
/// privacy review or a guarantee that arbitrary text contains no personal data.
pub fn sanitize_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut object) => {
            let private_payload = object
                .get("private")
                .or_else(|| object.get("is_private"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
                || object.get("privacy").and_then(|value| value.as_str()) == Some("private");

            for (key, value) in object.iter_mut() {
                if telemetry_key_is_sensitive(key)
                    || (private_payload && telemetry_key_is_private_content(key))
                {
                    *value = serde_json::Value::String("[REDACTED]".into());
                } else {
                    *value = sanitize_value(value.take());
                }
            }

            serde_json::Value::Object(object)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sanitize_value).collect())
        }
        serde_json::Value::String(value) => {
            serde_json::Value::String(redact_sensitive_text(&value))
        }
        other => other,
    }
}

fn telemetry_key_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let compact_key: String = key.chars().filter(|ch| *ch != '_' && *ch != '-').collect();
    key == "authorization"
        || key == "cookie"
        || key == "set-cookie"
        || matches!(
            compact_key.as_str(),
            "authorization"
                | "cookie"
                | "setcookie"
                | "database64"
                | "attachmentbytes"
                | "filebytes"
                | "contentbytes"
                | "bodybytes"
                | "rawbytes"
                | "bytesbase64"
                | "payloadbase64"
                | "filename"
                | "attachmentname"
                | "path"
                | "paths"
                | "authenticationcode"
                | "authorizationcode"
                | "devicecode"
                | "pairingcredential"
                | "credential"
        )
        || compact_key.ends_with("filename")
        || key.contains("secret")
        || compact_key.contains("secret")
        || key.contains("password")
        || compact_key.contains("password")
        || key.ends_with("_path")
        || key.ends_with("_paths")
        || compact_key.ends_with("path")
        || compact_key.ends_with("paths")
        || key.contains("session_token")
        || compact_key.contains("sessiontoken")
        || key.ends_with("_token")
        || key.ends_with("token")
        || compact_key.ends_with("token")
}

fn telemetry_key_is_private_content(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "content" | "message" | "text" | "body" | "preview"
    )
}

/// Redact values after recognized bearer, secret, token and password markers.
/// Unrecognized formats and arbitrary personal information remain unchanged.
pub fn redact_sensitive_text(input: &str) -> String {
    let mut redacted = input.to_owned();
    for marker in [
        "Bearer ",
        "bearer ",
        "secret=",
        "secret:",
        "token=",
        "token:",
        "password=",
        "password:",
    ] {
        redacted = redact_after_marker(&redacted, marker);
    }
    redacted
}

fn redact_after_marker(input: &str, marker: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(offset) = input[cursor..].find(marker) {
        let start = cursor + offset;
        let value_start = start + marker.len();
        output.push_str(&input[cursor..value_start]);
        let rest = &input[value_start..];
        let skip_ws = rest.len() - rest.trim_start().len();
        let rest = &input[value_start + skip_ws..];
        let end = rest
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ')' | '}' | ']'))
            .map(|index| value_start + skip_ws + index)
            .unwrap_or(input.len());
        output.push_str("[REDACTED]");
        cursor = end;
    }
    output.push_str(&input[cursor..]);
    output
}
