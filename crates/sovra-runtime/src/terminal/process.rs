//! Starting one shell command, streaming its output and stopping its whole
//! process tree.
//!
//! The shell runs in its own process group (Unix) or process group plus no
//! console window (Windows), with stdin closed, so a program waiting for input
//! fails fast instead of hanging. Timeouts, Stop and an interrupted
//! conversation all end the whole tree, not just the shell.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_channel::Sender;
use sovra_core::Event;
use sovra_core::terminal::{
    CallStatus, HostSystem, LineDecoder, NON_INTERACTIVE_ENV, OutputCapture, TerminalEvent,
    UI_MAX_LINES,
};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::oneshot;

/// How often output is forwarded to the chat: smooth to watch, cheap to render.
const FLUSH_EVERY: Duration = Duration::from_millis(80);
/// After the shell exits, how long to keep reading pipes a background child may hold.
const EXIT_GRACE: Duration = Duration::from_millis(300);
/// After a polite stop, how long before the tree is killed outright.
const KILL_AFTER: Duration = Duration::from_secs(2);
const READ_CHUNK: usize = 16 * 1024;

/// One command to run.
pub(super) struct Job<'a> {
    pub host: &'a HostSystem,
    pub env: &'a [(OsString, OsString)],
    pub command: &'a str,
    pub dir: &'a Path,
    pub timeout: Duration,
    pub call_id: &'a str,
    pub events: &'a Sender<Event>,
}

/// Runs `job` until it exits, times out or `stop` fires. Returns the final status and
/// the bounded output for the model.
pub(super) async fn run(job: Job<'_>, stop: oneshot::Receiver<()>) -> (CallStatus, OutputCapture) {
    let started = Instant::now();
    let mut command = tokio::process::Command::new(&job.host.shell_path);
    command
        .args(job.host.shell.invocation(job.command))
        .current_dir(job.dir)
        .envs(job.env.iter().map(|(key, value)| (key, value)))
        .envs(NON_INTERACTIVE_ENV.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    own_group(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, shell = %job.host.shell_path, "terminal command failed to start");
            return (
                CallStatus::FailedToStart {
                    message: format!("Could not start {}: {error}", job.host.shell.label()),
                },
                OutputCapture::default(),
            );
        }
    };
    let mut tree = ProcessTree::new(child.id());
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let mut stream = Stream::new(job.call_id, job.events);
    let mut out_buf = vec![0_u8; READ_CHUNK];
    let mut err_buf = vec![0_u8; READ_CHUNK];

    let deadline = tokio::time::sleep(job.timeout);
    tokio::pin!(deadline);
    tokio::pin!(stop);
    let mut stop_open = true;
    let mut flush = tokio::time::interval(FLUSH_EVERY);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ended: Option<fn(u64) -> CallStatus> = None;
    let mut hard_kill: Option<Pin<Box<tokio::time::Sleep>>> = None;
    let mut grace: Option<Pin<Box<tokio::time::Sleep>>> = None;
    let mut exit: Option<std::io::Result<std::process::ExitStatus>> = None;

    loop {
        if stdout.is_none() && stderr.is_none() && exit.is_some() {
            tree.exited_cleanly();
            break;
        }
        tokio::select! {
            read = read_some(&mut stdout, &mut out_buf), if stdout.is_some() => {
                if let Some(bytes) = read {
                    stream.out.push(&out_buf[..bytes], &mut stream.lines);
                } else {
                    stdout = None;
                    stream.out.finish(&mut stream.lines);
                }
            }
            read = read_some(&mut stderr, &mut err_buf), if stderr.is_some() => {
                if let Some(bytes) = read {
                    stream.err.push(&err_buf[..bytes], &mut stream.lines);
                } else {
                    stderr = None;
                    stream.err.finish(&mut stream.lines);
                }
            }
            status = child.wait(), if exit.is_none() => {
                exit = Some(status);
                grace = Some(Box::pin(tokio::time::sleep(EXIT_GRACE)));
            },
            () = wait_for(&mut grace), if grace.is_some() => {
                // The shell is gone but something it started still holds the pipes.
                stream.out.finish(&mut stream.lines);
                stream.err.finish(&mut stream.lines);
                break;
            },
            () = &mut deadline, if ended.is_none() && exit.is_none() => {
                ended = Some(|duration_ms| CallStatus::TimedOut { duration_ms });
                tree.terminate();
                hard_kill = Some(Box::pin(tokio::time::sleep(KILL_AFTER)));
            },
            signal = &mut stop, if stop_open && ended.is_none() && exit.is_none() => {
                stop_open = false;
                // A dropped sender means the call finished bookkeeping, not "stop".
                if signal.is_ok() {
                    ended = Some(|duration_ms| CallStatus::Stopped { duration_ms });
                    tree.terminate();
                    hard_kill = Some(Box::pin(tokio::time::sleep(KILL_AFTER)));
                }
            },
            () = wait_for(&mut hard_kill), if hard_kill.is_some() => {
                hard_kill = None;
                tree.kill();
                let _ = child.start_kill();
            },
            _ = flush.tick() => stream.flush(false),
        }
        stream.capture_new_lines();
    }
    stream.capture_new_lines();
    stream.flush(true);
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let status = match (ended, exit) {
        (Some(status), _) => status(duration_ms),
        (None, Some(Ok(status))) => CallStatus::Finished {
            exit_code: status.code(),
            duration_ms,
        },
        (None, Some(Err(error))) => {
            tracing::warn!(%error, "waiting for a terminal command failed");
            CallStatus::Finished {
                exit_code: None,
                duration_ms,
            }
        }
        (None, None) => CallStatus::Stopped { duration_ms },
    };
    (status, stream.capture)
}

