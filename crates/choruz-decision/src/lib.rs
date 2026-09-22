//! Structured decisions are evidence, never authority to execute an action.
//! Callers own consent, task scope, observation freshness and outcome verification.

pub mod browser_settings;
pub mod client;
pub mod programs;
pub mod settings;
pub mod task;
pub mod workflow;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid decision: {0}")]
    Invalid(String),
    #[error("Decision provider request failed")]
    Transport,
    #[error("Decision provider returned HTTP {0}")]
    Http(u16),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Question {
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
    Noul {
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<BTreeMap<String, Value>>,
    },
}

impl Question {
    pub fn validate(&self) -> Result<()> {
        let (instructions, valid) = match self {
            Self::Choice {
                instructions,
                criteria,
            } => (
                instructions,
                (2..=255).contains(&criteria.len())
                    && criteria
                        .iter()
                        .all(|(key, description)| bounded(key, 128) && rubric(description)),
            ),
            Self::Score {
                instructions,
                criteria,
            } => (
                instructions,
                (2..=10).contains(&criteria.len())
                    && criteria.iter().all(|level| content(level, 4000)),
            ),
            Self::Noul {
                instructions,
                criteria,
            } => (
                instructions,
                criteria.as_ref().is_none_or(|criteria| {
                    criteria.iter().all(|(key, value)| {
                        matches!(key.as_str(), "true" | "false") && rubric(value)
                    })
                }),
            ),
        };
        if !valid || !content(instructions, 8000) {
            return Err(Error::Invalid(
                "question instructions or criteria exceed their bounds".into(),
            ));
        }
        Ok(())
    }
}

// Preserve structured rubrics on the wire; JSON-encoding them into strings
// changes the provider's input. Limits apply to encoded bytes for containers.
fn content(value: &Value, limit: usize) -> bool {
    match value {
        Value::String(text) => bounded(text, limit),
        Value::Object(fields) => {
            !fields.is_empty() && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= limit)
        }
        Value::Array(items) => {
            !items.is_empty() && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= limit)
        }
        _ => false,
    }
}

fn rubric(value: &Value) -> bool {
    value.is_null() || value.as_str() == Some("") || content(value, 4000)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub state: Value,
    pub model: String,
    pub questions: BTreeMap<String, Question>,
}

impl Request {
    pub fn validate(&self) -> Result<()> {
        if !bounded(&self.model, 128)
            || !(1..=64).contains(&self.questions.len())
            || !matches!(
                self.state,
                Value::String(_) | Value::Object(_) | Value::Array(_)
            )
            || self.questions.keys().any(|id| !bounded(id, 128))
            || serde_json::to_vec(self).map_or(true, |body| body.len() > 256 * 1024)
        {
            return Err(Error::Invalid(
                "request requires bounded state, model and questions".into(),
            ));
        }
        for question in self.questions.values() {
            question.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Noul {
        noul: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

impl Response {
    /// Reject missing heads, invented options and malformed distributions before
    /// the result reaches routing code. Confidence does not establish correctness.
    pub fn validate(&self, request: &Request) -> Result<()> {
        request.validate()?;
        if !bounded(&self.model, 128) || self.answers.keys().ne(request.questions.keys()) {
            return Err(Error::Invalid(
                "provider returned a different question set or no model".into(),
            ));
        }
        for (id, question) in &request.questions {
            let valid = match (question, &self.answers[id]) {
                (
                    Question::Choice { criteria, .. },
                    Answer::Choice {
                        choice,
                        probabilities,
                        confidence,
                    },
                ) => {
                    criteria.contains_key(choice)
                        && criteria.keys().eq(probabilities.keys())
                        && distribution(probabilities)
                        && probability(*confidence)
                        && probabilities
                            .values()
                            .all(|p| *p <= probabilities[choice] + 1e-6)
                }
                (
                    Question::Score { criteria, .. },
                    Answer::Score {
                        score,
                        probabilities,
                        confidence,
                    },
                ) => {
                    probabilities.len() == criteria.len()
                        && (0..criteria.len()).all(|i| probabilities.contains_key(&i.to_string()))
                        && distribution(probabilities)
                        && probability(*confidence)
                        && score.is_finite()
                        && *score >= 0.0
                        && *score <= (criteria.len() - 1) as f64
                        && (*score
                            - probabilities
                                .iter()
                                .map(|(i, p)| i.parse::<usize>().unwrap_or(0) as f64 * p)
                                .sum::<f64>())
                        .abs()
                            <= 0.02
                }
                (Question::Noul { .. }, Answer::Noul { noul }) => probability(*noul),
                _ => false,
            };
            if !valid {
                return Err(Error::Invalid(
                    "provider answer does not match its question".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn choice(&self, id: &str, minimum_confidence: f64) -> Option<&str> {
        if !probability(minimum_confidence) {
            return None;
        }
        match self.answers.get(id)? {
            Answer::Choice {
                choice, confidence, ..
            } if *confidence >= minimum_confidence => Some(choice),
            _ => None,
        }
    }
}

pub trait Provider: Sync {
    fn decide(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = Result<Response>> + Send;
}

pub(crate) fn bounded(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}

pub(crate) fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn distribution(values: &BTreeMap<String, f64>) -> bool {
    values.values().all(|p| probability(*p)) && (values.values().sum::<f64>() - 1.0).abs() <= 0.001
}
