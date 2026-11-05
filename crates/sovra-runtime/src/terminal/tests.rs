//! Real processes through the real approval flow (Unix shells; Windows runs the
//! settings tests only).

use super::*;
use sovra_core::terminal::{AllowRule, AutoApproval};

struct Fixture {
    runtime: TerminalRuntime,
    events: async_channel::Receiver<Event>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn fixture(name: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("sovra-terminal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    fixture_in(dir).await
}

async fn fixture_in(dir: PathBuf) -> Fixture {
    let (events_tx, events) = async_channel::unbounded();
    let config = TerminalConfig {
        settings_path: dir.join("settings").join("terminal.json"),
        working_dir: dir.clone(),
        login_shell: None,
        search_path: std::env::var_os("PATH"),
        use_login_env: false,
    };
    let runtime =
        TerminalRuntime::start(Some(config), events_tx, &tokio::runtime::Handle::current()).await;
    let fixture = Fixture {
        runtime,
        events,
        dir,
    };
    match fixture.next().await {
        TerminalEvent::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    fixture
}

impl Fixture {
    async fn next(&self) -> TerminalEvent {
        let event = tokio::time::timeout(Duration::from_secs(20), self.events.recv())
            .await
            .expect("an event in time")
            .expect("channel open");
        match event {
            Event::Terminal(event) => event,
            other => panic!("unexpected event {other:?}"),
        }
    }

    /// The next call update, skipping output and policy events.
    async fn next_call(&self) -> TerminalCall {
        loop {
            if let TerminalEvent::Call(call) = self.next().await {
                return call;
            }
        }
    }

    fn access(&self) -> Arc<dyn TerminalAccess> {
        self.runtime.access().expect("terminal enabled")
    }

    fn spawn(&self, command: &str, timeout_secs: u64) -> tokio::task::JoinHandle<TerminalOutcome> {
        let access = self.access();
        let request = TerminalRequest {
            session_id: "chat-1".into(),
            call_id: "call_0".into(),
            command: command.into(),
            description: "test".into(),
            timeout_secs,
            working_dir: None,
        };
        #[expect(
            clippy::disallowed_methods,
            reason = "test drives the tool future directly"
        )]
        tokio::spawn(async move { access.run(request).await })
    }

    async fn awaiting(&self) -> TerminalCall {
        let call = self.next_call().await;
        assert_eq!(call.status, CallStatus::AwaitingApproval, "{call:?}");
        call
    }

    fn decide(&self, call: &TerminalCall, decision: TerminalDecision) {
        self.runtime.handle(TerminalCommand::Decide {
            call_id: call.call_id.clone(),
            decision,
        });
    }
}

async fn outcome(task: tokio::task::JoinHandle<TerminalOutcome>) -> TerminalOutcome {
    tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .expect("finished in time")
        .expect("task ran")
}

fn ran(outcome: &TerminalOutcome) -> (&CallStatus, &str) {
    match outcome {
        TerminalOutcome::Ran { status, output, .. } => (status, output),
        TerminalOutcome::NotRun(status) => panic!("did not run: {status:?}"),
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn read_only_commands_run_straight_away_and_stream() {
    let fixture = fixture("readonly").await;
    let task = fixture.spawn("echo hello && pwd", 10);
    let running = fixture.next_call().await;
    assert_eq!(running.status, CallStatus::Running);
    assert_eq!(running.permission, Permission::Auto(AutoApproval::ReadOnly));
    assert!(
        running.call_id.starts_with("call_0-"),
        "ids are made unique"
    );
    let result = outcome(task).await;
    let (status, output) = ran(&result);
    assert!(matches!(
        status,
        CallStatus::Finished {
            exit_code: Some(0),
            ..
        }
    ));
    assert!(output.starts_with("hello\n"), "{output}");
    // The chat saw the output and the final status.
    let mut streamed = Vec::new();
    loop {
        match fixture.next().await {
            TerminalEvent::Output { lines, .. } => streamed.extend(lines),
            TerminalEvent::Call(call) => {
                assert!(call.status.is_settled());
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(streamed.first().map(String::as_str), Some("hello"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn allow_once_runs_and_deny_does_not() {
    let fixture = fixture("once").await;
    let task = fixture.spawn("touch made.txt", 10);
    let card = fixture.awaiting().await;
    assert_eq!(
        card.prompt.suggested_rules,
        [AllowRule::new(vec!["touch".into()]).expect("rule")]
    );
    fixture.decide(&card, TerminalDecision::AllowOnce);
    let result = outcome(task).await;
    assert!(matches!(
        ran(&result).0,
        CallStatus::Finished {
            exit_code: Some(0),
            ..
        }
    ));
    assert!(fixture.dir.join("made.txt").exists());
    assert_eq!(
        lock(&fixture.runtime.shared.as_ref().expect("shared").policy).rules,
        []
    );

    let task = fixture.spawn("touch refused.txt", 10);
    let card = loop {
        let call = fixture.next_call().await;
        if call.status == CallStatus::AwaitingApproval {
            break call;
        }
    };
    fixture.decide(&card, TerminalDecision::Deny);
    assert_eq!(
        outcome(task).await,
        TerminalOutcome::NotRun(CallStatus::Declined)
    );
    assert!(!fixture.dir.join("refused.txt").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn always_allow_saves_the_rule_to_disk() {
    let fixture = fixture("always").await;
    let task = fixture.spawn("mkdir -p one", 10);
    let card = fixture.awaiting().await;
    fixture.decide(&card, TerminalDecision::AlwaysAllow);
    assert!(matches!(
        ran(&outcome(task).await).0,
        CallStatus::Finished {
            exit_code: Some(0),
            ..
        }
    ));

    // The next mkdir runs without a card.
    let task = fixture.spawn("mkdir -p two", 10);
    let result = outcome(task).await;
    assert!(matches!(ran(&result).0, CallStatus::Finished { .. }));
    assert!(fixture.dir.join("two").is_dir());

    let saved = std::fs::read_to_string(fixture.dir.join("settings/terminal.json")).expect("saved");
    let file: SettingsFile = serde_json::from_str(&saved).expect("json");
    assert_eq!(file.version, SETTINGS_VERSION);
    assert_eq!(
        file.policy.rules,
        [AllowRule::new(vec!["mkdir".into()]).expect("rule")]
    );

    // A fresh start loads the rule; removing it persists too.
    let mut fixture = fixture;
    let dir = std::mem::take(&mut fixture.dir);
    drop(fixture);
    let reloaded = fixture_in(dir).await;
    let shared = reloaded.runtime.shared.clone().expect("shared");
    assert_eq!(lock(&shared.policy).rules.len(), 1);
    reloaded.runtime.handle(TerminalCommand::RemoveRule(
        AllowRule::new(vec!["mkdir".into()]).expect("rule"),
    ));
    assert!(
        matches!(reloaded.next().await, TerminalEvent::PolicyChanged(policy) if policy.rules.is_empty())
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let text = tokio::fs::read_to_string(reloaded.dir.join("settings/terminal.json"))
                .await
                .unwrap_or_default();
            if text.contains("\"rules\": []") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("removal saved");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn timeouts_and_stop_end_the_command() {
    let fixture = fixture("stop").await;
    let started = Instant::now();
    let result = outcome(fixture.spawn("sleep 30", 1)).await;
    assert!(matches!(ran(&result).0, CallStatus::TimedOut { .. }));
    assert!(started.elapsed() < Duration::from_secs(10));

    let task = fixture.spawn("sleep 31", 60);
    let running = loop {
        let call = fixture.next_call().await;
        if call.status == CallStatus::Running && call.command == "sleep 31" {
            break call;
        }
    };
    fixture.runtime.handle(TerminalCommand::Stop {
        call_id: running.call_id,
    });
    let result = outcome(task).await;
    assert!(matches!(ran(&result).0, CallStatus::Stopped { .. }));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_turn_stops_its_command() {
    let fixture = fixture("abort").await;
    let marker = fixture.dir.join("late.txt");
    // The `touch` would only happen if the tree survived the interruption.
    let task = fixture.spawn(&format!("sleep 2; touch {}", marker.display()), 60);
    let task = {
        let call = fixture.awaiting().await;
        fixture.decide(&call, TerminalDecision::AllowOnce);
        assert_eq!(fixture.next_call().await.status, CallStatus::Running);
        task
    };
    task.abort();
    let stopped = fixture.next_call().await;
    assert!(
        matches!(stopped.status, CallStatus::Stopped { .. }),
        "{stopped:?}"
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!marker.exists(), "the process tree was killed");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn allowing_after_the_tool_future_is_aborted_still_runs() {
    let fixture = fixture("survive-timeout").await;
    let marker = fixture.dir.join("survived.txt");
    let task = fixture.spawn(&format!("touch {}", marker.display()), 10);
    let card = fixture.awaiting().await;
    // Same as the chat dropping the tool future when "The model response timed out".
    task.abort();
    // Give the old WaitGuard-on-drop path time to expire the card if the bug is back.
    tokio::time::sleep(Duration::from_millis(200)).await;
    while let Ok(event) = fixture.events.try_recv() {
        if let Event::Terminal(TerminalEvent::Call(call)) = event {
            assert_ne!(
                call.status,
                CallStatus::Expired,
                "a model timeout must not expire a still-waiting card: {call:?}"
            );
        }
    }
    fixture.decide(&card, TerminalDecision::AllowOnce);
    let running = fixture.next_call().await;
    assert_eq!(running.status, CallStatus::Running, "{running:?}");
    let finished = loop {
        let call = fixture.next_call().await;
        if call.status.is_settled() {
            break call;
        }
    };
    assert!(
        matches!(
            finished.status,
            CallStatus::Finished {
                exit_code: Some(0),
                ..
            }
        ),
        "{finished:?}"
    );
    assert!(
        marker.exists(),
        "Allow once still ran the command after the tool future was aborted"
    );
}

/// libc fully buffers stdout when it is a pipe. This program prints a unique
/// stdout line (no `fflush`) and a unique stderr line, then sleeps so a live
/// capture can observe both *before* exit. `echo` is not this case: it flushes.
#[cfg(unix)]
fn compile_block_buffered_printer(dir: &Path) -> PathBuf {
    let src = dir.join("live.c");
    std::fs::write(
        &src,
        r#"#include <stdio.h>
#include <unistd.h>
int main(void) {
    printf("LIVE-STDOUT-a8f3c1\n");
    fprintf(stderr, "LIVE-STDERR-a8f3c1\n");
    sleep(2);
    return 0;
}
"#,
    )
    .expect("write live.c");
    let bin = dir.join("live");
    let status = std::process::Command::new("cc")
        .args(["-O0", "-o"])
        .arg(&bin)
        .arg(&src)
        .status()
        .expect("cc must be available to prove live libc output");
    assert!(status.success(), "cc failed: {status}");
    bin
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn live_stdout_and_stderr_are_readable_while_the_command_is_still_running() {
    const STDOUT_MARK: &str = "LIVE-STDOUT-a8f3c1";
    const STDERR_MARK: &str = "LIVE-STDERR-a8f3c1";
    let fixture = fixture("live-io").await;
    compile_block_buffered_printer(&fixture.dir);
    let task = fixture.spawn("./live", 10);
    let card = fixture.awaiting().await;
    fixture.decide(&card, TerminalDecision::AllowOnce);
    assert_eq!(fixture.next_call().await.status, CallStatus::Running);

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut seen = String::new();
    let mut settled = false;
    while Instant::now() < deadline {
        let Ok(received) =
            tokio::time::timeout(Duration::from_millis(200), fixture.events.recv()).await
        else {
            continue;
        };
        match received.expect("event channel closed") {
            Event::Terminal(TerminalEvent::Output { lines, partial, .. }) => {
                for line in lines {
                    seen.push_str(&line);
                    seen.push('\n');
                }
                seen.push_str(&partial);
                if seen.contains(STDOUT_MARK) && seen.contains(STDERR_MARK) {
                    break;
                }
            }
            Event::Terminal(TerminalEvent::Call(call)) => {
                if call.status.is_settled() {
                    settled = true;
                    break;
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        !settled,
        "stdout and stderr must appear while the command is still running, not only after it exits"
    );
    assert!(
        seen.contains(STDOUT_MARK) && seen.contains(STDERR_MARK),
        "live card output missing unique stdout/stderr lines: {seen:?}"
    );

    let result = outcome(task).await;
    let report = result.report_for_model();
    let output = report["output"].as_str().unwrap_or_default();
    assert!(
        output.contains(STDOUT_MARK) && output.contains(STDERR_MARK),
        "model report missing captured text: {report}"
    );
    assert_eq!(report["status"], "finished");
    assert_eq!(report["exit_code"], 0);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn huge_output_is_bounded_for_the_model() {
    let fixture = fixture("flood").await;
    let task = fixture.spawn("seq 1 300000", 30);
    let card = fixture.awaiting().await;
    fixture.decide(&card, TerminalDecision::AllowOnce);
    match outcome(task).await {
        TerminalOutcome::Ran {
            output,
            total_lines,
            ..
        } => {
            assert_eq!(total_lines, 300_000);
            assert!(output.len() < 40 * 1024, "{} bytes", output.len());
            assert!(output.starts_with("1\n2\n"));
            assert!(output.ends_with("300000"));
        }
        other @ TerminalOutcome::NotRun(_) => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_off_declines_waiting_cards_and_hides_the_tool() {
    let fixture = fixture("disable").await;
    let task = fixture.spawn("touch nope.txt", 10);
    fixture.awaiting().await;
    fixture.runtime.handle(TerminalCommand::SetEnabled(false));
    assert_eq!(
        outcome(task).await,
        TerminalOutcome::NotRun(CallStatus::Disabled)
    );
    assert!(fixture.runtime.access().is_none());
    fixture.runtime.handle(TerminalCommand::SetEnabled(true));
    assert!(fixture.runtime.access().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_folder_is_reported_without_running() {
    let fixture = fixture("folder").await;
    let request = TerminalRequest {
        session_id: "chat-1".into(),
        call_id: String::new(),
        command: "ls".into(),
        description: String::new(),
        timeout_secs: 5,
        working_dir: Some("/definitely/not/here".into()),
    };
    match fixture.access().run(request).await {
        TerminalOutcome::NotRun(CallStatus::FailedToStart { message }) => {
            assert!(message.contains("does not exist"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let call = fixture.next_call().await;
    assert!(call.call_id.starts_with("terminal-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_configuration_means_no_terminal() {
    let (events, _rx) = async_channel::unbounded();
    let runtime = TerminalRuntime::start(None, events, &tokio::runtime::Handle::current()).await;
    assert!(runtime.access().is_none());
    runtime.handle(TerminalCommand::SetEnabled(true));
}
