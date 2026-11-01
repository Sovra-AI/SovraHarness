//! Sign-in runtime: owns the account records, verifies passwords off the
//! executor, runs the RFC 8252 loopback listener for Google Sign-In, and
//! persists everything through the memory store. One task owns all state.
//!
//! Secrets policy: password hashes (PHC strings) live in the memory store;
//! the Google refresh token goes to the providers' credential store — the OS
//! keychain, or dev files only under the approved `SOVRA_DEV=1` debug
//! exception. Tokens, codes and passwords never appear in events or logs.

use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use sovra_core::auth::{
    AuthAccount, AuthCommand, AuthEvent, AuthField, AuthProvider, AuthRecords, AuthSession,
    OauthCallback, callback_page, normalize_email, parse_oauth_redirect, password_strength,
};
use sovra_core::google::GoogleConnector;
use sovra_core::{Event, MemoryStore};
use sovra_providers::{GoogleAuthFlow, GoogleIdentity, GoogleOAuthConfig, ModelRepository};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// How long the loopback listener waits for the person to finish in the browser.
const GOOGLE_FLOW_TIMEOUT: Duration = Duration::from_secs(300);
/// Budget for one loopback request; a real redirect is a few hundred bytes.
const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Credential-store id for the Google refresh token. It sits beside the model
/// keys, so the release-keychain and `SOVRA_DEV` dev-file rules apply as-is.
const REFRESH_TOKEN_ID: &str = "google-signin-refresh-token";

enum Input {
    /// The memory store, once the runtime has opened it.
    Store(Option<Arc<dyn MemoryStore>>),
    Command(AuthCommand),
    /// What the Google browser hand-off produced.
    GoogleOutcome(Box<Result<GoogleIdentity, String>>),
}

/// Handle the runtime loop uses to hand work to the auth task.
pub(crate) struct AuthRuntime {
    inbox: Sender<Input>,
    task: tokio::task::JoinHandle<()>,
}

impl AuthRuntime {
    pub(crate) fn start(
        google: Option<GoogleOAuthConfig>,
        connector: Option<Arc<dyn GoogleConnector>>,
        repository: Option<ModelRepository>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (inbox, rx) = async_channel::unbounded();
        let actor = Actor {
            handle: handle.clone(),
            store: None,
            google,
            connector,
            repository,
            events,
            records: AuthRecords::default(),
            pending_load: false,
            browser_wait: None,
            inbox: inbox.clone(),
        };
        let task = handle.spawn(actor.run(rx));
        Self { inbox, task }
    }

    /// The runtime opens its store after the event plumbing exists.
    pub(crate) fn attach_store(&self, store: Option<Arc<dyn MemoryStore>>) {
        let _ = self.inbox.try_send(Input::Store(store));
    }

    pub(crate) fn handle(&self, command: AuthCommand) {
        let _ = self.inbox.try_send(Input::Command(command));
    }

    pub(crate) async fn stop(&mut self) {
        self.inbox.close();
        let _ = (&mut self.task).await;
    }
}

impl Drop for AuthRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Actor {
    handle: tokio::runtime::Handle,
    store: Option<Arc<dyn MemoryStore>>,
    google: Option<GoogleOAuthConfig>,
    /// Connections' Google adapter. When present, it decides the client id.
    connector: Option<Arc<dyn GoogleConnector>>,
    repository: Option<ModelRepository>,
    events: Sender<Event>,
    records: AuthRecords,
    /// A `Load` arrived before the store did; answer once records exist.
    pending_load: bool,
    /// The task awaiting the loopback redirect; aborted on cancel/retry.
    browser_wait: Option<tokio::task::JoinHandle<()>>,
    inbox: Sender<Input>,
}

impl Drop for Actor {
    fn drop(&mut self) {
        self.stop_browser_wait();
    }
}

impl Actor {
    async fn run(mut self, rx: Receiver<Input>) {
        while let Ok(input) = rx.recv().await {
            match input {
                Input::Store(store) => {
                    self.store = store;
                    self.load().await;
                    if std::mem::take(&mut self.pending_load) {
                        self.emit_loaded().await;
                    }
                }
                Input::Command(command) => self.command(command).await,
                Input::GoogleOutcome(outcome) => self.google_outcome(*outcome).await,
            }
        }
        self.stop_browser_wait();
    }

