//! The GitHub connection's lifecycle: restore, sign-in on GitHub's page or with a
//! pasted key, signing out, and the guarded calls agent tools make.
//!
//! One task owns the state and is the only writer; sign-in results, tool failures
//! and commands all reach it as messages. The device-flow waits run in a task this
//! runtime owns and aborts on Cancel, Disconnect or a new attempt. Keychain work
//! runs on blocking workers.

use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use sovra_adk::GitHubTools;
use sovra_core::Event;
use sovra_core::github::{
    DevicePoll, GitHubAccount, GitHubCommand, GitHubConnector, GitHubError, GitHubEvent, GitHubKey,
    GitHubState, poll_interval_secs,
};
use tokio::sync::RwLock;

/// People sometimes walk away mid sign-in; GitHub's codes last 15 minutes too.
const SIGN_IN_TIMEOUT: Duration = Duration::from_mins(15);

enum Message {
    Command(GitHubCommand),
    SignedIn {
        attempt: u64,
        result: Result<GitHubAccount, GitHubError>,
    },
    /// A tool call found the saved access no longer works.
    SignInExpired,
}

pub(crate) struct GitHubRuntime {
    messages: Sender<Message>,
    state: Arc<RwLock<GitHubState>>,
    connector: Option<Arc<dyn GitHubConnector>>,
    task: tokio::task::JoinHandle<()>,
}

impl GitHubRuntime {
    pub(crate) fn start(
        connector: Option<Arc<dyn GitHubConnector>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (messages, inbox) = async_channel::bounded(32);
        let state = Arc::new(RwLock::new(GitHubState::Loading));
        let worker = Worker {
            connector: connector.clone(),
            state: state.clone(),
            events,
            messages: messages.clone(),
            handle: handle.clone(),
            pending: None,
            attempt: 0,
        };
        let task = handle.spawn(worker.run(inbox));
        Self {
            messages,
            state,
            connector,
            task,
        }
    }

    pub(crate) fn send(&self, command: GitHubCommand) {
        if self.messages.try_send(Message::Command(command)).is_err() {
            tracing::warn!("github command dropped: too many pending");
        }
    }

    /// Tools for this turn, while GitHub is connected.
    pub(crate) async fn tools(&self) -> Option<GitHubTools> {
        let connector = self.connector.clone()?;
        if !matches!(&*self.state.read().await, GitHubState::Connected(_)) {
            return None;
        }
        let state = self.state.clone();
        let messages = self.messages.clone();
        Some(GitHubTools::from_fn(move |request| {
            let connector = connector.clone();
            let state = state.clone();
            let messages = messages.clone();
            async move {
                // Checked again per call: the person may disconnect mid-reply.
                if !matches!(&*state.read().await, GitHubState::Connected(_)) {
                    return Err(
                        "GitHub isn't connected any more, so Sovra can't use it.".to_owned()
                    );
                }
                let changes = request.changes_things();
                match connector.call(request).await {
                    Ok(result) => {
                        tracing::info!(changes, "github tool call");
                        Ok(result)
                    }
                    Err(GitHubError::SignInNeeded(message)) => {
                        let _ = messages.send(Message::SignInExpired).await;
                        Err(message)
                    }
                    Err(GitHubError::Failed(message)) => Err(message),
                }
            }
        }))
    }
}

impl Drop for GitHubRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Pending {
    task: tokio::task::JoinHandle<()>,
    /// What to go back to if this attempt does not finish.
    before: GitHubState,
}

struct Worker {
    connector: Option<Arc<dyn GitHubConnector>>,
    state: Arc<RwLock<GitHubState>>,
    events: Sender<Event>,
    messages: Sender<Message>,
    handle: tokio::runtime::Handle,
    pending: Option<Pending>,
    /// Ignores a late result from an attempt that was replaced or cancelled.
    attempt: u64,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, GitHubError> + Send + 'static,
) -> Result<T, GitHubError> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        Err(GitHubError::Failed(
            "Sovra's GitHub helper stopped unexpectedly. Please try again.".into(),
        ))
    })
}

/// Asks GitHub every few seconds until the person finishes on its page.
async fn wait_for_device(
    connector: Arc<dyn GitHubConnector>,
    device: sovra_core::github::DeviceCode,
) -> Result<GitHubAccount, GitHubError> {
    let mut slowed = 0;
    loop {
        let wait = poll_interval_secs(device.interval_secs, slowed);
        tokio::time::sleep(Duration::from_secs(wait)).await;
        match connector.poll_device_sign_in(&device).await? {
            DevicePoll::Waiting => {}
            DevicePoll::SlowDown => slowed += 1,
            DevicePoll::Done(account) => return Ok(account),
        }
    }
}

impl Worker {
    async fn run(mut self, inbox: Receiver<Message>) {
        while let Ok(message) = inbox.recv().await {
            match message {
                Message::Command(command) => self.command(command).await,
                Message::SignedIn { attempt, result } if attempt == self.attempt => {
                    self.signed_in(result).await;
                }
                Message::SignedIn { .. } => {}
                Message::SignInExpired => {
                    let current = self.state.read().await.clone();
                    if let GitHubState::Connected(account) = current {
                        self.publish(GitHubState::NeedsSignIn(account)).await;
                    }
                }
            }
        }
        if let Some(pending) = self.pending.take() {
            pending.task.abort();
        }
    }

