//! `WebDAV`: `PROPFIND` to list, `GET` to read, `PUT` to save, with the
//! username and password sent as HTTP Basic sign-in over HTTPS.

use sovra_core::storage::{Listing, StorageEntry, StorageError, StoragePlace, StorageSecret};

use super::{Doing, explain, listing_text, percent_decode, read_capped, unreachable, unreadable};
use crate::sigv4::uri_encode;

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

/// The address of `path` inside the place, each part percent-encoded. A
/// folder's address ends in a slash.
fn address(place: &StoragePlace, path: &str, folder: bool) -> String {
    let full = place.full_path(path);
    let mut address = place.endpoint.clone();
    if !address.ends_with('/') {
        address.push('/');
    }
    address.push_str(&uri_encode(&full, true));
    if folder && !full.is_empty() {
        address.push('/');
    }
    address
}

fn sign_in(
    request: reqwest::RequestBuilder,
    place: &StoragePlace,
    secret: &StorageSecret,
) -> reqwest::RequestBuilder {
    request.basic_auth(&place.user, Some(secret.as_str()))
}

/// The decoded path part of the place's own address, which every `href` in a
/// listing starts with.
fn base_path(place: &StoragePlace) -> String {
    url::Url::parse(&place.endpoint)
        .map(|url| percent_decode(url.path()))
        .unwrap_or_default()
}

/// Reads a `207 Multi-Status` answer into entries inside the place's folder.
pub(super) fn parse_listing(place: &StoragePlace, xml: &str) -> Result<Listing, StorageError> {
    let document = roxmltree::Document::parse(xml).map_err(|_| unreadable())?;
    let root = document.root_element();
    if root.tag_name().name() != "multistatus" {
        return Err(unreadable());
    }
    let base = base_path(place);
    let base = base.trim_end_matches('/');
    let mut listing = Listing::default();
    for response in root
        .descendants()
        .filter(|node| node.tag_name().name() == "response")
    {
        let Some(href) = response
            .children()
            .find(|node| node.tag_name().name() == "href")
            .and_then(|node| node.text())
        else {
            continue;
        };
        // Some servers answer with a full address, most with only the path.
        let href = url::Url::parse(href)
            .map_or_else(|_| href.trim().to_owned(), |url| url.path().to_owned());
        let decoded = percent_decode(&href);
        let Some(inside) = decoded.strip_prefix(base) else {
            continue;
        };
        let inside = inside.trim_matches('/');
        let Some(path) = place.relative_path(inside) else {
            continue;
        };
        let prop = |name: &str| {
            response
                .descendants()
                .find(|node| node.tag_name().name() == name)
        };
        let folder = prop("resourcetype").is_some_and(|node| {
            node.children()
                .any(|child| child.tag_name().name() == "collection")
        });
        listing.entries.push(StorageEntry {
            path: path.to_owned(),
            name: StorageEntry::name_of(path),
            folder,
            size: if folder {
                None
            } else {
                prop("getcontentlength")
                    .and_then(|node| node.text())
                    .and_then(|size| size.trim().parse().ok())
            },
            modified: prop("getlastmodified")
                .and_then(|node| node.text())
                .map(|text| text.trim().to_owned()),
        });
    }
    Ok(listing)
}

pub(super) async fn list(
    http: &reqwest::Client,
    place: &StoragePlace,
    secret: &StorageSecret,
    folder: &str,
) -> Result<Listing, StorageError> {
    let method = reqwest::Method::from_bytes(b"PROPFIND").map_err(|_| unreachable())?;
    let response = sign_in(
        http.request(method, address(place, folder, true)),
        place,
        secret,
    )
    .header("depth", "1")
    .header("content-type", "application/xml; charset=utf-8")
    .body(PROPFIND_BODY)
    .send()
    .await
    .map_err(|_| unreachable())?;
    let status = response.status().as_u16();
    if status != 207 {
        let body = crate::sigv4::error_text(response).await;
        // A plain web page answers 200 to anything; it is not a WebDAV folder.
        let status = if status == 200 { 405 } else { status };
        return Err(explain(place, Doing::Looking, status, &body, None));
    }
    let full = place.full_path(folder);
    let mut listing = parse_listing(place, &listing_text(response).await?)?;
    // The folder lists itself first; that is not one of its entries.
    let own = place.relative_path(&full).unwrap_or_default().to_owned();
    listing.entries.retain(|entry| entry.path != own);
    Ok(listing)
}

pub(super) async fn read(
    http: &reqwest::Client,
    place: &StoragePlace,
    secret: &StorageSecret,
    path: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, StorageError> {
    let response = sign_in(http.get(address(place, path, false)), place, secret)
        .send()
        .await
        .map_err(|_| unreachable())?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = crate::sigv4::error_text(response).await;
        return Err(explain(place, Doing::Reading, status, &body, None));
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
    // WebDAV does not make folders on its own, so each one on the way is made
    // first. One that is already there answers 405, which is fine.
    let mkcol = reqwest::Method::from_bytes(b"MKCOL").map_err(|_| unreachable())?;
    let mut folders: Vec<&str> = place.folder.split('/').filter(|p| !p.is_empty()).collect();
    let inner: Vec<&str> = path.split('/').collect();
    let mut made = String::new();
    folders.extend(inner.iter().take(inner.len().saturating_sub(1)));
    for part in folders {
        if !made.is_empty() {
            made.push('/');
        }
        made.push_str(part);
        let mut url = place.endpoint.clone();
        url.push_str(&uri_encode(&made, true));
        url.push('/');
        let response = sign_in(http.request(mkcol.clone(), url), place, secret)
            .send()
            .await
            .map_err(|_| unreachable())?;
        let status = response.status().as_u16();
        if !(response.status().is_success() || status == 405) {
            let body = crate::sigv4::error_text(response).await;
            return Err(explain(place, Doing::Saving, status, &body, None));
        }
    }
    let response = sign_in(http.put(address(place, path, false)), place, secret)
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|_| unreachable())?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = crate::sigv4::error_text(response).await;
        return Err(explain(place, Doing::Saving, status, &body, None));
    }
    Ok(())
}
