#![cfg(unix)]

use choruz_domain::evaluation::ReplayEnvironment;
use choruz_host_runtime::{TerminalSpec, evaluation_replay};
use serde_json::json;
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, process::Command};

#[tokio::test]
async fn replay_executes_multiple_steps_in_a_real_isolated_container() {
    // Docker is an explicit integration-test dependency. Only model generation
    // is replaced; the container, file effects and verification are real.
    let pull = Command::new("docker")
        .args(["pull", "alpine:3.22"])
        .output()
        .expect("Docker is required for replay tests");
    assert!(
        pull.status.success(),
        "{}",
        String::from_utf8_lossy(&pull.stderr)
    );
    let image = Command::new("docker")
        .args(["image", "inspect", "alpine:3.22", "--format", "{{.Id}}"])
        .output()
        .unwrap();
    assert!(image.status.success());
    let workspace = tempfile::tempdir().unwrap();
    let live_file = workspace.path().join("number.txt");
    std::fs::write(&live_file, "live workspace must remain unchanged").unwrap();
    let binary = workspace.path().join("model-fixture");
    std::fs::write(&binary, r#"#!/usr/bin/env python3
import json, os, sys
assert not os.listdir('.'), 'model must not receive the live workspace'
data = json.loads(sys.stdin.read().strip().splitlines()[-1])
assert 'check' not in data and 'expected' not in data
task = json.loads(data['task'].splitlines()[-1])
observations = task['observations']
if not observations:
    action = {'action':'command', 'command':'cat number.txt'}
elif len(observations) == 1:
    assert observations[0]['output'].strip() == '1'
    action = {'action':'command','command':'printf 2 > number.txt; test ! -e /var/run/docker.sock && test "$(ls /sys/class/net)" = lo && ! touch /host-write-test'}
else:
    assert observations[1]['success'] is True
    action = {'action':'finish','output':'done'}
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':json.dumps(action)}}))
"#).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let spec = TerminalSpec {
        authentication: false,
        terminal_id: "replay-test".into(),
        driver_type: "codex_terminal".into(),
        binary_path: Some(binary.to_string_lossy().into()),
        workspace_path: workspace.path().to_string_lossy().into(),
        cols: 120,
        rows: 40,
        resume_session_id: None,
        codex_home: None,
        model: Some("replay-fixture".into()),
        harness_account: json!({}),
    };
    let environment = ReplayEnvironment {
        image: String::from_utf8(image.stdout).unwrap().trim().into(),
        files: BTreeMap::from([("number.txt".into(), "1".into())]),
        verification: vec![
            "test \"$(cat number.txt)\" = 2".into(),
            "test -f number.txt".into(),
        ],
        max_steps: 3,
    };
    let result = evaluation_replay::run(
        spec.clone(),
        "Increment the file value".into(),
        String::new(),
        String::new(),
        environment.clone(),
    )
    .await
    .unwrap();
    assert_eq!(result.output, "done");
    assert_eq!(result.observations.len(), 2);
    assert_eq!(result.checks.len(), 2);
    assert!(result.checks.iter().all(|check| check["passed"] == true));
    let remaining = Command::new("docker")
        .args([
            "ps",
            "--all",
            "--filter",
            &format!("name=^/{}$", result.container_id),
            "--format",
            "{{.ID}}",
        ])
        .output()
        .unwrap();
    assert!(remaining.status.success());
    assert!(
        remaining.stdout.is_empty(),
        "owned container must be removed before completion"
    );
    let mut broken_check = environment.clone();
    broken_check.verification = vec!["test \"$(cat number.txt)\" = 999".into()];
    let rejected = evaluation_replay::run(
        spec.clone(),
        "Increment the file value".into(),
        String::new(),
        String::new(),
        broken_check,
    )
    .await
    .unwrap();
    assert_eq!(
        rejected.checks[0]["passed"], false,
        "a successful agent reply must not override a failing executable check"
    );
    let mut exhausted = environment;
    exhausted.max_steps = 1;
    let error = evaluation_replay::run(
        spec,
        "Increment the file value".into(),
        String::new(),
        String::new(),
        exhausted,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("step budget exhausted"), "{error}");
    let name = error
        .split_whitespace()
        .find(|part| part.starts_with("choruz-evaluation-"))
        .unwrap();
    let remaining = Command::new("docker")
        .args([
            "ps",
            "--all",
            "--filter",
            &format!("name=^/{name}$"),
            "--format",
            "{{.ID}}",
        ])
        .output()
        .unwrap();
    assert!(remaining.status.success());
    assert!(
        remaining.stdout.is_empty(),
        "failed replay must also remove its container"
    );
    assert_eq!(
        std::fs::read_to_string(live_file).unwrap(),
        "live workspace must remain unchanged"
    );
}
