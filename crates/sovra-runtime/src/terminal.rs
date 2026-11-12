//! Terminal access for the assistant: approval cards, saved "always allow" rules,
//! and running commands. The rules themselves are pure, in
//! `sovra_core::terminal`; this module does the waiting, the processes and the file.

mod host;
mod process;

pub(crate) use process::{GroupGuard, own_group as own_process_group};

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_channel::Sender;
use serde::{Deserialize, Serialize};
use sovra_core::Event;
use sovra_core::terminal::{
    APPROVAL_WAIT_SECS, CallStatus, HostSystem, Permission, TerminalAccess, TerminalCall,
    TerminalCommand, TerminalDecision, TerminalEvent, TerminalOutcome, TerminalPolicy,
    TerminalRequest, Verdict,
};
use tokio::sync::{OnceCell, oneshot};

/// Where terminal settings live and how commands start. Read once in `sovra-app`.
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// The JSON file holding [`TerminalPolicy`].
    pub settings_path: PathBuf,
    /// Commands start here unless the assistant names another folder.
    pub working_dir: PathBuf,
    /// The person's login shell (`$SHELL`), when known.
    pub login_shell: Option<String>,
    /// `PATH`, for finding a shell.
    pub search_path: Option<OsString>,
    /// Add the login shell's environment (`PATH` from shell profiles) to commands.
    /// The app wants this; tests do not want to run the developer's profile.
    pub use_login_env: bool,
}

const SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
    version: u32,
    policy: TerminalPolicy,
}

/// The runtime's terminal service. Absent configuration means no terminal at all.
pub(crate) struct TerminalRuntime {
    shared: Option<Arc<Shared>>,
    handle: tokio::runtime::Handle,
}

#[derive(Debug)]
struct Shared {
    host: HostSystem,
    settings_path: PathBuf,
    working_dir: PathBuf,
    /// The app's own `PATH`, used when the login shell gave none.
    search_path: Option<OsString>,
    policy: Mutex<TerminalPolicy>,
    /// Approval cards waiting for an answer, by call id.
    waiting: Mutex<HashMap<String, oneshot::Sender<TerminalDecision>>>,
    /// Stop buttons for running commands, by call id.
    running: Mutex<HashMap<String, oneshot::Sender<()>>>,
    events: Sender<Event>,
    login_env: OnceCell<Vec<(OsString, OsString)>>,
    next_call: AtomicU64,
    /// Serialises settings writes so the newest policy always lands last.
    saving: tokio::sync::Mutex<()>,
    /// Owned tasks (approval wait that must outlive a dropped tool future).
    handle: tokio::runtime::Handle,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section is a plain map/vec update that cannot leave a value
    // half-written, so a poisoned lock is still consistent.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TerminalRuntime {
    pub(crate) async fn start(
        config: Option<TerminalConfig>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let Some(config) = config else {
            return Self {
                shared: None,
                handle: handle.clone(),
            };
        };
        let host = host::detect(&config).await;
        let policy = load_policy(&config.settings_path).await;
        tracing::info!(
            os = %host.os_version,
            shell = %host.shell_path,
            rules = policy.rules.len(),
            enabled = policy.enabled,
            "terminal ready"
        );
        let shared = Arc::new(Shared {
            host: host.clone(),
            settings_path: config.settings_path,
            working_dir: config.working_dir,
            search_path: config.search_path.clone(),
            policy: Mutex::new(policy.clone()),
            waiting: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            events,
            login_env: if config.use_login_env {
                OnceCell::new()
            } else {
                OnceCell::new_with(Some(Vec::new()))
            },
            next_call: AtomicU64::new(1),
            saving: tokio::sync::Mutex::new(()),
            handle: handle.clone(),
        });
        shared.emit(TerminalEvent::Ready { host, policy });
        // Resolve the login environment now so the first command does not wait for it.
        let warm = Arc::clone(&shared);
        drop(handle.spawn(async move {
            warm.login_env().await;
        }));
        Self {
            shared: Some(shared),
            handle: handle.clone(),
        }
    }

    /// The person's login-shell environment, shared with other helpers that start
    /// programs (connected services), so `npx`, `uvx` and friends are found the
    /// same way the terminal finds them. `None` without terminal configuration.
    pub(crate) fn environment(&self) -> Option<LoginEnvironment> {
        self.shared.as_ref().map(|shared| LoginEnvironment {
            shared: Arc::clone(shared),
        })
    }

