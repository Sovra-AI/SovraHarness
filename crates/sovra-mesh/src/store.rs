//! Group records on disk: one JSON file per group, each replaced atomically.
//!
//! The record is small (a group's signed history) and written rarely, so plain file
//! IO is enough. A temporary file plus rename means a crash mid-write never leaves a
//! half-written record, and on Unix the file is readable by its owner only.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use std::sync::Arc;

use sovra_core::group::{GroupRecord, GroupStore, GroupStoreError, GroupStores};

/// Saves the group record as `<dir>/group.json`.
#[derive(Debug, Clone)]
pub struct FileGroupStore {
    path: PathBuf,
}

impl FileGroupStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            path: dir.into().join("group.json"),
        }
    }

    /// The record kept in `<dir>/<name>`, for a computer's further groups.
    fn named(dir: &std::path::Path, name: &str) -> Self {
        Self {
            path: dir.join(name),
        }
    }

    fn read(&self) -> Result<GroupRecord, GroupStoreError> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| GroupStoreError::Read(error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(GroupRecord::default())
            }
            Err(error) => Err(GroupStoreError::Read(error.to_string())),
        }
    }

    fn write(&self, record: &GroupRecord) -> Result<(), GroupStoreError> {
        let write = |error: std::io::Error| GroupStoreError::Write(error.to_string());
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(write)?;
        }
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| GroupStoreError::Write(error.to_string()))?;
        let temporary = self.path.with_extension("json.tmp");
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(write)?;
        file.write_all(&bytes).map_err(write)?;
        file.sync_all().map_err(write)?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(write)
    }
}

#[async_trait::async_trait]
impl GroupStore for FileGroupStore {
    async fn load(&self) -> Result<GroupRecord, GroupStoreError> {
        self.read()
    }

    async fn save(&self, record: &GroupRecord) -> Result<(), GroupStoreError> {
        self.write(record)
    }

    async fn remove(&self) -> Result<(), GroupStoreError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(GroupStoreError::Write(error.to_string())),
        }
    }
}

/// A file name for one more group: `group-<32 hex digits>.json`.
const MORE_PREFIX: &str = "group-";
const MORE_SUFFIX: &str = ".json";

/// The records of a computer's further groups, as `<dir>/group-<random>.json`
/// beside the first group's `group.json`.
#[derive(Debug, Clone)]
pub struct FileGroupStores {
    dir: PathBuf,
}

impl FileGroupStores {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Whether `name` is a further group's record and nothing else in the folder
    /// (the first group, a half-written temporary file, anything foreign).
    fn is_record(name: &str) -> bool {
        name.strip_prefix(MORE_PREFIX)
            .and_then(|rest| rest.strip_suffix(MORE_SUFFIX))
            .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
    }
}

#[async_trait::async_trait]
impl GroupStores for FileGroupStores {
    async fn existing(&self) -> Vec<Arc<dyn GroupStore>> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| Self::is_record(name))
            .collect();
        // The same order on every start, so the list of groups does not shuffle.
        names.sort();
        names
            .iter()
            .map(|name| Arc::new(FileGroupStore::named(&self.dir, name)) as Arc<dyn GroupStore>)
            .collect()
    }

    fn fresh(&self) -> Option<Arc<dyn GroupStore>> {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).ok()?;
        let hex: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        Some(Arc::new(FileGroupStore::named(
            &self.dir,
            &format!("{MORE_PREFIX}{hex}{MORE_SUFFIX}"),
        )))
    }
}
