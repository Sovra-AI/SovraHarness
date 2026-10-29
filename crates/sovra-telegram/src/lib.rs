//! Telegram adapter: implements `sovra_core::connections::MessagingChannel` over the
//! official Bot API (HTTPS long polling). A person creates a bot with `@BotFather`, pastes
//! its token into Settings, and sends the bot a code to become its owner; Sovra then
//! answers them in that chat.
//!
//! The token is the only secret and lives behind [`TokenStore`], which the composition
//! root backs with the OS keychain. This crate spawns nothing: the runtime calls
//! [`ChannelSession::poll`] in a loop and does the waiting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use sovra_core::Timestamp;
use sovra_core::connections::{
    ChannelError, ChannelEvent, ChannelSession, ChannelSink, ChannelStartOptions, ChatKind,
    IncomingMessage, LinkedDevice, MessagingChannel, PollOutcome,
};

const API_BASE: &str = "https://api.telegram.org";
/// Long-poll wait Telegram holds the request open for.
const POLL_WAIT_SECS: u64 = 25;
/// Client-side cap: the long poll plus network slack.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
/// Pause after a failed poll, so a dead network does not spin.
const RETRY_AFTER_ERROR: Duration = Duration::from_secs(5);
/// Messages older than this when first seen waited in Telegram's queue while Sovra was
/// closed; they are recorded but not answered.
const BACKLOG_AFTER: Duration = Duration::from_secs(60);
/// Telegram caps a message at 4096 characters; longer answers are split.
const MAX_MESSAGE_CHARS: usize = 4000;

/// Where the bot token is kept. The composition root decides (OS keychain in release).
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, token: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

impl std::fmt::Debug for dyn TokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenStore")
    }
}

/// A Telegram bot linked through this app.
#[derive(Clone)]
pub struct TelegramChannel {
    tokens: Arc<dyn TokenStore>,
    http: reqwest::Client,
}

impl std::fmt::Debug for TelegramChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramChannel").finish_non_exhaustive()
    }
}

impl TelegramChannel {
    pub fn new(tokens: Arc<dyn TokenStore>) -> Result<Self, ChannelError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ChannelError::Unavailable(format!("HTTP client: {error}")))?;
        Ok(Self { tokens, http })
    }
}

#[async_trait]
impl MessagingChannel for TelegramChannel {
    async fn start(
        &self,
        options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        let fresh = options.token.is_some();
        let Some(token) = options.token.or_else(|| self.tokens.load()) else {
            return Err(ChannelError::Unavailable(
                "No bot token is saved. Paste the token from @BotFather first.".into(),
            ));
        };
        events(ChannelEvent::Connecting);
        let api = Api {
            http: self.http.clone(),
            token: token.clone(),
        };
        let me: User = api.call("getMe", &serde_json::json!({})).await?;
        if fresh {
            self.tokens.save(&token).map_err(|error| {
                ChannelError::Failed(format!("Could not save the token: {error}"))
            })?;
        }
        let username = me.username.clone().unwrap_or_else(|| me.first_name.clone());
        tracing::info!(bot = %username, "telegram bot connected");
        events(ChannelEvent::Connected {
            device: LinkedDevice {
                phone_display: format!("@{username}"),
                device_jid: me.id.to_string(),
                companion_name: me.first_name,
                linked_at: Timestamp(0),
            },
        });
        Ok(Arc::new(Session {
            api,
            events,
            offset: AtomicI64::new(0),
            stopped: AtomicBool::new(false),
            offline: AtomicBool::new(false),
            started_at: now_unix(),
        }))
    }

    fn has_saved_link(&self) -> bool {
        self.tokens.load().is_some()
    }

