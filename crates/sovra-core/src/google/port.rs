//! The Google adapter port. `sovra-google` implements it; the runtime drives it.
//!
//! Methods documented as blocking touch the OS keychain or a local socket, so the
//! runtime calls them on a blocking worker; the async ones only do HTTP.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{GoogleAccount, GoogleError, GoogleRequest, GoogleService};

/// Stops a pending sign-in from any thread; waking the waiter is the adapter's job.
pub type SignInCanceller = Arc<dyn Fn() + Send + Sync>;

/// What the browser handed back, ready to exchange for tokens. Debug hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationGrant {
    pub code: String,
    /// PKCE verifier generated for this attempt.
    pub verifier: String,
    pub redirect_uri: String,
}

impl std::fmt::Debug for AuthorizationGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorizationGrant")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// One sign-in attempt: a link for the browser and a single-use local listener.
pub trait GoogleSignIn: Send + std::fmt::Debug {
    /// Google's consent page for this attempt.
    fn url(&self) -> &str;
    fn canceller(&self) -> SignInCanceller;
    /// Blocking: waits for the browser to come back (or for the canceller).
    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, GoogleError>;
}

#[async_trait::async_trait]
pub trait GoogleConnector: Send + Sync + std::fmt::Debug {
    /// Why this build cannot sign in to Google, if it cannot.
    fn unavailable_reason(&self) -> Option<String>;
    /// Blocking: the account saved in the keychain, if any.
    fn restore(&self) -> Result<Option<GoogleAccount>, GoogleError>;
    /// Blocking: opens the local listener and builds the consent link for `services`.
    fn begin_sign_in(
        &self,
        services: &BTreeSet<GoogleService>,
        login_hint: Option<&str>,
    ) -> Result<Box<dyn GoogleSignIn>, GoogleError>;
    /// Exchanges the grant for tokens and reads who signed in. The new tokens are
    /// held in memory until [`GoogleConnector::save`].
    async fn complete_sign_in(
        &self,
        grant: AuthorizationGrant,
    ) -> Result<GoogleAccount, GoogleError>;
    /// Blocking: keeps the account and its sign-in in the keychain.
    fn save(&self, account: &GoogleAccount) -> Result<(), GoogleError>;
    /// Asks Google to forget Sovra's access. Best effort: offline still signs out.
    async fn revoke(&self) -> Result<(), GoogleError>;
    /// Blocking: removes the keychain entry and the in-memory sign-in.
    fn forget(&self) -> Result<(), GoogleError>;
    /// One API call on behalf of an agent tool. The result is compact JSON for
    /// the model, with outside text already marked as untrusted.
    async fn call(&self, request: GoogleRequest) -> Result<serde_json::Value, GoogleError>;
}
