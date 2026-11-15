//! Owns Microsoft sign-in attempts, service guards and document decoding.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use sovra_adk::MicrosoftTools;
use sovra_core::Event;
use sovra_core::files::FileStore;
use sovra_core::microsoft::{
    MicrosoftClient, MicrosoftCommand, MicrosoftConnector, MicrosoftError, MicrosoftEvent,
    MicrosoftOutput, MicrosoftService, MicrosoftSession, MicrosoftState, SERVICES, UNTRUSTED,
};
use sovra_core::oauth::{OAuthError, SignInCanceller};
use tokio::sync::{Mutex, RwLock};

enum Message {
    Command(MicrosoftCommand),
    Opened {
        attempt: u64,
        url: String,
    },
    Finished {
        attempt: u64,
        result: Result<Arc<dyn MicrosoftSession>, MicrosoftError>,
    },
    Expired {
        epoch: u64,
    },
}

#[derive(Default)]
struct Snapshot {
    state: MicrosoftState,
    session: Option<Arc<dyn MicrosoftSession>>,
    epoch: u64,
}

pub(crate) struct MicrosoftRuntime {
    messages: Sender<Message>,
    snapshot: Arc<RwLock<Snapshot>>,
    gate: Arc<Mutex<()>>,
    files: Option<Arc<dyn FileStore>>,
    task: tokio::task::JoinHandle<()>,
}

impl MicrosoftRuntime {
    pub(crate) fn start(
        connector: Option<Arc<dyn MicrosoftConnector>>,
        files: Option<Arc<dyn FileStore>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (messages, inbox) = async_channel::bounded(32);
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let gate = Arc::new(Mutex::new(()));
        let task = handle.spawn(
            Worker {
                connector,
                client: None,
                events,
                messages: messages.clone(),
                snapshot: snapshot.clone(),
                gate: gate.clone(),
                pending: None,
                attempt: 0,
                handle: handle.clone(),
            }
            .run(inbox),
        );
        Self {
            messages,
            snapshot,
            gate,
            files,
            task,
        }
    }

    pub(crate) fn send(&self, command: MicrosoftCommand) {
        if self.messages.try_send(Message::Command(command)).is_err() {
            tracing::warn!("Microsoft command queue is full");
        }
    }

    pub(crate) async fn tools(&self) -> Option<MicrosoftTools> {
        let snapshot = self.snapshot.read().await;
        let MicrosoftState::Connected(account) = &snapshot.state else {
            return None;
        };
        let services: BTreeSet<_> = account
            .granted
            .iter()
            .filter(|service| account.active(**service))
            .copied()
            .collect();
        if services.is_empty() {
            return None;
        }
        let epoch = snapshot.epoch;
        let state = self.snapshot.clone();
        let gate = self.gate.clone();
        let messages = self.messages.clone();
        let files = self.files.clone();
        Some(MicrosoftTools::from_fn(services, move |request| {
            let state = state.clone();
            let gate = gate.clone();
            let messages = messages.clone();
            let files = files.clone();
            async move {
                let _guard = gate.lock().await;
                let session = {
                    let current = state.read().await;
                    if current.epoch != epoch
                        || !matches!(&current.state, MicrosoftState::Connected(account) if account.active(request.service()))
                    {
                        return Err(
                            "This Microsoft service is no longer connected or is switched off."
                                .into(),
                        );
                    }
                    current
                        .session
                        .clone()
                        .ok_or("Microsoft needs you to connect again.")?
                };
                let result = session.call(request).await;
                let output = match result {
                    Ok(MicrosoftOutput::Data(data)) => Ok(data),
                    Ok(MicrosoftOutput::File { name, bytes }) => {
                        let files =
                            files.ok_or("This copy of Sovra cannot read downloaded documents.")?;
                        tokio::task::spawn_blocking(move || {
                            let content = files
                                .read_document(&name, &bytes)
                                .map_err(|error| error.to_string())?;
                            let limit = 40_000;
                            Ok(serde_json::json!({"untrusted":UNTRUSTED,"name":name,
                                "text":sovra_core::connected_text::clip_text(&content.text, limit),
                                "truncated":content.text.chars().count() > limit,
                                "note":content.attachment.note,
                                "pictures_not_read":!content.media.is_empty()}))
                        })
                        .await
                        .map_err(|_| "The document reader stopped unexpectedly.".to_owned())?
                    }
                    Err(MicrosoftError::SignInNeeded) => {
                        let _ = messages.try_send(Message::Expired { epoch });
                        Err(MicrosoftError::SignInNeeded.to_string())
                    }
                    Err(error) => Err(error.to_string()),
                };
                if state.read().await.epoch != epoch {
                    return Err("The Microsoft connection changed while reading. Try again with the current account.".into());
                }
                output
            }
        }))
    }
}

