use super::DbService;
use choruz_common::AppError;
use choruz_decision::browser_settings::BrowserSettings;
use serde_json::{Value, json};

fn error(e: tokio_postgres::Error) -> AppError {
    AppError::Internal(format!("browser automation persistence: {e}"))
}

impl DbService {
    /// Claim dispatch once under the policy lock, preserving the
    /// original two-minute deadline (the receipt allows 30s for cleanup).
    /// Configuration changes fence unclaimed work; Stop also tombstones in-flight work.
    pub async fn claim_browser_dispatch(
        &self,
        workspace: &str,
        binding: &str,
        id: &str,
    ) -> Result<Option<u64>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(error)?;
        tx.query_opt("SELECT generation FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 FOR UPDATE", &[&workspace,&binding]).await.map_err(error)?;
        let row = tx
            .query_opt(
                "UPDATE browser_workflow_run r SET dispatch_claimed=TRUE
             FROM browser_automation a, experience_policy p
             WHERE a.workspace_id=r.workspace_id AND a.binding_id=r.binding_id
               AND p.workspace_id=r.workspace_id AND p.binding_id=r.binding_id
               AND NOT r.dispatch_claimed
               AND r.workspace_id=$1 AND r.binding_id=$2 AND r.id=$3
               AND r.status='running' AND r.expires_at>NOW()+INTERVAL '30 seconds'
               AND a.settings IS NOT NULL AND a.generation=r.automation_generation
               AND a.binding_fingerprint=r.binding_fingerprint
               AND p.enabled AND p.generation=r.learning_generation
               AND p.decision_settings IS NOT NULL
             RETURNING FLOOR(EXTRACT(EPOCH FROM r.expires_at - INTERVAL '30 seconds'))::bigint",
                &[&workspace, &binding, &id],
            )
            .await
            .map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(row.map(|row| row.get::<_, i64>(0) as u64))
    }
    pub async fn list_browser_runs(
        &self,
        workspace: &str,
        binding: &str,
    ) -> Result<Vec<Value>, AppError> {
        let rows=self.store.connect().await?.query("SELECT jsonb_build_object('id',id,'revision_id',revision_id,'status',CASE WHEN status='running' AND expires_at<=NOW() THEN 'outcome_unconfirmed' ELSE status END,'result',result,'created_at',created_at) FROM browser_workflow_run WHERE workspace_id=$1 AND binding_id=$2 ORDER BY created_at DESC LIMIT 50", &[&workspace,&binding]).await.map_err(error)?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }
    pub async fn existing_automatic_browser(
        &self,
        workspace: &str,
        binding: &str,
        actor: &str,
        id: &str,
        hash: &str,
    ) -> Result<bool, AppError> {
        let row=self.store.connect().await?.query_opt("SELECT actor_id,request_hash FROM browser_workflow_run WHERE workspace_id=$1 AND binding_id=$2 AND id=$3", &[&workspace,&binding,&id]).await.map_err(error)?;
        match row {
            None => Ok(false),
            Some(row) if row.get::<_, String>(0) == actor && row.get::<_, String>(1) == hash => {
                Ok(true)
            }
            Some(_) => Err(AppError::Conflict(
                "Run id belongs to another request".into(),
            )),
        }
    }
    /// Changing standing permission invalidates pending admissions. Disabling
    /// also cancels their receipts; the caller forwards cancellation to the device.
    pub async fn configure_browser_automation(
        &self,
        workspace: &str,
        binding: &str,
        owner: &str,
        fingerprint: &str,
        settings: Option<&BrowserSettings>,
    ) -> Result<Vec<String>, AppError> {
        if let Some(settings) = settings {
            settings
                .validate()
                .map_err(|e| AppError::Validation(e.to_string()))?;
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(error)?;
        let policy = tx.query_opt("SELECT owner_id,enabled,decision_settings FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 FOR UPDATE", &[&workspace,&binding]).await.map_err(error)?
            .ok_or_else(|| AppError::Conflict("Configure background learning first".into()))?;
        if policy.get::<_, String>(0) != owner {
            return Err(AppError::Forbidden(
                "Only the learning owner can configure browser automation".into(),
            ));
        }
        if settings.is_some()
            && (!policy.get::<_, bool>(1)
                || policy
                    .get::<_, Option<Value>>(2)
                    .is_none_or(|value| !value["builder_binding_id"].is_string()))
        {
            return Err(AppError::Conflict(
                "Enable learning and select a program-building Agent and decision model first"
                    .into(),
            ));
        }
        tx.execute("INSERT INTO browser_automation(workspace_id,binding_id,owner_id,settings,binding_fingerprint) VALUES($1,$2,$3,$4,$5) ON CONFLICT(workspace_id,binding_id) DO UPDATE SET settings=EXCLUDED.settings,binding_fingerprint=EXCLUDED.binding_fingerprint,generation=browser_automation.generation+1", &[&workspace,&binding,&owner,&settings.map(|s|json!(s)),&fingerprint]).await.map_err(error)?;
        let runs = tx.query("UPDATE browser_workflow_run SET status='cancelled' WHERE workspace_id=$1 AND binding_id=$2 AND automation_generation IS NOT NULL AND status='running' RETURNING id", &[&workspace,&binding]).await.map_err(error)?.into_iter().map(|row| row.get(0)).collect();
        tx.commit().await.map_err(error)?;
        Ok(runs)
    }

    pub async fn browser_automation(
        &self,
        workspace: &str,
        binding: &str,
    ) -> Result<Value, AppError> {
        let row = self.store.connect().await?.query_opt("SELECT to_jsonb(a) || jsonb_build_object('ready',a.settings IS NOT NULL AND p.enabled AND p.decision_settings IS NOT NULL,'decision_settings',p.decision_settings,'learning_generation',p.generation) FROM browser_automation a JOIN experience_policy p ON p.workspace_id=a.workspace_id AND p.binding_id=a.binding_id AND p.owner_id=a.owner_id WHERE a.workspace_id=$1 AND a.binding_id=$2", &[&workspace,&binding]).await.map_err(error)?;
        Ok(row.map(|r| r.get(0)).unwrap_or(Value::Null))
    }

    /// Failed or uncertain revisions cannot be retried with a new run id.
    /// A revised workflow can be generated from subsequent independent evidence.
    pub async fn automatic_browser_catalog(
        &self,
        workspace: &str,
        binding: &str,
    ) -> Result<Vec<Value>, AppError> {
        let rows = self.store.connect().await?.query("SELECT jsonb_build_object('revision_id',r.id,'binding_id',r.binding_id,'workflow',r.validation->'program_trial'->'workflow','validated',EXISTS(SELECT 1 FROM browser_workflow_run b WHERE b.workspace_id=r.workspace_id AND b.binding_id=r.binding_id AND b.revision_id=r.id AND b.status='finished' AND b.result->'report'->>'checks_matched'='true')) FROM experience_revision r WHERE r.workspace_id=$1 AND r.binding_id=$2 AND r.validation->'program_trial'->'workflow' IS NOT NULL AND NOT EXISTS(SELECT 1 FROM browser_workflow_run b WHERE b.workspace_id=r.workspace_id AND b.binding_id=r.binding_id AND b.revision_id=r.id AND (b.status<>'finished' OR b.result->'report'->>'checks_matched' IS DISTINCT FROM 'true')) ORDER BY r.created_at DESC,r.id DESC LIMIT 50", &[&workspace,&binding]).await.map_err(error)?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn admit_automatic_browser(
        &self,
        workspace: &str,
        binding: &str,
        actor: &str,
        id: &str,
        hash: &str,
        revision: &str,
        generation: i64,
        learning_generation: i64,
        fingerprint: &str,
        conversation: Option<&str>,
    ) -> Result<bool, AppError> {
        if id.is_empty()
            || id.len() > 128
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(AppError::Validation("Use a stable browser run id".into()));
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(error)?;
        // Match configuration's policy-before-permission lock order.
        tx.query_opt("SELECT generation FROM experience_policy WHERE workspace_id=$1 AND binding_id=$2 FOR UPDATE", &[&workspace,&binding]).await.map_err(error)?;
        let policy = tx.query_opt("SELECT a.generation,a.binding_fingerprint,(a.settings IS NOT NULL AND p.enabled AND p.decision_settings IS NOT NULL),p.generation FROM browser_automation a JOIN experience_policy p ON p.workspace_id=a.workspace_id AND p.binding_id=a.binding_id AND p.owner_id=a.owner_id WHERE a.workspace_id=$1 AND a.binding_id=$2 FOR UPDATE OF a,p", &[&workspace,&binding]).await.map_err(error)?.ok_or_else(||AppError::Forbidden("Browser automation is not enabled".into()))?;
        if let Some(row) = tx.query_opt("SELECT actor_id,request_hash,revision_id FROM browser_workflow_run WHERE workspace_id=$1 AND binding_id=$2 AND id=$3", &[&workspace,&binding,&id]).await.map_err(error)? {
            if row.get::<_,String>(0)==actor && row.get::<_,String>(1)==hash && row.get::<_,Option<String>>(2).as_deref()==Some(revision) { return Ok(false); }
            return Err(AppError::Conflict("Run id belongs to another request".into()));
        }
        if policy.get::<_, i64>(0) != generation
            || policy.get::<_, String>(1) != fingerprint
            || !policy.get::<_, bool>(2)
            || policy.get::<_, i64>(3) != learning_generation
        {
            return Err(AppError::Conflict(
                "Browser permission or execution account changed".into(),
            ));
        }
        let blocked=tx.query_opt("SELECT id FROM browser_workflow_run WHERE workspace_id=$1 AND binding_id=$2 AND automation_generation IS NOT NULL AND ((status='running' AND expires_at>NOW()) OR (revision_id=$3 AND (status<>'finished' OR result->'report'->>'checks_matched' IS DISTINCT FROM 'true'))) LIMIT 1", &[&workspace,&binding,&revision]).await.map_err(error)?.is_some();
        if blocked {
            return Err(AppError::Conflict(
                "Browser workflow is running or needs repair; do not repeat uncertain actions"
                    .into(),
            ));
        }
        tx.execute("INSERT INTO browser_workflow_run(workspace_id,binding_id,id,actor_id,request_hash,status,binding_fingerprint,revision_id,automation_generation,conversation_id,learning_generation) VALUES($1,$2,$3,$4,$5,'running',$6,$7,$8,$9,$10)", &[&workspace,&binding,&id,&actor,&hash,&fingerprint,&revision,&generation,&conversation,&learning_generation]).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(true)
    }

    pub async fn browser_completion_queue(&self) -> Result<Vec<Value>, AppError> {
        let rows=self.store.connect().await?.query("SELECT jsonb_build_object('workspace',r.workspace_id,'binding',r.binding_id,'id',r.id,'agent',b.agent_principal_id,'conversation',COALESCE(r.conversation_id,b.conversation_id)) FROM browser_workflow_run r JOIN agent_runtime_bindings b ON b.id=r.binding_id WHERE r.automation_generation IS NOT NULL AND NOT r.notified AND (r.status<>'running' OR r.expires_at<=NOW()) ORDER BY r.created_at LIMIT 50", &[]).await.map_err(error)?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }
    pub async fn mark_browser_notified(
        &self,
        workspace: &str,
        binding: &str,
        id: &str,
    ) -> Result<(), AppError> {
        self.store.connect().await?.execute("UPDATE browser_workflow_run SET notified=TRUE WHERE workspace_id=$1 AND binding_id=$2 AND id=$3", &[&workspace,&binding,&id]).await.map_err(error)?;
        Ok(())
    }
}
