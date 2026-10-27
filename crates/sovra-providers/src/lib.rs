//! IO adapters for model catalogs, configurable providers and secure credentials.
//! The composition root registers providers; runtime owns their tasks and lifecycle.

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

pub use catalog::{
    AnthropicProvider, CompatibleProvider, OllamaProvider, OpenRouterProvider, pull_ollama,
    test_connection,
};
pub use connection::{endpoint, validate_connection};
pub use storage::{
    CredentialStore, FileCredentialStore, ModelRepository, OsCredentialStore, SavedModels,
};
