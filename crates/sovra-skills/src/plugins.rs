//! Private, atomic persistence for downloaded declarative plugins. The same
//! bounded file helpers as imported skills keep package data out of executables.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, atomic::Ordering};

use sovra_core::plugins::{
    MAX_PLUGIN_LIBRARY_BYTES, PluginEntry, PluginLibrary, PluginPackage, PluginSkill, PluginStore,
    package_limit, repository_skill,
};
use sovra_core::skills::SkillError;

use crate::{NEXT_WRITE, io_error, private_directory, private_write, read_bytes};

#[derive(Debug)]
pub struct LocalPluginStore {
    directory: PathBuf,
    writes: Mutex<()>,
}

impl LocalPluginStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            writes: Mutex::new(()),
        }
    }
}

impl PluginStore for LocalPluginStore {
    fn load(&self) -> Result<PluginLibrary, SkillError> {
        private_directory(&self.directory)?;
        let path = self.directory.join("plugins.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PluginLibrary::default());
            }
            Err(error) => return Err(io_error(&error)),
            Ok(_) => {}
        }
        let bytes = read_bytes(&path, MAX_PLUGIN_LIBRARY_BYTES)?;
        let library: PluginLibrary = serde_json::from_slice(&bytes).map_err(|_| {
            SkillError(
                "The saved plugin library could not be read. Its contents have been preserved."
                    .into(),
            )
        })?;
        library.validate()?;
        Ok(library)
    }

    fn package(&self, entry: &PluginEntry) -> Result<PluginPackage, SkillError> {
        // Validate the entry before using its checksum as a path component.
        entry.validate()?;
        private_directory(&self.directory)?;
        let bytes = read_bytes(
            &self.directory.join(format!("{}.json", entry.sha256)),
            package_limit(&entry.package_url),
        )?;
        entry.verify_package(&bytes)
    }

    fn cache(&self, entry: &PluginEntry, bytes: &[u8]) -> Result<(), SkillError> {
        entry.verify_package(bytes)?;
        private_directory(&self.directory)?;
        let path = self.directory.join(format!("{}.json", entry.sha256));
        if path.try_exists().map_err(|error| io_error(&error))? {
            self.package(entry)?;
            return Ok(());
        }
        // Immutable content-addressed files can be shared by installations and
        // retained across updates; only the small index is rewritten.
        self.write(&path, bytes)
    }

    fn remove_unused(&self, library: &PluginLibrary) -> Result<usize, SkillError> {
        // Same lock as `save`, so the index and the files it names change in order.
        let _guard = self
            .writes
            .lock()
            .map_err(|_| SkillError("Plugin storage is unavailable.".into()))?;
        private_directory(&self.directory)?;
        let keep = library.package_files();
        let mut removed = 0;
        for item in fs::read_dir(&self.directory).map_err(|error| io_error(&error))? {
            let item = item.map_err(|error| io_error(&error))?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // Only regular files shaped like a package are candidates; a link or folder
            // with that name was not written by this store.
            let regular = item.file_type().is_ok_and(|kind| kind.is_file());
            if regular && PluginLibrary::is_package_file(&name) && !keep.contains(&name) {
                fs::remove_file(item.path()).map_err(|error| io_error(&error))?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn read_skill_file(
        &self,
        folder: Option<&str>,
        content: &str,
    ) -> Result<PluginSkill, SkillError> {
        // Same parser as imported skill folders. Tool grants in `allowed-tools` are
        // dropped: plugin skills are instructions only.
        let skill = crate::format::decode_named(content, folder)?;
        repository_skill(
            skill.name,
            &skill.title,
            &skill.description,
            skill.instructions,
        )
    }

    fn save(&self, library: &PluginLibrary) -> Result<(), SkillError> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| SkillError("Plugin storage is unavailable.".into()))?;
        library.validate()?;
        let bytes = serde_json::to_vec(library)
            .map_err(|_| SkillError("The plugin library could not be saved.".into()))?;
        if bytes.len() > MAX_PLUGIN_LIBRARY_BYTES {
            return Err(SkillError(
                "The installed plugin library exceeds 8 MB.".into(),
            ));
        }
        private_directory(&self.directory)?;
        let target = self.directory.join("plugins.json");
        if target
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(SkillError(
                "The plugin library cannot be a symbolic link.".into(),
            ));
        }
        self.write(&target, &bytes)
    }
}

impl LocalPluginStore {
    fn write(&self, target: &std::path::Path, bytes: &[u8]) -> Result<(), SkillError> {
        let staging = self.directory.join(format!(
            ".write-{}-{}",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            private_write(&staging, bytes)?;
            fs::rename(&staging, target).map_err(|error| io_error(&error))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result
    }
}
