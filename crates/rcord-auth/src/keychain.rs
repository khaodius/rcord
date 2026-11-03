/// Tokens live only in the system keychain: Windows Credential Manager, or the Secret
/// Service (GNOME Keyring, KWallet) on Linux. Nothing is written to rcord's own files.
#[derive(Debug, Clone)]
pub struct Keychain {
    service: String,
}

#[derive(Debug, thiserror::Error)]
#[error("keychain: {0}")]
pub struct KeychainError(#[from] keyring::Error);

impl Default for Keychain {
    fn default() -> Self {
        Self::new("rcord")
    }
}

impl Keychain {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, KeychainError> {
        Ok(keyring::Entry::new(&self.service, account)?)
    }

    /// The saved token for `account`, a user ID, if any.
    pub fn load(&self, account: &str) -> Result<Option<String>, KeychainError> {
        match self.entry(account)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, account: &str, token: &str) -> Result<(), KeychainError> {
        Ok(self.entry(account)?.set_password(token)?)
    }

    pub fn delete(&self, account: &str) -> Result<(), KeychainError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
