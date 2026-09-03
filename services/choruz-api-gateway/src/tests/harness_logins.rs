use super::*;

/// A stand-in for `claude` speaking the stream-json control protocol the
/// login driver uses. It hands out one authorization link, records the code
/// it is given, and reports a signed-in account.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, os, sys
authenticated = False
for line in sys.stdin:
    message = json.loads(line)
    request_id = message["request_id"]
    request = message["request"]
    subtype = request["subtype"]
    response = {}
    outcome = "success"
    if subtype == "initialize":
        response = {
            "account": {"email": "dev@example.test", "subscriptionType": "max"},
            "models": [],
        }
        if authenticated and not os.environ.get("FAKE_CLAUDE_NO_SNAPSHOT"):
            response["models"] = [{"value": "claude-sonnet-4-5", "displayName": "Sonnet 4.5"}]
    elif subtype == "claude_authenticate":
        response = {"manualUrl": "https://claude.ai/oauth/authorize?state=state-1"}
    elif subtype == "claude_oauth_callback":
        with open(os.path.join(os.environ["FAKE_CLAUDE_DIR"], "callback.json"), "w") as sink:
            json.dump(request, sink)
        if request.get("authorizationCode") != "code-1":
            outcome = "error"
        else:
            authenticated = True
    elif subtype == "claude_oauth_wait_for_completion":
        outcome = "error"
        response = "No active claude_authenticate flow"
    elif subtype == "get_usage":
        response = {"subscription_type": "max", "rate_limits": {}}
        if not os.environ.get("FAKE_CLAUDE_NO_SNAPSHOT"):
            response["rate_limits"] = {"five_hour": {"utilization": 12.5, "resets_at": "2026-09-03T12:00:00Z"}}
    print(json.dumps({"type": "control_response", "response": {"request_id": request_id, "subtype": outcome, "response": response}}), flush=True)
"#;

/// A stand-in for the Codex app-server browser protocol. It emits the official
/// completion notification, then serves the account snapshot.
const FAKE_CODEX: &str = r#"#!/usr/bin/env python3
import json, os, sys, time
with open(os.path.join(os.environ["FAKE_CODEX_DIR"], "profile.txt"), "w") as sink:
    sink.write(os.environ.get("CODEX_HOME", ""))
for line in sys.stdin:
    message = json.loads(line)
    request_id = message.get("id")
    method = message.get("method")
    if request_id is None:
        continue
    if method == "initialize":
        result = {"userAgent": "fake-codex"}
    elif method == "account/login/start":
        if message["params"].get("type") != "chatgpt":
            print(json.dumps({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32602, "message": "expected browser login"}}), flush=True)
            continue
        result = {
            "type": "chatgpt",
            "loginId": "browser-login-1",
            "authUrl": "https://auth.openai.com/oauth?state=state-1&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback",
        }
    elif method == "account/read":
        result = {"account": {"type": "chatgpt", "email": "codex@example.test", "planType": "team"}}
    elif method == "account/rateLimits/read":
        result = {"rateLimits": {"primary": {"usedPercent": 21.0, "windowDurationMins": 10080, "resetsAt": 1800000000}}}
    elif method == "model/list":
        result = {"data": [{"id": "gpt-test", "displayName": "GPT Test"}]}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "unknown method"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)
    if method == "account/login/start":
        while not os.path.exists(os.path.join(os.environ["FAKE_CODEX_DIR"], "authorized")):
            time.sleep(0.01)
        print(json.dumps({"jsonrpc": "2.0", "method": "account/login/completed", "params": {"loginId": "browser-login-1", "success": True, "error": None}}), flush=True)
"#;

struct LoginFixture {
    database: TestDatabase,
    router: Router,
    operator: choruz_domain::Principal,
    client: tokio_postgres::Client,
}

