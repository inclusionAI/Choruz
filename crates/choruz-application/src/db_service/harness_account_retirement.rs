use super::DbService;
use choruz_common::AppError;
use serde_json::json;

#[derive(Clone)]
pub struct AccountRetirement {
    pub account_id: String,
    pub workspace_id: String,
    pub runtime_host_id: Option<String>,
    pub actor_id: Option<String>,
    pub binding_ids: Vec<String>,
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("harness account removal: {error}"))
}

impl DbService {
    /// Fences new bindings and logins atomically before device cleanup. Repeated
    /// requests retain the original durable request and return its current scope.
    pub async fn request_account_retirement(
        &self,
        workspace: &str,
        account: &str,
        actor: &str,
    ) -> Result<AccountRetirement, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(internal)?;
        let row = tx.query_opt("SELECT runtime_host_id,removal_actor_id FROM harness_account WHERE company_id=$1 AND id=$2 AND removal_completed_at IS NULL AND (disabled_at IS NULL OR removal_requested_at IS NOT NULL) FOR UPDATE", &[&workspace,&account]).await.map_err(internal)?
            .ok_or_else(|| AppError::NotFound("Harness account not found".into()))?;
        tx.execute("UPDATE harness_account SET status='disabled',disabled_at=COALESCE(disabled_at,NOW()),removal_requested_at=COALESCE(removal_requested_at,NOW()),removal_actor_id=COALESCE(removal_actor_id,$3),last_error=NULL,updated_at=NOW() WHERE company_id=$1 AND id=$2", &[&workspace,&account,&actor]).await.map_err(internal)?;
        tx.execute("UPDATE agent_runtime_bindings b SET state='disabled',last_error='Harness account removal requested',updated_at=NOW() FROM principal p WHERE p.id=b.agent_principal_id AND p.workspace_id=$1 AND b.config_json->>'harness_account_id'=$2", &[&workspace,&account]).await.map_err(internal)?;
        tx.execute("UPDATE harness_account_login SET state='cancelled',callback_code=NULL,updated_at=NOW() WHERE company_id=$1 AND account_id=$2 AND state IN ('queued','awaiting_browser','authorizing')", &[&workspace,&account]).await.map_err(internal)?;
        tx.execute("UPDATE agent_commands c SET status='dead_letter',last_error='Harness account removed [kind=account_removed]',updated_at=NOW() FROM agent_runtime_bindings b,principal p WHERE p.id=b.agent_principal_id AND p.workspace_id=$1 AND b.config_json->>'harness_account_id'=$2 AND c.agent_id=b.agent_principal_id AND c.status IN ('pending','retry_scheduled')", &[&workspace,&account]).await.map_err(internal)?;
        tx.execute("UPDATE agent_commands c SET max_attempts=GREATEST(1,c.attempt_count) FROM agent_runtime_bindings b,principal p WHERE p.id=b.agent_principal_id AND p.workspace_id=$1 AND b.config_json->>'harness_account_id'=$2 AND c.agent_id=b.agent_principal_id AND c.status IN ('leased','started','heartbeating')", &[&workspace,&account]).await.map_err(internal)?;
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata,created_at) VALUES($1,$2,$3,'harness_account.removal_requested','harness_account',$4,$5,NOW())", &[&choruz_common::new_id(),&workspace,&actor,&account,&json!({"runtime_host_id":row.get::<_,Option<String>>("runtime_host_id")})]).await.map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        let binding_ids = self.account_bindings(workspace, account).await?;
        Ok(AccountRetirement {
            account_id: account.into(),
            workspace_id: workspace.into(),
            runtime_host_id: row.get("runtime_host_id"),
            actor_id: Some(
                row.get::<_, Option<String>>("removal_actor_id")
                    .unwrap_or_else(|| actor.into()),
            ),
            binding_ids,
        })
    }

    /// Internal durable queue; user-facing operations still authorize and scope
    /// their request before this queue can contain an account.
    pub async fn pending_account_retirements(&self) -> Result<Vec<AccountRetirement>, AppError> {
        let client = self.store.connect().await?;
        let rows=client.query("SELECT a.id,a.company_id,a.runtime_host_id,a.removal_actor_id,ARRAY(SELECT b.id FROM agent_runtime_bindings b JOIN principal p ON p.id=b.agent_principal_id WHERE p.workspace_id=a.company_id AND b.config_json->>'harness_account_id'=a.id) AS bindings FROM harness_account a WHERE removal_requested_at IS NOT NULL AND removal_completed_at IS NULL ORDER BY removal_requested_at,id LIMIT 32", &[]).await.map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| AccountRetirement {
                account_id: r.get("id"),
                workspace_id: r.get("company_id"),
                runtime_host_id: r.get("runtime_host_id"),
                actor_id: r.get("removal_actor_id"),
                binding_ids: r.get("bindings"),
            })
            .collect())
    }

    async fn account_bindings(
        &self,
        workspace: &str,
        account: &str,
    ) -> Result<Vec<String>, AppError> {
        self.store.connect().await?.query("SELECT b.id FROM agent_runtime_bindings b JOIN principal p ON p.id=b.agent_principal_id WHERE p.workspace_id=$1 AND b.config_json->>'harness_account_id'=$2", &[&workspace,&account]).await.map_err(internal).map(|rows|rows.into_iter().map(|r|r.get(0)).collect())
    }

    pub async fn finish_account_retirement(
        &self,
        request: &AccountRetirement,
    ) -> Result<Option<i64>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(internal)?;
        let active: bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM agent_commands c JOIN agent_runtime_bindings b ON b.agent_principal_id=c.agent_id JOIN principal p ON p.id=b.agent_principal_id WHERE p.workspace_id=$1 AND b.config_json->>'harness_account_id'=$2 AND c.status IN('leased','started','heartbeating'))", &[&request.workspace_id,&request.account_id]).await.map_err(internal)?.get(0);
        if active {
            return Ok(None);
        }
        let completed=tx.execute("UPDATE harness_account SET removal_completed_at=NOW(),last_error=NULL,updated_at=NOW() WHERE company_id=$1 AND id=$2 AND removal_requested_at IS NOT NULL AND removal_completed_at IS NULL", &[&request.workspace_id,&request.account_id]).await.map_err(internal)?;
        let count=tx.execute("UPDATE agent_runtime_bindings b SET in_flight_turn_id=NULL,updated_at=NOW() FROM principal p WHERE p.id=b.agent_principal_id AND p.workspace_id=$1 AND b.config_json->>'harness_account_id'=$2", &[&request.workspace_id,&request.account_id]).await.map_err(internal)?;
        if completed > 0
            && let Some(actor) = &request.actor_id
        {
            tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata,created_at) VALUES($1,$2,$3,'harness_account.removal_completed','harness_account',$4,$5,NOW())", &[&choruz_common::new_id(),&request.workspace_id,actor,&request.account_id,&json!({"disabled_bindings":count})]).await.map_err(internal)?;
        }
        tx.commit().await.map_err(internal)?;
        Ok(Some(count as i64))
    }

    pub async fn account_retirement_error(
        &self,
        request: &AccountRetirement,
        message: &str,
    ) -> Result<(), AppError> {
        self.store.connect().await?.execute("UPDATE harness_account SET last_error=$3,updated_at=NOW() WHERE company_id=$1 AND id=$2 AND removal_completed_at IS NULL", &[&request.workspace_id,&request.account_id,&message]).await.map_err(internal)?;
        Ok(())
    }
}
