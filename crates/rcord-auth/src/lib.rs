//! Signing in: email or phone and password with two-factor codes, QR code login through
//! Discord's remote auth gateway, and storage of the resulting token in the system keychain.

mod keychain;
mod login;
mod remote;

pub use keychain::{Keychain, KeychainError};
pub use login::{
    Captcha, CaptchaSolution, LoginError, LoginOutcome, MfaChallenge, MfaMethod, forgot_password,
    login, send_sms, verify_mfa,
};
pub use remote::{RemoteAuth, RemoteAuthError, RemoteAuthEvent, RemoteAuthFinish, ScannedUser};
