//! The secret-storage port account adapters share.
//!
//! Google and GitHub keep their sign-ins through the same store as model keys (the
//! OS keychain, or the development files under `SOVRA_DEV=1`). The composition root
//! implements this once over that store and hands it to each adapter.

/// Blocking: every method may touch the OS keychain, so callers run it on a
/// blocking worker. Errors are plain words, never the secret.
pub trait SecretVault: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}
