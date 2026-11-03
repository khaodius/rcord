//! Discord's HTTP API: one shared connection pool, per-route rate limits read from the
//! response headers, and retries on 429 that honour `retry_after`.

mod ratelimit;
pub mod routes;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use ratelimit::RateLimiter;

pub const API_BASE: &str = "https://discord.com/api/v9";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Discord returned {status}: {message}")]
    Api {
        status: StatusCode,
        /// Discord's JSON error code, such as 50001 for missing access.
        code: Option<u64>,
        message: String,
        /// Present when the request needs a captcha solved first.
        captcha: Option<serde_json::Value>,
    },
    #[error("could not read the response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("rate limited for {0:?}")]
    RateLimited(Duration),
}

impl Error {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Api { status, .. } => Some(*status),
            Self::Network(e) => e.status(),
            _ => None,
        }
    }

    pub fn is_unauthorized(&self) -> bool {
        self.status() == Some(StatusCode::UNAUTHORIZED)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Identifying headers Discord expects from its own clients.
#[derive(Debug, Clone)]
pub struct ClientIdentity {
    pub user_agent: String,
    /// The same properties IDENTIFY sends, as JSON.
    pub super_properties: serde_json::Value,
    pub locale: String,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    token: Option<Arc<str>>,
    base: String,
    limiter: RateLimiter,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.inner.base)
            .field("authenticated", &self.inner.token.is_some())
            .finish()
    }
}

impl Client {
    /// A client for `token`, or an anonymous one for sign-in requests.
    pub fn new(token: Option<Arc<str>>, identity: &ClientIdentity) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let props = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&identity.super_properties)?);
        if let Ok(v) = HeaderValue::from_str(&props) {
            headers.insert("X-Super-Properties", v);
        }
        if let Ok(v) = HeaderValue::from_str(&identity.locale) {
            headers.insert("X-Discord-Locale", v);
        }
        let http = reqwest::Client::builder()
            .user_agent(&identity.user_agent)
            .default_headers(headers)
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                token,
                base: API_BASE.to_owned(),
                limiter: RateLimiter::default(),
            }),
        })
    }

    /// The underlying HTTP client, shared with image loading so both use one pool.
    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request::<(), T>(Method::GET, path, None).await
    }

    pub async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.request(Method::POST, path, Some(body)).await
    }

    pub async fn patch<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.request(Method::PATCH, path, Some(body)).await
    }

    /// Sends a request whose response has no body worth reading.
    pub async fn send_empty<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<()> {
        self.execute(method, path, body).await.map(drop)
    }

    pub async fn request<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        let bytes = self.execute(method, path, body).await?;
        let bytes = if bytes.is_empty() {
            &b"null"[..]
        } else {
            &bytes[..]
        };
        Ok(serde_json::from_slice(bytes)?)
    }

    async fn execute<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<bytes::Bytes> {
        let route = ratelimit::route_key(&method, path);
        let encoded = body.map(serde_json::to_vec).transpose()?;
        let mut attempts: u32 = 0;
        loop {
            self.inner.limiter.acquire(&route).await;
            let mut req = self
                .inner
                .http
                .request(method.clone(), format!("{}{path}", self.inner.base));
            if let Some(token) = &self.inner.token {
                req = req.header(http::header::AUTHORIZATION, token.as_ref());
            }
            if let Some(body) = &encoded {
                req = req
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(body.clone());
            }
            let res = req.send().await?;
            let status = res.status();
            self.inner.limiter.update(&route, res.headers());
            let bytes = res.bytes().await?;
            if status == StatusCode::TOO_MANY_REQUESTS {
                let info: RateLimitBody = serde_json::from_slice(&bytes).unwrap_or_default();
                let wait = Duration::from_secs_f64(info.retry_after.clamp(0.0, 3_600.0));
                if info.global {
                    self.inner.limiter.block_global(wait);
                } else {
                    self.inner.limiter.block_route(&route, wait);
                }
                attempts += 1;
                if attempts > 3 || wait > Duration::from_secs(30) {
                    return Err(Error::RateLimited(wait));
                }
                continue;
            }
            if status.is_server_error() && attempts < 2 && method == Method::GET {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempts))).await;
                continue;
            }
            if !status.is_success() {
                let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap_or_default();
                return Err(Error::Api {
                    status,
                    code: err.code,
                    message: err
                        .message
                        .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").into()),
                    captcha: err
                        .captcha_key
                        .map(|_| serde_json::from_slice(&bytes).unwrap_or_default()),
                });
            }
            return Ok(bytes);
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct RateLimitBody {
    #[serde(default)]
    retry_after: f64,
    #[serde(default)]
    global: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    code: Option<u64>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    captcha_key: Option<serde_json::Value>,
}
