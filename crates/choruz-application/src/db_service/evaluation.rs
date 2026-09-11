use super::DbService;
use choruz_common::AppError;
use choruz_domain::evaluation::{EvaluationCandidate, EvaluationSuite};
use choruz_domain::optimization::{Optimization, OptimizationConfig};
use serde_json::{Value, json};

pub struct EvaluationClaim {
    pub final_review: bool,
    pub id: String,
    pub binding_id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub token: String,
    pub suite: EvaluationSuite,
    pub candidates: Vec<EvaluationCandidate>,
    pub context_fingerprint: String,
    pub next_case: usize,
    pub optimization: Option<Optimization>,
    pub analyst_binding_id: Option<String>,
    pub analyst_fingerprint: Option<String>,
}

pub struct EvaluationContext {
    pub fingerprint: String,
    pub optimization: Option<OptimizationSetup>,
    pub automatic_generation: Option<i64>,
}

pub struct OptimizationSetup {
    pub config: OptimizationConfig,
    pub analyst_binding_id: String,
    pub analyst_fingerprint: String,
}

impl DbService {
    pub async fn optimization_queue_error(
        &self,
        job: &Value,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client.execute("UPDATE experience_policy SET optimization_error=$4 WHERE binding_id=$1 AND workspace_id=$2 AND generation=$3", &[&job["binding"].as_str(),&job["workspace"].as_str(),&job["generation"].as_i64(),&error]).await.map_err(db_error)?;
        Ok(())
    }
    pub async fn pending_experience_optimizations(&self) -> Result<Vec<Value>, AppError> {
        let client = self.store.connect().await?;
        Ok(client.query("SELECT jsonb_build_object('binding',p.binding_id,'workspace',p.workspace_id,'owner',p.owner_id,'analyst',p.analyst_binding_id,'generation',p.generation,'settings',p.optimization_settings,'revision',r.id) AS job FROM experience_policy p CROSS JOIN LATERAL (SELECT id FROM experience_revision r WHERE r.binding_id=p.binding_id AND r.workspace_id=p.workspace_id AND r.policy_generation=p.generation AND r.disposition='candidate' AND r.validation->>'review'='passed' AND NOT EXISTS(SELECT 1 FROM experience_evaluation e WHERE e.binding_id=p.binding_id AND e.policy_generation=p.generation AND e.revision_id=r.id AND e.automatic) ORDER BY r.created_at DESC,r.id DESC LIMIT 1) r WHERE p.enabled AND p.optimization_settings IS NOT NULL AND NOT EXISTS(SELECT 1 FROM experience_evaluation e WHERE e.binding_id=p.binding_id AND e.status IN ('queued','running')) ORDER BY p.updated_at LIMIT 20", &[]).await.map_err(db_error)?.into_iter().map(|row|row.get("job")).collect())
    }

