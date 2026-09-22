use crate::{Error, Provider, Result, bounded, probability, programs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionTask {
    pub model: String,
    pub minimum_confidence: f64,
    pub job: Job,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Job {
    Classify {
        state: Value,
    },
    Supervise {
        state: Value,
    },
    Browser {
        goal: String,
        observation: programs::Observation,
    },
    Program {
        program: programs::Program,
        state: Value,
    },
}

impl DecisionTask {
    pub fn kind(&self) -> &'static str {
        match self.job {
            Job::Classify { .. } => "classify",
            Job::Supervise { .. } => "supervise",
            Job::Browser { .. } => "browser",
            Job::Program { .. } => "program",
        }
    }

    /// Uses one provider call. A caller must establish permission to transmit
    /// the supplied state before invoking this method.
    pub async fn run(self, provider: &impl Provider) -> Result<Value> {
        if !bounded(&self.model, 128) || !probability(self.minimum_confidence) {
            return Err(Error::Invalid(
                "model or confidence threshold is invalid".into(),
            ));
        }
        let result = match self.job {
            Job::Classify { state } => serde_json::to_value(
                ask(provider, programs::classification(self.model, state)).await?,
            ),
            Job::Supervise { state } => {
                serde_json::to_value(ask(provider, programs::supervision(self.model, state)).await?)
            }
            Job::Browser { goal, observation } => serde_json::to_value(
                programs::browser_action(
                    provider,
                    &self.model,
                    &goal,
                    observation,
                    self.minimum_confidence,
                )
                .await?,
            ),
            Job::Program { mut program, state } => {
                // A generated program cannot weaken the operator's threshold.
                program.minimum_confidence =
                    program.minimum_confidence.max(self.minimum_confidence);
                serde_json::to_value(program.run(provider, &self.model, state).await?)
            }
        };
        result.map_err(|_| Error::Invalid("decision result is not serializable".into()))
    }
}

async fn ask(provider: &impl Provider, request: crate::Request) -> Result<crate::Response> {
    request.validate()?;
    let response = provider.decide(request.clone()).await?;
    response.validate(&request)?;
    Ok(response)
}
