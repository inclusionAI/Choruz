use super::*;
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
    db.release_experience(&claim, None).await.unwrap();
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
