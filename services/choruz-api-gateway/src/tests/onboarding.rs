use super::*;
use std::{os::unix::fs::PermissionsExt, time::Instant};

#[tokio::test]
async fn onboarding_and_supervision_share_the_instance_directory() {
    const CHILD_ROOT: &str = "CHORUZ_TEST_CONNECTOR_ROOT";
    let Some(root) = env::var_os(CHILD_ROOT) else {
        // Child-only overrides avoid mutating environment read by parallel tests.
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("connector");
        fs::write(
            &binary,
            r#"#!/bin/sh
set -eu
test "$2" = --config
test "$(dirname "$3")" = "$CHORUZ_TEST_CONNECTOR_ROOT" || exit 42
case "$1" in
  pair-relay)
    cat >/dev/null
    printf '{"host_id":"test-host"}' > "$3"
    printf '%s' "$3" > "$CHORUZ_TEST_CONNECTOR_ROOT/paired"
    printf '{"host_id":"test-host","host_name":"Test device"}'
    ;;
  run)
    test -f "$3"
    printf '%s' "$3" > "$CHORUZ_TEST_CONNECTOR_ROOT/running"
    trap 'exit 0' TERM INT
    while :; do sleep 0.1; done
    ;;
  *) exit 43 ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::onboarding::onboarding_and_supervision_share_the_instance_directory",
                "--nocapture",
            ])
            .env(CHILD_ROOT, root.path())
            .env("CHORUZ_CONNECTOR_CONFIG_DIR", root.path())
            .env("CHORUZ_CONNECTOR_BINARY", binary)
            .env("CHORUZ_PLUGINS", "remote-control")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let root = PathBuf::from(root);
    assert_eq!(
        choruz_supervisor::connectors::connector_config_directory().unwrap(),
        root
    );
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let human = db
        .create_human_user("onboarding-test", "test-password-123")
        .await
        .unwrap();
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/runtime-host-onboarding")
                .header(CONTENT_TYPE, "application/json")
                .header("authorization", format!("Bearer {}", session_token(&human)))
                .body(Body::from(
                    json!({
                        "controller_gateway_url": "https://gateway.example",
                        "controller_credential": "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB",
                        "runtime_host_code": "12345678",
                        "name": "Test device",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["host_id"], "test-host");
    let paired = fs::read_to_string(root.join("paired")).unwrap();
    assert_eq!(Path::new(&paired).parent(), Some(root.as_path()));
    assert_eq!(
        fs::read_to_string(&paired).unwrap(),
        "{\"host_id\":\"test-host\"}"
    );
    let supervisor = choruz_supervisor::connectors::ConnectorSupervisor::start_default().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("running").exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(supervisor);
    assert_eq!(fs::read_to_string(root.join("running")).unwrap(), paired);
}
