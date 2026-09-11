//! Reviewed objective snapshots share the analysis transaction and scope.
use super::DbService;
use choruz_common::AppError;
use choruz_domain::evaluation::{EvaluationCase, EvaluationSplit, EvaluationSuite, TraceCase};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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

/// Partition objectives, not individual retries. The
/// assignment is stable as the corpus grows; each queued run owns its snapshot.
pub fn trace_suite(cases: &[TraceCase], budget: usize) -> Option<EvaluationSuite> {
    measured_trace_suite(cases, budget, &BTreeMap::new())
}

pub fn training_objectives(previous: &[TraceCase], changes: &[TraceCase]) -> BTreeSet<String> {
    let mut current: BTreeMap<_, _> = previous
        .iter()
        .map(|c| (c.episode_ref.clone(), c.clone()))
        .collect();
    for c in curate_changes(previous, changes) {
        current.insert(c.episode_ref.clone(), c);
    }
    let current: Vec<_> = current.into_values().collect();
    case_groups(&current)
        .into_iter()
        .filter(|g| !conflicting(g) && group_bucket(g) == 0)
        .flatten()
        .map(|c| c.episode_ref.clone())
        .collect()
}

fn group_bucket(group: &[&TraceCase]) -> usize {
    group
        .iter()
        .flat_map(|c| {
            std::iter::once(&c.episode_ref)
                .chain(c.group_ref.iter())
                .chain(c.classification.iter().flat_map(|c| c.related_refs.iter()))
        })
        .map(|r| usize::from(Sha256::digest(r.as_bytes())[0]) % 3)
        .min()
        .unwrap_or(0)
}

pub fn measured_trace_suite(
    cases: &[TraceCase],
    budget: usize,
    difficulty: &BTreeMap<String, String>,
) -> Option<EvaluationSuite> {
    let groups = case_groups(cases);
    let mut suite = EvaluationSuite {
        name: format!("Observed objectives {}", corpus_version(cases)),
        cases: vec![],
    };
    let per_split = (budget / 4).clamp(1, 16);
    let mut counts = [0usize; 3];
    let mut categories: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for group in groups {
        if conflicting(&group) {
            continue;
        }
        let Some(case) = group
            .iter()
            .find(|c| c.check.is_some() && c.validate().is_ok())
        else {
            continue;
        };
        // Connected duplicates share one partition and one vote. A merge moves
        // toward training, never from exposed training data into a holdout.
        let bucket = group_bucket(&group);
        let category = case
            .classification
            .as_ref()
            .map(|c| format!("{}/{}/{}", c.task_type, c.capability, c.structure))
            .unwrap_or_else(|| "unclassified".into());
        let stratum = if bucket == 0 {
            difficulty
                .get(&case.episode_ref)
                .map(String::as_str)
                .unwrap_or("insufficient")
        } else {
            "held_out"
        };
        categories
            .entry((category, stratum.to_owned()))
            .or_insert_with(Vec::new)
            .push((bucket, *case));
    }
    let mut ordered = Vec::new();
    let max = categories.values().map(Vec::len).max().unwrap_or(0);
    for index in 0..max {
        for group in categories.values() {
            if let Some(case) = group.get(index) {
                ordered.push(*case);
            }
        }
    }
    for (bucket, case) in ordered {
        if counts[bucket] >= per_split {
            continue;
        }
        counts[bucket] += 1;
        suite.cases.push(EvaluationCase {
            environment: None,
            source: Some(case.clone()),
            id: hex::encode(Sha256::digest(case.episode_ref.as_bytes())),
            split: [
                EvaluationSplit::Train,
                EvaluationSplit::Validation,
                EvaluationSplit::Test,
            ][bucket],
            input: if bucket == 0 {
                case.variant.as_ref().unwrap_or(&case.input).clone()
            } else {
                case.input.clone()
            },
            check: case.check.clone()?,
        });
    }
    suite.validate().ok().map(|()| suite)
}

