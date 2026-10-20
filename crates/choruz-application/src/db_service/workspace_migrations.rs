use choruz_common::{AppError, new_id};
use choruz_domain::{Conversation, Principal, PrincipalType};
use serde_json::json;

use super::{
    DbService,
    companies::workspace_access,
    helpers::{row_to_conversation, row_to_principal, rows_to_members},
};

fn storage(error: tokio_postgres::Error) -> AppError {
    AppError::Internal(format!(
        "workspace migration: {}",
        error
            .as_db_error()
            .map_or_else(|| error.to_string(), |error| error.message().to_owned())
    ))
}

async fn authorize(
    tx: &deadpool_postgres::Transaction<'_>,
    actor: &Principal,
    current: &str,
    target: &str,
) -> Result<(), AppError> {
    if !matches!(actor.principal_type, PrincipalType::Human) {
        return Err(AppError::Forbidden(
            "only humans can migrate workspaces".into(),
        ));
    }
    if target.trim().is_empty() {
        return Err(AppError::Validation("workspace_id is required".into()));
    }
    if !workspace_access(tx, actor, current).await? || !workspace_access(tx, actor, target).await? {
        return Err(AppError::Forbidden("cross-workspace access denied".into()));
    }
    Ok(())
}

async fn record_migration(
    tx: &deadpool_postgres::Transaction<'_>,
    actor: &Principal,
    kind: &str,
    id: &str,
    current: &str,
    target: &str,
) -> Result<(), AppError> {
    tx.execute(
        "INSERT INTO audit_log (id,workspace_id,actor_id,action,target_type,target_id,metadata,created_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,NOW())",
        &[&new_id(), &actor.workspace_id, &actor.id, &format!("{kind}.workspace_migrated"),
          &kind, &id, &json!({"old_workspace_id":current,"new_workspace_id":target})],
    ).await.map_err(storage)?;
    Ok(())
}

impl DbService {
    /// Move a principal between accessible workspaces and commit its audit record
    /// atomically. Storage failures leave both the row and audit unchanged.
    pub async fn migrate_principal_workspace(
        &self,
        principal_id: &str,
        new_workspace_id: &str,
        actor_id: &str,
    ) -> Result<Principal, AppError> {
        let actor = self.get_principal(actor_id).await?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let row = tx
            .query_opt(
                "SELECT * FROM principal WHERE id=$1 FOR UPDATE",
                &[&principal_id],
            )
            .await
            .map_err(storage)?
            .ok_or_else(|| AppError::NotFound("principal not found".into()))?;
        let current: String = row.get("workspace_id");
        authorize(&tx, &actor, &current, new_workspace_id).await?;
        let row = tx
            .query_one(
                "UPDATE principal SET workspace_id=$2,updated_at=NOW() WHERE id=$1 RETURNING *",
                &[&principal_id, &new_workspace_id],
            )
            .await
            .map_err(storage)?;
        record_migration(
            &tx,
            &actor,
            "principal",
            principal_id,
            &current,
            new_workspace_id,
        )
        .await?;
        tx.commit().await.map_err(storage)?;
        Ok(row_to_principal(&row))
    }

    /// Move only the conversation row, without rewriting historical event rows.
    /// The row and audit commit together, before any response succeeds.
    pub async fn migrate_conversation_workspace(
        &self,
        conversation_id: &str,
        new_workspace_id: &str,
        actor_id: &str,
    ) -> Result<Conversation, AppError> {
        let actor = self.get_principal(actor_id).await?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let row = tx
            .query_opt(
                "SELECT * FROM conversation WHERE id=$1 FOR UPDATE",
                &[&conversation_id],
            )
            .await
            .map_err(storage)?
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
        let current: String = row.get("workspace_id");
        authorize(&tx, &actor, &current, new_workspace_id).await?;
        let row = tx
            .query_one(
                "UPDATE conversation SET workspace_id=$2,updated_at=NOW() WHERE id=$1 RETURNING *",
                &[&conversation_id, &new_workspace_id],
            )
            .await
            .map_err(storage)?;
        let members = tx.query("SELECT principal_id,joined_at FROM conversation_member WHERE conv_id=$1 AND removed_at IS NULL", &[&conversation_id]).await.map_err(storage)?;
        record_migration(
            &tx,
            &actor,
            "conversation",
            conversation_id,
            &current,
            new_workspace_id,
        )
        .await?;
        tx.commit().await.map_err(storage)?;
        Ok(row_to_conversation(&row, rows_to_members(&members)))
    }
}
