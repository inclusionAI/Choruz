use crate::{Error, Result, bounded, probability};
use serde::{Deserialize, Serialize};

/// Explicit permission to transmit the opted-in binding's bounded evidence to
/// TypeSafe. A null setting disables both background and automatic decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningSettings {
    pub model: String,
    pub minimum_confidence: f64,
    pub classify: bool,
    pub supervise: bool,
    #[serde(default)]
    pub assist_turns: bool,
    pub builder_binding_id: Option<String>,
}

impl LearningSettings {
    pub fn validate(&self) -> Result<()> {
        if !bounded(&self.model, 128)
            || !probability(self.minimum_confidence)
            || !(self.classify
                || self.supervise
                || self.assist_turns
                || self.builder_binding_id.is_some())
            || self
                .builder_binding_id
                .as_ref()
                .is_some_and(|id| !bounded(id, 128))
        {
            return Err(Error::Invalid("select at least one capability, an explicit model and a bounded confidence threshold".into()));
        }
        Ok(())
    }
}
