//! Private, authenticated, bounded local IPC behind the stdio MCP transport.
use super::{Incoming, MAX_FRAME, MAX_RESPONSE, REQUEST_TIMEOUT};
use async_channel::Sender;
use serde::{Deserialize, Serialize};
use sovra_core::development::UiRequest;
use std::{
    fs::File,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Descriptor {
    version: u8,
    port: u16,
    pub(super) token: String,
    pub(super) generation: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    request: UiRequest,
}

pub(super) struct Active {
    pub(super) descriptor: Descriptor,
    pub(super) listener: TcpListener,
    directory: PathBuf,
    // An OS-held lock, released only after our descriptor is removed.
    _lock: File,
}

impl Active {
    pub(super) async fn open(directory: &Path) -> Result<Self, String> {
        let root = directory.to_owned();
        let lock = tokio::task::spawn_blocking(move || {
            private_directory(&root)?;
            let lock_path = root.join("owner.lock");
            if lock_path.exists() {
                private_file(&lock_path)?;
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let file = options.open(lock_path).map_err(|error| io_error(&error))?;
            file.try_lock()
                .map_err(|_| "Another Sovra instance owns development mode.".to_owned())?;
            Ok::<_, String>(file)
        })
        .await
        .map_err(|_| "Development setup task failed.".to_owned())??;
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| io_error(&error))?;
        let descriptor = Descriptor {
            version: 1,
            port: listener
                .local_addr()
                .map_err(|error| io_error(&error))?
                .port(),
            token: random_secret()?,
            generation: random_secret()?,
        };
        write_private(directory, "endpoint.json", &descriptor).await?;
        Ok(Self {
            descriptor,
            listener,
            directory: directory.to_owned(),
            _lock: lock,
        })
    }

    pub(super) async fn close(self) {
        // The lock guarantees no other app could have replaced this descriptor.
        if let Err(error) = tokio::fs::remove_file(self.directory.join("endpoint.json")).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "could not remove development endpoint; listener is revoked");
        }
    }
}

fn random_secret() -> Result<String, String> {
    use std::fmt::Write as _;
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Secure randomness unavailable.".to_owned())?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(result, "{byte:02x}");
    }
    Ok(result)
}

fn private_directory(path: &Path) -> Result<(), String> {
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
        return Err("Development directory must be a real, private directory.".into());
    }
    check_permissions(&metadata)
}

fn private_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| io_error(&error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Development file must be a real, private file.".into());
    }
    check_permissions(&metadata)
}

fn check_permissions(metadata: &std::fs::Metadata) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Development files must be accessible only to their owner (directory 0700, files 0600).".into());
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

fn io_error(error: &std::io::Error) -> String {
    format!("Development control IO failed: {}", error.kind())
}

async fn write_private<T: Serialize>(
    directory: &Path,
    name: &str,
    value: &T,
) -> Result<(), String> {
    let root = directory.to_owned();
    let name = name.to_owned();
    let bytes = serde_json::to_vec(value)
        .map_err(|_| "Could not encode development settings.".to_owned())?;
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
            std::fs::rename(&temporary, root.join(name)).map_err(|error| io_error(&error))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    })
    .await
    .map_err(|_| "Development file task failed.".to_owned())?
}

async fn read_private(directory: &Path, name: &str) -> Result<Vec<u8>, String> {
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
            return Err("Development file exceeds its limit.".into());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| "Development file task failed.".to_owned())?
}

pub(super) async fn load_enabled(directory: &Path) -> Result<bool, String> {
    if !tokio::fs::try_exists(directory.join("settings.json"))
        .await
        .map_err(|error| io_error(&error))?
    {
        return Ok(false);
    }
    serde_json::from_slice(&read_private(directory, "settings.json").await?)
        .map_err(|_| "Invalid development setting; mode remains off.".into())
}

pub(super) async fn save_enabled(directory: &Path, enabled: bool) -> Result<(), String> {
    if !enabled {
        return match tokio::fs::remove_file(directory.join("settings.json")).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(&error)),
        };
    }
    write_private(directory, "settings.json", &enabled).await
}

async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    BufReader::new(reader.take(u64::try_from(limit + 1).map_err(|_| "Invalid frame limit")?))
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(|error| io_error(&error))?;
    if bytes.len() > limit || bytes.last() != Some(&b'\n') {
        return Err("Invalid or oversized development frame.".into());
    }
    Ok(bytes)
}

pub(super) async fn serve_connection(
    mut stream: TcpStream,
    token: &str,
    generation: String,
    incoming: Sender<Incoming>,
) -> Result<(), String> {
    let bytes = read_frame(&mut stream, MAX_FRAME).await?;
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid development request")?;
    if envelope.token.len() != token.len()
        || envelope
            .token
            .bytes()
            .zip(token.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            != 0
    {
        return Err("Invalid development credential.".into());
    }
    envelope.request.validate().map_err(str::to_owned)?;
    let (reply, receiver) = oneshot::channel();
    incoming
        .try_send(Incoming {
            generation,
            request: envelope.request,
            reply,
        })
        .map_err(|_| "Development mode is busy")?;
    let result = tokio::select! {
        result = receiver => result.unwrap_or_else(|_| Err("Request expired or development mode was disabled; inspect before retrying an action.".into())),
        _ = stream.read_u8() => return Err("Development client disconnected or sent extra input.".into()),
    };
    let mut bytes =
        serde_json::to_vec(&result).map_err(|_| "Could not encode development reply")?;
    if bytes.len() >= MAX_RESPONSE {
        return Err("UI snapshot exceeds the response limit.".into());
    }
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .await
        .map_err(|error| io_error(&error))
}

pub(super) async fn call(
    directory: &Path,
    request: UiRequest,
) -> Result<serde_json::Value, String> {
    request.validate().map_err(str::to_owned)?;
    tokio::time::timeout(REQUEST_TIMEOUT, async {
        let descriptor: Descriptor = serde_json::from_slice(
            &read_private(directory, "endpoint.json")
                .await
                .map_err(|_| "Open Sovra and enable Development mode in Settings.".to_owned())?,
        )
        .map_err(|_| "Invalid local development descriptor.")?;
        if descriptor.version != 1 || descriptor.token.len() != 64 || descriptor.port == 0 {
            return Err("Unsupported development endpoint.".into());
        }
        let mut stream = TcpStream::connect(SocketAddrV4::new(
            Ipv4Addr::LOCALHOST,
            descriptor.port,
        ))
        .await
        .map_err(|_| {
            "Sovra is restarting or development mode is off. Inspect again when the app is ready."
                .to_owned()
        })?;
        let mut bytes = serde_json::to_vec(&Envelope {
            token: descriptor.token,
            request,
        })
        .map_err(|_| "Invalid request")?;
        bytes.push(b'\n');
        stream
            .write_all(&bytes)
            .await
            .map_err(|error| io_error(&error))?;
        let response = read_frame(&mut stream, MAX_RESPONSE).await?;
        serde_json::from_slice::<Result<serde_json::Value, String>>(&response)
            .map_err(|_| "Invalid development response".to_owned())?
    })
    .await
    .map_err(|_| {
        "Development request timed out. Inspect state before retrying an action.".to_owned()
    })?
}