pub(super) fn corpus_version(cases: &[TraceCase]) -> String {
    let canonical: BTreeMap<_, _> = cases.iter().map(|c| (&c.episode_ref, c)).collect();
    hex::encode(Sha256::digest(json!(canonical).to_string().as_bytes()))
}

pub(super) fn changed_case_contexts(previous: &[TraceCase], current: &[TraceCase]) -> Vec<String> {
    let contexts = |cases: &[TraceCase]| {
        let mut result = BTreeMap::new();
        for group in case_groups(cases) {
            let fingerprint = json!(group).to_string();
            for case in group {
                result.insert(case.episode_ref.clone(), fingerprint.clone());
            }
        }
        result
    };
    let before = contexts(previous);
    let after = contexts(current);
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

/// Connected components over reviewed semantic links, exact normalized inputs
/// and shared evidence. Source rows remain intact; only sampling is coalesced.
fn case_groups(cases: &[TraceCase]) -> Vec<Vec<&TraceCase>> {
    let canonical: BTreeMap<_, _> = cases.iter().map(|c| (&c.episode_ref, c)).collect();
    let mut groups: Vec<Vec<&TraceCase>> = Vec::new();
    for case in canonical.into_values() {
        let mut merged = vec![case];
        let mut index = 0;
        while index < groups.len() {
            if groups[index]
                .iter()
                .any(|a| merged.iter().any(|b| related(a, b)))
            {
                merged.extend(groups.remove(index));
                index = 0;
            } else {
                index += 1;
            }
        }
        merged.sort_by_key(|c| &c.episode_ref);
        groups.push(merged);
    }
    groups
}

fn related(a: &TraceCase, b: &TraceCase) -> bool {
    let normalized = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    a.group_ref
        .as_ref()
        .zip(b.group_ref.as_ref())
        .is_some_and(|(a, b)| a == b)
        || (!a.input.trim().is_empty() && normalized(&a.input) == normalized(&b.input))
        || a.evidence.iter().any(|r| b.evidence.contains(r))
        || a.classification
            .as_ref()
            .is_some_and(|c| c.related_refs.contains(&b.episode_ref))
        || b.classification
            .as_ref()
            .is_some_and(|c| c.related_refs.contains(&a.episode_ref))
        || a.classification
            .as_ref()
            .zip(b.classification.as_ref())
            .is_some_and(|(a, b)| a.related_refs.iter().any(|r| b.related_refs.contains(r)))
}

fn conflicting(group: &[&TraceCase]) -> bool {
    let answers: BTreeSet<_> = group.iter().map(|c| json!(c.check).to_string()).collect();
    answers.len() > 1
}

/// Persist only new/changed snapshots, including a surviving member whose
/// component anchor changed. Incoming model-supplied anchors are never trusted.
pub(super) fn curate_changes(previous: &[TraceCase], changes: &[TraceCase]) -> Vec<TraceCase> {
    let mut current: BTreeMap<_, _> = previous
        .iter()
        .map(|c| (c.episode_ref.clone(), c.clone()))
        .collect();
    for case in changes {
        let mut case = case.clone();
        case.group_ref = current
            .get(&case.episode_ref)
            .and_then(|c| c.group_ref.clone());
        current.insert(case.episode_ref.clone(), case);
    }
    let current: Vec<_> = current.into_values().collect();
    let mut result = Vec::new();
    for group in case_groups(&current) {
        let anchor = group
            .iter()
            .flat_map(|c| std::iter::once(&c.episode_ref).chain(c.group_ref.iter()))
            .min_by_key(|r| (Sha256::digest(r.as_bytes())[0] % 3, *r))
            .unwrap()
            .clone();
        for case in group {
            let mut case = case.clone();
            case.group_ref = Some(anchor.clone());
            if changes.iter().any(|c| c.episode_ref == case.episode_ref)
                || previous
                    .iter()
                    .find(|c| c.episode_ref == case.episode_ref)
                    .is_none_or(|old| json!(old) != json!(case))
            {
                result.push(case);
            }
        }
    }
    result
}

/// Immutable quality summary stored with each case-bearing analysis report.
/// Counts describe the bounded working corpus, not the entire trace archive.
pub fn dataset_report(previous: &[TraceCase], cases: &[TraceCase]) -> Value {
    let current: BTreeMap<_, _> = previous
        .iter()
        .map(|c| (c.episode_ref.clone(), c.clone()))
        .collect();
    let mut added = 0;
    let mut updated = 0;
    let mut withdrawn = 0;
    for case in cases {
        match current.get(&case.episode_ref) {
            None => added += 1,
            Some(old) if json!(old) != json!(case) => updated += 1,
            _ => {}
        }
        if case.check.is_none()
            && current
                .get(&case.episode_ref)
                .is_some_and(|c| c.check.is_some())
        {
            withdrawn += 1;
        }
    }
    let groups = case_groups(cases);
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    for case in cases {
        let category = case
            .classification
            .as_ref()
            .map(|c| c.task_type.as_str())
            .unwrap_or("unclassified");
        *categories.entry(category.into()).or_default() += 1;
        let outcome = case
            .classification
            .as_ref()
            .map(|c| c.outcome.as_str())
            .unwrap_or("unclassified");
        *outcomes.entry(outcome.into()).or_default() += 1;
    }
    json!({"version":corpus_version(cases),"previous_version":corpus_version(previous),"total":cases.len(),"added":added,"updated":updated,"withdrawn":withdrawn,
        "unevaluable":cases.iter().filter(|c| c.check.is_none()).count(),"duplicate_groups":groups.iter().filter(|g|g.len()>1).count(),"conflicting_groups":groups.iter().filter(|g|conflicting(g)).count(),
        "categories":categories,"outcomes":outcomes,"variants":cases.iter().filter(|c|c.variant.is_some()).count(),"cases":cases})
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_domain::evaluation::OutputCheck;

    #[test]
    fn objectives_are_deduplicated_partitioned_and_frozen_with_evidence() {
        let cases: Vec<_> = (0..30)
            .map(|i| TraceCase {
                variant: None,
                group_ref: None,
                classification: None,
                episode_ref: format!("one-session:{i}"),
                evidence: vec![format!("result:{i}")],
                input: format!("Increment {i}"),
                check: Some(OutputCheck::Exact {
                    expected: (i + 1).to_string(),
                }),
                reason: "User supplied the exact correction".into(),
            })
            .collect();
        let snapshot = trace_suite(&cases, 16).unwrap();
        let mut varied = cases.clone();
        for c in &mut varied {
            c.variant = Some(format!(
                "Increase {} by one",
                c.input.split_whitespace().last().unwrap()
            ));
        }
        let bands = varied
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    c.episode_ref.clone(),
                    if i % 2 == 0 { "hard" } else { "easy" }.to_owned(),
                )
            })
            .collect();
        let measured = measured_trace_suite(&varied, 16, &bands).unwrap();
        let heldout = |s: &EvaluationSuite| {
            s.cases
                .iter()
                .filter(|c| c.split != EvaluationSplit::Train)
                .map(|c| (&c.id, &c.input))
                .map(|(id, input)| (id.clone(), input.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(heldout(&snapshot), heldout(&measured));
        let train: Vec<_> = measured
            .cases
            .iter()
            .filter(|c| c.split == EvaluationSplit::Train)
            .collect();
        assert!(train.iter().all(|c| c.input.starts_with("Increase ")));
        assert_eq!(
            train.iter().map(|c| &c.id).collect::<BTreeSet<_>>().len(),
            train.len()
        );
        assert!(
            train
                .iter()
                .any(|c| bands[&c.source.as_ref().unwrap().episode_ref] == "hard")
        );
        assert!(
            train
                .iter()
                .any(|c| bands[&c.source.as_ref().unwrap().episode_ref] == "easy")
        );
        assert_eq!(snapshot.cases.len(), 12);
        for case in &snapshot.cases {
            assert!(case.source.is_some());
        }
        let mut repeated = cases.clone();
        repeated.extend(cases.clone());
        assert_eq!(
            serde_json::to_value(trace_suite(&repeated, 16)).unwrap(),
            serde_json::to_value(&snapshot).unwrap()
        );
        let mut withdrawn = cases.clone();
        for case in &mut withdrawn {
            case.check = None;
        }
        assert!(trace_suite(&withdrawn, 16).is_none());
        assert!(
            snapshot
                .cases
                .iter()
                .all(|case| case.source.as_ref().unwrap().check.is_some())
        );
        let mut overlap = cases;
        for case in &mut overlap {
            case.evidence = vec!["same-result".into()];
        }
        assert!(trace_suite(&overlap, 16).is_none());
    }

    #[test]
    fn semantic_groups_conflicts_and_category_sampling_preserve_meaning() {
        use choruz_domain::evaluation::CaseClassification;
        let mut cases: Vec<_> = (0..60)
            .map(|i| TraceCase {
                variant: None,
                group_ref: None,
                episode_ref: format!("task:{i}"),
                evidence: vec![format!("result:{i}")],
                input: format!("Task {i}"),
                check: Some(OutputCheck::Exact {
                    expected: "verified".into(),
                }),
                reason: "Checked result".into(),
                classification: Some(CaseClassification {
                    task_type: if i == 59 { "research" } else { "coding" }.into(),
                    capability: "verification".into(),
                    structure: "multi_step".into(),
                    outcome: "retried".into(),
                    related_refs: vec![],
                }),
            })
            .collect();
        let original = trace_suite(&cases, 12).unwrap();
        assert!(
            original
                .cases
                .iter()
                .any(|c| c.source.as_ref().unwrap().episode_ref == "task:59")
        );
        let version = dataset_report(&[], &cases);
        assert_eq!(version["categories"]["research"], 1);
        assert_eq!(version["outcomes"]["retried"], 60);
        let mut durable = cases.clone();
        let low = durable
            .iter()
            .position(|c| Sha256::digest(c.episode_ref.as_bytes())[0] % 3 == 0)
            .unwrap();
        let high = durable
            .iter()
            .position(|c| Sha256::digest(c.episode_ref.as_bytes())[0] % 3 == 2)
            .unwrap();
        durable[low].classification.as_mut().unwrap().related_refs =
            vec![durable[high].episode_ref.clone()];
        let mut durable = curate_changes(&[], &durable);
        let low_ref = cases[low].episode_ref.clone();
        let high_ref = cases[high].episode_ref.clone();
        assert_eq!(
            durable
                .iter()
                .find(|c| c.episode_ref == high_ref)
                .unwrap()
                .group_ref
                .as_deref(),
            Some(low_ref.as_str())
        );
        durable.retain(|c| c.episode_ref != low_ref);
        let refreshed = curate_changes(&durable, &[]);
        assert!(
            refreshed.is_empty(),
            "pruning must not reassign the surviving group"
        );
        let surviving = trace_suite(&durable, 256).unwrap();
        assert_eq!(
            surviving
                .cases
                .iter()
                .find(|c| c.source.as_ref().unwrap().episode_ref == high_ref)
                .unwrap()
                .split,
            EvaluationSplit::Train
        );
        cases[1]
            .classification
            .as_mut()
            .unwrap()
            .related_refs
            .push("task:0".into());
        let grouped = trace_suite(&cases, 256).unwrap();
        assert_eq!(
            grouped
                .cases
                .iter()
                .filter(|c| ["task:0", "task:1"]
                    .contains(&c.source.as_ref().unwrap().episode_ref.as_str()))
                .count(),
            1
        );
        cases[1].check = Some(OutputCheck::Exact {
            expected: "contradiction".into(),
        });
        let conflict = trace_suite(&cases, 256).unwrap();
        assert!(!conflict.cases.iter().any(|c| {
            ["task:0", "task:1"].contains(&c.source.as_ref().unwrap().episode_ref.as_str())
        }));
        let report = dataset_report(&[], &cases);
        assert_eq!(report["conflicting_groups"], 1);
        assert_ne!(report["version"], version["version"]);
        assert_eq!(version["conflicting_groups"], 0);
    }
}
