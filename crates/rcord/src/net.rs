//! The network side: a small Tokio runtime for the gateway, REST and image downloads,
//! and the bridge that hands results back to GPUI's executor.

use std::future::Future;
use std::path::PathBuf;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::Global;
use gpui_http_client::{AsyncBody, HttpClient, Request, Response, Url};
use parking_lot::RwLock;
use rcord_gateway::ClientProperties;
use rcord_rest::ClientIdentity;
use sha2::{Digest, Sha256};

/// Discord's client build number at the time of writing, used until the current one has been
/// read from Discord once. Settings > Advanced overrides it.
pub const DEFAULT_BUILD_NUMBER: u64 = 400_000;

/// The user agent for every request, the same browser the client properties describe.
pub fn user_agent() -> String {
    rcord_gateway::protocol::browser_user_agent()
}

pub struct Net {
    runtime: tokio::runtime::Runtime,
    anonymous: rcord_rest::Client,
    authed: RwLock<Option<rcord_rest::Client>>,
    pub identity: ClientIdentity,
    pub properties: ClientProperties,
}

impl Global for Net {}

impl Net {
    pub fn new(build_number: u64, locale: &str) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("rcord-net")
            .enable_all()
            .build()?;
        let properties = ClientProperties::for_this_system(build_number, locale);
        let identity = ClientIdentity {
            user_agent: user_agent(),
            super_properties: serde_json::to_value(&properties)?,
            locale: locale.to_owned(),
        };
        let anonymous = {
            let _guard = runtime.enter();
            rcord_rest::Client::new(None, &identity)?
        };
        Ok(Self {
            runtime,
            anonymous,
            authed: RwLock::new(None),
            identity,
            properties,
        })
    }

    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    pub fn anonymous(&self) -> rcord_rest::Client {
        self.anonymous.clone()
    }

    /// The signed-in client, if any.
    pub fn rest(&self) -> Option<rcord_rest::Client> {
        self.authed.read().clone()
    }

    pub fn sign_in(&self, token: &str) -> anyhow::Result<rcord_rest::Client> {
        let _guard = self.runtime.enter();
        let client = rcord_rest::Client::new(Some(token.into()), &self.identity)?;
        *self.authed.write() = Some(client.clone());
        Ok(client)
    }

    pub fn sign_out(&self) {
        *self.authed.write() = None;
    }

    /// Runs `fut` on the network runtime without waiting for its result.
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        drop(self.runtime.spawn(fut));
    }

    /// Runs `fut` on the network runtime and returns a future any executor can await.
    /// Resolves to `None` if the task panicked or the runtime shut down.
    pub fn run<T, F>(&self, fut: F) -> impl Future<Output = Option<T>> + use<T, F>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.runtime.spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.map(Result::ok)
    }
}

/// Serves GPUI's image loads from the network runtime, with a content-addressed disk cache
/// in front, so an avatar seen yesterday is not downloaded again today. Discord's CDN URLs
/// change whenever the image does, so cached files never go stale.
pub struct ImageHttp {
    handle: tokio::runtime::Handle,
    client: reqwest::Client,
    cache_dir: Option<PathBuf>,
    user_agent: http::HeaderValue,
}

impl ImageHttp {
    pub fn new(handle: tokio::runtime::Handle, cache_dir: Option<PathBuf>) -> anyhow::Result<Self> {
        let _guard = handle.enter();
        let client = reqwest::Client::builder()
            .user_agent(user_agent())
            .pool_max_idle_per_host(8)
            .build()?;
        if let Some(dir) = &cache_dir {
            let _ = std::fs::create_dir_all(dir);
        }
        Ok(Self {
            handle: handle.clone(),
            client,
            cache_dir,
            user_agent: http::HeaderValue::from_str(&user_agent())?,
        })
    }

    fn cache_path(&self, url: &str) -> Option<PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        let digest = Sha256::digest(url.as_bytes());
        let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        Some(dir.join(&name[..2]).join(name))
    }
}

/// Only Discord's own image hosts are cached on disk; other embeds are fetched each time.
fn cacheable(url: &str) -> bool {
    url.starts_with("https://cdn.discordapp.com/")
        || url.starts_with("https://media.discordapp.net/")
}

impl HttpClient for ImageHttp {
    fn type_name(&self) -> &'static str {
        "rcord::ImageHttp"
    }

    fn user_agent(&self) -> Option<&http::HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let url = req.uri().to_string();
        let method = req.method().clone();
        let client = self.client.clone();
        let cache = cacheable(&url).then(|| self.cache_path(&url)).flatten();
        let task = self.handle.spawn(async move {
            if method == http::Method::GET
                && let Some(path) = &cache
                && let Ok(bytes) = tokio::fs::read(path).await
            {
                return Ok::<_, anyhow::Error>((200u16, bytes));
            }
            let res = client.request(method, &url).send().await?;
            let status = res.status().as_u16();
            let bytes = res.bytes().await?.to_vec();
            if status == 200
                && let Some(path) = &cache
            {
                if let Some(dir) = path.parent() {
                    let _ = tokio::fs::create_dir_all(dir).await;
                }
                let tmp = path.with_extension("part");
                if tokio::fs::write(&tmp, &bytes).await.is_ok() {
                    let _ = tokio::fs::rename(&tmp, path).await;
                }
            }
            Ok((status, bytes))
        });
        async move {
            let (status, bytes) = task.await??;
            Ok(Response::builder()
                .status(status)
                .body(AsyncBody::from(bytes))?)
        }
        .boxed()
    }
}

/// Removes the oldest cached images until the cache fits in `max_bytes`.
pub fn prune_disk_cache(dir: &std::path::Path, max_bytes: u64) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let Ok(shards) = std::fs::read_dir(dir) else {
        return;
    };
    for shard in shards.flatten() {
        let Ok(entries) = std::fs::read_dir(shard.path()) else {
            continue;
        };
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                let used = meta
                    .accessed()
                    .or_else(|_| meta.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                files.push((used, meta.len(), e.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}
