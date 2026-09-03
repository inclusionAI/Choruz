//! `choruz` is the scriptable control-plane client for a running Choruz host.
//!
//! It deliberately talks to the same authenticated HTTP API used by the Web
//! Dashboard. It never writes the database directly, so CLI and Web behavior
//! share permissions, audit records, and validation.

use std::{
    env,
    fs::{self, OpenOptions},
    path::PathBuf,
    process::{ExitCode, Stdio},
};

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command as ProcessCommand,
};

const DEFAULT_API_URL: &str = "http://127.0.0.1:3000";
const DEFAULT_PIPELINE_URL: &str = "http://127.0.0.1:3020";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    Start,
    Status,
    CompanyList,
    AgentList,
    RemoteStatus,
    RemotePairingCredential,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    command: Command,
    json: bool,
    api_url: String,
    pipeline_url: String,
    token: Option<String>,
    operator_password: Option<String>,
}

fn usage() -> &'static str {
    "choruz — Choruz control-plane CLI

USAGE:
  choruz status [--json]
  choruz start
  choruz company list [--json]
  choruz agent list [--json]
  choruz remote status [--json]
  choruz remote pairing-credential [--json]

GLOBAL OPTIONS:
  --api-url <url>       API Gateway URL (default: CHORUZ_API_BASE_URL or http://127.0.0.1:3000)
  --pipeline-url <url>  Pipeline URL (default: CHORUZ_PIPELINE_URL or http://127.0.0.1:3020)
  --token <token>       Human session token (default: CHORUZ_SESSION_TOKEN)
  --json                Machine-readable JSON output

AUTHENTICATION:
  Commands that read the control plane use CHORUZ_SESSION_TOKEN. On a
  loopback host only, the CLI can instead obtain a short-lived token using
  CHORUZ_OPERATOR_USER (default: operator) and CHORUZ_OPERATOR_PASSWORD.
  A remote API URL always requires --token or CHORUZ_SESSION_TOKEN.

REMOTE CONTROL:
  `choruz remote pairing-credential` asks the running host to open the same
  credential-bound pairing flow as Dashboard → Actions → Remote Control.

  `choruz start` launches the bundled headless host in the background and prints
  a pasteable Remote Control pairing credential after its Gateway connection is
  ready. Enter it at the printed Dashboard URL; no Cloudflare account or Gateway
  secret is needed.
"
}

fn default_api_url() -> String {
    env::var("CHORUZ_API_BASE_URL").unwrap_or_else(|_| DEFAULT_API_URL.into())
}

fn default_pipeline_url() -> String {
    env::var("CHORUZ_PIPELINE_URL").unwrap_or_else(|_| DEFAULT_PIPELINE_URL.into())
}

fn parse_args(input: &[String]) -> Result<Args, String> {
    let mut positional = Vec::new();
    let mut json = false;
    let mut api_url = default_api_url();
    let mut pipeline_url = default_pipeline_url();
    let mut token = env::var("CHORUZ_SESSION_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let operator_password = env::var("CHORUZ_OPERATOR_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let mut index = 0;
    while index < input.len() {
        match input[index].as_str() {
            "--help" | "-h" => {
                return Ok(Args {
                    command: Command::Help,
                    json,
                    api_url,
                    pipeline_url,
                    token,
                    operator_password,
                });
            }
            "--version" | "-V" => {
                return Ok(Args {
                    command: Command::Version,
                    json,
                    api_url,
                    pipeline_url,
                    token,
                    operator_password,
                });
            }
            "--json" => json = true,
            "--api-url" => {
                index += 1;
                api_url = input
                    .get(index)
                    .ok_or("--api-url requires a value")?
                    .clone();
            }
            "--pipeline-url" => {
                index += 1;
                pipeline_url = input
                    .get(index)
                    .ok_or("--pipeline-url requires a value")?
                    .clone();
            }
            "--token" => {
                index += 1;
                token = Some(input.get(index).ok_or("--token requires a value")?.clone());
            }
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }
    let command = match positional.as_slice() {
        [] => Command::Help,
        [command] if command == "status" => Command::Status,
        [command] if command == "start" => Command::Start,
        [command] if command == "version" => Command::Version,
        [area, action] if area == "company" && action == "list" => Command::CompanyList,
        [area, action] if area == "agent" && action == "list" => Command::AgentList,
        [area, action] if area == "remote" && action == "status" => Command::RemoteStatus,
        [area, action] if area == "remote" && action == "pairing-credential" => {
            Command::RemotePairingCredential
        }
        _ => return Err(format!("unknown command: {}", positional.join(" "))),
    };
    Ok(Args {
        command,
        json,
        api_url,
        pipeline_url,
        token,
        operator_password,
    })
}

fn api_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn is_loopback(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return false;
    };
    parsed.scheme() == "http"
        && matches!(
            parsed.host_str().map(str::to_ascii_lowercase).as_deref(),
            Some("127.0.0.1" | "::1" | "[::1]")
        )
}

async fn authenticate(client: &Client, args: &Args) -> Result<String, String> {
    if let Some(token) = &args.token {
        return Ok(token.clone());
    }
    if !is_loopback(&args.api_url) {
        return Err("a remote API requires --token or CHORUZ_SESSION_TOKEN".into());
    }
    let password = args
        .operator_password
        .clone()
        .ok_or("set CHORUZ_SESSION_TOKEN, or set CHORUZ_OPERATOR_PASSWORD for this local host")?;
    let username = env::var("CHORUZ_OPERATOR_USER").unwrap_or_else(|_| "operator".into());
    let response = client
        .post(api_url(&args.api_url, "/v1/auth/local/login"))
        .json(&json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|error| format!("authenticate with Choruz: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "authenticate with Choruz: HTTP {}",
            response.status()
        ));
    }
    response
        .json::<Value>()
        .await
        .map_err(|error| format!("read authentication response: {error}"))?
        .get("session_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "authentication response did not include a session token".into())
}

async fn authenticated_request(client: &Client, args: &Args, path: &str) -> Result<Value, String> {
    let token = authenticate(client, args).await?;
    let response = client
        .get(api_url(&args.api_url, path))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;
    read_json_response(response, path).await
}

async fn authenticated_post(client: &Client, args: &Args, path: &str) -> Result<Value, String> {
    authenticated_json_post(client, args, path, Value::Null).await
}

async fn authenticated_json_post(
    client: &Client,
    args: &Args,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    let token = authenticate(client, args).await?;
    let response = client
        .post(api_url(&args.api_url, path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("request {path}: {error}"))?;
    read_json_response(response, path).await
}

async fn read_json_response(response: reqwest::Response, operation: &str) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("read {operation} response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "{operation}: HTTP {status}: {}",
            body.chars().take(240).collect::<String>()
        ));
    }
    serde_json::from_str(&body).map_err(|error| format!("decode {operation} response: {error}"))
}

