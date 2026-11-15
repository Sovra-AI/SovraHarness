//! Token-free Microsoft adapter boundary. Runtime owns scheduling and cancellation.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{
    MicrosoftAccount, MicrosoftClient, MicrosoftError, MicrosoftRequest, MicrosoftService,
};
use crate::oauth::{AuthorizationGrant, NativeSignIn};

/// A downloaded document is decoded by the existing file adapter on a blocking worker.
/// Debug must not dump the contents or a temporary preauthenticated URL.
pub enum MicrosoftOutput {
    Data(serde_json::Value),
    File { name: String, bytes: Vec<u8> },
}

impl std::fmt::Debug for MicrosoftOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(_) => f.write_str("MicrosoftOutput::Data(..)"),
            Self::File { bytes, .. } => f
                .debug_struct("MicrosoftOutput::File")
                .field("bytes", &bytes.len())
                .finish_non_exhaustive(),
        }
    }
}

#[async_trait::async_trait]
pub trait MicrosoftConnector: Send + Sync + std::fmt::Debug {
    async fn load_client(&self) -> Result<Option<MicrosoftClient>, MicrosoftError>;
    async fn save_client(&self, client: &MicrosoftClient) -> Result<(), MicrosoftError>;
    async fn forget_client(&self) -> Result<(), MicrosoftError>;
    /// Binds the local callback. Runtime calls this on a blocking worker.
    fn begin_sign_in(
        &self,
        client: &MicrosoftClient,
        services: &BTreeSet<MicrosoftService>,
        login_hint: Option<&str>,
    ) -> Result<Box<dyn NativeSignIn>, MicrosoftError>;
    async fn restore(
        &self,
        client: &MicrosoftClient,
    ) -> Result<Option<Arc<dyn MicrosoftSession>>, MicrosoftError>;
    /// Creates an uncommitted session: finishing an obsolete attempt must not persist it.
    async fn complete_sign_in(
        &self,
        client: &MicrosoftClient,
        grant: AuthorizationGrant,
    ) -> Result<Arc<dyn MicrosoftSession>, MicrosoftError>;
}

#[async_trait::async_trait]
pub trait MicrosoftSession: Send + Sync + std::fmt::Debug {
    fn account(&self) -> MicrosoftAccount;
    /// Persists tokens and the person's service switches after runtime adopts the session.
    async fn save(&self, account: &MicrosoftAccount) -> Result<(), MicrosoftError>;
    /// Serializes with renewal, disables further calls, and removes local credentials.
    async fn disconnect(&self) -> Result<(), MicrosoftError>;
    /// Disable an old in-memory session after a replacement was persisted.
    /// Unlike disconnect, this must not delete the replacement's credentials.
    async fn retire(&self);
    async fn call(&self, request: MicrosoftRequest) -> Result<MicrosoftOutput, MicrosoftError>;
}
