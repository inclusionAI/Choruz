//! Assembled remote execution; only provider inference and external CLIs are fixtures.
use super::*;
use choruz_host_runtime::{
    HostRequest,
    link::{ControllerFrame, LinkRequest},
};
use std::{os::unix::fs::PermissionsExt, time::Duration};

const ACCOUNT: &str = "12345678-1234-1234-1234-123456789abc";

#[tokio::test]
async fn assisted_browser_uses_remote_profile_and_cancels_active_child() {
    let database = TestDatabase::create().await;
    let store = choruz_store::EventStore::new(&database.database_url);
    let db = choruz_application::DbService::new(store.clone());
    let owner = db
        .create_human_user("browser-handoff", "test-password-123")
        .await
        .unwrap();
    let binding = learning_binding(&database, &owner).await;
    let files = tempfile::tempdir().unwrap();
    let a = files.path().join("device-a");
    let b = files.path().join("device-b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(b.join(".local/bin")).unwrap();
    std::fs::create_dir_all(b.join(format!("accounts/{ACCOUNT}/codex"))).unwrap();
    let cli = b.join("codex-fixture");
    std::fs::write(&cli, r#"#!/usr/bin/env python3
import os, json, sys, time
from pathlib import Path
home = Path(os.environ['HOME'])
prompt = sys.stdin.read()
assert '--sandbox' in sys.argv and 'read-only' in sys.argv
assert '--ignore-user-config' in sys.argv and '--ephemeral' in sys.argv
if '"requests"' in prompt:
    assert 'Write the title' in prompt
    values = {'title':'Generated draft'}
    if os.environ['BROWSER_CANCEL'] == 'invalid': values['unauthorized'] = 'Extra field'
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':json.dumps(values)}}))
    sys.exit(0)
assert 'Save draft' in prompt and '@e1' in prompt
(home/'profile.json').write_text(json.dumps({'profile':os.environ['CODEX_HOME'],'pid':os.getpid(),'args':sys.argv}))
if os.environ['BROWSER_CANCEL'] in ['1','external']:
    time.sleep(60)
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':'{"element_id":"@e1"}'}}))
"#).unwrap();
    let bsk = b.join(".local/bin/bsk");
    std::fs::write(&bsk, r#"#!/usr/bin/env python3
import os, sys, json
from pathlib import Path
h = Path(os.environ['HOME']); a = sys.argv[1:]
if a[:2] == ['session','start']:
    assert a[a.index('--browser')+1] == 'device-b-browser'
    (h/'session-open').write_text('open'); out={'session_id':'owned-b'}
elif a[:2] == ['session','stop']:
    (h/'session-open').unlink(); (h/'session-closed').write_text('closed'); out={'stopped':['owned-b']}
elif a[0] == 'navigate': out={'tab_id':1}
elif a[:2] == ['tab','list']: out={'tabs':[{'tab_id':1,'url':'https://example.org/'}]}
elif a[0] == 'observe': out={'tab_id':1,'truncated':False,'text':'@e1 button "Save draft"\n@e2 textbox "Title"\n@e3 combobox "Category"\n'+('Draft saved' if (h/'mutation').exists() else 'Editing')}
elif a[0] == 'fill':
    assert a[1]=='@e2' and a[a.index('--value')+1]=='Generated draft'; (h/'title').write_text('Generated draft'); out={'ok':True}
elif a[0] == 'select':
    assert a[1]=='@e3' and a[a.index('--value')+1]=='work'; (h/'category').write_text('work'); out={'ok':True}
elif a[0] == 'click':
    assert (h/'title').read_text()=='Generated draft' and (h/'category').read_text()=='work'
    assert a[1]=='@e1'; (h/'mutation').write_text('saved by B'); out={'ok':True}
else: raise RuntimeError(a)
print(json.dumps(out))
"#).unwrap();
    for path in [&cli, &bsk] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    client.execute("INSERT INTO company(id,name,slug,owner_id) VALUES($1,'Browser test',$1,$2) ON CONFLICT(id) DO NOTHING", &[&owner.workspace_id,&owner.id]).await.unwrap();
    client.execute("INSERT INTO runtime_host(id,company_id,name,token_hash,status) VALUES('browser-device-b',$1,'Device B','fixture-only','online')", &[&owner.workspace_id]).await.unwrap();
    client.execute("INSERT INTO harness_account(id,company_id,runtime_host_id,driver_type,name,profile_kind,status,models_json) VALUES($1,$2,'browser-device-b','codex_terminal','B account','isolated','active','[{\"id\":\"selected-model\"}]')", &[&ACCOUNT,&owner.workspace_id]).await.unwrap();
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',workspace_path=$2,config_json=$3 WHERE id=$1",
        &[&binding, &b.to_string_lossy().as_ref(), &json!({"runtime_host_id":"browser-device-b","binary_path":cli,"model":"selected-model","harness_account_id":ACCOUNT,"harness_account_profile_kind":"isolated"})]).await.unwrap();
    let links = crate::host_link::HostLinkHub::default();
    let (link, mut incoming) = links.decision_fixture("browser-device-b");
    let chat = choruz_application::ChatApp::new();
    let state = crate::ApiState {
        experience_worker: None,
        app: chat.clone(),
        db: db.clone(),
        runtime,
        session: PgSessionStore::new(&database.database_url),
        event_store: store.clone(),
        attachments: crate::attachments::AttachmentStore::new(&a, store),
        auth: LocalAuthConfig::from_env(),
        sync_wakeups: crate::sync_wakeup::SyncWakeupHub::spawn(database.database_url.clone()),
        remote_control_bridges: crate::remote_control_bridge::RemoteControlBridgeHub::new().0,
        local_host: crate::host_runtime::LocalHost::new(),
        host_links: links,
        online: crate::online_groups::OnlineHub::spawn(chat, db.clone()),
    };
    let app = axum::Router::new()
        .route(
            "/v1/runtime/bindings/{binding_id}/browser-workflows/{run_id}",
            axum::routing::post(crate::handlers_browser_automation::run)
                .get(crate::handlers_browser_workflows::get),
        )
        .route(
            "/v1/runtime/bindings/{binding_id}/browser-automation",
            axum::routing::put(crate::handlers_browser_automation::configure),
        )
        .with_state(state.clone());
    let execution = json!({"browser":"device-b-browser","url":"https://example.org/","model":"fixture","minimum_confidence":0.9,"assist_with_agent":true,
        "workflow":{"name":"Save","allowed_urls":["https://example.org/"],"steps":[
            {"goal":"Fill title","operation":"type_text","labels":["textbox \"Title\""],"value_key":"title"},
            {"goal":"Choose category","operation":"select","labels":["combobox \"Category\""],"value_key":"category"},
            {"goal":"Save draft","operation":"click","labels":["button \"Save draft\""],"value_key":null}]},
        "values":{"category":"work"},"text_requests":{"title":"Write the title"},"expected_text":["Draft saved"]});
    let analyst = learning_binding(&database, &owner).await;
    let agent = db
        .get_principal(
            &state
                .runtime
                .get_binding(&binding)
                .await
                .unwrap()
                .agent_principal_id,
        )
        .await
        .unwrap();
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &binding,
        &analyst,
        true,
        None,
    )
    .await
    .unwrap();
    let origin = db
        .create_group(choruz_application::CreateGroupRequest {
            actor_id: owner.id.clone(),
            name: "Browser source group".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![agent.id.clone()],
            workspace_id: Some(owner.workspace_id.clone()),
        })
        .await
        .unwrap();
    assert_ne!(
        origin.id,
        state
            .runtime
            .get_binding(&binding)
            .await
            .unwrap()
            .conversation_id
    );
    client
        .execute(
            "UPDATE experience_policy SET decision_settings=$2 WHERE binding_id=$1",
            &[&binding, &json!({"model":"fixture","minimum_confidence":0.9,"classify":true,"supervise":true,"assist_turns":true,"builder_binding_id":analyst})],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO experience_revision(id,binding_id,workspace_id,policy_generation,source_digest,source_references,analysis,instruction,disposition,validation) SELECT 'browser-revision',binding_id,workspace_id,generation,'browser-source','[]','Browser draft','Review before execution','candidate',$2 FROM experience_policy WHERE binding_id=$1", &[&binding,&json!({"program_trial":{"workflow":execution["workflow"],"model":"fixture"}})]).await.unwrap();
    let automation_endpoint = format!("/v1/runtime/bindings/{binding}/browser-automation");
    let permission = json!({"settings":{"browser":"device-b-browser","allowed_urls":["https://example.org/"],"scope":"Save a work draft"}});
    assert_eq!(
        api_json_payload_request(
            app.clone(),
            &agent,
            Method::PUT,
            automation_endpoint.clone(),
            permission.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api_json_payload_request(
            app.clone(),
            &owner,
            Method::PUT,
            automation_endpoint.clone(),
            permission.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    for run in ["complete", "reuse", "reuse-cancel", "cancel", "invalid"] {
        let cancel = run == "cancel" || run == "reuse-cancel";
        let invalid = run == "invalid";
        for name in [
            "mutation",
            "session-closed",
            "profile.json",
            "title",
            "category",
        ] {
            let path = b.join(name);
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        let revision = if run == "complete" || run == "reuse" {
            "browser-revision".to_string()
        } else {
            format!("browser-{run}")
        };
        if revision != "browser-revision" {
            client.execute("INSERT INTO experience_revision(id,binding_id,workspace_id,policy_generation,source_digest,source_references,analysis,instruction,disposition,validation) SELECT $2,binding_id,workspace_id,policy_generation,$2,source_references,analysis,instruction,disposition,validation FROM experience_revision WHERE id='browser-revision' AND binding_id=$1", &[&binding,&revision]).await.unwrap();
        }
        if run == "cancel" {
            assert_eq!(
                api_json_payload_request(
                    app.clone(),
                    &owner,
                    Method::PUT,
                    automation_endpoint.clone(),
                    permission.clone()
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        let endpoint = format!("/v1/runtime/bindings/{binding}/browser-workflows/{run}");
        let body = json!({"conversation_id":origin.id,"revision_id":revision,"task":"Save a work draft","url":"https://example.org/","values":{"category":"work"},"text_requests":{"title":"Write the title"},"expected_text":["Draft saved"]});
        let call = api_json_payload_request(
            app.clone(),
            &agent,
            Method::POST,
            endpoint.clone(),
            body.clone(),
        );
        let (status, _) = {
            let matching = async {
                let frame = tokio::time::timeout(Duration::from_secs(10), incoming.recv())
                    .await
                    .unwrap()
                    .unwrap();
                let ControllerFrame::Call { id, request } = serde_json::from_str(&frame).unwrap()
                else {
                    panic!("matching call")
                };
                let LinkRequest::Host {
                    request: HostRequest::Decision { request },
                } = *request
                else {
                    panic!("matching decision")
                };
                let choruz_decision::task::Job::Program { state: input, .. } = &request.job else {
                    panic!("program")
                };
                assert_eq!(input["task"], "Save a work draft");
                link.settle(&id, Ok(request.run(&MatchTask("yes")).await.unwrap()));
            };
            tokio::join!(call, matching).0
        };
        assert!(status.is_success(), "{status}");
        let frame = tokio::time::timeout(Duration::from_secs(10), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        let ControllerFrame::Call { id, request } = serde_json::from_str(&frame).unwrap() else {
            panic!("call");
        };
        let LinkRequest::Host { request } = *request else {
            panic!("host");
        };
        let HostRequest::BrowserWorkflow {
            assistant: Some(ref spec),
            ..
        } = request
        else {
            panic!("assisted browser");
        };
        assert_eq!(spec.workspace_path, b.to_string_lossy());
        assert_eq!(spec.harness_account["harness_account_id"], ACCOUNT);
        let input = b.join("request.json");
        std::fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "tests::experience::browser_handoff::device_child",
                "--nocapture",
            ])
            .env("HOME", &b)
            .env("CHORUZ_HARNESS_ACCOUNT_ROOT", b.join("accounts"))
            .env("BROWSER_CHILD_REQUEST", &input)
            .env(
                "BROWSER_CANCEL",
                if run == "reuse-cancel" {
                    "external"
                } else if cancel {
                    "1"
                } else {
                    run
                },
            )
            .kill_on_drop(true);
        let output = if run == "reuse-cancel" {
            let revoke = async {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while !b.join("profile.json").exists() {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap();
                let deletion = api_json_payload_request(
                    app.clone(),
                    &owner,
                    Method::PUT,
                    automation_endpoint.clone(),
                    json!({"settings":null}),
                );
                let deliver = async {
                    let frame = tokio::time::timeout(Duration::from_secs(10), incoming.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    let ControllerFrame::Call {
                        id: cancel_call,
                        request,
                    } = serde_json::from_str(&frame).unwrap()
                    else {
                        panic!("cancel call")
                    };
                    let LinkRequest::Host {
                        request: HostRequest::CancelBrowserWorkflow { id: device_run },
                    } = *request
                    else {
                        panic!("cancel request")
                    };
                    assert_eq!(
                        device_run,
                        crate::handlers_browser_workflows::device_id(
                            &owner.workspace_id,
                            &binding,
                            run
                        )
                    );
                    std::fs::write(b.join("cancel-signal"), device_run).unwrap();
                    link.settle(&cancel_call, Ok(json!({})));
                };
                assert_eq!(tokio::join!(deletion, deliver).0.0, StatusCode::OK);
            };
            tokio::join!(child.output(), revoke).0.unwrap()
        } else {
            child.output().await.unwrap()
        };
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value =
            serde_json::from_slice(&std::fs::read(b.join("result.json")).unwrap()).unwrap();
        link.settle(
            &id,
            if cancel || invalid {
                assert!(result["error"].as_str().unwrap().contains(if cancel {
                    "cancelled"
                } else {
                    "approved fields"
                }));
                Err(AppError::Internal(result["error"].as_str().unwrap().into()))
            } else {
                Ok(result["report"].clone())
            },
        );
        let receipt = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let (_, receipt) = api_json_payload_request(
                    app.clone(),
                    &owner,
                    Method::GET,
                    endpoint.clone(),
                    json!({}),
                )
                .await;
                if receipt["status"] == "finished" || receipt["status"] == "cancelled" {
                    break receipt;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        if run == "reuse-cancel" {
            assert_eq!(receipt["status"], "cancelled");
        } else if cancel || invalid {
            assert!(receipt["result"]["error"].is_string());
        } else {
            assert_eq!(receipt["result"]["report"]["checks_matched"], true);
            assert_eq!(
                receipt["result"]["report"]["generated_inputs"],
                json!(["title"])
            );
            assert_eq!(receipt["result"]["report"]["completed_steps"], 3);
            assert!(receipt["result"]["report"]["elapsed_ms"].as_u64().unwrap() > 0);
            assert_eq!(
                std::fs::read_to_string(b.join("mutation")).unwrap(),
                "saved by B"
            );
        }
        if invalid {
            assert!(!b.join("session-open").exists());
            assert!(!b.join("session-closed").exists());
            assert!(!b.join("title").exists());
            assert!(!b.join("mutation").exists());
            crate::browser_completion_worker::deliver(&state)
                .await
                .unwrap();
            let command=client.query_one("SELECT conversation_id,session_key,prompt FROM agent_commands WHERE agent_id=$1 AND metadata->>'browser_run_id'=$2", &[&agent.id,&run]).await.unwrap();
            assert_eq!(command.get::<_, String>(0), origin.id);
            assert_eq!(
                command.get::<_, String>(1),
                format!("{}:{}", agent.id, origin.id)
            );
            assert!(
                command
                    .get::<_, String>(2)
                    .contains("Execution unavailable or interrupted")
            );
            let retry = format!("/v1/runtime/bindings/{binding}/browser-workflows/retry-invalid");
            assert_eq!(
                api_json_payload_request(app.clone(), &agent, Method::POST, retry, body.clone())
                    .await
                    .0,
                StatusCode::CONFLICT
            );
            assert!(
                incoming.try_recv().is_err(),
                "failed revision must not call the device again"
            );
            continue;
        }
        let profile: Value =
            serde_json::from_slice(&std::fs::read(b.join("profile.json")).unwrap()).unwrap();
        assert_eq!(
            profile["profile"],
            b.join(format!("accounts/{ACCOUNT}/codex"))
                .to_string_lossy()
                .as_ref()
        );
        assert!(
            profile["args"]
                .as_array()
                .unwrap()
                .contains(&json!("selected-model"))
        );
        assert!(!a.join("profile.json").exists());
        assert!(!a.join("mutation").exists());
        assert!(b.join("session-closed").exists());
        assert!(!b.join("session-open").exists());
        if run == "complete" || run == "reuse" {
            crate::browser_completion_worker::deliver(&state)
                .await
                .unwrap();
            // Replaying after a worker restart must not queue a second continuation.
            client.execute("UPDATE browser_workflow_run SET notified=FALSE WHERE workspace_id=$1 AND binding_id=$2 AND id=$3", &[&owner.workspace_id,&binding,&run]).await.unwrap();
            crate::browser_completion_worker::deliver(&state)
                .await
                .unwrap();
            let count:i64=client.query_one("SELECT COUNT(*) FROM agent_commands WHERE agent_id=$1 AND metadata->>'browser_run_id'=$2", &[&agent.id,&run]).await.unwrap().get(0);
            assert_eq!(count, 1);
            let command=client.query_one("SELECT conversation_id,session_key FROM agent_commands WHERE agent_id=$1 AND metadata->>'browser_run_id'=$2", &[&agent.id,&run]).await.unwrap();
            assert_eq!(command.get::<_, String>(0), origin.id);
            assert_eq!(
                command.get::<_, String>(1),
                format!("{}:{}", agent.id, origin.id)
            );
            let (_, receipt) = api_json_payload_request(
                app.clone(),
                &agent,
                Method::POST,
                endpoint.clone(),
                body.clone(),
            )
            .await;
            assert_eq!(receipt["status"], "finished");
            assert!(
                incoming.try_recv().is_err(),
                "duplicate must not call the device"
            );
            let catalog = db
                .automatic_browser_catalog(&owner.workspace_id, &binding)
                .await
                .unwrap();
            assert!(
                catalog
                    .iter()
                    .any(|item| item["revision_id"] == revision && item["validated"] == true)
            );
        }
    }
    // Pause after durable admission, acknowledge disable, then release launch.
    client.execute("INSERT INTO browser_workflow_run(workspace_id,binding_id,id,actor_id,request_hash,status,binding_fingerprint,revision_id,automation_generation) SELECT workspace_id,binding_id,'before-dispatch',$2,'fixture','running',binding_fingerprint,'new-revision',generation FROM browser_automation WHERE binding_id=$1", &[&binding,&agent.id]).await.unwrap();
    let disable = api_json_payload_request(
        app.clone(),
        &owner,
        Method::PUT,
        automation_endpoint.clone(),
        json!({"settings":null}),
    );
    let acknowledge = async {
        let frame = tokio::time::timeout(Duration::from_secs(10), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        let ControllerFrame::Call { id, request } = serde_json::from_str(&frame).unwrap() else {
            panic!("cancel call")
        };
        let LinkRequest::Host {
            request: HostRequest::CancelBrowserWorkflow { id: cancelled },
        } = *request
        else {
            panic!("cancel before dispatch")
        };
        assert_eq!(
            cancelled,
            crate::handlers_browser_workflows::device_id(
                &owner.workspace_id,
                &binding,
                "before-dispatch"
            )
        );
        link.settle(&id, Ok(json!({"cancel_requested":true})));
    };
    assert_eq!(
        tokio::join!(disable, acknowledge).0.1["device_acknowledged"],
        true
    );
    let bound = state.runtime.get_binding(&binding).await.unwrap();
    crate::handlers_browser_workflows::launch(
        state.clone(),
        crate::host_runtime::RuntimeHost::for_binding(&state, &bound).unwrap(),
        owner.workspace_id.clone(),
        binding.clone(),
        "before-dispatch".into(),
        agent.id.clone(),
        serde_json::from_value(execution.clone()).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert!(
        incoming.try_recv().is_err(),
        "cancelled admission must not dispatch to the device"
    );
    assert_eq!(
        api_json_payload_request(
            app.clone(),
            &owner,
            Method::PUT,
            automation_endpoint,
            permission
        )
        .await
        .0,
        StatusCode::OK
    );
    let policy = db
        .browser_automation(&owner.workspace_id, &binding)
        .await
        .unwrap();
    assert!(
        db.admit_automatic_browser(
            &owner.workspace_id,
            &binding,
            &agent.id,
            "policy-changed",
            "fixture",
            "policy-revision",
            policy["generation"].as_i64().unwrap(),
            policy["learning_generation"].as_i64().unwrap(),
            policy["binding_fingerprint"].as_str().unwrap(),
            Some(&origin.id)
        )
        .await
        .unwrap()
    );
    client
        .execute(
            "UPDATE experience_policy SET generation=generation+1 WHERE binding_id=$1",
            &[&binding],
        )
        .await
        .unwrap();
    assert!(
        db.claim_browser_dispatch(&owner.workspace_id, &binding, "policy-changed")
            .await
            .unwrap()
            .is_none()
    );
    crate::handlers_browser_workflows::launch(
        state.clone(),
        crate::host_runtime::RuntimeHost::for_binding(&state, &bound).unwrap(),
        owner.workspace_id.clone(),
        binding.clone(),
        "policy-changed".into(),
        agent.id.clone(),
        serde_json::from_value(execution.clone()).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert!(
        incoming.try_recv().is_err(),
        "changed learning policy must fence admitted work"
    );
    client.execute("UPDATE browser_workflow_run SET status='cancelled' WHERE binding_id=$1 AND id='policy-changed'", &[&binding]).await.unwrap();
    let policy = db
        .browser_automation(&owner.workspace_id, &binding)
        .await
        .unwrap();
    assert!(
        db.admit_automatic_browser(
            &owner.workspace_id,
            &binding,
            &agent.id,
            "claim-once",
            "fixture",
            "claim-revision",
            policy["generation"].as_i64().unwrap(),
            policy["learning_generation"].as_i64().unwrap(),
            policy["binding_fingerprint"].as_str().unwrap(),
            Some(&origin.id)
        )
        .await
        .unwrap()
    );
    assert!(
        db.claim_browser_dispatch(&owner.workspace_id, &binding, "claim-once")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.claim_browser_dispatch(&owner.workspace_id, &binding, "claim-once")
            .await
            .unwrap()
            .is_none(),
        "restart must not repeat a claimed dispatch"
    );
    // A gateway restart finds expired work and resumes its originating group
    // with uncertainty, never a new device execution.
    client.execute("INSERT INTO browser_workflow_run(workspace_id,binding_id,id,actor_id,request_hash,status,binding_fingerprint,revision_id,automation_generation,conversation_id,expires_at) SELECT workspace_id,binding_id,'expired', $2,'fixture','running',binding_fingerprint,'expired-revision',generation,$3,NOW()-INTERVAL '1 second' FROM browser_automation WHERE binding_id=$1", &[&binding,&agent.id,&origin.id]).await.unwrap();
    assert!(
        db.claim_browser_dispatch(&owner.workspace_id, &binding, "expired")
            .await
            .unwrap()
            .is_none()
    );
    crate::browser_completion_worker::deliver(&state)
        .await
        .unwrap();
    crate::browser_completion_worker::deliver(&state)
        .await
        .unwrap();
    let commands=client.query("SELECT conversation_id,prompt FROM agent_commands WHERE agent_id=$1 AND metadata->>'browser_run_id'='expired'", &[&agent.id]).await.unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].get::<_, String>(0), origin.id);
    assert!(
        commands[0]
            .get::<_, String>(1)
            .contains("outcome_unconfirmed")
    );
    assert!(incoming.try_recv().is_err());
}

struct MatchTask(&'static str);
impl choruz_decision::Provider for MatchTask {
    async fn decide(
        &self,
        request: choruz_decision::Request,
    ) -> choruz_decision::Result<choruz_decision::Response> {
        let answers = request
            .questions
            .into_iter()
            .map(|(key, question)| {
                let choruz_decision::Question::Choice { criteria, .. } = question else {
                    panic!("choice")
                };
                let selected = if criteria.contains_key(self.0) {
                    self.0
                } else {
                    "yes"
                };
                (
                    key,
                    choruz_decision::Answer::Choice {
                        choice: selected.into(),
                        confidence: 1.0,
                        probabilities: criteria
                            .keys()
                            .map(|key| (key.clone(), if key == selected { 1.0 } else { 0.0 }))
                            .collect(),
                    },
                )
            })
            .collect();
        Ok(choruz_decision::Response {
            model: request.model,
            answers,
            usage: choruz_decision::Usage {
                input_tokens: 3,
                output_tokens: 1,
            },
        })
    }
}

#[tokio::test]
async fn device_child() {
    let Ok(input) = std::env::var("BROWSER_CHILD_REQUEST") else {
        return;
    };
    struct Abstain;
    impl choruz_decision::Provider for Abstain {
        async fn decide(
            &self,
            request: choruz_decision::Request,
        ) -> choruz_decision::Result<choruz_decision::Response> {
            let operation = if request.questions.contains_key("type_text") {
                "type_text"
            } else if request.questions.contains_key("select") {
                "select"
            } else {
                "blocked"
            };
            let answers = request
                .questions
                .into_iter()
                .map(|(key, question)| {
                    let choruz_decision::Question::Choice { criteria, .. } = question else {
                        panic!("choice");
                    };
                    let selected = if key == "operation" {
                        operation
                    } else {
                        criteria.keys().find(|key| key.as_str() != "none").unwrap()
                    };
                    (
                        key,
                        choruz_decision::Answer::Choice {
                            choice: selected.into(),
                            confidence: 1.0,
                            probabilities: criteria
                                .keys()
                                .map(|k| (k.clone(), if k == selected { 1.0 } else { 0.0 }))
                                .collect(),
                        },
                    )
                })
                .collect();
            Ok(choruz_decision::Response {
                model: request.model,
                answers,
                usage: choruz_decision::Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }
    }
    let HostRequest::BrowserWorkflow {
        id,
        expires_at,
        request,
        assistant,
    } = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap()
    else {
        panic!("browser request");
    };
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let work = choruz_host_runtime::browser_workflow::run_once(
        id.clone(),
        expires_at,
        request,
        Abstain,
        assistant.map(|s| *s),
    );
    let mode = std::env::var("BROWSER_CANCEL").unwrap();
    let cancel = mode == "1" || mode == "external";
    let result = if cancel {
        let signal = async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while !home.join("profile.json").exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                if mode == "external" {
                    while !home.join("cancel-signal").exists() {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    assert_eq!(
                        std::fs::read_to_string(home.join("cancel-signal")).unwrap(),
                        id
                    );
                }
            })
            .await
            .unwrap();
            choruz_host_runtime::browser_workflow::cancel(&id).unwrap();
        };
        tokio::join!(work, signal).0
    } else {
        work.await
    };
    if std::env::var("BROWSER_CANCEL").unwrap() != "invalid" {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !home.join("session-closed").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    if cancel {
        assert!(result.is_err());
        assert!(!home.join("mutation").exists());
        let profile: Value =
            serde_json::from_slice(&std::fs::read(home.join("profile.json")).unwrap()).unwrap();
        let pid = profile["pid"].as_u64().unwrap().to_string();
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::process::Command::new("kill")
                .args(["-0", &pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(!home.join("mutation").exists());
    }
    let value = match result {
        Ok(report) => json!({"report":report}),
        Err(error) => json!({"error":error.to_string()}),
    };
    std::fs::write(
        home.join("result.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}
