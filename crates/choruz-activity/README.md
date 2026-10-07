# Choruz activity

Prepare versioned activity events without a server, database or agent runtime.
The platform uses the same `TelemetryEvent`, `validate_batch`, `sanitize_value`
and `redact_sensitive_text` implementations.

```sh
cargo run -p choruz-activity --example prepare
```

The example deserializes an event, redacts its data, validates the batch and
prints JSON without persisting or uploading anything. Use a path dependency
on this crate and `choruz-common`; registry publication is not implied.

Sanitize data before validating it, matching HTTP ingest. Validation rejects
unknown wire fields, unsupported schema versions, invalid identifiers, negative
durations, non-object data, payloads larger than 16 KiB and batches outside
1–100 events. Deserialization rejects unknown fields; `validate_batch` checks
the typed values. It does not deduplicate events or authorize their actor.

Redaction screens known sensitive keys, explicit private content and recognized
secret markers. It does not guarantee removal of arbitrary personal information
or authorize community publication. Native traces are separate, private source
evidence. Browser-side filtering still runs before local IndexedDB storage.

The caller owns collection consent, authentication, transport, persistence,
acknowledgement, retries and retention. Choruz keeps these in its existing
browser outbox and authenticated activity API; using this library alone does
not start a collector or provide a second activity store.
