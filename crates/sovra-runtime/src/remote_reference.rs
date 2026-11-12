//! Runnable phone-protocol reference client; owns bounded IO and never replays input.
mod stdio;
pub use stdio::run_remote_reference;

use sovra_core::{
    development::UiRequest,
    group::{DeviceId, PeerAddress},
    remote_control::{
        ErrorCode, InvitationSecret, MAX_SESSION_REQUESTS, PROTOCOL_VERSION, PairingTicket,
        RemoteConnection, RemoteEndpoint, RemoteOperation, RemoteRequest, RemoteResponse,
    },
};
use std::{path::PathBuf, sync::Arc, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(5);

struct Session {
    connection: Arc<dyn RemoteConnection>,
    generation: DeviceId,
    nonce: DeviceId,
    next: u32,
}

/// The supplied endpoint must use a persistent phone key, separate from the
/// desktop key. Only public routing hints are stored in this client's directory.
pub struct RemoteReferenceClient {
    directory: PathBuf,
    endpoint: Arc<dyn RemoteEndpoint>,
    desktop: Option<PeerAddress>,
    session: Option<Session>,
}

impl std::fmt::Debug for RemoteReferenceClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteReferenceClient")
            .field("remembered", &self.desktop.is_some())
            .field("connected", &self.session.is_some())
            .finish_non_exhaustive()
    }
}

impl RemoteReferenceClient {
    pub async fn open(
        directory: PathBuf,
        endpoint: Arc<dyn RemoteEndpoint>,
    ) -> Result<Self, crate::Error> {
        let desktop = match tokio::fs::symlink_metadata(directory.join("desktop.json")).await {
            Ok(_) => {
                let bytes = crate::private_storage::read_private(&directory, "desktop.json")
                    .await
                    .map_err(crate::Error::RemoteReference)?;
                let address: PeerAddress = serde_json::from_slice(&bytes).map_err(|_| {
                    crate::Error::RemoteReference("Saved desktop address is damaged.".into())
                })?;
                // Apply exactly the same bounded address policy as QR enrollment.
                PairingTicket {
                    version: PROTOCOL_VERSION,
                    address: address.clone(),
                    secret: InvitationSecret([0; 32]),
                }
                .encode()
                .map_err(|_| crate::Error::RemoteReference("Invalid saved desktop.".into()))?;
                Some(address)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(crate::Error::RemoteReference(
                    "Could not read the saved desktop.".into(),
                ));
            }
        };
        Ok(Self {
            directory,
            endpoint,
            desktop,
            session: None,
        })
    }

    /// Recover a lost Pair acknowledgment with Hello, without requiring a second
    /// invitation. The invitation itself is never persisted or returned.
    pub async fn pair(&mut self, link: &str, name: String) -> RemoteResponse {
        let Ok(ticket) = PairingTicket::decode(link) else {
            return error(ErrorCode::InvalidInvitation, "Invalid pairing link.");
        };
        self.disconnect();
        let connection = match self.connect(&ticket.address).await {
            Ok(connection) => connection,
            Err(response) => return response,
        };
        let hello = exchange(&connection, RemoteOperation::Hello {}).await;
        let response = match hello {
            Ok(RemoteResponse::Ready {
                version: PROTOCOL_VERSION,
                ..
            }) => RemoteResponse::Paired {},
            Ok(RemoteResponse::Error {
                code: ErrorCode::NotPaired,
                ..
            }) => {
                match exchange(
                    &connection,
                    RemoteOperation::Pair {
                        secret: ticket.secret,
                        name,
                    },
                )
                .await
                {
                    Ok(response) => response,
                    Err(()) => error(
                        ErrorCode::OutcomeUnknown,
                        "Pairing reply was lost. Retry pairing to check Hello before sending the invitation again.",
                    ),
                }
            }
            Ok(response @ RemoteResponse::Error { .. }) => response,
            Ok(_) => error(
                ErrorCode::UnsupportedVersion,
                "Desktop returned an incompatible Hello response.",
            ),
            Err(()) => error(
                ErrorCode::OutcomeUnknown,
                "Desktop did not answer Hello. Retry when reachable.",
            ),
        };
        connection.close();
        if matches!(response, RemoteResponse::Paired {}) {
            if crate::private_storage::write_private(
                &self.directory,
                "desktop.json",
                &ticket.address,
            )
            .await
            .is_err()
            {
                return error(
                    ErrorCode::StorageUnavailable,
                    "Desktop paired this phone, but its address could not be saved. Retry pairing before closing this client.",
                );
            }
            self.desktop = Some(ticket.address);
        }
        response
    }

