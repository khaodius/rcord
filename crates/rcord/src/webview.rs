//! Opens `rcord-webview`, the small browser window on discord.com used when a sign-in needs
//! Discord's own page: a captcha, or signing in on Discord's page itself.

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebResult {
    CaptchaKey(String),
    Token(String),
    /// The window was closed before it finished.
    Closed,
    /// The window could not be opened; the text says why, for the person.
    Failed(String),
}

fn helper() -> PathBuf {
    let name = format!("rcord-webview{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn run(args: Vec<String>) -> WebResult {
    let mut cmd = Command::new(helper());
    cmd.args(&args)
        .arg("--user-agent")
        .arg(crate::net::user_agent())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("could not start rcord-webview: {e}");
            return WebResult::Failed(
                "rcord could not open Discord's window because rcord-webview is missing. \
                 Reinstall rcord to restore it."
                    .into(),
            );
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(value) = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
    {
        let text = |k: &str| value.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        if let Some(key) = text("captcha_key") {
            return WebResult::CaptchaKey(key);
        }
        if let Some(token) = text("token") {
            return WebResult::Token(token);
        }
    }
    if output.status.success() {
        return WebResult::Closed;
    }
    let err = String::from_utf8_lossy(&output.stderr);
    tracing::warn!("rcord-webview failed: {}", err.trim());
    WebResult::Failed(if cfg!(windows) {
        "Discord's window could not open. It needs the Microsoft Edge WebView2 Runtime, \
         which Windows Update installs."
            .into()
    } else {
        "Discord's window could not open. It needs WebKitGTK: install libwebkit2gtk-4.1 \
         (webkit2gtk4.1 on Fedora)."
            .into()
    })
}

/// Shows Discord's captcha and waits for the solved key. Blocks, so run it off the UI thread.
pub fn solve_captcha(sitekey: &str, rqdata: Option<&str>) -> WebResult {
    let mut args = vec!["captcha".to_owned(), sitekey.to_owned()];
    if let Some(r) = rqdata {
        args.push(r.to_owned());
    }
    run(args)
}

/// Shows Discord's own sign-in page and waits for the token. Blocks.
pub fn web_login() -> WebResult {
    run(vec!["login".to_owned()])
}