    fn forget(&self) -> Result<(), ChannelError> {
        self.tokens.clear().map_err(ChannelError::Failed)
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// One Bot API client bound to a token. The token never appears in errors or logs.
#[derive(Clone)]
struct Api {
    http: reqwest::Client,
    token: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
    error_code: Option<u16>,
    parameters: Option<ResponseParameters>,
}

#[derive(Debug, Deserialize)]
struct ResponseParameters {
    retry_after: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct User {
    id: i64,
    first_name: String,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(rename = "message_id")]
    id: i64,
    date: i64,
    chat: Chat,
    #[serde(default)]
    from: Option<User>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    #[serde(default)]
    message: Option<Message>,
}

/// Why a Bot API call failed, with the parts the caller acts on.
#[derive(Debug)]
enum ApiError {
    /// Telegram answered with an error. 401 means the token was revoked.
    Rejected {
        code: u16,
        description: String,
        retry_after: Option<u64>,
    },
    /// No usable answer (network, timeout, bad JSON).
    Transport(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected {
                code, description, ..
            } => write!(f, "Telegram refused the request ({code}): {description}"),
            Self::Transport(message) => write!(f, "Telegram could not be reached: {message}"),
        }
    }
}

impl From<ApiError> for ChannelError {
    fn from(error: ApiError) -> Self {
        match &error {
            ApiError::Rejected { code: 401, .. } => Self::Failed(
                "Telegram does not accept this token. Check it in @BotFather and paste it again."
                    .into(),
            ),
            ApiError::Rejected { .. } | ApiError::Transport(_) => Self::Failed(error.to_string()),
        }
    }
}

impl Api {
    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<T, ApiError> {
        let url = format!("{API_BASE}/bot{}/{method}", self.token);
        let response = self
            .http
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|error| ApiError::Transport(describe(&error)))?;
        let parsed: ApiResponse<T> = response
            .json()
            .await
            .map_err(|error| ApiError::Transport(describe(&error)))?;
        match (parsed.ok, parsed.result) {
            (true, Some(result)) => Ok(result),
            _ => Err(ApiError::Rejected {
                code: parsed.error_code.unwrap_or(0),
                description: parsed
                    .description
                    .unwrap_or_else(|| "no description".into()),
                retry_after: parsed.parameters.and_then(|p| p.retry_after),
            }),
        }
    }
}

/// Reqwest errors can embed the request URL, which carries the token; keep only the kind.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else if error.is_decode() {
        "unexpected answer".into()
    } else if let Some(status) = error.status() {
        format!("HTTP {status}")
    } else {
        "request failed".into()
    }
}

