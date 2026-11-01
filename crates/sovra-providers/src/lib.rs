//! IO adapters for model catalogs, configurable providers and secure credentials.
//! The composition root registers providers; runtime owns their tasks and lifecycle.

mod auth;
mod catalog;
mod connection;
mod destination;
mod embeddings;
mod summaries;
mod transport;
pub use destination::ResolvedModel;
mod storage;
pub use embeddings::HttpEmbedder;
pub use summaries::HttpSummarizer;

pub use auth::{
    AuthError, GoogleAuthFlow, GoogleIdentity, GoogleOAuthConfig, SignInProof, hash_password,
    verify_password,
};
pub use catalog::{
    AnthropicProvider, CompatibleProvider, OllamaProvider, OpenRouterProvider, pull_ollama,
    test_connection, verify_key,
};
pub use connection::{endpoint, validate_connection};
pub use storage::{
    CredentialStore, FileCredentialStore, ModelRepository, OsCredentialStore, SavedModels,
};
