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

    let agent_config = AgentConfig::with_api_key(read_gemini_api_key());
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
        Arc::new(sovra_providers::OsCredentialStore),
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
    let client =
        sovra_runtime::start_with_models(VERSION, agent_config, Some(memory), Some(models))
            .context("failed to start the runtime")?;
    sovra_ui::run(client);

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
fn read_gemini_api_key() -> Option<String> {
    std::env::var("GEMINI_API_KEY")
        .ok()
        .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
}
