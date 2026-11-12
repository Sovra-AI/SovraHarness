//! Recurring schedules persist, honour pause, and dispatch through the chat path.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sovra_core::connections::ChannelKind;
use sovra_core::{
    AgentConfig, Command, Event, ScheduleCommand, ScheduleEvent, ScheduleStatus, ScheduledSend,
    Timestamp,
};
use sovra_runtime::{Client, JsonScheduleStore, RuntimeServices};

const WAIT: Duration = Duration::from_secs(10);

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sovra-schedules-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos())
        ));
        Self { dir }
    }

    fn start(&self) -> Client {
        sovra_runtime::start_with_services(
            "sched-test",
            AgentConfig::default(),
            RuntimeServices {
                schedules: Some(Arc::new(JsonScheduleStore::new(
                    self.dir.join("schedules.json"),
                ))),
                ..RuntimeServices::default()
            },
        )
        .expect("runtime")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn wait_ready(events: &async_channel::Receiver<Event>) {
    wait_until(events, |event| matches!(event, Event::Ready { .. })).await;
}

async fn wait_snapshot(
    events: &async_channel::Receiver<Event>,
    pred: impl Fn(&[sovra_core::ScheduleItem], &[sovra_core::ScheduleRun]) -> bool,
) -> (Vec<sovra_core::ScheduleItem>, Vec<sovra_core::ScheduleRun>) {
    loop {
        match wait_schedule(events).await {
            ScheduleEvent::Snapshot { schedules, runs } if pred(&schedules, &runs) => {
                return (schedules, runs);
            }
            _ => {}
        }
    }
}

async fn wait_schedule(events: &async_channel::Receiver<Event>) -> ScheduleEvent {
    loop {
        if let Event::Schedule(event) = recv(events).await {
            return event;
        }
    }
}

async fn wait_until(
    events: &async_channel::Receiver<Event>,
    pred: impl Fn(&Event) -> bool,
) -> Event {
    loop {
        let event = recv(events).await;
        if pred(&event) {
            return event;
        }
    }
}

async fn recv(events: &async_channel::Receiver<Event>) -> Event {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("timed out waiting for the runtime")
        .expect("runtime stopped")
}

fn send(client: &Client, command: ScheduleCommand) {
    client
        .send(Command::Schedule(command))
        .expect("runtime accepts schedule commands");
}

#[tokio::test]
async fn persist_tick_pause_and_reload() {
    let fixture = Fixture::new();
    let client = fixture.start();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    let (schedules, _) = wait_snapshot(&events, |items, _| items.is_empty()).await;
    assert_eq!(
        schedules,
        Vec::new(),
        "a new workspace starts with no schedules"
    );

    send(
        &client,
        ScheduleCommand::Create {
            name: "Weekday briefing".into(),
            cadence: "every hour".into(),
            prompt: "Give me a short briefing of what's on today".into(),
            send: None,
        },
    );
    let (schedules, _) = wait_snapshot(&events, |items, _| items.len() == 1).await;
    assert_eq!(schedules[0].name, "Weekday briefing");
    assert_eq!(schedules[0].status, ScheduleStatus::Active);
    let active_id = schedules[0].id.clone();
    let due = schedules[0].next_run_at;

    send(
        &client,
        ScheduleCommand::Create {
            name: "Paused twin".into(),
            cadence: "every hour".into(),
            prompt: "This should stay quiet".into(),
            send: None,
        },
    );
    let (schedules, _) = wait_snapshot(&events, |items, _| items.len() == 2).await;
    let paused_id = schedules
        .iter()
        .find(|item| item.name == "Paused twin")
        .expect("paused twin")
        .id
        .clone();
    send(
        &client,
        ScheduleCommand::Pause {
            id: paused_id.clone(),
        },
    );
    let (schedules, _) = wait_snapshot(&events, |items, _| {
        items
            .iter()
            .any(|item| item.id == paused_id && item.status == ScheduleStatus::Paused)
    })
    .await;
    let paused_due = schedules
        .iter()
        .find(|item| item.id == paused_id)
        .expect("paused")
        .next_run_at;
    let tick_at = Timestamp(due.0.max(paused_due.0));

    send(&client, ScheduleCommand::Tick { now: tick_at });
    let fired = wait_until(&events, |event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::Fired { schedule_id, .. }) if *schedule_id == active_id
        )
    })
    .await;
    let Event::Schedule(ScheduleEvent::Fired {
        session_id, name, ..
    }) = fired
    else {
        panic!("expected a fired schedule");
    };
    assert_eq!(name, "Weekday briefing");
    assert!(
        session_id.starts_with(&active_id),
        "auto fire uses a dedicated session, got {session_id}"
    );

    send(&client, ScheduleCommand::Load);
    let (_, runs) = wait_snapshot(&events, |items, runs| {
        items.len() == 2
            && runs.iter().any(|run| run.schedule_id == active_id)
            && !runs.iter().any(|run| run.schedule_id == paused_id)
    })
    .await;
    assert!(
        runs.iter()
            .any(|run| run.schedule_id == active_id && run.session_id == session_id),
        "the dispatched run is recorded"
    );

    client.shutdown_blocking().expect("shutdown");

    let client = fixture.start();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    let (schedules, _) = wait_snapshot(&events, |items, _| items.len() == 2).await;
    assert!(schedules.iter().any(|item| item.name == "Weekday briefing"));
    assert!(
        schedules
            .iter()
            .any(|item| item.name == "Paused twin" && item.status == ScheduleStatus::Paused)
    );
    client.shutdown_blocking().expect("shutdown");
}

