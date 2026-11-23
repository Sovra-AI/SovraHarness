//! IO adapters for model catalogs, configurable providers, secure credentials and
//! online storage (S3-language buckets, `WebDAV` folders, Azure Blob containers).
//! The composition root registers providers; runtime owns their tasks and lifecycle.

mod auth;
mod catalog;
mod connection;
mod destination;
mod embeddings;
mod online_storage;
mod outlets;
mod sigv4;
mod summaries;
mod transport;
pub use destination::ResolvedModel;
mod storage;
pub use embeddings::HttpEmbedder;
pub use online_storage::HttpStorage;
pub use outlets::HttpModelOutlets;
pub use summaries::HttpSummarizer;

pub use auth::{
    AuthError, GoogleAuthFlow, GoogleIdentity, GoogleOAuthConfig, SignInProof, hash_password,
    verify_password,
};
pub use catalog::{
    AnthropicProvider, CompatibleProvider, OllamaProvider, OpenRouterProvider, pull_ollama,
    test_connection, verify_key,
};
pub use connection::{endpoint, model_turn_timeout, ollama_bind_address, validate_connection};
pub use storage::{
    CredentialStore, FileCredentialStore, ModelRepository, OsCredentialStore, SavedModels,
};
