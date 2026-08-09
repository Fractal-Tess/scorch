use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State, rejection::JsonRejection},
    http::{HeaderName, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use scorch_engine::{EngineError, ScorchEngine};
use scorch_types::{
    CrawlJobSummary, CrawlRequest, CrawlStatusRequest, DeleteResponse, ErrorResponse, MapRequest,
    ReadinessResponse, ScrapeRequest, SearchRequest,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tower_http::{
    catch_panic::CatchPanicLayer,
    classify::ServerErrorsFailureClass,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::{info, warn};
use uuid::Uuid;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Clone)]
struct AppState {
    engine: Arc<ScorchEngine>,
}

pub fn router(engine: Arc<ScorchEngine>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/v1/scrape", post(scrape))
        .route("/v1/search", post(search))
        .route("/v1/map", post(map))
        .route("/v1/crawls", post(start_crawl))
        .route("/v1/crawls/{id}", get(crawl_status).delete(delete_crawl))
        .fallback(not_found)
        .with_state(AppState { engine })
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<Body>| {
                    let request_id = request
                        .headers()
                        .get(&REQUEST_ID_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!(
                        "http_request",
                        request_id,
                        method = %request.method(),
                        path = request.uri().path()
                    )
                })
                .on_request(
                    |_request: &axum::http::Request<Body>, span: &tracing::Span| {
                        info!(parent: span, "request started");
                    },
                )
                .on_response(
                    |response: &Response, latency: Duration, span: &tracing::Span| {
                        info!(
                            parent: span,
                            status = %response.status(),
                            latency_ms = latency.as_millis() as u64,
                            "request completed"
                        );
                    },
                )
                .on_failure(
                    |failure: ServerErrorsFailureClass, latency: Duration, span: &tracing::Span| {
                        warn!(
                            parent: span,
                            %failure,
                            latency_ms = latency.as_millis() as u64,
                            "request failed"
                        );
                    },
                ),
        )
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(130),
        ))
        .layer(CatchPanicLayer::new())
}

pub async fn serve(
    bind: SocketAddr,
    engine: Arc<ScorchEngine>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;
    info!(%bind, "Scorch API is listening");
    axum::serve(listener, router(engine))
        .with_graceful_shutdown(shutdown)
        .await
        .context("API server failed")?;
    info!("Scorch API stopped");
    Ok(())
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let browser_available = state.engine.browser_available();
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
            }
            .into(),
            browser_available,
            browser_path: state.engine.config().browser_path.display().to_string(),
            max_concurrency: state.engine.config().max_concurrency,
            default_search_provider: state
                .engine
                .config()
                .default_search_provider
                .as_str()
                .into(),
        }),
    )
}

async fn scrape(
    State(state): State<AppState>,
    payload: Result<Json<ScrapeRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let request = json(payload)?;
    Ok(Json(state.engine.scrape(&request).await?))
}

async fn search(
    State(state): State<AppState>,
    payload: Result<Json<SearchRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let request = json(payload)?;
    Ok(Json(state.engine.search(&request).await?))
}

async fn map(
    State(state): State<AppState>,
    payload: Result<Json<MapRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let request = json(payload)?;
    Ok(Json(state.engine.map(&request).await?))
}

async fn start_crawl(
    State(state): State<AppState>,
    payload: Result<Json<CrawlRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let request = json(payload)?;
    let job = state.engine.start_crawl(request)?;
    Ok((StatusCode::ACCEPTED, Json(CrawlJobSummary::from(&job))))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CrawlStatusQuery {
    #[serde(default)]
    cursor: usize,
    #[serde(default = "default_page_size")]
    page_size: usize,
}

fn default_page_size() -> usize {
    10
}

async fn crawl_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<CrawlStatusQuery>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.engine.crawl_page(&CrawlStatusRequest {
        id,
        cursor: query.cursor,
        page_size: query.page_size,
    })?))
}

async fn delete_crawl(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let deleted = state.engine.delete_crawl(id);
    if !deleted {
        return Err(ApiError(EngineError::JobNotFound));
    }
    Ok(Json(DeleteResponse { id, deleted }))
}

fn json<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload
        .map(|Json(value)| value)
        .map_err(|error| ApiError(EngineError::InvalidRequest(error.body_text())))
}

async fn not_found() -> ApiError {
    ApiError(EngineError::NotFound("route".into()))
}

struct ApiError(EngineError);

impl From<EngineError> for ApiError {
    fn from(error: EngineError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            EngineError::InvalidRequest(_) | EngineError::UnsafeUrl(_) => StatusCode::BAD_REQUEST,
            EngineError::Timeout => StatusCode::GATEWAY_TIMEOUT,
            EngineError::ResponseTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            EngineError::UnsupportedContent(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            EngineError::Browser(_) => StatusCode::SERVICE_UNAVAILABLE,
            EngineError::Capacity(_) => StatusCode::TOO_MANY_REQUESTS,
            EngineError::NotFound(_) | EngineError::JobNotFound => StatusCode::NOT_FOUND,
            EngineError::Dns(_)
            | EngineError::Fetch(_)
            | EngineError::Extraction(_)
            | EngineError::Search(_) => StatusCode::BAD_GATEWAY,
        };
        let body = ErrorResponse {
            code: self.0.code().into(),
            message: self.0.to_string(),
            request_id: None,
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;
    use scorch_engine::EngineConfig;

    #[tokio::test]
    async fn health_reports_ok() {
        let engine = ScorchEngine::new(EngineConfig::default()).await.unwrap();
        let response = router(engine)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn request_ids_are_propagated() {
        let engine = ScorchEngine::new(EngineConfig::default()).await.unwrap();
        let response = router(engine)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .header(&REQUEST_ID_HEADER, "known-request-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(&REQUEST_ID_HEADER).unwrap(),
            "known-request-id"
        );
    }

    #[tokio::test]
    async fn malformed_json_is_a_json_error() {
        let engine = ScorchEngine::new(EngineConfig::default()).await.unwrap();
        let response = router(engine)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_request_fields_are_rejected() {
        let engine = ScorchEngine::new(EngineConfig::default()).await.unwrap();
        let response = router(engine)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/scrape")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"url":"https://example.com","unexpected":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_routes_return_not_found() {
        let engine = ScorchEngine::new(EngineConfig::default()).await.unwrap();
        let response = router(engine)
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