    async fn publish(&self, state: GitHubState) {
        state.clone_into(&mut *self.state.write().await);
        let device_sign_in = self
            .connector
            .as_ref()
            .is_some_and(|connector| connector.device_sign_in());
        let _ = self
            .events
            .send(Event::GitHub(GitHubEvent::Changed {
                state,
                device_sign_in,
            }))
            .await;
    }

    async fn notice(&self, message: impl Into<String>) {
        let _ = self
            .events
            .send(Event::GitHub(GitHubEvent::Notice(message.into())))
            .await;
    }

    async fn command(&mut self, command: GitHubCommand) {
        // Runtimes started without GitHub (tests, embedders) say nothing about it.
        let Some(connector) = self.connector.clone() else {
            return;
        };
        match command {
            GitHubCommand::Load => {
                let restored = blocking({
                    let connector = connector.clone();
                    move || connector.restore()
                })
                .await;
                match restored {
                    Ok(Some(account)) => self.publish(GitHubState::Connected(account)).await,
                    Ok(None) => self.publish(GitHubState::NotConnected).await,
                    Err(error) => {
                        tracing::warn!(%error, "github restore failed");
                        self.publish(GitHubState::NotConnected).await;
                        self.notice(error.to_string()).await;
                    }
                }
            }
            GitHubCommand::Connect => self.connect_on_github(&connector).await,
            GitHubCommand::UseKey(key) => self.use_key(&connector, key).await,
            GitHubCommand::Cancel => {
                if let Some(pending) = self.pending.take() {
                    pending.task.abort();
                    self.attempt += 1;
                    self.publish(pending.before).await;
                }
            }
            GitHubCommand::Disconnect => {
                if let Some(pending) = self.pending.take() {
                    pending.task.abort();
                    self.attempt += 1;
                }
                let forgotten = blocking({
                    let connector = connector.clone();
                    move || connector.forget()
                })
                .await;
                if let Err(error) = forgotten {
                    self.notice(error.to_string()).await;
                }
                self.publish(GitHubState::NotConnected).await;
            }
        }
    }

    /// Replaces any attempt in progress, keeping what it would go back to.
    async fn begin_attempt(&mut self) -> (u64, GitHubState) {
        let before = match self.pending.take() {
            Some(previous) => {
                previous.task.abort();
                previous.before
            }
            None => self.state.read().await.clone(),
        };
        self.attempt += 1;
        (self.attempt, before)
    }

    async fn connect_on_github(&mut self, connector: &Arc<dyn GitHubConnector>) {
        let (attempt, before) = self.begin_attempt().await;
        let device = match connector.start_device_sign_in().await {
            Ok(device) => device,
            Err(error) => {
                self.publish(before).await;
                self.notice(error.to_string()).await;
                return;
            }
        };
        let url = device.verification_uri.clone();
        let code = device.user_code.clone();
        self.publish(GitHubState::Connecting {
            code: code.clone(),
            url: url.clone(),
        })
        .await;
        let _ = self
            .events
            .send(Event::GitHub(GitHubEvent::OpenSignIn { url, code }))
            .await;
        let connector = connector.clone();
        let messages = self.messages.clone();
        let lifetime = SIGN_IN_TIMEOUT.min(Duration::from_secs(device.expires_in_secs.max(60)));
        let task = self.handle.spawn(async move {
            let result = tokio::time::timeout(lifetime, wait_for_device(connector, device))
                .await
                .unwrap_or_else(|_| {
                    Err(GitHubError::Failed(
                        "Nothing happened on GitHub's page, so Sovra stopped waiting. Press \
                         Connect GitHub to try again."
                            .into(),
                    ))
                });
            let _ = messages.send(Message::SignedIn { attempt, result }).await;
        });
        self.pending = Some(Pending { task, before });
    }

    async fn use_key(&mut self, connector: &Arc<dyn GitHubConnector>, key: GitHubKey) {
        let (attempt, before) = self.begin_attempt().await;
        self.publish(GitHubState::CheckingKey).await;
        let connector = connector.clone();
        let messages = self.messages.clone();
        let task = self.handle.spawn(async move {
            let result = connector.check_key(&key).await;
            let _ = messages.send(Message::SignedIn { attempt, result }).await;
        });
        self.pending = Some(Pending { task, before });
    }

    async fn signed_in(&mut self, result: Result<GitHubAccount, GitHubError>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let Some(connector) = self.connector.clone() else {
            return;
        };
        match result {
            Ok(account) => {
                let saved = blocking({
                    let connector = connector.clone();
                    let account = account.clone();
                    move || connector.save(&account)
                })
                .await;
                // Publish first so the "connected" notice is not replaced by a save warning.
                self.publish(GitHubState::Connected(account)).await;
                if let Err(error) = saved {
                    self.notice(error.to_string()).await;
                }
            }
            Err(error) => {
                self.publish(pending.before).await;
                self.notice(error.to_string()).await;
            }
        }
    }
}
