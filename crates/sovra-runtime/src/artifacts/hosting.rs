//! Bounded loopback hosting of an immutable saved HTML snapshot. No filesystem
//! routing or shell execution: generated JavaScript runs only in the browser.
//!
//! Each saved website keeps one address. It is served on a port worked out from
//! its artifact id (falling back to any free port if that one is taken), and
//! opening or running it again swaps the files in place instead of moving it.
//! So a link the person or the assistant was given keeps working, including
//! after Sovra restarts and the site is opened again.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

/// Below every common OS's ephemeral range (Linux starts at 32768, Windows and
/// macOS at 49152), so a preferred port rarely collides with outgoing connections.
const STABLE_PORTS: std::ops::Range<u16> = 20_000..30_000;

pub(super) struct Asset {
    pub(super) bytes: Vec<u8>,
    pub(super) mime: &'static str,
}

/// What a site serves now. Replaced as a whole, so a request sees one version.
struct Content {
    files: BTreeMap<String, Asset>,
    spa: bool,
}

type Shared = Arc<RwLock<Arc<Content>>>;

pub(super) struct Website {
    pub(super) url: String,
    content: Shared,
    task: JoinHandle<()>,
}

impl Drop for Website {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The port this artifact asks for first: the same id always asks for the same one.
pub(super) fn preferred_port(id: &str) -> u16 {
    // FNV-1a: stable across runs and builds, unlike the std hasher.
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let span = u64::from(STABLE_PORTS.end - STABLE_PORTS.start);
    STABLE_PORTS.start + u16::try_from(hash % span).unwrap_or_default()
}

pub(super) fn page(content: String) -> BTreeMap<String, Asset> {
    BTreeMap::from([(
        "index.html".into(),
        Asset {
            bytes: content.into_bytes(),
            mime: "text/html; charset=utf-8",
        },
    )])
}

/// Serves `files` for artifact `id` and returns its address. A site already running
/// for `id` keeps its address and just gets the new files.
pub(super) async fn host(
    websites: &mut BTreeMap<String, Website>,
    id: &str,
    files: BTreeMap<String, Asset>,
    spa: bool,
) -> std::io::Result<String> {
    if let Some(site) = websites.get(id) {
        site.replace(files, spa);
        return Ok(site.url.clone());
    }
    let site = Website::start_files(files, spa, preferred_port(id)).await?;
    let url = site.url.clone();
    websites.insert(id.to_owned(), site);
    Ok(url)
}

impl Website {
    pub(super) async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }

    /// Serves new files at the same address.
    pub(super) fn replace(&self, files: BTreeMap<String, Asset>, spa: bool) {
        *self.content.write().unwrap_or_else(PoisonError::into_inner) =
            Arc::new(Content { files, spa });
    }

    pub(super) async fn start_files(
        files: BTreeMap<String, Asset>,
        spa: bool,
        preferred_port: u16,
    ) -> std::io::Result<Self> {
        let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, preferred_port)).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::debug!(%error, preferred_port, "website port taken; using another");
                TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?
            }
        };
        let address = listener.local_addr()?;
        let content: Shared = Arc::new(RwLock::new(Arc::new(Content { files, spa })));
        let shared = content.clone();
        let task = tokio::runtime::Handle::current().spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept(), if clients.len() < 16 => {
                        let Ok((stream, _)) = accepted else { break };
                        let content = shared.read().unwrap_or_else(PoisonError::into_inner).clone();
                        clients.spawn(async move {
                            let _ = tokio::time::timeout(
                                Duration::from_secs(5), serve(stream, address, &content),
                            ).await;
                        });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {}
                }
            }
        });
        Ok(Self {
            url: format!("http://{address}/"),
            content,
            task,
        })
    }
}

async fn serve(
    mut stream: TcpStream,
    address: SocketAddr,
    content: &Content,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Ok(());
        }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > 8192 {
            return Ok(());
        }
    }
    let text = String::from_utf8_lossy(&request);
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default();
    let path = first.next().unwrap_or_default();
    // Host validation prevents arbitrary DNS names rebinding to this server: only
    // the loopback names, with this exact port, are served.
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    let host_ok = match hosts.as_slice() {
        [host] => allowed_host(host, address.port()),
        _ => false,
    };
    let accepts_html = text.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("accept") && value.contains("text/html")
        })
    });
    let path = request_path(path);
    let asset = path.as_ref().and_then(|path| {
        content.files.get(path).or_else(|| {
            (content.spa && accepts_html)
                .then(|| content.files.get("index.html"))
                .flatten()
        })
    });
    let (status, body, mime): (&str, &[u8], &str) = if !host_ok {
        (
            "403 Forbidden",
            b"This website is available only at its local address.",
            "text/plain",
        )
    } else if method != "GET" && method != "HEAD" {
        (
            "405 Method Not Allowed",
            b"Only GET and HEAD are supported.",
            "text/plain",
        )
    } else if let Some(asset) = asset {
        ("200 OK", &asset.bytes, asset.mime)
    } else {
        ("404 Not Found", b"Page not found.", "text/plain")
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    if method != "HEAD" {
        stream.write_all(body).await?;
    }
    stream.shutdown().await
}

