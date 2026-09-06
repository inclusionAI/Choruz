use choruz_common::{AppError, new_id};
use choruz_domain::Principal;
use serde_json::json;

use super::DbService;

/// Internal server credential. Never serialize this type into API responses or logs.
#[derive(Clone)]
pub struct OnlineIdentity {
    pub account_id: String,
    pub device_id: String,
    pub service_url: String,
    pub session_token: String,
    pub display_name: String,
}

impl DbService {
    /// Read only the caller's local Online identity; absence means signed out.
    pub async fn online_identity(
        &self,
        actor: &Principal,
    ) -> Result<Option<OnlineIdentity>, AppError> {
        let client = self.store.connect().await?;
        let row = client.query_opt("SELECT account_id,device_id,service_url,session_token,display_name FROM online_identity WHERE principal_id=$1 AND workspace_id=$2", &[&actor.id,&actor.workspace_id]).await
            .map_err(|e| AppError::Internal(format!("read Online identity: {e}")))?;
        Ok(row.map(|r| OnlineIdentity {
            account_id: r.get(0),
            device_id: r.get(1),
            service_url: r.get(2),
            session_token: r.get(3),
            display_name: r.get(4),
        }))
    }

    /// Insert without replacing a concurrent sign-in. Identity and audit commit
    /// atomically; the caller must revoke a newly issued token if insertion fails.
    pub async fn connect_online_identity(
        &self,
        actor: &Principal,
        identity: &OnlineIdentity,
    ) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin Online sign-in: {e}")))?;
        let count = tx.execute("INSERT INTO online_identity(principal_id,workspace_id,account_id,device_id,service_url,session_token,display_name) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(principal_id) DO NOTHING", &[&actor.id,&actor.workspace_id,&identity.account_id,&identity.device_id,&identity.service_url,&identity.session_token,&identity.display_name]).await
            .map_err(|e| AppError::Internal(format!("save Online identity: {e}")))?;
        if count == 0 {
            return Err(AppError::Conflict(
                "Sign out of Online before changing accounts".into(),
            ));
        }
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'online.signed_in','online_device',$4,$5)", &[&new_id(),&actor.workspace_id,&actor.id,&identity.device_id,&json!({"account_id":identity.account_id})]).await
            .map_err(|e| AppError::Internal(format!("audit Online sign-in: {e}")))?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit Online sign-in: {e}")))
    }

    /// Remove only the observed device identity, preserving a concurrent replacement.
    pub async fn disconnect_online_identity(
        &self,
        actor: &Principal,
        device_id: &str,
    ) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin Online sign-out: {e}")))?;
        let count = tx.execute("DELETE FROM online_identity WHERE principal_id=$1 AND workspace_id=$2 AND device_id=$3", &[&actor.id,&actor.workspace_id,&device_id]).await
            .map_err(|e| AppError::Internal(format!("remove Online identity: {e}")))?;
        if count > 0 {
            tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'online.signed_out','online_device',$4,'{}')", &[&new_id(),&actor.workspace_id,&actor.id,&device_id]).await
                .map_err(|e| AppError::Internal(format!("audit Online sign-out: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit Online sign-out: {e}")))
    }
}
