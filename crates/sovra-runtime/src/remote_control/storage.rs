//! Acquire one owner for the phone allowlist and restore only validated records.
use crate::private_storage::{io_error, private_directory, private_file, read_private};
use sovra_core::remote_control::RemoteAccess;
use std::{fs::File, path::Path};

pub(super) async fn open(directory: &Path) -> Result<(File, RemoteAccess), String> {
    let root = directory.to_owned();
    let lock = tokio::task::spawn_blocking(move || {
        private_directory(&root)?;
        let path = root.join("owner.lock");
        match std::fs::symlink_metadata(&path) {
            Ok(_) => private_file(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&error)),
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|error| io_error(&error))?;
        file.try_lock()
            .map_err(|_| "Another Sovra instance owns phone control.".to_owned())?;
        Ok::<_, String>(file)
    })
    .await
    .map_err(|_| "Phone storage worker stopped.".to_owned())??;
    let policy = match tokio::fs::symlink_metadata(directory.join("phones.json")).await {
        Ok(_) => {
            let bytes = read_private(directory, "phones.json").await?;
            let phones = serde_json::from_slice(&bytes)
                .map_err(|_| "Saved phone list is damaged; remote control remains off.")?;
            RemoteAccess::restore(phones).map_err(|error| error.to_string())?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RemoteAccess::default(),
        Err(error) => return Err(io_error(&error)),
    };
    Ok((lock, policy))
}
