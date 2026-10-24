use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sovra_core::models::{
    CatalogSnapshot, ModelConnection, ModelError, ModelPreferences, ModelSecret, ProviderKind,
};

/// Sync by design: the runtime executes keychain and disk work in `spawn_blocking`.
pub trait CredentialStore: Send + Sync {
    fn get(&self, id: &str) -> Result<ModelSecret, ModelError>;
    fn set(&self, id: &str, secret: &ModelSecret) -> Result<(), ModelError>;
    fn delete(&self, id: &str) -> Result<(), ModelError>;
}

#[derive(Debug, Default)]
pub struct OsCredentialStore;

impl CredentialStore for OsCredentialStore {
    fn get(&self, id: &str) -> Result<ModelSecret, ModelError> {
        let entry = keyring::Entry::new("com.Sovra.SovraHarness.models", id)
            .map_err(|_| credential_error())?;
        match entry.get_password() {
            Ok(secret) => Ok(ModelSecret(secret)),
            Err(keyring::Error::NoEntry) => Ok(ModelSecret::default()),
            Err(_) => Err(credential_error()),
        }
    }
    fn set(&self, id: &str, secret: &ModelSecret) -> Result<(), ModelError> {
        keyring::Entry::new("com.Sovra.SovraHarness.models", id)
            .map_err(|_| credential_error())?
            .set_password(&secret.0)
            .map_err(|_| credential_error())
    }
    fn delete(&self, id: &str) -> Result<(), ModelError> {
        let entry = keyring::Entry::new("com.Sovra.SovraHarness.models", id)
            .map_err(|_| credential_error())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(credential_error()),
        }
    }
}

/// Dev-only store: keys live in 0600 files so `cargo xtask dev` rebuilds never
/// re-trigger the macOS keychain password prompt (its ACL is bound to the binary's
/// code signature, which changes on every build). Never selected in release builds.
#[derive(Debug)]
pub struct FileCredentialStore {
    directory: PathBuf,
}

impl FileCredentialStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn path(&self, id: &str) -> Result<PathBuf, ModelError> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(ModelError("Invalid connection identifier.".into()));
        }
        Ok(self.directory.join(id))
    }
}

impl CredentialStore for FileCredentialStore {
    fn get(&self, id: &str) -> Result<ModelSecret, ModelError> {
        match std::fs::read_to_string(self.path(id)?) {
            Ok(secret) => Ok(ModelSecret(secret)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ModelSecret::default()),
            Err(_) => Err(ModelError("Could not read the dev credential file.".into())),
        }
    }
    fn set(&self, id: &str, secret: &ModelSecret) -> Result<(), ModelError> {
        let path = self.path(id)?;
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.directory)?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))?;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                // OpenOptions::mode only applies to new files. Tighten an existing
                // file before replacing its contents too.
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.set_len(0)?;
            file.write_all(secret.0.as_bytes())?;
            file.sync_all()
        };
        write().map_err(|_| ModelError("Could not save the dev credential file.".into()))
    }
    fn delete(&self, id: &str) -> Result<(), ModelError> {
        match std::fs::remove_file(self.path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(ModelError(
                "Could not remove the dev credential file.".into(),
            )),
        }
    }
}

