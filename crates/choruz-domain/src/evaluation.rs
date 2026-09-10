//! Fixed, tool-free evaluation tasks. Expected answers never enter the task prompt.
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
    pub id: String,
    pub split: EvaluationSplit,
    pub input: String,
    pub check: OutputCheck,
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
    pub fn score(&self, output: &str) -> f64 {
        let passed = match self {
            Self::Exact { expected } => output.trim() == expected.trim(),
            Self::Json { expected } => {
                serde_json::from_str::<Value>(output.trim()).is_ok_and(|value| value == *expected)
            }
        };
        if passed { 1.0 } else { 0.0 }
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
    fn checks_grade_outputs_not_claims_of_success() {
        let check = OutputCheck::Json {
            expected: json!({"answer":42,"proof":true}),
        };
        assert_eq!(check.score(r#"{"proof":true,"answer":42}"#), 1.0);
        assert_eq!(check.score(r#"{"answer":42}"#), 0.0);
        assert_eq!(check.score("I verified that the answer is 42."), 0.0);
        assert_eq!(
            OutputCheck::Exact {
                expected: "42".into()
            }
            .score(" 42\n"),
            1.0
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
