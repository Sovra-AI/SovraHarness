//! The Google connection's lifecycle: restore, browser sign-in, switching services,
//! signing out, and the guarded calls agent tools make.
//!
//! One task owns the state and is the only writer; sign-in waits, tool failures
//! and commands all reach it as messages. Keychain and socket work runs on
//! blocking workers, so the async workers never stall.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use sovra_adk::GoogleTools;
use sovra_core::Event;
use sovra_core::google::{
    GoogleAccount, GoogleCommand, GoogleConnector, GoogleError, GoogleEvent, GoogleService,
    GoogleState, SignInCanceller, service_info,
};
use tokio::sync::RwLock;

/// People sometimes walk away mid sign-in; stop listening after this.
const SIGN_IN_TIMEOUT: Duration = Duration::from_mins(5);

enum Message {
    Command(GoogleCommand),
    SignedIn {
        attempt: u64,
        result: Result<GoogleAccount, GoogleError>,
    },
    /// A tool call found the saved sign-in no longer works.
    SignInExpired,
}

pub(crate) struct GoogleRuntime {
    messages: Sender<Message>,
    state: Arc<RwLock<GoogleState>>,
    connector: Option<Arc<dyn GoogleConnector>>,
    task: tokio::task::JoinHandle<()>,
}

impl GoogleRuntime {
    pub(crate) fn start(
        connector: Option<Arc<dyn GoogleConnector>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (messages, inbox) = async_channel::bounded(32);
        let state = Arc::new(RwLock::new(GoogleState::Loading));
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

    pub(crate) fn send(&self, command: GoogleCommand) {
        if self.messages.try_send(Message::Command(command)).is_err() {
            tracing::warn!("google command dropped: too many pending");
        }
    }

    /// Tools for this turn: the services that are connected and switched on.
    pub(crate) async fn tools(&self) -> Option<GoogleTools> {
        let connector = self.connector.clone()?;
        let services = match &*self.state.read().await {
            GoogleState::Connected(account) => account.active(),
            GoogleState::Loading
            | GoogleState::Unavailable { .. }
            | GoogleState::NotConnected
            | GoogleState::Connecting { .. }
            | GoogleState::NeedsSignIn(_) => return None,
        };
        if services.is_empty() {
            return None;
        }
        let state = self.state.clone();
        let messages = self.messages.clone();
        Some(GoogleTools::from_fn(services, move |request| {
            let connector = connector.clone();
            let state = state.clone();
            let messages = messages.clone();
            async move {
                let service = request.service();
                // Checked again per call: the person may switch a service off mid-reply.
                let allowed = matches!(&*state.read().await,
                    GoogleState::Connected(account) if account.is_active(service));
                if !allowed {
                    return Err(format!(
                        "{} is switched off in Connections, so Sovra can't use it.",
                        service_info(service).product
                    ));
                }
                let changes = request.changes_things();
                match connector.call(request).await {
                    Ok(result) => {
                        tracing::info!(?service, changes, "google tool call");
                        Ok(result)
                    }
                    Err(GoogleError::SignInNeeded(message)) => {
                        let _ = messages.send(Message::SignInExpired).await;
                        Err(message)
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
        }))
    }
}

impl Drop for GoogleRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Pending {
    canceller: SignInCanceller,
    /// What to go back to if this attempt does not finish.
    before: GoogleState,
    adding: Option<GoogleService>,
}

struct Worker {
    connector: Option<Arc<dyn GoogleConnector>>,
    state: Arc<RwLock<GoogleState>>,
    events: Sender<Event>,
    messages: Sender<Message>,
    handle: tokio::runtime::Handle,
    pending: Option<Pending>,
    /// Ignores a late result from an attempt that was replaced or cancelled.
    attempt: u64,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, GoogleError> + Send + 'static,
) -> Result<T, GoogleError> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        Err(GoogleError::Failed(
            "Sovra's Google helper stopped unexpectedly. Please try again.".into(),
        ))
    })
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
                    if let GoogleState::Connected(account) = current {
                        self.publish(GoogleState::NeedsSignIn(account)).await;
                    }
                }
            }
        }
    }

    async fn publish(&self, state: GoogleState) {
        state.clone_into(&mut *self.state.write().await);
        let _ = self
            .events
            .send(Event::Google(GoogleEvent::Changed(state)))
            .await;
    }

    async fn notice(&self, message: impl Into<String>) {
        let _ = self
            .events
            .send(Event::Google(GoogleEvent::Notice(message.into())))
            .await;
    }

    async fn command(&mut self, command: GoogleCommand) {
        // Runtimes started without Google (tests, embedders) say nothing about it.
        let Some(connector) = self.connector.clone() else {
            return;
        };
        // Saving a client is what makes Google available, so it runs first.
        // The keychain read stays off this task.
        if !matches!(command, GoogleCommand::SaveClient { .. }) {
            let connector = connector.clone();
            let reason = tokio::task::spawn_blocking(move || connector.unavailable_reason())
                .await
                .ok()
                .flatten();
            if let Some(reason) = reason {
                self.publish(GoogleState::Unavailable { reason }).await;
                return;
            }
        }
        let current = self.state.read().await.clone();
        match command {
            GoogleCommand::Load => self.restore_account(&connector).await,
            GoogleCommand::SaveClient {
                client_id,
                client_secret,
            } => {
                let secret = client_secret.map(|secret| secret.as_str().to_owned());
                let saved = blocking({
                    let connector = connector.clone();
                    move || connector.save_desktop_client(&client_id, secret)
                })
                .await;
                match saved {
                    Ok(()) => self.restore_account(&connector).await,
                    Err(error) => {
                        self.notice(error.to_string()).await;
                        let connector = connector.clone();
                        let reason =
                            tokio::task::spawn_blocking(move || connector.unavailable_reason())
                                .await
                                .ok()
                                .flatten();
                        if let Some(reason) = reason {
                            self.publish(GoogleState::Unavailable { reason }).await;
                        }
                    }
                }
            }
            GoogleCommand::Connect => {
                let hint = current.account().map(|a| a.email.clone());
                let all = GoogleService::ALL.into_iter().collect();
                self.sign_in(&connector, all, hint, None).await;
            }
            GoogleCommand::SetService { service, on } => {
                let GoogleState::Connected(mut account) = current else {
                    return;
                };
                if on && !account.granted.contains(&service) {
                    // Desktop apps can't add one scope later; ask again for all of them.
                    let mut services = account.granted.clone();
                    services.insert(service);
                    let hint = Some(account.email.clone());
                    self.sign_in(&connector, services, hint, Some(service))
                        .await;
                    return;
                }
                if on {
                    account.turned_off.remove(&service);
                } else {
                    account.turned_off.insert(service);
                }
                self.save_and_publish(&connector, account).await;
            }
            GoogleCommand::Cancel => self.cancel_pending(),
            GoogleCommand::Disconnect => {
                // Drop the attempt outright so its late result can't restore anything.
                if let Some(pending) = self.pending.take() {
                    (pending.canceller)();
                    self.attempt += 1;
                }
                if let Err(error) = connector.revoke().await {
                    // Still sign out here; Google's own page can remove access later.
                    tracing::warn!(%error, "google revoke failed");
                    self.notice(error.to_string()).await;
                }
                let forgotten = blocking({
                    let connector = connector.clone();
                    move || connector.forget()
                })
                .await;
                if let Err(error) = forgotten {
                    self.notice(error.to_string()).await;
                }
                self.publish(GoogleState::NotConnected).await;
            }
        }
    }

    async fn restore_account(&self, connector: &Arc<dyn GoogleConnector>) {
        let restored = blocking({
            let connector = connector.clone();
            move || connector.restore()
        })
        .await;
        match restored {
            Ok(Some(account)) => self.publish(GoogleState::Connected(account)).await,
            Ok(None) => self.publish(GoogleState::NotConnected).await,
            Err(error) => {
                tracing::warn!(%error, "google restore failed");
                self.publish(GoogleState::NotConnected).await;
                self.notice(error.to_string()).await;
            }
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(pending) = &self.pending {
            (pending.canceller)();
        }
    }

    async fn save_and_publish(&self, connector: &Arc<dyn GoogleConnector>, account: GoogleAccount) {
        let saved = blocking({
            let connector = connector.clone();
            let account = account.clone();
            move || connector.save(&account)
        })
        .await;
        if let Err(error) = saved {
            self.notice(error.to_string()).await;
        }
        self.publish(GoogleState::Connected(account)).await;
    }

    async fn sign_in(
        &mut self,
        connector: &Arc<dyn GoogleConnector>,
        services: BTreeSet<GoogleService>,
        hint: Option<String>,
        adding: Option<GoogleService>,
    ) {
        // A second press replaces the first attempt, keeping what it would restore.
        let before = match self.pending.take() {
            Some(previous) => {
                (previous.canceller)();
                previous.before
            }
            None => self.state.read().await.clone(),
        };
        let begun = blocking({
            let connector = connector.clone();
            move || connector.begin_sign_in(&services, hint.as_deref())
        })
        .await;
        let sign_in = match begun {
            Ok(sign_in) => sign_in,
            Err(error) => {
                self.publish(before).await;
                self.notice(error.to_string()).await;
                return;
            }
        };
        self.attempt += 1;
        let attempt = self.attempt;
        let url = sign_in.url().to_owned();
        let canceller = sign_in.canceller();
        self.pending = Some(Pending {
            canceller: canceller.clone(),
            before,
            adding,
        });
        self.publish(GoogleState::Connecting {
            url: url.clone(),
            adding,
        })
        .await;
        let _ = self
            .events
            .send(Event::Google(GoogleEvent::OpenSignIn { url }))
            .await;
        let connector = connector.clone();
        let messages = self.messages.clone();
        drop(self.handle.spawn(async move {
            let waiting = tokio::task::spawn_blocking(move || sign_in.wait());
            let grant = match tokio::time::timeout(SIGN_IN_TIMEOUT, waiting).await {
                Ok(Ok(grant)) => grant,
                Ok(Err(_)) => Err(GoogleError::Failed("Sign-in stopped unexpectedly.".into())),
                Err(_) => {
                    canceller();
                    Err(GoogleError::Failed(
                        "Nothing happened in your browser, so Sovra stopped waiting. Press \
                         Connect Google to try again."
                            .into(),
                    ))
                }
            };
            let result = match grant {
                Ok(grant) => connector.complete_sign_in(grant).await,
                Err(error) => Err(error),
            };
            let _ = messages.send(Message::SignedIn { attempt, result }).await;
        }));
    }

    async fn signed_in(&mut self, result: Result<GoogleAccount, GoogleError>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let connector = match &self.connector {
            Some(connector) => connector.clone(),
            None => return,
        };
        match result {
            Ok(mut account) => {
                // Same person: keep what they switched off, except what they just turned on.
                if let Some(previous) = pending.before.account()
                    && previous.email == account.email
                {
                    account.turned_off.clone_from(&previous.turned_off);
                }
                if let Some(service) = pending.adding {
                    account.turned_off.remove(&service);
                    if !account.granted.contains(&service) {
                        self.notice(format!(
                            "Google didn't allow {}, so it stays off. Nothing else changed.",
                            service_info(service).product
                        ))
                        .await;
                    }
                }
                self.save_and_publish(&connector, account).await;
            }
            Err(error) => {
                self.publish(pending.before).await;
                if error != GoogleError::Cancelled {
                    self.notice(error.to_string()).await;
                }
            }
        }
    }
}