fn credential_error() -> ModelError {
    ModelError("Sovra couldn't open this computer's keychain. Unlock it and try again. Your key was not saved anywhere else.".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedModels {
    pub version: u32,
    pub connections: Vec<ModelConnection>,
    pub preferences: ModelPreferences,
}

impl Default for SavedModels {
    fn default() -> Self {
        Self {
            version: 1,
            connections: vec![
                ModelConnection::new("openrouter", ProviderKind::OpenRouter),
                ModelConnection::new("ollama", ProviderKind::Ollama),
            ],
            preferences: ModelPreferences::default(),
        }
    }
}

#[derive(Clone)]
pub struct ModelRepository {
    directory: PathBuf,
    credentials: Arc<dyn CredentialStore>,
    environment_keys: BTreeMap<String, ModelSecret>,
}

impl std::fmt::Debug for ModelRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRepository")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl ModelRepository {
    pub fn new(
        directory: PathBuf,
        credentials: Arc<dyn CredentialStore>,
        environment_keys: BTreeMap<String, ModelSecret>,
    ) -> Self {
        Self {
            directory,
            credentials,
            environment_keys,
        }
    }

    pub fn load(&self) -> Result<SavedModels, ModelError> {
        let file = self.directory.join("connections.json");
        match std::fs::read(&file) {
            Ok(bytes) => {
                let saved: SavedModels = serde_json::from_slice(&bytes).map_err(|_| ModelError("Saved model connections could not be read. The original file has been preserved.".into()))?;
                if saved.version != 1 {
                    return Err(ModelError(
                        "These model settings were saved by a newer app version.".into(),
                    ));
                }
                Ok(saved)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SavedModels::default()),
            Err(_) => Err(ModelError("Could not read saved model connections.".into())),
        }
    }

    pub fn save(&self, saved: &SavedModels) -> Result<(), ModelError> {
        atomic_json(&self.directory.join("connections.json"), saved)
    }

    pub fn credential(&self, connection: &ModelConnection) -> Result<ModelSecret, ModelError> {
        if connection.has_credential {
            return self.credentials.get(&connection.id);
        }
        // Environment keys belong to the built-in endpoint, never a user-edited
        // destination bearing the same ID.
        if self.has_environment_key(connection) {
            Ok(self
                .environment_keys
                .get(&connection.id)
                .cloned()
                .unwrap_or_default())
        } else {
            Ok(ModelSecret::default())
        }
    }

    pub fn has_environment_key(&self, connection: &ModelConnection) -> bool {
        matches!(
            (connection.id.as_str(), connection.kind),
            ("openrouter", ProviderKind::OpenRouter) | ("ollama", ProviderKind::Ollama)
        ) && connection.base_url == connection.kind.default_url()
            && self
                .environment_keys
                .get(&connection.id)
                .is_some_and(|k| !k.0.is_empty())
    }

    pub fn set_credential(&self, id: &str, secret: &ModelSecret) -> Result<(), ModelError> {
        self.credentials.set(id, secret)
    }

    pub fn delete_credential(&self, id: &str) -> Result<(), ModelError> {
        self.credentials.delete(id)
    }

    pub fn load_catalog(&self, id: &str) -> Result<Option<CatalogSnapshot>, ModelError> {
        let file = self.catalog_path(id)?;
        match std::fs::read(file) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(ModelError("Could not read the cached catalog.".into())),
        }
    }

    pub fn save_catalog(&self, id: &str, snapshot: &CatalogSnapshot) -> Result<(), ModelError> {
        atomic_json(&self.catalog_path(id)?, snapshot)
    }

    pub fn delete_catalog(&self, id: &str) -> Result<(), ModelError> {
        match std::fs::remove_file(self.catalog_path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(ModelError("Could not remove the cached catalog.".into())),
        }
    }

    fn catalog_path(&self, id: &str) -> Result<PathBuf, ModelError> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(ModelError("Invalid connection identifier.".into()));
        }
        Ok(self.directory.join("catalogs").join(format!("{id}.json")))
    }
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), ModelError> {
    let parent = path
        .parent()
        .ok_or_else(|| ModelError("Invalid settings path.".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| ModelError("Could not create model settings directory.".into()))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| ModelError("Could not serialize model settings.".into()))?;
    let temporary = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let write = || -> std::io::Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    };
    write().map_err(|_| {
        ModelError("Could not save model settings. Existing settings were preserved.".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_credentials_roundtrip_without_exposing_values_or_accepting_paths() {
        let path = std::env::temp_dir().join(format!("sovra-dev-key-test-{}", std::process::id()));
        let store = FileCredentialStore::new(path.clone());
        assert_eq!(store.get("openrouter").unwrap().0, "");
        store
            .set("openrouter", &ModelSecret("test-value".into()))
            .unwrap();
        assert_eq!(store.get("openrouter").unwrap().0, "test-value");
        assert!(!format!("{store:?}").contains("test-value"));
        for id in ["", "../escape", "/absolute", "other/key"] {
            assert!(store.get(id).is_err());
            assert!(store.set(id, &ModelSecret("test-value".into())).is_err());
            assert!(store.delete(id).is_err());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                path.join("openrouter"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            store
                .set("openrouter", &ModelSecret("replacement".into()))
                .unwrap();
            assert_eq!(
                std::fs::metadata(path.join("openrouter"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        store.delete("openrouter").unwrap();
        store.delete("openrouter").unwrap();
        assert_eq!(store.get("openrouter").unwrap().0, "");
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn preferences_saved_before_appearance_existed_load_as_dark() {
        use sovra_core::models::Appearance;
        let old: ModelPreferences =
            serde_json::from_str(r#"{"favorites":[],"active":null}"#).unwrap();
        assert_eq!(old.appearance, Appearance::Dark);
        let light = ModelPreferences {
            appearance: Appearance::Light,
            ..ModelPreferences::default()
        };
        let round: ModelPreferences =
            serde_json::from_str(&serde_json::to_string(&light).unwrap()).unwrap();
        assert_eq!(round.appearance, Appearance::Light);
    }
    #[derive(Default)]
    struct MemoryCredentials(std::sync::Mutex<BTreeMap<String, ModelSecret>>);
    impl CredentialStore for MemoryCredentials {
        fn get(&self, id: &str) -> Result<ModelSecret, ModelError> {
            Ok(self.0.lock().unwrap().get(id).cloned().unwrap_or_default())
        }
        fn set(&self, id: &str, secret: &ModelSecret) -> Result<(), ModelError> {
            self.0.lock().unwrap().insert(id.into(), secret.clone());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), ModelError> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }
    #[test]
    fn environment_credentials_are_bound_to_protocol_id_and_endpoint() {
        let repo = ModelRepository::new(
            PathBuf::new(),
            Arc::new(MemoryCredentials::default()),
            [(
                "openrouter".into(),
                ModelSecret("environment-secret".into()),
            )]
            .into(),
        );
        let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
        assert!(repo.has_environment_key(&connection));
        assert_eq!(
            repo.credential(&connection).unwrap().0,
            "environment-secret"
        );
        connection.base_url = "https://different.example/api/v1".into();
        assert!(!repo.has_environment_key(&connection));
        assert_eq!(repo.credential(&connection).unwrap().0, "");
        let connection = ModelConnection::new("openrouter", ProviderKind::OpenAiCompatible);
        assert!(!repo.has_environment_key(&connection));
        assert_eq!(repo.credential(&connection).unwrap().0, "");
        let connection = ModelConnection::new("custom", ProviderKind::OpenRouter);
        assert!(!repo.has_environment_key(&connection));
    }

    #[test]
    fn settings_and_catalog_roundtrip_without_plaintext_credentials() {
        let path =
            std::env::temp_dir().join(format!("sovra-model-storage-test-{}", std::process::id()));
        let repo = ModelRepository::new(
            path.clone(),
            Arc::new(MemoryCredentials::default()),
            BTreeMap::new(),
        );
        let mut saved = SavedModels::default();
        saved.connections[0].has_credential = true;
        repo.set_credential("openrouter", &ModelSecret("super-secret".into()))
            .unwrap();
        repo.save(&saved).unwrap();
        assert!(
            !std::fs::read_to_string(path.join("connections.json"))
                .unwrap()
                .contains("super-secret")
        );
        assert_eq!(repo.load().unwrap().connections, saved.connections);
        assert_eq!(
            repo.credential(&saved.connections[0]).unwrap().0,
            "super-secret"
        );
        let snapshot = CatalogSnapshot {
            models: vec![],
            fetched_at: 99,
        };
        repo.save_catalog("openrouter", &snapshot).unwrap();
        assert_eq!(repo.load_catalog("openrouter").unwrap(), Some(snapshot));
        assert!(repo.load_catalog("../escape").is_err());
        repo.delete_catalog("openrouter").unwrap();
        repo.delete_credential("openrouter").unwrap();
        assert_eq!(repo.load_catalog("openrouter").unwrap(), None);
        assert_eq!(repo.credential(&saved.connections[0]).unwrap().0, "");
        std::fs::remove_dir_all(path).unwrap();
    }
}
