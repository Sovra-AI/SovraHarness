# Online storage

How a person links a storage bucket or file server from **Connections →
Online storage**, and what Sovra can then do with the files there.

A *place* is one bucket, one Azure container, or one web folder. You can link
up to 32 places, each from a different service or the same one. Sovra lists,
finds and reads files there, and saves new files only in places where you
turn saving on.

## Services

| Kind | Services on the form | What you paste |
| --- | --- | --- |
| Storage buckets (the S3 language) | Amazon S3, Cloudflare R2, Backblaze B2, Wasabi, DigitalOcean Spaces, Google Cloud Storage (HMAC key), Hetzner, Scaleway, Akamai (Linode), MinIO, any other S3-compatible service | Region, bucket name, access key ID, secret access key. The address is filled in from the region where the service decides it. |
| Microsoft Azure | Azure Blob Storage | Storage account name, container name, and a shared access signature (SAS) for that container. |
| File servers (WebDAV) | Nextcloud, ownCloud, Synology, QNAP, Box and any server that speaks WebDAV | The folder's WebDAV address, username, and a password or app password. |

Each service's form says in one sentence where its keys are made. An optional
**Only this folder** keeps Sovra inside one folder of the place.

### Addresses Sovra accepts

- Addresses must start with `https://`.
- Plain `http://` is accepted for a service on this computer. For MinIO, other
  S3-compatible services and WebDAV servers it is also accepted on your home or
  office network: private IP addresses and names ending in `.local`, `.lan` or
  `.home.arpa`. A password sent over plain http can be read by anyone on that
  network.
- An S3 address is the service alone, with nothing after it. Requests use the
  path form (`/bucket/file`), which every S3-language service accepts.
- An example address from the form that still holds `<…>` is refused.

## What the person sees

| State | Row shows | Actions |
| --- | --- | --- |
| Not checked yet / Checking… | Name, bucket and service | Browse files · Check · Edit · Unlink |
| Working | A green tick | Same |
| Needs you | The service's refusal in plain words | Same; **Edit** to fix the keys |

- **Check and link** first asks the service for the top of the place. A place
  is only kept once the service says yes, so a typo in a key never leaves a
  broken place behind. A refusal stays on the form with its reason.
- Every linked place is checked again when Sovra starts and when you press
  **Check links** on Connections.
- **Edit** keeps the saved secret when its box is left empty.
- **Unlink** asks twice. It removes the place and its secret from this
  computer. The files in the place are not touched.
- **Browse files** shows one folder at a time, folders first, up to 200
  entries, with **Up** and **Close**.

## What Sovra can do with it

While at least one place is linked, the **Online storage helper**
(`StorageAgent`) is offered to the assistant with four tools:

| Tool | Does |
| --- | --- |
| `storage_list_places` | The linked places, their service, and whether saving is on |
| `storage_list_files` | One folder's files and folders |
| `storage_read_file` | A file's text, up to 20 MB. Text files are read as they are; PDF, Word, Excel, PowerPoint and other documents go through Sovra's document reader. The model sees at most 40,000 characters at once and is told when there is more. |
| `storage_save_file` | Saves text as a file (up to 5 MB) when you ask, in a place with **Let Sovra save files here when I ask** on. A file already at that path is kept unless you agree to replace it. |

Text read from a place is marked as untrusted: the assistant treats it as
information and never follows instructions inside it. Conversations from
WhatsApp or Telegram that do not have private access do not get these tools.

## Privacy and security

- Secrets (secret access keys, SAS tokens, passwords) live in the operating
  system's keychain, like model keys. The development exception for
  `cargo xtask dev` builds is described in [Configuration](configuration.md).
- The list of places is an owner-only file, `storage/places.json`, in Sovra's
  data folder. It never holds a secret.
- Requests go only to the address you typed. Redirects are refused, so a
  signed request or password is never replayed to another host.
- An error repeats only the service's status and short error name, such as
  `403 AccessDenied`, never its free text.
- For Azure, make the SAS for one container with only the rights you want:
  Read and List to look, plus Write to save.

## Limits

- Saving writes text files. Sovra does not upload pictures or other binary
  files from chat into a place.
- One file is saved in one request, so the 5 MB cap stays well inside every
  service's single-upload limit.
- SFTP, Dropbox and Box's own sign-in are not built. Google Drive and OneDrive
  are reached through the [Google](google.md) and
  [Microsoft 365](microsoft-365.md) connections.
- Azure is reached with a SAS only, not with the storage account key.

## For developers

| Piece | Where |
| --- | --- |
| Places, services, validation, requests, the `StorageConnector` port, the page's reducer | `crates/sovra-core/src/storage.rs` |
| HTTP adapter: S3 (SigV4, shared with the training uploader), WebDAV, Azure Blob | `crates/sovra-providers/src/online_storage/` and `sigv4.rs` |
| Saved list, keychain, checks, browsing, tool calls | `crates/sovra-runtime/src/storage.rs` |
| Agent tools | `crates/sovra-adk/src/storage.rs` |
| Page | `crates/sovra-ui/src/storage_view.rs` |

A new S3-language service is one `StorageService` variant and its words in
`crates/sovra-core/src/storage/services.rs`; nothing else changes.

To check the adapter against a real service or a local stand-in:

```text
cargo run -p sovra-app --example storage_selftest -- <service> <endpoint> <region> <bucket> <user> <secret> [folder]
```

It lists, saves a file with spaces and accents in its name, reads it back,
lists again, reads a missing file and tries a wrong secret. It has been run
against `rclone serve s3`, SeaweedFS, `rclone serve webdav` and Azurite.