fn print_value(value: &Value, json_output: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("JSON values serialize")
        );
    }
}

async fn status(client: &Client, args: &Args) -> Result<Value, String> {
    let api = client.get(api_url(&args.api_url, "/readyz")).send().await;
    let pipeline = client
        .get(api_url(&args.pipeline_url, "/readyz"))
        .send()
        .await;
    let api_status = api.map(|response| response.status().as_u16()).unwrap_or(0);
    let pipeline_status = pipeline
        .map(|response| response.status().as_u16())
        .unwrap_or(0);
    Ok(json!({
        "api": { "url": args.api_url, "status": api_status, "ready": api_status == StatusCode::OK.as_u16() },
        "pipeline": { "url": args.pipeline_url, "status": pipeline_status, "ready": pipeline_status == StatusCode::OK.as_u16() },
    }))
}

fn print_status(value: &Value, json_output: bool) {
    if json_output {
        print_value(value, true);
        return;
    }
    for name in ["api", "pipeline"] {
        let entry = &value[name];
        let state = if entry["ready"].as_bool() == Some(true) {
            "ready"
        } else {
            "unavailable"
        };
        println!(
            "{name:<9} {state:<11} {} ({})",
            entry["url"].as_str().unwrap_or(""),
            entry["status"].as_u64().unwrap_or(0)
        );
    }
}

fn print_companies(value: &Value, json_output: bool) {
    if json_output {
        return print_value(value, true);
    }
    for company in value.as_array().into_iter().flatten() {
        println!(
            "{}\t{}\t{}",
            company["name"].as_str().unwrap_or(""),
            company["id"].as_str().unwrap_or(""),
            company["folder_path"].as_str().unwrap_or("—")
        );
    }
}

fn print_agents(value: &Value, json_output: bool) {
    if json_output {
        return print_value(value, true);
    }
    for agent in value["agents"].as_array().into_iter().flatten() {
        println!(
            "{}\t{}\t{}",
            agent["name"].as_str().unwrap_or(""),
            agent["id"].as_str().unwrap_or(""),
            agent["workspace_id"].as_str().unwrap_or("")
        );
    }
}

