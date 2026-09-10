use super::DbService;
use choruz_common::AppError;
use choruz_domain::optimization::OptimizationSettings;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

pub struct ExperienceClaim {
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

pub struct ExperienceTurn {
    pub revision_id: String,
    pub instruction: String,
    pub team: Option<choruz_domain::team::Team>,
}

impl DbService {
    /// Capture the reviewed execution role from the selected revision. Disabled
    /// learning and rollback use the same policy pointer as prompt guidance.
    pub async fn experience_for_turn(
        &self,
        workspace_id: &str,
        binding_id: &str,
    ) -> Result<Option<ExperienceTurn>, AppError> {
        let client = self.store.connect().await?;
        let row = client.query_opt("SELECT r.id,r.instruction,CASE WHEN r.validation->'team'->>'review'='passed' THEN r.validation->'team'->'config' END AS team FROM experience_policy p JOIN experience_revision r ON r.id=p.active_revision_id AND r.binding_id=p.binding_id AND r.workspace_id=p.workspace_id WHERE p.binding_id=$1 AND p.workspace_id=$2 AND p.enabled AND r.disposition='active'", &[&binding_id,&workspace_id]).await
            .map_err(|e| AppError::Internal(format!("read execution role: {e}")))?;
        row.map(|row| {
            let team: Option<choruz_domain::team::Team> = row
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
        let client = self.store.connect().await?;
        let rows = client.query("SELECT c.id,b.agent_principal_id FROM conversation c JOIN conversation_member human ON human.conv_id=c.id AND human.principal_id=$1 AND human.removed_at IS NULL
            JOIN agent_runtime_bindings b ON b.id=$2 JOIN conversation_member agent ON agent.conv_id=c.id AND agent.principal_id=b.agent_principal_id
            WHERE c.workspace_id=$3 AND agent.removed_at IS NULL ORDER BY c.id", &[&claim.owner_id, &claim.binding_id, &claim.workspace_id]).await
            .map_err(|e| AppError::Internal(format!("read feedback scopes: {e}")))?;
        let mut cursors = claim.source_cursor["feedback"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut records = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        for row in rows {
            let id: String = row.get("id");
            let agent: String = row.get("agent_principal_id");
            let after = cursors.get(&id).and_then(Value::as_u64).unwrap_or(0);
            let messages = self.list_messages(&id, Some(100), Some(after)).await?;
            more |= messages.len() == 100;
            for message in messages {
                if message.sender_id != claim.owner_id && message.sender_id != agent {
                    cursors.insert(id.clone(), Value::from(message.server_seq));
                    continue;
                }
                let record = serde_json::json!({"ref":format!("message:{}",message.id),"kind":"conversation_feedback",
                    "conversation":id,"sender":message.sender_id,"owner_feedback":message.sender_id == claim.owner_id,
                    "content":message.content,"created_at":message.created_at});
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
                cursors.insert(id.clone(), Value::from(message.server_seq));
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
        let ids: Vec<_> = references
            .iter()
            .filter_map(|r| r.strip_prefix("message:"))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let client = self.store.connect().await?;
        let rows = client.query("SELECT e.event_id,e.conversation_id,e.seq FROM conversation_events e
            JOIN conversation c ON c.id=e.conversation_id
            JOIN agent_runtime_bindings b ON b.id=$2
            JOIN conversation_member human ON human.conv_id=c.id AND human.principal_id=$1 AND human.removed_at IS NULL
            JOIN conversation_member agent ON agent.conv_id=c.id AND agent.principal_id=b.agent_principal_id AND agent.removed_at IS NULL
            WHERE c.workspace_id=$3 AND e.event_id=ANY($4)
                AND e.event_type IN ('message','message.created','reply') AND (e.sender_id=$1 OR e.sender_id=b.agent_principal_id)",
            &[&claim.owner_id,&claim.binding_id,&claim.workspace_id,&ids]).await
            .map_err(|e| AppError::Internal(format!("verify historical learning feedback: {e}")))?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let conversation: String = row.get("conversation_id");
                let seq: i64 = row.get("seq");
                let after = claim.source_cursor["feedback"][&conversation].as_u64()?;
                (u64::try_from(seq).ok()? <= after)
                    .then(|| format!("message:{}", row.get::<_, String>("event_id")))
            })
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

    pub async fn active_experience(
        &self,
        workspace_id: &str,
        binding_id: &str,
    ) -> Result<Option<(String, String)>, AppError> {
        Ok(self
            .experience_for_turn(workspace_id, binding_id)
            .await?
            .map(|turn| (turn.revision_id, turn.instruction)))
    }
    /// A lease survives worker restarts and fences competing workers and settings changes.
    pub async fn claim_experience(&self) -> Result<Option<ExperienceClaim>, AppError> {
        let client = self.store.connect().await?;
        let token = choruz_common::new_id();
        let row = client.query_opt(
            "UPDATE experience_policy p SET lease_until=NOW()+INTERVAL '10 minutes', lease_token=$1
             WHERE binding_id=(SELECT binding_id FROM experience_policy WHERE enabled
                AND next_check_at<=NOW() AND (lease_until IS NULL OR lease_until<NOW())
                ORDER BY next_check_at,binding_id LIMIT 1 FOR UPDATE SKIP LOCKED)
             RETURNING p.*, COALESCE((SELECT instruction FROM experience_revision r WHERE r.id=p.active_revision_id), '') AS instruction,
                COALESCE((SELECT source_references FROM experience_revision r WHERE r.binding_id=p.binding_id ORDER BY r.created_at DESC,r.id DESC LIMIT 1),'[]'::jsonb) AS source_references",
            &[&token],
        ).await.map_err(|e| AppError::Internal(format!("claim learning job: {e}")))?;
        Ok(row.map(|row| ExperienceClaim {
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
        }))
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
        error: Option<&str>,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client.execute("UPDATE experience_policy SET lease_until=NULL,lease_token=NULL,checked_at=NOW(),next_check_at=NOW()+
            CASE WHEN $4::text IS NULL AND source_cursor->>'_more'='true' THEN INTERVAL '1 second' ELSE INTERVAL '2 minutes' END,last_error=$4
            WHERE binding_id=$1 AND workspace_id=$2 AND lease_token=$3",
            &[&claim.binding_id, &claim.workspace_id, &claim.token, &error]).await
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
                    tx.execute("INSERT INTO experience_problem_observation(binding_id,workspace_id,problem_key,episode_ref,evidence,applied_revision_id,report_id)
                        VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(binding_id,problem_key,episode_ref) DO NOTHING",
                        &[&claim.binding_id,&claim.workspace_id,&key,&episode,&problem["evidence"],&applied,&id]).await
                        .map_err(|e| AppError::Internal(format!("save learning observation: {e}")))?;
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
                    checked_at, last_error, optimization_settings, optimization_error FROM experience_policy
             WHERE binding_id=$1 AND workspace_id=$2 AND owner_id=$3",
                &[&binding_id, &workspace_id, &owner_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("read learning settings: {e}")))?;
        row.map(|row| {
            Ok(ExperiencePolicy {
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
             WHERE r.binding_id=$1 AND r.workspace_id=$2 AND p.workspace_id=$2 AND p.owner_id=$3
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
