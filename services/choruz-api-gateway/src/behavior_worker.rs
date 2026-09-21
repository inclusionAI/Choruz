//! Materialize reusable experience separately from foreground turns and learning checks.
use crate::{
    ApiState,
    handlers_terminals::{authorize_terminal_binding, terminal_spec},
    host_runtime::RuntimeHost,
};
use choruz_application::db_service::BehaviorClaim;
use choruz_common::AppError;
use choruz_community::behavior::{BehaviorRecord, ModelAttribution, ProblemCard, SCHEMA_VERSION};
use choruz_host_runtime::{
    HostRequest,
    experience_source::{Cursor, HistoricalRecord},
};
use choruz_learning::BehaviorDraft;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub(crate) async fn run(state: &ApiState) {
    tokio::join!(run_preparation(state), run_community(state));
}

async fn run_community(state: &ApiState) {
    let Ok(hub) = choruz_community::hub::Hub::new() else {
        tracing::error!("Community HTTP client could not start");
        return;
    };
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut next_sync = tokio::time::Instant::now();
    loop {
        interval.tick().await;
        if tokio::time::Instant::now() >= next_sync {
            let sync = async {
                if !state.db.community_sync_due().await? {
                    return Ok::<_, AppError>(());
                }
                next_sync = tokio::time::Instant::now() + Duration::from_secs(300);
                let previous = state.db.community_snapshot_revision().await?;
                let cached = state.db.community_cached_records().await?;
                if let Some(snapshot) = tokio::time::timeout(
                    Duration::from_secs(120),
                    hub.snapshot(previous.as_deref(), &cached),
                )
                .await
                .map_err(|_| AppError::Internal("Community synchronization timed out".into()))??
                {
                    state
                        .db
                        .save_community_snapshot(
                            &snapshot.revision,
                            &snapshot.records,
                            &snapshot.objects,
                        )
                        .await?;
                } else if let Some(revision) = previous {
                    state.db.community_sync_unchanged(&revision).await?;
                }
                Ok(())
            }
            .await;
            if sync.is_err() {
                if let Err(error) = state.db.community_sync_error("Community synchronization failed; the last accepted snapshot remains available.").await {
                    tracing::warn!(%error,"community sync diagnostic failed");
                }
            }
        }
        let Ok(token) = std::env::var("CHORUZ_COMMUNITY_HF_TOKEN") else {
            continue;
        };
        if token.trim().is_empty() {
            continue;
        }
        match state.db.claim_behavior_publication().await {
            Ok(Some(claim)) => {
                let public = prepare_public(state, &claim).await;
                match public {
                    Ok(Some(record)) => {
                        let outcome = hub.contribute(&token,&record).await;
                        let error = outcome.as_ref().err().map(|_|"Contribution result is uncertain. Automatic retry is disabled; check the community before resubmitting.");
                        if let Err(error) = state.db.finish_behavior_publication(&claim,outcome.as_deref().ok(),true,error).await {
                            tracing::warn!(event_id=%claim.id,%error,"community publication result failed");
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        if let Err(error) = state.db.finish_behavior_publication(&claim,None,false,Some("Public projection or independent privacy review did not pass. The experience remains local.")).await {
                            tracing::warn!(event_id=%claim.id,%error,"community privacy diagnostic failed");
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error,"community publication queue unavailable"),
        }
    }
}

async fn prepare_public(
    state: &ApiState,
    claim: &choruz_application::db_service::PublicationClaim,
) -> Result<Option<BehaviorRecord>, AppError> {
    let actor = state.db.get_principal(&claim.owner_id).await?;
    authorize_terminal_binding(state, &actor, &claim.binding_id)
        .await
        .map_err(|e| e.0)?;
    let analyst = authorize_terminal_binding(state, &actor, &claim.analyst_binding_id)
        .await
        .map_err(|e| e.0)?;
    let host = RuntimeHost::for_binding(state, &analyst)?;
    let public: Option<BehaviorRecord> = host
        .call(HostRequest::RedactBehavior {
            spec: Box::new(terminal_spec(&analyst, 120, 40, None, None)),
            record: Box::new(claim.record.clone()),
        })
        .await?;
    let public =
        public.ok_or_else(|| AppError::Validation("Public projection abstained".into()))?;
    let encoded = json!(public);
    if crate::handlers_events::sanitize_telemetry_value(encoded.clone()) != encoded {
        return Err(AppError::Validation(
            "Public projection retains a recognized credential".into(),
        ));
    }
    let review: choruz_learning::PrivacyReview = host
        .call(HostRequest::ReviewBehavior {
            spec: Box::new(terminal_spec(&analyst, 120, 40, None, None)),
            prompt: json!({"private":claim.record,"public":public}).to_string(),
        })
        .await?;
    if !review.accepted {
        return Err(AppError::Validation(
            "Public privacy review rejected the projection".into(),
        ));
    }
    let dispatched = state
        .db
        .begin_behavior_publication(claim, &public, &json!({"accepted":true,"reason":review.reason,"skill_sha256":format!("{:x}",Sha256::digest(choruz_learning::PRIVACY_REVIEW_SKILL.as_bytes()))}))
        .await?;
    Ok(dispatched.then_some(public))
}

async fn run_preparation(state: &ApiState) {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        match state.db.claim_behavior().await {
            Ok(Some(claim)) => {
                let outcome = prepare(state, &claim).await;
                let error = outcome.as_ref().err().map(|_| "Unable to prepare grounded behavior evidence. Check the analysis account and original source.");
                if let Err(error) = state
                    .db
                    .finish_behavior(&claim, outcome.as_ref().ok(), error)
                    .await
                {
                    tracing::warn!(event_id=%claim.id, %error, "behavior evidence commit failed");
                }
                state
                    .db
                    .record_audit(
                        &claim.workspace_id,
                        &claim.owner_id,
                        "learning.behavior.prepare",
                        "agent_binding",
                        &claim.binding_id,
                        json!({"event_id":claim.id,"success":outcome.is_ok()}),
                    )
                    .await
                    .unwrap_or_else(|error| tracing::warn!(%error,"behavior audit failed"));
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error,"behavior queue unavailable"),
        }
    }
}

async fn prepare(state: &ApiState, claim: &BehaviorClaim) -> Result<BehaviorRecord, AppError> {
    let actor = state.db.get_principal(&claim.owner_id).await?;
    let target = authorize_terminal_binding(state, &actor, &claim.binding_id)
        .await
        .map_err(|e| e.0)?;
    let analyst = authorize_terminal_binding(state, &actor, &claim.analyst_binding_id)
        .await
        .map_err(|e| e.0)?;
    let mut references: Vec<String> = serde_json::from_value(claim.references.clone())
        .map_err(|_| AppError::Validation("Invalid behavior evidence references".into()))?;
    references.push(claim.episode_ref.clone());
    references.sort();
    references.dedup();
    let mut records = state.db.behavior_feedback(claim, &references).await?;
    let host = RuntimeHost::for_binding(state, &target)?;
    let anchor = target.valid_terminal_session_anchor_for_context(
        None,
        None,
        Some(target.terminal_generation()),
        None,
    );
    for cursor in claim
        .cursor
        .as_object()
        .into_iter()
        .flat_map(|map| map.values())
    {
        let Ok(cursor) = serde_json::from_value::<Cursor>(cursor.clone()) else {
            continue;
        };
        let requested: Vec<_> = references
            .iter()
            .filter(|r| r.starts_with(&format!("{}:", cursor.session)))
            .cloned()
            .collect();
        if requested.is_empty() {
            continue;
        }
        let home = anchor
            .as_ref()
            .filter(|a| a.session_id == cursor.session)
            .map(|a| a.native_home_path.clone())
            .filter(|p| !p.is_empty());
        let recovered: Vec<HistoricalRecord> = host
            .call(HostRequest::BehaviorReferences {
                spec: Box::new(terminal_spec(
                    &target,
                    120,
                    40,
                    Some(cursor.session.clone()),
                    home,
                )),
                cursor,
                references: requested,
            })
            .await?;
        records.extend(
            recovered
                .into_iter()
                .map(|r| json!({"ref":r.reference,"record":r.record})),
        );
    }
    if references
        .iter()
        .any(|reference| !records.iter().any(|r| r["ref"] == *reference))
    {
        return Err(AppError::Validation(
            "Original behavior evidence is unavailable".into(),
        ));
    }
    if serde_json::to_vec(&records)
        .map_err(|_| AppError::Internal("Encode behavior evidence".into()))?
        .len()
        > 160 * 1024
    {
        return Err(AppError::Validation(
            "Behavior evidence exceeds the source window limit".into(),
        ));
    }
    let draft: Option<BehaviorDraft> = RuntimeHost::for_binding(state,&analyst)?.call(HostRequest::ExtractBehavior {
        spec:Box::new(terminal_spec(&analyst,120,40,None,None)),
        prompt:json!({"public_projection":false,"problem":claim.description,"observed_outcome":claim.kind,"solution":claim.solution,"records":records}).to_string(),
    }).await?;
    let draft =
        draft.ok_or_else(|| AppError::Validation("Behavior extraction abstained".into()))?;
    let model = observed_model(&records);
    let record = BehaviorRecord {
        schema_version: SCHEMA_VERSION,
        id: claim.id.clone(),
        occurrence_id: claim.occurrence_id.clone(),
        problem: ProblemCard {
            id: claim.problem_id.clone(),
            title: draft.title,
            input_background: draft.input_background,
            expected_behavior: draft.expected_behavior,
            bad_behavior: draft.bad_behavior,
            applicability: draft.applicability,
            tags: draft.tags,
        },
        model: ModelAttribution {
            observed: model,
            configured: terminal_spec(&target, 120, 40, None, None).model,
            harness: terminal_spec(&target, 120, 40, None, None).driver_type,
            harness_version: observed_metadata(&records, "harness_version"),
        },
        solution: claim.solution.clone(),
        kind: claim.kind,
        evidence_summary: draft.evidence_summary,
    };
    record.validate().map_err(AppError::Validation)?;
    Ok(record)
}

fn observed_model(records: &[Value]) -> Option<String> {
    observed_metadata(records, "model")
}

fn observed_metadata(records: &[Value], field: &str) -> Option<String> {
    let models: std::collections::BTreeSet<_> = records
        .iter()
        .filter_map(|r| r["record"]["_execution"][field].as_str())
        .filter(|m| !m.is_empty() && *m != "<synthetic>")
        .collect();
    (models.len() == 1).then(|| models.into_iter().next().unwrap().to_owned())
}
