//! SovraHarness desktop binary — the composition root.
//!
//! Process setup and wiring only: installer hooks, paths, logging, runtime, UI.
//! Logic belongs in the library crates.

mod logging;

#[cfg(test)]
mod google_client;

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
    let data_dir = data_directory(&dirs);
    let development_directory = data_dir.as_path().join("development-control");
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--remote-control-client")
    {
        let secret = device_key(
            credential_store(&data_dir).as_ref(),
            "remote-control-reference-phone-key",
        )
        .context("could not load the reference phone identity safely")?;
        return sovra_runtime::run_remote_reference(
            data_dir.join("remote-control-reference"),
            &remote_transport(secret),
        )
        .context("remote reference client failed");
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--development-mcp")
    {
        return sovra_runtime::run_development_mcp(development_directory)
            .context("development MCP server failed");
    }
    // Sovra's built-in sample MCP server: no window, no database, just stdio.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == sovra_runtime::SAMPLE_SERVER_FLAG)
    {
        return sovra_runtime::run_sample_mcp_server().context("sample MCP server failed");
    }
    let development = sovra_runtime::DevelopmentConfig {
        // Watched development is observable by default; installed/release binaries
        // always start with control off, regardless of a retained dev preference.
        startup_enabled: Some(
            cfg!(debug_assertions)
                && std::env::var_os("SOVRA_DEV").is_some_and(|value| value == "1"),
        ),
        directory: development_directory,
        executable: std::env::current_exe().context("could not resolve Sovra executable")?,
    };
    let _log_guard = logging::init(&data_dir.as_path().join("logs"))?;
    tracing::info!(version = VERSION, "starting SovraHarness");
    let mut session_namespace = [0_u8; 16];
    getrandom::fill(&mut session_namespace)
        .context("could not create unique conversation identities")?;

    let mut agent_config = AgentConfig::with_api_key(read_gemini_api_key());
    agent_config.searxng_base_url = read_searxng_base_url();
    // The composition root picks the memory backend; the runtime opens it on its own
    // executor. Swapping to a remote server (or another backend) happens here only.
    let memory_dir = data_dir.as_path().join("memory");
    let memory: sovra_runtime::MemoryFactory = Box::new(move || {
        Box::pin(async move {
            let store = SurrealStore::open(StoreConfig::on_disk(memory_dir)).await?;
            Ok(Arc::new(store) as Arc<dyn MemoryStore>)
        })
    });
    let model_repository = sovra_providers::ModelRepository::new(
        data_dir.as_path().join("models"),
        credential_store(&data_dir),
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
    let whatsapp = sovra_whatsapp::WhatsAppChannel::new(data_dir.as_path().join("whatsapp"));
    // The Telegram bot token is an API key: it goes where model keys go (keychain, or the
    // SOVRA_DEV plaintext files in debug builds).
    let telegram = sovra_telegram::TelegramChannel::new(Arc::new(KeychainTokenStore(
        credential_store(&data_dir),
    )))
    .map_err(|error| tracing::warn!(%error, "telegram linking unavailable"))
    .ok();
    let google = sovra_google::GoogleAdapter::new(
        read_google_sources(),
        Arc::new(CredentialVault(credential_store(&data_dir))),
    )
    .context("failed to prepare Google connections")?;
    let github = sovra_github::GitHubAdapter::new(
        read_github_client_id(),
        sovra_github::Endpoints::default(),
        Arc::new(CredentialVault(credential_store(&data_dir))),
    )
    .context("failed to prepare GitHub connections")?;
    let client = sovra_runtime::start_with_services(
        VERSION,
        agent_config,
        sovra_runtime::RuntimeServices {
            training: Some(sovra_runtime::TrainingConfig {
                engine: Arc::new(sovra_train::BurnTrainingEngine),
                directory: data_dir.as_path().join("training"),
            }),
            memory: Some(memory),
            memory_extraction_slots: read_memory_extraction_slots(),
            models: Some(models),
            files: Some(Arc::new(sovra_files::LocalFileStore::new(
                data_dir.as_path().join("attachments"),
            ))),
            skills: Some(Arc::new(sovra_skills::LocalSkillStore::new(
                data_dir.as_path().join("skills"),
            ))),
            index: Some(Arc::new(sovra_files::LocalFileIndex::new(
                data_dir.as_path().join("file-index"),
            ))),
            messaging: sovra_runtime::MessagingChannels {
                whatsapp: Some(Arc::new(whatsapp)),
                telegram: telegram.map(|channel| {
                    Arc::new(channel) as Arc<dyn sovra_core::connections::MessagingChannel>
                }),
            },
            // Continue with Google reads the adapter: saved client, else
            // environment, else baked. A separate startup config would disagree.
            signin: None,
            google: Some(Arc::new(google)),
            github: Some(Arc::new(github)),
            development: Some(development),
            remote_control: remote_services(&data_dir),
            probes: Vec::new(),
            terminal: Some(terminal_config(data_dir.as_path())),
            mcp_sample_program: std::env::current_exe().ok(),
            mcp_catalogue: Some(
                data_dir
                    .as_path()
                    .join("connected-services")
                    .join("services.json"),
            ),
            groups: group_services(&data_dir),
            schedules: Some(Arc::new(sovra_runtime::JsonScheduleStore::new(
                data_dir.as_path().join("schedules.json"),
            ))),
            schedule_search: None,
        },
    )
    .context("failed to start the runtime")?;
    sovra_ui::run(client.clone(), u128::from_be_bytes(session_namespace));
    client
        .shutdown_blocking()
        .context("runtime did not finish shutting down")?;

    tracing::info!("exiting");
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the composition root reads background capacity once at startup"
)]
fn read_memory_extraction_slots() -> usize {
    std::env::var("SOVRA_MEMORY_EXTRACTION_SLOTS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(sovra_core::DEFAULT_MEMORY_EXTRACTION_SLOTS)
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

/// Terminal settings live beside the other app data; commands start in the person's
/// home folder, in their own shell.
#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
fn terminal_config(data_dir: &std::path::Path) -> sovra_runtime::TerminalConfig {
    let working_dir = directories::BaseDirs::new().map_or_else(
        || data_dir.to_path_buf(),
        |base| base.home_dir().to_path_buf(),
    );
    sovra_runtime::TerminalConfig {
        settings_path: data_dir.join("terminal").join("settings.json"),
        working_dir,
        login_shell: std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.trim().is_empty()),
        search_path: std::env::var_os("PATH"),
        use_login_env: true,
    }
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

/// The Google Cloud "Desktop app" OAuth client. A client id saved in Settings
/// wins. Otherwise the environment overrides a value baked in at compile time
/// (`SOVRA_GOOGLE_CLIENT_ID` / `_SECRET`). A desktop-app secret is not
/// confidential (RFC 8252 §8.5); PKCE protects the flow either way.
#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
fn read_google_sources() -> sovra_google::ClientSources {
    let from_env = |id_name: &str, secret_name: &str| {
        client_config(std::env::var(id_name).ok(), std::env::var(secret_name).ok())
    };
    sovra_google::ClientSources {
        environment: from_env("SOVRA_GOOGLE_CLIENT_ID", "SOVRA_GOOGLE_CLIENT_SECRET"),
        baked: client_config(
            option_env!("SOVRA_GOOGLE_CLIENT_ID").map(str::to_owned),
            option_env!("SOVRA_GOOGLE_CLIENT_SECRET").map(str::to_owned),
        ),
    }
}

/// The public client id of Sovra's GitHub OAuth App (device flow on). The
/// environment overrides a value baked in at compile time (`SOVRA_GITHUB_CLIENT_ID`).
/// Without one, Connect GitHub walks the person through pasting a key instead.
#[expect(
    clippy::disallowed_methods,
    reason = "sovra-app is the composition root and reads process environment variables once at startup"
)]
fn read_github_client_id() -> Option<String> {
    std::env::var("SOVRA_GITHUB_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("SOVRA_GITHUB_CLIENT_ID").map(str::to_owned))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
}

fn client_config(id: Option<String>, secret: Option<String>) -> Option<sovra_google::ClientConfig> {
    let client_id = id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;
    let client_secret = secret
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    Some(sovra_google::ClientConfig {
        client_id,
        client_secret,
    })
}

/// Google and GitHub sign-ins share the model keys' store (keychain, or dev files).
struct CredentialVault(Arc<dyn sovra_providers::CredentialStore>);

impl sovra_core::secrets::SecretVault for CredentialVault {
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

/// The keychain entry holding this computer's group device key (32 bytes, hex).
const GROUP_DEVICE_KEY: &str = "group-device-key";

/// This computer's device key: read from the credential store, or made once and kept
/// there. Groups stay off rather than keep the key anywhere less safe.
fn device_key(credentials: &dyn sovra_providers::CredentialStore, entry: &str) -> Option<[u8; 32]> {
    // A locked or damaged credential store must never rotate a remembered identity.
    let secret = credentials.get(entry).ok()?;
    if !secret.0.is_empty() {
        return sovra_core::group::DeviceId::from_hex(secret.0.trim())
            .ok()
            .map(|id| id.0);
    }
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        tracing::warn!("device unavailable: no secure randomness for a device key");
        return None;
    }
    let hex = sovra_core::group::DeviceId(bytes).to_hex();
    match credentials.set(entry, &sovra_core::models::ModelSecret(hex)) {
        Ok(()) => Some(bytes),
        Err(error) => {
            tracing::warn!(%error, "device unavailable: the device key could not be kept safely");
            None
        }
    }
}

/// Groups: the device key, where the group record lives, and how far the network
/// reaches (`SOVRA_GROUP_REACH=local` keeps it to direct, same-network connections).
fn group_services(data_dir: &std::path::Path) -> Option<sovra_runtime::GroupServices> {
    let secret = device_key(credential_store(data_dir).as_ref(), GROUP_DEVICE_KEY)?;
    let reach = if std::env::var_os("SOVRA_GROUP_REACH").is_some_and(|value| value == "local") {
        sovra_mesh::Reach::LocalOnly
    } else {
        sovra_mesh::Reach::Internet
    };
    Some(sovra_runtime::GroupServices {
        transport: Box::new(move || {
            Box::pin(async move {
                sovra_mesh::Mesh::bind(secret, reach)
                    .await
                    .map(|mesh| Arc::new(mesh) as Arc<dyn sovra_core::group::GroupTransport>)
                    .map_err(|error| error.to_string())
            })
        }),
        store: Arc::new(sovra_mesh::FileGroupStore::new(data_dir.join("groups"))),
        // The WhatsApp adapter owns the QR library; invite links reuse its encoder.
        qr: Some(Arc::new(sovra_whatsapp::qr_matrix)),
        helper_model: None,
    })
}

/// Phone access has its own key and lifetime, independent of Groups and MCP.
fn remote_services(data_dir: &std::path::Path) -> Option<sovra_runtime::RemoteConfig> {
    let secret = device_key(
        credential_store(data_dir).as_ref(),
        "remote-control-device-key",
    )?;
    Some(sovra_runtime::RemoteConfig {
        directory: data_dir.join("remote-control"),
        transport: remote_transport(secret),
        qr: Arc::new(sovra_whatsapp::qr_matrix),
    })
}

fn remote_transport(secret: [u8; 32]) -> sovra_runtime::RemoteEndpointFactory {
    let reach = if std::env::var_os("SOVRA_REMOTE_REACH").is_some_and(|value| value == "local") {
        sovra_mesh::Reach::LocalOnly
    } else {
        sovra_mesh::Reach::Internet
    };
    Arc::new(move || {
        Box::pin(async move {
            sovra_mesh::RemoteMesh::bind(secret, reach)
                .await
                .map(|endpoint| {
                    Arc::new(endpoint) as Arc<dyn sovra_core::remote_control::RemoteEndpoint>
                })
                .map_err(|error| error.to_string())
        })
    })
}

/// Where this install keeps its data. A development build with `SOVRA_DEV=1` may
/// point `SOVRA_DATA_DIR` elsewhere so two Sovra computers can run side by side
/// (each with its own device key and group); release builds always use the OS default.
fn data_directory(dirs: &directories::ProjectDirs) -> std::path::PathBuf {
    if cfg!(debug_assertions)
        && std::env::var_os("SOVRA_DEV").is_some_and(|value| value == "1")
        && let Some(dir) = std::env::var_os("SOVRA_DATA_DIR").filter(|dir| !dir.is_empty())
    {
        return std::path::PathBuf::from(dir);
    }
    dirs.data_local_dir().to_path_buf()
}

/// `cargo xtask dev` sets `SOVRA_DEV`: every rebuild changes the binary's code
/// signature, which makes macOS re-prompt for the keychain password each reload.
fn credential_store(data_dir: &std::path::Path) -> Arc<dyn sovra_providers::CredentialStore> {
    if cfg!(debug_assertions) && std::env::var_os("SOVRA_DEV").is_some_and(|value| value == "1") {
        tracing::warn!("SOVRA_DEV set: model keys use plaintext dev files, not the keychain");
        Arc::new(sovra_providers::FileCredentialStore::new(
            data_dir.join("dev-credentials"),
        ))
    } else {
        Arc::new(sovra_providers::OsCredentialStore)
    }
}
