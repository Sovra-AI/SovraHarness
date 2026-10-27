//! Private local skill packages. Parsing and filesystem IO are bounded and run on
//! runtime-owned blocking workers; resources are data, never executable authority.

mod format;
#[cfg(test)]
mod tests;

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use sovra_core::skills::{
    MAX_SKILL_BYTES, MAX_SKILLS, Skill, SkillCatalog, SkillError, SkillListing, SkillStore,
    valid_name,
};

const MAX_RESOURCE_BYTES: usize = 128 * 1024;
const MAX_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 128;
static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct LocalSkillStore {
    directory: PathBuf,
    writes: Mutex<()>,
}

impl LocalSkillStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            writes: Mutex::new(()),
        }
    }

    fn root(&self) -> Result<PathBuf, SkillError> {
        private_directory(&self.directory)?;
        self.directory
            .canonicalize()
            .map_err(|error| io_error(&error))
    }

    fn location(&self, name: &str) -> Result<PathBuf, SkillError> {
        if !valid_name(name) || SkillCatalog::default().find(name).is_some() {
            return Err(SkillError(
                "Choose a valid skill name that is not a built-in command or alias.".into(),
            ));
        }
        let path = self.root()?.join(name);
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(SkillError("Skill folders cannot be symbolic links.".into()));
        }
        Ok(path)
    }
}

impl SkillStore for LocalSkillStore {
    fn list(&self) -> Result<SkillListing, SkillError> {
        let root = self.root()?;
        let mut result = SkillListing::default();
        let mut entries = fs::read_dir(&root)
            .map_err(|error| io_error(&error))?
            .take(MAX_SKILLS + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_error(&error))?;
        if entries.len() > MAX_SKILLS {
            return Err(SkillError(format!(
                "The skills folder exceeds {MAX_SKILLS} entries."
            )));
        }
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let loaded = (|| {
                let directory = self.location(&name)?;
                let content = read_text(&directory.join("SKILL.md"), MAX_SKILL_BYTES)?;
                let skill = format::decode(&content, &name)?;
                let mut candidates = sovra_core::skills::builtin_skills();
                candidates.extend(result.skills.clone());
                candidates.push(skill.clone());
                SkillCatalog::new(candidates)?;
                Ok::<_, SkillError>(skill)
            })();
            match loaded {
                Ok(skill) => result.skills.push(skill),
                Err(error) => result.issues.push(format!("{name}: {error}")),
            }
        }
        Ok(result)
    }

    fn save(&self, skill: &Skill) -> Result<(), SkillError> {
        let _guard = self.writes.lock().map_err(|_| {
            SkillError("Skill storage is busy after an interrupted write. Restart Sovra.".into())
        })?;
        if skill.builtin {
            return Err(SkillError("Built-in skills cannot be overwritten.".into()));
        }
        let mut candidates = sovra_core::skills::builtin_skills();
        candidates.extend(
            self.list()?
                .skills
                .into_iter()
                .filter(|existing| existing.name != skill.name),
        );
        candidates.push(skill.clone());
        SkillCatalog::new(candidates)?;
        let content = format::encode(skill)?;
        let directory = self.location(&skill.name)?;
        private_directory(&directory)?;
        let staging = directory.join(format!(
            ".write-{}-{}",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            private_write(&staging, content.as_bytes())?;
            fs::rename(&staging, directory.join("SKILL.md")).map_err(|error| io_error(&error))
        })();
        if result.is_err() {
            let _ = fs::remove_file(staging);
        }
        result
    }

    fn import(&self, directory: &Path) -> Result<Skill, SkillError> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| SkillError("Skill storage is unavailable.".into()))?;
        let name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SkillError("Select the folder containing SKILL.md.".into()))?;
        reject_symlink(directory)?;
        let skill = format::decode(
            &read_text(&directory.join("SKILL.md"), MAX_SKILL_BYTES)?,
            name,
        )?;
        let destination = self.location(name)?;
        if destination.exists() {
            return Err(SkillError(
                "A skill with this name is already installed. Edit it or choose another name."
                    .into(),
            ));
        }
        let mut candidates = sovra_core::skills::builtin_skills();
        candidates.extend(self.list()?.skills);
        candidates.push(skill.clone());
        SkillCatalog::new(candidates)?;
        let staging = self.root()?.join(format!(
            ".import-{}-{}",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        private_directory(&staging)?;
        let result = copy_package(directory, &staging)
            .and_then(|()| fs::rename(&staging, &destination).map_err(|error| io_error(&error)));
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result?;
        Ok(skill)
    }

    fn remove(&self, name: &str) -> Result<(), SkillError> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| SkillError("Skill storage is unavailable.".into()))?;
        let directory = self.location(name)?;
        match fs::remove_dir_all(directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(&error)),
        }
    }

    fn resource(&self, name: &str, relative_path: &str) -> Result<String, SkillError> {
        let mut path = self.location(name)?;
        let relative = Path::new(relative_path);
        if relative_path.is_empty() || relative_path.len() > 1024 || relative_path.contains('\\') {
            return Err(SkillError(
                "Choose a relative resource path inside this skill.".into(),
            ));
        }
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(SkillError(
                    "Resource paths must stay inside the skill folder.".into(),
                ));
            };
            if name.to_string_lossy().starts_with('.') {
                return Err(SkillError("Hidden skill resources are not exposed.".into()));
            }
            path.push(name);
            reject_symlink(&path)?;
        }
        read_text(&path, MAX_RESOURCE_BYTES)
    }
}

