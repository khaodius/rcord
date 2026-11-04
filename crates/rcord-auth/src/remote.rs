use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures::{SinkExt, StreamExt};
use rsa::pkcs8::EncodePublicKey;
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub const REMOTE_AUTH_URL: &str = "wss://remote-auth-gateway.discord.gg/?v=2";
pub const QR_PREFIX: &str = "https://discord.com/ra/";

#[derive(Debug, thiserror::Error)]
pub enum RemoteAuthError {
    #[error("could not connect: {0}")]
    Connect(String),
    #[error("the QR code expired")]
    Expired,
    #[error("sign-in was cancelled on the phone")]
    Cancelled,
    #[error("unexpected message from Discord: {0}")]
    Protocol(String),
    #[error("could not decrypt: {0}")]
    Crypto(String),
}

/// The account shown on the phone once the code is scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedUser {
    pub id: String,
    pub discriminator: String,
    pub avatar: Option<String>,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteAuthEvent {
    /// Show this URL as a QR code.
    Code(String),
    /// Scanned; waiting for the user to approve on the phone.
    Scanned(ScannedUser),
    /// Approved. The token is the account's.
    Token(String),
}

/// How an approved QR sign-in ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteAuthFinish {
    /// The account's token, ready to use.
    Token(String),
    /// A ticket to exchange for the token with [`RemoteAuth::exchange`].
    Ticket(String),
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Incoming {
    Hello {
        heartbeat_interval: u64,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    NonceProof {
        encrypted_nonce: String,
    },
    PendingRemoteInit {
        fingerprint: String,
    },
    PendingTicket {
        encrypted_user_payload: String,
    },
    PendingLogin {
        ticket: String,
    },
    /// Older form of the final step, with the token inline.
    PendingFinish {
        encrypted_token: String,
    },
    Finish {
        encrypted_token: String,
    },
    Cancel,
    HeartbeatAck,
    #[serde(other)]
    Other,
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Outgoing<'a> {
    Init { encoded_public_key: &'a str },
    NonceProof { proof: &'a str },
    Heartbeat,
}

#[derive(Serialize)]
struct TicketBody<'a> {
    ticket: &'a str,
}

const TICKET_PATH: &str = "/users/@me/remote-auth/login";

#[derive(Deserialize)]
struct TicketResponse {
    encrypted_token: String,
}

/// One QR sign-in attempt. Each attempt generates a fresh RSA key; Discord encrypts every
/// secret it sends with it, so the token never crosses the wire in the clear.
pub struct RemoteAuth {
    key: RsaPrivateKey,
}

impl std::fmt::Debug for RemoteAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteAuth")
    }
}

