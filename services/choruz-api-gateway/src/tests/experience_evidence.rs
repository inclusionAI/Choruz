use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn behavior_worker_recovers_shared_task_feedback_decisions_and_retained_profile() {
    behavior_evidence(true).await;
}

#[tokio::test]
async fn behavior_worker_recovers_retained_profile_without_foreground_anchor() {
    behavior_evidence(false).await;
}

async fn behavior_evidence(with_feedback: bool) {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("shared-evidence", "password-123")
        .await
        .unwrap();
    let root = learning_binding(&database, &owner).await;
    let child = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    let files = tempfile::tempdir().unwrap();
    let workspace = files.path().join("workspace");
    let old_home = files.path().join("retained-profile");
    let current_home = files.path().join("current-profile");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(old_home.join("sessions")).unwrap();
    fs::create_dir_all(current_home.join("sessions")).unwrap();
    let native = old_home.join("sessions/old.jsonl");
    let header = format!(
        "{}\n",
        json!({"type":"session_meta","payload":{"id":"old-session","cwd":workspace}})
    );
    let trace = format!(
        "{header}{}\n",
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"RETAINED_NATIVE"}]}})
    );
    fs::write(&native, &trace).unwrap();
    let native_ref = format!("old-session:{}", header.len());
    let binary = files.path().join("extractor");
    fs::write(
        files.path().join("expected-markers"),
        serde_json::to_string(&if with_feedback {
            vec!["CHILD_FEEDBACK", "FINITE_RESULT", "RETAINED_NATIVE"]
        } else {
            vec!["RETAINED_NATIVE"]
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(&binary,r#"#!/usr/bin/env python3
import json,sys,pathlib
text=sys.stdin.read()
assert all(marker in text for marker in json.loads((pathlib.Path(__file__).parent/'expected-markers').read_text()))
draft={'title':'Verify before completion','input_background':'A shared task required verification.',
 'expected_behavior':'Run the requested check.','bad_behavior':'The check was skipped.',
 'applicability':'Explicit acceptance checks','tags':['verification'],
 'evidence_summary':'The user correction and finite response agree with the retained native source.'}
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':json.dumps(draft)}}))
"#).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    for id in [&root, &child] {
        client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',workspace_path=$2,config_json=$3 WHERE id=$1",&[id,&workspace.to_string_lossy().as_ref(),&json!({"model":"fixture-model"})]).await.unwrap();
    }
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1",&[&analyst,&json!({"binary_path":binary,"model":"fixture-model"})]).await.unwrap();
    let child_binding = runtime.get_binding(&child).await.unwrap();
    let anchor = json!({"driver_type":"codex_terminal","session_id":"old-session","source":"native_cli","provenance":"terminal_process_captured",
        "binding_id":child,"conversation_id":child_binding.conversation_id,"agent_principal_id":child_binding.agent_principal_id,
        "company_id":owner.workspace_id,"workspace_id":owner.workspace_id,"workspace_path":workspace,
        "native_home_path":old_home,"native_session_path":native,"binding_generation":0,"captured_at":"2026-01-01T00:00:00Z"});
    client.execute("UPDATE agent_runtime_bindings SET config_json=config_json||jsonb_build_object('terminal_session',$2::jsonb) WHERE id=$1",&[&child,&anchor]).await.unwrap();
    // Retention must survive replacement of the foreground anchor and native home.
    client.execute("UPDATE agent_runtime_bindings SET config_json=(config_json-'terminal_session')||$2::jsonb WHERE id=$1",&[&child,&json!({"terminal_generation":1,"harness_account_profile_kind":"isolated","harness_account_profile_directory":current_home})]).await.unwrap();
    db.configure_experience(&owner.workspace_id, &owner.id, &root, &analyst, true, None)
        .await
        .unwrap();
    db.configure_task_profile(&owner.workspace_id, &owner.id, &root, true)
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO experience_task_link(binding_id,profile_binding_id) VALUES($1,$2)",
            &[&child, &root],
        )
        .await
        .unwrap();
    let message = choruz_common::new_id();
    let decision = choruz_common::new_id();
    client.execute("INSERT INTO conversation_events(conversation_id,seq,event_id,event_type,sender_id,content) VALUES($1,1,$2,'message',$3,'CHILD_FEEDBACK'),($1,2,$4,'runtime.decision',$5,'FINITE_RESULT')",&[&child_binding.conversation_id,&message,&owner.id,&decision,&child_binding.agent_principal_id]).await.unwrap();
    let references = if with_feedback {
        json!([
            format!("message:{message}"),
            format!("message:{decision}"),
            native_ref
        ])
    } else {
        json!([native_ref])
    };
    let checkpoint = json!({"feedback":{&child_binding.conversation_id:2},format!("task:{child}:old-session"):{"session":"old-session","offset":trace.len()}});
    let claim = db.claim_experience().await.unwrap().unwrap();
    db.save_experience_candidate(&claim,ExperienceReport {
        digest:"shared-evidence",references:&references,analysis:"Verified source observation",instruction:None,
        validation:&json!({"review":"not_required","problems":[{"key":"verification","description":"The requested check was skipped.","episode_ref":references[0],"evidence":references,"applied_revision_id":null}]}),
        checkpoint:Some(&checkpoint),activate:false,
    }).await.unwrap().unwrap();
    db.release_experience(
        &claim,
        choruz_application::db_service::CheckOutcome::Completed,
    )
    .await
    .unwrap();
    client
        .execute(
            "UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day' WHERE binding_id=$1",
            &[&root],
        )
        .await
        .unwrap();
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let card=tokio::time::timeout(std::time::Duration::from_secs(30),async {
        loop {
            let row=client.query_one("SELECT payload,publication_state,publication_error FROM experience_behavior_event WHERE binding_id=$1 AND problem_key='verification'",&[&root]).await.unwrap();
            if let Some(payload)=row.get::<_,Option<Value>>("payload") {break payload;}
            assert_ne!(row.get::<_,String>("publication_state"),"blocked","{:?}",row.get::<_,Option<String>>("publication_error"));
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("shared source evidence must become a persisted local card");
    assert_eq!(card["problem"]["title"], "Verify before completion");
    let row=client.query_one("SELECT publication_state,public_payload FROM experience_behavior_event WHERE binding_id=$1 AND problem_key='verification'",&[&root]).await.unwrap();
    assert_eq!(row.get::<_, String>("publication_state"), "local");
    assert!(
        row.get::<_, Option<Value>>("public_payload").is_none(),
        "source reuse grants no publication consent"
    );
    drop(app);
    // The shared resolver must still reject evidence after membership is removed.
    client
        .execute(
            "UPDATE conversation_member SET removed_at=NOW() WHERE conv_id=$1 AND principal_id=$2",
            &[&child_binding.conversation_id, &owner.id],
        )
        .await
        .unwrap();
    let mut claim = claim;
    claim.source_cursor = checkpoint;
    assert!(
        db.experience_feedback_references(
            &claim,
            &[format!("message:{message}"), format!("message:{decision}")]
        )
        .await
        .unwrap()
        .is_empty()
    );
}
