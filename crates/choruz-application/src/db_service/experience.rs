use super::DbService;
use choruz_common::AppError;
use choruz_evaluation::optimization::OptimizationSettings;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

pub struct ExperienceClaim {
    pub decision_settings: Option<choruz_decision::settings::LearningSettings>,
    pub trace_cases: bool,
    pub measured: bool,
    pub binding_id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub analyst_binding_id: String,
    pub generation: i64,
    pub token: String,
    pub active_revision_id: Option<String>,
    pub instruction: String,
    pub source_cursor: Value,
    pub source_summary: String,
    pub source_references: Value,
}

pub struct ExperienceReport<'a> {
    pub digest: &'a str,
    pub references: &'a Value,
    pub analysis: &'a str,
    pub instruction: Option<&'a str>,
    pub validation: &'a Value,
    pub checkpoint: Option<&'a Value>,
    pub activate: bool,
}

#[derive(Debug, Serialize)]
pub struct ExperiencePolicy {
    pub reuse_for_new_tasks: bool,
    pub shared_task_history: bool,
    pub active_decision_revision_id: Option<String>,
    pub decision_settings: Option<choruz_decision::settings::LearningSettings>,
    pub optimization_settings: Option<OptimizationSettings>,
    pub optimization_error: Option<String>,
    pub binding_id: String,
    pub analyst_binding_id: String,
    pub enabled: bool,
    pub generation: i64,
    pub active_revision_id: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExperienceRevision {
    pub id: String,
    pub parent_id: Option<String>,
    pub source_references: Value,
    pub analysis: String,
    pub instruction: String,
    pub disposition: String,
    pub validation: Value,
    pub created_at: DateTime<Utc>,
}

/// How a learning check ended, which decides when the next one runs.
pub enum CheckOutcome<'a> {
    Completed,
    /// Shown to the owner; the next check waits two minutes.
    Failed(&'a str),
    /// A transient condition such as a turn in progress: not an error.
    Deferred(std::time::Duration),
}

pub struct ExperienceTurn {
    pub revision_id: String,
    pub instruction: String,
    pub team: Option<choruz_evaluation::team::Team>,
}

async fn discard_window_checkpoints(
    tx: &deadpool_postgres::Transaction<'_>,
    claim: &ExperienceClaim,
) -> Result<(), AppError> {
    for table in [
        "experience_window_checkpoint",
        "experience_stage_checkpoint",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE workspace_id=$1 AND binding_id=$2 AND policy_generation=$3"),
            &[&claim.workspace_id, &claim.binding_id, &claim.generation],
        )
        .await
        .map_err(|e| AppError::Internal(format!("discard learning window: {e}")))?;
    }
    Ok(())
}

