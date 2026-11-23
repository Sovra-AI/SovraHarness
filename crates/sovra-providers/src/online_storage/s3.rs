//! The S3 language: path-style requests signed with AWS Signature Version 4.
//!
//! Path style (`/bucket/key`) is the form every S3-language service accepts,
//! so one shape of request serves Amazon, R2, B2, `MinIO` and the rest.

use std::time::{SystemTime, UNIX_EPOCH};

use sovra_core::storage::{Listing, StorageEntry, StorageError, StoragePlace, StorageSecret};

use super::{Doing, explain, listing_text, read_capped, unreachable, unreadable};
use crate::sigv4::{Signing, authorization, canonical_query, hex, sha256, stamps, uri_encode};

/// The hash S3 expects for an empty body.
const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

struct Request<'a> {
    method: reqwest::Method,
    key: &'a str,
    query: &'a [(&'a str, &'a str)],
    body: Option<(Vec<u8>, &'a str)>,
}

fn host_of(base: &url::Url) -> Result<String, StorageError> {
    match (base.host_str(), base.port()) {
        (Some(host), Some(port)) => Ok(format!("{host}:{port}")),
        (Some(host), None) => Ok(host.to_owned()),
        (None, _) => Err(unreachable()),
    }
}

async fn send(
    http: &reqwest::Client,
    place: &StoragePlace,
    secret: &StorageSecret,
    request: Request<'_>,
) -> Result<reqwest::Response, StorageError> {
    let base = url::Url::parse(&place.endpoint).map_err(|_| unreachable())?;
    let host = host_of(&base)?;
    let path = if request.key.is_empty() {
        format!("/{}", uri_encode(&place.bucket, false))
    } else {
        format!(
            "/{}/{}",
            uri_encode(&place.bucket, false),
            uri_encode(request.key, true)
        )
    };
    let query = canonical_query(request.query);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (date, time) = stamps(now);
    let payload_hash = request
        .body
        .as_ref()
        .map_or_else(|| EMPTY_HASH.to_owned(), |(bytes, _)| hex(&sha256(bytes)));
    let mut headers = Vec::with_capacity(4);
    if let Some((_, content_type)) = &request.body {
        headers.push(("content-type", (*content_type).to_owned()));
    }
    headers.push(("host", host.clone()));
    headers.push(("x-amz-content-sha256", payload_hash.clone()));
    headers.push(("x-amz-date", time.clone()));
    let signature = authorization(&Signing {
        method: request.method.as_str(),
        path: &path,
        query: &query,
        headers: &headers,
        payload_hash: &payload_hash,
        region: &place.region,
        access_key_id: &place.user,
        secret: secret.as_str(),
        date: &date,
        time: &time,
    });
    let mut address = format!("{}://{host}{path}", base.scheme());
    if !query.is_empty() {
        address.push('?');
        address.push_str(&query);
    }
    let mut builder = http
        .request(request.method, address)
        .header("x-amz-content-sha256", &payload_hash)
        .header("x-amz-date", &time)
        .header("authorization", signature);
    if let Some((bytes, content_type)) = request.body {
        builder = builder.header("content-type", content_type).body(bytes);
    }
    builder.send().await.map_err(|_| unreachable())
}

async fn refused(place: &StoragePlace, doing: Doing, response: reqwest::Response) -> StorageError {
    let status = response.status().as_u16();
    let region = response
        .headers()
        .get("x-amz-bucket-region")
        .and_then(|value| value.to_str().ok())
        .filter(|region| {
            !region.is_empty()
                && region.len() <= 40
                && region
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && *region != place.region
        })
        .map(str::to_owned);
    let body = crate::sigv4::error_text(response).await;
    explain(place, doing, status, &body, region.as_deref())
}

/// Reads a `ListObjectsV2` answer into entries inside the place's folder.
pub(super) fn parse_listing(place: &StoragePlace, xml: &str) -> Result<Listing, StorageError> {
    let document = roxmltree::Document::parse(xml).map_err(|_| unreadable())?;
    let root = document.root_element();
    if root.tag_name().name() != "ListBucketResult" {
        return Err(unreadable());
    }
    let child_text = |node: roxmltree::Node<'_, '_>, name: &str| {
        node.children()
            .find(|child| child.tag_name().name() == name)
            .and_then(|child| child.text())
            .map(str::to_owned)
    };
    let mut listing = Listing::default();
    for node in root.children().filter(roxmltree::Node::is_element) {
        match node.tag_name().name() {
            "Contents" => {
                let Some(key) = child_text(node, "Key") else {
                    continue;
                };
                // A zero-byte "folder/" object stands for an empty folder.
                if key.ends_with('/') {
                    continue;
                }
                let Some(path) = place.relative_path(&key) else {
                    continue;
                };
                listing.entries.push(StorageEntry {
                    path: path.to_owned(),
                    name: StorageEntry::name_of(path),
                    folder: false,
                    size: child_text(node, "Size").and_then(|size| size.parse().ok()),
                    modified: child_text(node, "LastModified"),
                });
            }
            "CommonPrefixes" => {
                let Some(prefix) = child_text(node, "Prefix") else {
                    continue;
                };
                let Some(path) = place.relative_path(prefix.trim_end_matches('/')) else {
                    continue;
                };
                listing.entries.push(StorageEntry {
                    path: path.to_owned(),
                    name: StorageEntry::name_of(path),
                    folder: true,
                    size: None,
                    modified: None,
                });
            }
            "IsTruncated" => listing.more = node.text() == Some("true"),
            _ => {}
        }
    }
    Ok(listing)
}

pub(super) async fn list(
    http: &reqwest::Client,
    place: &StoragePlace,
    secret: &StorageSecret,
    folder: &str,
    limit: usize,
) -> Result<Listing, StorageError> {
    let full = place.full_path(folder);
    let prefix = if full.is_empty() {
        String::new()
    } else {
        format!("{full}/")
    };
    // One more than shown, so "there is more" is known without a second call.
    let max_keys = (limit + 1).min(1000).to_string();
    let query = [
        ("list-type", "2"),
        ("delimiter", "/"),
        ("max-keys", max_keys.as_str()),
        ("prefix", prefix.as_str()),
    ];
    let response = send(
        http,
        place,
        secret,
        Request {
            method: reqwest::Method::GET,
            key: "",
            query: &query,
            body: None,
        },
    )
    .await?;
    if !response.status().is_success() {
        return Err(refused(place, Doing::Looking, response).await);
    }
    parse_listing(place, &listing_text(response).await?)
}

pub(super) async fn read(
    http: &reqwest::Client,
    place: &StoragePlace,
    secret: &StorageSecret,
    path: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, StorageError> {
    let key = place.full_path(path);
    let response = send(
        http,
        place,
        secret,
        Request {
            method: reqwest::Method::GET,
            key: &key,
            query: &[],
            body: None,
        },
    )
    .await?;
    if !response.status().is_success() {
        return Err(refused(place, Doing::Reading, response).await);
    }
    read_capped(response, max_bytes).await
}

pub(super) async fn write(
    http: &reqwest::Client,
    place: &StoragePlace,
    secret: &StorageSecret,
    path: &str,
    bytes: Vec<u8>,
    content_type: &str,
) -> Result<(), StorageError> {
    let key = place.full_path(path);
    let response = send(
        http,
        place,
        secret,
        Request {
            method: reqwest::Method::PUT,
            key: &key,
            query: &[],
            body: Some((bytes, content_type)),
        },
    )
    .await?;
    if !response.status().is_success() {
        return Err(refused(place, Doing::Saving, response).await);
    }
    Ok(())
}