impl Drop for MicrosoftRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Pending {
    task: tokio::task::JoinHandle<()>,
    before: MicrosoftState,
    adding: Option<MicrosoftService>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct WakeOnDrop(SignInCanceller);
impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

struct Worker {
    connector: Option<Arc<dyn MicrosoftConnector>>,
    client: Option<MicrosoftClient>,
    events: Sender<Event>,
    messages: Sender<Message>,
    snapshot: Arc<RwLock<Snapshot>>,
    gate: Arc<Mutex<()>>,
    pending: Option<Pending>,
    attempt: u64,
    handle: tokio::runtime::Handle,
}

impl Worker {
    async fn run(mut self, inbox: Receiver<Message>) {
        while let Ok(message) = inbox.recv().await {
            match message {
                Message::Command(command) => self.command(command).await,
                Message::Opened { attempt, url } => {
                    if attempt == self.attempt && self.pending.is_some() {
                        self.publish(MicrosoftState::Connecting { url: url.clone() })
                            .await;
                        let _ = self
                            .events
                            .send(Event::Microsoft(MicrosoftEvent::OpenSignIn { url }))
                            .await;
                    }
                }
                Message::Finished { attempt, result } => {
                    if attempt == self.attempt {
                        self.finish(result).await;
                    }
                }
                Message::Expired { epoch } => {
                    let snapshot = self.snapshot.read().await;
                    if snapshot.epoch == epoch
                        && let MicrosoftState::Connected(account) = &snapshot.state
                    {
                        let account = account.clone();
                        drop(snapshot);
                        self.publish(MicrosoftState::NeedsSignIn(account)).await;
                    }
                }
            }
        }
    }

    async fn publish(&self, state: MicrosoftState) {
        self.snapshot.write().await.state = state.clone();
        let _ = self
            .events
            .send(Event::Microsoft(MicrosoftEvent::Changed(state)))
            .await;
    }

    async fn notice(&self, error: impl ToString) {
        let _ = self
            .events
            .send(Event::Microsoft(MicrosoftEvent::Notice(error.to_string())))
            .await;
    }

    async fn command(&mut self, command: MicrosoftCommand) {
        // Embedders without this adapter keep the minimal runtime event contract.
        // The desktop app supplies an adapter even before it has a client ID.
        let Some(connector) = self.connector.clone() else {
            return;
        };
        match command {
            MicrosoftCommand::Load => {
                if self.pending.is_some() || self.snapshot.read().await.session.is_some() {
                    return;
                }
                match connector.load_client().await {
                    Ok(client) => self.setup_changed(client).await,
                    Err(error) => {
                        self.setup_changed(None).await;
                        self.publish(MicrosoftState::NeedsSetup).await;
                        self.notice(error).await;
                        return;
                    }
                }
                let Some(client) = &self.client else {
                    self.publish(MicrosoftState::NeedsSetup).await;
                    return;
                };
                match connector.restore(client).await {
                    Ok(Some(session)) => {
                        let account = session.account();
                        self.snapshot.write().await.session = Some(session);
                        self.publish(MicrosoftState::Connected(account)).await;
                    }
                    Ok(None) => self.publish(MicrosoftState::NotConnected).await,
                    Err(error) => {
                        self.publish(MicrosoftState::NotConnected).await;
                        self.notice(error).await;
                    }
                }
            }
            MicrosoftCommand::SaveAndConnect(client) => {
                if self.pending.is_some() || self.snapshot.read().await.session.is_some() {
                    self.notice(
                        "Cancel sign-in or disconnect Microsoft 365 before changing setup.",
                    )
                    .await;
                    return;
                }
                if let Err(error) = client.validate() {
                    self.notice(error).await;
                    return;
                }
                if let Err(error) = connector.save_client(&client).await {
                    self.notice(error).await;
                    return;
                }
                self.setup_changed(Some(client)).await;
                self.publish(MicrosoftState::NotConnected).await;
                self.connect(connector, None).await;
            }
            MicrosoftCommand::ForgetSetup => {
                if self.pending.is_some() || self.snapshot.read().await.session.is_some() {
                    self.notice(
                        "Cancel sign-in or disconnect Microsoft 365 before forgetting setup.",
                    )
                    .await;
                    return;
                }
                match connector.forget_client().await {
                    Ok(()) => {
                        self.setup_changed(None).await;
                        self.publish(MicrosoftState::NeedsSetup).await;
                    }
                    Err(error) => self.notice(error).await,
                }
            }
            MicrosoftCommand::Connect => self.connect(connector, None).await,
            MicrosoftCommand::Cancel => {
                self.attempt = self.attempt.wrapping_add(1);
                if let Some(pending) = self.pending.take() {
                    self.publish(pending.before.clone()).await;
                }
            }
            MicrosoftCommand::Disconnect => {
                self.attempt = self.attempt.wrapping_add(1);
                self.pending = None;
                {
                    let mut snapshot = self.snapshot.write().await;
                    snapshot.epoch = snapshot.epoch.wrapping_add(1);
                }
                self.publish(MicrosoftState::NotConnected).await;
                let _guard = self.gate.lock().await;
                let session = self.snapshot.read().await.session.clone();
                if let Some(session) = session {
                    match session.disconnect().await {
                        Ok(()) => {
                            self.snapshot.write().await.session = None;
                        }
                        Err(error) => {
                            self.publish(MicrosoftState::NeedsSignIn(session.account()))
                                .await;
                            self.notice(error).await;
                        }
                    }
                }
            }
            MicrosoftCommand::SetService { service, on } => {
                let current = self.snapshot.read().await.state.clone();
                let MicrosoftState::Connected(mut account) = current else {
                    return;
                };
                if on && !account.granted.contains(&service) {
                    self.connect(connector, Some(service)).await;
                    return;
                }
                if on {
                    account.turned_off.remove(&service);
                } else {
                    account.turned_off.insert(service);
                }
                let _guard = self.gate.lock().await;
                let session = self.snapshot.read().await.session.clone();
                if let Some(session) = session {
                    match session.save(&account).await {
                        Ok(()) => self.publish(MicrosoftState::Connected(account)).await,
                        Err(error) => self.notice(error).await,
                    }
                }
            }
        }
    }

