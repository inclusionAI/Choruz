//! The same exchange record is used locally and publicly. Private source linkage
//! belongs to storage, not an optional field that an exporter must remember to remove.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION: u32 = 1;
pub const COMMUNITY_REPOSITORY: &str = "gjcjcg/ai-bad-behavior-library";

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommunitySettings {
    pub search: bool,
    pub automatic_trial: bool,
    pub contribute: bool,
}

impl CommunitySettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.automatic_trial && !self.search {
            return Err("Community trials require community search".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProblemCard {
    pub id: String,
    pub title: String,
    pub input_background: String,
    pub expected_behavior: String,
    pub bad_behavior: String,
    pub applicability: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelAttribution {
    /// Exact model observed in the execution source, never inferred from settings.
    pub observed: Option<String>,
    pub configured: Option<String>,
    pub harness: String,
    pub harness_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SolutionVersion {
    pub id: String,
    pub based_on: Vec<String>,
    pub instruction: String,
    pub team: Option<choruz_evaluation::team::Team>,
    pub applicability: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Encountered,
    Applied,
    Effective,
    Ineffective,
    Recurrence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BehaviorRecord {
    pub schema_version: u32,
    pub id: String,
    /// Opaque stable identity for one independent objective; not a trace path/hash.
    pub occurrence_id: String,
    pub problem: ProblemCard,
    pub model: ModelAttribution,
    pub solution: Option<SolutionVersion>,
    pub kind: EvidenceKind,
    pub evidence_summary: String,
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl BehaviorRecord {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION
            || !valid_id(&self.id)
            || !valid_id(&self.occurrence_id)
            || !valid_id(&self.problem.id)
            || !text(&self.problem.title, 240)
            || !text(&self.problem.input_background, 4000)
            || !text(&self.problem.expected_behavior, 2000)
            || !text(&self.problem.bad_behavior, 2000)
            || !text(&self.problem.applicability, 2000)
            || self.problem.tags.len() > 12
            || self.problem.tags.iter().any(|t| !text(t, 80))
            || !text(&self.model.harness, 80)
            || [
                &self.model.observed,
                &self.model.configured,
                &self.model.harness_version,
            ]
            .iter()
            .any(|v| v.as_ref().is_some_and(|v| !text(v, 200)))
            || !text(&self.evidence_summary, 4000)
        {
            return Err("Invalid or unsupported behavior record".into());
        }
        if let Some(solution) = &self.solution {
            if !valid_id(&solution.id)
                || solution.based_on.len() > 8
                || solution.based_on.iter().any(|id| !valid_id(id))
                || !text(&solution.applicability, 2000)
                || solution.instruction.len() > 16000
                || (solution.instruction.trim().is_empty() && solution.team.is_none())
            {
                return Err("Invalid behavior solution".into());
            }
            if let Some(team) = &solution.team {
                team.validate(4)?;
            }
        } else if self.kind != EvidenceKind::Encountered {
            return Err("Solution outcomes require an exact solution version".into());
        }
        Ok(())
    }

    /// Redaction may rewrite prose, but cannot invent a result or change identity.
    pub fn same_evidence_identity(&self, other: &Self) -> bool {
        self.id == other.id
            && self.occurrence_id == other.occurrence_id
            && self.problem.id == other.problem.id
            && self.model == other.model
            && self.kind == other.kind
            && self.solution.as_ref().map(|s| &s.id) == other.solution.as_ref().map(|s| &s.id)
            && self.solution.as_ref().map(|s| &s.based_on)
                == other.solution.as_ref().map(|s| &s.based_on)
    }
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct EvidenceCounts {
    pub encountered: usize,
    pub applied: usize,
    pub effective: usize,
    pub ineffective: usize,
    pub recurrence: usize,
}

/// Counts claims, not a failure rate. A later contradictory verdict prevents an
/// occurrence from being advertised as effective; repeated imports add nothing.
pub fn counts(records: &[BehaviorRecord]) -> EvidenceCounts {
    let mut events: BTreeMap<(&str, &str, Option<&str>), BTreeSet<u8>> = BTreeMap::new();
    for record in records {
        let kinds = events
            .entry((
                &record.problem.id,
                &record.occurrence_id,
                record.solution.as_ref().map(|s| s.id.as_str()),
            ))
            .or_default();
        kinds.insert(match record.kind {
            EvidenceKind::Encountered => 0,
            EvidenceKind::Applied => 1,
            EvidenceKind::Effective => 2,
            EvidenceKind::Ineffective => 3,
            EvidenceKind::Recurrence => 4,
        });
    }
    let mut result = EvidenceCounts::default();
    let mut encountered = BTreeSet::new();
    for ((problem, occurrence, _), kinds) in events {
        if kinds.contains(&0) || kinds.contains(&4) {
            encountered.insert((problem, occurrence));
        }
        result.applied += usize::from(kinds.iter().any(|kind| *kind != 0));
        result.effective +=
            usize::from(kinds.contains(&2) && !kinds.contains(&3) && !kinds.contains(&4));
        result.ineffective += usize::from(kinds.contains(&3));
        result.recurrence += usize::from(kinds.contains(&4));
    }
    result.encountered = encountered.len();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> BehaviorRecord {
        serde_json::from_value(json!({
            "schema_version":1,"id":"event-a","occurrence_id":"objective-a",
            "problem":{"id":"problem-a","title":"Premature completion","input_background":"A task required a passing check.","expected_behavior":"Verify the check result.","bad_behavior":"Reported completion without a check.","applicability":"Tasks with explicit checks","tags":["verification"]},
            "model":{"observed":null,"configured":"requested-model","harness":"codex_terminal","harness_version":null},
            "solution":{"id":"solution-a","based_on":[],"instruction":"Verify checks before reporting completion.","team":null,"applicability":"Tasks with explicit checks"},
            "kind":"encountered","evidence_summary":"A later user correction identified the missing check."
        })).unwrap()
    }

    #[test]
    fn independent_evidence_not_reanalysis_owns_counts_and_later_disputes() {
        let first = record();
        let mut applied = first.clone();
        applied.id = "event-b".into();
        applied.kind = EvidenceKind::Applied;
        let mut success = applied.clone();
        success.id = "event-c".into();
        success.kind = EvidenceKind::Effective;
        let mut dispute = success.clone();
        dispute.id = "event-d".into();
        dispute.kind = EvidenceKind::Ineffective;
        let mut another = first.clone();
        another.id = "event-e".into();
        another.occurrence_id = "objective-b".into();
        let records = vec![
            first.clone(),
            first,
            applied,
            success.clone(),
            success,
            dispute,
            another,
        ];
        assert_eq!(
            counts(&records),
            EvidenceCounts {
                encountered: 2,
                applied: 1,
                effective: 0,
                ineffective: 1,
                recurrence: 0
            }
        );
    }

    #[test]
    fn exchange_rejects_private_fields_and_fabricated_versioned_outcomes() {
        let original = record();
        original.validate().unwrap();
        assert_eq!(original.model.observed, None);
        for field in [
            "trace",
            "source_references",
            "credential",
            "private_context",
        ] {
            let mut value = json!(original);
            value[field] = json!("private");
            assert!(serde_json::from_value::<BehaviorRecord>(value).is_err());
        }
        let mut value = json!(original);
        value["solution"]["command"] = json!("download-and-execute");
        assert!(serde_json::from_value::<BehaviorRecord>(value).is_err());
        let mut altered = original.clone();
        altered.kind = EvidenceKind::Effective;
        assert!(!original.same_evidence_identity(&altered));
        altered.solution = None;
        assert!(altered.validate().is_err());
        let mut unsupported = original.clone();
        unsupported.schema_version = 2;
        assert!(unsupported.validate().is_err());
        let mut traversal = original.clone();
        traversal.id = "../private".into();
        assert!(traversal.validate().is_err());
        let mut anonymous = original.clone();
        anonymous.problem.input_background = "An anonymous task required a check.".into();
        assert!(original.same_evidence_identity(&anonymous));
    }
}