async fn read_some<R: AsyncRead + Unpin>(pipe: &mut Option<R>, buf: &mut [u8]) -> Option<usize> {
    let reader = pipe.as_mut()?;
    match reader.read(buf).await {
        Ok(0) | Err(_) => None,
        Ok(bytes) => Some(bytes),
    }
}

async fn wait_for(timer: &mut Option<Pin<Box<tokio::time::Sleep>>>) {
    match timer.as_mut() {
        Some(sleep) => sleep.as_mut().await,
        None => std::future::pending().await,
    }
}

/// Decoded output on its way to the model's capture and the chat.
struct Stream<'a> {
    call_id: &'a str,
    events: &'a Sender<Event>,
    out: LineDecoder,
    err: LineDecoder,
    /// Lines decoded since the last loop turn.
    lines: Vec<String>,
    /// Lines not yet forwarded to the chat.
    pending: VecDeque<String>,
    skipped: u64,
    capture: OutputCapture,
    last_partial: String,
}

impl<'a> Stream<'a> {
    fn new(call_id: &'a str, events: &'a Sender<Event>) -> Self {
        Self {
            call_id,
            events,
            out: LineDecoder::default(),
            err: LineDecoder::default(),
            lines: Vec::new(),
            pending: VecDeque::new(),
            skipped: 0,
            capture: OutputCapture::default(),
            last_partial: String::new(),
        }
    }

    fn capture_new_lines(&mut self) {
        for line in self.lines.drain(..) {
            self.capture.push(line.clone());
            if self.pending.len() == UI_MAX_LINES {
                // Faster than anyone can read: keep the newest, count the rest.
                self.pending.pop_front();
                self.skipped += 1;
            }
            self.pending.push_back(line);
        }
    }

    fn flush(&mut self, last: bool) {
        let partial = if last {
            String::new()
        } else {
            // The unfinished line, e.g. a progress message or a prompt.
            let out = self.out.partial();
            if out.is_empty() {
                self.err.partial()
            } else {
                out
            }
            .to_owned()
        };
        if self.pending.is_empty() && self.skipped == 0 && partial == self.last_partial {
            return;
        }
        self.last_partial.clone_from(&partial);
        let _ = self.events.try_send(Event::Terminal(TerminalEvent::Output {
            call_id: self.call_id.to_owned(),
            lines: self.pending.drain(..).collect(),
            partial,
            skipped_lines: std::mem::take(&mut self.skipped),
        }));
    }
}

/// Starts the command in a new process group so the whole tree can be signalled
/// (and, on Windows, without a console window).
pub(crate) fn own_group(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

/// Keeps a console window from flashing up on Windows; nothing elsewhere.
pub(super) fn hide_window(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Stops a whole process group when dropped, whatever state it is in. Used for
/// long-lived helper programs (connected services) started with [`own_group`].
pub(crate) struct GroupGuard(ProcessTree);

impl GroupGuard {
    pub(crate) fn new(pid: Option<u32>) -> Self {
        Self(ProcessTree::new(pid))
    }
}

impl std::fmt::Debug for GroupGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("GroupGuard")
            .field(&self.0.pid)
            .finish()
    }
}

/// The started command's process tree. Dropping it while the command may still be
/// running (Stop, timeout, an interrupted conversation) kills every process in it.
struct ProcessTree {
    pid: Option<u32>,
    live: bool,
}

impl ProcessTree {
    fn new(pid: Option<u32>) -> Self {
        Self {
            live: pid.is_some(),
            pid,
        }
    }

    /// The shell exited and nothing kept its output open: nothing is left to kill.
    fn exited_cleanly(&mut self) {
        self.live = false;
    }

    /// Asks every process in the tree to stop.
    fn terminate(&self) {
        self.signal(false);
    }

    /// Ends every process in the tree now.
    fn kill(&self) {
        self.signal(true);
    }

    #[cfg(unix)]
    fn signal(&self, hard: bool) {
        use nix::sys::signal::{Signal, killpg};
        let Some(pid) = self.pid.and_then(|pid| i32::try_from(pid).ok()) else {
            return;
        };
        let signal = if hard {
            Signal::SIGKILL
        } else {
            Signal::SIGTERM
        };
        // ESRCH (already gone) is the normal race here.
        let _ = killpg(nix::unistd::Pid::from_raw(pid), signal);
    }

    #[cfg(windows)]
    fn signal(&self, _hard: bool) {
        // Windows has no group signals for console-less children; taskkill walks the
        // tree. It is a short-lived helper, so it is not waited for.
        let Some(pid) = self.pid else {
            return;
        };
        let mut taskkill = std::process::Command::new("taskkill");
        taskkill
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        {
            use std::os::windows::process::CommandExt;
            taskkill.creation_flags(0x0800_0000);
        }
        let _ = taskkill.spawn();
    }

    #[cfg(not(any(unix, windows)))]
    fn signal(&self, _hard: bool) {}
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        if self.live {
            self.kill();
        }
    }
}