fn copy_package(source: &Path, destination: &Path) -> Result<(), SkillError> {
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf(), 0)];
    let mut entries = 0;
    let mut bytes = 0;
    while let Some((source, destination, depth)) = pending.pop() {
        for entry in fs::read_dir(source).map_err(|error| io_error(&error))? {
            let entry = entry.map_err(|error| io_error(&error))?;
            entries += 1;
            if entries > MAX_PACKAGE_FILES {
                return Err(SkillError(
                    "A skill package may contain at most 128 entries.".into(),
                ));
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.')
                || ["target", "node_modules", "__pycache__"]
                    .iter()
                    .any(|excluded| name == *excluded)
            {
                continue;
            }
            let kind = entry.file_type().map_err(|error| io_error(&error))?;
            if kind.is_symlink() {
                return Err(SkillError(
                    "Skill packages cannot contain symbolic links.".into(),
                ));
            }
            let target = destination.join(name);
            if kind.is_dir() {
                if depth >= 4 {
                    return Err(SkillError(
                        "Skill resource folders may be at most four levels deep.".into(),
                    ));
                }
                private_directory(&target)?;
                pending.push((entry.path(), target, depth + 1));
            } else if kind.is_file() {
                let content = read_bytes(&entry.path(), MAX_PACKAGE_BYTES - bytes)?;
                bytes += content.len();
                private_write(&target, &content)?;
            } else {
                return Err(SkillError(
                    "Skill packages may contain only regular files and folders.".into(),
                ));
            }
        }
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), SkillError> {
    if fs::symlink_metadata(path)
        .map_err(|error| io_error(&error))?
        .file_type()
        .is_symlink()
    {
        return Err(SkillError(
            "Symbolic links are not allowed in skill packages.".into(),
        ));
    }
    Ok(())
}

fn read_text(path: &Path, limit: usize) -> Result<String, SkillError> {
    String::from_utf8(read_bytes(path, limit)?)
        .map_err(|_| SkillError("This skill resource is not UTF-8 text.".into()))
}

fn read_bytes(path: &Path, limit: usize) -> Result<Vec<u8>, SkillError> {
    reject_symlink(path)?;
    if !fs::metadata(path)
        .map_err(|error| io_error(&error))?
        .is_file()
    {
        return Err(SkillError(
            "The skill resource is not a regular file.".into(),
        ));
    }
    let file = File::open(path).map_err(|error| io_error(&error))?;
    let mut bytes = Vec::new();
    file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(&error))?;
    if bytes.len() > limit {
        return Err(SkillError(
            "This skill resource exceeds the size limit.".into(),
        ));
    }
    Ok(bytes)
}

fn private_directory(path: &Path) -> Result<(), SkillError> {
    if path.exists() {
        reject_symlink(path)?;
    }
    fs::create_dir_all(path).map_err(|error| io_error(&error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| io_error(&error))?;
    }
    Ok(())
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<(), SkillError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| io_error(&error))?;
    file.write_all(bytes).map_err(|error| io_error(&error))?;
    file.sync_all().map_err(|error| io_error(&error))
}

fn io_error(error: &std::io::Error) -> SkillError {
    SkillError(format!("Skill storage: {error}"))
}
