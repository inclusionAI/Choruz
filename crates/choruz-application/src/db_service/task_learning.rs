use super::DbService;
use choruz_common::AppError;
use serde_json::{Value, json};

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("task learning: {error}"))
}

impl DbService {
    pub async fn learning_binding_id(
        &self,
        workspace: &str,
        binding: &str,
        owner: Option<&str>,
    ) -> Result<String, AppError> {
        Ok(self
            .store
            .connect()
            .await?
            .query_one(
                "SELECT effective_experience_binding($1,$2,$3)",
                &[&workspace, &binding, &owner],
            )
            .await
            .map_err(internal)?
            .get(0))
    }

    pub async fn configure_task_profile(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        enabled: bool,
    ) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(internal)?;
        let previous = tx.query_opt(
            "SELECT COALESCE(profile.enabled,FALSE) AS reuse FROM experience_policy p LEFT JOIN experience_task_profile profile ON profile.binding_id=p.binding_id WHERE p.workspace_id=$1 AND p.owner_id=$2 AND p.binding_id=$3 FOR UPDATE OF p",
            &[&workspace,&owner,&binding],
        ).await.map_err(internal)?.ok_or_else(||AppError::Forbidden("Learning profile belongs to another scope".into()))?;
        let changed = tx.execute(
            "INSERT INTO experience_task_profile(binding_id,enabled) SELECT binding_id,$4 FROM experience_policy WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3
             ON CONFLICT(binding_id) DO UPDATE SET enabled=EXCLUDED.enabled,updated_at=NOW()",
            &[&workspace,&owner,&binding,&enabled],
        ).await.map_err(internal)?;
        if changed != 1 {
            return Err(AppError::Forbidden(
                "Learning profile belongs to another scope".into(),
            ));
        }
        if enabled && !previous.get::<_, bool>("reuse") {
            tx.execute("UPDATE experience_policy SET community_settings=jsonb_set(community_settings,'{contribute}','false'::jsonb,TRUE),generation=generation+1,lease_token=NULL,lease_until=NULL,updated_at=NOW() WHERE binding_id=$1 AND workspace_id=$2", &[&binding,&workspace]).await.map_err(internal)?;
            tx.execute("UPDATE experience_behavior_event SET lease_token=NULL,lease_until=NULL WHERE workspace_id=$1 AND binding_id=$2 AND publication_state IN('preparing','local','blocked')", &[&workspace,&binding]).await.map_err(internal)?;
        }
        tx.commit().await.map_err(internal)?;
        Ok(())
    }

    /// Only the task-first opt-in path inherits. The transaction records even a
    /// no-match result, so a provisioning retry cannot switch to a newer profile.
    pub async fn inherit_task_learning(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
    ) -> Result<(), AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(internal)?;
        let child = tx.query_one(
            "SELECT b.config_json,b.driver_type,b.workspace_path FROM agent_runtime_bindings b JOIN conversation c ON c.id=b.conversation_id WHERE b.id=$1 AND c.workspace_id=$2 AND b.state<>'disabled' FOR UPDATE OF b",
            &[&binding,&workspace],
        ).await.map_err(internal)?;
        let mut config: Value = child.get("config_json");
        if config["inherit_learning"] != true || config["learning_inheritance_checked"] == true {
            return Ok(());
        }
        let driver: String = child.get("driver_type");
        let path: String = child.get("workspace_path");
        let model = config["model"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let root = tx.query_opt(
            "SELECT p.binding_id,b.config_json->'model' AS model FROM experience_task_profile profile
             JOIN experience_policy p ON p.binding_id=profile.binding_id JOIN agent_runtime_bindings b ON b.id=p.binding_id JOIN conversation c ON c.id=b.conversation_id
             WHERE profile.enabled AND p.enabled AND p.workspace_id=$1 AND p.owner_id=$2 AND c.workspace_id=$1 AND b.state<>'disabled'
               AND b.id<>$3 AND b.driver_type=$4 AND b.workspace_path=$5
               AND COALESCE(b.config_json->'runtime_host_id','null'::jsonb)=COALESCE($6::jsonb->'runtime_host_id','null'::jsonb)
               AND COALESCE(b.config_json->'harness_account_id','null'::jsonb)=COALESCE($6::jsonb->'harness_account_id','null'::jsonb)
               AND ($7::text IS NULL OR NULLIF(BTRIM(b.config_json->>'model'),'')=$7)
               AND NOT EXISTS(SELECT 1 FROM experience_policy own WHERE own.binding_id=$3)
             ORDER BY profile.updated_at DESC,p.binding_id LIMIT 1 FOR UPDATE OF b",
            &[&workspace,&owner,&binding,&driver,&path,&config,&model],
        ).await.map_err(internal)?;
        if let Some(root) = root {
            let root_id: String = root.get("binding_id");
            if let Some(selected) = root
                .get::<_, Option<Value>>("model")
                .filter(Value::is_string)
            {
                choruz_agent_runtime::headless::validate_model(
                    selected.as_str().expect("string model"),
                )
                .map_err(|error| AppError::Validation(error.into()))?;
                config["model"] = selected;
            }
            tx.execute("INSERT INTO experience_task_link(binding_id,profile_binding_id) VALUES($1,$2) ON CONFLICT DO NOTHING", &[&binding,&root_id]).await.map_err(internal)?;
            tx.execute(
                "UPDATE experience_task_profile SET shared_task_history=TRUE WHERE binding_id=$1",
                &[&root_id],
            )
            .await
            .map_err(internal)?;
        }
        config["learning_inheritance_checked"] = json!(true);
        tx.execute(
            "UPDATE agent_runtime_bindings SET config_json=$2,updated_at=NOW() WHERE id=$1",
            &[&binding, &config],
        )
        .await
        .map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        Ok(())
    }

    pub async fn learning_source_bindings(
        &self,
        workspace: &str,
        root: &str,
    ) -> Result<Vec<String>, AppError> {
        Ok(self.store.connect().await?.query(
            "SELECT b.id FROM agent_runtime_bindings b JOIN conversation c ON c.id=b.conversation_id WHERE c.workspace_id=$1 AND effective_experience_binding($1,b.id,NULL)=$2 ORDER BY b.created_at,b.id",
            &[&workspace,&root],
        ).await.map_err(internal)?.into_iter().map(|row|row.get(0)).collect())
    }
}
