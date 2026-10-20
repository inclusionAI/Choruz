//! Resolve retained native evidence under the learning profile's current authority.
use crate::{
    ApiError, ApiState,
    handlers_terminals::{authorize_terminal_binding, terminal_spec},
};
use choruz_agent_runtime::RuntimeBinding;
use choruz_common::AppError;
use choruz_domain::Principal;
use choruz_host_runtime::TerminalSpec;

pub(crate) struct NativeSource {
    pub key: String,
    pub binding: RuntimeBinding,
    pub spec: TerminalSpec,
}

pub(crate) async fn native_sources(
    state: &ApiState,
    actor: &Principal,
    workspace: &str,
    target: &RuntimeBinding,
) -> Result<Vec<NativeSource>, AppError> {
    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in state
        .db
        .learning_source_bindings(workspace, &target.id)
        .await?
    {
        let binding = match authorize_terminal_binding(state, actor, &id).await {
            Ok(binding) => binding,
            Err(ApiError(AppError::Forbidden(_) | AppError::NotFound(_))) if id != target.id => {
                continue;
            }
            Err(error) => return Err(error.0),
        };
        if state
            .db
            .learning_binding_id(workspace, &binding.id, Some(&actor.id))
            .await?
            != target.id
        {
            continue;
        }
        let mut retained = state.runtime.retained_native_sources(&binding).await?;
        if let Some(anchor) = binding.valid_terminal_session_anchor_for_context(
            None,
            None,
            Some(binding.terminal_generation()),
            None,
        ) && !retained
            .iter()
            .any(|source| source.session_id == anchor.session_id)
        {
            retained.push(anchor);
        }
        for source in retained {
            if seen.insert(source.session_id.clone()) {
                sources.push(native_source(
                    &target.id,
                    &binding,
                    Some(source.session_id),
                    Some(source.native_home_path),
                ));
            }
        }
        if let Some(external) = &binding.external_session_id
            && seen.insert(external.clone())
        {
            sources.push(native_source(
                &target.id,
                &binding,
                Some(external.clone()),
                None,
            ));
        }
    }
    if sources.is_empty() {
        sources.push(native_source(&target.id, target, None, None));
    }
    Ok(sources)
}

fn native_source(
    root: &str,
    binding: &RuntimeBinding,
    session: Option<String>,
    home: Option<String>,
) -> NativeSource {
    let session_key = session.as_deref().unwrap_or("live");
    NativeSource {
        key: if binding.id == root {
            session_key.to_owned()
        } else {
            format!("task:{}:{session_key}", binding.id)
        },
        spec: terminal_spec(
            binding,
            120,
            40,
            session,
            home.filter(|path| !path.is_empty()),
        ),
        binding: binding.clone(),
    }
}
