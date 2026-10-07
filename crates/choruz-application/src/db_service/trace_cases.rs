//! Reviewed objective snapshots share the analysis transaction and scope.
use super::DbService;
use choruz_common::AppError;
use choruz_evaluation::evaluation::TraceCase;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

impl DbService {
    pub async fn experience_trace_cases(
        &self,
        workspace: &str,
        binding: &str,
    ) -> Result<Vec<TraceCase>, AppError> {
        let client = self.store.connect().await?;
        read_trace_cases(&client, workspace, binding).await
    }
}

pub(super) async fn read_trace_cases(
    client: &impl deadpool_postgres::GenericClient,
    workspace: &str,
    binding: &str,
) -> Result<Vec<TraceCase>, AppError> {
    let rows = client.query("SELECT validation->'evaluation_cases' AS cases FROM experience_revision WHERE workspace_id=$1 AND binding_id=$2 AND jsonb_array_length(validation->'evaluation_cases')>0 ORDER BY created_at DESC,id DESC LIMIT 100", &[&workspace,&binding]).await
            .map_err(|e| AppError::Internal(format!("read trace cases: {e}")))?;
    let mut latest = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut bytes = 0usize;
    for row in rows {
        let cases: Vec<TraceCase> = serde_json::from_value(row.get::<_, Value>("cases"))
            .map_err(|_| AppError::Internal("Invalid stored trace cases".into()))?;
        for case in cases {
            if !seen.insert(case.episode_ref.clone()) {
                continue;
            }
            let size = serde_json::to_vec(&case)
                .map_err(|e| AppError::Internal(e.to_string()))?
                .len();
            if latest.len() >= 64 || bytes + size > 48 * 1024 {
                continue;
            }
            bytes += size;
            latest.entry(case.episode_ref.clone()).or_insert(case);
        }
    }
    Ok(latest.into_values().collect())
}
