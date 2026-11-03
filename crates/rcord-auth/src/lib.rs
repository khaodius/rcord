//! Signing in: QR code login through Discord's remote auth gateway, the flow the mobile app
//! approves, and storage of the resulting token in the system keychain.

mod keychain;
mod remote;

pub use keychain::{Keychain, KeychainError};
pub use remote::{RemoteAuth, RemoteAuthError, RemoteAuthEvent, ScannedUser};
