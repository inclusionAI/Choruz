use super::DbService;
use choruz_common::AppError;
use serde_json::{Value, json};

impl DbService {
    pub async fn browser_workflow_run(
        &self,
        workspace: &str,
        binding: &str,
        id: &str,
    ) -> Result<Value, AppError> {
        let db = self.store.connect().await?;
        let row = db.query_opt("SELECT status,result,expires_at > NOW() AS live,grant_id FROM browser_workflow_run WHERE workspace_id=$1 AND binding_id=$2 AND id=$3", &[&workspace,&binding,&id]).await
            .map_err(|e| AppError::Internal(format!("read browser run: {e}")))?.ok_or_else(|| AppError::NotFound("Browser run not found".into()))?;
        let status: String = row.get(0);
        let live: bool = row.get(2);
        Ok(
            json!({"id":id,"grant_id":row.get::<_, Option<String>>(3),"status":if status == "running" && !live { "outcome_unconfirmed" } else { &status },"result":row.get::<_, Option<Value>>(1)}),
        )
    }

    pub async fn finish_browser_workflow(
        &self,
        workspace: &str,
        binding: &str,
        id: &str,
        result: &Value,
    ) -> Result<(), AppError> {
        self.store.connect().await?.execute("UPDATE browser_workflow_run SET status='finished',result=$4 WHERE workspace_id=$1 AND binding_id=$2 AND id=$3 AND status='running' AND expires_at > NOW()", &[&workspace,&binding,&id,&result]).await
            .map_err(|e| AppError::Internal(format!("finish browser run: {e}")))?;
        Ok(())
    }

    pub async fn cancel_browser_workflow(
        &self,
        workspace: &str,
        binding: &str,
        id: &str,
    ) -> Result<(), AppError> {
        self.store.connect().await?.execute("UPDATE browser_workflow_run SET status='cancelled' WHERE workspace_id=$1 AND binding_id=$2 AND id=$3 AND status='running'", &[&workspace,&binding,&id]).await
            .map_err(|e| AppError::Internal(format!("cancel browser run: {e}")))?;
        Ok(())
    }
}