#[tokio::test]
async fn reload_clears_stale_in_flight_so_the_next_tick_fires() {
    let fixture = Fixture::new();
    let client = fixture.start();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    wait_snapshot(&events, |items, _| items.is_empty()).await;

    send(
        &client,
        ScheduleCommand::Create {
            name: "Hourly brief".into(),
            cadence: "every hour".into(),
            prompt: "Give me a short briefing of what's on today".into(),
            send: None,
        },
    );
    let (schedules, _) = wait_snapshot(&events, |items, _| items.len() == 1).await;
    let schedule_id = schedules[0].id.clone();
    let due = schedules[0].next_run_at;

    send(&client, ScheduleCommand::Tick { now: due });
    let (schedules, _) = wait_snapshot(&events, |items, runs| {
        items.len() == 1
            && items[0].next_run_at.0 > due.0
            && runs
                .iter()
                .any(|run| run.schedule_id == schedule_id && run.finished_at.is_none())
    })
    .await;
    let next = schedules[0].next_run_at;
    let fired = wait_until(&events, |event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::Fired { schedule_id: id, .. }) if *id == schedule_id
        )
    })
    .await;
    let Event::Schedule(ScheduleEvent::Fired { session_id, .. }) = fired else {
        panic!("expected a fired schedule");
    };
    client.shutdown_blocking().expect("shutdown");

    let client = fixture.start();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    let (_, runs) = wait_snapshot(&events, |items, runs| {
        items.len() == 1
            && runs.iter().any(|run| run.schedule_id == schedule_id)
            && runs.iter().all(|run| run.finished_at.is_some())
    })
    .await;
    assert!(
        runs.iter().all(|run| run.finished_at.is_some()),
        "a restart must close leftover in-flight runs so the job can fire again"
    );

    send(&client, ScheduleCommand::Tick { now: next });
    let fired = wait_until(&events, |event| {
        matches!(
            event,
            Event::Schedule(ScheduleEvent::Fired { schedule_id: id, .. }) if *id == schedule_id
        )
    })
    .await;
    let Event::Schedule(ScheduleEvent::Fired {
        session_id: again, ..
    }) = fired
    else {
        panic!("expected the next due tick to fire after reload");
    };
    assert_ne!(again, session_id, "the second fire is a new run");
    client.shutdown_blocking().expect("shutdown");
}

#[tokio::test]
async fn create_in_one_minute_lists_as_one_shot_not_daily() {
    let fixture = Fixture::new();
    let client = fixture.start();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    wait_snapshot(&events, |items, _| items.is_empty()).await;

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| {
            i64::try_from(time.as_millis()).unwrap_or(i64::MAX)
        });

    send(
        &client,
        ScheduleCommand::Create {
            name: "Wake Ryan".into(),
            cadence: "in 1 minute".into(),
            prompt: "text Ryan: wake up from sovra".into(),
            send: None,
        },
    );
    let (schedules, _) = wait_snapshot(&events, |items, _| items.len() == 1).await;
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| {
            i64::try_from(time.as_millis()).unwrap_or(i64::MAX)
        });

    assert_eq!(schedules[0].name, "Wake Ryan");
    assert_eq!(schedules[0].prompt, "text Ryan: wake up from sovra");
    assert_eq!(
        schedules[0].cadence,
        sovra_core::ScheduleCadence::Once { delay_ms: 60_000 }
    );
    let label = schedules[0].cadence_label();
    assert!(
        !label.contains("Every day"),
        "one-shot listed as repeating: {label}"
    );
    assert!(
        label.contains("Once"),
        "expected a one-shot label, got {label}"
    );
    let next = schedules[0].next_run_at.0;
    assert!(
        next >= before + 50_000,
        "next-run {next} is too soon after {before}"
    );
    assert!(
        next <= after + 70_000,
        "next-run {next} is too far after {after}"
    );

    send(&client, ScheduleCommand::Load);
    let (listed, _) = wait_snapshot(&events, |items, _| items.len() == 1).await;
    assert_eq!(listed[0].cadence_label(), label);
    assert!(!listed[0].cadence_label().contains("Every day"));
    client.shutdown_blocking().expect("shutdown");
}

