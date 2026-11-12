//! The GitHub adapter port. `sovra-github` implements it; the runtime drives it.
//!
//! Methods documented as blocking touch the OS keychain, so the runtime calls them
//! on a blocking worker; the async ones only do HTTP. The runtime owns the waiting
//! between sign-in checks, so the adapter never sleeps.

use super::{GitHubAccount, GitHubError, GitHubKey, GitHubRequest};

/// One sign-in attempt on GitHub's page. `Debug` hides the device code.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCode {
    /// Sent back to GitHub on every check; secret for the length of the attempt.
    pub device_code: String,
    /// What the person types on GitHub's page, e.g. `WDJB-MJHT`.
    pub user_code: String,
    pub verification_uri: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .finish_non_exhaustive()
    }
}

/// The result of one check on a sign-in attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Waiting,
    SlowDown,
    /// The person pressed Authorize. The access is held in memory until
    /// [`GitHubConnector::save`].
    Done(GitHubAccount),
}

#[async_trait::async_trait]
pub trait GitHubConnector: Send + Sync + std::fmt::Debug {
    /// Whether this copy can sign in on GitHub's page (a GitHub app is built in or
    /// configured). Without one only a pasted key works. Not blocking.
    fn device_sign_in(&self) -> bool;
    /// Blocking: the account saved in the keychain, if any.
    fn restore(&self) -> Result<Option<GitHubAccount>, GitHubError>;
    /// Asks GitHub for a code the person types on its page.
    async fn start_device_sign_in(&self) -> Result<DeviceCode, GitHubError>;
    /// Asks GitHub once whether the person has finished on its page.
    async fn poll_device_sign_in(&self, device: &DeviceCode) -> Result<DevicePoll, GitHubError>;
    /// Checks a pasted key with GitHub. A working key is held in memory until
    /// [`GitHubConnector::save`].
    async fn check_key(&self, key: &GitHubKey) -> Result<GitHubAccount, GitHubError>;
    /// Blocking: keeps the account and the access held in memory in the keychain.
    fn save(&self, account: &GitHubAccount) -> Result<(), GitHubError>;
    /// Blocking: removes the keychain entry and the in-memory access.
    fn forget(&self) -> Result<(), GitHubError>;
    /// One API call on behalf of an agent tool. The result is compact JSON for the
    /// model, with outside text already marked as untrusted.
    async fn call(&self, request: GitHubRequest) -> Result<serde_json::Value, GitHubError>;
}
