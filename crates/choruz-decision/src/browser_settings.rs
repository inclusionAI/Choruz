use crate::{Error, Result, bounded};
use serde::{Deserialize, Serialize};

/// Standing permission for this binding, not a request to replay past tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSettings {
    pub browser: String,
    pub allowed_urls: Vec<String>,
    pub scope: String,
}

impl BrowserSettings {
    pub fn validate(&self) -> Result<()> {
        if !bounded(&self.browser, 128)
            || !bounded(&self.scope, 2000)
            || self.allowed_urls.is_empty()
            || self.allowed_urls.len() > 16
            || self.allowed_urls.iter().any(|value| {
                !bounded(value, 4000)
                    || reqwest::Url::parse(value).map_or(true, |url| {
                        !matches!(url.scheme(), "http" | "https")
                            || url.host_str().is_none()
                            || !url.username().is_empty()
                            || url.password().is_some()
                    })
            })
        {
            return Err(Error::Invalid("Choose a browser, permitted task scope and 1 to 16 exact HTTP(S) page URLs without credentials".into()));
        }
        Ok(())
    }
}
