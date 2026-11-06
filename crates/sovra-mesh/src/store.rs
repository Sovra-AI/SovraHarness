//! The group record on disk: one JSON file, replaced atomically.
//!
//! The record is small (a group's signed history) and written rarely, so plain file
//! IO is enough. A temporary file plus rename means a crash mid-write never leaves a
//! half-written record, and on Unix the file is readable by its owner only.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use sovra_core::group::{GroupRecord, GroupStore, GroupStoreError};

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
}
