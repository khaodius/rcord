//! Discord's web client build number, read from Discord's own web app.
//!
//! Discord ships its web client several times a day and every request carries the client's
//! build number. rcord sends the newest number it knows: the one read at the previous start,
//! kept in the cache folder, or the built-in one on a first start. Each start reads the
//! current number again in the background for the next start, so a long-unused install
//! catches up after one launch and nothing at startup waits on the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

const APP_URL: &str = "https://discord.com/login";
/// Older numbers than this are not plausible and are ignored.
const MIN_PLAUSIBLE: u64 = 100_000;
/// Scripts read at most before giving up.
const MAX_SCRIPTS: usize = 12;

fn cache_file(cache_dir: &Path) -> PathBuf {
    cache_dir.join("client-build-number")
}

/// The number read at the previous start, if any.
pub fn cached(cache_dir: &Path) -> Option<u64> {
    std::fs::read_to_string(cache_file(cache_dir))
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|n| *n >= MIN_PLAUSIBLE)
}

/// Reads the current number from Discord and stores it for the next start.
pub async fn refresh(cache_dir: PathBuf, user_agent: String) {
    match fetch(&user_agent).await {
        Ok(n) => {
            tracing::debug!("Discord client build number is {n}");
            let file = cache_file(&cache_dir);
            let tmp = file.with_extension("tmp");
            let written = std::fs::create_dir_all(&cache_dir)
                .and_then(|_| std::fs::write(&tmp, n.to_string()))
                .and_then(|_| std::fs::rename(&tmp, &file));
            if let Err(e) = written {
                tracing::debug!("could not store the build number: {e}");
            }
        }
        Err(e) => tracing::debug!("could not read Discord's build number: {e}"),
    }
}

async fn fetch(user_agent: &str) -> anyhow::Result<u64> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(15))
        .build()?;
    let page = client
        .get(APP_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    // The number is in one of the page's scripts, usually one of the last ones.
    let mut scripts = script_paths(&page);
    scripts.reverse();
    for path in scripts.into_iter().take(MAX_SCRIPTS) {
        let url = format!("https://discord.com{path}");
        let Ok(response) = client.get(&url).send().await else {
            continue;
        };
        let Ok(body) = response.text().await else {
            continue;
        };
        if let Some(n) = find_build_number(&body) {
            return Ok(n);
        }
    }
    anyhow::bail!("no build number in Discord's scripts")
}

/// The `/assets/….js` script paths in a page, in page order.
fn script_paths(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("src=\"/assets/") {
        let tail = &rest[start + 5..];
        let Some(end) = tail.find('"') else { break };
        let path = &tail[..end];
        if path.ends_with(".js") && !out.iter().any(|p| p == path) {
            out.push(path.to_owned());
        }
        rest = &tail[end..];
    }
    out
}

/// Finds the number after a `buildNumber` or `build_number` key in a script, in any of the
/// forms Discord's bundler has emitted: `buildNumber:"412345"`, `"buildNumber",412345`,
/// `build_number:412345`.
fn find_build_number(script: &str) -> Option<u64> {
    for key in ["buildNumber", "build_number"] {
        let mut rest = script;
        while let Some(at) = rest.find(key) {
            let after = &rest[at + key.len()..];
            // Up to a few separator characters (quotes, colon, comma, spaces), then digits.
            let skip = after
                .char_indices()
                .take(6)
                .find(|(_, c)| !matches!(c, '"' | '\'' | ':' | ',' | ' ' | '='))
                .map(|(i, _)| i);
            if let Some(skip) = skip {
                let digits: String = after[skip..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                if let Ok(n) = digits.parse::<u64>()
                    && n >= MIN_PLAUSIBLE
                {
                    return Some(n);
                }
            }
            rest = after;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_script_paths() {
        let html = r#"<script src="/assets/web.a1b2.js" defer></script>
            <link href="/assets/x.css"><script src="/assets/sentry.c3d4.js"></script>
            <script src="/assets/web.a1b2.js"></script>"#;
        assert_eq!(
            script_paths(html),
            ["/assets/web.a1b2.js", "/assets/sentry.c3d4.js"]
        );
    }

    #[test]
    fn finds_build_number_forms() {
        assert_eq!(
            find_build_number(r#"x={release:"a",buildNumber:"412345",y:1}"#),
            Some(412_345)
        );
        assert_eq!(
            find_build_number(r#"e.set("buildNumber",398765)"#),
            Some(398_765)
        );
        assert_eq!(find_build_number("{build_number: 401234}"), Some(401_234));
        assert_eq!(find_build_number(r#"buildNumber:e,buildNumber:"2""#), None);
    }

    #[test]
    fn ignores_implausible_cache() {
        let dir = std::env::temp_dir().join(format!("rcord-build-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(cache_file(&dir), "12").unwrap();
        assert_eq!(cached(&dir), None);
        std::fs::write(cache_file(&dir), "412345\n").unwrap();
        assert_eq!(cached(&dir), Some(412_345));
        std::fs::remove_dir_all(&dir).ok();
    }
}
