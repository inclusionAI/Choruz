use super::*;
use choruz_application::db_service::ExperienceReport;

async fn finished_evaluation(
    app: &Router,
    owner: &choruz_domain::Principal,
    endpoint: String,
    expected_status: &str,
) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let (status, response) =
                api_json_request(app.clone(), owner, Method::GET, endpoint.clone()).await;
            assert_eq!(status, StatusCode::OK);
            let report = &response["evaluations"][0];
            if ["completed", "failed", "cancelled"].contains(&report["status"].as_str().unwrap()) {
                assert_eq!(report["status"], expected_status, "{report}");
                break report.clone();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("background evaluation did not finish")
}

#[cfg(unix)]
#[tokio::test]
async fn automatic_optimization_requires_holdout_and_content_review_then_supports_rollback() {
    use std::os::unix::fs::PermissionsExt;
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("measured-owner", "password-123")
        .await
        .unwrap();
    let files = tempfile::tempdir().unwrap();
    let binary = files.path().join("measured-model");
    fs::write(
        &binary,
        include_str!(
            "../../../../crates/choruz-host-runtime/tests/fixtures/experience-evaluator.py"
        ),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let client = RuntimeStore::new(&database.database_url)
        .connect()
        .await
        .unwrap();
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    for expected_status in [
        "applied",
        "no_improvement",
        "review_rejected",
        "failed",
        "not_requested",
        "team_applied",
    ] {
        let team_search = expected_status == "team_applied";
        let expected_status = if team_search {
            "applied"
        } else {
            expected_status
        };
        let target = learning_binding(&database, &owner).await;
        let analyst = learning_binding(&database, &owner).await;
        for binding in [&target, &analyst] {
            client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[binding,&json!({"binary_path":binary,"model":if team_search {"team-evaluation-fixture"} else {"evaluation-fixture"}})]).await.unwrap();
        }
        let endpoint = format!("/v1/runtime/bindings/{target}/experience");
        let settings = json!({"suite":{"name":"Measured formatting","cases":[
            {"id":"training","split":"train","input":"Increment 10","check":{"type":"exact","expected":"11"}},
            {"id":"validation","split":"validation","input":"Increment 20","check":{"type":"exact","expected":"21"}},
            {"id":"test","split":"test","input":"Increment 30","check":{"type":"exact","expected":if expected_status=="no_improvement" {"The answer is 31"} else {"31"}}}
        ]},"config":{"max_metric_calls":if team_search {24} else {4},"max_proposals":if team_search {2} else {1},"minibatch_size":1,"seed":7,"merge":true,"cache_evaluations":false,"evolve_team":team_search,"max_agents":3},"auto_apply":expected_status!="not_requested"});
        let (status, configured) = api_json_payload_request(
            app.clone(),
            &owner,
            Method::PUT,
            endpoint.clone(),
            json!({"enabled":true,"analyst_binding_id":analyst,"optimization_settings":settings}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{configured}");
        assert_eq!(configured["policy"]["optimization_settings"], settings);
        // Seed storage is the analysis boundary; evaluation, review and selection
        // use the real asynchronous worker, HTTP settings and native transport.
        client.execute("UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day',lease_token=NULL,lease_until=NULL WHERE binding_id=$1", &[&target]).await.unwrap();
        let seed = choruz_common::new_id();
        client.execute("INSERT INTO experience_revision(id,binding_id,workspace_id,policy_generation,source_digest,source_references,analysis,instruction,disposition,validation) SELECT $1,binding_id,workspace_id,generation,$1,'[]',$3,'Return the number only.','candidate',$4 FROM experience_policy WHERE binding_id=$2", &[&seed,&target,&match expected_status {"review_rejected"=>"Reject this proposal","failed"=>"Review unavailable",_=>"Use exact numeric answers"},&json!({"review":"passed","addressed_problems":["formatting"],"source_complete":true,"escalation_candidates":if team_search {vec!["formatting"]} else {vec![]}})]).await.unwrap();
        client.execute("INSERT INTO experience_problem(binding_id,workspace_id,problem_key,description) VALUES($1,$2,'formatting','Extra text in numeric answers')", &[&target,&owner.workspace_id]).await.unwrap();
        let run_id = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let (_, response) = api_json_request(
                    app.clone(),
                    &owner,
                    Method::GET,
                    format!("{endpoint}/evaluations"),
                )
                .await;
                if let Some(id) = response["evaluations"][0]["id"].as_str() {
                    break id.to_owned();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        let report = finished_evaluation(
            &app,
            &owner,
            format!("{endpoint}/evaluations?id={run_id}"),
            if expected_status == "failed" {
                "failed"
            } else {
                "completed"
            },
        )
        .await;
        assert_eq!(report["application_status"], expected_status, "{report}");
        assert_eq!(
            report["optimization"]["winner"],
            if team_search { 2 } else { 1 }
        );
        assert_eq!(
            report["final_review_reserved"],
            !["no_improvement", "not_requested"].contains(&expected_status)
        );
        if !team_search {
            assert_eq!(
                report["optimization"]["model_calls_reserved"],
                if ["no_improvement", "not_requested"].contains(&expected_status) {
                    4
                } else {
                    5
                }
            );
        }
        let active = db
            .active_experience(&owner.workspace_id, &target)
            .await
            .unwrap();
        if expected_status == "applied" {
            let (id, instruction) = active.unwrap();
            if team_search {
                let applied = db
                    .experience_for_turn(&owner.workspace_id, &target)
                    .await
                    .unwrap()
                    .unwrap();
                let team = applied.team.unwrap();
                assert_eq!(team.members.len(), 2);
                assert_eq!(team.order, choruz_domain::team::Order::Parallel);
                assert_eq!(team.members[0].prompt, "Derive the requested number.");
                assert_eq!(team.members[1].prompt, "Check the requested format.");
                let observations = report["optimization"]["observations"].as_array().unwrap();
                assert!(observations.iter().any(|row| {
                    row["preflight"]
                        .as_str()
                        .unwrap()
                        .contains("Derive the requested number.")
                }));
            } else {
                assert_eq!(instruction, "Return the number only.");
            }
            assert_eq!(report["applied_revision_id"], id);
            let problem_revision: String = client
                .query_one(
                    "SELECT prompt_revision_id FROM experience_problem WHERE binding_id=$1",
                    &[&target],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(problem_revision, id);
            for selected in [Value::Null, json!(id)] {
                let (status, response) = api_json_payload_request(
                    app.clone(),
                    &owner,
                    Method::PATCH,
                    endpoint.clone(),
                    json!({"revision_id":selected}),
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_eq!(response["policy"]["active_revision_id"], selected);
                assert_eq!(
                    db.active_experience(&owner.workspace_id, &target)
                        .await
                        .unwrap()
                        .is_some(),
                    !selected.is_null()
                );
            }
        } else {
            assert!(active.is_none());
        }
        assert!(
            !db.pending_experience_optimizations()
                .await
                .unwrap()
                .iter()
                .any(|job| job["revision"] == seed),
            "a completed or rejected run must not automatically spend again"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn evaluation_compares_frozen_revisions_without_activating_or_replaying_tasks() {
    use std::os::unix::fs::PermissionsExt;
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("evaluation-owner", "password-123")
        .await
        .unwrap();
    let outsider = db
        .create_human_user("evaluation-outsider", "password-123")
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
    let claim = db.claim_experience().await.unwrap().unwrap();
    let revision = db.save_experience_candidate(&claim, ExperienceReport {
        digest:"evaluation-candidate", references:&json!([]), analysis:"A reviewed format correction",
        instruction:Some("Return the number only."),
        validation:&json!({"review":"passed","preflight_role":{"review":"passed","focus":"Check the requested output format."}}),
        checkpoint:None, activate:false,
    }).await.unwrap().unwrap();
    let optimizer_seed = db
        .save_experience_candidate(
            &claim,
            ExperienceReport {
                digest: "optimizer-seed",
                references: &json!([]),
                analysis: "A seed needing behavioral improvement",
                instruction: Some("Explain the answer."),
                validation: &json!({"review":"passed"}),
                checkpoint: None,
                activate: false,
            },
        )
        .await
        .unwrap()
        .unwrap();
    db.release_experience(&claim, None).await.unwrap();
    let runtime = RuntimeStore::new(&database.database_url);
    let client = runtime.connect().await.unwrap();
    // Upgrade an actual saved reviewer before exercising evaluation and rollback.
    client
        .batch_execute(include_str!(
            "../../../../migrations/V057__execution_teams.sql"
        ))
        .await
        .unwrap();
    client
        .execute(
            "UPDATE experience_policy SET next_check_at=NOW()+INTERVAL '1 day' WHERE binding_id=$1",
            &[&target],
        )
        .await
        .unwrap();
    let files = tempfile::tempdir().unwrap();
    let binary = files.path().join("evaluation-model");
    fs::write(
        &binary,
        include_str!(
            "../../../../crates/choruz-host-runtime/tests/fixtures/experience-evaluator.py"
        ),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[&target,&json!({"binary_path":binary,"model":"evaluation-fixture"})]).await.unwrap();
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[&analyst,&json!({"binary_path":binary,"model":"evaluation-fixture"})]).await.unwrap();
    let endpoint = format!("/v1/runtime/bindings/{target}/experience/evaluations");
    let suite = json!({"name":"Number format","cases":[
        {"id":"train","split":"train","input":"Increment 1","check":{"type":"exact","expected":"2"}},
        {"id":"validation","split":"validation","input":"Increment 2","check":{"type":"exact","expected":"3"}},
        {"id":"test","split":"test","input":"Increment 3","check":{"type":"exact","expected":"4"}}
    ]});
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let (status, _) = api_json_request(app.clone(), &outsider, Method::GET, endpoint.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, queued) = api_json_payload_request(
        app.clone(),
        &owner,
        Method::POST,
        endpoint.clone(),
        json!({"revision_id":revision,"suite":suite}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    let (status, _) = api_json_payload_request(
        app.clone(),
        &owner,
        Method::POST,
        endpoint.clone(),
        json!({"revision_id":revision,"suite":suite}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let detail_endpoint = format!("{endpoint}?id={}", queued["id"].as_str().unwrap());
    let report = finished_evaluation(&app, &owner, detail_endpoint, "completed").await;
    assert_eq!(report["suite"], suite);
    assert_eq!(report["candidates"][1]["revision_id"], revision);
    assert!(report.get("lease_token").is_none());
    let results = report["results"].as_array().unwrap();
    assert_eq!(results.len(), 6);
    for (index, result) in results.iter().enumerate() {
        assert_eq!(
            result["score"],
            if index % 2 == 0 {
                json!(0.0)
            } else {
                json!(1.0)
            }
        );
        assert_eq!(result["case_id"], suite["cases"][index / 2]["id"]);
    }
    let search_suite = json!({"name":"Independent format tasks","cases":[
        {"id":"train-a","split":"train","input":"Increment 10","check":{"type":"exact","expected":"11"}},
        {"id":"train-b","split":"train","input":"Increment 11","check":{"type":"exact","expected":"12"}},
        {"id":"validation","split":"validation","input":"Increment 20","check":{"type":"exact","expected":"21"}},
        {"id":"test","split":"test","input":"Increment 30","check":{"type":"exact","expected":"31"}}
    ]});
    let (status, queued) = api_json_payload_request(app.clone(), &owner, Method::POST, endpoint.clone(), json!({
        "revision_id":optimizer_seed,"suite":search_suite,
        "optimization":{"max_metric_calls":32,"max_proposals":3,"minibatch_size":1,"seed":7,"merge":true,"cache_evaluations":true,"evolve_team":true}
    })).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    let optimized = finished_evaluation(
        &app,
        &owner,
        format!("{endpoint}?id={}", queued["id"].as_str().unwrap()),
        "completed",
    )
    .await;
    let search = &optimized["optimization"];
    assert_eq!(
        search["config"]["evolve_team"], false,
        "consent without reviewed recurrence must stay prompt-only"
    );
    let winner = search["winner"].as_u64().unwrap() as usize;
    assert!(
        winner >= 2,
        "the optimizer must produce and evaluate a new candidate"
    );
    assert_eq!(
        search["candidates"][winner]["guidance"]["instruction"],
        "Return the number only."
    );
    assert_eq!(search["candidates"][winner]["origin"], "reflection");
    assert!(search["proposal_calls"].as_u64().unwrap() > 0);
    let observations = search["observations"].as_array().unwrap();
    assert_eq!(
        observations.len() as u64,
        search["metric_calls"].as_u64().unwrap()
    );
    assert!(
        observations
            .iter()
            .any(|o| o["candidate"] == winner && o["case"] == 3 && o["score"] == 1.0)
    );
    assert!(
        observations
            .iter()
            .any(|o| o["candidate"] == 0 && o["case"] == 3 && o["score"] == 0.0)
    );
    assert!(
        db.active_experience(&owner.workspace_id, &target)
            .await
            .unwrap()
            .is_none()
    );
    drop(app);

    // Queue with the HTTP run's frozen context while the worker is stopped, then
    // change the analyst before restart. This avoids a timing race with claim.
    let fenced = db
        .queue_experience_evaluation(
            &owner.workspace_id,
            &owner.id,
            &target,
            &optimizer_seed,
            &serde_json::from_value(search_suite).unwrap(),
            &choruz_application::db_service::EvaluationContext {
                automatic_generation: None,
                fingerprint: optimized["context_fingerprint"].as_str().unwrap().into(),
                optimization: Some(choruz_application::db_service::OptimizationSetup {
                    config: serde_json::from_value(search["config"].clone()).unwrap(),
                    analyst_binding_id: analyst.clone(),
                    analyst_fingerprint: optimized["analyst_fingerprint"].as_str().unwrap().into(),
                }),
            },
        )
        .await
        .unwrap();
    client.execute("UPDATE agent_runtime_bindings SET config_json=jsonb_set(config_json,'{model}','\"changed-model\"') WHERE id=$1", &[&analyst]).await.unwrap();
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let fenced_report =
        finished_evaluation(&app, &owner, format!("{endpoint}?id={fenced}"), "failed").await;
    assert_eq!(fenced_report["error_code"], "context_changed");
    assert_eq!(fenced_report["optimization"]["observations"], json!([]));
    assert_eq!(fenced_report["next_case"], 1);
    assert_eq!(fenced_report["results"][0]["status"], "failed");
    drop(app);

    // Recovery uses durable claims, never a second charge for an uncertain call.
    let suite = serde_json::from_value(suite).unwrap();
    let id = db
        .queue_experience_evaluation(
            &owner.workspace_id,
            &owner.id,
            &target,
            &revision,
            &suite,
            &choruz_application::db_service::EvaluationContext {
                automatic_generation: None,
                fingerprint: "context".into(),
                optimization: None,
            },
        )
        .await
        .unwrap();
    let claimed = db.claim_experience_evaluation().await.unwrap().unwrap();
    assert_eq!(claimed.id, id);
    assert!(db.claim_experience_evaluation().await.unwrap().is_none());
    client
        .execute(
            "UPDATE experience_evaluation SET lease_until=NOW()-INTERVAL '1 second' WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap();
    assert!(db.claim_experience_evaluation().await.unwrap().is_none());
    assert!(
        !db.finish_experience_evaluation_case(&claimed, &json!({"score":1}), None)
            .await
            .unwrap()
    );
    let status: String = client
        .query_one(
            "SELECT status FROM experience_evaluation WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "failed");

    let id = db
        .queue_experience_evaluation(
            &owner.workspace_id,
            &owner.id,
            &target,
            &revision,
            &suite,
            &choruz_application::db_service::EvaluationContext {
                automatic_generation: None,
                fingerprint: "context".into(),
                optimization: None,
            },
        )
        .await
        .unwrap();
    let claimed = db.claim_experience_evaluation().await.unwrap().unwrap();
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(
        !db.finish_experience_evaluation_case(&claimed, &json!({"score":1}), None)
            .await
            .unwrap()
    );
    assert!(db.claim_experience_evaluation().await.unwrap().is_none());
    let status: String = client
        .query_one(
            "SELECT status FROM experience_evaluation WHERE id=$1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "cancelled");
}

#[cfg(unix)]
#[tokio::test]
async fn enabling_learning_runs_native_source_through_background_analyst_and_persists_revision() {
    background_learning_cross_window_marker(false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn background_learning_verifies_marker_body_even_when_its_reference_was_retained() {
    background_learning_cross_window_marker(true).await;
}

#[cfg(unix)]
async fn background_learning_cross_window_marker(retain_marker: bool) {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("background-learner", "password-123")
        .await
        .unwrap();
    let target = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;
    let files = tempfile::tempdir().unwrap();
    let workspace = files.path().join("source-workspace");
    let account = files.path().join("source-account");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(account.join("sessions")).unwrap();
    let native = account.join("sessions/source.jsonl");
    fs::write(&native, format!("{}\n{}\n", json!({"type":"session_meta","payload":{"id":"native-source","cwd":workspace}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Objective opener: run the required check."}]}}))).unwrap();
    // The opener crosses a no-change window with no cited evidence. The failure
    // also precedes the last window, retaining the pending-problem coverage.
    let mut transcript = fs::OpenOptions::new().append(true).open(&native).unwrap();
    for index in 0..6 {
        if index == 3 {
            writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Your completion claim was wrong: the required check was never run. Verify before claiming completion."}]}})).unwrap();
            writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Shared completion claim was wrong too: the separate shared report's check was never run."}]}})).unwrap();
        }
        writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"padding".repeat(8500)}]}})).unwrap();
    }
    drop(transcript);
    let cli = files.path().join("analyst");
    fs::write(
        &cli,
        include_str!("../../../../crates/choruz-host-runtime/tests/fixtures/experience-analyst.py"),
    )
    .unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = RuntimeStore::new(&database.database_url);
    let binding = runtime.get_binding(&target).await.unwrap();
    let anchor = json!({"driver_type":"codex_terminal","session_id":"native-source","source":"native_cli","provenance":"terminal_process_captured",
        "binding_id":target,"conversation_id":binding.conversation_id,"agent_principal_id":binding.agent_principal_id,
        "company_id":owner.workspace_id,"workspace_id":owner.workspace_id,"workspace_path":workspace,
        "native_home_path":account,"native_session_path":native,"binding_generation":0,"captured_at":"2026-01-01T00:00:00Z"});
    let client = runtime.connect().await.unwrap();
    let shared_opening = choruz_common::new_id();
    client.execute("INSERT INTO conversation_events(conversation_id,seq,event_id,event_type,sender_id,content) VALUES($1,1,$2,'message',$3,'Shared objective opener: verify the separate shared report.')",
        &[&binding.conversation_id,&shared_opening,&owner.id]).await.unwrap();
    let shared_ref = format!("message:{shared_opening}");
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',workspace_path=$2,config_json=$3 WHERE id=$1",
        &[&target,&workspace.to_string_lossy().as_ref(),&json!({"terminal_session":anchor,"binary_path":cli})]).await.unwrap();
    client.execute("UPDATE agent_runtime_bindings SET driver_type='codex_terminal',config_json=$2 WHERE id=$1", &[&analyst,&json!({"binary_path":cli})]).await.unwrap();
    assert!(
        runtime
            .get_binding(&target)
            .await
            .unwrap()
            .valid_terminal_session_anchor_for_context(None, None, Some(0), None)
            .is_some()
    );
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let (status, body) = api_json_payload_request(
        app.clone(),
        &owner,
        Method::PUT,
        format!("/v1/runtime/bindings/{target}/experience"),
        json!({"enabled":true,"analyst_binding_id":analyst}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let error = client.query_one("SELECT last_error,source_cursor FROM experience_policy WHERE binding_id=$1", &[&target]).await.unwrap();
            if let Some(error_message) = error.get::<_, Option<String>>("last_error") {
                let reports = db.experience_revisions(&owner.workspace_id, &owner.id, &target).await.unwrap();
                assert!(!reports.is_empty(), "analysis must have committed the opening window");
                assert_eq!(reports.last().unwrap().source_references, json!([]));
                panic!("background analysis rejected the continued objective: {error_message}; committed cursor: {}; opening summary: {}", error.get::<_, Value>("source_cursor"), reports.last().unwrap().analysis);
            }
            if let Some((_, instruction)) = db
                .active_experience(&owner.workspace_id, &target)
                .await
                .unwrap()
            {
                assert_eq!(
                    instruction,
                    "Verify required checks before reporting completion."
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("background worker must activate the reviewed native-source report");
    let revisions = db
        .experience_revisions(&owner.workspace_id, &owner.id, &target)
        .await
        .unwrap();
    assert_eq!(revisions.len(), 4);
    assert_eq!(revisions[3].disposition, "no_change");
    assert_eq!(revisions[3].source_references, json!([]));
    assert!(revisions[3].analysis.contains(&shared_ref));
    assert_eq!(revisions[2].disposition, "no_change");
    assert_eq!(revisions[2].source_references, json!([]));
    assert!(
        revisions[2].validation["problems"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(revisions[1].disposition, "no_change");
    assert!(revisions[1].validation["research"].is_null());
    assert_eq!(revisions[0].disposition, "active");
    assert_eq!(revisions[0].validation["review_details"]["passed"], true);
    assert_eq!(
        revisions[0].validation["review_details"]["instruction_matches"],
        true
    );
    let diagnostic_trace = revisions[0].validation["trace_id"].as_str().unwrap();
    let audit = db.list_audit_logs(&owner.workspace_id).await.unwrap();
    let stages: Vec<_> = audit
        .iter()
        .filter(|row| row.metadata["trace_id"] == diagnostic_trace)
        .collect();
    let selected = stages
        .iter()
        .find(|row| {
            row.metadata["stage"] == "source_selection" && row.metadata["outcome"] == "selected"
        })
        .unwrap();
    let source_digest: String = client
        .query_one(
            "SELECT source_digest FROM experience_revision WHERE id=$1",
            &[&revisions[0].id],
        )
        .await
        .unwrap()
        .get("source_digest");
    assert_eq!(selected.metadata["source_digest"], source_digest);
    for stage in ["analysis", "research", "adaptation", "content_review"] {
        let started = stages
            .iter()
            .find(|row| row.metadata["stage"] == stage && row.metadata["outcome"] == "started")
            .unwrap();
        if stage != "research" {
            assert!(started.metadata["input"]["input_bytes"].as_u64().unwrap() > 0);
            assert_eq!(
                started.metadata["input"]["input_digest"]
                    .as_str()
                    .unwrap()
                    .len(),
                64
            );
        }
        let completed = stages
            .iter()
            .find(|row| {
                row.metadata["call_id"] == started.metadata["call_id"]
                    && row.metadata["result"]["outcome"] == "completed"
            })
            .unwrap();
        assert!(
            completed.metadata["result"]["output_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(
            completed.metadata["result"]["output_digest"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert!(!completed.metadata.to_string().contains("paddingpadding"));
    }
    assert!(
        revisions[0].validation["research"]
            .as_str()
            .unwrap()
            .contains("observable check evidence")
    );
    let observation = client.query_one("SELECT p.prompt_revision_id,o.episode_ref FROM experience_problem p JOIN experience_problem_observation o USING(binding_id,problem_key) WHERE p.binding_id=$1 AND o.episode_ref LIKE 'native-source:%'", &[&target]).await.unwrap();
    let shared_observation = client.query_one("SELECT episode_ref FROM experience_problem_observation WHERE binding_id=$1 AND episode_ref=$2", &[&target,&shared_ref]).await.unwrap();
    assert_eq!(
        shared_observation.get::<_, String>("episode_ref"),
        shared_ref
    );
    let cursor: Value = client
        .query_one(
            "SELECT source_cursor FROM experience_policy WHERE binding_id=$1",
            &[&target],
        )
        .await
        .unwrap()
        .get("source_cursor");
    assert_eq!(cursor["feedback"][&binding.conversation_id], 1);
    assert_eq!(
        observation.get::<_, String>("prompt_revision_id"),
        revisions[0].id
    );
    assert!(
        observation
            .get::<_, String>("episode_ref")
            .starts_with("native-source:")
    );
    assert!(
        revisions[2]
            .analysis
            .contains(&observation.get::<_, String>("episode_ref"))
    );
    assert!(
        revisions[0]
            .source_references
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference.as_str().unwrap().starts_with("native-source:"))
    );
    let applied_revision = revisions[0].id.clone();
    let marker_ref = format!("native-source:{}", fs::metadata(&native).unwrap().len());
    let mut transcript = fs::OpenOptions::new().append(true).open(&native).unwrap();
    let retention = if retain_marker {
        " Retain marker reference."
    } else {
        ""
    };
    writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":format!("[choruz-experience revision={applied_revision}] Verify required checks before reporting completion.{retention}")}]}})).unwrap();
    writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"[choruz-experience revision=obsolete] A different revision must not replace the applied marker."}]}})).unwrap();
    drop(transcript);
    // Commit the marker's own window before the distinct failure exists. A
    // summary can tell the analyst which ref to cite, but cannot verify it.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            client.execute("UPDATE experience_policy SET next_check_at=NOW() WHERE binding_id=$1 AND lease_token IS NULL", &[&target]).await.unwrap();
            let reports = db.experience_revisions(&owner.workspace_id, &owner.id, &target).await.unwrap();
            if reports.len() == 5 {
                assert_eq!(reports[0].disposition, "no_change");
                assert_eq!(reports[0].source_references, if retain_marker { json!([marker_ref]) } else { json!([]) });
                assert!(reports[0].analysis.contains(&marker_ref));
                assert_eq!(db.active_experience(&owner.workspace_id, &target).await.unwrap().unwrap().0, applied_revision);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("marker window must commit without activation before the new failure");
    let mut transcript = fs::OpenOptions::new().append(true).open(&native).unwrap();
    writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"For the next task, your completion claim was again wrong: you never ran the required check."}]}})).unwrap();
    drop(transcript);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            // Advance only this test's idle schedule, not an in-flight lease.
            client.execute("UPDATE experience_policy SET next_check_at=NOW() WHERE binding_id=$1 AND lease_token IS NULL", &[&target]).await.unwrap();
            let reports = db
                .experience_revisions(&owner.workspace_id, &owner.id, &target)
                .await
                .unwrap();
            let error: Option<String> = client.query_one("SELECT last_error FROM experience_policy WHERE binding_id=$1", &[&target]).await.unwrap().get("last_error");
            assert!(error.is_none(), "historical marker must validate through scoped recovery: {error:?}");
            if reports.len() == 6 {
                assert!(reports[0].source_references.as_array().unwrap().contains(&json!(marker_ref)));
                assert_eq!(reports[0].validation["problems"][0]["applied_revision_id"], applied_revision);
                assert_eq!(
                    reports[0].validation["escalation_candidates"],
                    json!(["skipped-verification"])
                );
                let context = db.experience_for_turn(&owner.workspace_id, &target).await.unwrap().unwrap();
                assert_eq!(context.revision_id, reports[0].id);
                assert!(context.team.is_none(), "recurrence alone must not bypass measured team search consent");
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("a distinct failure after applying guidance must be recognized");
    let observations = client.query("SELECT episode_ref, applied_revision_id FROM experience_problem_observation WHERE binding_id=$1", &[&target]).await.unwrap();
    assert_eq!(observations.len(), 3);
    assert_eq!(
        observations
            .iter()
            .filter(|row| row
                .get::<_, Option<String>>("applied_revision_id")
                .as_deref()
                == Some(&applied_revision))
            .count(),
        1
    );
    db.select_experience_revision(
        &owner.workspace_id,
        &owner.id,
        &target,
        Some(&applied_revision),
        None,
    )
    .await
    .unwrap();
    let restored = db
        .experience_for_turn(&owner.workspace_id, &target)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.revision_id, applied_revision);
    assert!(
        restored.team.is_none(),
        "restoring a prompt-only revision must remove the execution role"
    );
    let mut transcript = fs::OpenOptions::new().append(true).open(&native).unwrap();
    writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Reject this proposal fixture: another completion claim needs review."}]}})).unwrap();
    drop(transcript);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            client.execute("UPDATE experience_policy SET next_check_at=NOW() WHERE binding_id=$1 AND lease_token IS NULL", &[&target]).await.unwrap();
            let reports = db.experience_revisions(&owner.workspace_id, &owner.id, &target).await.unwrap();
            if reports.len() == 7 {
                assert_eq!(reports[0].disposition, "candidate");
                let details = &reports[0].validation["review_details"];
                assert_eq!(details["passed"], false);
                assert_eq!(details["instruction_matches"], false);
                assert_eq!(details["evidence_verified"], true);
                assert!(details["reviewer_report"]["summary"].as_str().unwrap().contains("does not address this task"));
                assert!(!details.to_string().contains("fixture-secret"));
                assert_eq!(db.active_experience(&owner.workspace_id, &target).await.unwrap().unwrap().0, applied_revision);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("rejected content review must retain its reason without activation");
    let checkpoint: Value = client
        .query_one(
            "SELECT source_cursor FROM experience_policy WHERE binding_id=$1",
            &[&target],
        )
        .await
        .unwrap()
        .get("source_cursor");
    let mut transcript = fs::OpenOptions::new().append(true).open(&native).unwrap();
    writeln!(transcript, "{}", json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Historical-only evidence: repeat the earlier completion claim complaint."}]}})).unwrap();
    drop(transcript);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            client.execute("UPDATE experience_policy SET next_check_at=NOW() WHERE binding_id=$1 AND lease_token IS NULL", &[&target]).await.unwrap();
            let row = client.query_one("SELECT last_error,source_cursor FROM experience_policy WHERE binding_id=$1", &[&target]).await.unwrap();
            if let Some(error) = row.get::<_, Option<String>>("last_error") {
                assert_eq!(error, "Learning problem lacks new source evidence");
                assert_eq!(row.get::<_, Value>("source_cursor"), checkpoint);
                assert_eq!(db.experience_revisions(&owner.workspace_id, &owner.id, &target).await.unwrap().len(), 7);
                let records = db.list_audit_logs(&owner.workspace_id).await.unwrap();
                let failure = records.iter().find(|record| record.action == "learning.check" && record.metadata["outcome"] == "failed").expect("failed checks remain durable without creating a revision");
                assert_eq!(failure.metadata["error_category"], "validation");
                assert!(failure.metadata.get("error").is_none());
                assert!(failure.metadata["trace_id"].is_string());
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("historical evidence alone must not count as a new failure");
    assert!(
        !workspace.join(".choruz/sessions").exists(),
        "reading a native trace must not launch a foreground session"
    );
    assert!(
        runtime
            .get_binding(&analyst)
            .await
            .unwrap()
            .external_session_id
            .is_none(),
        "scratch analysis must not bind the analyst's live session"
    );
    drop(app);
}

async fn learning_binding(database: &TestDatabase, owner: &choruz_domain::Principal) -> String {
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let agent = choruz_common::new_id();
    let conversation = choruz_common::new_id();
    client.execute("INSERT INTO principal(id,workspace_id,type,name,disabled,created_at,updated_at) VALUES($1,$2,'agent',$1,FALSE,NOW(),NOW())",
        &[&agent, &owner.workspace_id]).await.unwrap();
    client.execute("INSERT INTO conversation(id,workspace_id,type,name,creator_id,created_at,updated_at) VALUES($1,$2,'direct','Learning test',$3,NOW(),NOW())",
        &[&conversation, &owner.workspace_id, &owner.id]).await.unwrap();
    client.execute("INSERT INTO conversation_member(conv_id,principal_id,joined_at) VALUES($1,$2,NOW()),($1,$3,NOW())",
        &[&conversation, &owner.id, &agent]).await.unwrap();
    RuntimeStore::new(&database.database_url)
        .create_binding(CreateBindingInput {
            conversation_id: conversation,
            agent_principal_id: agent,
            driver_type: DriverType::ClaudeTerminal,
            workspace_path: "/nonexistent-learning-test".into(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn experience_settings_scope_and_stale_analysis_activation() {
    let database = TestDatabase::create().await;
    let db =
        choruz_application::DbService::new(choruz_store::EventStore::new(&database.database_url));
    let owner = db
        .create_human_user("learner", "test-password-123")
        .await
        .unwrap();
    let outsider = db
        .create_human_user("outsider", "test-password-123")
        .await
        .unwrap();
    let target = learning_binding(&database, &owner).await;
    let analyst = learning_binding(&database, &owner).await;
    let foreign = learning_binding(&database, &outsider).await;
    let app = router_with_db(choruz_application::ChatApp::new(), &database.database_url);
    let endpoint = format!("/v1/runtime/bindings/{target}/experience");
    let (status, _) = api_json_payload_request(
        app.clone(),
        &owner,
        Method::PUT,
        endpoint.clone(),
        json!({"enabled":false,"analyst_binding_id":target}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = api_json_payload_request(
        app.clone(),
        &owner,
        Method::PUT,
        endpoint.clone(),
        json!({"enabled":false,"analyst_binding_id":foreign}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, result) = api_json_payload_request(
        app.clone(),
        &owner,
        Method::PUT,
        endpoint.clone(),
        json!({"enabled":false,"analyst_binding_id":analyst}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["policy"]["analyst_binding_id"], analyst);
    let (status, _) = api_json_request(app.clone(), &outsider, Method::GET, endpoint).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    drop(app);

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
    let mut claim = db.claim_experience().await.unwrap().unwrap();
    assert_eq!(claim.binding_id, target);
    let runtime = RuntimeStore::new(&database.database_url);
    let binding = runtime.get_binding(&target).await.unwrap();
    let client = runtime.connect().await.unwrap();
    let mut historical_refs = Vec::new();
    for (seq, sender, content) in [
        (1_i64, &owner.id, "Prefer concise reports"),
        (2, &outsider.id, "Unconsented third-party content"),
    ] {
        let id = choruz_common::new_id();
        client.execute("INSERT INTO conversation_events(conversation_id,seq,event_id,event_type,sender_id,content) VALUES($1,$2,$3,'message',$4,$5)",
            &[&binding.conversation_id,&seq,&id,sender,&content]).await.unwrap();
        historical_refs.push(format!("message:{id}"));
    }
    let feedback = db.experience_feedback(&claim).await.unwrap();
    assert_eq!(feedback["records"].as_array().unwrap().len(), 1);
    assert_eq!(feedback["records"][0]["content"], "Prefer concise reports");
    assert!(
        db.experience_feedback_references(&claim, &historical_refs)
            .await
            .unwrap()
            .is_empty(),
        "unread feedback is not historical evidence"
    );
    claim.source_cursor = json!({"feedback":feedback["cursor"]});
    let unread = choruz_common::new_id();
    client.execute("INSERT INTO conversation_events(conversation_id,seq,event_id,event_type,sender_id,content) VALUES($1,3,$2,'message',$3,'Unread feedback')", &[&binding.conversation_id,&unread,&owner.id]).await.unwrap();
    historical_refs.push(format!("message:{unread}"));
    assert_eq!(
        db.experience_feedback_references(&claim, &historical_refs)
            .await
            .unwrap(),
        vec![historical_refs[0].clone()],
        "a committed cursor must not authorize another sender or post-cursor feedback"
    );
    let workspace = claim.workspace_id.clone();
    claim.workspace_id = outsider.workspace_id.clone();
    assert!(
        db.experience_feedback_references(&claim, &historical_refs)
            .await
            .unwrap()
            .is_empty()
    );
    claim.workspace_id = workspace;
    client
        .execute(
            "UPDATE conversation_member SET removed_at=NOW() WHERE conv_id=$1 AND principal_id=$2",
            &[&binding.conversation_id, &owner.id],
        )
        .await
        .unwrap();
    assert!(
        db.experience_feedback(&claim).await.unwrap()["records"]
            .as_array()
            .unwrap()
            .is_empty(),
        "removed membership must not export feedback"
    );
    assert!(
        db.experience_feedback_references(&claim, &historical_refs)
            .await
            .unwrap()
            .is_empty(),
        "removed membership also revokes historical feedback recovery"
    );
    client
        .execute(
            "UPDATE conversation_member SET removed_at=NULL WHERE conv_id=$1 AND principal_id=$2",
            &[&binding.conversation_id, &owner.id],
        )
        .await
        .unwrap();
    assert!(
        db.claim_experience().await.unwrap().is_none(),
        "a live lease is not claimed twice"
    );
    let candidate = db
        .save_experience_candidate(
            &claim,
            ExperienceReport {
                digest: "source-one",
                references: &json!(["record-1"]),
                analysis: "Supported preference",
                instruction: Some("Use concise reports."),
                validation: &json!({"review":"passed","addressed_problems":["ignored-format"],"problems":[
                    {"key":"ignored-format","description":"Ignored requested report format","episode_ref":"record-1","evidence":["record-1"],"applied_revision_id":null}]}),
                checkpoint: None,
                activate: false,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        db.active_experience(&owner.workspace_id, &target)
            .await
            .unwrap()
            .is_none(),
        "saving a candidate does not activate it"
    );
    db.configure_experience(
        &owner.workspace_id,
        &owner.id,
        &target,
        &analyst,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(
        db.select_experience_revision(
            &owner.workspace_id,
            &owner.id,
            &target,
            Some(&candidate),
            Some(&claim.token)
        )
        .await
        .is_err(),
        "disabled policy rejects a late model response"
    );
    assert!(
        db.save_experience_candidate(
            &claim,
            ExperienceReport {
                digest: "source-late",
                references: &json!(["record-2"]),
                analysis: "Late report",
                instruction: Some("Stale instruction"),
                validation: &json!({"review":"passed"}),
                checkpoint: Some(&json!({"offset":99})),
                activate: true
            }
        )
        .await
        .unwrap()
        .is_none()
    );

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
    let fresh = db.claim_experience().await.unwrap().unwrap();
    client.batch_execute("ALTER TABLE audit_log ADD CONSTRAINT reject_learning_commit CHECK(action <> 'learning.check')").await.unwrap();
    let rejected_audit = db
        .save_experience_candidate(
            &fresh,
            ExperienceReport {
                digest: "audit-rollback",
                references: &json!([]),
                analysis: "No change",
                instruction: Some("Do not activate without its audit"),
                validation: &json!({"review":"passed","trace_id":"atomic-check"}),
                checkpoint: None,
                activate: true,
            },
        )
        .await;
    assert!(rejected_audit.is_err());
    assert!(
        db.active_experience(&owner.workspace_id, &target)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !db.experience_source_seen(&fresh, "audit-rollback")
            .await
            .unwrap()
    );
    client
        .batch_execute("ALTER TABLE audit_log DROP CONSTRAINT reject_learning_commit")
        .await
        .unwrap();
    let revision = db
        .save_experience_candidate(
            &fresh,
            ExperienceReport {
                digest: "source-two",
                references: &json!(["record-3"]),
                analysis: "New report",
                instruction: Some("Explain results briefly."),
                validation: &json!({"review":"passed","addressed_problems":["ignored-format"],"problems":[
                    {"key":"ignored-format","description":"Ignored requested report format","episode_ref":"record-1","evidence":["record-3"],"applied_revision_id":null}]}),
                checkpoint: Some(&json!({"offset":120})),
                activate: true,
            },
        )
        .await
        .unwrap()
        .unwrap();
    let problems = db.experience_problems(&fresh).await.unwrap();
    assert_eq!(
        problems[0]["episodes"].as_array().unwrap().len(),
        1,
        "a second report of the same episode is not recurrence"
    );
    assert_eq!(problems[0]["prompt_revision_id"], revision);
    db.select_experience_revision(
        &owner.workspace_id,
        &owner.id,
        &target,
        Some(&revision),
        Some(&fresh.token),
    )
    .await
    .unwrap();
    assert_eq!(
        db.active_experience(&owner.workspace_id, &target)
            .await
            .unwrap(),
        Some((revision.clone(), "Explain results briefly.".into()))
    );
    assert!(
        db.active_experience(&outsider.workspace_id, &target)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.select_experience_revision(&owner.workspace_id, &outsider.id, &target, None, None)
            .await
            .is_err()
    );
    db.select_experience_revision(&owner.workspace_id, &owner.id, &target, None, None)
        .await
        .unwrap();
    assert!(
        db.active_experience(&owner.workspace_id, &target)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.experience_revisions(&owner.workspace_id, &owner.id, &target)
            .await
            .unwrap()
            .len(),
        2
    );
}
