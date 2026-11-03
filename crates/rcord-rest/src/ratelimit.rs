use std::collections::HashMap;
use std::time::Duration;

use http::{HeaderMap, Method};
use parking_lot::Mutex;
use tokio::time::Instant;

/// Tracks Discord's rate limits. Routes are keyed by method and path with IDs replaced,
/// except the channel, guild or webhook ID that Discord limits separately.
#[derive(Debug, Default)]
pub struct RateLimiter {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    global_until: Option<Instant>,
    routes: HashMap<String, Bucket>,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    remaining: u32,
    reset_at: Instant,
}

impl RateLimiter {
    /// Waits until a request on `route` is allowed, and counts it.
    pub async fn acquire(&self, route: &str) {
        loop {
            let wait = {
                let mut state = self.state.lock();
                let now = Instant::now();
                let global = state.global_until.filter(|t| *t > now);
                let route_wait = match state.routes.get_mut(route) {
                    Some(b) if b.reset_at <= now => {
                        state.routes.remove(route);
                        None
                    }
                    Some(b) if b.remaining == 0 => Some(b.reset_at),
                    Some(b) => {
                        b.remaining -= 1;
                        None
                    }
                    None => None,
                };
                match (global, route_wait) {
                    (None, None) => return,
                    (a, b) => a.max(b).map(|t| t - now),
                }
            };
            if let Some(wait) = wait {
                tokio::time::sleep(wait).await;
            }
        }
    }

    /// Reads `X-RateLimit-Remaining` and `X-RateLimit-Reset-After` from a response.
    pub fn update(&self, route: &str, headers: &HeaderMap) {
        let num = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok())
        };
        let (Some(remaining), Some(reset_after)) =
            (num("x-ratelimit-remaining"), num("x-ratelimit-reset-after"))
        else {
            return;
        };
        let bucket = Bucket {
            remaining: remaining.max(0.0) as u32,
            reset_at: Instant::now() + Duration::from_secs_f64(reset_after.clamp(0.0, 3_600.0)),
        };
        self.state.lock().routes.insert(route.to_owned(), bucket);
    }

    pub fn block_route(&self, route: &str, wait: Duration) {
        self.state.lock().routes.insert(
            route.to_owned(),
            Bucket {
                remaining: 0,
                reset_at: Instant::now() + wait,
            },
        );
    }

    pub fn block_global(&self, wait: Duration) {
        self.state.lock().global_until = Some(Instant::now() + wait);
    }
}

/// `POST /channels/123/messages/456/reactions/x/@me` becomes
/// `POST /channels/123/messages/:id/reactions/:id/@me`: the first ID after a major
/// resource is kept and the rest are replaced.
pub fn route_key(method: &Method, path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let mut out = String::with_capacity(path.len() + 8);
    out.push_str(method.as_str());
    out.push(' ');
    let mut prev = "";
    let mut kept_major = false;
    for (i, seg) in path.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        let is_id = !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit());
        let major = matches!(prev, "channels" | "guilds" | "webhooks");
        if is_id && major && !kept_major {
            out.push_str(seg);
            kept_major = true;
        } else if is_id || prev == "reactions" {
            out.push_str(":id");
        } else {
            out.push_str(seg);
        }
        prev = seg;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_keys() {
        assert_eq!(
            route_key(
                &Method::PUT,
                "/channels/1/messages/2/reactions/%F0%9F%94%A5/@me"
            ),
            "PUT /channels/1/messages/:id/reactions/:id/@me"
        );
        assert_eq!(
            route_key(&Method::GET, "/channels/1/messages?limit=50"),
            "GET /channels/1/messages"
        );
        assert_eq!(
            route_key(&Method::GET, "/users/5/profile"),
            "GET /users/:id/profile"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn waits_for_reset() {
        let limiter = RateLimiter::default();
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        headers.insert("x-ratelimit-reset-after", "2.5".parse().unwrap());
        limiter.update("r", &headers);
        let start = Instant::now();
        limiter.acquire("r").await;
        assert!(start.elapsed() >= Duration::from_millis(2_500));
    }
}