impl LoginFixture {
    async fn create() -> Self {
        let database = TestDatabase::create().await;
        let app = choruz_application::ChatApp::new();
        let operator = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: "harness-login-company".into(),
                principal_type: PrincipalType::Human,
                name: "Harness Login Operator".into(),
                avatar_url: None,
            })
            .unwrap();
        seed_principal_to_db(&database.database_url, &operator).await;
        let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
            .await
            .unwrap();
        tokio::spawn(async move { connection.await.unwrap() });
        client
            .execute(
                "INSERT INTO company (id, name, slug, owner_id) VALUES ($1, 'Harness Logins', $1, $2)",
                &[&operator.workspace_id, &operator.id],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO company_member (company_id, principal_id) VALUES ($1, $2)",
                &[&operator.workspace_id, &operator.id],
            )
            .await
            .unwrap();
        let router = router_with_db(app, &database.database_url);
        Self {
            database,
            router,
            operator,
            client,
        }
    }

    async fn account(&self, runtime_host_id: Option<&str>) -> String {
        self.account_for_driver(runtime_host_id, "claude_terminal")
            .await
    }

    async fn account_for_driver(&self, runtime_host_id: Option<&str>, driver_type: &str) -> String {
        let account_id = Uuid::now_v7().to_string();
        self.client
            .execute(
                "INSERT INTO harness_account
                    (id, company_id, runtime_host_id, driver_type, name, profile_kind)
                 VALUES ($1, $2, $3, $4, $1, 'isolated')",
                &[
                    &account_id,
                    &self.operator.workspace_id,
                    &runtime_host_id,
                    &driver_type,
                ],
            )
            .await
            .unwrap();
        account_id
    }

    async fn runtime_host(&self) -> String {
        let host_id = Uuid::now_v7().to_string();
        self.client
            .execute(
                "INSERT INTO runtime_host (id, company_id, name, token_hash)
                 VALUES ($1, $2, 'Build Server', $1)",
                &[&host_id, &self.operator.workspace_id],
            )
            .await
            .unwrap();
        host_id
    }

    fn logins_uri(&self, account_id: &str) -> String {
        format!(
            "/v1/companies/{}/harness-accounts/{account_id}/logins",
            self.operator.workspace_id
        )
    }

    async fn start(&self, account_id: &str) -> (StatusCode, Value) {
        api_json_request(
            self.router.clone(),
            &self.operator,
            Method::POST,
            self.logins_uri(account_id),
        )
        .await
    }

    async fn login(&self, account_id: &str, login_id: &str) -> Value {
        let (status, login) = api_json_request(
            self.router.clone(),
            &self.operator,
            Method::GET,
            format!("{}/{login_id}", self.logins_uri(account_id)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        login
    }

    /// Poll the login until the in-process runner moves it to `state`.
    async fn wait_for_state(&self, account_id: &str, login_id: &str, state: &str) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let login = self.login(account_id, login_id).await;
            if login["state"] == state {
                return login;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "login never reached {state}: {login}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_for_account_model(
        &self,
        account_id: &str,
        model_id: &str,
    ) -> tokio_postgres::Row {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let row = self
                .client
                .query_one(
                    "SELECT status, subscription_type, models_json, usage_json
                       FROM harness_account WHERE id = $1",
                    &[&account_id],
                )
                .await
                .unwrap();
            if row.get::<_, Value>("models_json")[0]["id"] == model_id {
                return row;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "account snapshot never included model {model_id}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn cancel(&self, account_id: &str, login_id: &str) -> StatusCode {
        api_json_request(
            self.router.clone(),
            &self.operator,
            Method::POST,
            format!("{}/{login_id}/cancel", self.logins_uri(account_id)),
        )
        .await
        .0
    }

    async fn submit_callback(&self, account_id: &str, login_id: &str, code: &str) -> StatusCode {
        api_json_payload_request(
            self.router.clone(),
            &self.operator,
            Method::POST,
            format!("{}/{login_id}/callback", self.logins_uri(account_id)),
            json!({ "code": code }),
        )
        .await
        .0
    }
}

#[tokio::test]
async fn local_codex_login_uses_browser_completion_and_verifies_the_account() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("codex-login");
    let binary = dir.join("codex");
    write_executable_script(&binary, FAKE_CODEX);
    let _binary = EnvVarGuard::set_path("CHORUZ_CODEX_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CODEX_DIR", &dir);
    let profiles = dir.join("accounts");
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &profiles);
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account_for_driver(None, "codex_terminal").await;

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap().to_owned();
    let waiting = fixture
        .wait_for_state(&account_id, &login_id, "awaiting_browser")
        .await;
    assert_eq!(
        waiting["authorization_url"],
        "https://auth.openai.com/oauth?state=state-1&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"
    );
    assert_eq!(waiting["user_code"], Value::Null);
    fs::write(dir.join("authorized"), "authorized").unwrap();

    let verified = fixture
        .wait_for_state(&account_id, &login_id, "verified")
        .await;
    assert_eq!(verified["authorization_url"], Value::Null);
    let row = fixture
        .wait_for_account_model(&account_id, "gpt-test")
        .await;
    assert_eq!(row.get::<_, String>("status"), "active");
    assert_eq!(
        row.get::<_, Option<String>>("subscription_type").as_deref(),
        Some("team")
    );
    assert_eq!(row.get::<_, Value>("models_json")[0]["id"], "gpt-test");
    assert_eq!(
        row.get::<_, Value>("usage_json")["windows"][0]["remainingPercent"],
        79.0
    );
    let profile = fs::read_to_string(dir.join("profile.txt")).unwrap();
    assert!(profile.starts_with(profiles.to_string_lossy().as_ref()));
    assert!(profile.contains(&account_id));
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn local_claude_login_accepts_the_complete_callback_and_verifies_the_account() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-login");
    let binary = dir.join("claude");
    write_executable_script(&binary, FAKE_CLAUDE);
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CLAUDE_DIR", &dir);
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;
    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    assert_eq!(login["state"], "authorizing");
    assert_eq!(login["runtime_host_id"], Value::Null);
    assert_eq!(login["driver_type"], "claude_terminal");
    let login_id = login["id"].as_str().unwrap().to_owned();

    let (duplicate, _) = fixture.start(&account_id).await;
    assert_eq!(duplicate, StatusCode::CONFLICT);

    let waiting = fixture
        .wait_for_state(&account_id, &login_id, "awaiting_browser")
        .await;
    assert_eq!(
        waiting["authorization_url"],
        "https://claude.ai/oauth/authorize?state=state-1"
    );
    assert_eq!(waiting["user_code"], Value::Null);
    assert!(
        dir.join("accounts").exists(),
        "the isolated profile directory is created before the Harness starts"
    );

    let wrong_state = fixture
        .submit_callback(
            &account_id,
            &login_id,
            "https://localhost/callback?code=code-1&state=other",
        )
        .await;
    assert_eq!(wrong_state, StatusCode::NO_CONTENT);
    let failed = fixture
        .wait_for_state(&account_id, &login_id, "failed")
        .await;
    assert_eq!(
        failed["error"],
        "The callback does not belong to this Claude login"
    );
    assert!(!dir.join("callback.json").exists());

    let (status, retry) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{retry}");
    let retry_id = retry["id"].as_str().unwrap().to_owned();
    fixture
        .wait_for_state(&account_id, &retry_id, "awaiting_browser")
        .await;
    let submitted = fixture
        .submit_callback(&account_id, &retry_id, "code-1#state-1")
        .await;
    assert_eq!(submitted, StatusCode::NO_CONTENT);
    let again = fixture
        .submit_callback(&account_id, &retry_id, "code-1")
        .await;
    assert_eq!(again, StatusCode::CONFLICT);

    let verified = fixture
        .wait_for_state(&account_id, &retry_id, "verified")
        .await;
    assert_eq!(verified["authorization_url"], Value::Null);
    assert_eq!(verified["error"], Value::Null);
    let callback: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("callback.json")).unwrap()).unwrap();
    assert_eq!(callback["authorizationCode"], "code-1");
    assert_eq!(callback["state"], "state-1");

    let row = fixture
        .client
        .query_one(
            "SELECT status, account_fingerprint, subscription_type, models_json, usage_json,
                    probed_at IS NOT NULL AS probed
               FROM harness_account WHERE id = $1",
            &[&account_id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("status"), "active");
    assert_eq!(row.get::<_, String>("account_fingerprint").len(), 64);
    assert_eq!(
        row.get::<_, Option<String>>("subscription_type").as_deref(),
        Some("max")
    );
    assert_eq!(
        row.get::<_, Value>("models_json"),
        json!([{ "id": "claude-sonnet-4-5", "label": "Sonnet 4.5" }])
    );
    assert_eq!(
        row.get::<_, Value>("usage_json")["windows"][0]["usedPercent"],
        12.5
    );
    assert!(row.get::<_, bool>("probed"));
    let secrets: i64 = fixture
        .client
        .query_one(
            "SELECT COUNT(*) FROM harness_account_login WHERE callback_code IS NOT NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(secrets, 0, "the pasted code is consumed, never retained");
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn local_claude_login_stays_active_when_catalog_refresh_fails() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-login-no-snapshot");
    let binary = dir.join("claude");
    write_executable_script(&binary, FAKE_CLAUDE);
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CLAUDE_DIR", &dir);
    let _no_snapshot = EnvVarGuard::set_path("FAKE_CLAUDE_NO_SNAPSHOT", std::path::Path::new("1"));
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;
    fixture
        .client
        .execute(
            "UPDATE harness_account
                SET status = 'error', last_error = 'stale failure',
                    models_json = '[{\"id\":\"last-good\",\"label\":\"Last good\"}]'::jsonb,
                    usage_json = '{\"windows\":[{\"id\":\"weekly\",\"usedPercent\":20,\"remainingPercent\":80}]}'::jsonb,
                    probed_at = '2026-09-01T00:00:00Z'
              WHERE id = $1",
            &[&account_id],
        )
        .await
        .unwrap();

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap().to_owned();
    fixture
        .wait_for_state(&account_id, &login_id, "awaiting_browser")
        .await;
    assert_eq!(
        fixture
            .submit_callback(&account_id, &login_id, "code-1#state-1")
            .await,
        StatusCode::NO_CONTENT
    );
    fixture
        .wait_for_state(&account_id, &login_id, "verified")
        .await;

    let row = fixture
        .client
        .query_one(
            "SELECT status, models_json, usage_json, probed_at, last_error
               FROM harness_account WHERE id = $1",
            &[&account_id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("status"), "active");
    assert_eq!(row.get::<_, Value>("models_json")[0]["id"], "last-good");
    assert_eq!(
        row.get::<_, Value>("usage_json")["windows"][0]["remainingPercent"],
        80
    );
    assert_eq!(
        row.get::<_, Option<chrono::DateTime<chrono::Utc>>>("probed_at")
            .unwrap()
            .to_rfc3339(),
        "2026-09-01T00:00:00+00:00"
    );
    assert!(row.get::<_, Option<String>>("last_error").is_none());
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn local_claude_login_marks_the_account_failed_when_the_binary_is_missing() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-login-missing");
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &dir.join("no-such-claude"));
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap().to_owned();
    let failed = fixture
        .wait_for_state(&account_id, &login_id, "failed")
        .await;
    let error = failed["error"].as_str().unwrap();
    assert!(
        error.starts_with("start Claude Code control session:"),
        "{error}"
    );
    let callback = fixture
        .submit_callback(&account_id, &login_id, "code-1")
        .await;
    assert_eq!(callback, StatusCode::CONFLICT);
    let account = fixture
        .client
        .query_one(
            "SELECT status, last_error FROM harness_account WHERE id = $1",
            &[&account_id],
        )
        .await
        .unwrap();
    let status: String = account.get(0);
    let last_error: Option<String> = account.get(1);
    assert_eq!(status, "error");
    assert_eq!(last_error.as_deref(), Some(error));
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn incompatible_codex_cli_reports_how_to_select_a_current_binary() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("codex-login-incompatible");
    let binary = dir.join("codex");
    write_executable_script(&binary, "#!/bin/sh\nexit 2\n");
    let _binary = EnvVarGuard::set_path("CHORUZ_CODEX_BINARY", &binary);
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account_for_driver(None, "codex_terminal").await;

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap();
    let failed = fixture
        .wait_for_state(&account_id, login_id, "failed")
        .await;
    assert_eq!(
        failed["error"],
        "Codex app-server is unavailable; update Codex or set CHORUZ_CODEX_BINARY to a current Codex CLI"
    );
    let account = fixture
        .client
        .query_one(
            "SELECT status, last_error FROM harness_account WHERE id = $1",
            &[&account_id],
        )
        .await
        .unwrap();
    assert_eq!(account.get::<_, String>("status"), "error");
    assert_eq!(
        account.get::<_, Option<String>>("last_error").as_deref(),
        failed["error"].as_str()
    );
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn cancelling_an_open_login_lets_a_new_one_start_at_once() {
    let fixture = LoginFixture::create().await;
    let host_id = fixture.runtime_host().await;
    let account_id = fixture.account(Some(&host_id)).await;

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap().to_owned();
    let (duplicate, _) = fixture.start(&account_id).await;
    assert_eq!(duplicate, StatusCode::CONFLICT);

    assert_eq!(
        fixture.cancel(&account_id, &login_id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        fixture.login(&account_id, &login_id).await["state"],
        "cancelled"
    );
    assert_eq!(
        fixture.cancel(&account_id, &login_id).await,
        StatusCode::CONFLICT
    );
    let (status, replacement) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{replacement}");
    assert_ne!(replacement["id"], login_id);
    drop(fixture.database);
}

#[tokio::test]
async fn cancelling_a_local_claude_login_stops_the_driver_and_keeps_the_account_pending() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-login-cancel");
    let binary = dir.join("claude");
    write_executable_script(&binary, FAKE_CLAUDE);
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CLAUDE_DIR", &dir);
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap().to_owned();
    fixture
        .wait_for_state(&account_id, &login_id, "awaiting_browser")
        .await;

    assert_eq!(
        fixture.cancel(&account_id, &login_id).await,
        StatusCode::NO_CONTENT
    );
    // The driver notices at its next callback poll and ends without
    // touching the cancelled row or the account.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let cancelled = fixture.login(&account_id, &login_id).await;
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["error"], Value::Null);
    let account_status: String = fixture
        .client
        .query_one(
            "SELECT status FROM harness_account WHERE id = $1",
            &[&account_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(account_status, "pending");
    let (status, replacement) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{replacement}");
    drop(fixture.database);
}

#[tokio::test]
async fn remote_harness_login_waits_for_the_connector_and_expires() {
    let fixture = LoginFixture::create().await;
    let host_id = fixture.runtime_host().await;
    let account_id = fixture.account(Some(&host_id)).await;
    let other_company_account = Uuid::now_v7().to_string();

    let (missing, _) = fixture.start(&other_company_account).await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    assert_eq!(login["state"], "queued");
    assert_eq!(login["runtime_host_id"], host_id);
    let login_id = login["id"].as_str().unwrap().to_owned();
    assert_eq!(
        fixture.login(&account_id, &login_id).await["state"],
        "queued"
    );
    let early = fixture
        .submit_callback(&account_id, &login_id, "code-1")
        .await;
    assert_eq!(early, StatusCode::CONFLICT);

    fixture
        .client
        .execute(
            "UPDATE harness_account_login SET expires_at = NOW() - INTERVAL '1 minute' WHERE id = $1",
            &[&login_id],
        )
        .await
        .unwrap();
    assert_eq!(
        fixture.login(&account_id, &login_id).await["state"],
        "expired"
    );
    let (status, replacement) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{replacement}");
    assert_ne!(replacement["id"], login_id);
    let stale: String = fixture
        .client
        .query_one(
            "SELECT state FROM harness_account_login WHERE id = $1",
            &[&login_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stale, "expired");
    drop(fixture.database);
}
