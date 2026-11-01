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
    let development_directory = dirs.data_local_dir().join("development-control");
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--development-mcp")
    {
        return sovra_runtime::run_development_mcp(development_directory)
            .context("development MCP server failed");
    }
    let development = sovra_runtime::DevelopmentConfig {
        directory: development_directory,
        executable: std::env::current_exe().context("could not resolve Sovra executable")?,
    };
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
    // WhatsApp credentials (Signal keys) live in one SQLite file in the app data directory.
    let whatsapp = sovra_whatsapp::WhatsAppChannel::new(dirs.data_local_dir().join("whatsapp"));
    // The Telegram bot token is an API key: it goes where model keys go (keychain, or the
    // SOVRA_DEV plaintext files in debug builds).
    let telegram =
        sovra_telegram::TelegramChannel::new(Arc::new(KeychainTokenStore(credential_store(&dirs))))
            .map_err(|error| tracing::warn!(%error, "telegram linking unavailable"))
            .ok();
    let google = sovra_google::GoogleAdapter::new(
        read_google_client(),
        Arc::new(CredentialVault(credential_store(&dirs))),
    )
    .context("failed to prepare Google connections")?;
    let client = sovra_runtime::start_with_services(
        VERSION,
        agent_config,
        sovra_runtime::RuntimeServices {
            memory: Some(memory),
            models: Some(models),
            files: Some(Arc::new(sovra_files::LocalFileStore::new(
                dirs.data_local_dir().join("attachments"),
            ))),
            skills: Some(Arc::new(sovra_skills::LocalSkillStore::new(
                dirs.data_local_dir().join("skills"),
            ))),
            index: Some(Arc::new(sovra_files::LocalFileIndex::new(
                dirs.data_local_dir().join("file-index"),
            ))),
            messaging: sovra_runtime::MessagingChannels {
                whatsapp: Some(Arc::new(whatsapp)),
                telegram: telegram.map(|channel| {
                    Arc::new(channel) as Arc<dyn sovra_core::connections::MessagingChannel>
                }),
            },
            signin: read_google_signin_config(),
            google: Some(Arc::new(google)),
            development: Some(development),
        },
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

#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
/// Google Sign-In is optional: without a client ID the login screen offers
/// email + password only. A desktop-app client secret is not confidential
/// (RFC 8252 §8.5); PKCE protects the flow either way.
fn read_google_signin_config() -> Option<sovra_providers::GoogleOAuthConfig> {
    let client_id = std::env::var("SOVRA_GOOGLE_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;
    let client_secret = std::env::var("SOVRA_GOOGLE_CLIENT_SECRET")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    Some(sovra_providers::GoogleOAuthConfig {
        client_id,
        client_secret,
    })
}

/// Keeps the Telegram bot token beside the model keys, under one fixed credential id.
struct KeychainTokenStore(Arc<dyn sovra_providers::CredentialStore>);

impl KeychainTokenStore {
    const ID: &'static str = "telegram-bot-token";
}

impl sovra_telegram::TokenStore for KeychainTokenStore {
    fn load(&self) -> Option<String> {
        self.0
            .get(Self::ID)
            .ok()
            .map(|secret| secret.0)
            .filter(|token| !token.trim().is_empty())
    }

    fn save(&self, token: &str) -> Result<(), String> {
        self.0
            .set(Self::ID, &sovra_core::models::ModelSecret(token.to_owned()))
            .map_err(|error| error.to_string())
    }

    fn clear(&self) -> Result<(), String> {
        self.0.delete(Self::ID).map_err(|error| error.to_string())
    }
}

/// The Google Cloud "Desktop app" OAuth client. Release builds bake it in at
/// compile time (`SOVRA_GOOGLE_CLIENT_ID`/`_SECRET`) so the single binary needs no
/// setup; the same variables at run time override it for development and for
/// people using their own Google Cloud client. Empty means "not set up yet".
#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
fn read_google_client() -> sovra_google::ClientConfig {
    let read = |name: &str, baked: Option<&'static str>| {
        std::env::var(name)
            .ok()
            .or_else(|| baked.map(str::to_owned))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    sovra_google::ClientConfig {
        client_id: read(
            "SOVRA_GOOGLE_CLIENT_ID",
            option_env!("SOVRA_GOOGLE_CLIENT_ID"),
        )
        .unwrap_or_default(),
        client_secret: read(
            "SOVRA_GOOGLE_CLIENT_SECRET",
            option_env!("SOVRA_GOOGLE_CLIENT_SECRET"),
        ),
    }
}

/// The Google sign-in shares the model keys' store (keychain, or dev files).
struct CredentialVault(Arc<dyn sovra_providers::CredentialStore>);

impl sovra_google::SecretVault for CredentialVault {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.0
            .get(key)
            .map(|secret| Some(secret.0).filter(|s| !s.is_empty()))
            .map_err(|e| e.0)
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0
            .set(key, &sovra_core::models::ModelSecret(secret.to_owned()))
            .map_err(|e| e.0)
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key).map_err(|e| e.0)
    }
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