    /// The terminal for one conversation turn; `None` when unavailable or switched off.
    pub(crate) fn access(&self) -> Option<Arc<dyn TerminalAccess>> {
        let shared = self.shared.as_ref()?;
        if !lock(&shared.policy).enabled {
            return None;
        }
        Some(Arc::new(Access(Arc::clone(shared))))
    }

    /// Long-lived project builder access; every run still checks the current policy.
    pub(crate) fn project_access(&self) -> Option<Arc<dyn TerminalAccess>> {
        self.shared
            .as_ref()
            .map(|shared| Arc::new(Access(Arc::clone(shared))) as Arc<dyn TerminalAccess>)
    }

    pub(crate) fn handle(&self, command: TerminalCommand) {
        let Some(shared) = &self.shared else {
            return;
        };
        match command {
            TerminalCommand::Decide { call_id, decision } => {
                if let Some(answer) = lock(&shared.waiting).remove(&call_id) {
                    let _ = answer.send(decision);
                }
            }
            TerminalCommand::Stop { call_id } => {
                if let Some(stop) = lock(&shared.running).remove(&call_id) {
                    let _ = stop.send(());
                } else if let Some(answer) = lock(&shared.waiting).remove(&call_id) {
                    let _ = answer.send(TerminalDecision::Deny);
                }
            }
            TerminalCommand::RemoveRule(rule) => {
                if lock(&shared.policy).remove_rule(&rule) {
                    self.policy_changed(shared);
                }
            }
            TerminalCommand::SetEnabled(enabled) => {
                {
                    let mut policy = lock(&shared.policy);
                    if policy.enabled == enabled {
                        return;
                    }
                    policy.enabled = enabled;
                }
                if !enabled {
                    // Switching off revokes access now: pending cards are declined and
                    // running commands stopped.
                    for (_, answer) in lock(&shared.waiting).drain() {
                        let _ = answer.send(TerminalDecision::Deny);
                    }
                    for (_, stop) in lock(&shared.running).drain() {
                        let _ = stop.send(());
                    }
                }
                self.policy_changed(shared);
            }
        }
    }

    fn policy_changed(&self, shared: &Arc<Shared>) {
        shared.emit(TerminalEvent::PolicyChanged(lock(&shared.policy).clone()));
        let saver = Arc::clone(shared);
        drop(self.handle.spawn(async move { saver.save().await }));
    }
}

impl Shared {
    fn emit(&self, event: TerminalEvent) {
        // Unbounded channel: never blocks, so this is safe from `Drop`.
        let _ = self.events.try_send(Event::Terminal(event));
    }

    async fn login_env(&self) -> &[(OsString, OsString)] {
        self.login_env
            .get_or_init(|| host::login_environment(&self.host))
            .await
    }

    async fn save(&self) {
        let _writing = self.saving.lock().await;
        let file = SettingsFile {
            version: SETTINGS_VERSION,
            policy: lock(&self.policy).clone(),
        };
        if let Err(error) = write_settings(&self.settings_path, &file).await {
            tracing::warn!(%error, path = %self.settings_path.display(), "could not save terminal settings");
        }
    }

    /// The folder to run in: the assistant's choice when it exists, else the default.
    async fn resolve_dir(&self, requested: Option<&str>) -> Result<PathBuf, String> {
        let Some(requested) = requested else {
            return Ok(self.working_dir.clone());
        };
        let path = match requested
            .strip_prefix("~/")
            .or(requested.strip_prefix("~\\"))
        {
            Some(rest) => self.working_dir.join(rest),
            None if requested == "~" => self.working_dir.clone(),
            None => PathBuf::from(requested),
        };
        if !path.is_absolute() {
            return Err(format!(
                "The folder must be a full path; \"{requested}\" is relative."
            ));
        }
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.is_dir() => Ok(path),
            _ => Err(format!("The folder \"{}\" does not exist.", path.display())),
        }
    }
}

