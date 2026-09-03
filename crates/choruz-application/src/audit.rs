use choruz_common::AppResult;
use serde_json::json;

use crate::{ChatApp, MetricsSnapshot, PhaseStatus};

impl ChatApp {
    pub fn phase_status(&self) -> PhaseStatus {
        PhaseStatus {
            phase_0_complete: true,
            phase_1_complete: true,
            phase_2_in_progress: true,
        }
    }

    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        let state = self.inner.read().expect("lock poisoned");
        MetricsSnapshot {
            principals_total: state.principals.len(),
            conversations_total: state.conversations.len(),
            messages_total: state.messages_injected,
            audit_logs_total: state.audit_logs.len(),
            event_backlog_total: state.events.values().map(Vec::len).sum(),
        }
    }

    pub fn audit_attachment_upload(
        &self,
        actor_id: &str,
        attachment_id: &str,
        filename: &str,
    ) -> AppResult<()> {
        let mut state = self.inner.write().expect("lock poisoned");
        let actor = self.require_active_principal(&state, actor_id)?;
        self.record_audit(
            &mut state,
            &actor,
            "attachment.uploaded",
            "attachment",
            attachment_id,
            json!({ "filename": filename }),
        );
        Ok(())
    }
}
