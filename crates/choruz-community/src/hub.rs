//! Public, revision-pinned HF dataset exchange. Never send source queries or traces.
use crate::behavior::{BehaviorRecord, COMMUNITY_REPOSITORY, valid_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use choruz_common::AppError;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

/// A fully validated immutable revision. Persist records and object IDs together
/// only after success; a failed fetch must not replace a caller's previous cache.
pub struct Snapshot {
    pub revision: String,
    pub records: Vec<BehaviorRecord>,
    pub objects: BTreeMap<String, String>,
}

/// Exchange with the fixed public community dataset. This client owns neither
/// privacy approval nor durable retry state; it never uploads source traces.
pub struct Hub {
    client: reqwest::Client,
    origin: String,
}

impl Hub {
    /// Build a bounded HTTPS client. Network failures expose no token or response body.
    pub fn new() -> Result<Self, AppError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 4 {
                        attempt.error("Too many community redirects")
                    } else if attempt.url().scheme() == "https"
                        && attempt.url().host_str() == Some("huggingface.co")
                    {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .build()
                .map_err(network)?,
            origin: "https://huggingface.co".into(),
        })
    }

    /// Fetch one pinned revision, reusing records whose object IDs match the cache.
    /// `None` means the supplied revision is current. Invalid identities, schema,
    /// pagination or size limits fail the whole fetch rather than returning a partial cache.
    pub async fn snapshot(
        &self,
        previous: Option<&str>,
        cached: &BTreeMap<String, (String, BehaviorRecord)>,
    ) -> Result<Option<Snapshot>, AppError> {
        let info = read_json(
            self.client
                .get(format!(
                    "{}/api/datasets/{COMMUNITY_REPOSITORY}?expand[]=sha",
                    self.origin
                ))
                .send()
                .await
                .map_err(network)?,
            64 * 1024,
        )
        .await?;
        let revision = info["sha"]
            .as_str()
            .filter(|sha| sha.len() == 40 && sha.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| {
                AppError::Validation("Community did not supply an immutable revision".into())
            })?;
        if previous == Some(revision) {
            return Ok(None);
        }
        let tree_path = format!("/api/datasets/{COMMUNITY_REPOSITORY}/tree/{revision}");
        let mut next = Some(format!(
            "{}{tree_path}?recursive=true&limit=100",
            self.origin
        ));
        let mut pages = BTreeSet::new();
        let mut records = Vec::new();
        let mut ids = BTreeSet::new();
        let mut objects = BTreeMap::new();
        let mut bytes = 0;
        while let Some(url) = next.take() {
            if !pages.insert(url.clone()) || pages.len() > 1000 {
                return Err(AppError::Validation(
                    "Community pagination repeated or exceeded the synchronization budget".into(),
                ));
            }
            let response = self.client.get(&url).send().await.map_err(network)?;
            next = response
                .headers()
                .get("link")
                .and_then(|v| v.to_str().ok())
                .and_then(|header| {
                    header
                        .split(',')
                        .find(|part| part.contains("rel=\"next\""))
                        .and_then(|part| part.trim().strip_prefix('<'))
                        .and_then(|part| part.split('>').next())
                        .map(str::to_owned)
                });
            if let Some(next) = &next {
                let parsed = reqwest::Url::parse(next)
                    .map_err(|_| AppError::Validation("Invalid community pagination URL".into()))?;
                let origin = reqwest::Url::parse(&self.origin)
                    .map_err(|_| AppError::Internal("Invalid community origin".into()))?;
                if parsed.origin() != origin.origin()
                    || parsed.path() != tree_path
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err(AppError::Validation(
                        "Community pagination left the pinned dataset".into(),
                    ));
                }
            }
            let page = read_json(response, 2 * 1024 * 1024).await?;
            let files = page
                .as_array()
                .ok_or_else(|| AppError::Validation("Invalid community file listing".into()))?;
            for file in files {
                if file["type"] != "file" {
                    continue;
                }
                let Some(path) = file["path"].as_str() else {
                    continue;
                };
                let Some(id) = path
                    .strip_prefix("records/")
                    .and_then(|p| p.strip_suffix(".json"))
                    .filter(|id| valid_id(id))
                else {
                    continue;
                };
                if !ids.insert(id.to_owned()) {
                    return Err(AppError::Validation(
                        "Duplicate community record identity".into(),
                    ));
                }
                let oid = file["oid"]
                    .as_str()
                    .filter(|oid| oid.len() == 40 && oid.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(|| {
                        AppError::Validation("Community file lacks an immutable object id".into())
                    })?;
                objects.insert(id.to_owned(), oid.to_owned());
                let record = if let Some((_, record)) = cached.get(id).filter(|(old, _)| old == oid)
                {
                    record.clone()
                } else {
                    let payload = read_json(
                        self.client
                            .get(format!(
                                "{}/datasets/{COMMUNITY_REPOSITORY}/resolve/{revision}/{path}",
                                self.origin
                            ))
                            .send()
                            .await
                            .map_err(network)?,
                        64 * 1024,
                    )
                    .await?;
                    serde_json::from_value::<BehaviorRecord>(payload).map_err(|_| {
                        AppError::Validation("Invalid community exchange schema".into())
                    })?
                };
                record.validate().map_err(AppError::Validation)?;
                if record.id != id {
                    return Err(AppError::Validation(
                        "Community file identity does not match its record".into(),
                    ));
                }
                bytes += serde_json::to_vec(&record)
                    .map_err(|_| AppError::Internal("Encode cached behavior".into()))?
                    .len();
                if bytes > 64 * 1024 * 1024 {
                    return Err(AppError::Validation(
                        "Community snapshot exceeds the 64 MiB cache budget".into(),
                    ));
                }
                records.push(record);
            }
        }
        Ok(Some(Snapshot {
            revision: revision.to_owned(),
            records,
            objects,
        }))
    }

    /// Submit an already privacy-reviewed record as a dataset PR, not an accepted record.
    /// Schema validation does not establish privacy or consent. A network error after
    /// dispatch has an uncertain outcome; callers must not blindly retry publication.
    pub async fn contribute(
        &self,
        token: &str,
        record: &BehaviorRecord,
    ) -> Result<String, AppError> {
        record.validate().map_err(AppError::Validation)?;
        let content = serde_json::to_vec(record)
            .map_err(|_| AppError::Internal("Encode public behavior record".into()))?;
        let body = format!(
            "{}\n{}\n",
            json!({"key":"header","value":{"summary":format!("Add behavior evidence {}",record.id),"description":"An independently privacy-reviewed experience record. Contributor claims require community review."}}),
            json!({"key":"file","value":{"path":format!("records/{}.json",record.id),"encoding":"base64","content":STANDARD.encode(content)}})
        );
        let response = read_json(
            self.client
                .post(format!(
                    "{}/api/datasets/{COMMUNITY_REPOSITORY}/commit/main?create_pr=1",
                    self.origin
                ))
                .bearer_auth(token)
                .header("Content-Type", "application/x-ndjson")
                .body(body)
                .send()
                .await
                .map_err(network)?,
            64 * 1024,
        )
        .await?;
        let prefix = format!("https://huggingface.co/datasets/{COMMUNITY_REPOSITORY}/discussions/");
        response["pullRequestUrl"]
            .as_str()
            .filter(|url| {
                response["success"] == true
                    && url
                        .strip_prefix(&prefix)
                        .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()))
            })
            .map(str::to_owned)
            .ok_or_else(|| {
                AppError::Validation("Community did not confirm a contribution PR".into())
            })
    }
}

