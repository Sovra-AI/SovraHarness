//! Owner-private bounded files shared by local control and phone pairing.
use serde::Serialize;
use std::{fs::File, path::Path};

pub(crate) fn random_secret() -> Result<String, String> {
    use std::fmt::Write as _;
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Secure randomness unavailable.".to_owned())?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(result, "{byte:02x}");
    }
    Ok(result)
}

pub(crate) fn private_directory(path: &Path) -> Result<(), String> {
    if !path.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(path).map_err(|error| io_error(&error))?;
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| io_error(&error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Private directory must be a real, private directory.".into());
    }
    check_permissions(&metadata)
}

pub(crate) fn private_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| io_error(&error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Private file must be a real, private file.".into());
    }
    check_permissions(&metadata)
}

fn check_permissions(metadata: &std::fs::Metadata) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Private files must be accessible only to their owner (directory 0700, files 0600).".into());
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

pub(crate) fn io_error(error: &std::io::Error) -> String {
    format!("Private control IO failed: {}", error.kind())
}

pub(crate) async fn write_private<T: Serialize>(
    directory: &Path,
    name: &str,
    value: &T,
) -> Result<(), String> {
    let root = directory.to_owned();
    let name = name.to_owned();
    let bytes =
        serde_json::to_vec(value).map_err(|_| "Could not encode private settings.".to_owned())?;
    tokio::task::spawn_blocking(move || {
        use std::io::Write as _;
        private_directory(&root)?;
        let temporary = root.join(format!(".{}", random_secret()?));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options.open(&temporary).map_err(|error| io_error(&error))?;
            file.write_all(&bytes).map_err(|error| io_error(&error))?;
            file.sync_all().map_err(|error| io_error(&error))?;
            std::fs::rename(&temporary, root.join(name)).map_err(|error| io_error(&error))?;
            #[cfg(unix)]
            File::open(&root)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| io_error(&error))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    })
    .await
    .map_err(|_| "Private file task failed.".to_owned())?
}

pub(crate) async fn read_private(directory: &Path, name: &str) -> Result<Vec<u8>, String> {
    let directory = directory.to_owned();
    let name = name.to_owned();
    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        private_directory(&directory)?;
        let path = directory.join(name);
        private_file(&path)?;
        let file = File::open(path).map_err(|error| io_error(&error))?;
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|error| io_error(&error))?;
        if bytes.len() > 4096 {
            return Err("Private file exceeds its limit.".into());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| "Private file task failed.".to_owned())?
}
