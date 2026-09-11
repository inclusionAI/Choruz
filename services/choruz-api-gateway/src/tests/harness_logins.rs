use super::*;

/// Interactive sign-in and read-only account probing share an isolated profile.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, os, sys
with open(os.path.join(os.environ["FAKE_CLAUDE_DIR"], "pid"), "w") as sink:
    sink.write(str(os.getpid()))
profile = os.environ.get("CLAUDE_CONFIG_DIR", os.environ["FAKE_CLAUDE_DIR"])
marker = os.path.join(profile, "signed-in")
if len(sys.argv) == 1:
    with open(os.path.join(os.environ["FAKE_CLAUDE_DIR"], "cwd"), "w") as sink: sink.write(os.getcwd())
    assert "CHORUZ_SEND" not in os.environ
    assert sys.stdin.isatty()
    print("Official CLI sign-in fixture", flush=True)
    if input().strip() == "private-code#state":
        with open(marker, "w") as sink: sink.write("yes")
        print("Signed in", flush=True)
        input()
    sys.exit(0)
authenticated = os.path.exists(marker) or os.environ.get("FAKE_CLAUDE_SIGNED_IN")
for line in sys.stdin:
    message = json.loads(line)
    request_id = message["request_id"]
    request = message["request"]
    subtype = request["subtype"]
    response = {}
    outcome = "success"
    if subtype == "initialize":
        response = {
            "account": {"email": "dev@example.test", "subscriptionType": "max"} if authenticated else None,
            "models": [],
        }
        if (authenticated or os.environ.get("FAKE_CLAUDE_SIGNED_IN")) and not os.environ.get("FAKE_CLAUDE_NO_SNAPSHOT"):
            response["models"] = [{"value": "claude-sonnet-4-5", "displayName": "Sonnet 4.5"}]
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
            "authUrl": open(os.path.join(os.environ["FAKE_CODEX_DIR"], "auth-url")).read(),
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

    async fn complete(&self, account_id: &str, login_id: &str) -> (StatusCode, Value) {
        api_json_request(
            self.router.clone(),
            &self.operator,
            Method::POST,
            format!("{}/{login_id}/complete", self.logins_uri(account_id)),
        )
        .await
    }
}

#[tokio::test]
async fn local_codex_login_uses_browser_completion_and_verifies_the_account() {
    codex_gateway_login(false).await;
}

#[tokio::test]
async fn gateway_codex_login_accepts_a_callback_from_another_browser_device() {
    codex_gateway_login(true).await;
}

