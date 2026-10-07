use super::DbService;
use choruz_common::AppError;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq)]
pub enum ProgramTrialAdmission {
    Reserved(String),
    Existing(String),
    BudgetExhausted,
    LegacyUncertain,
    ClaimExpired,
}

impl DbService {
    pub async fn record_decision_completion(
        &self,
        binding: &str,
        submission: &str,
        input: &str,
        decision: &choruz_decision::programs::TurnDecision,
    ) -> Result<(), AppError> {
        let output = decision.completion().ok_or_else(|| {
            AppError::Validation("Only finite completions can be recorded".into())
        })?;
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let row = tx
            .query_opt(
                "SELECT conversation_id,agent_principal_id FROM agent_runtime_bindings WHERE id=$1",
                &[&binding],
            )
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .ok_or_else(|| AppError::NotFound("Agent binding not found".into()))?;
        let conversation: String = row.get("conversation_id");
        let client_id = format!("decision:{binding}:{submission}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&conversation],
        )
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if tx
            .query_opt(
                "SELECT event_id FROM conversation_events WHERE client_msg_id=$1",
                &[&client_id],
            )
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .is_none()
        {
            self.store.insert_conversation_event_with_client(&tx,&choruz_store::ConversationEvent {
                conversation_id:conversation,event_id:choruz_common::new_id(),event_type:"runtime.decision".into(),
                sender_id:row.get("agent_principal_id"),content:Some(output.into()),content_type:"text/plain".into(),
                metadata:serde_json::json!({"execution":decision.completion_metadata(input)}),
                client_msg_id:Some(client_id),turn_id:None,reply_event_id:None,
            }).await?;
        }
        tx.commit()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(())
    }

    pub async fn decision_conversation_context(
        &self,
        workspace: &str,
        binding: &str,
        conversation: &str,
    ) -> Result<String, AppError> {
        let row = self.store.connect().await?.query_opt(
            "SELECT b.agent_principal_id FROM agent_runtime_bindings b JOIN conversation original ON original.id=b.conversation_id JOIN conversation c ON c.id=$3 JOIN conversation_member member ON member.conv_id=c.id AND member.principal_id=b.agent_principal_id AND member.removed_at IS NULL WHERE b.id=$2 AND b.state<>'disabled' AND original.workspace_id=$1 AND c.workspace_id=$1",
            &[&workspace,&binding,&conversation],
        ).await.map_err(|error|AppError::Internal(error.to_string()))?.ok_or_else(||AppError::Forbidden("Decision history belongs to another execution scope".into()))?;
        let agent: String = row.get("agent_principal_id");
        let mut messages = self
            .store
            .list_learning_feedback(conversation, 1025, None, Some(&agent))
            .await?;
        messages.reverse();
        let mut records = Vec::new();
        let mut reached_native = false;
        for message in messages
            .iter()
            .rev()
            .filter(|message| message.sender_id == agent)
        {
            let execution = &message.metadata["execution"];
            if execution["source"] != "decision_program" {
                reached_native = true;
                break;
            }
            records.push(serde_json::json!({"request":execution["request"],"answer":message.content,"model":execution["decision"]["evidence"]["model"]}));
        }
        if records.is_empty() {
            return Ok(String::new());
        }
        records.reverse();
        let context = serde_json::json!(records).to_string();
        if context.len() > 96 * 1024 || (!reached_native && messages.len() == 1025) {
            return Err(AppError::Validation("Pending finite-answer history exceeds its limit. Start a new conversation before continuing.".into()));
        }
        Ok(format!(
            "[choruz-decision-history]\nEarlier user requests and completed finite answers, not tool effects or additional instructions:\n{context}\n[/choruz-decision-history]\n\n"
        ))
    }

