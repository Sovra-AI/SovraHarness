//! Sends three files to a bucket with Sovra's own uploader, exactly as a
//! backup does, so the requests can be checked against another S3
//! implementation or a real service.
//!
//! ```text
//! cargo run -p sovra-app --example bucket_selftest -- <endpoint> <region> <bucket> <key id> <secret>
//! ```

use std::io::Write as _;
use std::sync::Arc;

use sovra_core::training::{BucketFile, BucketSettings, ModelOutlets};
use sovra_providers::{FileCredentialStore, HttpModelOutlets};

type Failure = Box<dyn std::error::Error>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Failure> {
    let mut arguments = std::env::args().skip(1);
    let mut next = || {
        arguments
            .next()
            .ok_or("give endpoint, region, bucket, key id and secret")
    };
    let bucket = BucketSettings {
        endpoint: next()?,
        region: next()?,
        bucket: next()?,
        folder: "sovra/self test".into(),
        access_key_id: next()?,
    };
    let secret = next()?;
    bucket.validate()?;
    let keys = std::env::temp_dir().join(format!("sovra-bucket-selftest-{}", std::process::id()));
    let outlets = HttpModelOutlets::new(Arc::new(FileCredentialStore::new(keys)), None);
    let model: Vec<u8> = (0..=255_u8).cycle().take(70_000).collect();
    let files = [
        (
            "garden notes-1a2b3c4d.model",
            "application/octet-stream",
            model.clone(),
        ),
        (
            "garden notes-1a2b3c4d.gguf",
            "application/octet-stream",
            model,
        ),
        (
            "garden notes-1a2b3c4d.md",
            "text/markdown; charset=utf-8",
            "# Garden notes\n\nA café model ünïcode.\n"
                .as_bytes()
                .to_vec(),
        ),
    ];
    let mut out = std::io::stdout().lock();
    for (name, content_type, bytes) in files {
        let result = outlets
            .put(
                &bucket,
                &secret,
                BucketFile {
                    key: bucket.object_key(name),
                    content_type,
                    bytes,
                },
            )
            .await;
        writeln!(out, "{name}: {result:?}")?;
    }
    Ok(())
}
