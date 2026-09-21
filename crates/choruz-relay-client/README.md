# Choruz relay client

Carry HTTP requests and stream frames over Choruz's encrypted Remote Control transport. This crate has no platform crate dependencies or database. The browser implements the same wire contract separately; gateway deployment and device enrollment are outside this library.

```sh
cargo test -p choruz-relay-client
```

Callers supply paired credentials and remain responsible for authorization, secure credential storage and the lifetime of their connection. A timeout is not proof that a remote mutation did not execute; reconcile the operation before retrying. Library use does not start a host daemon or enroll a runtime device.
