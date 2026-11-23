//! Online storage over HTTPS: S3-language buckets, `WebDAV` folders and Azure
//! Blob containers behind the core [`StorageConnector`] port.
//!
//! Requests go only to the address the person typed. Redirects are refused,
//! because a signed or password-carrying request must not be replayed to
//! another host. Answers are read with a size cap, and a service's own words
//! are never repeated beyond its short error name.

mod azure;
mod s3;
mod webdav;

use std::sync::Arc;
use std::time::Duration;

use sovra_core::models::ModelSecret;
use sovra_core::storage::{
    Listing, StorageConnector, StorageError, StoragePlace, StorageProtocol, StorageSecret,
};

use crate::CredentialStore;
use crate::sigv4::error_code;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A 20 MB file on a slow connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// Most of a listing answer that is read. A thousand entries fit easily.
const MAX_LISTING_BYTES: usize = 4 * 1024 * 1024;

/// The real storage adapter.
pub struct HttpStorage {
    store: Arc<dyn CredentialStore>,
    http: Result<reqwest::Client, String>,
}

impl std::fmt::Debug for HttpStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpStorage").finish_non_exhaustive()
    }
}

impl HttpStorage {
    pub fn new(store: Arc<dyn CredentialStore>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not start a connection.".to_owned());
        Self { store, http }
    }

    fn http(&self) -> Result<&reqwest::Client, StorageError> {
        self.http
            .as_ref()
            .map_err(|message| StorageError::Failed(message.clone()))
    }
}

/// What Sovra was doing when a service said no, for the words of the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Doing {
    Looking,
    Reading,
    Saving,
}

fn unreachable() -> StorageError {
    StorageError::Failed(
        "Could not reach the storage service. Check the storage address and your internet connection."
            .into(),
    )
}

/// Explains a refusal in words the person can act on, adding the service's
/// status and short error name (never its free text).
pub(crate) fn explain(
    place: &StoragePlace,
    doing: Doing,
    status: u16,
    body: &str,
    region_hint: Option<&str>,
) -> StorageError {
    let code = error_code(body);
    let service = place.service;
    let verb = match doing {
        Doing::Looking => "list files",
        Doing::Reading => "read files",
        Doing::Saving => "save files",
    };
    let what = match (status, code) {
        (
            _,
            Some(
                "SignatureDoesNotMatch" | "InvalidAccessKeyId" | "AuthenticationFailed"
                | "InvalidSecurity" | "AuthorizationFailure",
            ),
        )
        | (401, _) => format!(
            "The storage service did not accept the sign-in. Check the {} and the {}.",
            sovra_core::storage::in_sentence(service.user_label()),
            sovra_core::storage::in_sentence(service.secret_label())
        ),
        (_, Some("RequestTimeTooSkewed")) => "The storage service refused because this computer's clock is wrong. Set the date and time, then try again.".to_owned(),
        (_, Some("AuthorizationPermissionMismatch" | "AuthorizationResourceTypeMismatch"))
        | (403, _) => format!(
            "The storage service refused to let Sovra {verb}. Check that the keys may {verb} in this place."
        ),
        (_, Some("NoSuchKey" | "BlobNotFound")) => "That file was not found.".to_owned(),
        (_, Some("NoSuchBucket" | "ContainerNotFound")) => match place.protocol() {
            StorageProtocol::AzureBlob => {
                "The container was not found. Check the container name.".to_owned()
            }
            StorageProtocol::S3 | StorageProtocol::WebDav => {
                "The bucket was not found. Check the bucket name and the storage address."
                    .to_owned()
            }
        },
        (404, _) => match (doing, place.protocol()) {
            (Doing::Reading, _) => "That file was not found.".to_owned(),
            (_, StorageProtocol::WebDav) => {
                "That folder was not found. Check the storage address.".to_owned()
            }
            (_, StorageProtocol::AzureBlob) => {
                "The container was not found. Check the container name.".to_owned()
            }
            (_, StorageProtocol::S3) => {
                "The bucket was not found. Check the bucket name and the storage address."
                    .to_owned()
            }
        },
        (301 | 307 | 308 | 400, _) | (_, Some("AuthorizationHeaderMalformed" | "PermanentRedirect")) => {
            match region_hint {
                Some(region) => format!(
                    "This bucket is in the {region} region. Change the region to {region}, then try again."
                ),
                None => "The storage service did not accept the request. Check the storage address and the region.".to_owned(),
            }
        }
        (405, _) if place.protocol() == StorageProtocol::WebDav => {
            "This address does not accept WebDAV. Check that it is the WebDAV address of a folder."
                .to_owned()
        }
        (413, _) => "The storage service said the file is too large.".to_owned(),
        (507, _) | (_, Some("QuotaExceeded")) => {
            "The storage place is full. Make room there, then try again.".to_owned()
        }
        _ => "The storage service could not do that now. Try again later.".to_owned(),
    };
    StorageError::Failed(match code {
        Some(code) => format!("{what} (It answered {status} {code}.)"),
        None => format!("{what} (It answered {status}.)"),
    })
}

