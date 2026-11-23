//! The storage services the form offers, each with the words and starting
//! values that make its setup a matter of copying a few things across.
//!
//! A new service is one [`StorageService`] variant plus one arm in each match
//! here. Services that speak the same language share a [`StorageProtocol`], so
//! only the protocol decides how Sovra talks to them.

use serde::{Deserialize, Serialize};

/// How Sovra talks to a service. One adapter per protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageProtocol {
    /// Amazon's storage language, which many other services speak too.
    S3,
    /// The web folder language Nextcloud, ownCloud and most home file servers speak.
    WebDav,
    /// Microsoft Azure Blob Storage, reached with a shared access signature.
    AzureBlob,
}

/// Which kind of storage a place is. Order is the order of the form.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum StorageService {
    #[default]
    AmazonS3,
    CloudflareR2,
    BackblazeB2,
    Wasabi,
    DigitalOceanSpaces,
    GoogleCloudStorage,
    Hetzner,
    Scaleway,
    Linode,
    MinIo,
    OtherS3,
    AzureBlob,
    Nextcloud,
    OwnCloud,
    OtherWebDav,
}

/// How the address of a service is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressRule {
    /// Always the same address.
    Fixed(&'static str),
    /// Made from the region: `{region}` is replaced.
    FromRegion(&'static str),
    /// Made from the account or user name: `{user}` is replaced.
    FromUser(&'static str),
    /// The person types it; the text is an example.
    Typed(&'static str),
}

impl StorageService {
    pub const ALL: [Self; 15] = [
        Self::AmazonS3,
        Self::CloudflareR2,
        Self::BackblazeB2,
        Self::Wasabi,
        Self::DigitalOceanSpaces,
        Self::GoogleCloudStorage,
        Self::Hetzner,
        Self::Scaleway,
        Self::Linode,
        Self::MinIo,
        Self::OtherS3,
        Self::AzureBlob,
        Self::Nextcloud,
        Self::OwnCloud,
        Self::OtherWebDav,
    ];

    pub const fn protocol(self) -> StorageProtocol {
        match self {
            Self::AmazonS3
            | Self::CloudflareR2
            | Self::BackblazeB2
            | Self::Wasabi
            | Self::DigitalOceanSpaces
            | Self::GoogleCloudStorage
            | Self::Hetzner
            | Self::Scaleway
            | Self::Linode
            | Self::MinIo
            | Self::OtherS3 => StorageProtocol::S3,
            Self::AzureBlob => StorageProtocol::AzureBlob,
            Self::Nextcloud | Self::OwnCloud | Self::OtherWebDav => StorageProtocol::WebDav,
        }
    }

    /// A stable name for control IDs.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::AmazonS3 => "amazon-s3",
            Self::CloudflareR2 => "cloudflare-r2",
            Self::BackblazeB2 => "backblaze-b2",
            Self::Wasabi => "wasabi",
            Self::DigitalOceanSpaces => "digitalocean-spaces",
            Self::GoogleCloudStorage => "google-cloud-storage",
            Self::Hetzner => "hetzner",
            Self::Scaleway => "scaleway",
            Self::Linode => "linode",
            Self::MinIo => "minio",
            Self::OtherS3 => "other-s3",
            Self::AzureBlob => "azure-blob",
            Self::Nextcloud => "nextcloud",
            Self::OwnCloud => "owncloud",
            Self::OtherWebDav => "other-webdav",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::AmazonS3 => "Amazon S3",
            Self::CloudflareR2 => "Cloudflare R2",
            Self::BackblazeB2 => "Backblaze B2",
            Self::Wasabi => "Wasabi",
            Self::DigitalOceanSpaces => "DigitalOcean Spaces",
            Self::GoogleCloudStorage => "Google Cloud Storage",
            Self::Hetzner => "Hetzner Object Storage",
            Self::Scaleway => "Scaleway Object Storage",
            Self::Linode => "Akamai (Linode) Object Storage",
            Self::MinIo => "MinIO",
            Self::OtherS3 => "Another S3-compatible service",
            Self::AzureBlob => "Azure Blob Storage",
            Self::Nextcloud => "Nextcloud",
            Self::OwnCloud => "ownCloud",
            Self::OtherWebDav => "A WebDAV file server or NAS",
        }
    }

    /// One plain sentence under the name on the form.
    pub const fn sentence(self) -> &'static str {
        match self {
            Self::AmazonS3 => "Amazon's storage buckets.",
            Self::CloudflareR2 => "Cloudflare's buckets, with no fee to download.",
            Self::BackblazeB2 => "Low-cost buckets from Backblaze.",
            Self::Wasabi => "Low-cost buckets from Wasabi.",
            Self::DigitalOceanSpaces => "Buckets on DigitalOcean.",
            Self::GoogleCloudStorage => "Google Cloud buckets, with an HMAC key.",
            Self::Hetzner => "Buckets in Hetzner's European data centres.",
            Self::Scaleway => "Buckets in Scaleway's European data centres.",
            Self::Linode => "Buckets on Akamai's Linode cloud.",
            Self::MinIo => "Your own MinIO server, at home or at work.",
            Self::OtherS3 => "Any other service that speaks the S3 language.",
            Self::AzureBlob => "A container in a Microsoft Azure storage account.",
            Self::Nextcloud => "Files on your Nextcloud.",
            Self::OwnCloud => "Files on your ownCloud.",
            Self::OtherWebDav => "Synology, QNAP, Box or any server that speaks WebDAV.",
        }
    }

    pub const fn address(self) -> AddressRule {
        match self {
            Self::AmazonS3 => AddressRule::FromRegion("https://s3.{region}.amazonaws.com"),
            Self::CloudflareR2 => {
                AddressRule::Typed("https://<account id>.r2.cloudflarestorage.com")
            }
            Self::BackblazeB2 => AddressRule::FromRegion("https://s3.{region}.backblazeb2.com"),
            Self::Wasabi => AddressRule::FromRegion("https://s3.{region}.wasabisys.com"),
            Self::DigitalOceanSpaces => {
                AddressRule::FromRegion("https://{region}.digitaloceanspaces.com")
            }
            Self::GoogleCloudStorage => AddressRule::Fixed("https://storage.googleapis.com"),
            Self::Hetzner => AddressRule::FromRegion("https://{region}.your-objectstorage.com"),
            Self::Scaleway => AddressRule::FromRegion("https://s3.{region}.scw.cloud"),
            Self::Linode => AddressRule::FromRegion("https://{region}.linodeobjects.com"),
            Self::MinIo => AddressRule::Typed("https://minio.example.com"),
            Self::OtherS3 => AddressRule::Typed("https://storage.example.com"),
            Self::AzureBlob => AddressRule::FromUser("https://{user}.blob.core.windows.net"),
            Self::Nextcloud | Self::OwnCloud => {
                AddressRule::Typed("https://cloud.example.com/remote.php/dav/files/<username>/")
            }
            Self::OtherWebDav => AddressRule::Typed("https://nas.example.com:5006/home/"),
        }
    }

    /// The region filled in for a new place. Empty when the protocol has none.
    pub const fn default_region(self) -> &'static str {
        match self {
            Self::AmazonS3
            | Self::Wasabi
            | Self::GoogleCloudStorage
            | Self::MinIo
            | Self::Linode
            | Self::OtherS3 => "us-east-1",
            Self::CloudflareR2 => "auto",
            Self::BackblazeB2 => "us-west-004",
            Self::DigitalOceanSpaces => "nyc3",
            Self::Hetzner => "fsn1",
            Self::Scaleway => "fr-par",
            Self::AzureBlob | Self::Nextcloud | Self::OwnCloud | Self::OtherWebDav => "",
        }
    }

    /// Whether the form asks for a region.
    pub const fn has_region(self) -> bool {
        matches!(self.protocol(), StorageProtocol::S3)
    }

    /// What the bucket field is called; `None` when the service has none.
    pub const fn bucket_label(self) -> Option<&'static str> {
        match self.protocol() {
            StorageProtocol::S3 => Some("Bucket name"),
            StorageProtocol::AzureBlob => Some("Container name"),
            StorageProtocol::WebDav => None,
        }
    }

    /// What the name-like half of the sign-in is called.
    pub const fn user_label(self) -> &'static str {
        match self.protocol() {
            StorageProtocol::S3 => "Access key ID",
            StorageProtocol::AzureBlob => "Storage account name",
            StorageProtocol::WebDav => "Username",
        }
    }

    /// What the private half of the sign-in is called.
    pub const fn secret_label(self) -> &'static str {
        match self.protocol() {
            StorageProtocol::S3 => "Secret access key",
            StorageProtocol::AzureBlob => "Shared access signature (SAS)",
            StorageProtocol::WebDav => "Password or app password",
        }
    }

    /// Where the person makes the keys this service needs, in one sentence.
    pub const fn keys_help(self) -> &'static str {
        match self {
            Self::AmazonS3 => {
                "In the AWS console, open IAM, choose a user, then Security credentials and Create access key."
            }
            Self::CloudflareR2 => {
                "In the Cloudflare dashboard, open R2, then Manage API tokens. The address is shown on the bucket's Settings page."
            }
            Self::BackblazeB2 => {
                "In Backblaze, open Application Keys and add a key. The region is part of the bucket's Endpoint."
            }
            Self::Wasabi => "In the Wasabi console, open Access Keys and create a new key.",
            Self::DigitalOceanSpaces => {
                "In DigitalOcean, open Spaces Object Storage, then Access Keys, and create a key."
            }
            Self::GoogleCloudStorage => {
                "In Google Cloud, open Cloud Storage, Settings, then Interoperability, and create an HMAC key."
            }
            Self::Hetzner => {
                "In the Hetzner Console, open your project's Security page and create S3 credentials."
            }
            Self::Scaleway => {
                "In the Scaleway console, open IAM, then API keys, and generate a key for Object Storage."
            }
            Self::Linode => {
                "In Akamai Cloud Manager, open Object Storage, then Access Keys, and create a key."
            }
            Self::MinIo => "In the MinIO console, open Access Keys and create one.",
            Self::OtherS3 => "Your storage service's settings list its access keys and address.",
            Self::AzureBlob => {
                "In the Azure portal, open the container, choose Shared access tokens, allow Read and List (and Write to save), then copy the Blob SAS token."
            }
            Self::Nextcloud => {
                "In Nextcloud, open Settings, Security, and create an app password. Files settings show the WebDAV address."
            }
            Self::OwnCloud => {
                "In ownCloud, open Settings, Security, and create an app password. Files settings show the WebDAV address."
            }
            Self::OtherWebDav => {
                "Your file server's settings show its WebDAV address. Turn WebDAV on there if it is off."
            }
        }
    }

    /// The address for `region` or `user`, when the service decides it.
    pub fn suggested_address(self, region: &str, user: &str) -> Option<String> {
        let region = region.trim();
        let user = user.trim();
        match self.address() {
            AddressRule::Fixed(address) => Some(address.to_owned()),
            AddressRule::FromRegion(template) if !region.is_empty() => {
                Some(template.replace("{region}", region))
            }
            AddressRule::FromUser(template) if !user.is_empty() => {
                Some(template.replace("{user}", &user.to_ascii_lowercase()))
            }
            AddressRule::FromRegion(_) | AddressRule::FromUser(_) | AddressRule::Typed(_) => None,
        }
    }

    /// An example address for the field's placeholder.
    pub fn address_example(self) -> &'static str {
        match self.address() {
            AddressRule::Fixed(text)
            | AddressRule::FromRegion(text)
            | AddressRule::FromUser(text)
            | AddressRule::Typed(text) => text,
        }
    }
}
