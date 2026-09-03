use super::*;

#[tokio::test]
async fn channel_kanban_migration_hard_resets_legacy_workflow_rows_and_adds_constraints() {
    let database = TestDatabase::create_without_migrations().await;
    database
        .apply_migrations_through("0024_hybrid_agent_routing.sql")
        .await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for channel kanban migration setup");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute(
            "
            INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
            VALUES
              ('kanban-human', 'kanban-ws', 'human', 'Kanban Human', FALSE, NOW(), NOW()),
              ('kanban-agent', 'kanban-ws', 'agent', 'Kanban Agent', FALSE, NOW(), NOW());

            INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
            VALUES ('kanban-conv', 'kanban-ws', 'group', 'Kanban Group', 'kanban-human', NOW(), NOW());

            INSERT INTO conversation_member (conv_id, principal_id, role, joined_at)
            VALUES
              ('kanban-conv', 'kanban-human', 'owner', NOW()),
              ('kanban-conv', 'kanban-agent', 'member', NOW());

            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, description, status, source_message_id, created_by)
            VALUES
              ('legacy-task', 'kanban-conv', 'LEGACY-1', 'Legacy task', 'Legacy description',
               'needs_human', 'legacy-message', 'kanban-human');

            INSERT INTO group_workflow_task_participant
              (id, task_id, principal_id, role_key)
            VALUES ('legacy-participant', 'legacy-task', 'kanban-agent', 'owner');

            INSERT INTO group_workflow_event
              (id, conversation_id, task_id, actor_principal_id, kind, payload)
            VALUES
              ('legacy-event', 'kanban-conv', 'legacy-task', 'kanban-human',
               'human_input_needed', '{\"question\":\"Ship it?\"}'::jsonb);
            ",
        )
        .await
        .expect("seed legacy workflow rows");

    database
        .apply_migration("0025_channel_kanban_board.sql")
        .await;

    let counts = client
        .query_one(
            "
            SELECT
              (SELECT COUNT(*)::BIGINT FROM group_workflow_task) AS tasks,
              (SELECT COUNT(*)::BIGINT FROM group_workflow_task_participant) AS participants,
              (SELECT COUNT(*)::BIGINT FROM group_workflow_event) AS events
            ",
            &[],
        )
        .await
        .expect("query hard reset counts");
    assert_eq!(counts.get::<_, i64>("tasks"), 0);
    assert_eq!(counts.get::<_, i64>("participants"), 0);
    assert_eq!(counts.get::<_, i64>("events"), 0);

    let visibility = client
        .query_one(
            "SELECT channel_visibility FROM principal WHERE id = 'kanban-agent'",
            &[],
        )
        .await
        .expect("query principal visibility")
        .get::<_, String>("channel_visibility");
    assert_eq!(visibility, "visible");

    client
        .execute(
            "
            INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
            VALUES ('kanban-new-agent', 'kanban-ws', 'agent', 'Kanban New Agent', FALSE, NOW(), NOW())
            ",
            &[],
        )
        .await
        .expect("insert principal after channel visibility migration");
    let new_visibility = client
        .query_one(
            "SELECT channel_visibility FROM principal WHERE id = 'kanban-new-agent'",
            &[],
        )
        .await
        .expect("query new principal visibility")
        .get::<_, String>("channel_visibility");
    assert_eq!(new_visibility, "visible");

    let invalid_visibility = client
        .execute(
            "UPDATE principal SET channel_visibility = 'private' WHERE id = 'kanban-agent'",
            &[],
        )
        .await;
    assert!(invalid_visibility.is_err());
    client
        .execute(
            "UPDATE principal SET channel_visibility = 'internal' WHERE id = 'kanban-agent'",
            &[],
        )
        .await
        .expect("mark agent internal");
    let db = choruz_application::DbService::new(choruz_store::EventStore::new(
        database.database_url.clone(),
    ));
    let internal_agent = db
        .get_principal("kanban-agent")
        .await
        .expect("load internal agent principal");
    assert_eq!(
        internal_agent.channel_visibility,
        choruz_domain::ChannelVisibility::Internal
    );

    let legacy_status = client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, status, assignee_principal_id, created_by)
            VALUES
              ('bad-status-task', 'kanban-conv', 'BAD-1', 'Bad status', 'needs_human',
               'kanban-human', 'kanban-human')
            ",
            &[],
        )
        .await;
    assert!(legacy_status.is_err());

    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, assignee_principal_id, created_by,
               idempotency_key)
            VALUES
              ('default-task', 'kanban-conv', 'TASK-1', 'Default status', 'kanban-human',
               'kanban-human', 'idem-1')
            ",
            &[],
        )
        .await
        .expect("insert board-compatible task");

    let default_status = client
        .query_one(
            "SELECT status, source_kind, version FROM group_workflow_task WHERE id = 'default-task'",
            &[],
        )
        .await
        .expect("query default board task");
    assert_eq!(default_status.get::<_, String>("status"), "todo");
    assert_eq!(default_status.get::<_, String>("source_kind"), "agent");
    assert_eq!(default_status.get::<_, i64>("version"), 1);

    let description_column = client
        .query_opt(
            "
            SELECT 1
            FROM information_schema.columns
            WHERE table_name = 'group_workflow_task'
              AND column_name = 'description'
            ",
            &[],
        )
        .await
        .expect("query task description column");
    assert!(
        description_column.is_none(),
        "board persistence must not keep an undefined description column"
    );

    let duplicate_task_key = client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, assignee_principal_id, created_by)
            VALUES
              ('duplicate-task-key', 'kanban-conv', 'TASK-1', 'Duplicate task key',
               'kanban-human', 'kanban-human')
            ",
            &[],
        )
        .await;
    assert!(duplicate_task_key.is_err());

    client
        .execute(
            "
            INSERT INTO group_workflow_task_participant
              (id, task_id, principal_id, role_key)
            VALUES ('owner-human', 'default-task', 'kanban-human', 'owner')
            ",
            &[],
        )
        .await
        .expect("insert first owner participant");
    let duplicate_owner = client
        .execute(
            "
            INSERT INTO group_workflow_task_participant
              (id, task_id, principal_id, role_key)
            VALUES ('owner-agent', 'default-task', 'kanban-agent', 'owner')
            ",
            &[],
        )
        .await;
    assert!(
        duplicate_owner.is_err(),
        "routing compatibility must not allow multiple owner participants"
    );

    let duplicate_idempotency = client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, assignee_principal_id, created_by,
               idempotency_key)
            VALUES
              ('duplicate-idem-task', 'kanban-conv', 'TASK-2', 'Duplicate idempotency',
               'kanban-human', 'kanban-human', 'idem-1')
            ",
            &[],
        )
        .await;
    assert!(duplicate_idempotency.is_err());

    client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, source_kind, source_message_id,
               assignee_principal_id, created_by)
            VALUES
              ('message-task', 'kanban-conv', 'TASK-3', 'Message task', 'message',
               'source-message-1', 'kanban-human', 'kanban-human')
            ",
            &[],
        )
        .await
        .expect("insert message-derived task");
    let duplicate_message = client
        .execute(
            "
            INSERT INTO group_workflow_task
              (id, conversation_id, task_key, title, source_kind, source_message_id,
               assignee_principal_id, created_by)
            VALUES
              ('duplicate-message-task', 'kanban-conv', 'TASK-4', 'Duplicate message task',
               'message', 'source-message-1', 'kanban-human', 'kanban-human')
            ",
            &[],
        )
        .await;
    assert!(duplicate_message.is_err());

    let index_rows = client
        .query(
            "
            SELECT indexname
            FROM pg_indexes
            WHERE schemaname = 'public'
              AND indexname IN (
                'idx_principal_channel_visible_agents',
                'idx_group_workflow_task_conversation_status',
                'idx_group_workflow_task_assignee',
                'idx_group_workflow_task_conversation_assignee',
                'group_workflow_task_single_owner_participant_idx',
                'group_workflow_task_agent_idempotency_idx',
                'group_workflow_task_message_dedupe_idx'
              )
            ",
            &[],
        )
        .await
        .expect("query channel kanban indexes");
    let index_names: Vec<String> = index_rows
        .iter()
        .map(|row| row.get::<_, String>("indexname"))
        .collect();
    for expected_index in [
        "idx_principal_channel_visible_agents",
        "idx_group_workflow_task_conversation_status",
        "idx_group_workflow_task_assignee",
        "idx_group_workflow_task_conversation_assignee",
        "group_workflow_task_single_owner_participant_idx",
        "group_workflow_task_agent_idempotency_idx",
        "group_workflow_task_message_dedupe_idx",
    ] {
        assert!(
            index_names
                .iter()
                .any(|index_name| index_name == expected_index),
            "missing expected index {expected_index}"
        );
    }
}