async fn load_policy(path: &Path) -> TerminalPolicy {
    match tokio::fs::read(path).await {
        Ok(bytes) => match serde_json::from_slice::<SettingsFile>(&bytes) {
            Ok(file) if file.version == SETTINGS_VERSION => file.policy,
            Ok(file) => {
                tracing::warn!(
                    version = file.version,
                    "unknown terminal settings version; using defaults"
                );
                TerminalPolicy::default()
            }
            Err(error) => {
                tracing::warn!(%error, "unreadable terminal settings; using defaults");
                TerminalPolicy::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => TerminalPolicy::default(),
        Err(error) => {
            tracing::warn!(%error, "could not read terminal settings; using defaults");
            TerminalPolicy::default()
        }
    }
}

async fn write_settings(path: &Path, file: &SettingsFile) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(file).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Write-then-rename so a crash never leaves half a file.
    let temporary = path.with_extension("json.tmp");
    tokio::fs::write(&temporary, json).await?;
    tokio::fs::rename(&temporary, path).await
}

/// A handle on the cached login-shell environment (see [`TerminalRuntime::environment`]).
#[derive(Debug, Clone)]
pub(crate) struct LoginEnvironment {
    shared: Arc<Shared>,
}

impl LoginEnvironment {
    /// The login environment, resolved once (the first caller waits).
    pub(crate) async fn variables(&self) -> Vec<(OsString, OsString)> {
        self.shared.login_env().await.to_vec()
    }

    /// `PATH` to search for programs: the login shell's, else the app's.
    pub(crate) async fn search_path(&self) -> Option<OsString> {
        let login = self.shared.login_env().await;
        login
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value.clone())
            .or_else(|| self.shared.search_path.clone())
    }
}

/// The per-turn handle given to the assistant's tool.
#[derive(Debug)]
struct Access(Arc<Shared>);

/// Marks an approval card expired if the owned waiter is dropped (app shutdown)
/// while it waits. Dropping the *tool future* (model-response timeout) must not
/// expire a still-waiting card — Allow once / Always allow still run it.
struct WaitGuard {
    shared: Arc<Shared>,
    call: TerminalCall,
    armed: bool,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if self.armed {
            lock(&self.shared.waiting).remove(&self.call.call_id);
            let mut call = self.call.clone();
            call.status = CallStatus::Expired;
            self.shared.emit(TerminalEvent::Call(call));
        }
    }
}

/// Marks a running command stopped if the turn is dropped while it runs (the
/// process tree itself is killed by `process::ProcessTree`).
struct RunGuard {
    shared: Arc<Shared>,
    call: TerminalCall,
    started: Instant,
    armed: bool,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        lock(&self.shared.running).remove(&self.call.call_id);
        if self.armed {
            let mut call = self.call.clone();
            call.status = CallStatus::Stopped {
                duration_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            };
            self.shared.emit(TerminalEvent::Call(call));
        }
    }
}

/// Work that must keep going if the model turn is dropped while a card is waiting.
struct OwnedCall {
    shared: Arc<Shared>,
    request: TerminalRequest,
    call: TerminalCall,
    dir: PathBuf,
    needs_approval: bool,
    parent_dead: oneshot::Receiver<()>,
    outcome_tx: oneshot::Sender<TerminalOutcome>,
}

#[async_trait::async_trait]
impl TerminalAccess for Access {
    fn host(&self) -> &HostSystem {
        &self.0.host
    }

    async fn run(&self, request: TerminalRequest) -> TerminalOutcome {
        let shared = &*self.0;
        // Model call ids repeat across turns ("call_0"), so make them unique.
        let serial = shared.next_call.fetch_add(1, Ordering::Relaxed);
        let call_id = if request.call_id.is_empty() {
            format!("terminal-{serial}")
        } else {
            format!("{}-{serial}", request.call_id)
        };
        let policy = lock(&shared.policy).clone();
        let dir = shared.resolve_dir(request.working_dir.as_deref()).await;
        let mut call = TerminalCall {
            call_id: call_id.clone(),
            session_id: request.session_id.clone(),
            command: request.command.clone(),
            description: request.description.clone(),
            working_dir: dir.as_ref().map_or_else(
                |_| request.working_dir.clone().unwrap_or_default(),
                |dir| dir.display().to_string(),
            ),
            status: CallStatus::AwaitingApproval,
            permission: Permission::Pending,
            prompt: sovra_core::terminal::ApprovalPrompt::default(),
        };
        if !policy.enabled {
            return settle(shared, call, CallStatus::Disabled);
        }
        let dir = match dir {
            Ok(dir) => dir,
            Err(message) => return settle(shared, call, CallStatus::FailedToStart { message }),
        };

        let needs_approval = match policy.evaluate(&request.command, shared.host.os) {
            Verdict::Run(auto) => {
                call.permission = Permission::Auto(auto);
                false
            }
            Verdict::Ask(prompt) => {
                call.prompt = prompt;
                true
            }
        };

        // The waiter lives on the runtime handle so a model-response timeout (which
        // drops this tool future) cannot expire a card the person has not answered.
        let (parent_alive, parent_dead) = oneshot::channel();
        let (outcome_tx, outcome_rx) = oneshot::channel();
        drop(shared.handle.spawn(run_owned(OwnedCall {
            shared: Arc::clone(&self.0),
            request,
            call,
            dir,
            needs_approval,
            parent_dead,
            outcome_tx,
        })));
        let _turn = parent_alive;
        outcome_rx
            .await
            .unwrap_or(TerminalOutcome::NotRun(CallStatus::Expired))
    }
}

