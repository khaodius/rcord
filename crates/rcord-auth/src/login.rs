//! Signing in with an email or phone number and password, the way Discord's login screen
//! does: the password step, then a two-factor code when the account has one, with a
//! captcha in between whenever Discord asks for one.

use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A captcha Discord wants solved before it accepts a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captcha {
    pub service: String,
    pub sitekey: String,
    pub rqdata: Option<String>,
    pub rqtoken: Option<String>,
    pub session_id: Option<String>,
}

impl Captcha {
    /// Reads the captcha fields from an error body, when there are any.
    pub fn from_error(err: &rcord_rest::Error) -> Option<Self> {
        let rcord_rest::Error::Api {
            captcha: Some(body),
            ..
        } = err
        else {
            return None;
        };
        let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            service: text("captcha_service").unwrap_or_else(|| "hcaptcha".into()),
            sitekey: text("captcha_sitekey")?,
            rqdata: text("captcha_rqdata"),
            rqtoken: text("captcha_rqtoken"),
            session_id: text("captcha_session_id"),
        })
    }

    /// The headers that carry `key`, the solved captcha, on the retried request.
    pub fn solved(&self, key: String) -> CaptchaSolution {
        CaptchaSolution {
            key,
            rqtoken: self.rqtoken.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

/// A solved captcha, sent with the request that asked for it.
#[derive(Debug, Clone)]
pub struct CaptchaSolution {
    pub key: String,
    pub rqtoken: Option<String>,
    pub session_id: Option<String>,
}

impl CaptchaSolution {
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![("X-Captcha-Key", self.key.clone())];
        if let Some(t) = &self.rqtoken {
            out.push(("X-Captcha-Rqtoken", t.clone()));
        }
        if let Some(s) = &self.session_id {
            out.push(("X-Captcha-Session-Id", s.clone()));
        }
        out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    /// Solve the captcha, then send the same request again with the solution.
    #[error("Discord wants a captcha solved")]
    Captcha(Captcha),
    #[error("{0}")]
    Rejected(String),
    #[error("{0}")]
    Network(String),
}

impl From<rcord_rest::Error> for LoginError {
    fn from(err: rcord_rest::Error) -> Self {
        if let Some(c) = Captcha::from_error(&err) {
            return Self::Captcha(c);
        }
        match err {
            rcord_rest::Error::Api { message, .. } => Self::Rejected(message),
            rcord_rest::Error::RateLimited(wait) => Self::Rejected(format!(
                "Too many attempts. Try again in {} seconds.",
                wait.as_secs().max(1)
            )),
            other => Self::Network(other.to_string()),
        }
    }
}

/// The ways an account can confirm a sign-in after the password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfaChallenge {
    pub ticket: String,
    pub totp: bool,
    pub sms: bool,
    pub backup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    Token(String),
    Mfa(MfaChallenge),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfaMethod {
    /// A code from an authenticator app.
    Totp,
    /// A code sent by text message.
    Sms,
    /// One of the account's backup codes.
    Backup,
}

impl MfaMethod {
    fn path(self) -> &'static str {
        match self {
            Self::Totp => "/auth/mfa/totp",
            Self::Sms => "/auth/mfa/sms",
            Self::Backup => "/auth/mfa/backup",
        }
    }
}

#[derive(Serialize)]
struct LoginBody<'a> {
    login: &'a str,
    password: &'a str,
    undelete: bool,
    login_source: Option<()>,
    gift_code_sku_id: Option<()>,
}

#[derive(Deserialize)]
struct LoginResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    mfa: bool,
    #[serde(default)]
    ticket: Option<String>,
    #[serde(default)]
    totp: bool,
    #[serde(default)]
    sms: bool,
    #[serde(default)]
    backup: bool,
}

#[derive(Serialize)]
struct MfaBody<'a> {
    code: &'a str,
    ticket: &'a str,
    login_source: Option<()>,
    gift_code_sku_id: Option<()>,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Serialize)]
struct TicketBody<'a> {
    ticket: &'a str,
}

#[derive(Deserialize)]
struct SmsSent {
    #[serde(default)]
    phone: Option<String>,
}

#[derive(Serialize)]
struct ForgotBody<'a> {
    login: &'a str,
}

#[derive(Deserialize)]
struct Experiments {
    #[serde(default)]
    fingerprint: Option<String>,
}

/// The browser fingerprint Discord's login page sends with the password; without one some
/// sign-ins are refused or asked for a captcha more often.
async fn fingerprint(rest: &rcord_rest::Client) -> Option<String> {
    let e: Experiments = rest.get("/experiments").await.ok()?;
    e.fingerprint
}