fn print_remote_status(value: &Value, json_output: bool) {
    if json_output {
        return print_value(value, true);
    }
    println!(
        "gateway: {}",
        value["gateway_url"].as_str().unwrap_or("not configured")
    );
    println!(
        "bridge ticket: {}",
        if value["gateway_ticket"].is_string() {
            "available"
        } else {
            "unavailable"
        }
    );
}

fn print_pairing(value: &Value, json_output: bool) {
    if json_output {
        return print_value(value, true);
    }
    println!(
        "Pairing credential: {}",
        value["credential"].as_str().unwrap_or("")
    );
    println!("Expires: {}", value["expires_at"].as_str().unwrap_or(""));
    if let Some(url) = value["gateway_url"].as_str() {
        println!("Remote Dashboard: {url}");
    }
}

fn remote_gateway_configured(value: &Value) -> bool {
    value["gateway_url"]
        .as_str()
        .is_some_and(|url| !url.trim().is_empty())
}

async fn create_remote_pairing(client: &Client, args: &Args) -> Result<Value, String> {
    let settings = authenticated_request(client, args, "/v1/remote-control/settings").await?;
    if !remote_gateway_configured(&settings) {
        return Err("Remote Control Gateway is unavailable on this host".into());
    }
    let pairing = authenticated_post(client, args, "/v1/remote-control/pairings").await?;
    for field in ["gateway_url", "credential", "expires_at"] {
        if !pairing[field].is_string() {
            return Err(format!("pairing response did not include {field}"));
        }
    }
    Ok(pairing)
}

fn server_binary() -> Result<PathBuf, String> {
    if let Ok(value) = env::var("CHORUZ_SERVER_BINARY") {
        let candidate = PathBuf::from(value);
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err("CHORUZ_SERVER_BINARY does not point to an executable file".into());
    }
    let current = env::current_exe().map_err(|error| format!("locate choruz binary: {error}"))?;
    let candidate = current
        .parent()
        .ok_or("choruz binary has no parent directory")?
        .join("choruz-server");
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "could not find bundled choruz-server next to {}; install a full Choruz release or set CHORUZ_SERVER_BINARY",
            current.display()
        ))
    }
}

