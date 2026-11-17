//! Shared loopback HTTP fixture; requests never leave this process's host.

use serde_json::Value;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) path: String,
    pub(crate) headers: String,
    pub(crate) body: Value,
}
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) content_type: &'static str,
    pub(crate) body: String,
    pub(crate) tail: Option<(Arc<Mutex<std::sync::mpsc::Receiver<()>>>, String)>,
}
impl Reply {
    pub(crate) fn json(value: &Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: value.to_string(),
            tail: None,
        }
    }
}
pub(crate) struct Server {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    pub(crate) requests: Arc<Mutex<Vec<Request>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
type Handler = Arc<Mutex<Box<dyn Fn(&Request) -> Reply + Send>>>;

impl Server {
    /// Answers one connection at a time, so request order is deterministic.
    pub(crate) fn new(handler: impl Fn(&Request) -> Reply + Send + 'static) -> Self {
        Self::start(handler, false)
    }

    /// Answers every connection on its own thread, so a reply held open by a
    /// `tail` gate does not stop other requests from being answered.
    #[allow(dead_code, reason = "only some test binaries need concurrent replies")]
    pub(crate) fn concurrent(handler: impl Fn(&Request) -> Reply + Send + 'static) -> Self {
        Self::start(handler, true)
    }

    fn start(handler: impl Fn(&Request) -> Reply + Send + 'static, concurrent: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let handler: Handler = Arc::new(Mutex::new(Box::new(handler)));
        let thread = std::thread::spawn(move || {
            for socket in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let socket = socket.unwrap();
                if concurrent {
                    let handler = handler.clone();
                    let observed = observed.clone();
                    std::thread::spawn(move || serve(socket, &handler, &observed));
                } else {
                    serve(socket, &handler, &observed);
                }
            }
        });
        Self {
            address,
            stop,
            requests,
            thread: Some(thread),
        }
    }
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}

fn serve(mut socket: TcpStream, handler: &Handler, observed: &Mutex<Vec<Request>>) {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let count = match socket.read(&mut chunk) {
            Ok(0) | Err(_) => break None,
            Ok(n) => n,
        };
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(end + 4);
        }
        assert!(bytes.len() < 1024 * 1024);
    };
    let Some(header_end) = header_end else {
        return;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, v)| v.trim().parse::<usize>().unwrap());
    while bytes.len() < header_end + length {
        let n = socket.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let path = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let body = serde_json::from_slice(&bytes[header_end..]).unwrap_or(Value::Null);
    let request = Request {
        path,
        headers,
        body,
    };
    let reply = (*handler.lock().unwrap())(&request);
    observed.lock().unwrap().push(request);
    let status = if reply.status == 200 { "OK" } else { "Error" };
    let response = format!(
        "HTTP/1.1 {} {status}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.status,
        reply.content_type,
        reply.body.len()
            + reply.tail.as_ref().map_or(0, |(_, tail)| {
                tail.len() - tail.matches(TAIL_PART).count() * TAIL_PART.len_utf8()
            }),
        reply.body
    );
    let _ = socket.write_all(response.as_bytes());
    if let Some((gate, tail)) = reply.tail {
        // Each `TAIL_PART`-separated piece waits for its own release, so a test
        // can stream a reply slowly.
        for part in tail.split(TAIL_PART) {
            if gate
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .is_err()
            {
                break;
            }
            let _ = socket.write_all(part.as_bytes());
            let _ = socket.flush();
        }
    }
}

/// Separates the pieces of a `Reply::tail` that are released one at a time.
/// Counted out of the `Content-Length`, which covers only what is written.
#[allow(dead_code, reason = "only some test binaries stream replies slowly")]
pub(crate) const TAIL_PART: char = '\u{1e}';

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
