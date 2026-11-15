//! A model trained here goes to a storage bucket and to Ollama by itself, with
//! the real engine, the real runtime and the real senders. The bucket and
//! Ollama are small stand-ins on this computer that check what arrives the way
//! the real services do: every signed header is sent, the body is the one that
//! was hashed and signed, and Ollama receives a file whose fingerprint matches.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use sovra_core::models::{ModelConnection, ProviderKind};
use sovra_core::training::{
    BaseModel, BucketSettings, CopyState, ModelId, SAMPLE_TEXT, TrainingBase, TrainingCommand,
    TrainingEvent, TrainingPlan, TrainingPreset, TrainingSettings, TrainingSource, TrainingState,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_providers::{FileCredentialStore, HttpModelOutlets};
use sovra_runtime::{Client, RuntimeServices};

const WAIT: Duration = Duration::from_secs(180);
const KEY_ID: &str = "SOVRATESTKEY";
const SECRET: &str = "sovra-test-secret/with+symbols=";

/// One request as it arrived.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

type Answer = Box<dyn Fn(&Seen) -> (u16, String) + Send + Sync>;

/// A one-thread HTTP/1.1 server on this computer that keeps what it is sent.
struct StandIn {
    address: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl StandIn {
    fn start(answer: Answer) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free local port");
        let address = listener.local_addr().expect("the port is known");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { continue };
                let _ = socket.set_read_timeout(Some(Duration::from_secs(10)));
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 16_384];
                let head_end = loop {
                    match socket.read(&mut chunk) {
                        Ok(0) | Err(_) => break None,
                        Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                    }
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(end);
                    }
                };
                let Some(head_end) = head_end else { continue };
                let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
                let mut lines = head.split("\r\n");
                let mut start = lines.next().unwrap_or_default().split(' ');
                let method = start.next().unwrap_or_default().to_owned();
                let path = start.next().unwrap_or_default().to_owned();
                let headers: BTreeMap<String, String> = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(name, value)| {
                        (name.trim().to_ascii_lowercase(), value.trim().to_owned())
                    })
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let mut body = bytes[head_end + 4..].to_vec();
                while body.len() < length {
                    match socket.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => body.extend_from_slice(&chunk[..count]),
                    }
                }
                let request = Seen {
                    method,
                    path,
                    headers,
                    body,
                };
                let (status, reply) = answer(&request);
                kept.lock().expect("requests lock").push(request);
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        Self { address, seen }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("requests lock").clone()
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// A bucket that answers like S3 and refuses what S3 would refuse.
fn bucket_stand_in() -> StandIn {
    StandIn::start(Box::new(|request| {
        let refuse = |why: &str| (403, format!("<Error><Code>{why}</Code></Error>"));
        let Some(authorization) = request.headers.get("authorization") else {
            return refuse("AccessDenied");
        };
        // Every header the signature covers must have been sent, and the body
        // must be the one whose hash was signed.
        let signed = authorization
            .split("SignedHeaders=")
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .unwrap_or_default();
        let hash = request.headers.get("x-amz-content-sha256");
        if request.method != "PUT"
            || !authorization.starts_with(&format!("AWS4-HMAC-SHA256 Credential={KEY_ID}/"))
            || !authorization.contains("/us-east-1/s3/aws4_request")
            || signed != "content-type;host;x-amz-content-sha256;x-amz-date"
            || signed
                .split(';')
                .any(|name| !request.headers.contains_key(name))
            || hash.map(String::as_str) != Some(hex(&Sha256::digest(&request.body)).as_str())
        {
            return refuse("SignatureDoesNotMatch");
        }
        (200, String::new())
    }))
}