    async fn load(&mut self) {
        let Some(store) = &self.store else { return };
        match store.load_auth().await {
            Ok(Some(saved)) => self.records = saved,
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "saved sign-in records unreadable"),
        }
    }

    async fn command(&mut self, command: AuthCommand) {
        match command {
            AuthCommand::Load => {
                if self.store.is_none() {
                    self.pending_load = true;
                }
                self.emit_loaded().await;
            }
            AuthCommand::CreateAccount {
                email,
                password,
                full_name,
            } => self.create_account(email, &password, full_name).await,
            AuthCommand::SignIn { email, password } => self.sign_in(email, &password).await,
            AuthCommand::StartGoogleSignIn => self.start_google().await,
            AuthCommand::CancelGoogleSignIn => {
                self.stop_browser_wait();
                self.emit(AuthEvent::GoogleSignInCancelled).await;
            }
            AuthCommand::SignOut => {
                self.stop_browser_wait();
                self.records.session = None;
                self.save().await;
                self.forget_refresh_token().await;
                self.emit(AuthEvent::SignedOut).await;
            }
        }
    }

    async fn emit_loaded(&self) {
        let google_ready = self.resolve_google_config().await.is_some();
        self.emit(AuthEvent::Loaded {
            session: self.records.session.clone(),
            has_accounts: !self.records.accounts.is_empty(),
            google_ready,
        })
        .await;
    }

    /// Saved on this computer, else the environment, else baked — the same
    /// decision Connect Google uses. A connector is that decision by itself.
    async fn resolve_google_config(&self) -> Option<GoogleOAuthConfig> {
        if let Some(connector) = self.connector.clone() {
            let client = tokio::task::spawn_blocking(move || connector.desktop_client())
                .await
                .ok()
                .flatten()?;
            return Some(GoogleOAuthConfig {
                client_id: client.client_id,
                client_secret: client.client_secret,
            });
        }
        self.google
            .clone()
            .filter(|config| !config.client_id.trim().is_empty())
    }

    async fn create_account(&mut self, email: String, password: &str, full_name: String) {
        self.emit(AuthEvent::Submitting).await;
        let email = match normalize_email(&email) {
            Ok(email) => email,
            Err(message) => return self.fail(AuthField::Email, message).await,
        };
        if self.records.account_by_email(&email).is_some() {
            return self
                .fail(
                    AuthField::Email,
                    "There's already an account with this email. Sign in instead.".into(),
                )
                .await;
        }
        let strength = password_strength(password);
        if !strength.accepted() {
            return self
                .fail(AuthField::Password, strength.label().into())
                .await;
        }
        let password = password.to_owned();
        let hashed = tokio::task::spawn_blocking(move || sovra_providers::hash_password(&password))
            .await
            .map_err(|_| sovra_providers::AuthError::Hashing)
            .and_then(|result| result);
        let password_phc = match hashed {
            Ok(phc) => phc,
            Err(error) => return self.fail(AuthField::General, error.to_string()).await,
        };
        let full_name = display_name(&full_name, &email);
        let account = AuthAccount {
            id: account_id(&self.records),
            email,
            full_name,
            provider: AuthProvider::Password,
            password_phc: Some(password_phc),
            created_at: crate::now(),
        };
        self.records.accounts.push(account.clone());
        self.finish_sign_in(&account, true).await;
    }

    async fn sign_in(&mut self, email: String, password: &str) {
        self.emit(AuthEvent::Submitting).await;
        let email = match normalize_email(&email) {
            Ok(email) => email,
            Err(message) => return self.fail(AuthField::Email, message).await,
        };
        let Some(account) = self.records.account_by_email(&email).cloned() else {
            return self
                .fail(
                    AuthField::Email,
                    "No account with that email yet. Create one below.".into(),
                )
                .await;
        };
        let Some(phc) = account.password_phc.clone() else {
            return self
                .fail(
                    AuthField::Password,
                    "This account signs in with Google. Use the Google button.".into(),
                )
                .await;
        };
        let password = password.to_owned();
        let verified =
            tokio::task::spawn_blocking(move || sovra_providers::verify_password(&phc, &password))
                .await
                .unwrap_or(false);
        if !verified {
            return self
                .fail(
                    AuthField::Password,
                    "That password isn't right. Check it and try again.".into(),
                )
                .await;
        }
        self.finish_sign_in(&account, false).await;
    }

    async fn start_google(&mut self) {
        let Some(config) = self.resolve_google_config().await else {
            return self
                .fail(
                    AuthField::General,
                    "Google needs a desktop client id before you can continue. Add one in \
                     Settings, under Connections. You can also use email and password."
                        .into(),
                )
                .await;
        };
        self.stop_browser_wait();
        // RFC 8252 §7.3: an OS-assigned ephemeral port on the IPv4 loopback.
        // The redirect URI uses the literal `127.0.0.1`, so one listener is enough.
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%error, "loopback listener failed to bind");
                return self
                    .fail(
                        AuthField::General,
                        "Could not open a local port for Google Sign-In.".into(),
                    )
                    .await;
            }
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(_) => {
                return self
                    .fail(
                        AuthField::General,
                        "Could not open a local port for Google Sign-In.".into(),
                    )
                    .await;
            }
        };
        let flow = match GoogleAuthFlow::begin(config, port) {
            Ok(flow) => flow,
            Err(error) => return self.fail(AuthField::General, error.to_string()).await,
        };
        self.emit(AuthEvent::GoogleSignInStarted {
            auth_url: flow.auth_url().to_owned(),
        })
        .await;
        let inbox = self.inbox.clone();
        self.browser_wait = Some(self.handle.spawn(async move {
            let outcome = await_google(listener, flow).await;
            let _ = inbox.try_send(Input::GoogleOutcome(Box::new(outcome)));
        }));
    }

    async fn google_outcome(&mut self, outcome: Result<GoogleIdentity, String>) {
        self.browser_wait = None;
        let identity = match outcome {
            Ok(identity) => identity,
            Err(message) => return self.fail(AuthField::General, message).await,
        };
        let new_account = self.records.account_by_email(&identity.email).is_none();
        let account = if new_account {
            let account = AuthAccount {
                id: account_id(&self.records),
                email: identity.email.clone(),
                full_name: identity.full_name.clone(),
                provider: AuthProvider::Google,
                password_phc: None,
                created_at: crate::now(),
            };
            self.records.accounts.push(account.clone());
            account
        } else {
            // The email matched an existing account and Google verified it,
            // so this is the same person; keep their account as it is.
            self.records
                .account_by_email(&identity.email)
                .cloned()
                .expect("account_by_email checked above")
        };
        self.store_refresh_token(identity.refresh_token).await;
        self.finish_sign_in(&account, new_account).await;
    }

    async fn finish_sign_in(&mut self, account: &AuthAccount, new_account: bool) {
        let session = AuthSession {
            account_id: account.id.clone(),
            email: account.email.clone(),
            full_name: account.full_name.clone(),
            provider: account.provider,
            signed_in_at: crate::now(),
        };
        self.records.session = Some(session.clone());
        self.save().await;
        self.emit(AuthEvent::SignedIn {
            session,
            new_account,
        })
        .await;
    }

    async fn store_refresh_token(&self, token: Option<String>) {
        let (Some(repository), Some(token)) = (self.repository.clone(), token) else {
            return;
        };
        let result = tokio::task::spawn_blocking(move || {
            repository.set_credential(REFRESH_TOKEN_ID, &sovra_core::models::ModelSecret(token))
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!("could not store the Google refresh token in the credential store");
        }
    }

    async fn forget_refresh_token(&self) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let result =
            tokio::task::spawn_blocking(move || repository.delete_credential(REFRESH_TOKEN_ID))
                .await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!("could not remove the Google refresh token from the credential store");
        }
    }

    fn stop_browser_wait(&mut self) {
        if let Some(task) = self.browser_wait.take() {
            task.abort();
        }
    }

    async fn fail(&self, field: AuthField, message: String) {
        self.emit(AuthEvent::Failed { field, message }).await;
    }

    async fn emit(&self, event: AuthEvent) {
        let _ = self.events.send(Event::Auth(event)).await;
    }

    async fn save(&self) {
        if let Some(store) = &self.store
            && let Err(error) = store.save_auth(&self.records).await
        {
            tracing::warn!(%error, "failed to persist sign-in records");
        }
    }
}

