use choruz_common::{AppError, AppResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{AuditActor, RuntimeStore, binding::map_db_error};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoMode {
    Disabled,
    MentionedOnly,
    MetadataOnly,
}

impl AutoMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::MentionedOnly => "mentioned_only",
            Self::MetadataOnly => "metadata_only",
        }
    }

    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "mentioned_only" => Ok(Self::MentionedOnly),
            "metadata_only" => Ok(Self::MetadataOnly),
            other => Err(AppError::Internal(format!(
                "unknown auto mode from database: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UntaggedHumanMode {
    MentionedOnly,
    CoordinatorOnly,
    AllAgents,
}

impl UntaggedHumanMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MentionedOnly => "mentioned_only",
            Self::CoordinatorOnly => "coordinator_only",
            Self::AllAgents => "all_agents",
        }
    }

    fn from_str(value: &str) -> AppResult<Self> {
        match value {
            "mentioned_only" => Ok(Self::MentionedOnly),
            "coordinator_only" => Ok(Self::CoordinatorOnly),
            "all_agents" => Ok(Self::AllAgents),
            other => Err(AppError::Internal(format!(
                "unknown untagged human mode from database: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationRuntimePolicy {
    pub conversation_id: String,
    pub auto_mode: AutoMode,
    pub max_auto_turns: i32,
    pub max_workflow_turns: i32,
    pub require_human_after_n_turns: i32,
    pub allow_agent_to_agent: bool,
    pub allow_file_write: bool,
    pub default_reviewer_agent_id: Option<String>,
    pub default_coordinator_agent_id: Option<String>,
    pub untagged_human_mode: UntaggedHumanMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertPolicyInput {
    pub conversation_id: String,
    pub auto_mode: AutoMode,
    pub max_auto_turns: i32,
    pub max_workflow_turns: i32,
    pub require_human_after_n_turns: i32,
    pub allow_agent_to_agent: bool,
    pub allow_file_write: bool,
    pub default_reviewer_agent_id: Option<String>,
    pub default_coordinator_agent_id: Option<String>,
    pub untagged_human_mode: UntaggedHumanMode,
    pub audit_actor: Option<AuditActor>,
}

impl RuntimeStore {
    pub async fn get_policy(&self, conversation_id: &str) -> AppResult<ConversationRuntimePolicy> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT
                   conversation_id,
                   auto_mode,
                   max_auto_turns,
                   max_workflow_turns,
                   require_human_after_n_turns,
                   allow_agent_to_agent,
                   allow_file_write,
                   default_reviewer_agent_id,
                   default_coordinator_agent_id,
                   untagged_human_mode,
                   created_at,
                   updated_at
                 FROM conversation_runtime_policies
                 WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(map_db_error)?;

        match row {
            Some(row) => policy_from_row(&row),
            None => Ok(ConversationRuntimePolicy {
                conversation_id: conversation_id.to_owned(),
                auto_mode: AutoMode::MentionedOnly,
                max_auto_turns: 4,
                max_workflow_turns: 20,
                require_human_after_n_turns: 4,
                allow_agent_to_agent: true,
                allow_file_write: true,
                default_reviewer_agent_id: None,
                default_coordinator_agent_id: None,
                untagged_human_mode: UntaggedHumanMode::MentionedOnly,
                created_at: self.clock.now(),
                updated_at: self.clock.now(),
            }),
        }
    }

    pub async fn upsert_policy(
        &self,
        input: UpsertPolicyInput,
    ) -> AppResult<ConversationRuntimePolicy> {
        if input.max_auto_turns <= 0 {
            return Err(AppError::Validation(
                "max_auto_turns must be positive".into(),
            ));
        }
        if input.max_workflow_turns <= 0 {
            return Err(AppError::Validation(
                "max_workflow_turns must be positive".into(),
            ));
        }
        if input.require_human_after_n_turns <= 0 {
            return Err(AppError::Validation(
                "require_human_after_n_turns must be positive".into(),
            ));
        }
        if let Some(agent_id) = input.default_coordinator_agent_id.as_deref()
            && agent_id.trim().is_empty()
        {
            return Err(AppError::Validation(
                "default_coordinator_agent_id must not be empty".into(),
            ));
        }

        let client = self.connect().await?;
        if let Some(agent_id) = input.default_coordinator_agent_id.as_deref() {
            let row = client
                .query_opt("SELECT type FROM principal WHERE id = $1", &[&agent_id])
                .await
                .map_err(map_db_error)?;
            let principal_type = row.map(|row| row.get::<_, String>("type"));
            if principal_type.as_deref() != Some("agent") {
                return Err(AppError::Validation(
                    "default_coordinator_agent_id must reference an agent principal".into(),
                ));
            }
        }
        let now = self.clock.now();
        let row = client
            .query_one(
                "INSERT INTO conversation_runtime_policies (
                   conversation_id,
                   auto_mode,
                   max_auto_turns,
                   max_workflow_turns,
                   require_human_after_n_turns,
                   allow_agent_to_agent,
                   allow_file_write,
                   default_reviewer_agent_id,
                   default_coordinator_agent_id,
                   untagged_human_mode,
                   created_at,
                   updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
                 ON CONFLICT (conversation_id) DO UPDATE
                 SET auto_mode = EXCLUDED.auto_mode,
                     max_auto_turns = EXCLUDED.max_auto_turns,
                     max_workflow_turns = EXCLUDED.max_workflow_turns,
                     require_human_after_n_turns = EXCLUDED.require_human_after_n_turns,
                     allow_agent_to_agent = EXCLUDED.allow_agent_to_agent,
                     allow_file_write = EXCLUDED.allow_file_write,
                     default_reviewer_agent_id = EXCLUDED.default_reviewer_agent_id,
                     default_coordinator_agent_id = EXCLUDED.default_coordinator_agent_id,
                     untagged_human_mode = EXCLUDED.untagged_human_mode,
                     updated_at = EXCLUDED.updated_at
                 RETURNING
                   conversation_id,
                   auto_mode,
                   max_auto_turns,
                   max_workflow_turns,
                   require_human_after_n_turns,
                   allow_agent_to_agent,
                   allow_file_write,
                   default_reviewer_agent_id,
                   default_coordinator_agent_id,
                   untagged_human_mode,
                   created_at,
                   updated_at",
                &[
                    &input.conversation_id,
                    &input.auto_mode.as_str(),
                    &input.max_auto_turns,
                    &input.max_workflow_turns,
                    &input.require_human_after_n_turns,
                    &input.allow_agent_to_agent,
                    &input.allow_file_write,
                    &input.default_reviewer_agent_id,
                    &input.default_coordinator_agent_id,
                    &input.untagged_human_mode.as_str(),
                    &now,
                ],
            )
            .await
            .map_err(map_db_error)?;
        let policy = policy_from_row(&row)?;

        if let Some(actor) = &input.audit_actor {
            self.record_audit(
                &client,
                actor,
                "runtime.policy_upserted",
                "conversation_runtime_policy",
                &policy.conversation_id,
                serde_json::json!({
                    "auto_mode": policy.auto_mode.as_str(),
                    "max_auto_turns": policy.max_auto_turns,
                    "max_workflow_turns": policy.max_workflow_turns,
                    "require_human_after_n_turns": policy.require_human_after_n_turns,
                    "allow_agent_to_agent": policy.allow_agent_to_agent,
                    "allow_file_write": policy.allow_file_write,
                    "default_reviewer_agent_id": policy.default_reviewer_agent_id,
                    "default_coordinator_agent_id": policy.default_coordinator_agent_id,
                    "untagged_human_mode": policy.untagged_human_mode.as_str(),
                }),
            )
            .await?;
        }

        Ok(policy)
    }
}

fn policy_from_row(row: &tokio_postgres::Row) -> AppResult<ConversationRuntimePolicy> {
    Ok(ConversationRuntimePolicy {
        conversation_id: row.get("conversation_id"),
        auto_mode: AutoMode::from_str(row.get::<_, &str>("auto_mode"))?,
        max_auto_turns: row.get("max_auto_turns"),
        max_workflow_turns: row.get("max_workflow_turns"),
        require_human_after_n_turns: row.get("require_human_after_n_turns"),
        allow_agent_to_agent: row.get("allow_agent_to_agent"),
        allow_file_write: row.get("allow_file_write"),
        default_reviewer_agent_id: row.get("default_reviewer_agent_id"),
        default_coordinator_agent_id: row.get("default_coordinator_agent_id"),
        untagged_human_mode: UntaggedHumanMode::from_str(
            row.get::<_, &str>("untagged_human_mode"),
        )?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
