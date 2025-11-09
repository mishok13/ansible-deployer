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

#[derive(Deserialize, Debug)]
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
#[derive(Deserialize, Debug)]
struct Repository {}
#[derive(Deserialize, Debug)]
struct Sender {}

#[derive(Deserialize, Debug)]
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

    // Needs to fire off a request to populate allowlisted IP addresses. Also the list probably needs to be
    // updated on occassion (how often? hourly? daily?)

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
    tracing::debug!("Got the UA {:?}", user_agent);
    tracing::debug!("Got the headers {:?}", headers);
    tracing::debug!("Got the payload {:?}", payload);
    StatusCode::ACCEPTED
}

fn validate_signature(secret: &[u8], payload: &[u8], signature: &[u8]) -> bool {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
    println!("{:x?}", ring::hmac::sign(&key, payload).as_ref());
    ring::hmac::verify(&key, payload, signature)
        .map_err(|e| println!("WTFFFFFF {:?}", e))
        .is_ok()
}

fn hex_to_u8(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|index| {
            s.get(index..index + 2)
                .map(|pair| u8::from_str_radix(pair, 16).expect("Invalid hex values"))
                .expect("Invalid hex string length")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_signature() {
        // Test values as provided by GitHub here
        // https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries#validating-webhook-deliveries
        let secret = b"It's a Secret to Everybody";
        let payload = b"Hello, World!";
        let signature =
            hex_to_u8("757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17");

        assert!(validate_signature(secret, payload, &signature));
    }
}
