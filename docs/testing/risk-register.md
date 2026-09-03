# Risk Register

| ID | Risk | Impact | Mitigation |
| --- | --- | --- | --- |
| R-01 | No local host services installed | Cannot execute PG/Redis/NATS/ZITADEL/MinIO integration tests locally | Provide `infra/host` bootstrap scripts and keep pure Rust tests as the default fallback |
| R-02 | No PostgreSQL service on host | SQL migration rehearsal cannot be executed locally today | Commit production-targeted migrations and keep app logic testable via memory adapter |
| R-03 | Empty starting repository | Large implementation surface | Work phase-by-phase with commit gates and contract-first artifacts |
| R-04 | No git remote configured | Passing features cannot be pushed automatically | Commit locally and push once a remote is provided |
