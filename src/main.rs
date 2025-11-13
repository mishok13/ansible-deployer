use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, FromRequestParts, MatchedPath, State},
    http::{
        HeaderMap, HeaderValue, Request, StatusCode,
        header::{ToStrError, USER_AGENT},
        request::Parts,
    },
    response::{IntoResponse, Response},
    routing::post,
};
use clap::Parser;
use flate2::read::GzDecoder;
use ipnet::IpNet;
use serde::Deserialize;
use std::{
    io::{self},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tar::Archive;
use tokio::{
    net::TcpListener,
    spawn,
    sync::mpsc::{Sender, channel},
};
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::Span;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(about="Webhook for deploying", long_about=None)]
struct Cli {
    #[arg(short, long, env = "SECRET")]
    secret: String,
    #[arg(short = 'H', long, env = "HOST", default_value = "0.0.0.0")]
    host: String,
    #[arg(short, long, env = "PORT", default_value_t = 18823)]
    port: u16,
    #[arg(
        short,
        long,
        env = "META_URL",
        default_value = "https://api.github.com/meta"
    )]
    meta_url: String,
    #[arg(short, long, env = "GITHUB_TOKEN")]
    github_token: String,
}

#[derive(Deserialize, Debug)]
struct Release {
    tarball_url: String,
}
#[derive(Deserialize, Debug)]
struct Repository {
    full_name: String,
}
#[derive(Deserialize, Debug)]
struct GitHubSender {
    login: String,
}

#[derive(Deserialize, Debug)]
struct WebhookPayload {
    action: String,
    release: Release,
    repository: Repository,
    sender: GitHubSender,
}

#[derive(Deserialize, Debug)]
struct GitHubMeta {
    hooks: Vec<IpNet>,
}

#[derive(Debug)]
struct AppState {
    secret: Vec<u8>,
    permitted_ranges: Vec<IpNet>,
    sender: Sender<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
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

    let args = Cli::parse();

    let github_client = reqwest::Client::builder().user_agent("mishok13").build()?;
    let mut meta = github_client
        .get(args.meta_url)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(args.github_token)
        .send()
        .await?
        .error_for_status()?
        .json::<GitHubMeta>()
        .await?;
    meta.hooks.push("127.0.0.1/32".parse()?);

    let (tx, mut rx) = channel(10);

    spawn(async move {
        while let Some(tarball_url) = rx.recv().await {
            // TODO: Rewrite with proper chaining
            tracing::debug!("Processing tarball {}", tarball_url);
            match github_client.get(&tarball_url).send().await {
                Ok(response) => {
                    tracing::debug!("Got the tarball baby!!11");
                    if !response.status().is_success() {
                        tracing::warn!(
                            "Got an error from tarball URL {} {:?}",
                            response.status().as_str(),
                            response.text().await.unwrap()
                        )
                    } else {
                        match response.bytes().await {
                            Ok(bytes) => {
                                tracing::debug!("Got em bytes {}", bytes.len());
                                match Archive::new(GzDecoder::new(&bytes.to_vec()[..]))
                                    .unpack("/tmp/foobar/")
                                {
                                    Ok(()) => {
                                        tracing::debug!("Unpacked into /tmp/foobar")
                                        // time to run uv ansible and the rest
                                    }
                                    Err(err) => {
                                        tracing::warn!("Could not untar the thing {:?}", err)
                                    }
                                }
                            }
                            Err(err) => tracing::warn!(
                                "Failed to fetch tarball bytes {} {:?}",
                                tarball_url,
                                err
                            ),
                        }
                    }
                }
                Err(err) => tracing::warn!("Failed to download tarball {} {:?}", tarball_url, err),
            }
        }
    });

    let state = Arc::new(AppState {
        secret: args.secret.into_bytes(),
        permitted_ranges: meta.hooks,
        sender: tx,
    });

    let app = Router::new()
        .route("/", post(webhook))
        .with_state(state)
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
                .on_response(|_response: &Response, _latency: Duration, _span: &Span| {
                    tracing::debug!("responding");
                })
                .on_body_chunk(|_chunk: &Bytes, _latency: Duration, _span: &Span| {})
                .on_eos(
                    |_trailers: Option<&HeaderMap>, _stream_duration: Duration, _span: &Span| {},
                )
                .on_failure(
                    |_error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {},
                ),
        );

    let listener = TcpListener::bind(format!("0.0.0.0:{}", args.port))
        .await
        .unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    Ok(axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?)
}

struct ExtractUserAgent(HeaderValue);

impl<S> FromRequestParts<S> for ExtractUserAgent
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
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

#[derive(Debug)]
enum AppError {
    BadJson,
    ChecksumMismatch,
    IpOutOfRange,
    BadUser,
    Unhandled(anyhow::Error),
}

// impl From<serde_json::Error> for AppError {
//     fn from(_: serde_json::Error) -> Self {
//         Self::BadJson
//     }
// }

// impl From<ToStrError> for AppError {
//     fn from(_: ToStrError) -> Self {
//         Self::ChecksumMismatch
//     }
// }

// impl From<io::Error> for AppError {
//     fn from(_: io::Error) -> Self {
//         Self::BadJson
//     }
// }

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Unhandled(err.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::IpOutOfRange | Self::BadUser => {
                (StatusCode::FORBIDDEN, "Forbidden").into_response()
            }
            Self::Unhandled(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
        }
    }
}

struct ClientIpExtractor(IpAddr);

impl<S> FromRequestParts<S> for ClientIpExtractor
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        parts
            .headers
            .get("cf-connecting-ip") // Try CloudFlare first
            .or_else(|| parts.headers.get("x-forwaded-for")) // Then Caddy
            .and_then(|value| value.to_str().ok())
            .and_then(|s| s.parse().ok())
            // Welp I guess we're exposing ourselves to the whole interwebs OR running dev
            .or_else(|| {
                parts
                    .extensions
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|ConnectInfo(addr)| addr.ip())
            })
            .inspect(|v| tracing::debug!("inspecting {:?}", v))
            .map(Self)
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "foobar"))
    }
}

async fn webhook(
    State(state): State<Arc<AppState>>,
    ExtractUserAgent(user_agent): ExtractUserAgent,
    ClientIpExtractor(ip): ClientIpExtractor,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(), AppError> {
    let payload: WebhookPayload = serde_json::from_slice(&body)?;
    let signature = headers
        .get("x-hub-signature-256")
        .ok_or(AppError::ChecksumMismatch)?
        .to_str()?
        .strip_prefix("sha256=")
        .map(hex_to_u8)
        .ok_or(AppError::ChecksumMismatch)?;
    // Use anyhow::ensure! maybe?
    // also, store key instead of secret?
    if !validate_signature(&state.secret, &body, &signature) {
        return Err(AppError::ChecksumMismatch);
    }

    if !state
        .permitted_ranges
        .iter()
        .any(|range| range.contains(&ip))
    {
        return Err(AppError::IpOutOfRange);
    };

    if payload.sender.login != "mishok13" {
        return Err(AppError::BadUser);
    }

    // anyhow::ensure!(payload.repository.full_name == "mishok13/dotfiles", AppError::BadUser);

    // validate repo
    // get tarball url
    // download tarball?
    // extract tarball
    // run ze kommand!!1
    // but perhaps all of this in background?
    state.sender.send(payload.release.tarball_url).await?;

    Ok(())
}

fn validate_signature(secret: &[u8], payload: &[u8], signature: &[u8]) -> bool {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
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
