# choruz-auth

Token and secret primitives for the API gateway: HMAC-SHA256 session tokens (`issue_session_token`, `verify_session_token`, `SessionClaims`, the `choruz_session` cookie name), SHA-256 hashed agent secrets (`issue_secret`, `hash_secret`, `verify_secret`) and the deterministic `local_user_principal_id`. `crates/choruz-application` and `services/choruz-api-gateway` depend on it.

## Entry points

- `src/lib.rs` — the whole crate; there are no submodules

## Tests

`cargo test -p choruz-auth`; no PostgreSQL.

## Related

- [docs/subsystems/api-gateway.md](../../docs/subsystems/api-gateway.md) — where the tokens are issued and checked
- [docs/architecture.md](../../docs/architecture.md)
