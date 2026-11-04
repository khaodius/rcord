//! A small browser window on discord.com, opened by rcord when a sign-in needs something only
//! Discord's own page can do.
//!
//! - `rcord-webview captcha <sitekey> [rqdata]` shows Discord's captcha (hCaptcha) over
//!   discord.com, where Discord's site key is valid, and prints the solved key.
//! - `rcord-webview login` shows Discord's own sign-in page, and prints the account token once
//!   the page starts using it.
//!
//! The result is one line of JSON on stdout: `{"captcha_key": "..."}`, `{"token": "..."}`, or
//! nothing if the window was closed. It runs as its own process so the main client never
//! links a browser engine.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Write as _;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

const LOGIN_URL: &str = "https://discord.com/login";

enum Mode {
    Captcha {
        sitekey: String,
        rqdata: Option<String>,
    },
    Login,
}

struct Args {
    mode: Mode,
    user_agent: Option<String>,
}

fn parse_args() -> Option<Args> {
    let mut positional = Vec::new();
    let mut user_agent = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if arg == "--user-agent" {
            user_agent = it.next();
        } else {
            positional.push(arg);
        }
    }
    let mut positional = positional.into_iter();
    let mode = match positional.next()?.as_str() {
        "captcha" => Mode::Captcha {
            sitekey: positional.next()?,
            rqdata: positional.next().filter(|s| !s.is_empty()),
        },
        "login" => Mode::Login,
        _ => return None,
    };
    Some(Args { mode, user_agent })
}

/// Reports every Authorization header the page sets, which is the account token once the
/// page has signed in.
const TOKEN_HOOK: &str = r#"
(() => {
  if (window.__rcordHook) return;
  window.__rcordHook = true;
  const report = (v) => {
    if (typeof v === 'string' && v.length > 20 && !v.startsWith('Bot ')) {
      window.ipc.postMessage(JSON.stringify({ token: v }));
    }
  };
  const setHeader = XMLHttpRequest.prototype.setRequestHeader;
  XMLHttpRequest.prototype.setRequestHeader = function (name, value) {
    if (String(name).toLowerCase() === 'authorization') report(value);
    return setHeader.apply(this, arguments);
  };
  const fetch = window.fetch;
  window.fetch = function (input, init) {
    try {
      const source = (init && init.headers) || (input instanceof Request ? input.headers : undefined);
      if (source) report(new Headers(source).get('authorization'));
    } catch (_) {}
    return fetch.apply(this, arguments);
  };
})();
"#;

/// Covers the page with the captcha. It runs on discord.com, so Discord's site key is valid
/// and the page's own policy already allows hCaptcha's script.
const CAPTCHA_OVERLAY: &str = r#"
(() => {
  if (window.top !== window || window.__rcordCaptcha) return;
  window.__rcordCaptcha = true;
  const boot = () => {
    const overlay = document.createElement('div');
    overlay.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:#313338;' +
      'display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;' +
      'font:16px/1.4 "gg sans","Noto Sans",sans-serif;color:#dbdee1;text-align:center;padding:24px';
    overlay.innerHTML =
      '<div style="font-size:24px;font-weight:700;color:#f2f3f5">Are you human?</div>' +
      '<div style="color:#b5bac1;max-width:420px">Discord wants to make sure it is you. ' +
      'Complete the check and rcord will continue signing you in.</div>' +
      '<div id="rcord-captcha" style="margin-top:12px;min-height:80px"></div>';
    document.documentElement.appendChild(overlay);
    window.__rcordReady = () => {
      const id = hcaptcha.render('rcord-captcha', {
        sitekey: SITEKEY,
        theme: 'dark',
        callback: (key) => window.ipc.postMessage(JSON.stringify({ captcha_key: key })),
      });
      if (RQDATA) hcaptcha.setData(id, { rqdata: RQDATA });
    };
    const script = document.createElement('script');
    script.src = 'https://js.hcaptcha.com/1/api.js?render=explicit&onload=__rcordReady&recaptchacompat=off';
    script.async = true;
    document.head.appendChild(script);
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot);
  else boot();
})();
"#;

fn js_string(s: Option<&str>) -> String {
    s.map_or_else(
        || "null".to_owned(),
        |s| serde_json::Value::String(s.to_owned()).to_string(),
    )
}

/// Prints the result for rcord and ends the process.
fn finish(line: &str) -> ! {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
    std::process::exit(0);
}

fn main() {
    let Some(args) = parse_args() else {
        eprintln!(
            "usage: rcord-webview captcha <sitekey> [rqdata] | rcord-webview login [--user-agent UA]"
        );
        std::process::exit(2);
    };
    let (title, script, wanted) = match &args.mode {
        Mode::Captcha { sitekey, rqdata } => (
            "Verify it's you",
            CAPTCHA_OVERLAY
                .replace("SITEKEY", &js_string(Some(sitekey)))
                .replace("RQDATA", &js_string(rqdata.as_deref())),
            "captcha_key",
        ),
        Mode::Login => ("Sign in to Discord", TOKEN_HOOK.to_owned(), "token"),
    };
    let (width, height) = match args.mode {
        Mode::Captcha { .. } => (520.0, 640.0),
        Mode::Login => (1000.0, 760.0),
    };

    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = match WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(width, height))
        .with_min_inner_size(LogicalSize::new(400.0, 520.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("rcord-webview: could not open a window: {e}");
            std::process::exit(1);
        }
    };

    // RCORD_WEBVIEW_URL points the window elsewhere, to test the page scripts locally.
    let url = std::env::var("RCORD_WEBVIEW_URL").unwrap_or_else(|_| LOGIN_URL.to_owned());
    let mut builder = WebViewBuilder::new()
        .with_url(&url)
        .with_incognito(true)
        .with_initialization_script(&script)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(req.body().clone());
        });
    if let Some(ua) = &args.user_agent {
        builder = builder.with_user_agent(ua);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let built = builder.build(&window);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let built = {
        use tao::platform::unix::WindowExtUnix as _;
        use wry::WebViewBuilderExtUnix as _;
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => {
                eprintln!("rcord-webview: no GTK container for the page");
                std::process::exit(1);
            }
        }
    };
    let webview = match built {
        Ok(w) => w,
        Err(e) => {
            eprintln!("rcord-webview: could not start the web view: {e}");
            std::process::exit(1);
        }
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _ = &webview;
        match event {
            Event::UserEvent(message) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&message) else {
                    return;
                };
                if value.get(wanted).and_then(|v| v.as_str()).is_some() {
                    finish(&value.to_string());
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => std::process::exit(0),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_script_values() {
        assert_eq!(js_string(None), "null");
        assert_eq!(js_string(Some("a\"b")), r#""a\"b""#);
        let s = CAPTCHA_OVERLAY.replace("SITEKEY", &js_string(Some("k")));
        assert!(s.contains(r#"sitekey: "k""#));
    }
}