/// `127.0.0.1:PORT` or `localhost:PORT`, any letter case, for this server's port.
fn allowed_host(host: &str, port: u16) -> bool {
    let Some((name, given)) = host.rsplit_once(':') else {
        return false;
    };
    given == port.to_string() && (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost"))
}

// Decode URL paths without ever mapping a request to the host filesystem.
fn request_path(raw: &str) -> Option<String> {
    let raw = raw.split('?').next()?.strip_prefix('/')?;
    let mut bytes = raw.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        });
    }
    let path = String::from_utf8(decoded).ok()?;
    if path.contains(['\\', '\0']) || path.split('/').any(|part| matches!(part, "." | "..")) {
        return None;
    }
    Some(if path.is_empty() {
        "index.html".into()
    } else {
        path
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_build_assets_and_navigation_without_exposing_paths() {
        let site = Website::start_files(
            BTreeMap::from([
                (
                    "index.html".into(),
                    Asset {
                        bytes: b"<h1>App</h1>".to_vec(),
                        mime: "text/html",
                    },
                ),
                (
                    "assets/app.js".into(),
                    Asset {
                        bytes: b"export default 42".to_vec(),
                        mime: "text/javascript",
                    },
                ),
            ]),
            true,
            0,
        )
        .await
        .unwrap();
        let client = reqwest::Client::new();
        let asset = client
            .get(format!("{}assets/app.js?v=1", site.url))
            .send()
            .await
            .unwrap();
        assert_eq!(asset.status(), 200);
        assert_eq!(asset.headers()["content-type"], "text/javascript");
        assert_eq!(asset.text().await.unwrap(), "export default 42");
        let page = client
            .get(format!("{}settings", site.url))
            .header("Accept", "text/html")
            .send()
            .await
            .unwrap();
        assert_eq!(page.text().await.unwrap(), "<h1>App</h1>");
        assert_eq!(
            client
                .get(format!("{}missing.js", site.url))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(request_path("/%2e%2e/private"), None);
        assert_eq!(request_path("/a%5cb"), None);
        assert_eq!(request_path("/%zz"), None);
        assert_eq!(
            request_path("/assets/app.js?v=1").as_deref(),
            Some("assets/app.js")
        );
        let url = site.url.clone();
        site.stop().await;
        assert!(client.get(url).send().await.is_err());
    }

    #[tokio::test]
    async fn new_files_are_served_at_the_same_address() {
        let site = Website::start_files(page("<p>one</p>".into()), false, 0)
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let first = client.get(&site.url).send().await.unwrap();
        assert_eq!(first.text().await.unwrap(), "<p>one</p>");
        site.replace(page("<p>two</p>".into()), false);
        let second = client.get(&site.url).send().await.unwrap();
        assert_eq!(second.text().await.unwrap(), "<p>two</p>");
        site.stop().await;
    }

    #[tokio::test]
    async fn the_preferred_port_is_used_and_a_taken_one_falls_back() {
        let port = preferred_port("artifact-stable-port-test");
        assert_eq!(port, preferred_port("artifact-stable-port-test"));
        assert!(STABLE_PORTS.contains(&port));
        let blocker = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken = blocker.local_addr().unwrap().port();
        let site = Website::start_files(page(String::new()), false, taken)
            .await
            .unwrap();
        assert_ne!(site.url, format!("http://127.0.0.1:{taken}/"));
        site.stop().await;
    }

    #[tokio::test]
    async fn localhost_works_and_other_names_are_refused() {
        assert!(allowed_host("localhost:8080", 8080));
        assert!(allowed_host("LocalHost:8080", 8080));
        assert!(allowed_host("127.0.0.1:8080", 8080));
        assert!(!allowed_host("localhost:8081", 8080));
        assert!(!allowed_host("evil.example:8080", 8080));
        assert!(!allowed_host("localhost", 8080));
        let site = Website::start_files(page("<p>hi</p>".into()), false, 0)
            .await
            .unwrap();
        let port = site.url.rsplit(':').next().unwrap().trim_end_matches('/');
        let client = reqwest::Client::new();
        let local = client
            .get(format!("http://localhost:{port}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(local.status(), 200);
        let forged = client
            .get(&site.url)
            .header("Host", format!("evil.example:{port}"))
            .send()
            .await
            .unwrap();
        assert_eq!(forged.status(), 403);
        site.stop().await;
    }
}
