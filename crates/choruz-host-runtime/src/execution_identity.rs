//! Credential-free execution identity shared by learning reservations and evaluation.
use crate::TerminalSpec;
use serde::{Deserialize, Serialize};

/// Stable execution inputs, excluding terminal attachment and credential material.
/// Callers add their procedure digest, policy generation and reservation scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionIdentity {
    driver: String,
    binary: Option<String>,
    workspace: String,
    model: Option<String>,
    host: Option<String>,
    account: Option<String>,
    profile: Option<String>,
    directory: Option<String>,
}

impl From<&TerminalSpec> for ExecutionIdentity {
    fn from(spec: &TerminalSpec) -> Self {
        let field = |key: &str| spec.harness_account[key].as_str().map(str::to_owned);
        Self {
            driver: spec.driver_type.clone(),
            binary: spec.binary_path.clone(),
            workspace: spec.workspace_path.clone(),
            model: spec.model.clone(),
            host: field("runtime_host_id"),
            account: field("harness_account_id"),
            profile: field("harness_account_profile_kind"),
            directory: field("harness_account_profile_directory"),
        }
    }
}