async fn read_json(mut response: reqwest::Response, limit: usize) -> Result<Value, AppError> {
    if !response.status().is_success() {
        return Err(AppError::Validation(format!(
            "Community HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if bytes.len() + chunk.len() > limit {
            return Err(AppError::Validation(
                "Community response exceeds its size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Validation("Community returned invalid JSON".into()))
}

fn network(_: reqwest::Error) -> AppError {
    AppError::Internal("Community network request failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, response::IntoResponse};
    use std::sync::{Arc, Mutex};

    struct Fixture {
        origin: String,
        record: Value,
        requests: Mutex<Vec<(String, String)>>,
    }

    fn record() -> Value {
        json!({"schema_version":1,"id":"event-a","occurrence_id":"objective-a",
            "problem":{"id":"problem-a","title":"Missing check","input_background":"A task required verification.","expected_behavior":"Run the check.","bad_behavior":"Skipped the check.","applicability":"Explicit acceptance checks","tags":["verification"]},
            "model":{"observed":"model-actual","configured":null,"harness":"codex_terminal","harness_version":"1.0"},
            "solution":null,"kind":"encountered","evidence_summary":"The user corrected the unsupported completion claim."})
    }

    async fn serve(
        State(state): State<Arc<Fixture>>,
        request: axum::extract::Request,
    ) -> axum::response::Response {
        let uri = request.uri().to_string();
        let method = request.method().clone();
        let body = axum::body::to_bytes(request.into_body(), 128 * 1024)
            .await
            .unwrap();
        state
            .requests
            .lock()
            .unwrap()
            .push((uri.clone(), String::from_utf8(body.to_vec()).unwrap()));
        if method == axum::http::Method::POST {
            return axum::Json(json!({"success":true,"pullRequestUrl":format!("https://huggingface.co/datasets/{COMMUNITY_REPOSITORY}/discussions/3")})).into_response();
        }
        if uri.contains("/tree/") {
            if uri.contains("cursor=next") {
                return axum::Json(json!([])).into_response();
            }
            let mut response=axum::Json(json!([{"type":"file","path":"records/event-a.json","oid":"b".repeat(40)},{"type":"file","path":"README.md"}])).into_response();
            response.headers_mut().insert(
                "link",
                format!(
                    "<{}/api/datasets/{COMMUNITY_REPOSITORY}/tree/{}?cursor=next>; rel=\"next\"",
                    state.origin,
                    "a".repeat(40)
                )
                .parse()
                .unwrap(),
            );
            return response;
        }
        if uri.contains("/resolve/") {
            return axum::Json(state.record.clone()).into_response();
        }
        axum::Json(json!({"sha":"a".repeat(40)})).into_response()
    }

    #[tokio::test]
    async fn pinned_paginated_reads_and_pr_upload_use_only_exchange_records() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Fixture {
            origin: origin.clone(),
            record: record(),
            requests: Mutex::new(Vec::new()),
        });
        let router = axum::Router::new()
            .fallback(serve)
            .with_state(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let hub = Hub {
            client: reqwest::Client::new(),
            origin,
        };
        let Snapshot {
            revision,
            records,
            objects,
        } = hub.snapshot(None, &BTreeMap::new()).await.unwrap().unwrap();
        assert_eq!(revision, "a".repeat(40));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model.observed.as_deref(), Some("model-actual"));
        assert!(
            hub.snapshot(Some(&revision), &BTreeMap::new())
                .await
                .unwrap()
                .is_none()
        );
        let cached = [(
            records[0].id.clone(),
            (objects[&records[0].id].clone(), records[0].clone()),
        )]
        .into();
        let refreshed = hub
            .snapshot(Some(&"c".repeat(40)), &cached)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refreshed.records, records);
        assert_eq!(
            state
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(uri, _)| uri.contains("/resolve/"))
                .count(),
            1,
            "unchanged blobs must not be downloaded again"
        );
        let url = hub
            .contribute("synthetic-publisher-token", &records[0])
            .await
            .unwrap();
        assert!(url.ends_with("/discussions/3"));
        let requests = state.requests.lock().unwrap().clone();
        assert!(requests.iter().any(|(uri, _)| uri.contains("cursor=next")));
        let (_, body) = requests
            .iter()
            .find(|(uri, _)| uri.contains("create_pr=1"))
            .unwrap();
        let operation: Value = serde_json::from_str(body.lines().nth(1).unwrap()).unwrap();
        assert_eq!(operation["value"]["path"], "records/event-a.json");
        let uploaded: Value = serde_json::from_slice(
            &STANDARD
                .decode(operation["value"]["content"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(uploaded, record());
        assert!(
            requests
                .iter()
                .all(|(_, body)| !body.contains("synthetic-publisher-token"))
        );
        server.abort();
        let _ = server.await;
    }
}
