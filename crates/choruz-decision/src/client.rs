use crate::{Error, Provider, Request, Response, Result};
use std::time::Duration;

/// No implicit retries: the owner budgets calls and decides whether a failed
/// inference can be repeated. Neither error bodies nor credentials are exposed.
pub struct Client {
    http: reqwest::Client,
    key: String,
    endpoint: String,
}

impl Client {
    pub fn new(key: String) -> Result<Self> {
        Self::with_endpoint(key, "https://api.typesafe.ai/v1/systemone".into())
    }

    fn with_endpoint(key: String, endpoint: String) -> Result<Self> {
        if key.trim().is_empty() || key.contains(['\r', '\n']) {
            return Err(Error::Invalid(
                "provider key is missing or malformed".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Transport)?;
        Ok(Self {
            http,
            key,
            endpoint,
        })
    }
}

impl Provider for Client {
    async fn decide(&self, request: Request) -> Result<Response> {
        request.validate()?;
        let mut response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.key)
            .json(&request)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            if bytes.len() + chunk.len() > 512 * 1024 {
                return Err(Error::Invalid("provider response exceeds its limit".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: Response = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Invalid("provider response is not a decision result".into()))?;
        result.validate(&request)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};

    #[tokio::test]
    async fn official_wire_contract_and_untrusted_responses() {
        let app = Router::new().route(
            "/",
            post(
                |headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer fixture-key");
                    assert_eq!(body["questions"]["route"]["type"], "choice");
                    assert_eq!(
                        body["questions"]["route"]["instructions"]["question"],
                        "Pick route"
                    );
                    assert_eq!(
                        body["questions"]["route"]["criteria"]["run"],
                        json!({"when":"Supported"})
                    );
                    assert!(body["questions"]["route"]["criteria"]["abstain"].is_null());
                    Json(
                        json!({"model":"jev-fixture", "usage":{"input_tokens":10,"output_tokens":3},
                "answers":{"route":{"type":"choice","choice":body["state"],"confidence":0.9,
                    "probabilities":{"run":0.95,"abstain":0.05}}}}),
                    )
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client =
            Client::with_endpoint("fixture-key".into(), format!("http://{address}/")).unwrap();
        let mut request: Request = serde_json::from_value(json!({"model":"jev-fixture","state":"run",
            "questions":{"route":{"type":"choice","instructions":{"question":"Pick route"},"criteria":{"run":{"when":"Supported"},"abstain":null}}}})).unwrap();
        let result = client.decide(request.clone()).await;
        request.state = json!("invented-action");
        let invalid = client.decide(request).await;
        server.abort();
        let _ = server.await;
        assert_eq!(result.unwrap().choice("route", 0.8), Some("run"));
        assert!(matches!(invalid, Err(Error::Invalid(_))));
    }
}