impl RemoteAuth {
    /// Generates the key pair. This takes a moment, so run it off the UI thread.
    pub fn new() -> Result<Self, RemoteAuthError> {
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)
            .map_err(|e| RemoteAuthError::Crypto(e.to_string()))?;
        Ok(Self { key })
    }

    fn decrypt(&self, b64: &str) -> Result<Vec<u8>, RemoteAuthError> {
        let data = STANDARD
            .decode(b64)
            .map_err(|e| RemoteAuthError::Crypto(e.to_string()))?;
        self.key
            .decrypt(Oaep::new::<Sha256>(), &data)
            .map_err(|e| RemoteAuthError::Crypto(e.to_string()))
    }

    /// Runs the handshake, reporting each step to `on_event`, until the phone approves or the
    /// attempt fails.
    pub async fn run(
        &self,
        mut on_event: impl FnMut(RemoteAuthEvent),
    ) -> Result<RemoteAuthFinish, RemoteAuthError> {
        let mut request = REMOTE_AUTH_URL
            .into_client_request()
            .map_err(|e| RemoteAuthError::Connect(e.to_string()))?;
        request.headers_mut().insert(
            "Origin",
            http::HeaderValue::from_static("https://discord.com"),
        );
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| RemoteAuthError::Connect(e.to_string()))?;
        let (mut sink, mut stream) = ws.split();

        let send = |msg: Outgoing<'_>| {
            serde_json::to_string(&msg)
                .map(|s| Message::Text(s.into()))
                .map_err(|e| RemoteAuthError::Protocol(e.to_string()))
        };

        let mut heartbeat = tokio::time::interval(Duration::from_secs(3600));
        heartbeat.tick().await;
        let mut deadline = Box::pin(tokio::time::sleep(Duration::from_secs(180)));

        loop {
            let frame = tokio::select! {
                frame = stream.next() => frame,
                _ = heartbeat.tick() => {
                    sink.send(send(Outgoing::Heartbeat)?).await
                        .map_err(|e| RemoteAuthError::Connect(e.to_string()))?;
                    continue;
                }
                () = &mut deadline => return Err(RemoteAuthError::Expired),
            };
            let text = match frame {
                Some(Ok(Message::Text(t))) => t,
                Some(Ok(Message::Close(_))) | None => return Err(RemoteAuthError::Expired),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(RemoteAuthError::Connect(e.to_string())),
            };
            let msg: Incoming = serde_json::from_str(&text)
                .map_err(|e| RemoteAuthError::Protocol(e.to_string()))?;
            match msg {
                Incoming::Hello {
                    heartbeat_interval,
                    timeout_ms,
                } => {
                    let period = Duration::from_millis(heartbeat_interval.max(1_000));
                    heartbeat =
                        tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                    if let Some(ms) = timeout_ms {
                        deadline
                            .as_mut()
                            .reset(tokio::time::Instant::now() + Duration::from_millis(ms));
                    }
                    let public = RsaPublicKey::from(&self.key)
                        .to_public_key_der()
                        .map_err(|e| RemoteAuthError::Crypto(e.to_string()))?;
                    let encoded = STANDARD.encode(public.as_bytes());
                    sink.send(send(Outgoing::Init {
                        encoded_public_key: &encoded,
                    })?)
                    .await
                    .map_err(|e| RemoteAuthError::Connect(e.to_string()))?;
                }
                Incoming::NonceProof { encrypted_nonce } => {
                    let nonce = self.decrypt(&encrypted_nonce)?;
                    let proof = URL_SAFE_NO_PAD.encode(Sha256::digest(&nonce));
                    sink.send(send(Outgoing::NonceProof { proof: &proof })?)
                        .await
                        .map_err(|e| RemoteAuthError::Connect(e.to_string()))?;
                }
                Incoming::PendingRemoteInit { fingerprint } => {
                    on_event(RemoteAuthEvent::Code(format!("{QR_PREFIX}{fingerprint}")));
                }
                Incoming::PendingTicket {
                    encrypted_user_payload,
                } => {
                    let payload = self.decrypt(&encrypted_user_payload)?;
                    if let Some(user) = parse_user_payload(&String::from_utf8_lossy(&payload)) {
                        on_event(RemoteAuthEvent::Scanned(user));
                    }
                }
                Incoming::PendingLogin { ticket } => {
                    return Ok(RemoteAuthFinish::Ticket(ticket));
                }
                Incoming::PendingFinish { encrypted_token }
                | Incoming::Finish { encrypted_token } => {
                    let token = self.decrypt_token(&encrypted_token)?;
                    on_event(RemoteAuthEvent::Token(token.clone()));
                    return Ok(RemoteAuthFinish::Token(token));
                }
                Incoming::Cancel => return Err(RemoteAuthError::Cancelled),
                Incoming::HeartbeatAck | Incoming::Other => {}
            }
        }
    }

    /// Exchanges the ticket from an approved scan for the token. Discord may ask for a
    /// captcha first; solve it and call this again with the solution.
    pub async fn exchange(
        &self,
        rest: &rcord_rest::Client,
        ticket: &str,
        captcha: Option<&crate::CaptchaSolution>,
    ) -> Result<String, crate::LoginError> {
        let headers = captcha
            .map(crate::CaptchaSolution::headers)
            .unwrap_or_default();
        let res: TicketResponse = rest
            .request_with(
                http::Method::POST,
                TICKET_PATH,
                Some(&TicketBody { ticket }),
                &headers,
            )
            .await?;
        self.decrypt_token(&res.encrypted_token)
            .map_err(|e| crate::LoginError::Rejected(e.to_string()))
    }

    fn decrypt_token(&self, b64: &str) -> Result<String, RemoteAuthError> {
        String::from_utf8(self.decrypt(b64)?).map_err(|e| RemoteAuthError::Crypto(e.to_string()))
    }
}

/// `id:discriminator:avatar:username`, where the username may itself contain colons.
fn parse_user_payload(s: &str) -> Option<ScannedUser> {
    let mut parts = s.splitn(4, ':');
    let id = parts.next()?.to_owned();
    let discriminator = parts.next()?.to_owned();
    let avatar = parts.next()?;
    let username = parts.next()?.to_owned();
    Some(ScannedUser {
        id,
        discriminator,
        avatar: (!avatar.is_empty() && avatar != "0").then(|| avatar.to_owned()),
        username,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_payload() {
        assert_eq!(
            parse_user_payload("123:0:abc:name:with:colons"),
            Some(ScannedUser {
                id: "123".into(),
                discriminator: "0".into(),
                avatar: Some("abc".into()),
                username: "name:with:colons".into(),
            })
        );
        assert_eq!(parse_user_payload("123:0::n").unwrap().avatar, None);
    }

    #[test]
    fn decrypts_what_discord_would_encrypt() {
        let auth = RemoteAuth::new().unwrap();
        let public = RsaPublicKey::from(&auth.key);
        let sealed = public
            .encrypt(
                &mut rsa::rand_core::OsRng,
                Oaep::new::<Sha256>(),
                b"nonce-bytes",
            )
            .unwrap();
        let decrypted = auth.decrypt(&STANDARD.encode(sealed)).unwrap();
        assert_eq!(decrypted, b"nonce-bytes");
    }

    #[test]
    fn parses_server_messages() {
        let m: Incoming =
            serde_json::from_str(r#"{"op":"pending_remote_init","fingerprint":"abc"}"#).unwrap();
        assert!(matches!(m, Incoming::PendingRemoteInit { fingerprint } if fingerprint == "abc"));
        let m: Incoming = serde_json::from_str(r#"{"op":"something_new"}"#).unwrap();
        assert!(matches!(m, Incoming::Other));
        assert_eq!(
            serde_json::to_string(&Outgoing::Heartbeat).unwrap(),
            r#"{"op":"heartbeat"}"#
        );
    }
}