#[tokio::test]
async fn timed_send_create_rejects_a_first_name() {
    let fixture = Fixture::new();
    let client = fixture.start();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    wait_snapshot(&events, |items, _| items.is_empty()).await;

    send(
        &client,
        ScheduleCommand::Create {
            name: "Keep this".into(),
            cadence: "in 1 minute".into(),
            prompt: "Say ping".into(),
            send: None,
        },
    );
    wait_snapshot(&events, |items, _| items.len() == 1).await;

    send(
        &client,
        ScheduleCommand::Create {
            name: "Wake Ryan".into(),
            cadence: "in 1 minute".into(),
            prompt: "Send a WhatsApp to Ryan".into(),
            send: Some(ScheduledSend {
                channel: ChannelKind::WhatsApp,
                recipient: "Ryan".into(),
                text: "wake up from sovra".into(),
            }),
        },
    );
    send(&client, ScheduleCommand::Load);
    let (listed, _) = wait_snapshot(&events, |items, _| items.len() == 1).await;
    assert_eq!(listed[0].name, "Keep this");
    assert!(listed.iter().all(|item| item.send.is_none()));
    client.shutdown_blocking().expect("shutdown");
}

#[derive(Debug, Default)]
struct UnavailableStore {
    fail: std::sync::atomic::AtomicBool,
}

impl sovra_core::ScheduleStore for UnavailableStore {
    fn load(&self) -> Result<sovra_core::ScheduleBook, sovra_core::ScheduleError> {
        Ok(sovra_core::ScheduleBook::default())
    }
    fn save(&self, _: &sovra_core::ScheduleBook) -> Result<(), sovra_core::ScheduleError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            Err(sovra_core::ScheduleError("Test disk unavailable".into()))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn failed_schedule_write_is_not_published_as_saved_and_can_be_retried() {
    let store = Arc::new(UnavailableStore::default());
    let client = sovra_runtime::start_with_services(
        "schedule-write-failure",
        AgentConfig::default(),
        RuntimeServices {
            schedules: Some(store.clone()),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    let events = client.events();
    wait_ready(&events).await;
    send(&client, ScheduleCommand::Load);
    wait_snapshot(&events, |_, _| true).await;
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let create = ScheduleCommand::Create {
        name: "Garden reminder".into(),
        cadence: "every day at 9".into(),
        prompt: "Check the garden".into(),
        send: None,
    };
    send(&client, create.clone());
    wait_until(&events, |event| {
        matches!(event, Event::Schedule(ScheduleEvent::Error(_)))
    })
    .await;
    let (schedules, _) = wait_snapshot(&events, |_, _| true).await;
    assert!(
        schedules.is_empty(),
        "failed creation must not become an active task"
    );
    store.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    send(&client, create);
    let (schedules, _) = wait_snapshot(&events, |items, _| items.len() == 1).await;
    let created = wait_until(&events, |event| {
        matches!(event, Event::Schedule(ScheduleEvent::Created { .. }))
    })
    .await;
    assert!(
        matches!(created, Event::Schedule(ScheduleEvent::Created { schedule }) if schedule.id == schedules[0].id)
    );
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    for command in [
        ScheduleCommand::RunNow {
            id: schedules[0].id.clone(),
            session_id: Some("must-not-start".into()),
        },
        ScheduleCommand::Pause {
            id: schedules[0].id.clone(),
        },
        ScheduleCommand::Delete {
            id: schedules[0].id.clone(),
        },
    ] {
        send(&client, command);
        wait_until(&events, |event| {
            matches!(event, Event::Schedule(ScheduleEvent::Error(_)))
        })
        .await;
        let (after, runs) = wait_snapshot(&events, |_, _| true).await;
        assert_eq!(
            after, schedules,
            "failed changes must preserve the saved task"
        );
        assert!(
            runs.is_empty(),
            "failed run setup must not become a recorded run"
        );
        // A read acts as a barrier after any incorrectly dispatched run events.
        send(&client, ScheduleCommand::Load);
        loop {
            let event = recv(&events).await;
            assert!(
                !matches!(event, Event::Schedule(ScheduleEvent::Fired { .. })),
                "failed persistence must not dispatch work"
            );
            if matches!(event, Event::Schedule(ScheduleEvent::Snapshot { .. })) {
                break;
            }
        }
    }
    client.shutdown_blocking().unwrap();
}
