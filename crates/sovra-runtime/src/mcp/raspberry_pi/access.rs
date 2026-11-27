//! Password sign-in with a remembered host identity and OS-backed secret storage.
//! The socket guard also stops the SSH library's task on cancellation or timeout.

use russh::{
    ChannelMsg, client,
    keys::{HashAlg, PublicKeyOrCertificate},
};
use sovra_core::{
    models::ModelSecret,
    raspberry_pi::{PiSetup, credential_id, validate_pairing},
};
use sovra_providers::CredentialStore;
use std::{net::Shutdown, sync::Arc, time::Duration};

const CONNECTION_FAILED: &str = "Couldn't reach your Pi. Check its power, keep both computers on the same network, and enable remote access on the Pi.";
const IDENTITY_CHANGED: &str = "This Pi's connection identity has changed. Sovra stopped before sending your password. If you reinstalled the Pi, remove its saved connection in Details and pair it again.";
const STORE_FAILED: &str = "Couldn't read or save this Pi's sign-in in secure storage. Nothing is marked connected. Check this computer's credential storage and try again.";

pub(crate) struct TrustQuestion {
    pub fingerprint: String,
    pub answer: tokio::sync::oneshot::Sender<bool>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedAccess {
    setup: PiSetup,
    fingerprint: String,
    password: String,
}

struct SocketGuard(std::net::TcpStream);
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

struct HostIdentity {
    expected: Option<String>,
    questions: Option<async_channel::Sender<TrustQuestion>>,
    observed: tokio::sync::oneshot::Sender<String>,
    cancelled: tokio::sync::oneshot::Receiver<()>,
}

impl client::Handler for HostIdentity {
    type Error = crate::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        if let Some(expected) = &self.expected {
            if expected != &fingerprint {
                return Err(crate::Error::PiIdentityChanged);
            }
        } else {
            let Some(questions) = &self.questions else {
                return Err(crate::Error::PiIdentityUnconfirmed);
            };
            let (answer, reply) = tokio::sync::oneshot::channel();
            questions
                .send(TrustQuestion {
                    fingerprint: fingerprint.clone(),
                    answer,
                })
                .await
                .map_err(|_| crate::Error::PiPairingCancelled)?;
            let accepted = tokio::select! {
                result = tokio::time::timeout(Duration::from_secs(120), reply) => result.ok().and_then(Result::ok).unwrap_or(false),
                _ = &mut self.cancelled => false,
            };
            if !accepted {
                return Err(crate::Error::PiPairingCancelled);
            }
        }
        // Rekeying checks the same trusted identity without asking the person again.
        self.expected = Some(fingerprint.clone());
        let (replacement, _) = tokio::sync::oneshot::channel();
        let sender = std::mem::replace(&mut self.observed, replacement);
        let _ = sender.send(fingerprint);
        Ok(true)
    }
}

struct Connection {
    session: client::Handle<HostIdentity>,
    _socket: SocketGuard,
    _cancel: tokio::sync::oneshot::Sender<()>,
    fingerprint: String,
}

fn connection_problem(error: &crate::Error) -> String {
    match error {
        crate::Error::PiIdentityChanged => IDENTITY_CHANGED.into(),
        crate::Error::PiIdentityUnconfirmed => {
            "This Pi hasn't been paired yet. Enter its account password in Connection details."
                .into()
        }
        crate::Error::PiPairingCancelled => "Pi sign-in cancelled.".into(),
        crate::Error::PiConnection(_)
        | crate::Error::Start(_)
        | crate::Error::Stopped
        | crate::Error::Development(_)
        | crate::Error::BuiltinMcp(_)
        | crate::Error::RemoteReference(_) => CONNECTION_FAILED.into(),
    }
}

impl Connection {
    async fn open(
        setup: &PiSetup,
        port: u16,
        expected: Option<String>,
        questions: Option<async_channel::Sender<TrustQuestion>>,
    ) -> Result<Self, String> {
        let socket = tokio::time::timeout(
            Duration::from_secs(8),
            tokio::net::TcpStream::connect((setup.host.as_str(), port)),
        )
        .await
        .map_err(|_| CONNECTION_FAILED)?
        .map_err(|_| CONNECTION_FAILED)?;
        let socket = socket.into_std().map_err(|_| CONNECTION_FAILED)?;
        let guard = SocketGuard(socket.try_clone().map_err(|_| CONNECTION_FAILED)?);
        let socket = tokio::net::TcpStream::from_std(socket).map_err(|_| CONNECTION_FAILED)?;
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let (observed, fingerprint) = tokio::sync::oneshot::channel();
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(150)),
            window_size: 64 * 1024,
            maximum_packet_size: 32 * 1024,
            channel_buffer_size: 8,
            ..Default::default()
        });
        let session = client::connect_stream(
            config,
            socket,
            HostIdentity {
                expected,
                questions,
                observed,
                cancelled,
            },
        )
        .await
        .map_err(|error| connection_problem(&error))?;
        let fingerprint = fingerprint.await.map_err(|_| CONNECTION_FAILED)?;
        Ok(Self {
            session,
            _socket: guard,
            _cancel: cancel,
            fingerprint,
        })
    }

    async fn sign_in(&mut self, setup: &PiSetup, password: &str) -> Result<(), String> {
        let result = self
            .session
            .authenticate_password(&setup.user, password)
            .await
            .map_err(|_| CONNECTION_FAILED)?;
        if result.success() {
            Ok(())
        } else {
            Err("Your Pi didn't accept that account name or password. Try the same details you use to sign in on the Pi. If password sign-in is disabled there, use your existing sign-in key.".into())
        }
    }

    async fn python(&mut self, script: &[u8], limit: usize) -> Result<String, String> {
        let channel = self
            .session
            .channel_open_session()
            .await
            .map_err(|_| CONNECTION_FAILED)?;
        channel
            .exec(true, "python3 -I -")
            .await
            .map_err(|_| CONNECTION_FAILED)?;
        let (mut reader, writer) = channel.split();
        let send = async {
            writer
                .data(script)
                .await
                .map_err(|_| CONNECTION_FAILED.to_owned())?;
            writer.eof().await.map_err(|_| CONNECTION_FAILED.to_owned())
        };
        let receive = async {
            let mut output = Vec::new();
            let mut error_bytes = 0;
            let mut exit_code = None;
            while let Some(message) = reader.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        if output.len().saturating_add(data.len()) > limit { return Err("The Pi sent too much information. The operation stopped.".into()); }
                        output.extend_from_slice(&data);
                    }
                    ChannelMsg::ExtendedData { data, .. } => {
                        error_bytes += data.len();
                        if error_bytes > 16 * 1024 { return Err("The Pi sent too much information. The operation stopped.".into()); }
                    }
                    ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
                    ChannelMsg::Failure => return Err("The Pi couldn't start Python 3. Check that Python 3 is installed on the Pi.".into()),
                    _ => {}
                }
            }
            if exit_code != Some(0) {
                return Err("The Pi couldn't finish this operation. Check Python 3, available storage and the project requirements.".into());
            }
            String::from_utf8(output).map_err(|_| "The Pi returned unreadable text.".into())
        };
        let ((), output) = tokio::try_join!(send, receive)?;
        Ok(output)
    }
}