fn with_captcha(
    mut headers: Vec<(&'static str, String)>,
    captcha: Option<&CaptchaSolution>,
) -> Vec<(&'static str, String)> {
    if let Some(c) = captcha {
        headers.extend(c.headers());
    }
    headers
}

/// The password step. `rest` is an unauthenticated client.
pub async fn login(
    rest: &rcord_rest::Client,
    login: &str,
    password: &str,
    captcha: Option<&CaptchaSolution>,
) -> Result<LoginOutcome, LoginError> {
    let mut headers = Vec::new();
    if let Some(f) = fingerprint(rest).await {
        headers.push(("X-Fingerprint", f));
    }
    let res: LoginResponse = rest
        .request_with(
            Method::POST,
            "/auth/login",
            Some(&LoginBody {
                login: login.trim(),
                password,
                undelete: false,
                login_source: None,
                gift_code_sku_id: None,
            }),
            &with_captcha(headers, captcha),
        )
        .await?;
    outcome(res)
}

fn outcome(res: LoginResponse) -> Result<LoginOutcome, LoginError> {
    if let Some(token) = res.token.filter(|t| !t.is_empty()) {
        return Ok(LoginOutcome::Token(token));
    }
    match res.ticket {
        Some(ticket) if res.mfa => Ok(LoginOutcome::Mfa(MfaChallenge {
            ticket,
            totp: res.totp,
            sms: res.sms,
            backup: res.backup,
        })),
        _ => Err(LoginError::Rejected(
            "Discord did not finish the sign-in. Try again, or sign in with the QR code.".into(),
        )),
    }
}

/// The two-factor step: a code for `method`, with the ticket from the password step.
pub async fn verify_mfa(
    rest: &rcord_rest::Client,
    method: MfaMethod,
    ticket: &str,
    code: &str,
    captcha: Option<&CaptchaSolution>,
) -> Result<String, LoginError> {
    let code: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    let res: TokenResponse = rest
        .request_with(
            Method::POST,
            method.path(),
            Some(&MfaBody {
                code: &code,
                ticket,
                login_source: None,
                gift_code_sku_id: None,
            }),
            &with_captcha(Vec::new(), captcha),
        )
        .await?;
    Ok(res.token)
}

/// Texts a code to the account's phone. Returns the masked number it went to.
pub async fn send_sms(rest: &rcord_rest::Client, ticket: &str) -> Result<String, LoginError> {
    let res: SmsSent = rest
        .post("/auth/mfa/sms/send", &TicketBody { ticket })
        .await?;
    Ok(res.phone.unwrap_or_else(|| "your phone".into()))
}

/// Emails a password reset link, or texts one for a phone number.
pub async fn forgot_password(
    rest: &rcord_rest::Client,
    login: &str,
    captcha: Option<&CaptchaSolution>,
) -> Result<(), LoginError> {
    rest.request_with::<_, Value>(
        Method::POST,
        "/auth/forgot",
        Some(&ForgotBody {
            login: login.trim(),
        }),
        &with_captcha(Vec::new(), captcha),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(body: Value) -> rcord_rest::Error {
        rcord_rest::Error::Api {
            status: http::StatusCode::BAD_REQUEST,
            code: None,
            message: "Bad Request".into(),
            captcha: body.get("captcha_key").map(|_| body.clone()),
        }
    }

    #[test]
    fn reads_captcha_challenge() {
        let err = api(serde_json::json!({
            "captcha_key": ["captcha-required"],
            "captcha_sitekey": "a9b5fb07-92ff-493f-86fe-352a2803b3df",
            "captcha_service": "hcaptcha",
            "captcha_rqdata": "rq",
            "captcha_rqtoken": "tok",
            "captcha_session_id": "sid"
        }));
        let LoginError::Captcha(c) = LoginError::from(err) else {
            panic!("not a captcha");
        };
        assert_eq!(c.sitekey, "a9b5fb07-92ff-493f-86fe-352a2803b3df");
        let headers = c.solved("key".into()).headers();
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0], ("X-Captcha-Key", "key".to_owned()));
    }

    #[test]
    fn reads_login_outcomes() {
        let token: LoginResponse =
            serde_json::from_value(serde_json::json!({"user_id": "1", "token": "t"})).unwrap();
        assert_eq!(outcome(token).unwrap(), LoginOutcome::Token("t".into()));
        let mfa: LoginResponse = serde_json::from_value(serde_json::json!({
            "user_id": "1", "mfa": true, "sms": false, "ticket": "tk", "backup": true,
            "totp": true, "webauthn": null
        }))
        .unwrap();
        assert_eq!(
            outcome(mfa).unwrap(),
            LoginOutcome::Mfa(MfaChallenge {
                ticket: "tk".into(),
                totp: true,
                sms: false,
                backup: true
            })
        );
    }
}
