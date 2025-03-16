use axum::{
    Json, Router,
    body::Bytes,
    extract::{FromRequestParts, MatchedPath},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header::USER_AGENT, request::Parts},
    response::{Html, Response},
    routing::{get, post},
};
use serde::Deserialize;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::Span;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Deserialize)]
struct Release {
    // assets: Vec<Asset>,
    // assets_url: String,
    // author: Option<Author>,
    body: Option<String>,
    created_at: Option<String>,
    draft: bool,
    html_url: String,
    id: u64,
    name: Option<String>,
    node_id: String,
    prerelease: bool,
    published_at: Option<String>,
    tag_name: String,
    tarball_url: Option<String>,
    target_commitish: String,
    upload_url: String,
    url: String,
    zipball_url: Option<String>,
}
#[derive(Deserialize)]
struct Repository {}
#[derive(Deserialize)]
struct Sender {}

#[derive(Deserialize)]
struct WebhookPayload {
    action: String,
    release: Release,
    repository: Repository,
    sender: Sender,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!(
                    "{}=debug,tower_http=debug,axum::rejection=trace",
                    env!("CARGO_CRATE_NAME")
                )
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app = Router::new()
        .route("/", get(handler))
        .route("/", post(webhook))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let matched_path = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);

                    tracing::debug_span!(
                        "request",
                        method = %request.method(),
                        uri = %request.uri(),
                        matched_path,
                        foo=tracing::field::Empty,
                    )
                })
                .on_request(|_request: &Request<_>, _span: &Span| {
                    tracing::debug!("kicking off");
                    _span.record("foo", "bar");
                    tracing::debug!("request handled");
                })
                .on_response(|_response: &Response, _latency: Duration, _span: &Span| {})
                .on_body_chunk(|_chunk: &Bytes, _latency: Duration, _span: &Span| {})
                .on_eos(
                    |_trailers: Option<&HeaderMap>, _stream_duration: Duration, _span: &Span| {},
                )
                .on_failure(
                    |_error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {},
                ),
        );

    let listener = TcpListener::bind("127.0.0.1:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

struct ExtractUserAgent(HeaderValue);

impl<S> FromRequestParts<S> for ExtractUserAgent
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        parts
            .headers
            .get(USER_AGENT)
            .filter(|&ua| {
                ua.to_str()
                    .is_ok_and(|ua| ua.starts_with("GitHub-Hookshot/"))
            })
            .map(|ua| ExtractUserAgent(ua.clone()))
            .ok_or((StatusCode::BAD_REQUEST, "Go away"))
    }
}

async fn handler() -> Html<&'static str> {
    tracing::debug!("response generated");
    Html("<h1>Hello, World!</h1>")
}

async fn webhook(
    ExtractUserAgent(user_agent): ExtractUserAgent,
    headers: HeaderMap,
    Json(payload): Json<WebhookPayload>,
) -> StatusCode {
    tracing::debug!("Got the payload {:?}", user_agent);
    StatusCode::ACCEPTED
}