    pub async fn queue_experience_evaluation(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        revision: &str,
        suite: &EvaluationSuite,
        context: &EvaluationContext,
    ) -> Result<String, AppError> {
        suite.validate().map_err(AppError::Validation)?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(db_error)?;
        let policy = tx.query_opt("SELECT generation,active_revision_id,analyst_binding_id,optimization_settings FROM experience_policy WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3 AND enabled FOR UPDATE", &[&workspace,&owner,&binding]).await.map_err(db_error)?
            .ok_or_else(|| AppError::NotFound("Enabled learning policy not found".into()))?;
        let generation: i64 = policy.get("generation");
        if context.automatic_generation.is_some() && suite.cases.iter().any(|c| c.source.is_some())
        {
            let current = super::trace_cases::read_trace_cases(&tx, workspace, binding).await?;
            if suite.name
                != format!(
                    "Observed objectives {}",
                    super::trace_cases::corpus_version(&current)
                )
            {
                return Err(AppError::Conflict("Dataset changed before queueing".into()));
            }
        }
        // The policy lock serializes this snapshot check with case corrections.
        // A correction committed after corpus selection must not queue stale truth.
        for source in suite.cases.iter().filter_map(|case| case.source.as_ref()) {
            let latest = tx.query_opt("SELECT c AS source FROM experience_revision r CROSS JOIN LATERAL jsonb_array_elements(COALESCE(r.validation->'evaluation_cases','[]'::jsonb)) c WHERE r.workspace_id=$1 AND r.binding_id=$2 AND c->>'episode_ref'=$3 ORDER BY r.created_at DESC,r.id DESC LIMIT 1", &[&workspace,&binding,&source.episode_ref]).await.map_err(db_error)?;
            if latest.is_none_or(|row| row.get::<_, Value>("source") != json!(source)) {
                return Err(AppError::Conflict(
                    "Trace case evidence changed before queueing".into(),
                ));
            }
        }
        let automatic = context.automatic_generation.is_some();
        if context
            .automatic_generation
            .is_some_and(|expected| expected != generation)
        {
            return Err(AppError::Conflict(
                "Learning settings changed before evaluation".into(),
            ));
        }
        let settings: Option<Value> = policy.get("optimization_settings");
        let auto_apply = automatic && settings.as_ref().is_some_and(|s| s["auto_apply"] == true);
        let baseline: Option<String> = policy.get("active_revision_id");
        if baseline.as_deref() == Some(revision) {
            return Err(AppError::Conflict(
                "Choose a candidate other than the active revision".into(),
            ));
        }
        let mut candidates = Vec::new();
        let mut recurring_problem = false;
        for id in [baseline.as_deref(), Some(revision)] {
            let candidate = match id {
                Some(id) => {
                    let row = tx.query_opt("SELECT instruction,validation FROM experience_revision WHERE id=$1 AND binding_id=$2 AND workspace_id=$3 AND validation->>'review'='passed' AND disposition IN ('candidate','active','superseded')", &[&id,&binding,&workspace]).await.map_err(db_error)?
                        .ok_or_else(|| AppError::Conflict("Evaluation requires a reviewed revision from this Agent".into()))?;
                    let validation: Value = row.get("validation");
                    if id == revision {
                        recurring_problem = validation["source_complete"] == true
                            && validation["escalation_candidates"]
                                .as_array()
                                .is_some_and(|keys| !keys.is_empty());
                    }
                    EvaluationCandidate {
                        revision_id: Some(id.into()),
                        instruction: row.get("instruction"),
                        team: if validation["team"]["review"] == "passed" {
                            Some(
                                serde_json::from_value(validation["team"]["config"].clone())
                                    .map_err(|e| {
                                        AppError::Validation(format!("Invalid reviewed team: {e}"))
                                    })?,
                            )
                        } else {
                            None
                        },
                    }
                }
                None => EvaluationCandidate {
                    revision_id: None,
                    instruction: String::new(),
                    team: None,
                },
            };
            candidates.push(candidate);
        }
        let (optimization, analyst_id, analyst_fingerprint) =
            if let Some(setup) = &context.optimization {
                if policy.get::<_, String>("analyst_binding_id") != setup.analyst_binding_id {
                    return Err(AppError::Conflict("The selected analyst changed".into()));
                }
                let mut config = setup.config.clone();
                config.evolve_team &= recurring_problem
                    || candidates.iter().any(|candidate| candidate.team.is_some());
                (
                    Some(json!(
                        Optimization::new(config, suite, candidates.clone())
                            .map_err(AppError::Validation)?
                    )),
                    Some(&setup.analyst_binding_id),
                    Some(&setup.analyst_fingerprint),
                )
            } else {
                (None, None, None)
            };
        let id = choruz_common::new_id();
        let inserted = tx.execute("INSERT INTO experience_evaluation(id,binding_id,workspace_id,owner_id,revision_id,policy_generation,suite,candidates,context_fingerprint,optimization,analyst_binding_id,analyst_fingerprint,automatic,auto_apply,application_status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,CASE WHEN $14 THEN 'pending' ELSE 'not_requested' END) ON CONFLICT DO NOTHING", &[&id,&binding,&workspace,&owner,&revision,&generation,&json!(suite),&json!(candidates),&context.fingerprint,&optimization,&analyst_id,&analyst_fingerprint,&automatic,&auto_apply]).await.map_err(db_error)?;
        if inserted == 0 {
            return Err(AppError::Conflict(
                "An evaluation is already pending for this Agent".into(),
            ));
        }
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'experience.evaluate','runtime_binding',$4,$5)", &[&choruz_common::new_id(),&workspace,&owner,&binding,&json!({"evaluation_id":id,"revision_id":revision,"case_count":suite.cases.len()})]).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(id)
    }

    pub async fn experience_evaluations(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        id: Option<&str>,
    ) -> Result<Vec<Value>, AppError> {
        let client = self.store.connect().await?;
        client.query("SELECT optimization,suite,CASE WHEN $4::text IS NULL THEN jsonb_build_object('id',id,'revision_id',revision_id,'status',status,'next_case',next_case,'case_count',jsonb_array_length(suite->'cases'),'name',suite->>'name','error_code',error_code,'created_at',created_at,'updated_at',updated_at,'automatic',automatic,'application_status',application_status,'applied_revision_id',applied_revision_id) ELSE to_jsonb(e)-'lease_token'-'lease_until' END AS report FROM experience_evaluation e WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3 AND ($4::text IS NULL OR id=$4) ORDER BY created_at DESC LIMIT 20", &[&workspace,&owner,&binding,&id]).await.map_err(db_error)?
            .into_iter().map(|row| {
                let mut report: Value = row.get("report");
                if let Some(value) = row.get::<_,Option<Value>>("optimization") {
                    let search: Optimization = serde_json::from_value(value).map_err(|e| AppError::Internal(e.to_string()))?;
                    let suite = serde_json::from_value(row.get("suite")).map_err(|e| AppError::Internal(e.to_string()))?;
                    report["search_summary"] = json!({"metric_calls":search.metric_calls,"proposal_calls":search.proposal_calls,"model_calls_reserved":search.model_calls_reserved,"candidate_count":search.candidates.len(),"winner":search.winner,"scores":search.application_scores(&suite)});
                }
                Ok(report)
            }).collect()
    }

    /// A lost lease denotes an uncertain model call, not permission to spend the
    /// same budget again. Completed cases remain available in failed runs.
    pub async fn claim_experience_evaluation(&self) -> Result<Option<EvaluationClaim>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(db_error)?;
        tx.execute("UPDATE experience_evaluation e SET status='cancelled',application_status=CASE WHEN auto_apply THEN 'cancelled' ELSE application_status END,error_code='policy_changed',updated_at=NOW(),lease_token=NULL,lease_until=NULL FROM experience_policy p WHERE e.binding_id=p.binding_id AND e.workspace_id=p.workspace_id AND e.status IN ('queued','running') AND (NOT p.enabled OR p.generation<>e.policy_generation OR p.active_revision_id IS DISTINCT FROM e.candidates->0->>'revision_id')", &[]).await.map_err(db_error)?;
        tx.execute("UPDATE experience_evaluation SET status='failed',application_status=CASE WHEN auto_apply THEN 'failed' ELSE application_status END,error_code='interrupted',updated_at=NOW(),lease_token=NULL,lease_until=NULL WHERE status='running' AND lease_until<NOW()", &[]).await.map_err(db_error)?;
        let token = choruz_common::new_id();
        // Covers bounded team preparation (70s), replay and cleanup (170s),
        // and independent judging (75s), without renewing a lost execution.
        let row = tx.query_opt("UPDATE experience_evaluation SET status='running',lease_token=$1,lease_until=NOW()+INTERVAL '6 minutes',updated_at=NOW() WHERE id=(SELECT id FROM experience_evaluation WHERE status='queued' ORDER BY updated_at,id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING *", &[&token]).await.map_err(db_error)?;
        let mut claim = row
            .map(|row| {
                Ok(EvaluationClaim {
                    final_review: false,
                    id: row.get("id"),
                    binding_id: row.get("binding_id"),
                    workspace_id: row.get("workspace_id"),
                    owner_id: row.get("owner_id"),
                    token,
                    suite: serde_json::from_value(row.get("suite"))
                        .map_err(|e| AppError::Internal(e.to_string()))?,
                    candidates: serde_json::from_value(row.get("candidates"))
                        .map_err(|e| AppError::Internal(e.to_string()))?,
                    context_fingerprint: row.get("context_fingerprint"),
                    next_case: row.get::<_, i32>("next_case") as usize,
                    optimization: row
                        .get::<_, Option<Value>>("optimization")
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|e| AppError::Internal(e.to_string()))?,
                    analyst_binding_id: row.get("analyst_binding_id"),
                    analyst_fingerprint: row.get("analyst_fingerprint"),
                })
            })
            .transpose()?;
        if let Some(claim) = &mut claim {
            if let Some(search) = &mut claim.optimization {
                let action = search
                    .next_action(&claim.suite)
                    .map_err(AppError::Validation)?;
                let auto_apply: bool = tx
                    .query_one(
                        "SELECT auto_apply FROM experience_evaluation WHERE id=$1",
                        &[&claim.id],
                    )
                    .await
                    .map_err(db_error)?
                    .get(0);
                claim.final_review =
                    action.is_none() && auto_apply && search.can_apply(&claim.suite);
                if claim.final_review {
                    search.model_calls_reserved += 1;
                }
                let status = if action.is_some() || claim.final_review {
                    "running"
                } else {
                    "completed"
                };
                tx.execute("UPDATE experience_evaluation SET optimization=$2,status=$3,final_review_reserved=$4,application_status=CASE WHEN $3='completed' AND auto_apply THEN 'no_improvement' ELSE application_status END,lease_token=CASE WHEN $3='completed' THEN NULL ELSE lease_token END,lease_until=CASE WHEN $3='completed' THEN NULL ELSE lease_until END WHERE id=$1", &[&claim.id,&json!(search),&status,&claim.final_review]).await.map_err(db_error)?;
                if action.is_none() && !claim.final_review {
                    tx.commit().await.map_err(db_error)?;
                    return Ok(None);
                }
            }
        }
        tx.commit().await.map_err(db_error)?;
        Ok(claim)
    }

    pub async fn finish_experience_evaluation_case(
        &self,
        claim: &EvaluationClaim,
        result: &Value,
        error_code: Option<&str>,
    ) -> Result<bool, AppError> {
        if claim.final_review && error_code.is_none() {
            return self.finish_optimization_review(claim, result).await;
        }
        let status = if error_code.is_some() {
            "failed"
        } else if claim.optimization.is_none()
            && claim.next_case + 1 == claim.suite.cases.len() * claim.candidates.len()
        {
            "completed"
        } else {
            "queued"
        };
        let client = self.store.connect().await?;
        let optimization = claim.optimization.as_ref().map(|s| json!(s));
        let changed = client.execute("UPDATE experience_evaluation e SET results=results || jsonb_build_array($4::jsonb),next_case=next_case+1,status=$5,application_status=CASE WHEN $5='failed' AND auto_apply THEN 'failed' ELSE application_status END,error_code=$6,optimization=$7,lease_token=NULL,lease_until=NULL,updated_at=NOW() FROM experience_policy p WHERE e.id=$1 AND e.workspace_id=$2 AND e.lease_token=$3 AND e.status='running' AND e.lease_until>NOW() AND p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id AND p.enabled AND p.generation=e.policy_generation AND p.active_revision_id IS NOT DISTINCT FROM e.candidates->0->>'revision_id'", &[&claim.id,&claim.workspace_id,&claim.token,&result,&status,&error_code,&optimization]).await.map_err(db_error)?;
        Ok(changed == 1)
    }

    pub async fn evaluation_seed_evidence(
        &self,
        claim: &EvaluationClaim,
    ) -> Result<Value, AppError> {
        let client = self.store.connect().await?;
        let row = client.query_one("SELECT r.analysis,r.source_references,r.validation FROM experience_revision r JOIN experience_evaluation e ON e.revision_id=r.id AND e.workspace_id=r.workspace_id AND e.binding_id=r.binding_id WHERE e.id=$1 AND e.workspace_id=$2 AND e.owner_id=$3", &[&claim.id,&claim.workspace_id,&claim.owner_id]).await.map_err(db_error)?;
        Ok(
            json!({"analysis":row.get::<_,String>("analysis"),"references":row.get::<_,Value>("source_references"),"validation":row.get::<_,Value>("validation")}),
        )
    }

    async fn finish_optimization_review(
        &self,
        claim: &EvaluationClaim,
        result: &Value,
    ) -> Result<bool, AppError> {
        let search = claim
            .optimization
            .as_ref()
            .filter(|s| s.can_apply(&claim.suite))
            .ok_or_else(|| AppError::Validation("Optimization has no eligible winner".into()))?;
        let winner = &search.candidates[search
            .winner
            .ok_or_else(|| AppError::Validation("Missing winner".into()))?]
        .guidance;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(db_error)?;
        // Lock the policy before the job, matching enqueue and cancellation order.
        let valid = tx.query_opt("SELECT p.binding_id FROM experience_policy p JOIN experience_evaluation e ON e.binding_id=p.binding_id AND e.workspace_id=p.workspace_id WHERE e.id=$1 AND e.workspace_id=$2 AND e.owner_id=$3 AND e.lease_token=$4 AND e.lease_until>NOW() AND e.status='running' AND e.final_review_reserved AND e.auto_apply AND p.enabled AND p.generation=e.policy_generation AND p.active_revision_id IS NOT DISTINCT FROM e.candidates->0->>'revision_id' FOR UPDATE OF p,e", &[&claim.id,&claim.workspace_id,&claim.owner_id,&claim.token]).await.map_err(db_error)?;
        if valid.is_none() {
            return Ok(false);
        }
        let mut applied = None;
        let status = if result["review_passed"] == true {
            let seed = tx.query_one("SELECT r.* FROM experience_revision r JOIN experience_evaluation e ON e.revision_id=r.id WHERE e.id=$1 AND r.workspace_id=$2 AND r.binding_id=$3", &[&claim.id,&claim.workspace_id,&claim.binding_id]).await.map_err(db_error)?;
            let mut validation: Value = seed.get("validation");
            validation["evaluation_id"] = json!(claim.id);
            validation["review_details"] = result.clone();
            validation["team"] = winner
                .team
                .as_ref()
                .map(|team| json!({"config":team,"review":"passed"}))
                .unwrap_or(Value::Null);
            let id = choruz_common::new_id();
            tx.execute("INSERT INTO experience_revision(id,binding_id,workspace_id,policy_generation,parent_id,source_digest,source_references,analysis,instruction,disposition,validation) SELECT $1,e.binding_id,e.workspace_id,e.policy_generation,e.candidates->0->>'revision_id',$2,$3,$4,$5,'candidate',$6 FROM experience_evaluation e WHERE e.id=$7", &[&id,&format!("evaluation:{}",claim.id),&seed.get::<_,Value>("source_references"),&seed.get::<_,String>("analysis"),&winner.instruction,&validation,&claim.id]).await.map_err(db_error)?;
            super::experience::activate_revision(
                &tx,
                &claim.workspace_id,
                &claim.binding_id,
                Some(&id),
                true,
            )
            .await?;
            if let Some(keys) = validation["addressed_problems"].as_array() {
                for key in keys.iter().filter_map(Value::as_str) {
                    tx.execute("UPDATE experience_problem SET prompt_revision_id=COALESCE(prompt_revision_id,$1) WHERE workspace_id=$2 AND binding_id=$3 AND problem_key=$4", &[&id,&claim.workspace_id,&claim.binding_id,&key]).await.map_err(db_error)?;
                }
            }
            applied = Some(id);
            "applied"
        } else {
            "review_rejected"
        };
        tx.execute("UPDATE experience_evaluation SET status='completed',application_status=$2,applied_revision_id=$3,results=results || jsonb_build_array($4::jsonb),next_case=next_case+1,lease_token=NULL,lease_until=NULL,updated_at=NOW() WHERE id=$1", &[&claim.id,&status,&applied,&result]).await.map_err(db_error)?;
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'experience.optimization_application','runtime_binding',$4,$5)", &[&choruz_common::new_id(),&claim.workspace_id,&claim.owner_id,&claim.binding_id,&json!({"evaluation_id":claim.id,"status":status,"revision_id":applied})]).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(true)
    }
}

fn db_error(error: tokio_postgres::Error) -> AppError {
    AppError::Internal(format!("evaluation store: {error}"))
}
