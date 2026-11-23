//! Lists, saves, reads back and lists again in a real storage place with
//! Sovra's own storage adapter, exactly as the Connections page and the
//! assistant's tools do, so it can be checked against a real service or a
//! local stand-in (`MinIO`, a `WebDAV` server, Azurite).
//!
//! ```text
//! cargo run -p sovra-app --example storage_selftest -- <service> <endpoint> <region> <bucket> <user> <secret> [folder]
//! ```
//!
//! `<service>` is a slug such as `minio`, `amazon-s3`, `azure-blob` or
//! `other-webdav`; give `-` for a region or bucket the service has none of.

use std::io::Write as _;
use std::sync::Arc;

use sovra_core::storage::{StorageConnector, StoragePlace, StorageSecret, StorageService};
use sovra_providers::{FileCredentialStore, HttpStorage};

type Failure = Box<dyn std::error::Error>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Failure> {
    let mut arguments = std::env::args().skip(1);
    let mut next = || {
        arguments
            .next()
            .ok_or("give service, endpoint, region, bucket, user and secret")
    };
    let slug = next()?;
    let service = StorageService::ALL
        .into_iter()
        .find(|service| service.slug() == slug)
        .ok_or("unknown service slug")?;
    let dash = |text: String| if text == "-" { String::new() } else { text };
    let place = StoragePlace {
        id: "selftest".into(),
        name: "Self test".into(),
        service,
        endpoint: next()?,
        region: dash(next()?),
        bucket: dash(next()?),
        user: next()?,
        folder: std::env::args().nth(7).unwrap_or_default(),
        allow_saving: true,
    }
    .tidied();
    let secret = StorageSecret::new(next()?).ok_or("empty secret")?;
    place.validate()?;
    let keys = std::env::temp_dir().join(format!("sovra-storage-selftest-{}", std::process::id()));
    let storage = HttpStorage::new(Arc::new(FileCredentialStore::new(keys)));
    let mut out = std::io::stdout().lock();

    let top = storage.list(&place, &secret, "", 50).await;
    writeln!(out, "list top: {top:?}")?;
    let text = "# Self test\n\nA café note with ünïcode and spaces.\n";
    let path = "sovra self test/notes & plans.md";
    let saved = storage
        .write(
            &place,
            &secret,
            path,
            text.as_bytes().to_vec(),
            sovra_core::storage::content_type(path),
        )
        .await;
    writeln!(out, "write: {saved:?}")?;
    let read = storage.read(&place, &secret, path, 1024 * 1024).await;
    let same = read.as_ref().is_ok_and(|bytes| bytes == text.as_bytes());
    writeln!(
        out,
        "read back identical: {same} ({:?})",
        read.map(|b| b.len())
    )?;
    let folder = storage.list(&place, &secret, "sovra self test", 50).await;
    writeln!(out, "list folder: {folder:?}")?;
    let top = storage.list(&place, &secret, "", 50).await;
    writeln!(out, "list top again: {top:?}")?;
    let missing = storage
        .read(&place, &secret, "no such file.txt", 1024)
        .await;
    writeln!(out, "missing file: {missing:?}")?;
    let wrong = StorageSecret::new("wrong-secret").ok_or("secret")?;
    let refused = storage.list(&place, &wrong, "", 1).await;
    writeln!(out, "wrong secret: {refused:?}")?;
    Ok(())
}