async fn start_host(client: &Client, args: &Args) -> Result<(), String> {
    let mut local = args.clone();
    local.api_url = DEFAULT_API_URL.into();
    // choruz-server is loopback-only; its documented first-run credentials are
    // the local defaults unless the operator supplied an override.
    if local.token.is_none() && local.operator_password.is_none() {
        local.operator_password = Some("choruz-local".into());
    }
    if !api_ready(client, &local.api_url).await {
        let binary = server_binary()?;
        let data_dir = dirs::data_dir()
            .ok_or("no OS data directory available")?
            .join("choruz");
        fs::create_dir_all(&data_dir)
            .map_err(|error| format!("create {}: {error}", data_dir.display()))?;
        let log_path = data_dir.join("choruz-server.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| format!("open {}: {error}", log_path.display()))?;
        let mut command = ProcessCommand::new(binary);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        let mut child = spawn_background_process(&mut command)
            .map_err(|error| format!("start choruz-server: {error}"))?;
        let pid = child
            .id()
            .ok_or("choruz-server did not report its process id")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("capture choruz-server startup output")?;
        let mut lines = BufReader::new(stdout).lines();
        let port = loop {
            let line = lines
                .next_line()
                .await
                .map_err(|error| format!("read choruz-server startup output: {error}"))?
                .ok_or("choruz-server exited before it became ready")?;
            if let Some(value) = line.strip_prefix("CHORUZ_LISTENING=") {
                break value
                    .parse::<u16>()
                    .map_err(|_| "choruz-server reported an invalid listening port")?;
            }
        };
        local.api_url = format!("http://127.0.0.1:{port}");
        fs::write(data_dir.join("choruz-server.pid"), format!("{pid}\n"))
            .map_err(|error| format!("record choruz-server pid: {error}"))?;
        println!("Choruz host started in the background (pid {pid}).");
        println!("Logs: {}", log_path.display());
    } else {
        println!("Choruz host is already running in the background.");
    }
    let pairing = create_remote_pairing(client, &local).await?;
    print_pairing(&pairing, false);
    Ok(())
}

async fn api_ready(client: &Client, base_url: &str) -> bool {
    client
        .get(api_url(base_url, "/readyz"))
        .send()
        .await
        .is_ok_and(|response| response.status() == StatusCode::OK)
}

#[cfg(unix)]
fn configure_background_process(command: &mut ProcessCommand) {
    // SAFETY: `setsid` is async-signal-safe and touches no memory shared with
    // the parent between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_background_process(_command: &mut ProcessCommand) {}

fn spawn_background_process(
    command: &mut ProcessCommand,
) -> std::io::Result<tokio::process::Child> {
    configure_background_process(command);
    command.kill_on_drop(false).spawn()
}

#[tokio::main]
async fn main() -> ExitCode {
    if install_tls_provider().is_err() {
        eprintln!("error: initialize TLS cryptography provider");
        return ExitCode::FAILURE;
    }
    let raw: Vec<String> = env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("error: {error}\n\n{}", usage());
            return ExitCode::from(2);
        }
    };
    match args.command.clone() {
        Command::Help => print!("{}", usage()),
        Command::Version => println!("choruz {}", env!("CARGO_PKG_VERSION")),
        command => {
            let client = match Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(20))
                .build()
            {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("error: create HTTP client: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let outcome = match command {
                Command::Status => status(&client, &args).await.map(|value| {
                    print_status(&value, args.json);
                }),
                Command::Start => start_host(&client, &args).await,
                Command::CompanyList => authenticated_request(&client, &args, "/v1/companies")
                    .await
                    .map(|value| print_companies(&value, args.json)),
                Command::AgentList => {
                    authenticated_request(&client, &args, "/v1/bootstrap?limit=1")
                        .await
                        .map(|value| print_agents(&value, args.json))
                }
                Command::RemoteStatus => {
                    authenticated_request(&client, &args, "/v1/remote-control/settings")
                        .await
                        .map(|value| print_remote_status(&value, args.json))
                }
                Command::RemotePairingCredential => create_remote_pairing(&client, &args)
                    .await
                    .map(|value| print_pairing(&value, args.json)),
                Command::Help | Command::Version => unreachable!(),
            };
            if let Err(error) = outcome {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn install_tls_provider() -> Result<(), ()> {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::time::Duration;

    fn values(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).into()).collect()
    }

    #[test]
    fn parses_remote_pairing_with_machine_output() {
        let args = parse_args(&values(&[
            "remote",
            "pairing-credential",
            "--json",
            "--api-url",
            "http://127.0.0.1:3999",
        ]))
        .unwrap();
        assert_eq!(args.command, Command::RemotePairingCredential);
        assert!(args.json);
        assert_eq!(args.api_url, "http://127.0.0.1:3999");
    }

    #[test]
    fn installs_a_deterministic_tls_provider_for_pairing() {
        assert_eq!(install_tls_provider(), Ok(()));
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

    #[test]
    fn parses_start_as_the_zero_configuration_host_flow() {
        let args = parse_args(&values(&["start"])).unwrap();
        assert_eq!(args.command, Command::Start);
        assert!(!args.json);
    }

    #[test]
    fn rejects_unknown_command_instead_of_guessing() {
        assert!(parse_args(&values(&["remote", "pair"])).is_err());
    }

    #[test]
    fn recognizes_only_loopback_automatic_login_targets() {
        assert!(is_loopback("http://127.0.0.1:3000"));
        assert!(is_loopback("http://[::1]:3000"));
        assert!(!is_loopback("https://choruz.example"));
        assert!(!is_loopback("http://127.0.0.1.attacker.example"));
        assert!(!is_loopback("http://localhost.attacker.example"));
        assert!(!is_loopback("http://localhost:3000"));
    }

    #[test]
    fn requires_a_configured_gateway_before_creating_a_code() {
        assert!(!remote_gateway_configured(&json!({ "gateway_url": null })));
        assert!(remote_gateway_configured(
            &json!({ "gateway_url": "https://gateway.example" })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_process_survives_its_child_handle() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("still-running");
        let mut command = ProcessCommand::new("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 0.05; printf ready > \"$1\"")
            .arg("choruz-background-test")
            .arg(&marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = spawn_background_process(&mut command).unwrap();
        drop(child);

        tokio::time::timeout(Duration::from_secs(2), async {
            while !matches!(fs::read_to_string(&marker), Ok(contents) if contents == "ready") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the detached process should outlive the dropped child handle");
        assert_eq!(fs::read_to_string(marker).unwrap(), "ready");
    }
}
