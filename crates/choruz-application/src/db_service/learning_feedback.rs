//! Current shared-conversation authority for private learning evidence.
use super::DbService;
use choruz_common::AppError;
use serde_json::{Value, json};

impl DbService {
    pub(super) async fn learning_feedback_scopes(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
    ) -> Result<Vec<(String, Vec<String>)>, AppError> {
        let rows = self.store.connect().await?.query("SELECT c.id,ARRAY_AGG(DISTINCT b.agent_principal_id) AS agents FROM conversation c
            JOIN conversation_member human ON human.conv_id=c.id AND human.principal_id=$1 AND human.removed_at IS NULL
            JOIN agent_runtime_bindings b ON effective_experience_binding($3,b.id,$1)=$2
            JOIN conversation_member agent ON agent.conv_id=c.id AND agent.principal_id=b.agent_principal_id AND agent.removed_at IS NULL
            WHERE c.workspace_id=$3 GROUP BY c.id ORDER BY c.id", &[&owner,&binding,&workspace]).await
            .map_err(|e| AppError::Internal(format!("read learning feedback scopes: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get("id"), row.get("agents")))
            .collect())
    }

    /// Recover only currently shared evidence. A committed cursor is required
    /// for historical consumers; pending windows revalidate membership separately.
    pub(super) async fn learning_feedback_records(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        cursor: Option<&Value>,
        references: &[String],
    ) -> Result<Vec<Value>, AppError> {
        let ids: Vec<_> = references
            .iter()
            .filter_map(|r| r.strip_prefix("message:"))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = self.store.connect().await?.query("SELECT DISTINCT e.event_id,e.conversation_id,e.seq,e.content FROM conversation_events e
            JOIN conversation c ON c.id=e.conversation_id
            JOIN agent_runtime_bindings b ON effective_experience_binding($3,b.id,$1)=$2
            JOIN conversation_member human ON human.conv_id=c.id AND human.principal_id=$1 AND human.removed_at IS NULL
            JOIN conversation_member agent ON agent.conv_id=c.id AND agent.principal_id=b.agent_principal_id AND agent.removed_at IS NULL
            WHERE c.workspace_id=$3 AND e.event_id=ANY($4)
              AND e.event_type IN ('message','message.created','reply','runtime.decision') AND (e.sender_id=$1 OR e.sender_id=b.agent_principal_id)", &[&owner,&binding,&workspace,&ids]).await
            .map_err(|e|AppError::Internal(format!("recover learning feedback: {e}")))?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let conversation: String = row.get("conversation_id");
                let seq: i64 = row.get("seq");
                let seq = u64::try_from(seq).ok()?;
                if cursor.is_some_and(|cursor| {
                    cursor["feedback"][&conversation]
                        .as_u64()
                        .is_none_or(|end| seq > end)
                }) {
                    return None;
                }
                Some(
                    json!({"ref":format!("message:{}",row.get::<_,String>("event_id")),
                "record":{"content":row.get::<_,Option<String>>("content")}}),
                )
            })
            .collect())
    }
}
