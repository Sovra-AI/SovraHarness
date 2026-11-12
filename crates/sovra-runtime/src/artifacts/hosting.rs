//! Bounded loopback hosting of an immutable saved HTML snapshot. No filesystem
//! routing or shell execution: generated JavaScript runs only in the browser.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

pub(super) struct Asset {
    pub(super) bytes: Vec<u8>,
    pub(super) mime: &'static str,
}

pub(super) struct Website {
    pub(super) url: String,
    task: JoinHandle<()>,
}

impl Drop for Website {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Website {
    pub(super) async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }

    pub(super) async fn start(content: String) -> std::io::Result<Self> {
        Self::start_files(
            BTreeMap::from([(
                "index.html".into(),
                Asset {
                    bytes: content.into_bytes(),
                    mime: "text/html; charset=utf-8",
                },
            )]),
            false,
        )
        .await
    }

    pub(super) async fn start_files(
        files: BTreeMap<String, Asset>,
        spa: bool,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let files = Arc::new(files);
        let task = tokio::runtime::Handle::current().spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept(), if clients.len() < 16 => {
                        let Ok((stream, _)) = accepted else { break };
                        let files = files.clone();
                        clients.spawn(async move {
                            let _ = tokio::time::timeout(
                                Duration::from_secs(5), serve(stream, address, &files, spa),
                            ).await;
                        });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {}
                }
            }
        });
        Ok(Self {
            url: format!("http://{address}/"),
            task,
        })
    }
}

async fn serve(
    mut stream: TcpStream,
    address: SocketAddr,
    files: &BTreeMap<String, Asset>,
    spa: bool,
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
    // Exact Host validation prevents arbitrary DNS names rebinding to this server.
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    let host = address.to_string();
    let accepts_html = text.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("accept") && value.contains("text/html")
        })
    });
    let path = request_path(path);
    let asset = path.as_ref().and_then(|path| {
        files.get(path).or_else(|| {
            (spa && accepts_html)
                .then(|| files.get("index.html"))
                .flatten()
        })
    });
    let (status, body, mime): (&str, &[u8], &str) = if hosts.as_slice() != [host.as_str()] {
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
}
