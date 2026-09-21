//! Fixed evaluation tasks and assessments. Answer keys never enter the task prompt.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationSuite {
    pub name: String,
    pub cases: Vec<EvaluationCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationCase {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<ReplayEnvironment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TraceCase>,
    pub id: String,
    pub split: EvaluationSplit,
    pub input: String,
    pub check: OutputCheck,
}

/// A user-supplied reproducible starting point, never an inferred live snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayEnvironment {
    pub image: String,
    pub files: std::collections::BTreeMap<String, String>,
    pub verification: Vec<String>,
    pub max_steps: usize,
}

impl ReplayEnvironment {
    pub fn validate(&self) -> Result<(), String> {
        let digest = self.image.strip_prefix("sha256:").unwrap_or_default();
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !(1..=8).contains(&self.max_steps)
            || self.files.len() > 64
            || self.files.iter().any(|(path, text)| {
                path.is_empty()
                    || path.len() > 240
                    || path
                        .split('/')
                        .any(|part| part.is_empty() || part == "." || part == "..")
                    || path.contains(['\\', '\0', ':'])
                    || text.len() > 64_000
            })
            || self.files.values().map(String::len).sum::<usize>() > 256_000
            || !(1..=8).contains(&self.verification.len())
            || self.verification.iter().any(|command| {
                command.trim().is_empty() || command.len() > 4000 || command.contains('\0')
            })
        {
            return Err("Replay requires a pinned image, bounded relative files, fixed checks and 1 to 8 steps".into());
        }
        Ok(())
    }
}

/// One objective, including its retries and later corrections. A null check
/// withdraws an earlier answer rather than silently retaining stale truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceCase {
    /// A source-equivalent paraphrase, admitted by the fixed task reviewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Storage-owned component anchor, retained when an older member ages out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<CaseClassification>,
    pub episode_ref: String,
    pub evidence: Vec<String>,
    pub input: String,
    pub check: Option<OutputCheck>,
    pub reason: String,
}

/// Fixed annotation vocabulary; related references identify paraphrases of the
/// same problem, not merely tasks exercising the same capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseClassification {
    pub task_type: String,
    pub capability: String,
    pub structure: String,
    pub outcome: String,
    pub related_refs: Vec<String>,
}

impl CaseClassification {
    pub fn validate(&self) -> Result<(), String> {
        if ![
            "coding",
            "math",
            "research",
            "writing",
            "data",
            "operations",
            "other",
        ]
        .contains(&self.task_type.as_str())
            || ![
                "reasoning",
                "debugging",
                "planning",
                "extraction",
                "instruction_following",
                "verification",
                "other",
            ]
            .contains(&self.capability.as_str())
            || !["single_step", "multi_step"].contains(&self.structure.as_str())
            || !["direct", "retried", "incomplete", "disputed"].contains(&self.outcome.as_str())
            || self.related_refs.len() > 64
            || self
                .related_refs
                .iter()
                .any(|r| r.is_empty() || r.len() > 256)
        {
            return Err("Case classification must use the fixed vocabulary and bounded objective references".into());
        }
        Ok(())
    }
}

impl TraceCase {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(check) = &self.check {
            check.validate()?;
        }
        if let Some(classification) = &self.classification {
            classification.validate()?;
        }
        if self.episode_ref.is_empty()
            || self
                .variant
                .as_ref()
                .is_some_and(|v| v.trim().is_empty() || v.len() > 16_000)
            || self
                .group_ref
                .as_ref()
                .is_some_and(|r| r.is_empty() || r.len() > 256)
            || self.episode_ref.len() > 256
            || self.evidence.is_empty()
            || self.evidence.len() > 20
            || self.evidence.iter().any(|r| r.is_empty() || r.len() > 256)
            || self.input.len() > 16_000
            || self.reason.trim().is_empty()
            || self.reason.len() > 2000
            || (self.check.is_some() && self.input.trim().is_empty())
            || serde_json::to_string(&self.check)
                .map_err(|e| e.to_string())?
                .len()
                > 16_000
        {
            return Err(
                "Trace case requires bounded source references and an answer rationale".into(),
            );
        }
        Ok(())
    }
}