struct Session {
    api: Api,
    events: ChannelSink,
    /// Next `update_id` to ask for; Telegram drops everything before it.
    offset: AtomicI64,
    stopped: AtomicBool,
    offline: AtomicBool,
    started_at: i64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("stopped", &self.stopped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ChannelSession for Session {
    async fn send_text(&self, chat_jid: &str, text: &str) -> Result<String, ChannelError> {
        let chat_id: i64 = chat_jid
            .parse()
            .map_err(|_| ChannelError::Failed(format!("Bad Telegram chat id: {chat_jid}")))?;
        let mut last_id = String::new();
        for part in split_message(text) {
            let sent: Message = self
                .api
                .call(
                    "sendMessage",
                    &serde_json::json!({ "chat_id": chat_id, "text": part }),
                )
                .await?;
            last_id = sent.id.to_string();
        }
        Ok(last_id)
    }

    async fn logout(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn own_chat_jid(&self) -> Option<String> {
        None
    }

    async fn poll(&self) -> PollOutcome {
        if self.stopped.load(Ordering::SeqCst) {
            return PollOutcome::Finished;
        }
        let body = serde_json::json!({
            "offset": self.offset.load(Ordering::SeqCst),
            "timeout": POLL_WAIT_SECS,
            "allowed_updates": ["message"],
        });
        let updates: Vec<Update> = match self.api.call("getUpdates", &body).await {
            Ok(updates) => updates,
            Err(ApiError::Rejected {
                code: 401,
                description,
                ..
            }) => {
                self.stopped.store(true, Ordering::SeqCst);
                (self.events)(ChannelEvent::LoggedOut {
                    reason: description,
                });
                return PollOutcome::Finished;
            }
            Err(ApiError::Rejected { code: 409, .. }) => {
                (self.events)(ChannelEvent::Offline {
                    reason: "another program is reading this bot's messages".into(),
                });
                self.offline.store(true, Ordering::SeqCst);
                return PollOutcome::Wait(RETRY_AFTER_ERROR * 2);
            }
            Err(ApiError::Rejected {
                code: 429,
                retry_after,
                ..
            }) => {
                return PollOutcome::Wait(Duration::from_secs(retry_after.unwrap_or(5).max(1)));
            }
            Err(error) => {
                if !self.offline.swap(true, Ordering::SeqCst) {
                    (self.events)(ChannelEvent::Offline {
                        reason: error.to_string(),
                    });
                }
                return PollOutcome::Wait(RETRY_AFTER_ERROR);
            }
        };
        if self.offline.swap(false, Ordering::SeqCst) {
            let me: Result<User, _> = self.api.call("getMe", &serde_json::json!({})).await;
            if let Ok(me) = me {
                let username = me.username.clone().unwrap_or_else(|| me.first_name.clone());
                (self.events)(ChannelEvent::Connected {
                    device: LinkedDevice {
                        phone_display: format!("@{username}"),
                        device_jid: me.id.to_string(),
                        companion_name: me.first_name,
                        linked_at: Timestamp(0),
                    },
                });
            }
        }
        let backlog_before = self.started_at - i64::try_from(BACKLOG_AFTER.as_secs()).unwrap_or(60);
        for update in updates {
            self.offset.store(update.update_id + 1, Ordering::SeqCst);
            let Some(message) = update.message else {
                continue;
            };
            let Some(text) = message.text else { continue };
            let sender = message.from.as_ref();
            (self.events)(ChannelEvent::Message(IncomingMessage {
                id: format!("{}:{}", message.chat.id, message.id),
                chat_jid: message.chat.id.to_string(),
                sender_jid: sender.map_or_else(String::new, |user| user.id.to_string()),
                sender_name: sender.map_or_else(String::new, display_name),
                text,
                from_me: false,
                chat: if message.chat.kind == "private" {
                    ChatKind::Direct
                } else {
                    ChatKind::Group
                },
                backlog: message.date < backlog_before,
            }));
        }
        PollOutcome::Again
    }
}

fn display_name(user: &User) -> String {
    match (&user.last_name, &user.username) {
        (Some(last), _) if !last.is_empty() => format!("{} {last}", user.first_name),
        (_, Some(username)) if user.first_name.is_empty() => format!("@{username}"),
        _ => user.first_name.clone(),
    }
}

/// Splits long answers at paragraph or line breaks so each part fits Telegram's limit.
fn split_message(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for paragraph in text.split_inclusive('\n') {
        if current.chars().count() + paragraph.chars().count() > MAX_MESSAGE_CHARS
            && !current.is_empty()
        {
            parts.push(std::mem::take(&mut current));
        }
        if paragraph.chars().count() > MAX_MESSAGE_CHARS {
            let mut chunk = String::new();
            for ch in paragraph.chars() {
                chunk.push(ch);
                if chunk.chars().count() >= MAX_MESSAGE_CHARS {
                    parts.push(std::mem::take(&mut chunk));
                }
            }
            current = chunk;
        } else {
            current.push_str(paragraph);
        }
    }
    if !current.trim().is_empty() || parts.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_answers_split_at_line_breaks_within_the_limit() {
        let text = format!("{}\n{}", "a".repeat(3000), "b".repeat(3000));
        let parts = split_message(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_MESSAGE_CHARS));
        assert_eq!(split_message("short"), vec!["short".to_owned()]);
        let huge = "x".repeat(9000);
        let parts = split_message(&huge);
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn names_fall_back_sensibly() {
        let user = User {
            id: 1,
            first_name: "Sam".into(),
            last_name: Some("Lee".into()),
            username: Some("sam".into()),
        };
        assert_eq!(display_name(&user), "Sam Lee");
        let only_handle = User {
            id: 2,
            first_name: String::new(),
            last_name: None,
            username: Some("handle".into()),
        };
        assert_eq!(display_name(&only_handle), "@handle");
    }
}