    /// Reads reconnect once automatically. Mutations never reconnect or retry:
    /// targets and revisions must come from an inspection in the current session.
    pub async fn control(
        &mut self,
        request: UiRequest,
        revision: Option<String>,
    ) -> RemoteResponse {
        if let Err(message) = request.validate() {
            return error(ErrorCode::InvalidRequest, message);
        }
        let read = matches!(request, UiRequest::Inspect {} | UiRequest::Screenshot {});
        for attempt in 0..2 {
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.next > MAX_SESSION_REQUESTS)
            {
                self.disconnect();
            }
            if self.session.is_none() {
                if !read {
                    return error(
                        ErrorCode::StaleSession,
                        "Inspect before sending input on a new connection.",
                    );
                }
                if let Err(response) = self.hello().await {
                    return response;
                }
            }
            let session = self.session.as_mut().expect("Hello installed the session");
            let operation = RemoteOperation::Control {
                generation: session.generation,
                session: session.nonce,
                sequence: session.next,
                revision: revision.clone(),
                request: request.clone(),
            };
            session.next += 1;
            if let Ok(response) = exchange(&session.connection, operation).await {
                if matches!(
                    &response,
                    RemoteResponse::Error {
                        code: ErrorCode::StaleSession | ErrorCode::Disabled | ErrorCode::NotPaired,
                        ..
                    }
                ) {
                    self.disconnect();
                    if read && attempt == 0 {
                        continue;
                    }
                }
                return response;
            }
            self.disconnect();
            if !read || attempt == 1 {
                return error(
                    ErrorCode::OutcomeUnknown,
                    "Connection ended without a reply. Inspect again; do not replay uncertain input.",
                );
            }
        }
        error(
            ErrorCode::OutcomeUnknown,
            "Desktop is unreachable. Inspect again when available.",
        )
    }

    async fn connect(
        &self,
        address: &PeerAddress,
    ) -> Result<Arc<dyn RemoteConnection>, RemoteResponse> {
        match tokio::time::timeout(TIMEOUT, self.endpoint.connect(address)).await {
            Ok(Ok(connection)) if connection.peer() == address.device => Ok(connection),
            Ok(Ok(connection)) => {
                connection.close();
                Err(error(
                    ErrorCode::InvalidRequest,
                    "Desktop identity did not match the saved key.",
                ))
            }
            Ok(Err(_)) | Err(_) => Err(error(
                ErrorCode::OutcomeUnknown,
                "Desktop is unreachable. Start remote control on the computer and retry.",
            )),
        }
    }

    async fn hello(&mut self) -> Result<(), RemoteResponse> {
        let address = self.desktop.as_ref().ok_or_else(|| {
            error(
                ErrorCode::NotPaired,
                "Pair this client using the desktop invitation first.",
            )
        })?;
        let connection = self.connect(address).await?;
        match exchange(&connection, RemoteOperation::Hello {}).await {
            Ok(RemoteResponse::Ready {
                version: PROTOCOL_VERSION,
                generation,
                session,
                ..
            }) => {
                self.session = Some(Session {
                    connection,
                    generation,
                    nonce: session,
                    next: 1,
                });
                Ok(())
            }
            response => {
                connection.close();
                Err(match response {
                    Ok(response @ RemoteResponse::Error { .. }) => response,
                    Ok(_) => error(
                        ErrorCode::UnsupportedVersion,
                        "Desktop returned an incompatible Hello response.",
                    ),
                    Err(()) => error(ErrorCode::OutcomeUnknown, "Desktop did not answer Hello."),
                })
            }
        }
    }

    fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            session.connection.close();
        }
    }

    pub async fn close(mut self) {
        self.disconnect();
        let _ = tokio::time::timeout(TIMEOUT, self.endpoint.close()).await;
    }
}

impl Drop for RemoteReferenceClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

async fn exchange(
    connection: &Arc<dyn RemoteConnection>,
    operation: RemoteOperation,
) -> Result<RemoteResponse, ()> {
    tokio::time::timeout(
        TIMEOUT,
        connection.request(&RemoteRequest {
            version: PROTOCOL_VERSION,
            operation,
        }),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

fn error(code: ErrorCode, message: &str) -> RemoteResponse {
    RemoteResponse::Error {
        code,
        message: message.into(),
    }
}