#[tokio::test]
async fn workflow_task_service_synchronizes_assignee_owner_and_versioned_events() {
    let database = TestDatabase::create().await;
    let app = choruz_application::ChatApp::new();
    let operator = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: "ws-workflow-sync".into(),
            principal_type: PrincipalType::Human,
            name: "Operator".into(),
            avatar_url: None,
        })
        .unwrap();
    let human = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Human".into(),
            avatar_url: None,
        })
        .unwrap();
    let outsider = app
        .create_principal(CreatePrincipalRequest {
            workspace_id: operator.workspace_id.clone(),
            principal_type: PrincipalType::Human,
            name: "Outsider".into(),
            avatar_url: None,
        })
        .unwrap();
    let agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Visible Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let reviewer_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Reviewer Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let quality_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Quality Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let approver_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Approver Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let coordinator_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Coordinator Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let task_role_coordinator_agent = app
        .create_agent(CreateAgentRequest {
            actor_id: operator.id.clone(),
            name: "Task Role Coordinator Agent".into(),
            scopes: vec!["messages:read".into()],
            workspace_id: None,
            channel_visibility: None,
        })
        .unwrap()
        .principal;
    let conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Workflow Sync".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![
                human.id.clone(),
                agent.id.clone(),
                reviewer_agent.id.clone(),
                quality_agent.id.clone(),
                approver_agent.id.clone(),
                coordinator_agent.id.clone(),
                task_role_coordinator_agent.id.clone(),
            ],
            workspace_id: None,
        })
        .unwrap();
    let other_conversation = app
        .create_group(CreateGroupRequest {
            actor_id: operator.id.clone(),
            name: "Other Workflow Sync".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![human.id.clone(), agent.id.clone()],
            workspace_id: None,
        })
        .unwrap();
    for principal in [
        &operator,
        &human,
        &outsider,
        &agent,
        &reviewer_agent,
        &quality_agent,
        &approver_agent,
        &coordinator_agent,
        &task_role_coordinator_agent,
    ] {
        seed_principal_to_db(&database.database_url, principal).await;
    }
    seed_conversation_to_db(&database.database_url, &conversation).await;
    seed_conversation_to_db(&database.database_url, &other_conversation).await;

    let (policy_client, policy_connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for coordinator policy");
    tokio::spawn(async move {
        let _ = policy_connection.await;
    });
    policy_client
        .execute(
            "INSERT INTO conversation_runtime_policies
                (conversation_id, default_coordinator_agent_id)
             VALUES ($1, $2)",
            &[&conversation.id, &coordinator_agent.id],
        )
        .await
        .expect("seed coordinator policy");

    let db = choruz_application::DbService::new(choruz_store::EventStore::new(
        database.database_url.clone(),
    ));
    let task = db
        .create_group_workflow_task(
            &conversation.id,
            &human.id,
            choruz_application::CreateGroupWorkflowTaskRequest {
                task_key: "SYNC-1".into(),
                title: "Synchronize owner".into(),
                assignee_principal_id: agent.id.clone(),
                source_message_id: None,
                participants: vec![
                    choruz_application::WorkflowTaskParticipantInput {
                        principal_id: human.id.clone(),
                        role_key: "reviewer".into(),
                        responsibility: None,
                        required: true,
                    },
                    choruz_application::WorkflowTaskParticipantInput {
                        principal_id: reviewer_agent.id.clone(),
                        role_key: "reviewer".into(),
                        responsibility: None,
                        required: true,
                    },
                    choruz_application::WorkflowTaskParticipantInput {
                        principal_id: quality_agent.id.clone(),
                        role_key: "quality_check".into(),
                        responsibility: None,
                        required: true,
                    },
                    choruz_application::WorkflowTaskParticipantInput {
                        principal_id: approver_agent.id.clone(),
                        role_key: "approver".into(),
                        responsibility: None,
                        required: true,
                    },
                    choruz_application::WorkflowTaskParticipantInput {
                        principal_id: task_role_coordinator_agent.id.clone(),
                        role_key: "coordinator".into(),
                        responsibility: None,
                        required: true,
                    },
                ],
            },
        )
        .await
        .expect("create workflow task");
    let other_task = db
        .create_group_workflow_task(
            &other_conversation.id,
            &human.id,
            choruz_application::CreateGroupWorkflowTaskRequest {
                task_key: "OTHER-1".into(),
                title: "Other task".into(),
                assignee_principal_id: agent.id.clone(),
                source_message_id: None,
                participants: Vec::new(),
            },
        )
        .await
        .expect("create other workflow task");
    assert_eq!(task.assignee_principal_id, agent.id);
    assert_eq!(task.version, 1);
    assert!(
        task.participants
            .iter()
            .any(|participant| participant.role_key == "owner"
                && participant.principal_id == agent.id)
    );

    let owner_add = db
        .add_group_workflow_task_participant(
            &task.id,
            choruz_application::WorkflowTaskParticipantInput {
                principal_id: human.id.clone(),
                role_key: "owner".into(),
                responsibility: None,
                required: true,
            },
        )
        .await;
    assert!(owner_add.is_err());

    let owner_agent_noop_check = db
        .append_group_workflow_event(
            &task.id,
            &agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.passed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "agent owner passed external check"
                }),
            },
        )
        .await
        .expect("agent owner can record passed external check");
    assert_eq!(owner_agent_noop_check.resulting_version, None);
    assert_eq!(
        owner_agent_noop_check.payload["workflow_diagnostic"]["reason_code"],
        "workflow_status_noop"
    );
    let after_owner_agent_noop = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_owner_agent_noop.status, "todo");
    assert_eq!(after_owner_agent_noop.version, 1);

    let updated = db
        .update_group_workflow_task(
            &task.id,
            choruz_application::UpdateGroupWorkflowTaskRequest {
                title: None,
                status: Some("blocked".into()),
                assignee_principal_id: Some(human.id.clone()),
                participants: None,
            },
        )
        .await
        .expect("update assignee");
    assert_eq!(updated.assignee_principal_id, human.id);
    assert_eq!(updated.version, 2);
    let owner_rows = updated
        .participants
        .iter()
        .filter(|participant| participant.role_key == "owner")
        .collect::<Vec<_>>();
    assert_eq!(owner_rows.len(), 1);
    assert_eq!(owner_rows[0].principal_id, human.id);

    let event = db
        .append_group_workflow_event(
            &task.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.started".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "start",
                    "workflow_diagnostic": { "reason_code": "workflow_status_unauthorized" },
                    "previous": { "status": "done", "raw_internal": true },
                    "new": { "status": "done", "tool_diagnostics": ["unsafe"] }
                }),
            },
        )
        .await
        .expect("append status event");
    assert_eq!(event.resulting_version, Some(3));
    assert_eq!(event.payload["previous"]["status"], json!("blocked"));
    assert_eq!(event.payload["new"]["status"], json!("in_progress"));
    assert_eq!(event.payload["previous"].get("raw_internal"), None);
    assert_eq!(event.payload["new"].get("tool_diagnostics"), None);

    let (fanout_client, fanout_connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for workflow fanout assertion");
    tokio::spawn(async move {
        let _ = fanout_connection.await;
    });
    let workflow_fanout = fanout_client
        .query_one(
            "
            SELECT ce.event_type, ce.metadata
            FROM conversation_events ce
            WHERE ce.conversation_id = $1
              AND ce.event_type = 'channel_task.updated'
            ORDER BY ce.seq DESC
            LIMIT 1
            ",
            &[&conversation.id],
        )
        .await
        .expect("load workflow metadata fanout");
    assert_eq!(
        workflow_fanout.get::<_, String>("event_type"),
        "channel_task.updated"
    );
    let workflow_fanout_metadata: Value = workflow_fanout.get("metadata");
    assert_eq!(workflow_fanout_metadata["task_id"], task.id);
    assert_eq!(workflow_fanout_metadata["version"], 3);
    assert_eq!(workflow_fanout_metadata["task"]["status"], "in_progress");
    assert_eq!(
        workflow_fanout_metadata["task"].get("workflow_diagnostic"),
        None
    );
    let workflow_task_router_outbox_count = fanout_client
        .query_one(
            "
            SELECT COUNT(*)::BIGINT AS count
            FROM event_outbox
            WHERE aggregate_id = $1
              AND event_type = 'channel_task.updated'
            ",
            &[&conversation.id],
        )
        .await
        .expect("count workflow task router outbox rows")
        .get::<_, i64>("count");
    assert_eq!(
        workflow_task_router_outbox_count, 0,
        "workflow metadata task fanout must not enqueue router-visible outbox rows"
    );

    let informational_event = db
        .append_group_workflow_event(
            &task.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.passed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "no board status effect",
                    "reason_code": "private_runtime_label",
                    "previous": { "status": "done", "raw_internal": true },
                    "new": { "status": "blocked", "tool_diagnostics": ["unsafe"] }
                }),
            },
        )
        .await
        .expect("preserve human external check metadata without board mutation");
    assert_eq!(informational_event.resulting_version, None);
    assert_eq!(informational_event.payload.get("previous"), None);
    assert_eq!(informational_event.payload.get("new"), None);
    assert_eq!(informational_event.payload.get("reason_code"), None);
    assert_eq!(
        informational_event.payload["workflow_diagnostic"]["reason_code"],
        "workflow_status_unauthorized"
    );

    let reloaded = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(reloaded.status, "in_progress");
    assert_eq!(reloaded.version, 3);

    let unauthorized_noop = db
        .append_group_workflow_event(
            &task.id,
            &reviewer_agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.passed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "reviewer cannot record passed external check"
                }),
            },
        )
        .await
        .expect("preserve unauthorized no-op metadata");
    assert_eq!(unauthorized_noop.resulting_version, None);
    assert_eq!(
        unauthorized_noop.payload["workflow_diagnostic"]["reason_code"],
        "workflow_status_unauthorized"
    );

    let after_unauthorized_noop = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_unauthorized_noop.status, "in_progress");
    assert_eq!(after_unauthorized_noop.version, 3);

    let task_role_coordinator_external_failure = db
        .append_group_workflow_event(
            &task.id,
            &task_role_coordinator_agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.failed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "task-level coordinator role is routing-only for agents"
                }),
            },
        )
        .await
        .expect("preserve unconfigured coordinator-role metadata");
    assert_eq!(
        task_role_coordinator_external_failure.resulting_version,
        None
    );
    assert_eq!(
        task_role_coordinator_external_failure.payload["workflow_diagnostic"]["reason_code"],
        "workflow_status_unauthorized"
    );
    let after_task_role_coordinator = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_task_role_coordinator.status, "in_progress");
    assert_eq!(after_task_role_coordinator.version, 3);

    let coordinator_external_failure = db
        .append_group_workflow_event(
            &task.id,
            &coordinator_agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.failed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "configured coordinator records failed external check"
                }),
            },
        )
        .await
        .expect("configured coordinator can apply failed external check");
    assert_eq!(coordinator_external_failure.resulting_version, Some(4));
    assert_eq!(
        coordinator_external_failure.payload["new"]["status"],
        json!("blocked")
    );

    let writer_handoff = db
        .append_group_workflow_event(
            &task.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.ready_for_next_step".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "next_role": "writer"
                }),
            },
        )
        .await
        .expect("append writer handoff");
    assert_eq!(writer_handoff.resulting_version, Some(5));
    assert_eq!(
        writer_handoff.payload["new"]["status"],
        json!("in_progress")
    );

    let review_handoff = db
        .append_group_workflow_event(
            &task.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.ready_for_next_step".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "workflow": {
                        "next_role": "reviewer"
                    }
                }),
            },
        )
        .await
        .expect("append review handoff");
    assert_eq!(review_handoff.resulting_version, Some(6));
    assert_eq!(review_handoff.payload["new"]["status"], json!("in_review"));

    let created_with_status = db
        .append_group_workflow_event(
            &task.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.created".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "status": "completed"
                }),
            },
        )
        .await
        .expect("append created status event");
    assert_eq!(created_with_status.resulting_version, Some(7));
    assert_eq!(created_with_status.payload["new"]["status"], json!("done"));
    let after_created_status = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_created_status.status, "done");
    assert_eq!(after_created_status.version, 7);

    let invalid_created_status = db
        .append_group_workflow_event(
            &task.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.created".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "status": "waiting"
                }),
            },
        )
        .await
        .expect("preserve unsupported explicit status");
    assert_eq!(invalid_created_status.resulting_version, None);
    assert_eq!(
        invalid_created_status.payload["workflow_diagnostic"]["reason_code"],
        "workflow_status_unsupported"
    );
    let after_invalid_status = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_invalid_status.status, "done");
    assert_eq!(after_invalid_status.version, 7);

    let reviewer_feedback = db
        .append_group_workflow_event(
            &task.id,
            &reviewer_agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.feedback".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "review feedback"
                }),
            },
        )
        .await
        .expect("reviewer workflow feedback can update board status");
    assert_eq!(reviewer_feedback.resulting_version, Some(8));
    assert_eq!(
        reviewer_feedback.payload["new"]["status"],
        json!("in_progress")
    );

    let quality_feedback = db
        .append_group_workflow_event(
            &task.id,
            &quality_agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.feedback".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "quality feedback"
                }),
            },
        )
        .await
        .expect("quality_check workflow feedback can update board status");
    assert_eq!(quality_feedback.resulting_version, Some(9));
    assert_eq!(
        quality_feedback.payload["new"]["status"],
        json!("in_progress")
    );

    let approver_feedback = db
        .append_group_workflow_event(
            &task.id,
            &approver_agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.feedback".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "approval feedback"
                }),
            },
        )
        .await
        .expect("approver workflow feedback can update board status");
    assert_eq!(approver_feedback.resulting_version, Some(10));
    assert_eq!(
        approver_feedback.payload["new"]["status"],
        json!("in_progress")
    );

    for (actor_id, role_name) in [
        (quality_agent.id.as_str(), "quality_check"),
        (approver_agent.id.as_str(), "approver"),
    ] {
        let restricted_role_event = db
            .append_group_workflow_event(
                &task.id,
                actor_id,
                choruz_application::AppendGroupWorkflowEventRequest {
                    kind: "task.completed".into(),
                    task_key: Some("SYNC-1".into()),
                    source_message_id: None,
                    actor_principal_id: None,
                    payload: json!({
                        "note": format!("{role_name} cannot complete through workflow metadata")
                    }),
                },
            )
            .await
            .expect("preserve unauthorized restricted-role workflow metadata");
        assert_eq!(restricted_role_event.resulting_version, None);
        assert_eq!(
            restricted_role_event.payload["workflow_diagnostic"]["reason_code"],
            "workflow_status_unauthorized"
        );
    }
    let after_restricted_roles = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_restricted_roles.status, "in_progress");
    assert_eq!(after_restricted_roles.version, 10);

    let unauthorized_event = db
        .append_group_workflow_event(
            &task.id,
            &agent.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.completed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: Some(human.id.clone()),
                payload: json!({
                    "note": "agent cannot spoof the owner",
                    "workflow_authority": { "type": "trusted_system" },
                    "reason_code": "private_runtime_label",
                    "previous": { "status": "in_progress", "raw_internal": true },
                    "new": { "status": "done", "tool_diagnostics": ["unsafe"] }
                }),
            },
        )
        .await
        .expect("preserve unauthorized workflow metadata");
    assert_eq!(unauthorized_event.resulting_version, None);
    assert_eq!(
        unauthorized_event.payload["workflow_diagnostic"]["reason_code"],
        "workflow_status_unauthorized"
    );
    assert_eq!(
        unauthorized_event.payload["workflow_diagnostic"]["status_effect"],
        "done"
    );
    assert_eq!(unauthorized_event.payload.get("previous"), None);
    assert_eq!(unauthorized_event.payload.get("new"), None);
    assert_eq!(unauthorized_event.payload.get("reason_code"), None);
    assert_eq!(unauthorized_event.payload.get("workflow_authority"), None);

    let after_unauthorized = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_unauthorized.status, "in_progress");
    assert_eq!(after_unauthorized.version, 10);

    let detail = db
        .get_channel_task_detail(&human.id, &task.id)
        .await
        .expect("load redacted task detail");
    let projected_owner_noop = detail
        .events
        .iter()
        .find(|event| event.event_id == owner_agent_noop_check.id)
        .expect("accepted external_check.passed no-op is projected safely");
    assert_eq!(
        projected_owner_noop.workflow_kind.as_deref(),
        Some("external_check.passed")
    );
    assert_eq!(
        projected_owner_noop.actor_principal_id.as_deref(),
        Some(agent.id.as_str())
    );
    assert_eq!(projected_owner_noop.status_effect, None);
    assert_eq!(projected_owner_noop.previous, None);
    assert_eq!(projected_owner_noop.new, None);
    assert_eq!(projected_owner_noop.reason_code, None);
    assert!(
        detail
            .events
            .iter()
            .any(|projected| projected.event_id == event.id),
        "accepted mutating events must remain projectable even when input spoofed diagnostics"
    );
    assert!(
        detail
            .events
            .iter()
            .all(|event| event.event_id != unauthorized_event.id),
        "unauthorized diagnostic metadata must remain internal-only"
    );

    let unresolved_event = db
        .append_group_workflow_event_for_conversation(
            &conversation.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.completed".into(),
                task_key: Some("SYNC-MISSING".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "task key no longer resolves",
                    "previous": { "status": "in_progress", "raw_internal": true },
                    "new": { "status": "done", "tool_diagnostics": ["unsafe"] }
                }),
            },
        )
        .await
        .expect("preserve unresolved workflow metadata");
    assert_eq!(unresolved_event.task_id, None);
    assert_eq!(unresolved_event.resulting_version, None);
    assert_eq!(
        unresolved_event.payload["workflow_diagnostic"]["reason_code"],
        "workflow_task_unresolved"
    );
    assert_eq!(
        unresolved_event.payload["workflow_diagnostic"]["status_effect"],
        "done"
    );
    assert_eq!(unresolved_event.payload.get("previous"), None);
    assert_eq!(unresolved_event.payload.get("new"), None);

    let after_unresolved = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_unresolved.status, "in_progress");
    assert_eq!(after_unresolved.version, 10);

    let cross_conversation_task_id = db
        .append_group_workflow_event_for_conversation(
            &conversation.id,
            &human.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "task.completed".into(),
                task_key: Some("SYNC-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "task_id": other_task.id.clone(),
                    "note": "wrong conversation task id"
                }),
            },
        )
        .await
        .expect("preserve cross-conversation task id metadata");
    assert_eq!(cross_conversation_task_id.task_id, None);
    assert_eq!(cross_conversation_task_id.resulting_version, None);
    assert_eq!(
        cross_conversation_task_id.payload["workflow_diagnostic"]["reason_code"],
        "workflow_task_unresolved"
    );
    let after_cross_conversation = db.get_group_workflow_task(&task.id).await.unwrap();
    assert_eq!(after_cross_conversation.status, "in_progress");
    assert_eq!(after_cross_conversation.version, 10);
    let other_after_cross_conversation = db.get_group_workflow_task(&other_task.id).await.unwrap();
    assert_eq!(other_after_cross_conversation.status, "todo");
    assert_eq!(other_after_cross_conversation.version, 1);

    let trusted_external_failure = db
        .append_group_workflow_event_for_conversation_trusted_system(
            &other_conversation.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.failed".into(),
                task_key: Some("OTHER-1".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({
                    "note": "trusted server-side external check failure"
                }),
            },
        )
        .await
        .expect("trusted system path can apply failed external check");
    assert_eq!(trusted_external_failure.resulting_version, Some(2));
    assert_eq!(trusted_external_failure.actor_principal_id.as_deref(), None);
    assert_eq!(
        trusted_external_failure.payload["workflow_authority"]["type"],
        "trusted_system"
    );
    let other_after_trusted = db.get_group_workflow_task(&other_task.id).await.unwrap();
    assert_eq!(other_after_trusted.status, "blocked");
    assert_eq!(other_after_trusted.version, 2);

    let non_member_diagnostic = db
        .append_group_workflow_event_for_conversation(
            &conversation.id,
            &outsider.id,
            choruz_application::AppendGroupWorkflowEventRequest {
                kind: "external_check.passed".into(),
                task_key: Some("SYNC-MISSING".into()),
                source_message_id: None,
                actor_principal_id: None,
                payload: json!({ "note": "non-member diagnostic" }),
            },
        )
        .await;
    assert!(
        non_member_diagnostic.is_err(),
        "non-members cannot append diagnostic-only workflow metadata"
    );
}