/// `account-<n>-<millis>`: readable, unique, and stable across restarts.
fn account_id(records: &AuthRecords) -> String {
    format!("account-{}-{}", records.accounts.len() + 1, crate::now().0)
}

/// A friendly display name, falling back to the email's local part.
fn display_name(full_name: &str, email: &str) -> String {
    let trimmed = full_name.trim();
    if trimmed.is_empty() {
        email.split('@').next().unwrap_or(email).to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Waits for the browser redirect, answers it with a friendly page, then
/// redeems the code. Every error is a message safe to show in the form.
async fn await_google(
    listener: TcpListener,
    flow: GoogleAuthFlow,
) -> Result<GoogleIdentity, String> {
    let code = tokio::time::timeout(GOOGLE_FLOW_TIMEOUT, wait_for_callback(listener, &flow))
        .await
        .map_err(|_| {
            "Google Sign-In timed out. Click the Google button to try again.".to_owned()
        })??;
    flow.exchange(code).await.map_err(|error| error.to_string())
}

async fn wait_for_callback(listener: TcpListener, flow: &GoogleAuthFlow) -> Result<String, String> {
    loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "The local sign-in listener stopped unexpectedly.".to_owned())?;
        // Only this computer may answer the redirect.
        if !peer.ip().is_loopback() {
            continue;
        }
        let mut buffer = vec![0_u8; MAX_REQUEST_BYTES];
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer)).await;
        let Ok(Ok(length)) = read else { continue };
        let request = String::from_utf8_lossy(&buffer[..length]);
        let line = request.lines().next().unwrap_or_default();
        match parse_oauth_redirect(line) {
            None => {
                // Favicons and probes: answer and keep waiting for the real redirect.
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
                    .await;
            }
            Some(OauthCallback::Denied { reason }) => {
                let _ = stream.write_all(callback_page(false).as_bytes()).await;
                return Err(denied_message(&reason));
            }
            Some(OauthCallback::Granted { code, state }) => {
                if flow.state_matches(&state) {
                    let _ = stream.write_all(callback_page(true).as_bytes()).await;
                    return Ok(code);
                }
                let _ = stream.write_all(callback_page(false).as_bytes()).await;
                return Err(
                    "The sign-in answer didn't match this attempt, so it was ignored. \
                     Try again."
                        .to_owned(),
                );
            }
        }
    }
}

fn denied_message(reason: &str) -> String {
    if reason == "access_denied" {
        "Sign-in was cancelled in the browser. Nothing was changed.".to_owned()
    } else {
        format!("Google could not finish the sign-in ({reason}). Try again.")
    }
}
