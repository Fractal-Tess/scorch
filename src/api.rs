use std::{env, path::Path, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderName, StatusCode},
    routing::get,
};
use serde::Serialize;
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::config::Config;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Clone)]
pub struct AppState {
    config: Arc<Config>,
}

impl AppState {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

pub fn router(state: AppState) -> Router {
    let timeout = Duration::from_secs(state.config.request_timeout_secs);

    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .fallback(not_found)
        .with_state(state)
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(CatchPanicLayer::new())
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadinessResponse {
    status: &'static str,
    browser_path: String,
    max_concurrency: usize,
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let browser_available = executable_exists(&state.config.browser_path);
    let status = if browser_available {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(ReadinessResponse {
            status: if browser_available {
                "ready"
            } else {
                "browser unavailable"
            },
            browser_path: state.config.browser_path.display().to_string(),
            max_concurrency: state.config.max_concurrency,
        }),
    )
}

fn executable_exists(path: &Path) -> bool {
    if path.components().count() > 1 {
        return path.is_file();
    }

    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| directory.join(path).is_file())
    })
}

#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
}

async fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: "not found" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reports_ok() {
        let Json(response) = health().await;
        assert_eq!(response.status, "ok");
    }
}
