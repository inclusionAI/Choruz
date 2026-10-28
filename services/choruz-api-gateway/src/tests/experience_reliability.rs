use super::*;
use choruz_host_runtime::HostRequest;
use choruz_host_runtime::link::{ControllerFrame, LinkRequest};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn learning_builder_recovery_preserves_core_analysis_without_repeating_it() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("builder-recovery", "password-123")
        .await
        .unwrap();
    let target = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;
    let builder = learning_binding(&database, &owner).await;
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    let files = tempfile::tempdir().unwrap();
    let workspace = files.path().join("workspace");
    let home = files.path().join("account");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(home.join("sessions")).unwrap();
    let native = home.join("sessions/source.jsonl");
    fs::write(&native,format!("{}\n{}\n",json!({"type":"session_meta","payload":{"id":"builder-source","cwd":workspace}}),json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Retain this completed task."}]}}))).unwrap();
    let binding = runtime.get_binding(&target).await.unwrap();
    let anchor = json!({"driver_type":"codex_terminal","session_id":"builder-source","source":"native_cli","provenance":"terminal_process_captured",
        "binding_id":target,"conversation_id":binding.conversation_id,"agent_principal_id":binding.agent_principal_id,
        "company_id":owner.workspace_id,"workspace_id":owner.workspace_id,"workspace_path":workspace,
        "native_home_path":home,"native_session_path":native,"binding_generation":0,"captured_at":"2026-01-01T00:00:00Z"});
    let cli = files.path().join("analyst");
    fs::write(
        &cli,
        r#"#!/usr/bin/env python3
import json, pathlib, sys
source = sys.stdin.read()
root = pathlib.Path(__file__).parent
if 'Training examples:' in source:
    (root / 'builder-finished').touch()
    result = 'null'
else:
    with (root / 'analysis-calls').open('a') as log:
        log.write('called\n')
    result = json.dumps({'summary':'Source analysis is complete.','instruction':None,'evidence':[],
          'previous_revision_outcome':'not_observed','problems':[],'addressed_problems':[]})
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':result}}))
"#,
    )
    .unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',workspace_path=$2,config_json=$3 WHERE id=$1", &[&target,&workspace.to_string_lossy().as_ref(),&json!({"terminal_session":anchor})]).await.unwrap();
    for id in [&analyst, &builder] {
        client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[id,&json!({"binary_path":cli})]).await.unwrap();
    }
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    let settings = choruz_decision::settings::LearningSettings {
        model: "fixture-model".into(),
        minimum_confidence: 0.9,
        classify: false,
        supervise: false,
        assist_turns: false,
        complete_turns: false,
        builder_binding_id: Some(builder.clone()),
    };
    db.configure_decisions(&owner.workspace_id, &owner.id, &target, Some(&settings))
        .await
        .unwrap();
    let claim = db.claim_experience().await.unwrap().unwrap();
    let cases: Vec<choruz_evaluation::evaluation::TraceCase> = (0..30).map(|index|serde_json::from_value(json!({"episode_ref":format!("independent-{index}"),"evidence":[format!("answer-{index}")],"input":format!("Classify billing ticket {index}"),"check":{"type":"exact","expected":"billing"},"reason":"Independently reviewed objective"})).unwrap()).collect();
    db.save_experience_candidate(
        &claim,
        ExperienceReport {
            digest: "seed-corpus",
            references: &json!([]),
            analysis: "Reviewed corpus fixture",
            instruction: None,
            validation: &json!({"evaluation_cases":cases}),
            checkpoint: None,
            activate: false,
        },
    )
    .await
    .unwrap()
    .unwrap();
    db.release_experience(
        &claim,
        choruz_application::db_service::CheckOutcome::Completed,
    )
    .await
    .unwrap();
    // The corpus represents objectives already checked earlier in this paged
    // sweep. This scenario targets builder recovery, not curation withdrawal.
    client.execute("UPDATE experience_policy SET source_cursor=$2 WHERE binding_id=$1",&[&target,&json!({"_curation_started":chrono::Utc::now().timestamp(),"_curation_seen":cases.iter().map(|case|&case.episode_ref).collect::<Vec<_>>(),"_more":true})]).await.unwrap();
    client
        .execute(
            "UPDATE agent_runtime_bindings SET state='disabled' WHERE id=$1",
            &[&builder],
        )
        .await
        .unwrap();
    make_learning_due(&**client, &target).await;
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let reports = db
                .experience_revisions(&owner.workspace_id, &owner.id, &target)
                .await
                .unwrap();
            if let Some(report) = reports
                .iter()
                .find(|report| report.validation["program_trial"]["status"] == "pending")
            {
                assert_eq!(report.analysis, "Source analysis is complete.");
                assert_eq!(
                    db.experience_trace_cases(&owner.workspace_id, &target)
                        .await
                        .unwrap()
                        .len(),
                    30,
                    "optional builder failure must preserve the reviewed benchmark corpus"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("disabled optional builder must not discard core analysis");
    client
        .execute(
            "UPDATE agent_runtime_bindings SET state='idle' WHERE id=$1",
            &[&builder],
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            make_learning_due(&**client, &target).await;
            let reports = db
                .experience_revisions(&owner.workspace_id, &owner.id, &target)
                .await
                .unwrap();
            if reports
                .iter()
                .any(|report| report.validation["program_trial"]["status"] == "not_suitable")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("restored builder must reach an explicit final suitability outcome");
    assert!(files.path().join("builder-finished").exists());
    assert_eq!(
        fs::read_to_string(files.path().join("analysis-calls"))
            .unwrap()
            .lines()
            .count(),
        1,
        "recovering the optional builder must reuse completed analysis, not call the model twice"
    );
    let retained: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM experience_window_checkpoint WHERE binding_id=$1",
            &[&target],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(retained, 0);
    drop(app);
}

#[tokio::test]
async fn learning_parallel_profiles_are_bounded_and_do_not_wait_for_a_slow_peer() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("parallel-learning", "password-123")
        .await
        .unwrap();
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    let files = tempfile::tempdir().unwrap();
    let cli = files.path().join("analyst");
    // Only model generation is replaced. Native evidence collection, worker
    // admission, device dispatch and every database commit remain production code.
    fs::write(&cli, r#"#!/usr/bin/env python3
import json, pathlib, sys, time
sys.stdin.read()
root = pathlib.Path(__file__).parent
model = sys.argv[sys.argv.index('--model')+1]
(root / (model + '.started')).touch()
if model.startswith('slow-'):
    while not (root / (model + '.release')).exists():
        time.sleep(0.01)
report = {'summary':'Observed task retained.','instruction':None,'evidence':[],
          'previous_revision_outcome':'not_observed','problems':[],'addressed_problems':[]}
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':json.dumps(report)}}))
"#).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let mut bindings = Vec::new();
    for index in 0..5 {
        let target = learning_binding(&database, &owner).await;
        let analyst = learning_binding(&database, &owner).await;
        let workspace = files.path().join(format!("workspace-{index}"));
        let home = files.path().join(format!("account-{index}"));
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(home.join("sessions")).unwrap();
        let native = home.join("sessions/source.jsonl");
        let session = format!("parallel-source-{index}");
        fs::write(&native,format!("{}\n{}\n",json!({"type":"session_meta","payload":{"id":session,"cwd":workspace}}),json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Retain this completed task."}]}}))).unwrap();
        let binding = runtime.get_binding(&target).await.unwrap();
        let anchor = json!({"driver_type":"codex_terminal","session_id":session,"source":"native_cli","provenance":"terminal_process_captured",
            "binding_id":target,"conversation_id":binding.conversation_id,"agent_principal_id":binding.agent_principal_id,
            "company_id":owner.workspace_id,"workspace_id":owner.workspace_id,"workspace_path":workspace,
            "native_home_path":home,"native_session_path":native,"binding_generation":0,"captured_at":"2026-01-01T00:00:00Z"});
        client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',workspace_path=$2,config_json=$3 WHERE id=$1", &[&target,&workspace.to_string_lossy().as_ref(),&json!({"terminal_session":anchor})]).await.unwrap();
        let model = if index < 4 {
            format!("slow-{index}")
        } else {
            "fast".into()
        };
        client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[&analyst,&json!({"binary_path":cli,"model":model})]).await.unwrap();
        db.configure_experience(
            &owner.workspace_id,
            &owner.id,
            &target,
            &analyst,
            true,
            None,
        )
        .await
        .unwrap();
        bindings.push(target);
    }
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if (0..4).all(|index| files.path().join(format!("slow-{index}.started")).exists()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !files.path().join("fast.started").exists(),
            "the fifth profile must not exceed the four-profile bound"
        );
        fs::write(files.path().join("slow-0.release"), "").unwrap();
        loop {
            if !db
                .experience_revisions(&owner.workspace_id, &owner.id, &bindings[4])
                .await
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(files.path().join("fast.started").exists());
        assert!(
            !files.path().join("slow-1.release").exists(),
            "a fast profile commits while another profile is still blocked"
        );
    })
    .await;
    // Release every owned model process even if the assertions above failed.
    for index in 0..4 {
        fs::write(files.path().join(format!("slow-{index}.release")), "").unwrap();
    }
    outcome.expect("independent profiles must progress through the bounded worker");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let mut completed = true;
            for binding in &bindings {
                completed &= !db
                    .experience_revisions(&owner.workspace_id, &owner.id, binding)
                    .await
                    .unwrap()
                    .is_empty();
            }
            if completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("all released model processes must finish their owned work");
    drop(app);
}

#[tokio::test]
async fn evaluations_of_other_bindings_do_not_wait_for_a_slow_one() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    let files = tempfile::tempdir().unwrap();
    let evaluator = files.path().join("evaluation-model");
    fs::write(
        &evaluator,
        include_str!(
            "../../../../crates/choruz-host-runtime/tests/fixtures/experience-evaluator.py"
        ),
    )
    .unwrap();
    // Only model generation is replaced; the slow device holds its case until
    // released, while queueing, leases, dispatch and results stay production code.
    let slow = files.path().join("slow-model");
    fs::write(
        &slow,
        format!(
            r#"#!/usr/bin/env python3
import os, pathlib, sys, time
root = pathlib.Path({root:?})
(root / "slow.started").touch()
while not (root / "slow.release").exists():
    time.sleep(0.01)
os.execv({evaluator:?}, [{evaluator:?}] + sys.argv[1:])
"#,
            root = files.path(),
            evaluator = evaluator
        ),
    )
    .unwrap();
    for binary in [&evaluator, &slow] {
        fs::set_permissions(binary, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let suite = json!({"name":"Number format","cases":[
        {"id":"train","split":"train","input":"Increment 10","check":{"type":"exact","expected":"11"}},
        {"id":"validation","split":"validation","input":"Increment 11","check":{"type":"exact","expected":"12"}},
        {"id":"test","split":"test","input":"Increment 12","check":{"type":"exact","expected":"13"}}
    ]});
    // Prepare both candidates before the workers start: a running learning
    // worker would otherwise claim these due policies first.
    let mut candidates = Vec::new();
    for (name, binary) in [("slow-evaluation", &slow), ("fast-evaluation", &evaluator)] {
        let owner = db.create_human_user(name, "password-123").await.unwrap();
        let target = learning_binding(&database, &owner).await;
        let analyst = learning_binding(&database, &owner).await;
        db.configure_experience(
            &owner.workspace_id,
            &owner.id,
            &target,
            &analyst,
            true,
            None,
        )
        .await
        .unwrap();
        let claim = db.claim_experience().await.unwrap().unwrap();
        let revision = db
            .save_experience_candidate(
                &claim,
                ExperienceReport {
                    digest: "evaluation-candidate",
                    references: &json!([]),
                    analysis: "A reviewed format correction",
                    instruction: Some("Return the number only."),
                    validation: &json!({"review":"passed"}),
                    checkpoint: None,
                    activate: false,
                },
            )
            .await
            .unwrap()
            .unwrap();
        db.release_experience(
            &claim,
            choruz_application::db_service::CheckOutcome::Completed,
        )
        .await
        .unwrap();
        client.execute("UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day' WHERE binding_id=$1", &[&target]).await.unwrap();
        for binding in [&target, &analyst] {
            client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[binding,&json!({"binary_path":binary,"model":"evaluation-fixture"})]).await.unwrap();
        }
        candidates.push((name, owner, target, revision));
    }
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let mut evaluations = Vec::new();
    for (name, owner, target, revision) in candidates {
        let endpoint = format!("/v1/runtime/bindings/{target}/experience/evaluations");
        let (status, queued) = api_json_payload_request(
            app.clone(),
            &owner,
            Method::POST,
            endpoint.clone(),
            json!({"revision_id":revision,"suite":suite}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        evaluations.push((
            owner,
            format!("{endpoint}?id={}", queued["id"].as_str().unwrap()),
        ));
        if name == "slow-evaluation" {
            tokio::time::timeout(Duration::from_secs(30), async {
                while !files.path().join("slow.started").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the slow evaluation must hold its case before the fast one is queued");
        }
    }
    let (fast_owner, fast_endpoint) = &evaluations[1];
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        finished_evaluation(&app, fast_owner, fast_endpoint.clone(), "completed"),
    )
    .await;
    let slow_still_held = !files.path().join("slow.release").exists();
    // Release the owned model process even if the assertion below fails.
    fs::write(files.path().join("slow.release"), "").unwrap();
    assert!(
        outcome.is_ok() && slow_still_held,
        "another binding's evaluation must complete while a slow one is still running"
    );
    let (slow_owner, slow_endpoint) = &evaluations[0];
    finished_evaluation(&app, slow_owner, slow_endpoint.clone(), "completed").await;
}

#[tokio::test]
async fn a_busy_source_defers_and_a_rate_limit_fails_through_the_worker() {
    let database = TestDatabase::create().await;
    let store = choruz_store::EventStore::new(&database.database_url);
    let db = choruz_application::DbService::new(store.clone());
    let owner = db
        .create_human_user("busy-source", "password-123")
        .await
        .unwrap();
    let target = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    let files = tempfile::tempdir().unwrap();
    client
        .execute(
            "UPDATE agent_runtime_bindings SET workspace_path=$2 WHERE id=$1",
            &[&target, &files.path().to_string_lossy().as_ref()],
        )
        .await
        .unwrap();
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    // A second source lives on a remote device that answers its read with a
    // rate limit, sent through the device link's wire encoding.
    let limited = learning_binding(&database, &owner).await;
    client.execute("UPDATE agent_runtime_bindings SET config_json=config_json||'{\"runtime_host_id\":\"rate-limited-device\"}'::jsonb WHERE id=$1", &[&limited]).await.unwrap();
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &limited,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    client
        .execute(
            "UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day' WHERE binding_id=$1",
            &[&limited],
        )
        .await
        .unwrap();
    let links = crate::host_link::HostLinkHub::default();
    let (device, mut incoming) = links.decision_fixture("rate-limited-device");
    let state = crate::ApiState {
        experience_worker: None,
        account_retirement_worker: None,
        app: choruz_application::ChatApp::new(),
        db: db.clone(),
        runtime,
        session: PgSessionStore::new(&database.database_url),
        event_store: store.clone(),
        attachments: crate::attachments::AttachmentStore::new(files.path(), store),
        auth: LocalAuthConfig::from_env(),
        sync_wakeups: crate::wakeup::WakeupHub::spawn(
            database.database_url.clone(),
            crate::wakeup::SYNC_CHANNEL,
        ),
        worker_wakeups: crate::wakeup::WakeupHub::spawn(
            database.database_url.clone(),
            crate::wakeup::WORKER_CHANNEL,
        ),
        remote_control_bridges: crate::remote_control_bridge::RemoteControlBridgeHub::new().0,
        local_host: crate::host_runtime::LocalHost::new(),
        host_links: links,
        online: crate::online_groups::OnlineHub::spawn(db.clone()),
    };
    // Record the source session as mid-turn through the journal wire contract,
    // for exactly the specs the worker will read.
    let binding = crate::handlers_terminals::authorize_terminal_binding(&state, &owner, &target)
        .await
        .map_err(|error| error.0)
        .unwrap();
    for source in
        crate::learning_sources::native_sources(&state, &owner, &owner.workspace_id, &binding)
            .await
            .unwrap()
    {
        let spec = source.spec;
        let key = choruz_host_runtime::session::owner_key(&spec);
        let directory = std::path::Path::new(&spec.workspace_path).join(".choruz/sessions");
        fs::create_dir_all(&directory).unwrap();
        let account = &spec.harness_account;
        fs::write(directory.join(format!("{}-{key}.json", spec.terminal_id)), json!({
            "version":1,
            "owner":{"binding":spec.terminal_id,"driver":spec.driver_type,"workspace":spec.workspace_path,
                "account":account["harness_account_id"],"profile":account["harness_account_profile_kind"],
                "generation":account["terminal_generation"],"host":account["runtime_host_id"],
                "company":account["conversation_workspace_id"]},
            "state":{"owner":key,"session_id":"busy","turn_id":null,"status":"running","items":[],"requests":[],"error":null}
        }).to_string()).unwrap();
    }
    let claim = db.claim_experience().await.unwrap().unwrap();
    crate::experience_worker::process_claim(&state, claim).await;
    let (error, retry_in) = learning_retry(&client, &target).await;
    assert!(
        error.is_none() && (3.0..=6.0).contains(&retry_in),
        "a busy source turn defers within seconds without an error: {error:?} {retry_in}"
    );

    client
        .execute(
            "UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day' WHERE binding_id=$1",
            &[&target],
        )
        .await
        .unwrap();
    make_learning_due(&**client, &limited).await;
    let claim = db.claim_experience().await.unwrap().unwrap();
    assert_eq!(claim.binding_id, limited);
    let answer = async {
        while let Some(frame) = incoming.recv().await {
            let ControllerFrame::Call { id, request } = serde_json::from_str(&frame).unwrap()
            else {
                panic!("expected a device call")
            };
            let LinkRequest::Host { request } = *request else {
                panic!("expected a host request")
            };
            assert!(matches!(request, HostRequest::ExperienceTrace { .. }));
            let wire: choruz_host_runtime::link::LinkError = serde_json::from_value(
                serde_json::to_value(choruz_host_runtime::link::LinkError::from(
                    AppError::RateLimited {
                        retry_after_ms: 5_000,
                    },
                ))
                .unwrap(),
            )
            .unwrap();
            device.settle(&id, Err(wire.into()));
        }
    };
    tokio::select! {
        _ = crate::experience_worker::process_claim(&state, claim) => {}
        _ = answer => panic!("the device link closed before the check finished"),
    }
    let (error, retry_in) = learning_retry(&client, &limited).await;
    assert!(
        error.is_some() && retry_in > 60.0,
        "a host or provider rate limit is a visible failure with the normal retry: {error:?} {retry_in}"
    );
}

/// The policy's visible error and seconds until its next check.
async fn learning_retry(client: &tokio_postgres::Client, binding: &str) -> (Option<String>, f64) {
    let row = client
        .query_one(
            "SELECT last_error, EXTRACT(EPOCH FROM next_check_at-NOW())::float8 FROM experience_policy WHERE binding_id=$1",
            &[&binding],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

/// Wait for a background-work notification naming `worker`.
async fn expect_wakeup(
    wakeups: &mut tokio::sync::broadcast::Receiver<String>,
    worker: &str,
    action: &str,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if wakeups.recv().await.unwrap() == worker {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{action} must wake the {worker} worker"));
}

#[tokio::test]
async fn due_background_work_notifies_its_worker() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let hub = crate::wakeup::WakeupHub::spawn(
        database.database_url.clone(),
        crate::wakeup::WORKER_CHANNEL,
    );
    hub.wait_ready().await.unwrap();
    let mut wakeups = hub.subscribe();
    let owner = db
        .create_human_user("wakeup-owner", "password-123")
        .await
        .unwrap();
    let target = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;

    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    expect_wakeup(&mut wakeups, "learning", "saving learning settings").await;

    let claim = db.claim_experience().await.unwrap().unwrap();
    let revision = db.save_experience_candidate(
        &claim,
        ExperienceReport {
            digest: "wakeup-candidate",
            references: &json!([]),
            analysis: "A reviewed correction",
            instruction: Some("Verify before reporting completion."),
            validation: &json!({"review":"passed","problems":[{"key":"unverified-completion",
                "description":"Completion without verification","episode_ref":"episode-1","evidence":["source-1"]}]}),
            checkpoint: None,
            activate: false,
        },
    )
    .await
    .unwrap()
    .unwrap();
    expect_wakeup(&mut wakeups, "evaluation", "saving a reviewed candidate").await;
    expect_wakeup(&mut wakeups, "behavior", "recording behavior evidence").await;

    db.release_experience(
        &claim,
        choruz_application::db_service::CheckOutcome::Completed,
    )
    .await
    .unwrap();

    let client = RuntimeStore::new(&database.database_url)
        .connect()
        .await
        .unwrap();
    for binding in [&target, &analyst] {
        client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[binding,&json!({"model":"evaluation-fixture"})]).await.unwrap();
    }
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let suite = json!({"name":"Number format","cases":[
        {"id":"train","split":"train","input":"Increment 10","check":{"type":"exact","expected":"11"}},
        {"id":"validation","split":"validation","input":"Increment 11","check":{"type":"exact","expected":"12"}},
        {"id":"test","split":"test","input":"Increment 12","check":{"type":"exact","expected":"13"}}
    ]});
    let (status, queued) = api_json_payload_request(
        app,
        &owner,
        Method::POST,
        format!("/v1/runtime/bindings/{target}/experience/evaluations"),
        json!({"revision_id":revision,"suite":suite}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    expect_wakeup(&mut wakeups, "evaluation", "queueing an evaluation").await;
}

#[tokio::test]
async fn saving_learning_settings_starts_a_check_without_waiting_for_the_poll() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("prompt-learning", "password-123")
        .await
        .unwrap();
    let target = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;
    let client = RuntimeStore::new(&database.database_url)
        .connect()
        .await
        .unwrap();
    // A policy due before the workers start is taken by their first poll;
    // seeing it checked proves that poll ran, so only a wakeup can start the
    // next check before the 15 s interval.
    let sentinel = learning_binding(&database, &owner).await;
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &sentinel,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    wait_until_checked(
        &client,
        &sentinel,
        "the first learning poll takes the due sentinel",
    )
    .await;
    let saved = tokio::time::Instant::now();
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    wait_until_checked(&client, &target, "a newly enabled policy is checked").await;
    assert!(
        saved.elapsed() < Duration::from_secs(5),
        "a newly enabled policy is checked within seconds, not at the next 15 s poll"
    );
    drop(app);
}

async fn wait_until_checked(client: &tokio_postgres::Client, binding: &str, expectation: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let checked: Option<chrono::DateTime<chrono::Utc>> = client
                .query_one(
                    "SELECT checked_at FROM experience_policy WHERE binding_id=$1",
                    &[&binding],
                )
                .await
                .unwrap()
                .get(0);
            if checked.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expectation}"));
}

#[tokio::test]
async fn a_saved_candidate_wakes_evaluation_queueing_and_behavior_preparation() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let client = RuntimeStore::new(&database.database_url)
        .connect()
        .await
        .unwrap();
    // Each policy's learning lease is held, so the running learning worker
    // cannot act; only evaluation queueing and behavior preparation do.
    let sentinel = optimized_policy(&database, &db, &client, "woken-sentinel").await;
    let woken = optimized_policy(&database, &db, &client, "woken-pipeline").await;
    save_reviewed_candidate(&db, &sentinel.2).await;
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    // The sentinel's candidate exists before the workers start, so their first
    // poll handles it; once handled, only a wakeup can act before 15 s.
    wait_for_woken_pipeline(
        &client,
        &sentinel.1,
        Duration::from_secs(30),
        "the workers' first poll handles the sentinel",
    )
    .await;
    save_reviewed_candidate(&db, &woken.2).await;
    wait_for_woken_pipeline(
        &client,
        &woken.1,
        Duration::from_secs(5),
        "a reviewed candidate is queued for evaluation and its behavior evidence is prepared within seconds, not at the next 15 s poll",
    )
    .await;
    drop(app);
}

/// An owner, target and held learning claim with automatic optimization.
async fn optimized_policy(
    database: &TestDatabase,
    db: &choruz_application::DbService,
    client: &tokio_postgres::Client,
    name: &str,
) -> (
    choruz_domain::Principal,
    String,
    choruz_application::db_service::ExperienceClaim,
) {
    let owner = db.create_human_user(name, "password-123").await.unwrap();
    let target = learning_binding(database, &owner).await;
    let analyst = learning_binding(database, &owner).await;
    for binding in [&target, &analyst] {
        client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[binding,&json!({"model":"evaluation-fixture"})]).await.unwrap();
    }
    let settings: choruz_evaluation::optimization::OptimizationSettings =
        serde_json::from_value(json!({"suite":{"name":"Measured formatting","cases":[
            {"id":"training","split":"train","input":"Increment 10","check":{"type":"exact","expected":"11"}},
            {"id":"validation","split":"validation","input":"Increment 20","check":{"type":"exact","expected":"21"}},
            {"id":"test","split":"test","input":"Increment 30","check":{"type":"exact","expected":"31"}}
        ]},"config":{"max_metric_calls":4,"max_proposals":1,"minibatch_size":1,"seed":7,"merge":false},"auto_apply":false}))
        .unwrap();
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        true,
        Some(&settings),
    )
    .await
    .unwrap();
    let claim = db.claim_experience().await.unwrap().unwrap();
    assert_eq!(claim.binding_id, target);
    (owner, target, claim)
}

async fn save_reviewed_candidate(
    db: &choruz_application::DbService,
    claim: &choruz_application::db_service::ExperienceClaim,
) {
    db.save_experience_candidate(
        claim,
        ExperienceReport {
            digest: "woken-candidate",
            references: &json!([]),
            analysis: "A reviewed correction",
            instruction: Some("Verify before reporting completion."),
            validation: &json!({"review":"passed","problems":[{"key":"unverified-completion",
                "description":"Completion without verification","episode_ref":"episode-1","evidence":["source-1"]}]}),
            checkpoint: None,
            activate: false,
        },
    )
    .await
    .unwrap()
    .unwrap();
}

/// The binding has an automatic evaluation queued and no behavior event
/// still awaiting preparation.
async fn wait_for_woken_pipeline(
    client: &tokio_postgres::Client,
    binding: &str,
    within: Duration,
    expectation: &str,
) {
    tokio::time::timeout(within, async {
        loop {
            let row = client
                .query_one(
                    "SELECT (SELECT COUNT(*) FROM experience_evaluation WHERE binding_id=$1 AND automatic),
                            (SELECT COUNT(*) FROM experience_behavior_event WHERE binding_id=$1 AND publication_state='preparing'),
                            (SELECT COUNT(*) FROM experience_behavior_event WHERE binding_id=$1)",
                    &[&binding],
                )
                .await
                .unwrap();
            let (queued, preparing, events): (i64, i64, i64) = (row.get(0), row.get(1), row.get(2));
            if queued > 0 && events > 0 && preparing == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expectation}"));
}