impl DbService {
    /// One private, projected source window per opted-in binding. Saving settings
    /// cancels it through the policy-generation trigger, including stored results.
    pub async fn experience_window(
        &self,
        claim: &ExperienceClaim,
        proposed: Option<&Value>,
    ) -> Result<Option<Value>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if tx.query_opt("SELECT binding_id FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 AND generation=$3 AND lease_token=$4 AND enabled AND lease_until>NOW() FOR UPDATE", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&claim.token]).await.map_err(|e|AppError::Internal(e.to_string()))?.is_none() {
            return Err(AppError::Conflict("Learning settings changed before restoring its window".into()));
        }
        if let Some(payload) = proposed {
            if payload.to_string().len() > 1024 * 1024 {
                return Err(AppError::Validation(
                    "Learning continuation exceeds its private storage bound".into(),
                ));
            }
            tx.execute("INSERT INTO experience_window_checkpoint(workspace_id,binding_id,policy_generation,payload) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&payload]).await.map_err(|e|AppError::Internal(e.to_string()))?;
        }
        let row = tx.query_opt("SELECT payload FROM experience_window_checkpoint WHERE workspace_id=$1 AND binding_id=$2 AND policy_generation=$3", &[&claim.workspace_id,&claim.binding_id,&claim.generation]).await.map_err(|e|AppError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(row.map(|row| row.get(0)))
    }

    pub async fn finish_experience_window(&self, claim: &ExperienceClaim) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if tx.query_opt("SELECT binding_id FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 AND generation=$3 AND lease_token=$4 AND enabled AND lease_until>NOW() FOR UPDATE", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&claim.token]).await.map_err(|e|AppError::Internal(e.to_string()))?.is_none() {
            return Err(AppError::Conflict("Learning settings changed before completing its window".into()));
        }
        discard_window_checkpoints(&tx, claim).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(())
    }

    /// Renew only the live reservation; saving settings or disabling consent fences it.
    pub async fn renew_experience(&self, claim: &ExperienceClaim) -> Result<bool, AppError> {
        Ok(self.store.connect().await?.execute(
            "UPDATE experience_policy SET lease_until=NOW()+INTERVAL '15 minutes' WHERE workspace_id=$1 AND binding_id=$2 AND generation=$3 AND lease_token=$4 AND enabled AND lease_until>NOW()",
            &[&claim.workspace_id,&claim.binding_id,&claim.generation,&claim.token],
        ).await.map_err(|e|AppError::Internal(format!("renew learning claim: {e}")))? == 1)
    }

    /// Successful pure model stages survive retries. An interrupted dispatch is
    /// not silently charged again; saving learning settings starts a new generation.
    pub async fn begin_experience_stage(
        &self,
        claim: &ExperienceClaim,
        stage: &str,
        digest: &str,
    ) -> Result<Option<Value>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if tx.query_opt("SELECT binding_id FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 AND generation=$3 AND lease_token=$4 AND enabled AND lease_until>NOW() FOR UPDATE", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&claim.token]).await.map_err(|e|AppError::Internal(e.to_string()))?.is_none() {
            return Err(AppError::Conflict("Learning settings changed or its reservation expired".into()));
        }
        let prior = tx.query_opt("SELECT state,result,attempts FROM experience_stage_checkpoint WHERE workspace_id=$1 AND binding_id=$2 AND policy_generation=$3 AND stage=$4 AND request_digest=$5", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&stage,&digest]).await.map_err(|e|AppError::Internal(e.to_string()))?;
        if let Some(row) = prior {
            match row.get::<_, String>("state").as_str() {
                "completed" => return Ok(Some(row.get("result"))),
                "retryable" => {
                    if row.get::<_, i32>("attempts") >= 3 {
                        return Err(AppError::Validation("Learning stage exhausted three observed attempts. Check the dependency and save learning settings to authorize another attempt.".into()));
                    }
                }
                _ => {
                    tx.execute("UPDATE experience_stage_checkpoint SET state='uncertain',updated_at=NOW() WHERE workspace_id=$1 AND binding_id=$2 AND policy_generation=$3 AND stage=$4 AND request_digest=$5 AND state='running'", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&stage,&digest]).await.map_err(|e|AppError::Internal(e.to_string()))?;
                    tx.commit()
                        .await
                        .map_err(|e| AppError::Internal(e.to_string()))?;
                    return Err(AppError::Conflict("A prior learning model call has an unknown outcome. Save learning settings to cancel this continuation and authorize a fresh attempt.".into()));
                }
            }
        }
        tx.execute("INSERT INTO experience_stage_checkpoint(workspace_id,binding_id,policy_generation,stage,request_digest,claim_token,state) VALUES($1,$2,$3,$4,$5,$6,'running') ON CONFLICT(workspace_id,binding_id,policy_generation,stage,request_digest) DO UPDATE SET state='running',claim_token=EXCLUDED.claim_token,result=NULL,attempts=experience_stage_checkpoint.attempts+1,updated_at=NOW()", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&stage,&digest,&claim.token]).await.map_err(|e|AppError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(None)
    }

    pub async fn finish_experience_stage(
        &self,
        claim: &ExperienceClaim,
        stage: &str,
        digest: &str,
        result: Option<&Value>,
        retryable: bool,
    ) -> Result<(), AppError> {
        let status = if result.is_some() {
            "completed"
        } else if retryable {
            "retryable"
        } else {
            "uncertain"
        };
        let changed = self.store.connect().await?.execute("UPDATE experience_stage_checkpoint s SET state=$7,result=$8,updated_at=NOW() FROM experience_policy p WHERE s.workspace_id=$1 AND s.binding_id=$2 AND s.policy_generation=$3 AND s.stage=$4 AND s.request_digest=$5 AND s.claim_token=$6 AND s.state='running' AND p.workspace_id=s.workspace_id AND p.binding_id=s.binding_id AND p.generation=s.policy_generation AND p.lease_token=$6 AND p.enabled AND p.lease_until>NOW()", &[&claim.workspace_id,&claim.binding_id,&claim.generation,&stage,&digest,&claim.token,&status,&result]).await.map_err(|e|AppError::Internal(e.to_string()))?;
        if changed != 1 {
            return Err(AppError::Conflict(
                "Learning settings changed before stage checkpoint".into(),
            ));
        }
        Ok(())
    }

    pub async fn configure_decisions(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        settings: Option<&choruz_decision::settings::LearningSettings>,
    ) -> Result<(), AppError> {
        if let Some(settings) = settings {
            settings
                .validate()
                .map_err(|e| AppError::Validation(e.to_string()))?;
        }
        let builder = settings.and_then(|s| s.builder_binding_id.as_deref());
        let client = self.store.connect().await?;
        let changed = client.execute("UPDATE experience_policy p SET decision_settings=$4, active_decision_revision_id=NULL, generation=generation+1,
            source_cursor=CASE WHEN $4::jsonb->>'builder_binding_id' IS DISTINCT FROM decision_settings->>'builder_binding_id' THEN '{}'::jsonb ELSE source_cursor END,
            next_check_at=NOW(),lease_token=NULL,lease_until=NULL,last_error=NULL,updated_at=NOW()
            WHERE binding_id=$1 AND workspace_id=$2 AND owner_id=$3
            AND ($5::text IS NULL OR EXISTS (SELECT 1 FROM agent_runtime_bindings b JOIN conversation c ON c.id=b.conversation_id WHERE b.id=$5 AND b.driver_type='codex_terminal' AND c.workspace_id=$2 AND b.id<>$1))",
            &[&binding,&workspace,&owner,&settings.map(|s| json!(s)),&builder]).await.map_err(|e| AppError::Internal(format!("save decision settings: {e}")))?;
        if changed != 1 {
            return Err(AppError::Forbidden("Configure owned learning and select a separate Codex builder in the same workspace".into()));
        }
        Ok(())
    }
    /// Capture the reviewed execution role from the selected revision. Disabled
    /// learning and rollback use the same policy pointer as prompt guidance.
    pub async fn experience_for_turn(
        &self,
        workspace_id: &str,
        binding_id: &str,
    ) -> Result<Option<ExperienceTurn>, AppError> {
        let client = self.store.connect().await?;
        let row = client.query_opt("SELECT r.id,r.instruction,CASE WHEN r.validation->'team'->>'review'='passed' THEN r.validation->'team'->'config' END AS team FROM experience_policy p JOIN experience_revision r ON r.id=p.active_revision_id AND r.binding_id=p.binding_id AND r.workspace_id=p.workspace_id WHERE p.binding_id=effective_experience_binding($2,$1,NULL) AND p.workspace_id=$2 AND p.enabled AND r.disposition='active'", &[&binding_id,&workspace_id]).await
            .map_err(|e| AppError::Internal(format!("read execution role: {e}")))?;
        row.map(|row| {
            let team: Option<choruz_evaluation::team::Team> = row
                .get::<_, Option<Value>>("team")
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| {
                    AppError::Internal(format!("Selected execution team is invalid: {e}"))
                })?;
            if let Some(team) = &team {
                team.validate(4).map_err(AppError::Validation)?;
            }
            Ok(ExperienceTurn {
                revision_id: row.get("id"),
                instruction: row.get("instruction"),
                team,
            })
        })
        .transpose()
    }
    /// Return only this policy's problem history. Counts refer to distinct work
    /// episodes, not model calls or repeated feedback about the same episode.
    pub async fn experience_problems(&self, claim: &ExperienceClaim) -> Result<Value, AppError> {
        let client = self.store.connect().await?;
        let rows = client.query(
            "SELECT p.problem_key,p.description,p.prompt_revision_id,
                COALESCE(jsonb_agg(jsonb_build_object('episode_ref',o.episode_ref,'applied_revision_id',o.applied_revision_id,'evidence',o.evidence)
                    ORDER BY o.created_at,o.episode_ref) FILTER (WHERE o.episode_ref IS NOT NULL),'[]'::jsonb) AS episodes
             FROM experience_problem p LEFT JOIN experience_problem_observation o
                ON o.binding_id=p.binding_id AND o.problem_key=p.problem_key AND o.workspace_id=p.workspace_id
             JOIN experience_policy policy ON policy.binding_id=p.binding_id AND policy.workspace_id=p.workspace_id
             WHERE p.binding_id=$1 AND p.workspace_id=$2 AND policy.owner_id=$3
             GROUP BY p.binding_id,p.problem_key ORDER BY p.created_at,p.problem_key",
            &[&claim.binding_id,&claim.workspace_id,&claim.owner_id],
        ).await.map_err(|e| AppError::Internal(format!("read learning problems: {e}")))?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|row| {
                    serde_json::json!({
                        "key":row.get::<_,String>("problem_key"),
                        "description":row.get::<_,String>("description"),
                        "prompt_revision_id":row.get::<_,Option<String>>("prompt_revision_id"),
                        "episodes":row.get::<_,Value>("episodes"),
                    })
                })
                .collect(),
        ))
    }
    /// Feedback is limited to conversations shared by this owner and the target
    /// Agent in the policy's Company. No other private conversation is exported.
    pub async fn experience_feedback(&self, claim: &ExperienceClaim) -> Result<Value, AppError> {
        let scopes = self
            .learning_feedback_scopes(&claim.workspace_id, &claim.owner_id, &claim.binding_id)
            .await?;
        let mut cursors = claim.source_cursor["feedback"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut records = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        for (id, agents) in scopes {
            let after = cursors.get(&id).and_then(Value::as_u64).unwrap_or(0);
            let messages = self
                .store
                .list_learning_feedback(
                    &id,
                    100,
                    Some(i64::try_from(after).map_err(|_| {
                        AppError::Validation("Feedback cursor exceeds its range".into())
                    })?),
                    None,
                )
                .await?;
            more |= messages.len() == 100;
            for message in messages {
                if message.sender_id != claim.owner_id && !agents.contains(&message.sender_id) {
                    cursors.insert(id.clone(), Value::from(message.seq));
                    continue;
                }
                let record = serde_json::json!({"ref":format!("message:{}",message.event_id),"kind":"conversation_feedback",
                    "conversation":id,"sender":message.sender_id,"owner_feedback":message.sender_id == claim.owner_id,
                    "content":message.content,"execution":message.metadata["execution"],"created_at":message.created_at});
                let size = record.to_string().len();
                if size > 160 * 1024 {
                    return Err(AppError::Validation(
                        "A feedback message exceeds the analysis window".into(),
                    ));
                }
                if bytes + size > 160 * 1024 {
                    more = true;
                    break;
                }
                bytes += size;
                cursors.insert(id.clone(), Value::from(message.seq));
                records.push(record);
            }
            if more {
                break;
            }
        }
        Ok(serde_json::json!({"records":records,"cursor":cursors,"more":more,"reset":false}))
    }
    /// Revalidate historical feedback only within the policy's current shared
    /// conversations and committed feedback cursors. Returns no message content.
    pub async fn experience_feedback_references(
        &self,
        claim: &ExperienceClaim,
        references: &[String],
    ) -> Result<Vec<String>, AppError> {
        self.experience_feedback_references_inner(claim, references, true)
            .await
    }

    /// Pending windows have not consumed their cursor, but every referenced
    /// feedback message must still belong to a currently shared conversation.
    pub async fn experience_window_feedback_current(
        &self,
        claim: &ExperienceClaim,
        references: &[String],
    ) -> Result<bool, AppError> {
        let mut references = references.to_vec();
        references.sort();
        references.dedup();
        Ok(self
            .experience_feedback_references_inner(claim, &references, false)
            .await?
            .len()
            == references.len())
    }

    async fn experience_feedback_references_inner(
        &self,
        claim: &ExperienceClaim,
        references: &[String],
        committed: bool,
    ) -> Result<Vec<String>, AppError> {
        Ok(self
            .learning_feedback_records(
                &claim.workspace_id,
                &claim.owner_id,
                &claim.binding_id,
                committed.then_some(&claim.source_cursor),
                references,
            )
            .await?
            .into_iter()
            .filter_map(|record| record["ref"].as_str().map(str::to_owned))
            .collect())
    }

    /// Select an already-reviewed revision for subsequent tasks. A running task
    /// retains its captured instruction. A null revision disables instruction use.
    pub async fn select_experience_revision(
        &self,
        workspace_id: &str,
        owner_id: &str,
        binding_id: &str,
        revision_id: Option<&str>,
        lease_token: Option<&str>,
    ) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin learning selection: {e}")))?;
        let policy = tx.query_opt("SELECT binding_id FROM experience_policy WHERE binding_id=$1 AND workspace_id=$2 AND owner_id=$3
            AND ($4::text IS NULL OR (lease_token=$4 AND enabled AND lease_until>NOW())) FOR UPDATE",
            &[&binding_id, &workspace_id, &owner_id, &lease_token]).await
            .map_err(|e| AppError::Internal(format!("lock learning policy: {e}")))?;
        if policy.is_none() {
            return Err(AppError::Conflict(
                "Learning settings changed before activation".into(),
            ));
        }
        if let Some(revision) = revision_id {
            let revision = tx.query_opt("SELECT id FROM experience_revision WHERE id=$1 AND binding_id=$2 AND workspace_id=$3
                AND validation->>'review'='passed' AND disposition IN ('candidate','active','superseded')",
                &[&revision, &binding_id, &workspace_id]).await
                .map_err(|e| AppError::Internal(format!("verify learning revision: {e}")))?;
            if revision.is_none() {
                return Err(AppError::Validation(
                    "Select a reviewed revision from this Agent".into(),
                ));
            }
        }
        activate_revision(
            &tx,
            workspace_id,
            binding_id,
            revision_id,
            lease_token.is_none(),
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit learning selection: {e}")))
    }

    /// A lease survives worker restarts and fences competing workers and settings changes.
    pub async fn claim_experience(&self) -> Result<Option<ExperienceClaim>, AppError> {
        let client = self.store.connect().await?;
        let token = choruz_common::new_id();
        let row = client.query_opt(
            "UPDATE experience_policy p SET lease_until=NOW()+INTERVAL '15 minutes', lease_token=$1
             WHERE binding_id=(SELECT binding_id FROM experience_policy WHERE enabled
                AND next_check_at<=NOW() AND (lease_until IS NULL OR lease_until<NOW())
                ORDER BY next_check_at,binding_id LIMIT 1 FOR UPDATE SKIP LOCKED)
             RETURNING p.*, COALESCE((SELECT instruction FROM experience_revision r WHERE r.id=p.active_revision_id), '') AS instruction,
                COALESCE((SELECT source_references FROM experience_revision r WHERE r.binding_id=p.binding_id ORDER BY r.created_at DESC,r.id DESC LIMIT 1),'[]'::jsonb) AS source_references",
            &[&token],
        ).await.map_err(|e| AppError::Internal(format!("claim learning job: {e}")))?;
        row.map(|row| {
            Ok(ExperienceClaim {
                decision_settings: row
                    .get::<_, Option<Value>>("decision_settings")
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| AppError::Internal("Invalid stored decision settings".into()))?,
                trace_cases: row
                    .get::<_, Option<Value>>("optimization_settings")
                    .is_some_and(|s| s["trace_cases"] == true)
                    || row
                        .get::<_, Option<Value>>("decision_settings")
                        .is_some_and(|s| s["builder_binding_id"].is_string()),
                measured: row
                    .get::<_, Option<Value>>("optimization_settings")
                    .is_some(),
                binding_id: row.get("binding_id"),
                workspace_id: row.get("workspace_id"),
                owner_id: row.get("owner_id"),
                analyst_binding_id: row.get("analyst_binding_id"),
                generation: row.get("generation"),
                token,
                active_revision_id: row.get("active_revision_id"),
                instruction: row.get("instruction"),
                source_cursor: row.get("source_cursor"),
                source_summary: row.get("source_summary"),
                source_references: row.get("source_references"),
            })
        })
        .transpose()
    }

    pub async fn experience_source_seen(
        &self,
        claim: &ExperienceClaim,
        digest: &str,
    ) -> Result<bool, AppError> {
        let client = self.store.connect().await?;
        Ok(client.query_opt("SELECT id FROM experience_revision WHERE binding_id=$1 AND workspace_id=$2 AND policy_generation=$3 AND source_digest=$4",
            &[&claim.binding_id, &claim.workspace_id, &claim.generation, &digest]).await
            .map_err(|e| AppError::Internal(format!("check learning source: {e}")))?.is_some())
    }

    pub async fn release_experience(
        &self,
        claim: &ExperienceClaim,
        outcome: CheckOutcome<'_>,
    ) -> Result<(), AppError> {
        let (error, defer_ms) = match outcome {
            CheckOutcome::Completed => (None, None),
            CheckOutcome::Failed(error) => (Some(error), None),
            CheckOutcome::Deferred(delay) => (
                None,
                Some(i64::try_from(delay.as_millis()).unwrap_or(i64::MAX)),
            ),
        };
        let client = self.store.connect().await?;
        client.execute("UPDATE experience_policy SET lease_until=NULL,lease_token=NULL,checked_at=NOW(),next_check_at=NOW()+
            CASE WHEN $5::bigint IS NOT NULL THEN $5::bigint * INTERVAL '1 millisecond'
                 WHEN $4::text IS NULL AND source_cursor->>'_more'='true' THEN INTERVAL '1 second' ELSE INTERVAL '2 minutes' END,last_error=$4
            WHERE binding_id=$1 AND workspace_id=$2 AND lease_token=$3",
            &[&claim.binding_id, &claim.workspace_id, &claim.token, &error, &defer_ms]).await
            .map_err(|e| AppError::Internal(format!("finish learning check: {e}")))?;
        Ok(())
    }

    /// Commit evidence, the read cursor and optional activation together. An obsolete
    /// lease cannot consume source records or change the active instruction.
    pub async fn save_experience_candidate(
        &self,
        claim: &ExperienceClaim,
        report: ExperienceReport<'_>,
    ) -> Result<Option<String>, AppError> {
        let ExperienceReport {
            digest,
            references,
            analysis,
            instruction,
            validation,
            checkpoint,
            activate,
        } = report;
        if activate && (instruction.is_none() || validation["review"] != "passed") {
            return Err(AppError::Validation(
                "Activation requires reviewed instruction evidence".into(),
            ));
        }
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin experience report: {e}")))?;
        if tx.query_opt("SELECT binding_id FROM experience_policy WHERE binding_id=$1 AND workspace_id=$2 AND lease_token=$3 AND enabled AND lease_until>NOW() FOR UPDATE",
            &[&claim.binding_id, &claim.workspace_id, &claim.token]).await.map_err(|e| AppError::Internal(format!("lock experience report: {e}")))?.is_none() {
            return Ok(None);
        }
        let id = choruz_common::new_id();
        let case_report = claim.trace_cases || validation["evaluation_cases"].is_array();
        let previous_cases = if case_report {
            super::trace_cases::read_trace_cases(&tx, &claim.workspace_id, &claim.binding_id)
                .await?
        } else {
            Vec::new()
        };
        let mut validation = validation.clone();
        if let Some(cases) = validation.get("evaluation_cases") {
            let cases: Vec<choruz_evaluation::evaluation::TraceCase> =
                serde_json::from_value(cases.clone())
                    .map_err(|_| AppError::Validation("Invalid evaluation cases".into()))?;
            validation["evaluation_cases"] = json!(choruz_evaluation::dataset::curate_changes(
                &previous_cases,
                &cases
            ));
        }
        let disposition = if instruction.is_some() {
            "candidate"
        } else {
            "no_change"
        };
        let row = tx.query_opt(
            "INSERT INTO experience_revision(id,binding_id,workspace_id,policy_generation,parent_id,source_digest,source_references,analysis,instruction,disposition,validation)
             SELECT $1,binding_id,workspace_id,generation,active_revision_id,$5,$6,$7,$8,$9,$10 FROM experience_policy
             WHERE binding_id=$2 AND workspace_id=$3 AND lease_token=$4 AND enabled AND lease_until>NOW()
             ON CONFLICT(binding_id,policy_generation,source_digest) DO NOTHING RETURNING id",
            &[&id, &claim.binding_id, &claim.workspace_id, &claim.token, &digest, &references,
                &analysis, &instruction.unwrap_or(""), &disposition, &validation],
        ).await.map_err(|e| AppError::Internal(format!("save learning candidate: {e}")))?;
        if row.is_some() {
            if case_report {
                let current = super::trace_cases::read_trace_cases(
                    &tx,
                    &claim.workspace_id,
                    &claim.binding_id,
                )
                .await?;
                if claim.trace_cases {
                    let dataset =
                        choruz_evaluation::dataset::dataset_report(&previous_cases, &current);
                    tx.execute("UPDATE experience_revision SET validation=jsonb_set(validation,'{dataset}',$2) WHERE id=$1 AND workspace_id=$3", &[&id,&dataset,&claim.workspace_id]).await.map_err(|e| AppError::Internal(format!("save dataset version: {e}")))?;
                }
                let changed =
                    choruz_evaluation::dataset::changed_case_contexts(&previous_cases, &current);
                // Corrections invalidate the evidence, not merely the current
                // selection. The old program cannot be selected again.
                tx.execute("UPDATE experience_revision SET validation=jsonb_set(validation,'{program_trial,status}','\"evidence_changed\"'::jsonb) WHERE binding_id=$1 AND workspace_id=$2 AND id<>$4 AND validation->'program_trial'->>'status'='validated' AND EXISTS (SELECT 1 FROM jsonb_array_elements(validation->'program_trial'->'suite'->'cases') c WHERE c->'source'->>'episode_ref'=ANY($3::text[]))", &[&claim.binding_id,&claim.workspace_id,&changed,&id]).await.map_err(|e| AppError::Internal(format!("invalidate program evidence: {e}")))?;
                tx.execute("UPDATE experience_policy p SET active_decision_revision_id=NULL WHERE p.binding_id=$1 AND p.workspace_id=$2 AND EXISTS (SELECT 1 FROM experience_revision r WHERE r.id=p.active_decision_revision_id AND r.validation->'program_trial'->>'status'='evidence_changed')", &[&claim.binding_id,&claim.workspace_id]).await.map_err(|e| AppError::Internal(format!("clear invalid program: {e}")))?;
                tx.execute("UPDATE experience_evaluation SET status='cancelled',error_code='case_evidence_changed',application_status=CASE WHEN auto_apply THEN 'cancelled' ELSE application_status END,lease_token=NULL,lease_until=NULL,updated_at=NOW() WHERE binding_id=$1 AND workspace_id=$2 AND status IN ('queued','running') AND EXISTS (SELECT 1 FROM jsonb_array_elements(suite->'cases') c WHERE c->'source'->>'episode_ref'=ANY($3::text[]))", &[&claim.binding_id,&claim.workspace_id,&changed]).await.map_err(|e| AppError::Internal(format!("invalidate changed trace evaluation: {e}")))?;
            }
            if let Some(problems) = validation["problems"].as_array() {
                for problem in problems {
                    let key = problem["key"].as_str().ok_or_else(|| {
                        AppError::Validation("Learning problem has no key".into())
                    })?;
                    let description = problem["description"].as_str().ok_or_else(|| {
                        AppError::Validation("Learning problem has no description".into())
                    })?;
                    let episode = problem["episode_ref"].as_str().ok_or_else(|| {
                        AppError::Validation("Learning problem has no episode".into())
                    })?;
                    let applied = problem["applied_revision_id"].as_str();
                    // The first activated prompt addressing this problem is the
                    // intervention to observe; later reports do not reset it.
                    let addressed = validation["addressed_problems"]
                        .as_array()
                        .is_some_and(|keys| keys.iter().any(|k| k == key));
                    let prompt_revision = (activate && addressed).then_some(id.as_str());
                    tx.execute("INSERT INTO experience_problem(binding_id,workspace_id,problem_key,description,prompt_revision_id)
                        VALUES($1,$2,$3,$4,$5) ON CONFLICT(binding_id,problem_key) DO UPDATE SET
                        prompt_revision_id=COALESCE(experience_problem.prompt_revision_id,EXCLUDED.prompt_revision_id)",
                        &[&claim.binding_id,&claim.workspace_id,&key,&description,&prompt_revision]).await
                        .map_err(|e| AppError::Internal(format!("save learning problem: {e}")))?;
                    if validation["review"] == "passed"
                        && let Some(source) =
                            validation["behavior_sources"]
                                .as_array()
                                .and_then(|sources| {
                                    sources.iter().find(|source| source["problem_key"] == key)
                                })
                        && let Some(problem_id) =
                            source["candidate"]["record"]["problem"]["id"].as_str()
                    {
                        let linked=tx.execute("UPDATE experience_problem p SET community_problem_id=$4 WHERE workspace_id=$1 AND binding_id=$2 AND problem_key=$3 AND community_problem_id IS DISTINCT FROM $4 AND NOT EXISTS(SELECT 1 FROM experience_behavior_event e WHERE e.binding_id=p.binding_id AND e.problem_key=p.problem_key AND e.public_payload IS NOT NULL)", &[&claim.workspace_id,&claim.binding_id,&key,&problem_id]).await
                            .map_err(|e|AppError::Internal(format!("link matching community problem: {e}")))?;
                        if linked != 0 {
                            tx.execute("UPDATE experience_behavior_event SET payload=CASE WHEN payload IS NULL THEN NULL ELSE jsonb_set(payload,'{problem,id}',to_jsonb($4::text)) END,lease_token=NULL,lease_until=NULL,updated_at=NOW() WHERE workspace_id=$1 AND binding_id=$2 AND problem_key=$3 AND public_payload IS NULL", &[&claim.workspace_id,&claim.binding_id,&key,&problem_id]).await.map_err(|e|AppError::Internal(format!("reconcile unpublished problem identity: {e}")))?;
                        }
                    }
                    tx.execute("INSERT INTO experience_problem_observation(binding_id,workspace_id,problem_key,episode_ref,evidence,applied_revision_id,report_id)
                        VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(binding_id,problem_key,episode_ref) DO NOTHING",
                        &[&claim.binding_id,&claim.workspace_id,&key,&episode,&problem["evidence"],&applied,&id]).await
                        .map_err(|e| AppError::Internal(format!("save learning observation: {e}")))?;
                    tx.execute("INSERT INTO experience_behavior_event(id,workspace_id,binding_id,problem_key,source_key,episode_ref,evidence_kind,solution_revision_id,source_references)
                        VALUES($1,$2,$3,$4,$5,$5,$6,$7,$8) ON CONFLICT(binding_id,problem_key,source_key) DO NOTHING",
                        &[&choruz_common::new_id(),&claim.workspace_id,&claim.binding_id,&key,&episode,&if applied.is_some(){"recurrence"}else{"encountered"},&applied,&problem["evidence"]]).await
                        .map_err(|e| AppError::Internal(format!("queue behavior evidence: {e}")))?;
                }
            }
            if let Some(outcomes) = validation["solution_outcomes"].as_array() {
                for outcome in outcomes {
                    let key = outcome["problem_key"].as_str().ok_or_else(|| {
                        AppError::Validation("Missing solution outcome problem".into())
                    })?;
                    let episode = outcome["episode_ref"].as_str().ok_or_else(|| {
                        AppError::Validation("Missing solution outcome episode".into())
                    })?;
                    let kind = outcome["outcome"]
                        .as_str()
                        .filter(|kind| matches!(*kind, "applied" | "effective" | "ineffective"))
                        .ok_or_else(|| AppError::Validation("Invalid solution outcome".into()))?;
                    let revision = claim.active_revision_id.as_deref().ok_or_else(|| {
                        AppError::Validation("Outcome lacks an applied revision".into())
                    })?;
                    let source_key = format!("{episode}:{revision}:{kind}");
                    tx.execute("INSERT INTO experience_behavior_event(id,workspace_id,binding_id,problem_key,source_key,episode_ref,evidence_kind,solution_revision_id,source_references)
                        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(binding_id,problem_key,source_key) DO NOTHING",
                        &[&choruz_common::new_id(),&claim.workspace_id,&claim.binding_id,&key,&source_key,&episode,&kind,&revision,&outcome["evidence"]]).await
                        .map_err(|e|AppError::Internal(format!("queue solution outcome: {e}")))?;
                }
            }
            if activate {
                activate_revision(
                    &tx,
                    &claim.workspace_id,
                    &claim.binding_id,
                    Some(&id),
                    false,
                )
                .await?;
            }
            tx.execute("UPDATE experience_policy SET source_cursor=COALESCE($2::jsonb,source_cursor),source_summary=CASE WHEN $2::jsonb IS NULL THEN source_summary ELSE $3 END,updated_at=NOW() WHERE binding_id=$1",
                &[&claim.binding_id, &checkpoint, &analysis]).await
                .map_err(|e| AppError::Internal(format!("checkpoint experience: {e}")))?;
            // A final report consumes its window in the same commit, so its
            // outcome is never visible while the window could still resume.
            if checkpoint.is_some() {
                discard_window_checkpoints(&tx, claim).await?;
            }
            if let Some(trace_id) = validation["trace_id"].as_str() {
                tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata,created_at) VALUES($1,$2,$3,'learning.check','agent_binding',$4,$5,NOW())",
                    &[&choruz_common::new_id(), &claim.workspace_id, &claim.owner_id, &claim.binding_id,
                      &json!({"trace_id":trace_id,"stage":"revision_commit","outcome":"committed","revision_id":id,"activated":activate,"policy_generation":claim.generation})]).await
                    .map_err(|e| AppError::Internal(format!("audit learning commit: {e}")))?;
            }
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit experience: {e}")))?;
        Ok(row.map(|row| row.get("id")))
    }

    pub async fn experience_policy(
        &self,
        workspace_id: &str,
        owner_id: &str,
        binding_id: &str,
    ) -> Result<Option<ExperiencePolicy>, AppError> {
        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "SELECT binding_id, analyst_binding_id, enabled, generation, active_revision_id,
                    checked_at, last_error, optimization_settings, optimization_error, decision_settings, active_decision_revision_id,
                    COALESCE((SELECT enabled FROM experience_task_profile profile WHERE profile.binding_id=experience_policy.binding_id),FALSE) AS reuse_for_new_tasks,
                    COALESCE((SELECT shared_task_history FROM experience_task_profile profile WHERE profile.binding_id=experience_policy.binding_id),FALSE) AS shared_task_history FROM experience_policy
             WHERE binding_id=effective_experience_binding($2,$1,$3) AND workspace_id=$2 AND owner_id=$3",
                &[&binding_id, &workspace_id, &owner_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("read learning settings: {e}")))?;
        row.map(|row| {
            Ok(ExperiencePolicy {
                reuse_for_new_tasks: row.get("reuse_for_new_tasks"),
                shared_task_history: row.get("shared_task_history"),
                active_decision_revision_id: row.get("active_decision_revision_id"),
                decision_settings: row
                    .get::<_, Option<Value>>("decision_settings")
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| AppError::Internal("Invalid stored decision settings".into()))?,
                optimization_error: row.get("optimization_error"),
                optimization_settings: row
                    .get::<_, Option<Value>>("optimization_settings")
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| AppError::Internal(format!("read optimization settings: {e}")))?,
                binding_id: row.get("binding_id"),
                analyst_binding_id: row.get("analyst_binding_id"),
                enabled: row.get("enabled"),
                generation: row.get("generation"),
                active_revision_id: row.get("active_revision_id"),
                checked_at: row.get("checked_at"),
                last_error: row.get("last_error"),
            })
        })
        .transpose()
    }

    /// Changing settings fences in-flight analysis. The gateway separately checks
    /// both binding permissions; this query also enforces their workspace scope.
    pub async fn configure_experience(
        &self,
        workspace_id: &str,
        owner_id: &str,
        binding_id: &str,
        analyst_binding_id: &str,
        enabled: bool,
        optimization_settings: Option<&OptimizationSettings>,
    ) -> Result<(), AppError> {
        if let Some(settings) = optimization_settings {
            settings.validate().map_err(AppError::Validation)?;
        }
        if binding_id == analyst_binding_id {
            return Err(AppError::Validation(
                "Choose a different Agent for background analysis".into(),
            ));
        }
        let client = self.store.connect().await?;
        let count = client.execute(
            "INSERT INTO experience_policy(binding_id, workspace_id, owner_id, analyst_binding_id, enabled, optimization_settings)
             SELECT target.id, $2, $3, analyst.id, $5, $6
             FROM agent_runtime_bindings target
             JOIN conversation tc ON tc.id=target.conversation_id
             JOIN agent_runtime_bindings analyst ON analyst.id=$4
             JOIN conversation ac ON ac.id=analyst.conversation_id
             WHERE target.id=$1 AND tc.workspace_id=$2 AND ac.workspace_id=$2
             ON CONFLICT(binding_id) DO UPDATE SET
                analyst_binding_id=EXCLUDED.analyst_binding_id, enabled=EXCLUDED.enabled,
                source_cursor=CASE WHEN EXCLUDED.optimization_settings->>'trace_cases'='true' AND experience_policy.optimization_settings->>'trace_cases' IS DISTINCT FROM 'true' THEN '{}'::jsonb ELSE experience_policy.source_cursor END,
                source_summary=CASE WHEN EXCLUDED.optimization_settings->>'trace_cases'='true' AND experience_policy.optimization_settings->>'trace_cases' IS DISTINCT FROM 'true' THEN '' ELSE experience_policy.source_summary END,
                optimization_settings=EXCLUDED.optimization_settings, optimization_error=NULL,
                generation=experience_policy.generation+1, next_check_at=NOW(),
                lease_token=NULL, lease_until=NULL, last_error=NULL, updated_at=NOW()
             WHERE experience_policy.workspace_id=$2 AND experience_policy.owner_id=$3",
            &[&binding_id, &workspace_id, &owner_id, &analyst_binding_id, &enabled, &optimization_settings.map(|s| json!(s))],
        ).await.map_err(|e| AppError::Internal(format!("save learning settings: {e}")))?;
        if count != 1 {
            return Err(AppError::Forbidden(
                "Learning settings belong to another scope".into(),
            ));
        }
        Ok(())
    }

    pub async fn experience_revisions(
        &self,
        workspace_id: &str,
        owner_id: &str,
        binding_id: &str,
    ) -> Result<Vec<ExperienceRevision>, AppError> {
        let client = self.store.connect().await?;
        let rows = client.query(
            "SELECT r.* FROM experience_revision r JOIN experience_policy p ON p.binding_id=r.binding_id
             WHERE r.binding_id=effective_experience_binding($2,$1,$3) AND r.workspace_id=$2 AND p.workspace_id=$2 AND p.owner_id=$3
             ORDER BY r.created_at DESC,r.id DESC LIMIT 50",
            &[&binding_id, &workspace_id, &owner_id],
        ).await.map_err(|e| AppError::Internal(format!("read learning revisions: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| ExperienceRevision {
                id: row.get("id"),
                parent_id: row.get("parent_id"),
                source_references: row.get("source_references"),
                analysis: row.get("analysis"),
                instruction: row.get("instruction"),
                disposition: row.get("disposition"),
                validation: row.get("validation"),
                created_at: row.get("created_at"),
            })
            .collect())
    }
}

/// Callers hold the policy lock and validate their own lease and review evidence.
pub(super) async fn activate_revision(
    tx: &tokio_postgres::Transaction<'_>,
    workspace: &str,
    binding: &str,
    revision: Option<&str>,
    invalidate: bool,
) -> Result<(), AppError> {
    tx.execute("UPDATE experience_revision SET disposition='superseded' WHERE binding_id=$1 AND workspace_id=$2 AND disposition='active'", &[&binding,&workspace]).await.map_err(|e| AppError::Internal(e.to_string()))?;
    tx.execute("UPDATE experience_revision SET disposition='active' WHERE id=$1 AND binding_id=$2 AND workspace_id=$3", &[&revision,&binding,&workspace]).await.map_err(|e| AppError::Internal(e.to_string()))?;
    tx.execute("UPDATE experience_policy SET active_revision_id=$1,updated_at=NOW(),generation=generation+CASE WHEN $4 THEN 1 ELSE 0 END,lease_token=CASE WHEN $4 THEN NULL ELSE lease_token END,lease_until=CASE WHEN $4 THEN NULL ELSE lease_until END WHERE binding_id=$2 AND workspace_id=$3", &[&revision,&binding,&workspace,&invalidate]).await.map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}