async fn codex_gateway_login(paste_callback: bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("codex-login");
    let binary = dir.join("codex");
    write_executable_script(&binary, FAKE_CODEX);
    let _binary = EnvVarGuard::set_path("CHORUZ_CODEX_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CODEX_DIR", &dir);
    let profiles = dir.join("accounts");
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &profiles);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let authorization = format!(
        "https://auth.openai.com/oauth?state=state-1&redirect_uri=http%3A%2F%2F127.0.0.1%3A{}%2Fauth%2Fcallback",
        listener.local_addr().unwrap().port()
    );
    fs::write(dir.join("auth-url"), authorization.as_str()).unwrap();
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account_for_driver(None, "codex_terminal").await;

    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED, "{login}");
    let login_id = login["id"].as_str().unwrap().to_owned();
    let waiting = fixture
        .wait_for_state(&account_id, &login_id, "awaiting_browser")
        .await;
    assert_eq!(waiting["authorization_url"], authorization.as_str());
    assert_eq!(waiting["runtime_host_id"], Value::Null);
    assert_eq!(waiting["user_code"], Value::Null);
    if paste_callback {
        assert_eq!(
            fixture
                .submit_callback(
                    &account_id,
                    &login_id,
                    "http://localhost:9999/auth/callback?code=code-1&state=state-1"
                )
                .await,
            StatusCode::NO_CONTENT
        );
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("gateway login must consume the pasted callback")
            .unwrap();
        let mut request = vec![0; 2048];
        let size = stream.read(&mut request).await.unwrap();
        assert!(
            String::from_utf8_lossy(&request[..size])
                .starts_with("GET /auth/callback?code=code-1&state=state-1 ")
        );
        stream
            .write_all(b"HTTP/1.1 302 Found\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
    }
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
async fn claude_terminal_input_signs_in_the_selected_profile_without_relay_storage() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("claude-pty-sign-in");
    let binary = dir.join("claude");
    write_executable_script(&binary, FAKE_CLAUDE);
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &binary);
    let _fake = EnvVarGuard::set_path("FAKE_CLAUDE_DIR", &dir);
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account = fixture.account(None).await;
    let (_, login) = fixture.start(&account).await;
    let id = login["id"].as_str().unwrap();
    assert_eq!(
        fixture.complete(&account, id).await.0,
        StatusCode::BAD_REQUEST
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = fixture.router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let url = format!(
        "ws://{addr}/v1/ws/harness-logins/{}/{account}/{id}?token={}",
        fixture.operator.workspace_id,
        session_token(&fixture.operator)
    );
    let (mut socket, _) = connect_async(&url).await.unwrap();
    let mut output = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !output.contains("Official CLI sign-in fixture") {
            let frame = socket.next().await.unwrap().unwrap();
            output.push_str(&String::from_utf8_lossy(&frame.into_data()));
        }
    })
    .await
    .unwrap();
    let (mut second, _) = connect_async(&url).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), second.next())
        .await
        .unwrap();
    assert!(matches!(
        closed,
        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None
    ));
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "private-code#state\r".into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !output.contains("Signed in") {
            let frame = socket.next().await.unwrap().unwrap();
            output.push_str(&String::from_utf8_lossy(&frame.into_data()));
        }
    })
    .await
    .unwrap();
    let signed_in_pid = fs::read_to_string(dir.join("pid")).unwrap();
    let login_directory = fs::read_to_string(dir.join("cwd")).unwrap();
    assert_eq!(
        fixture.complete(&account, id).await.0,
        StatusCode::NO_CONTENT
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::process::Command::new("kill")
            .args(["-0", signed_in_pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Done must stop the still-interactive official CLI");
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::path::Path::new(&login_directory).exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Done must release the login session and its private directory");
    assert_eq!(fixture.login(&account, id).await["state"], "verified");
    assert_eq!(
        fixture
            .account_row(&account)
            .await
            .get::<_, String>("status"),
        "active"
    );
    let secrets: i64 = fixture.client.query_one("SELECT COUNT(*) FROM harness_account_login WHERE authorization_url IS NOT NULL OR user_code IS NOT NULL OR callback_code IS NOT NULL", &[]).await.unwrap().get(0);
    assert_eq!(secrets, 0);
    let _ = socket.close(None).await;
    for expire in [false, true] {
        let (_, login) = fixture.start(&account).await;
        let id = login["id"].as_str().unwrap();
        let url = format!(
            "ws://{addr}/v1/ws/harness-logins/{}/{account}/{id}?token={}",
            fixture.operator.workspace_id,
            session_token(&fixture.operator)
        );
        let (mut socket, _) = connect_async(url).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut text = String::new();
            while !text.contains("Official CLI sign-in fixture") {
                text.push_str(&String::from_utf8_lossy(
                    &socket.next().await.unwrap().unwrap().into_data(),
                ));
            }
        })
        .await
        .unwrap();
        let pid = fs::read_to_string(dir.join("pid")).unwrap();
        let login_directory = fs::read_to_string(dir.join("cwd")).unwrap();
        if expire {
            fixture.client.execute("UPDATE harness_account_login SET expires_at = NOW() - INTERVAL '1 second' WHERE id = $1", &[&id]).await.unwrap();
        } else {
            assert_eq!(fixture.cancel(&account, id).await, StatusCode::NO_CONTENT);
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(
                    socket.next().await,
                    None | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
                        | Some(Err(_))
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .output()
                .unwrap()
                .status
                .success()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("closed authentication terminal must stop its CLI");
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::path::Path::new(&login_directory).exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancelled or expired login must release its private directory");
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn claude_login_does_not_start_an_oauth_relay_or_store_callback_material() {
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;
    let (status, login) = fixture.start(&account_id).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(login["state"], "authorizing");
    assert!(login["authorization_url"].is_null());
    let id = login["id"].as_str().unwrap();
    assert_eq!(
        fixture
            .submit_callback(&account_id, id, "private-code#state")
            .await,
        StatusCode::CONFLICT
    );
    let row = fixture.client.query_one("SELECT authorization_url, user_code, callback_code FROM harness_account_login WHERE id = $1", &[&id]).await.unwrap();
    for column in ["authorization_url", "user_code", "callback_code"] {
        assert!(row.get::<_, Option<String>>(column).is_none());
    }
    assert_eq!(
        fixture.cancel(&account_id, id).await,
        StatusCode::NO_CONTENT
    );
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
    let _signed_in = EnvVarGuard::set_path("FAKE_CLAUDE_SIGNED_IN", std::path::Path::new("1"));
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
    assert_eq!(
        fixture.complete(&account_id, &login_id).await.0,
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
async fn missing_claude_binary_does_not_claim_the_account_is_signed_in() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-login-missing");
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &dir.join("missing"));
    let fixture = LoginFixture::create().await;
    let account = fixture.account(None).await;
    let (_, login) = fixture.start(&account).await;
    let id = login["id"].as_str().unwrap();
    let (status, _) = fixture.complete(&account, id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        fixture
            .account_row(&account)
            .await
            .get::<_, String>("status"),
        "pending"
    );
    fixture.cancel(&account, id).await;
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
    let (resumed_status, resumed) = fixture.start(&account_id).await;
    assert_eq!(resumed_status, StatusCode::OK);
    assert_eq!(resumed["id"], login["id"]);

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
async fn concurrent_login_starts_resume_one_authoritative_job() {
    let fixture = LoginFixture::create().await;
    let host_id = fixture.runtime_host().await;
    let account_id = fixture.account(Some(&host_id)).await;
    let (first, second) = tokio::join!(fixture.start(&account_id), fixture.start(&account_id));
    assert!(
        (first.0 == StatusCode::CREATED && second.0 == StatusCode::OK)
            || (second.0 == StatusCode::CREATED && first.0 == StatusCode::OK)
    );
    assert_eq!(first.1["id"], second.1["id"]);
    let count: i64 = fixture
        .client
        .query_one(
            "SELECT COUNT(*) FROM harness_account_login WHERE account_id = $1",
            &[&account_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
    let login_id = first.1["id"].as_str().unwrap();
    assert_eq!(
        fixture.cancel(&account_id, login_id).await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn cancelling_claude_login_prevents_done_from_activating_the_account() {
    let fixture = LoginFixture::create().await;
    let account = fixture.account(None).await;
    let (_, login) = fixture.start(&account).await;
    let id = login["id"].as_str().unwrap();
    assert_eq!(fixture.cancel(&account, id).await, StatusCode::NO_CONTENT);
    assert_eq!(
        fixture.complete(&account, id).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .account_row(&account)
            .await
            .get::<_, String>("status"),
        "pending"
    );
}

#[tokio::test]
async fn remote_harness_login_waits_for_the_connector_and_expires() {
    let fixture = LoginFixture::create().await;
    let host_id = fixture.runtime_host().await;
    let account_id = fixture
        .account_for_driver(Some(&host_id), "codex_terminal")
        .await;
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

impl LoginFixture {
    async fn probe(&self, account_id: &str) -> (StatusCode, Value) {
        api_json_request(
            self.router.clone(),
            &self.operator,
            Method::POST,
            format!(
                "/v1/companies/{}/harness-accounts/{account_id}/probe",
                self.operator.workspace_id
            ),
        )
        .await
    }

    async fn account_row(&self, account_id: &str) -> tokio_postgres::Row {
        self.client
            .query_one(
                "SELECT status, last_error, models_json, usage_json, probed_at
                   FROM harness_account WHERE id = $1",
                &[&account_id],
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn probing_a_signed_in_local_account_stores_its_snapshot_without_a_sign_in() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-probe");
    let binary = dir.join("claude");
    write_executable_script(&binary, FAKE_CLAUDE);
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CLAUDE_DIR", &dir);
    let _signed_in = EnvVarGuard::set_path("FAKE_CLAUDE_SIGNED_IN", std::path::Path::new("1"));
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;

    let (status, body) = fixture.probe(&account_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let row = fixture.account_row(&account_id).await;
    assert_eq!(row.get::<_, String>("status"), "active");
    assert_eq!(row.get::<_, Option<String>>("last_error"), None);
    assert_eq!(
        row.get::<_, Value>("models_json")[0]["id"],
        "claude-sonnet-4-5"
    );
    assert_eq!(
        row.get::<_, Value>("usage_json")["windows"][0]["remainingPercent"],
        87.5
    );
    assert!(
        row.get::<_, Option<chrono::DateTime<chrono::Utc>>>("probed_at")
            .is_some()
    );
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn probing_an_account_without_a_login_records_the_reason_and_answers_409() {
    let _env = api_test_env_lock().lock().await;
    let dir = isolated_test_dir("harness-probe-unsigned");
    let binary = dir.join("claude");
    write_executable_script(&binary, FAKE_CLAUDE);
    let _binary = EnvVarGuard::set_path("CHORUZ_CLAUDE_BINARY", &binary);
    let _fake_dir = EnvVarGuard::set_path("FAKE_CLAUDE_DIR", &dir);
    let _profiles = EnvVarGuard::set_path("CHORUZ_HARNESS_ACCOUNT_ROOT", &dir.join("accounts"));
    let fixture = LoginFixture::create().await;
    let account_id = fixture.account(None).await;

    let (status, body) = fixture.probe(&account_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let row = fixture.account_row(&account_id).await;
    assert_eq!(row.get::<_, String>("status"), "reauth_required");
    assert!(
        row.get::<_, Option<String>>("last_error")
            .unwrap_or_default()
            .contains("sign in"),
    );

    // A remote account whose device holds no link is refused without being
    // marked: the device, not the login, is what is missing.
    let host_id = fixture.runtime_host().await;
    let remote_account_id = fixture.account(Some(&host_id)).await;
    let (status, body) = fixture.probe(&remote_account_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("not connected"), "{body}");
    let row = fixture.account_row(&remote_account_id).await;
    assert_eq!(row.get::<_, String>("status"), "pending");
    drop(fixture);
    fs::remove_dir_all(&dir).ok();
}