async fn run_owned(owned: OwnedCall) {
    let OwnedCall {
        shared,
        request,
        call,
        dir,
        needs_approval,
        parent_dead,
        outcome_tx,
    } = owned;
    if let Some(outcome) =
        execute_owned(shared, request, call, dir, needs_approval, parent_dead).await
    {
        let _ = outcome_tx.send(outcome);
    }
}

async fn execute_owned(
    shared: Arc<Shared>,
    request: TerminalRequest,
    mut call: TerminalCall,
    dir: PathBuf,
    needs_approval: bool,
    mut parent_dead: oneshot::Receiver<()>,
) -> Option<TerminalOutcome> {
    if needs_approval {
        let (answer, decision) = oneshot::channel();
        lock(&shared.waiting).insert(call.call_id.clone(), answer);
        shared.emit(TerminalEvent::Call(call.clone()));
        let mut guard = WaitGuard {
            shared: Arc::clone(&shared),
            call: call.clone(),
            armed: true,
        };
        let decided = tokio::time::timeout(Duration::from_secs(APPROVAL_WAIT_SECS), decision).await;
        guard.armed = false;
        drop(guard);
        lock(&shared.waiting).remove(&call.call_id);
        match decided {
            Ok(Ok(TerminalDecision::AlwaysAllow)) if !call.prompt.suggested_rules.is_empty() => {
                let rules = call.prompt.suggested_rules.clone();
                let changed = lock(&shared.policy).add_rules(&rules);
                if changed {
                    shared.emit(TerminalEvent::PolicyChanged(lock(&shared.policy).clone()));
                    shared.save().await;
                }
                call.permission = Permission::Saved(rules);
            }
            // "Always" with nothing to remember is the same as "once".
            Ok(Ok(TerminalDecision::AllowOnce | TerminalDecision::AlwaysAllow)) => {
                call.permission = Permission::Once;
            }
            Ok(Ok(TerminalDecision::Deny)) => {
                call.permission = Permission::Refused;
                let status = if lock(&shared.policy).enabled {
                    CallStatus::Declined
                } else {
                    CallStatus::Disabled
                };
                return Some(settle(&shared, call, status));
            }
            Ok(Err(_)) | Err(_) => return Some(settle(&shared, call, CallStatus::Expired)),
        }
    }

    // A model timeout while the card was waiting must still run the command once
    // the person allows it. A timeout after the process started still stops it.
    let turn_already_ended = !matches!(
        parent_dead.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );

    call.status = CallStatus::Running;
    let (stop, stopped) = oneshot::channel();
    lock(&shared.running).insert(call.call_id.clone(), stop);
    shared.emit(TerminalEvent::Call(call.clone()));
    let mut guard = RunGuard {
        shared: Arc::clone(&shared),
        call: call.clone(),
        started: Instant::now(),
        armed: true,
    };
    let env = shared.login_env().await;
    let process_job = process::Job {
        host: &shared.host,
        env,
        command: &request.command,
        dir: &dir,
        timeout: Duration::from_secs(request.bounded_timeout_secs()),
        call_id: &call.call_id,
        events: &shared.events,
    };
    let (status, capture) = if turn_already_ended {
        tracing::info!(
            call_id = %call.call_id,
            "person allowed a command after the model turn ended; running it"
        );
        process::run(process_job, stopped).await
    } else {
        tokio::select! {
            result = process::run(process_job, stopped) => result,
            _ = parent_dead => {
                // Dropping process::run kills the tree. RunGuard emits Stopped.
                return None;
            }
        }
    };
    guard.armed = false;
    drop(guard);
    tracing::info!(
        call_id = %call.call_id,
        ?status,
        lines = capture.total_lines(),
        "terminal command settled"
    );
    call.status = status.clone();
    shared.emit(TerminalEvent::Call(call));
    Some(match status {
        CallStatus::FailedToStart { .. } => TerminalOutcome::NotRun(status),
        _ => TerminalOutcome::Ran {
            status,
            output: capture.render(),
            total_lines: capture.total_lines(),
        },
    })
}

/// Shows a call that never ran and returns the matching outcome.
fn settle(shared: &Shared, mut call: TerminalCall, status: CallStatus) -> TerminalOutcome {
    call.status = status.clone();
    shared.emit(TerminalEvent::Call(call));
    TerminalOutcome::NotRun(status)
}

#[cfg(test)]
mod tests;
