use axum::{
    Json,
    http::{StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
};
use choruz_session::PgSessionStore;
use choruz_store::EventStore;

pub(crate) async fn liveness() -> Json<choruz_common::HostServiceStatus> {
    status("ok")
}

pub(crate) async fn readiness(
    event_store: EventStore,
    session_store: PgSessionStore,
) -> impl IntoResponse {
    let (events, sessions) = tokio::join!(event_store.health_check(), session_store.health_check());
    readiness_response(events.is_ok() && sessions.is_ok())
}

/// Encodes the process-wide registry in `crates/choruz-common/src/metrics.rs`.
pub(crate) async fn metrics() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, choruz_common::metrics::TEXT_CONTENT_TYPE)],
        choruz_common::metrics::text(),
    )
}

fn status(value: &'static str) -> Json<choruz_common::HostServiceStatus> {
    Json(choruz_common::HostServiceStatus::new(
        "choruz-pipeline",
        value,
    ))
}

fn readiness_response(
    database_ready: bool,
) -> (StatusCode, Json<choruz_common::HostServiceStatus>) {
    let code = if database_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let status = if database_ready { "ready" } else { "not_ready" };
    (
        code,
        Json(choruz_common::HostServiceStatus {
            database: Some(database_ready),
            ..choruz_common::HostServiceStatus::new("choruz-pipeline", status)
        }),
    )
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn liveness_identifies_a_compatible_pipeline() {
        let response = super::liveness().await;
        assert_eq!(response.0.service, "choruz-pipeline");
        assert_eq!(
            response.0.protocol_version,
            choruz_common::HOST_SERVICE_PROTOCOL_VERSION
        );
    }

    #[tokio::test]
    async fn metrics_serves_the_shared_registry_as_prometheus_text() {
        use axum::response::IntoResponse;

        let counter = choruz_common::metrics::register_counter(
            "choruz_pipeline_meta_test_total",
            "Pipeline metrics endpoint test counter.",
        );
        counter.inc();

        let response = super::metrics().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/plain; version=0.0.4")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("metrics body");
        let text = String::from_utf8(body.to_vec()).expect("utf-8 body");
        assert!(
            text.lines()
                .any(|line| line == "# TYPE choruz_pipeline_meta_test_total counter")
        );
        assert!(
            text.lines()
                .any(|line| line == "choruz_pipeline_meta_test_total 1")
        );
    }

    #[test]
    fn readiness_reports_healthy_dependencies() {
        let (code, response) = super::readiness_response(true);
        assert_eq!(code, axum::http::StatusCode::OK);
        assert_eq!(response.0.status, "ready");
        assert_eq!(response.0.database, Some(true));
    }

    #[test]
    fn readiness_rejects_unavailable_dependencies() {
        let (code, response) = super::readiness_response(false);
        assert_eq!(code, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.0.status, "not_ready");
        assert_eq!(response.0.database, Some(false));
    }
}
