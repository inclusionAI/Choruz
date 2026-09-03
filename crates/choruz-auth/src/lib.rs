use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
pub const SESSION_COOKIE_NAME: &str = "choruz_session";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionClaims {
    pub principal_id: String,
    pub workspace_id: String,
    pub display_name: String,
    pub expires_at_epoch_s: i64,
}

pub fn issue_secret() -> String {
    format!("agt_{}", Uuid::now_v7())
}

pub fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn verify_secret(secret: &str, expected_hash: &str) -> bool {
    let actual = hash_secret(secret);
    actual.as_bytes().ct_eq(expected_hash.as_bytes()).into()
}

pub fn local_user_principal_id(workspace_id: &str, display_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_id.trim().as_bytes());
    hasher.update(b":");
    hasher.update(display_name.trim().to_lowercase().as_bytes());
    format!("local-user-{:x}", hasher.finalize())
}

pub fn issue_session_token(
    claims: &SessionClaims,
    signing_secret: &str,
) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_vec(claims)?;
    let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
    let signature = sign_bytes(encoded_payload.as_bytes(), signing_secret);
    Ok(format!("{encoded_payload}.{signature}"))
}

pub fn verify_session_token(
    token: &str,
    signing_secret: &str,
    now_epoch_s: i64,
) -> Option<SessionClaims> {
    let (encoded_payload, encoded_signature) = token.split_once('.')?;
    let expected_signature = sign_bytes(encoded_payload.as_bytes(), signing_secret);
    let sig_match: bool = expected_signature
        .as_bytes()
        .ct_eq(encoded_signature.as_bytes())
        .into();
    if !sig_match {
        return None;
    }

    let payload = URL_SAFE_NO_PAD.decode(encoded_payload.as_bytes()).ok()?;
    let claims: SessionClaims = serde_json::from_slice(&payload).ok()?;
    (claims.expires_at_epoch_s > now_epoch_s).then_some(claims)
}

fn sign_bytes(payload: &[u8], signing_secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .expect("HMAC accepts arbitrary key size");
    mac.update(payload);
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::{
        SESSION_COOKIE_NAME, SessionClaims, hash_secret, issue_secret, issue_session_token,
        local_user_principal_id, verify_secret, verify_session_token,
    };

    #[test]
    fn session_cookie_uses_only_the_choruz_identity() {
        assert_eq!(SESSION_COOKIE_NAME, "choruz_session");
        assert_ne!(SESSION_COOKIE_NAME, "echat_session");
    }

    #[test]
    fn hashes_are_deterministic() {
        let secret = issue_secret();
        let hash = hash_secret(&secret);
        assert!(verify_secret(&secret, &hash));
    }

    #[test]
    fn local_user_ids_are_stable() {
        assert_eq!(
            local_user_principal_id("ws-acme", "Operator"),
            local_user_principal_id("ws-acme", "operator"),
        );
    }

    #[test]
    fn session_tokens_round_trip_and_expire() {
        let token = issue_session_token(
            &SessionClaims {
                principal_id: "principal-1".into(),
                workspace_id: "ws-acme".into(),
                display_name: "Operator".into(),
                expires_at_epoch_s: 200,
            },
            "secret-key",
        )
        .unwrap();

        let claims = verify_session_token(&token, "secret-key", 100).unwrap();
        assert_eq!(claims.principal_id, "principal-1");
        assert!(verify_session_token(&token, "wrong-secret", 100).is_none());
        assert!(verify_session_token(&token, "secret-key", 300).is_none());
    }
}