/// A batch-level approval must cite every admitted case, not only its first row.
pub fn case_review_covers(cases: &[TraceCase], evidence: &[String]) -> bool {
    cases
        .iter()
        .filter(|case| case.check.is_some())
        .all(|case| {
            std::iter::once(&case.episode_ref)
                .chain(case.evidence.iter())
                .all(|reference| evidence.contains(reference))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationSplit {
    Train,
    Validation,
    Test,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputCheck {
    Exact { expected: String },
    Json { expected: Value },
    Judge { expected: String, rubric: String },
}

/// An inconclusive assessment cannot enter candidate ranking as a zero score.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeResult {
    pub verdict: JudgeVerdict,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeVerdict {
    Pass,
    Fail,
    Inconclusive,
}

impl JudgeResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.reason.trim().is_empty() || self.reason.len() > 4000 {
            return Err("Judge requires a bounded assessment rationale".into());
        }
        Ok(())
    }

    pub fn score(&self) -> Option<f64> {
        match self.verdict {
            JudgeVerdict::Pass => Some(1.0),
            JudgeVerdict::Fail => Some(0.0),
            JudgeVerdict::Inconclusive => None,
        }
    }
}

impl EvaluationSuite {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > 120
            || !(3..=64).contains(&self.cases.len())
        {
            return Err("An evaluation requires a name and 3 to 64 cases".into());
        }
        let mut ids = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        for case in &self.cases {
            case.check.validate()?;
            if let Some(environment) = &case.environment {
                environment.validate()?;
            }
            if let Some(source) = &case.source {
                source.validate()?;
                if (source.input != case.input
                    && !(case.split == EvaluationSplit::Train
                        && source.variant.as_ref() == Some(&case.input)))
                    || serde_json::to_value(&source.check).map_err(|e| e.to_string())?
                        != serde_json::to_value(&case.check).map_err(|e| e.to_string())?
                {
                    return Err("Trace provenance must match the evaluated input and answer".into());
                }
            }
            if case.id.is_empty()
                || case.id.len() > 80
                || !ids.insert(&case.id)
                || case.input.trim().is_empty()
                || case.input.len() > 16_000
                || !inputs.insert(case.input.trim())
                || serde_json::to_string(&case.check)
                    .map_err(|e| e.to_string())?
                    .len()
                    > 16_000
            {
                return Err("Evaluation cases require distinct identifiers and inputs within the size limits".into());
            }
        }
        for split in [
            EvaluationSplit::Train,
            EvaluationSplit::Validation,
            EvaluationSplit::Test,
        ] {
            if !self.cases.iter().any(|case| case.split == split) {
                return Err("Provide separate training, validation and test cases".into());
            }
        }
        Ok(())
    }
}

impl OutputCheck {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Judge { expected, rubric } = self
            && (expected.trim().is_empty()
                || expected.len() > 8000
                || rubric.trim().is_empty()
                || rubric.len() > 4000)
        {
            return Err("Judge requires a reference result and bounded acceptance criteria".into());
        }
        Ok(())
    }

    pub fn model_calls(&self) -> usize {
        usize::from(matches!(self, Self::Judge { .. }))
    }

    /// None requires an independent judge; callers must not assume a failure.
    pub fn score(&self, output: &str) -> Option<f64> {
        let passed = match self {
            Self::Exact { expected } => output.trim() == expected.trim(),
            Self::Json { expected } => {
                serde_json::from_str::<Value>(output.trim()).is_ok_and(|value| value == *expected)
            }
            Self::Judge { .. } => return None,
        };
        Some(if passed { 1.0 } else { 0.0 })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationCandidate {
    pub revision_id: Option<String>,
    pub instruction: String,
    pub team: Option<crate::team::Team>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replay_requires_an_immutable_bounded_starting_state() {
        let valid = ReplayEnvironment {
            image: format!("sha256:{}", "a".repeat(64)),
            files: [("src/main.py".into(), "print(1)".into())].into(),
            verification: vec!["test -f src/main.py".into()],
            max_steps: 3,
        };
        assert!(valid.validate().is_ok());
        for path in [
            "/etc/passwd",
            "../outside",
            "a/../../outside",
            "a\\outside",
            "a//b",
            "C:/outside",
        ] {
            let mut invalid = valid.clone();
            invalid.files = [(path.into(), "x".into())].into();
            assert!(invalid.validate().is_err(), "{path}");
        }
        let mut invalid = valid.clone();
        invalid.image = "alpine:latest".into();
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.verification.clear();
        assert!(invalid.validate().is_err());
        invalid = valid;
        invalid.max_steps = 9;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn open_ended_checks_require_a_judge_and_uncertainty_has_no_score() {
        let mut check = OutputCheck::Judge {
            expected: "A supported conclusion".into(),
            rubric: "Cite the supplied evidence".into(),
        };
        assert!(check.validate().is_ok());
        assert_eq!(check.score("A supported conclusion"), None);
        assert_eq!(check.model_calls(), 1);
        let result = JudgeResult {
            verdict: JudgeVerdict::Inconclusive,
            reason: "The input omits the necessary measurements".into(),
        };
        assert!(result.validate().is_ok());
        assert_eq!(result.score(), None);
        if let OutputCheck::Judge { rubric, .. } = &mut check {
            rubric.clear();
        }
        assert!(check.validate().is_err());
        assert!(
            serde_json::from_value::<JudgeResult>(
                json!({"verdict":"pass","reason":"ok","score":1})
            )
            .is_err()
        );
    }

    #[test]
    fn case_review_requires_evidence_for_every_answer() {
        let cases: Vec<TraceCase> = (0..2)
            .map(|i| TraceCase {
                variant: None,
                group_ref: None,
                classification: None,
                episode_ref: format!("task:{i}"),
                evidence: vec![format!("answer:{i}")],
                input: format!("Task {i}"),
                check: Some(OutputCheck::Exact {
                    expected: "ok".into(),
                }),
                reason: "User accepted".into(),
            })
            .collect();
        assert!(!case_review_covers(
            &cases,
            &["task:0".into(), "answer:0".into()]
        ));
        assert!(case_review_covers(
            &cases,
            &[
                "task:0".into(),
                "answer:0".into(),
                "task:1".into(),
                "answer:1".into()
            ]
        ));
    }

    #[test]
    fn checks_grade_outputs_not_claims_of_success() {
        let check = OutputCheck::Json {
            expected: json!({"answer":42,"proof":true}),
        };
        assert_eq!(check.score(r#"{"proof":true,"answer":42}"#), Some(1.0));
        assert_eq!(check.score(r#"{"answer":42}"#), Some(0.0));
        assert_eq!(check.score("I verified that the answer is 42."), Some(0.0));
        assert_eq!(
            OutputCheck::Exact {
                expected: "42".into()
            }
            .score(" 42\n"),
            Some(1.0)
        );
    }

    #[test]
    fn suite_rejects_overlap_and_missing_holdout() {
        let mut suite = EvaluationSuite {
            name: "Arithmetic".into(),
            cases: [
                EvaluationSplit::Train,
                EvaluationSplit::Validation,
                EvaluationSplit::Test,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, split)| EvaluationCase {
                environment: None,
                source: None,
                id: i.to_string(),
                split,
                input: format!("Compute {i}+1"),
                check: OutputCheck::Exact {
                    expected: (i + 1).to_string(),
                },
            })
            .collect(),
        };
        assert!(suite.validate().is_ok());
        suite.cases[2].input = suite.cases[0].input.clone();
        assert!(suite.validate().is_err());
        suite.cases.pop();
        assert!(suite.validate().is_err());
    }
}