    async fn setup_changed(&mut self, client: Option<MicrosoftClient>) {
        self.client.clone_from(&client);
        let _ = self
            .events
            .send(Event::Microsoft(MicrosoftEvent::SetupChanged(client)))
            .await;
    }

    async fn connect(
        &mut self,
        connector: Arc<dyn MicrosoftConnector>,
        adding: Option<MicrosoftService>,
    ) {
        let Some(client) = self.client.clone() else {
            self.publish(MicrosoftState::NeedsSetup).await;
            return;
        };
        self.attempt = self.attempt.wrapping_add(1);
        let attempt = self.attempt;
        let before = if let Some(pending) = self.pending.take() {
            pending.before.clone()
        } else {
            self.snapshot.read().await.state.clone()
        };
        let mut services = before
            .account()
            .map_or_else(MicrosoftService::defaults, |account| {
                account.granted.clone()
            });
        if let Some(service) = adding {
            services.insert(service);
        }
        let hint = before.account().map(|account| account.email.clone());
        let messages = self.messages.clone();
        let task = self.handle.spawn(async move {
            let result =
                sign_in(connector, client, services, hint, messages.clone(), attempt).await;
            let _ = messages.send(Message::Finished { attempt, result }).await;
        });
        self.pending = Some(Pending {
            task,
            before,
            adding,
        });
        self.publish(MicrosoftState::Connecting { url: String::new() })
            .await;
    }

    async fn finish(&mut self, result: Result<Arc<dyn MicrosoftSession>, MicrosoftError>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let session = match result {
            Ok(session) => session,
            Err(error) => {
                self.publish(pending.before.clone()).await;
                self.notice(error).await;
                return;
            }
        };
        let mut account = session.account();
        if let Some(before) = pending.before.account() {
            if before.id != account.id {
                self.publish(pending.before.clone()).await;
                self.notice("Choose the same Microsoft account, or disconnect it before switching accounts.").await;
                return;
            }
            account.turned_off = before.turned_off.clone();
        } else {
            account.turned_off = SERVICES
                .iter()
                .filter(|info| info.organization_only)
                .map(|info| info.service)
                .collect();
        }
        if let Some(service) = pending.adding {
            account.turned_off.remove(&service);
        }
        let _guard = self.gate.lock().await;
        if let Err(error) = session.save(&account).await {
            self.publish(pending.before.clone()).await;
            self.notice(error).await;
            return;
        }
        let old = self.snapshot.write().await.session.replace(session);
        if let Some(old) = old {
            old.retire().await;
        }
        {
            let mut snapshot = self.snapshot.write().await;
            snapshot.epoch = snapshot.epoch.wrapping_add(1);
        }
        self.publish(MicrosoftState::Connected(account)).await;
    }
}

async fn sign_in(
    connector: Arc<dyn MicrosoftConnector>,
    client: MicrosoftClient,
    services: BTreeSet<MicrosoftService>,
    hint: Option<String>,
    messages: Sender<Message>,
    attempt: u64,
) -> Result<Arc<dyn MicrosoftSession>, MicrosoftError> {
    let begin = connector.clone();
    let config = client.clone();
    let pending = tokio::task::spawn_blocking(move || {
        begin.begin_sign_in(&config, &services, hint.as_deref())
    })
    .await
    .map_err(|_| stopped())??;
    let _cancel = WakeOnDrop(pending.canceller());
    let url = pending.url().to_owned();
    let _ = messages.send(Message::Opened { attempt, url }).await;
    let grant = tokio::time::timeout(
        Duration::from_secs(300),
        tokio::task::spawn_blocking(move || pending.wait()),
    )
    .await
    .map_err(|_| {
        MicrosoftError::Failed("Microsoft sign-in timed out. Press Connect to try again.".into())
    })?
    .map_err(|_| stopped())?
    .map_err(|error| match error {
        OAuthError::Cancelled => MicrosoftError::Cancelled,
        OAuthError::Denied => MicrosoftError::NotAllowed,
        OAuthError::Failed(message) => MicrosoftError::Failed(message),
    })?;
    connector.complete_sign_in(&client, grant).await
}

fn stopped() -> MicrosoftError {
    MicrosoftError::Failed(
        "The Microsoft connection stopped unexpectedly. Please try again.".into(),
    )
}