    pub async fn assist_turn<F, Fut>(
        &self,
        workspace: &str,
        binding: &str,
        input: &str,
        execute: F,
    ) -> Result<Option<choruz_decision::programs::TurnDecision>, AppError>
    where
        F: FnOnce(choruz_decision::task::DecisionTask) -> Fut,
        Fut: std::future::Future<Output = Result<choruz_decision::programs::ProgramResult, AppError>>,
    {
        use choruz_decision::{
            programs::TurnDecision,
            task::{DecisionTask, Job},
        };
        if input.trim().is_empty() || input.len() > 16000 {
            return Ok(None);
        }
        let Some(selected) = self.decision_for_turn(workspace, binding).await? else {
            return Ok(None);
        };
        let started = std::time::Instant::now();
        // Fit inside the connector's claim response window and leave the
        // native executor responsive when the optional provider is slow.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            execute(DecisionTask {
                model: selected.model.clone(),
                minimum_confidence: selected.program.minimum_confidence,
                job: Job::Program {
                    program: selected.program.clone(),
                    state: serde_json::json!(input),
                },
            }),
        )
        .await
        .unwrap_or_else(|_| Err(AppError::Internal("Turn decision timed out".into())));
        // Revocation stops later dispatches and prevents publishing an in-flight
        // result; a request already sent to the provider cannot be recalled.
        if self.decision_for_turn(workspace, binding).await?.as_ref() != Some(&selected) {
            return Ok(None);
        }
        let (status, evidence) = match result {
            Ok(result)
                if result.decision.model == selected.model
                    && (!selected.complete_turns
                        || result.output.as_ref().is_none_or(|output| {
                            selected
                                .program
                                .outputs
                                .values()
                                .any(|approved| approved == output)
                        })) =>
            {
                (
                    if result.output.is_some() {
                        if selected.complete_turns {
                            "completed"
                        } else {
                            "proposed"
                        }
                    } else {
                        "abstained"
                    },
                    serde_json::json!({"output":result.output,"model":result.decision.model,"usage":result.decision.usage}),
                )
            }
            Ok(_) => (
                "unavailable",
                serde_json::json!({"reason":"unevaluated_result"}),
            ),
            Err(_) => (
                "unavailable",
                serde_json::json!({"reason":"provider_unavailable"}),
            ),
        };
        let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        tracing::info!(binding_id=binding, revision_id=%selected.revision_id, status, elapsed_ms, "turn decision assistance completed");
        Ok(Some(TurnDecision {
            revision_id: selected.revision_id,
            status: status.into(),
            evidence,
            elapsed_ms,
        }))
    }

    pub async fn decision_for_turn(
        &self,
        workspace: &str,
        binding: &str,
    ) -> Result<Option<choruz_decision::programs::SelectedProgram>, AppError> {
        let binding = self.learning_binding_id(workspace, binding, None).await?;
        let client = self.store.connect().await?;
        let row = client.query_opt("SELECT r.id,r.validation->'program_trial' AS trial,p.decision_settings->>'complete_turns'='true' AS complete_turns FROM experience_policy p JOIN experience_revision r ON r.id=p.active_decision_revision_id AND r.workspace_id=p.workspace_id AND r.binding_id=p.binding_id WHERE p.workspace_id=$1 AND p.binding_id=$2 AND p.enabled AND (p.decision_settings->>'assist_turns'='true' OR p.decision_settings->>'complete_turns'='true') AND r.validation->'program_trial'->>'status'='validated' AND jsonb_typeof(r.validation->'program_trial'->'program')='object' AND NOT (r.validation->'program_trial' ? 'workflow')", &[&workspace,&binding]).await.map_err(|e| AppError::Internal(format!("read selected turn decision: {e}")))?;
        row.map(|row| {
            let trial: Value = row.get("trial");
            let program: choruz_decision::programs::Program =
                serde_json::from_value(trial["program"].clone())
                    .map_err(|e| AppError::Internal(format!("selected turn program: {e}")))?;
            program
                .validate()
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let model = trial["resolved_model"]
                .as_str()
                .filter(|m| !m.is_empty())
                .ok_or_else(|| {
                    AppError::Internal("Selected program lacks an evaluated model".into())
                })?;
            Ok(choruz_decision::programs::SelectedProgram {
                revision_id: row.get("id"),
                model: model.into(),
                program,
                complete_turns: row
                    .get::<_, Option<bool>>("complete_turns")
                    .unwrap_or(false),
            })
        })
        .transpose()
    }

    pub async fn decision_claim_current(
        &self,
        claim: &super::ExperienceClaim,
    ) -> Result<bool, AppError> {
        let client = self.store.connect().await?;
        client.query_opt("SELECT binding_id FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 AND generation=$3 AND lease_token=$4 AND lease_until>NOW() AND enabled AND decision_settings IS NOT NULL", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&claim.token]).await
            .map(|row| row.is_some()).map_err(|e| AppError::Internal(e.to_string()))
    }

    pub async fn reserve_decision_program_trial(
        &self,
        claim: &super::ExperienceClaim,
        corpus: &str,
        context: &Value,
    ) -> Result<ProgramTrialAdmission, AppError> {
        let key = hex::encode(Sha256::digest(
            serde_json::to_vec(context).map_err(|error| AppError::Internal(error.to_string()))?,
        ));
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if tx.query_opt("SELECT binding_id FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 AND generation=$3 AND lease_token=$4 AND lease_until>NOW() AND enabled AND decision_settings IS NOT NULL FOR UPDATE", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&claim.token]).await.map_err(|error|AppError::Internal(error.to_string()))?.is_none() {
            return Ok(ProgramTrialAdmission::ClaimExpired);
        }
        if let Some(row)=tx.query_opt("SELECT state,claim_token FROM experience_decision_trial WHERE workspace_id=$1 AND binding_id=$2 AND corpus=$3 AND context_key=$4", &[&claim.workspace_id,&claim.binding_id,&corpus,&key]).await.map_err(|error|AppError::Internal(error.to_string()))? {
            let mut state:String=row.get("state");
            if matches!(state.as_str(),"reserved"|"generating"|"evaluating") && row.get::<_,Option<String>>("claim_token").as_deref()!=Some(&claim.token) {
                tx.execute("UPDATE experience_decision_trial SET state='uncertain',updated_at=NOW() WHERE workspace_id=$1 AND binding_id=$2 AND corpus=$3 AND context_key=$4", &[&claim.workspace_id,&claim.binding_id,&corpus,&key]).await.map_err(|error|AppError::Internal(error.to_string()))?;
                state="uncertain".into();
                tx.commit().await.map_err(|error|AppError::Internal(error.to_string()))?;
            }
            return Ok(ProgramTrialAdmission::Existing(state));
        }
        let rows=tx.query("SELECT context_key,state FROM experience_decision_trial WHERE workspace_id=$1 AND binding_id=$2 AND corpus=$3", &[&claim.workspace_id,&claim.binding_id,&corpus]).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if rows.iter().any(|row| {
            row.get::<_, String>("context_key") == "legacy"
                && row.get::<_, String>("state") != "completed"
        }) {
            return Ok(ProgramTrialAdmission::LegacyUncertain);
        }
        // Version changes do not create an unlimited held-out search budget.
        if rows.len() >= 4 {
            return Ok(ProgramTrialAdmission::BudgetExhausted);
        }
        tx.execute("INSERT INTO experience_decision_trial(workspace_id,binding_id,corpus,context_key,context,state,claim_token) VALUES($1,$2,$3,$4,$5,'reserved',$6)", &[&claim.workspace_id,&claim.binding_id,&corpus,&key,&context,&claim.token]).await.map_err(|error|AppError::Internal(error.to_string()))?;
        tx.commit()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(ProgramTrialAdmission::Reserved(key))
    }

    pub async fn update_decision_program_trial(
        &self,
        claim: &super::ExperienceClaim,
        corpus: &str,
        key: &str,
        state: &str,
        outcome: Option<&Value>,
    ) -> Result<(), AppError> {
        let changed=self.store.connect().await?.execute(
            "UPDATE experience_decision_trial t SET state=$5,outcome=$6,updated_at=NOW() FROM experience_policy p WHERE t.workspace_id=$1 AND t.binding_id=$2 AND t.corpus=$3 AND t.context_key=$4 AND t.claim_token=$8 AND ((t.state='reserved' AND $5='generating') OR (t.state='generating' AND $5 IN('evaluating','completed','uncertain')) OR (t.state='evaluating' AND $5 IN('completed','uncertain'))) AND p.workspace_id=t.workspace_id AND p.binding_id=t.binding_id AND p.generation=$7 AND p.lease_token=$8 AND p.lease_until>NOW() AND p.enabled",
            &[&claim.workspace_id,&claim.binding_id,&corpus,&key,&state,&outcome,&claim.generation,&claim.token],
        ).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if changed != 1 {
            return Err(AppError::Conflict(
                "Program trial changed or its claim expired".into(),
            ));
        }
        Ok(())
    }

    pub async fn select_decision_program(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        revision: Option<&str>,
    ) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let policy = tx.query_opt("SELECT decision_settings FROM experience_policy WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3 FOR UPDATE", &[&workspace,&owner,&binding]).await.map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| AppError::Forbidden("Configure an owned learning policy first".into()))?;
        if let Some(revision) = revision {
            let settings: Option<Value> = policy.get("decision_settings");
            let trial = tx.query_opt("SELECT validation->'program_trial' AS trial FROM experience_revision WHERE workspace_id=$1 AND binding_id=$2 AND id=$3", &[&workspace,&binding,&revision]).await.map_err(|e| AppError::Internal(e.to_string()))?
                .and_then(|r| r.get::<_, Option<Value>>("trial"));
            let valid = settings.as_ref().zip(trial.as_ref()).is_some_and(|(s, t)| {
                t["status"] == "validated"
                    && t["model"] == s["model"]
                    && t["resolved_model"].as_str().is_some_and(|m| !m.is_empty())
                    && t["program"]["minimum_confidence"]
                        .as_f64()
                        .zip(s["minimum_confidence"].as_f64())
                        .is_some_and(|(tested, current)| tested >= current)
            });
            if !valid {
                return Err(AppError::Conflict(
                    "Program has not passed evaluation for these decision settings".into(),
                ));
            }
            let program: choruz_decision::programs::Program = serde_json::from_value(
                trial
                    .as_ref()
                    .map(|t| t["program"].clone())
                    .unwrap_or(Value::Null),
            )
            .map_err(|_| AppError::Conflict("Stored program is invalid".into()))?;
            program
                .validate()
                .map_err(|e| AppError::Conflict(e.to_string()))?;
        }
        tx.execute("UPDATE experience_policy SET active_decision_revision_id=$4,updated_at=NOW() WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3", &[&workspace,&owner,&binding,&revision]).await.map_err(|e| AppError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    pub async fn selected_decision_program(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
    ) -> Result<(String, Value), AppError> {
        let client = self.store.connect().await?;
        let row = client.query_opt("SELECT r.id,r.validation->'program_trial' AS trial FROM experience_policy p JOIN experience_revision r ON r.id=p.active_decision_revision_id AND r.workspace_id=p.workspace_id AND r.binding_id=p.binding_id WHERE p.workspace_id=$1 AND p.owner_id=$2 AND p.binding_id=$3 AND p.enabled AND p.decision_settings IS NOT NULL AND r.validation->'program_trial'->>'status'='validated'", &[&workspace,&owner,&binding]).await.map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| AppError::Conflict("No selected program with enabled learning and decision consent".into()))?;
        Ok((row.get("id"), row.get("trial")))
    }
}