/// The body of `response`, refusing anything over `max` bytes.
pub(crate) async fn read_capped(
    mut response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, StorageError> {
    let too_large = || {
        StorageError::Failed(format!(
            "That file is larger than {} MB, so Sovra will not read it.",
            max / (1024 * 1024)
        ))
    };
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unreachable())? {
        if bytes.len() + chunk.len() > max {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Percent-decodes a path from a service's answer. Invalid escapes stay as written.
pub(crate) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                char::from(bytes[i + 1]).to_digit(16),
                char::from(bytes[i + 2]).to_digit(16),
            )
        {
            out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn keychain() -> StorageError {
    StorageError::Failed("The storage sign-in could not be read from the keychain.".into())
}

#[async_trait::async_trait]
impl StorageConnector for HttpStorage {
    fn secret(&self, place: &StoragePlace) -> Result<Option<StorageSecret>, StorageError> {
        let saved = self
            .store
            .get(&place.secret_entry())
            .map_err(|_| keychain())?;
        Ok(StorageSecret::new(saved.0))
    }

    fn set_secret(
        &self,
        place: &StoragePlace,
        secret: Option<&StorageSecret>,
    ) -> Result<(), StorageError> {
        let entry = place.secret_entry();
        match secret {
            Some(secret) => self
                .store
                .set(&entry, &ModelSecret(secret.as_str().to_owned())),
            None => self.store.delete(&entry),
        }
        .map_err(|_| {
            StorageError::Failed("The storage sign-in could not be saved in the keychain.".into())
        })
    }

    async fn list(
        &self,
        place: &StoragePlace,
        secret: &StorageSecret,
        folder: &str,
        limit: usize,
    ) -> Result<Listing, StorageError> {
        let http = self.http()?;
        let listing = match place.protocol() {
            StorageProtocol::S3 => s3::list(http, place, secret, folder, limit).await?,
            StorageProtocol::WebDav => webdav::list(http, place, secret, folder).await?,
            StorageProtocol::AzureBlob => azure::list(http, place, secret, folder, limit).await?,
        };
        Ok(listing.tidy(folder, limit))
    }

    async fn read(
        &self,
        place: &StoragePlace,
        secret: &StorageSecret,
        path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, StorageError> {
        let http = self.http()?;
        match place.protocol() {
            StorageProtocol::S3 => s3::read(http, place, secret, path, max_bytes).await,
            StorageProtocol::WebDav => webdav::read(http, place, secret, path, max_bytes).await,
            StorageProtocol::AzureBlob => azure::read(http, place, secret, path, max_bytes).await,
        }
    }

    async fn write(
        &self,
        place: &StoragePlace,
        secret: &StorageSecret,
        path: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        let http = self.http()?;
        match place.protocol() {
            StorageProtocol::S3 => s3::write(http, place, secret, path, bytes, content_type).await,
            StorageProtocol::WebDav => {
                webdav::write(http, place, secret, path, bytes, content_type).await
            }
            StorageProtocol::AzureBlob => {
                azure::write(http, place, secret, path, bytes, content_type).await
            }
        }
    }
}

/// The text of an answer, at most [`MAX_LISTING_BYTES`].
pub(crate) async fn listing_text(response: reqwest::Response) -> Result<String, StorageError> {
    let bytes = read_capped(response, MAX_LISTING_BYTES)
        .await
        .map_err(|_| {
            StorageError::Failed("The storage service sent a listing too large to read.".into())
        })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// A listing answer Sovra could not make sense of.
pub(crate) fn unreadable() -> StorageError {
    StorageError::Failed(
        "The storage service answered in a way Sovra does not understand. Check the storage address."
            .into(),
    )
}

#[cfg(test)]
mod tests;