async fn load(
    store: Arc<dyn CredentialStore>,
    setup: &PiSetup,
) -> Result<Option<SavedAccess>, String> {
    let id = credential_id(setup);
    let secret = tokio::task::spawn_blocking(move || store.get(&id))
        .await
        .map_err(|_| STORE_FAILED)?
        .map_err(|_| STORE_FAILED)?;
    if secret.0.is_empty() {
        return Ok(None);
    }
    if secret.0.len() > 4096 {
        return Err(STORE_FAILED.into());
    }
    let saved: SavedAccess = serde_json::from_str(&secret.0).map_err(|_| STORE_FAILED)?;
    if saved.setup.board_id != setup.board_id
        || !saved.setup.host.eq_ignore_ascii_case(&setup.host)
        || saved.setup.user != setup.user
        || !saved.fingerprint.starts_with("SHA256:")
        || saved.fingerprint.len() > 100
    {
        return Err(STORE_FAILED.into());
    }
    validate_pairing(setup, &ModelSecret(saved.password.clone()))?;
    Ok(Some(saved))
}

pub(crate) async fn pair(
    store: Arc<dyn CredentialStore>,
    setup: &PiSetup,
    password: ModelSecret,
    questions: async_channel::Sender<TrustQuestion>,
) -> Result<(), String> {
    pair_at(store, setup, password, questions, 22).await
}

async fn pair_at(
    store: Arc<dyn CredentialStore>,
    setup: &PiSetup,
    password: ModelSecret,
    questions: async_channel::Sender<TrustQuestion>,
    port: u16,
) -> Result<(), String> {
    validate_pairing(setup, &password)?;
    let saved = load(Arc::clone(&store), setup).await?;
    let operation = async {
        let mut connection = Connection::open(
            setup,
            port,
            saved.map(|saved| saved.fingerprint),
            Some(questions),
        )
        .await?;
        connection.sign_in(setup, &password.0).await?;
        let output = connection
            .python(include_bytes!("health.py"), 16 * 1024)
            .await?;
        sovra_core::raspberry_pi::device_model(setup, output.as_bytes())?;
        let saved = SavedAccess {
            setup: setup.clone(),
            fingerprint: connection.fingerprint.clone(),
            password: password.0,
        };
        let secret = ModelSecret(serde_json::to_string(&saved).map_err(|_| STORE_FAILED)?);
        let id = credential_id(setup);
        tokio::task::spawn_blocking(move || store.set(&id, &secret))
            .await
            .map_err(|_| STORE_FAILED)?
            .map_err(|_| STORE_FAILED)?;
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(150), operation)
        .await
        .map_err(|_| "Pi sign-in timed out. Try connecting again.".to_owned())?
}

/// `None` preserves existing OpenSSH key setups; an invalid saved identity never
/// falls back to another authentication method.
pub(super) async fn run_saved(
    store: Option<&Arc<dyn CredentialStore>>,
    setup: &PiSetup,
    script: &[u8],
    seconds: u64,
    limit: usize,
) -> Result<Option<String>, String> {
    run_saved_at(store, setup, script, seconds, limit, 22).await
}

async fn run_saved_at(
    store: Option<&Arc<dyn CredentialStore>>,
    setup: &PiSetup,
    script: &[u8],
    seconds: u64,
    limit: usize,
    port: u16,
) -> Result<Option<String>, String> {
    let Some(store) = store else {
        return Ok(None);
    };
    let Some(saved) = load(Arc::clone(store), setup).await? else {
        return Ok(None);
    };
    let operation = async {
        let mut connection = Connection::open(setup, port, Some(saved.fingerprint), None).await?;
        connection.sign_in(setup, &saved.password).await?;
        connection.python(script, limit).await.map(Some)
    };
    tokio::time::timeout(Duration::from_secs(seconds), operation).await.map_err(|_| "The Pi operation timed out. Check the device before retrying if a project was being sent.".to_owned())?
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
