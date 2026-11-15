//! Blocking credential IO presented as an async adapter port.

use std::sync::Arc;

use sovra_core::secrets::{AsyncSecretVault, SecretVault};

/// The composition root supplies the existing OS/development vault once.
pub fn background_vault(vault: Arc<dyn SecretVault>) -> Arc<dyn AsyncSecretVault> {
    Arc::new(BackgroundVault(vault))
}

struct BackgroundVault(Arc<dyn SecretVault>);

#[async_trait::async_trait]
impl AsyncSecretVault for BackgroundVault {
    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        let vault = self.0.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || vault.get(&key))
            .await
            .map_err(stopped)?
    }
    async fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let vault = self.0.clone();
        let key = key.to_owned();
        let secret = secret.to_owned();
        tokio::task::spawn_blocking(move || vault.set(&key, &secret))
            .await
            .map_err(stopped)?
    }
    async fn delete(&self, key: &str) -> Result<(), String> {
        let vault = self.0.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || vault.delete(&key))
            .await
            .map_err(stopped)?
    }
}

fn stopped(_: tokio::task::JoinError) -> String {
    "The credential store stopped unexpectedly.".into()
}
