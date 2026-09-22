use super::DbService;
use choruz_common::AppError;
use serde_json::Value;

impl DbService {
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
            Ok(result) if result.decision.model == selected.model => (
                if result.output.is_some() {
                    "proposed"
                } else {
                    "abstained"
                },
                serde_json::json!({"output":result.output,"model":result.decision.model,"usage":result.decision.usage}),
            ),
            Ok(_) => ("unavailable", serde_json::json!({"reason":"model_changed"})),
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
        let client = self.store.connect().await?;
        let row = client.query_opt("SELECT r.id,r.validation->'program_trial' AS trial FROM experience_policy p JOIN experience_revision r ON r.id=p.active_decision_revision_id AND r.workspace_id=p.workspace_id AND r.binding_id=p.binding_id WHERE p.workspace_id=$1 AND p.binding_id=$2 AND p.enabled AND p.decision_settings->>'assist_turns'='true' AND r.validation->'program_trial'->>'status'='validated'", &[&workspace,&binding]).await.map_err(|e| AppError::Internal(format!("read selected turn decision: {e}")))?;
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
    ) -> Result<bool, AppError> {
        let client = self.store.connect().await?;
        let inserted = client.execute(
            "INSERT INTO experience_decision_trial(workspace_id,binding_id,corpus) SELECT workspace_id,binding_id,$3 FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 AND generation=$4 AND lease_token=$5 AND lease_until>NOW() AND enabled AND decision_settings IS NOT NULL ON CONFLICT DO NOTHING",
            &[&claim.workspace_id, &claim.binding_id, &corpus, &claim.generation, &claim.token],
        ).await.map_err(|e| AppError::Internal(format!("reserve program trial: {e}")))?;
        Ok(inserted == 1)
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
