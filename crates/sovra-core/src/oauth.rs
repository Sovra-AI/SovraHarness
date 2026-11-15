//! Shared native sign-in contract. IO and randomness live in `sovra-oauth`.

use std::sync::Arc;

/// Wakes a pending browser callback without leaving a listener behind.
pub type SignInCanceller = Arc<dyn Fn() + Send + Sync>;

/// A one-use code bound to its PKCE verifier. Never display or log either.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationGrant {
    pub code: String,
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    #[error("Sign-in was cancelled.")]
    Cancelled,
    #[error("Sign-in wasn't allowed. Nothing was connected.")]
    Denied,
    #[error("{0}")]
    Failed(String),
}

/// Runtime calls `wait` on an owned blocking worker and cancels on timeout/drop.
pub trait NativeSignIn: Send + std::fmt::Debug {
    fn url(&self) -> &str;
    fn canceller(&self) -> SignInCanceller;
    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, OAuthError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_debug_keeps_the_code_and_verifier_private() {
        let grant = AuthorizationGrant {
            code: "secret-code".into(),
            verifier: "secret-verifier".into(),
            redirect_uri: "http://localhost:1234".into(),
        };
        let debug = format!("{grant:?}");
        assert!(!debug.contains("secret"));
        assert!(debug.contains("http://localhost:1234"));
    }
}
