//! SovraHarness desktop binary — the composition root.
//!
//! Process setup and wiring only: installer hooks, paths, logging, runtime, UI.
//! Logic belongs in the library crates.

mod logging;

use std::sync::Arc;

use anyhow::Context as _;
use sovra_core::{AgentConfig, MemoryStore};
use sovra_store_surreal::{StoreConfig, SurrealStore};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> anyhow::Result<()> {
    // Must run first: handles Velopack install/update/uninstall hooks and may exit.
    velopack::VelopackApp::build().run();

    let dirs = directories::ProjectDirs::from("com", "Sovra", "SovraHarness")
        .context("could not resolve the user's home directory")?;
    let _log_guard = logging::init(&dirs.data_local_dir().join("logs"))?;
    tracing::info!(version = VERSION, "starting SovraHarness");

    let mut agent_config = AgentConfig::with_api_key(read_gemini_api_key());
    agent_config.searxng_base_url = read_searxng_base_url();
    // The composition root picks the memory backend; the runtime opens it on its own
    // executor. Swapping to a remote server (or another backend) happens here only.
    let memory_dir = dirs.data_local_dir().join("memory");
    let memory: sovra_runtime::MemoryFactory = Box::new(move || {
        Box::pin(async move {
            let store = SurrealStore::open(StoreConfig::on_disk(memory_dir)).await?;
            Ok(Arc::new(store) as Arc<dyn MemoryStore>)
        })
    });
    let model_repository = sovra_providers::ModelRepository::new(
        dirs.data_local_dir().join("models"),
        credential_store(&dirs),
        read_model_keys(),
    );
    let models = sovra_runtime::ModelsConfig {
        repository: model_repository,
        providers: vec![
            Arc::new(sovra_providers::OpenRouterProvider),
            Arc::new(sovra_providers::OllamaProvider),
            Arc::new(sovra_providers::CompatibleProvider),
            Arc::new(sovra_providers::AnthropicProvider),
        ],
    };
    let client = sovra_runtime::start_with_file_index(
        VERSION,
        agent_config,
        Some(memory),
        Some(models),
        Some(Arc::new(sovra_files::LocalFileStore::new(
            dirs.data_local_dir().join("attachments"),
        ))),
        Some(Arc::new(sovra_skills::LocalSkillStore::new(
            dirs.data_local_dir().join("skills"),
        ))),
        Some(Arc::new(sovra_files::LocalFileIndex::new(
            dirs.data_local_dir().join("file-index"),
        ))),
    )
    .context("failed to start the runtime")?;
    sovra_ui::run(client.clone());
    client
        .shutdown_blocking()
        .context("runtime did not finish shutting down")?;

    tracing::info!("exiting");
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the composition root reads environment credentials once at startup"
)]
fn read_model_keys() -> std::collections::BTreeMap<String, sovra_core::models::ModelSecret> {
    [
        ("openrouter", "OPENROUTER_API_KEY"),
        ("ollama", "OLLAMA_API_KEY"),
    ]
    .into_iter()
    .filter_map(|(id, variable)| {
        std::env::var(variable)
            .ok()
            .filter(|key| !key.trim().is_empty())
            .map(|key| {
                (
                    id.to_owned(),
                    sovra_core::models::ModelSecret(key.trim().to_owned()),
                )
            })
    })
    .collect()
}

#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
/// Optional: web search needs no setup. A self-hosted `SearXNG` only adds a source.
fn read_searxng_base_url() -> Option<String> {
    std::env::var("SOVRA_SEARXNG_URL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
fn read_gemini_api_key() -> Option<String> {
    std::env::var("GEMINI_API_KEY")
        .ok()
        .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
}

/// `cargo xtask dev` sets `SOVRA_DEV`: every rebuild changes the binary's code
/// signature, which makes macOS re-prompt for the keychain password each reload.
fn credential_store(dirs: &directories::ProjectDirs) -> Arc<dyn sovra_providers::CredentialStore> {
    if cfg!(debug_assertions) && std::env::var_os("SOVRA_DEV").is_some_and(|value| value == "1") {
        tracing::warn!("SOVRA_DEV set: model keys use plaintext dev files, not the keychain");
        Arc::new(sovra_providers::FileCredentialStore::new(
            dirs.data_local_dir().join("dev-credentials"),
        ))
    } else {
        Arc::new(sovra_providers::OsCredentialStore)
    }
}
