use sovra_core::storage::{StoragePlace, StorageService};

use super::*;

fn place(service: StorageService, folder: &str) -> StoragePlace {
    StoragePlace {
        id: "place-1".into(),
        name: "Test".into(),
        bucket: "photos".into(),
        user: "sam".into(),
        folder: folder.into(),
        ..StoragePlace::new(service)
    }
}

#[test]
fn s3_listings_keep_folders_files_and_more() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>photos</Name><Prefix>sovra/</Prefix><KeyCount>4</KeyCount>
  <IsTruncated>true</IsTruncated>
  <Contents><Key>sovra/</Key><Size>0</Size></Contents>
  <Contents><Key>sovra/notes &amp; plans.md</Key><LastModified>2026-11-01T10:00:00.000Z</LastModified><Size>42</Size></Contents>
  <Contents><Key>elsewhere/secret.txt</Key><Size>1</Size></Contents>
  <CommonPrefixes><Prefix>sovra/2026/</Prefix></CommonPrefixes>
</ListBucketResult>"#;
    let listing = s3::parse_listing(&place(StorageService::AmazonS3, "sovra"), xml)
        .expect("listing")
        .tidy("", 50);
    assert!(listing.more);
    let entries: Vec<_> = listing
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.folder, e.size))
        .collect();
    assert_eq!(
        entries,
        [("2026", true, None), ("notes & plans.md", false, Some(42))]
    );
    assert_eq!(
        listing.entries[1].modified.as_deref(),
        Some("2026-11-01T10:00:00.000Z")
    );
    assert!(s3::parse_listing(&place(StorageService::AmazonS3, ""), "<Error/>").is_err());
    assert!(s3::parse_listing(&place(StorageService::AmazonS3, ""), "not xml").is_err());
}

#[test]
fn webdav_listings_decode_names_and_skip_the_folder_itself() {
    let mut nextcloud = place(StorageService::Nextcloud, "Documents");
    nextcloud.endpoint = "https://cloud.example.com/remote.php/dav/files/sam/".into();
    let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
 <d:response><d:href>/remote.php/dav/files/sam/Documents/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
 <d:response><d:href>/remote.php/dav/files/sam/Documents/Tax%202026/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>https://cloud.example.com/remote.php/dav/files/sam/Documents/caf%C3%A9.txt</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>12</d:getcontentlength>
  <d:getlastmodified>Sun, 01 Nov 2026 10:00:00 GMT</d:getlastmodified></d:prop></d:propstat></d:response>
</d:multistatus>"#;
    let listing = webdav::parse_listing(&nextcloud, xml)
        .expect("listing")
        .tidy("", 50);
    let entries: Vec<_> = listing
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.folder, e.size))
        .collect();
    assert_eq!(
        entries,
        [("Tax 2026", true, None), ("café.txt", false, Some(12))]
    );
}

#[test]
fn azure_listings_read_blobs_and_prefixes() {
    let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<EnumerationResults ServiceEndpoint="https://acct.blob.core.windows.net/" ContainerName="photos">
  <Prefix>sovra/</Prefix><Delimiter>/</Delimiter>
  <Blobs>
    <Blob><Name>sovra/report.pdf</Name><Properties><Last-Modified>Sun, 01 Nov 2026 10:00:00 GMT</Last-Modified><Content-Length>2048</Content-Length></Properties></Blob>
    <BlobPrefix><Name>sovra/old/</Name></BlobPrefix>
  </Blobs>
  <NextMarker>2!abc</NextMarker>
</EnumerationResults>"#;
    let listing = azure::parse_listing(&place(StorageService::AzureBlob, "sovra"), xml)
        .expect("listing")
        .tidy("", 50);
    assert!(listing.more);
    let entries: Vec<_> = listing
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.folder, e.size))
        .collect();
    assert_eq!(
        entries,
        [("old", true, None), ("report.pdf", false, Some(2048))]
    );
}

#[test]
fn refusals_are_explained_without_repeating_the_service() {
    let s3 = place(StorageService::AmazonS3, "");
    let denied = "<Error><Code>AccessDenied</Code><Message>private detail</Message></Error>";
    let message = explain(&s3, Doing::Saving, 403, denied, None).to_string();
    assert!(message.contains("save files"), "{message}");
    assert!(message.contains("403 AccessDenied") && !message.contains("private detail"));
    let keys = explain(
        &s3,
        Doing::Looking,
        403,
        "<Code>SignatureDoesNotMatch</Code>",
        None,
    )
    .to_string();
    assert!(
        keys.contains("access key ID") && keys.contains("secret access key"),
        "{keys}"
    );
    let region = explain(&s3, Doing::Looking, 301, "", Some("eu-west-2")).to_string();
    assert!(region.contains("eu-west-2 region"), "{region}");
    assert!(
        explain(&s3, Doing::Reading, 404, "<Code>NoSuchKey</Code>", None)
            .to_string()
            .contains("file was not found")
    );
    let azure = place(StorageService::AzureBlob, "");
    assert!(
        explain(
            &azure,
            Doing::Looking,
            404,
            "<Code>ContainerNotFound</Code>",
            None
        )
        .to_string()
        .contains("container")
    );
    let dav = place(StorageService::Nextcloud, "");
    assert!(
        explain(&dav, Doing::Looking, 401, "", None)
            .to_string()
            .contains("username and the password or app password")
    );
    assert!(
        explain(&dav, Doing::Looking, 405, "", None)
            .to_string()
            .contains("WebDAV")
    );
}

#[test]
fn percent_escapes_are_decoded_and_bad_ones_kept() {
    assert_eq!(percent_decode("a%20b%C3%A9"), "a bé");
    assert_eq!(percent_decode("100%"), "100%");
    assert_eq!(percent_decode("%zz%4"), "%zz%4");
}