/// An Ollama of `version` that takes a model the way the real one does.
fn ollama_stand_in(version: &'static str) -> StandIn {
    let blobs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    StandIn::start(Box::new(move |request| {
        let mut blobs = blobs.lock().expect("blobs lock");
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/version") => (200, format!(r#"{{"version":"{version}"}}"#)),
            ("HEAD", path) if path.starts_with("/api/blobs/") => {
                let digest = &path["/api/blobs/".len()..];
                (
                    if blobs.iter().any(|held| held == digest) {
                        200
                    } else {
                        404
                    },
                    String::new(),
                )
            }
            ("POST", path) if path.starts_with("/api/blobs/") => {
                let digest = &path["/api/blobs/".len()..];
                if digest != format!("sha256:{}", hex(&Sha256::digest(&request.body))) {
                    return (400, r#"{"error":"digest mismatch"}"#.into());
                }
                blobs.push(digest.to_owned());
                (201, String::new())
            }
            ("POST", "/api/generate") => (200, r#"{"response":"a","done":true}"#.into()),
            ("POST", "/api/create") => {
                let body: serde_json::Value =
                    serde_json::from_slice(&request.body).unwrap_or_default();
                let file = body["files"]["model.gguf"].as_str().unwrap_or_default();
                if blobs.iter().any(|held| held == file) {
                    (200, r#"{"status":"success"}"#.into())
                } else {
                    (400, r#"{"error":"blob not found"}"#.into())
                }
            }
            _ => (404, String::new()),
        }
    }))
}

struct Computer {
    client: Client,
    events: async_channel::Receiver<Event>,
}

fn start(dir: &std::path::Path, ollama: &StandIn) -> Computer {
    let mut connection = ModelConnection::new("ollama", ProviderKind::Ollama);
    connection.base_url = format!("http://{}", ollama.address);
    let outlets = HttpModelOutlets::new(Arc::new(FileCredentialStore::new(dir.join("keys"))), None)
        .with_ollama(connection);
    let client = sovra_runtime::start_with_services(
        "model-outlets",
        AgentConfig::default(),
        RuntimeServices {
            training: Some(sovra_runtime::TrainingConfig {
                engine: Arc::new(sovra_train::BurnTrainingEngine::processor()),
                directory: dir.join("training"),
                outlets: Some(Arc::new(outlets)),
            }),
            ..RuntimeServices::default()
        },
    )
    .expect("runtime starts");
    let events = client.events();
    Computer { client, events }
}

impl Computer {
    fn send(&self, command: TrainingCommand) {
        self.client
            .send(Command::Training(command))
            .expect("runtime accepts commands");
    }

    /// The first training state that `test` accepts, reporting any error seen.
    async fn until(&self, what: &str, test: impl Fn(&TrainingState) -> bool) -> TrainingState {
        let mut last_error = None;
        let found = tokio::time::timeout(WAIT, async {
            loop {
                if let Event::Training(TrainingEvent(state)) =
                    self.events.recv().await.expect("runtime running")
                {
                    if state.error.is_some() {
                        last_error.clone_from(&state.error);
                    }
                    if test(&state) {
                        return *state;
                    }
                }
            }
        })
        .await;
        found.unwrap_or_else(|_| panic!("timed out waiting for {what}; last error {last_error:?}"))
    }

    async fn stop(self) {
        drop(self.events);
        tokio::task::spawn_blocking(move || self.client.shutdown_blocking())
            .await
            .expect("the runtime stops")
            .expect("the runtime was running");
    }
}

fn bucket(at: &StandIn) -> BucketSettings {
    BucketSettings {
        endpoint: format!("http://{}", at.address),
        region: "us-east-1".into(),
        bucket: "models".into(),
        folder: "sovra/finished".into(),
        access_key_id: KEY_ID.into(),
    }
}

fn plan() -> TrainingPlan {
    let mut settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        BaseModel::Small.architecture(),
        SAMPLE_TEXT.len(),
        &sovra_core::training::HardwareInfo::default(),
    );
    settings.rounds = 20;
    settings.warmup_rounds = 2;
    TrainingPlan {
        name: "Garden notes".into(),
        base: TrainingBase::Fresh(BaseModel::Small),
        source: TrainingSource::Sample,
        preset: None,
        settings,
        with_group: false,
    }
}

fn done(state: &TrainingState, model: ModelId) -> (Option<String>, Option<String>) {
    let place = |copy: &Option<CopyState>| match copy {
        Some(CopyState::Done { place, .. }) => Some(place.clone()),
        _ => None,
    };
    state.outlets.copies(model).map_or((None, None), |copies| {
        (place(&copies.bucket), place(&copies.ollama))
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_model_goes_to_the_bucket_and_to_ollama_by_itself() {
    let dir = std::env::temp_dir().join(format!("sovra-outlets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let storage = bucket_stand_in();
    let ollama = ollama_stand_in("0.40.16");
    let computer = start(&dir, &ollama);
    computer.send(TrainingCommand::Load);
    computer
        .until("the page to load", |state| state.hardware.is_some())
        .await;

    // Nothing can be switched on before there is somewhere to send to.
    computer.send(TrainingCommand::SetAutomaticBackup(true));
    let refused = computer
        .until("a refusal", |state| state.error.is_some())
        .await;
    assert!(refused.error.unwrap().contains("bucket"));
    assert!(!refused.outlets.backup_automatically);

    computer.send(TrainingCommand::SaveBucket {
        settings: bucket(&storage),
        secret: Some(SECRET.into()),
    });
    computer
        .until("the bucket to be saved", |state| {
            state.outlets.bucket_key_saved
        })
        .await;
    computer.send(TrainingCommand::SetAutomaticBackup(true));
    computer.send(TrainingCommand::SetAutomaticOllama(true));
    computer
        .until("both switches on", |state| {
            state.outlets.backup_automatically && state.outlets.ollama_automatically
        })
        .await;

    computer.send(TrainingCommand::Start(Box::new(plan())));
    let finished = computer
        .until("the model to arrive in both places", |state| {
            state.models.first().is_some_and(|model| {
                let (bucket, ollama) = done(state, model.card.id);
                bucket.is_some() && ollama.is_some()
            })
        })
        .await;
    let model = finished.models[0].clone();
    let (place, name) = done(&finished, model.card.id);
    let stem_start: String = model.card.id.to_hex().chars().take(8).collect();
    assert_eq!(
        place.as_deref(),
        Some(format!("models/sovra/finished/garden-notes-{stem_start}.model").as_str())
    );
    assert_eq!(
        name.as_deref(),
        Some(format!("sovra-garden-notes-{stem_start}").as_str())
    );
    assert!(finished.notice.is_some());

    // The bucket received the model file exactly as it is in the library, its
    // GGUF form, and a description, all signed with the saved keys.
    let library_file = std::fs::read(&model.saved_at).expect("the library file");
    let puts = storage.seen();
    let keys: Vec<&str> = puts.iter().map(|put| put.path.as_str()).collect();
    let stem = format!("/models/sovra/finished/garden-notes-{stem_start}");
    assert_eq!(
        keys,
        [
            format!("{stem}.model"),
            format!("{stem}.gguf"),
            format!("{stem}.md")
        ]
    );
    assert_eq!(puts[0].body, library_file);
    assert_eq!(&puts[1].body[..4], b"GGUF");
    let readme = String::from_utf8(puts[2].body.clone()).expect("plain text");
    assert!(readme.starts_with("# Garden notes"));
    // The text the model learned from never leaves.
    let sample_line = SAMPLE_TEXT.lines().find(|line| line.len() > 30).unwrap();
    for put in &puts {
        assert!(!String::from_utf8_lossy(&put.body).contains(sample_line));
        assert!(!put.headers.values().any(|value| value.contains(SECRET)));
    }

    // Ollama got the same GGUF, under the model's name, set up to continue text.
    let seen = ollama.seen();
    let create = seen
        .iter()
        .find(|request| request.path == "/api/create")
        .expect("the model was created");
    let body: serde_json::Value = serde_json::from_slice(&create.body).unwrap();
    assert_eq!(body["model"], name.unwrap());
    assert_eq!(body["template"], "{{ .Prompt }}");
    assert_eq!(body["parameters"]["num_ctx"], 64);
    assert_eq!(
        body["files"]["model.gguf"],
        format!("sha256:{}", hex(&Sha256::digest(&puts[1].body)))
    );

    // Everything chosen survives a restart; nothing is being sent afterwards,
    // and the secret key is never in the settings file.
    computer.stop().await;
    let saved = std::fs::read_to_string(dir.join("training/outlets.json")).unwrap();
    assert!(!saved.contains(SECRET));
    let again = start(&dir, &ollama);
    again.send(TrainingCommand::Load);
    let reopened = again
        .until("the choices to come back", |state| {
            state.outlets.bucket.is_some() && !state.models.is_empty()
        })
        .await;
    assert!(reopened.outlets.bucket_key_saved);
    assert!(reopened.outlets.backup_automatically && reopened.outlets.ollama_automatically);
    assert_eq!(done(&reopened, model.card.id).0, place);

    // Forgetting the bucket removes the key and turns the backup off.
    again.send(TrainingCommand::ForgetBucket);
    let forgot = again
        .until("the bucket to be forgotten", |state| {
            state.outlets.bucket.is_none()
        })
        .await;
    assert!(!forgot.outlets.bucket_key_saved && !forgot.outlets.backup_automatically);
    again.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ollama_too_old_for_the_model_is_named_with_the_version_to_get() {
    let dir = std::env::temp_dir().join(format!("sovra-outlets-old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ollama = ollama_stand_in("0.33.3");
    let computer = start(&dir, &ollama);
    computer.send(TrainingCommand::Load);
    computer
        .until("the page to load", |state| state.hardware.is_some())
        .await;
    computer.send(TrainingCommand::Start(Box::new(plan())));
    let trained = computer
        .until("the model to finish", |state| !state.models.is_empty())
        .await;
    let model = trained.models[0].card.id;
    computer.send(TrainingCommand::AddToOllama(model));
    let failed = computer
        .until("the copy to fail", |state| {
            matches!(
                state
                    .outlets
                    .copies(model)
                    .and_then(|copies| copies.ollama.clone()),
                Some(CopyState::Failed(_))
            )
        })
        .await;
    let Some(CopyState::Failed(why)) = failed.outlets.copies(model).unwrap().ollama.clone() else {
        unreachable!("checked above");
    };
    assert!(why.contains("0.33.3") && why.contains("0.40.15"), "{why}");
    // Nothing was handed over to an Ollama that cannot run it.
    assert!(
        ollama
            .seen()
            .iter()
            .all(|request| request.path == "/api/version")
    );
    computer.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}
