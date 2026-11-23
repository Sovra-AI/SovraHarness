//! Azure Blob Storage, reached with a shared access signature (SAS).
//!
//! A SAS is made in the Azure portal for one container with only the rights
//! the person picks, so Sovra never holds the storage account's master key.
//! It travels in the query string, as Azure expects.

use sovra_core::storage::{Listing, StorageEntry, StorageError, StoragePlace, StorageSecret};

use super::{Doing, explain, listing_text, read_capped, unreachable, unreadable};
use crate::sigv4::uri_encode;

/// The Blob service version Sovra speaks.
const VERSION: &str = "2021-08-06";

fn address(
    place: &StoragePlace,
    blob: &str,
    query: &[(&str, &str)],
    secret: &StorageSecret,
) -> String {
    let mut address = format!(
        "{}/{}",
        place.endpoint.trim_end_matches('/'),
        uri_encode(&place.bucket, false)
    );
    if !blob.is_empty() {
        address.push('/');
        address.push_str(&uri_encode(blob, true));
    }
    address.push('?');
    for (name, value) in query {
        address.push_str(&uri_encode(name, false));
        address.push('=');
        address.push_str(&uri_encode(value, false));
        address.push('&');
    }
    address.push_str(secret.as_str());
    address
}

async fn refused(place: &StoragePlace, doing: Doing, response: reqwest::Response) -> StorageError {
    let status = response.status().as_u16();
    let body = crate::sigv4::error_text(response).await;
    explain(place, doing, status, &body, None)
}

fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| child.tag_name().name() == name)
}

/// Reads a List Blobs answer into entries inside the place's folder.
pub(super) fn parse_listing(place: &StoragePlace, xml: &str) -> Result<Listing, StorageError> {
    let document = roxmltree::Document::parse(xml).map_err(|_| unreadable())?;
    let root = document.root_element();
    if root.tag_name().name() != "EnumerationResults" {
        return Err(unreadable());
    }
    let text = |node: Option<roxmltree::Node<'_, '_>>| {
        node.and_then(|node| node.text()).map(str::to_owned)
    };
    let mut listing = Listing {
        entries: Vec::new(),
        more: text(child(root, "NextMarker")).is_some_and(|marker| !marker.is_empty()),
    };
    let Some(blobs) = child(root, "Blobs") else {
        return Ok(listing);
    };
    for node in blobs.children().filter(roxmltree::Node::is_element) {
        let Some(name) = text(child(node, "Name")) else {
            continue;
        };
        let folder = match node.tag_name().name() {
            "Blob" => false,
            "BlobPrefix" => true,
            _ => continue,
        };
        if !folder && name.ends_with('/') {
            continue;
        }
        let Some(path) = place.relative_path(name.trim_end_matches('/')) else {
            continue;
        };
        let properties = child(node, "Properties");
        listing.entries.push(StorageEntry {
            path: path.to_owned(),
            name: StorageEntry::name_of(path),
            folder,
            size: properties
                .and_then(|p| text(child(p, "Content-Length")))
                .and_then(|size| size.parse().ok()),
            modified: properties.and_then(|p| text(child(p, "Last-Modified"))),
        });
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
    let most = (limit + 1).min(5000).to_string();
    let query = [
        ("restype", "container"),
        ("comp", "list"),
        ("delimiter", "/"),
        ("maxresults", most.as_str()),
        ("prefix", prefix.as_str()),
    ];
    let response = http
        .get(address(place, "", &query, secret))
        .header("x-ms-version", VERSION)
        .send()
        .await
        .map_err(|_| unreachable())?;
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
    let response = http
        .get(address(place, &place.full_path(path), &[], secret))
        .header("x-ms-version", VERSION)
        .send()
        .await
        .map_err(|_| unreachable())?;
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
    let response = http
        .put(address(place, &place.full_path(path), &[], secret))
        .header("x-ms-version", VERSION)
        .header("x-ms-blob-type", "BlockBlob")
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|_| unreachable())?;
    if !response.status().is_success() {
        return Err(refused(place, Doing::Saving, response).await);
    }
    Ok(())
}
