//! A catalogue refresh probes installed links independently. One stalled link
//! cannot delay another, and cancelling the refresh drops the probes. A saved
//! link whose check never reached the service does not change its row.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sovra_core::connection_catalogue::{
    CatalogueKind, CatalogueSources, LinkPhase, LinkProbe, ProbeReport, catalogue_from,
};
use sovra_core::connections::{
    ChannelError, ChannelEvent, ChannelSession, ChannelSink, ChannelStartOptions,
    ConnectionCommand, ConnectionEvent, ConnectionsState, LinkedDevice, MessagingChannel,
    TelegramStatus, WhatsAppStatus,
};
use sovra_core::google::GoogleState;
use sovra_core::mcp::McpService;
use sovra_core::models::{CatalogState, ModelConnection};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{MessagingChannels, RuntimeServices};

struct FastLink;

#[async_trait::async_trait]
impl LinkProbe for FastLink {
    fn kind(&self) -> CatalogueKind {
        CatalogueKind::Telegram
    }

    async fn probe(&self) -> Option<ProbeReport> {
        Some(ProbeReport {
            kind: CatalogueKind::Telegram,
            phase: LinkPhase::Linked,
        })
    }
}

struct HeldLink {
    entered: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    release: async_channel::Receiver<()>,
}

struct Alive(Arc<AtomicBool>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl LinkProbe for HeldLink {
    fn kind(&self) -> CatalogueKind {
        CatalogueKind::WhatsApp
    }

    async fn probe(&self) -> Option<ProbeReport> {
        self.entered.store(true, Ordering::SeqCst);
        self.alive.store(true, Ordering::SeqCst);
        let _alive = Alive(self.alive.clone());
        let _ = self.release.recv().await;
        self.finished.store(true, Ordering::SeqCst);
        Some(ProbeReport {
            kind: CatalogueKind::WhatsApp,
            phase: LinkPhase::NeedsYou,
        })
    }
}

#[tokio::test]
async fn a_stalled_link_does_not_delay_or_rewrite_a_finished_one() {
    let entered = Arc::new(AtomicBool::new(false));
    let alive = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let (release_tx, release) = async_channel::bounded(1);
    let client = sovra_runtime::start_with_services(
        "catalogue-refresh",
        AgentConfig::default(),
        RuntimeServices {
            probes: vec![
                Arc::new(FastLink),
                Arc::new(HeldLink {
                    entered: entered.clone(),
                    alive: alive.clone(),
                    finished: finished.clone(),
                    release,
                }),
            ],
            ..RuntimeServices::default()
        },
    )
    .expect("runtime");
    client
        .send(Command::Connections(ConnectionCommand::Refresh))
        .expect("refresh");

    let finished_first = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::Connections(ConnectionEvent::KindProbed(report)) =
                client.events().recv().await.expect("event")
            {
                return report;
            }
        }
    })
    .await
    .expect("the fast link reports");
    assert_eq!(finished_first.kind, CatalogueKind::Telegram);
    assert_eq!(finished_first.phase, LinkPhase::Linked);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the stalled probe started");
    assert!(
        !finished.load(Ordering::SeqCst),
        "the stalled probe must still be held"
    );

    client
        .send(Command::Connections(ConnectionCommand::CancelRefresh))
        .expect("cancel");
    tokio::time::timeout(Duration::from_secs(5), async {
        while alive.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelling drops the stalled probe");
    // If the probe were still running, closing the gate would let it finish and
    // report a failure that would overwrite nothing — but it must not report at all.
    drop(release_tx);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
    let mut late_telegram = false;
    let mut late_whatsapp = false;
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, client.events().recv()).await {
        if let Event::Connections(ConnectionEvent::KindProbed(report)) = event {
            match report.kind {
                CatalogueKind::Telegram => late_telegram = true,
                CatalogueKind::WhatsApp => late_whatsapp = true,
                _ => {}
            }
        }
    }
    assert!(
        !late_telegram,
        "the link that already finished keeps its result"
    );
    assert!(!late_whatsapp, "a cancelled probe does not report");
    assert!(
        !finished.load(Ordering::SeqCst),
        "the stalled probe does not run on after cancellation"
    );
    client.shutdown_blocking().expect("shutdown");
}

const TELEGRAM_TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw5";

/// A saved credential the refresh can see. Pairing calls `forget`, and the
/// credential stays, which is the restart case the catalogue must not promote.
struct SavedQuietLink {
    sink: Mutex<Option<ChannelSink>>,
}

impl SavedQuietLink {
    fn new() -> Self {
        Self {
            sink: Mutex::new(None),
        }
    }

    fn start(&self, events: ChannelSink) -> Arc<dyn ChannelSession> {
        *self.sink.lock().expect("sink lock") = Some(events);
        Arc::new(IdleSession)
    }

    fn emit(&self, event: ChannelEvent) {
        let sink = self.sink.lock().expect("sink lock").clone();
        let sink = sink.expect("the link has started");
        sink(event);
    }
}

struct IdleSession;

#[async_trait::async_trait]
impl ChannelSession for IdleSession {
    async fn send_text(&self, _chat_jid: &str, _text: &str) -> Result<String, ChannelError> {
        Ok("idle".to_owned())
    }

    async fn logout(&self) {}

    async fn stop(&self) {}

    fn own_chat_jid(&self) -> Option<String> {
        None
    }
}

/// Uses the trait's default check, which does not contact the service.
struct DefaultCheckLink {
    inner: SavedQuietLink,
}

#[async_trait::async_trait]
impl MessagingChannel for DefaultCheckLink {
    async fn start(
        &self,
        _options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        Ok(self.inner.start(events))
    }

    fn has_saved_link(&self) -> bool {
        true
    }

    fn forget(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

/// Answers the check without contacting the service.
struct ExplicitQuietLink {
    inner: SavedQuietLink,
    checks: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl MessagingChannel for ExplicitQuietLink {
    async fn start(
        &self,
        _options: ChannelStartOptions,
        events: ChannelSink,
    ) -> Result<Arc<dyn ChannelSession>, ChannelError> {
        Ok(self.inner.start(events))
    }

    fn has_saved_link(&self) -> bool {
        true
    }

    fn forget(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn check(&self) -> Result<bool, ChannelError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}

fn shown_phase(state: &ConnectionsState, kind: CatalogueKind) -> LinkPhase {
    let catalogs: BTreeMap<String, CatalogState> = BTreeMap::new();
    let google = GoogleState::NotConnected;
    catalogue_from(&CatalogueSources {
        connections: &[] as &[ModelConnection],
        catalogs: &catalogs,
        whatsapp: &state.whatsapp,
        telegram: &state.telegram,
        google: &google,
        github: &sovra_core::github::GitHubState::NotConnected,
        services: &[] as &[McpService],
        probed: &state.probed,
    })
    .card(kind)
    .phase
}

async fn until_connection(
    events: &async_channel::Receiver<Event>,
    state: &mut ConnectionsState,
    pred: impl Fn(&ConnectionEvent) -> bool,
) {
    let found = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("runtime event");
            if let Event::Connections(connection) = event {
                let matched = pred(&connection);
                state.apply(&connection);
                if matched {
                    return;
                }
            }
        }
    })
    .await;
    found.expect("the link reached the expected status");
}

#[tokio::test]
async fn a_saved_link_check_that_never_reached_the_service_keeps_the_live_row() {
    let whatsapp = Arc::new(DefaultCheckLink {
        inner: SavedQuietLink::new(),
    });
    let checks = Arc::new(AtomicU64::new(0));
    let telegram = Arc::new(ExplicitQuietLink {
        inner: SavedQuietLink::new(),
        checks: Arc::clone(&checks),
    });
    let client = sovra_runtime::start_with_services(
        "catalogue-saved-link",
        AgentConfig::default(),
        RuntimeServices {
            messaging: MessagingChannels {
                whatsapp: Some(whatsapp.clone()),
                telegram: Some(telegram.clone()),
            },
            probes: Vec::new(),
            ..RuntimeServices::default()
        },
    )
    .expect("runtime");
    let events = client.events();
    let mut state = ConnectionsState::default();

    client
        .send(Command::Connections(
            ConnectionCommand::StartWhatsAppPairing,
        ))
        .expect("pair");
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .expect("load");
    until_connection(&events, &mut state, |event| {
        matches!(event, ConnectionEvent::Loaded(_))
    })
    .await;
    whatsapp.inner.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "+1 555-0142".into(),
            device_jid: "15550142:1@s.whatsapp.net".into(),
            companion_name: "Sovra".into(),
            linked_at: sovra_core::Timestamp(0),
        },
    });
    until_connection(&events, &mut state, |event| {
        matches!(
            event,
            ConnectionEvent::StatusChanged(WhatsAppStatus::Connected { online: true, .. })
        )
    })
    .await;
    whatsapp.inner.emit(ChannelEvent::Offline {
        reason: "the connection dropped".into(),
    });
    until_connection(&events, &mut state, |event| {
        matches!(
            event,
            ConnectionEvent::StatusChanged(WhatsAppStatus::Connected { online: false, .. })
        )
    })
    .await;

    client
        .send(Command::Connections(ConnectionCommand::ConnectTelegram {
            token: TELEGRAM_TOKEN.to_owned(),
        }))
        .expect("telegram");
    client
        .send(Command::Connections(ConnectionCommand::Load))
        .expect("load");
    until_connection(&events, &mut state, |event| {
        matches!(event, ConnectionEvent::Loaded(_))
    })
    .await;
    telegram.inner.emit(ChannelEvent::Connected {
        device: LinkedDevice {
            phone_display: "@kitchen_notes_bot".into(),
            device_jid: "123456789".into(),
            companion_name: "Sovra".into(),
            linked_at: sovra_core::Timestamp(0),
        },
    });
    until_connection(&events, &mut state, |event| {
        matches!(
            event,
            ConnectionEvent::TelegramStatusChanged(TelegramStatus::AwaitingOwner { .. })
        )
    })
    .await;
    assert_eq!(
        shown_phase(&state, CatalogueKind::WhatsApp),
        LinkPhase::Working
    );
    assert_eq!(
        shown_phase(&state, CatalogueKind::Telegram),
        LinkPhase::NeedsYou
    );

    client
        .send(Command::Connections(ConnectionCommand::Refresh))
        .expect("refresh");
    tokio::time::timeout(Duration::from_secs(5), async {
        while checks.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SavedLinkProbe called the channel check");

    let mut reported = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
        if let Event::Connections(connection) = event {
            if let ConnectionEvent::KindProbed(report) = &connection {
                reported.push(report.clone());
            }
            state.apply(&connection);
        }
    }
    assert!(
        reported.iter().all(|report| {
            report.phase != LinkPhase::Linked
                || !matches!(
                    report.kind,
                    CatalogueKind::WhatsApp | CatalogueKind::Telegram
                )
        }),
        "a check that did not reach the service must not report Linked"
    );
    assert!(
        reported.iter().all(|report| {
            !matches!(
                report.kind,
                CatalogueKind::WhatsApp | CatalogueKind::Telegram
            )
        }),
        "a check that did not reach the service must not change that row"
    );
    assert_eq!(
        shown_phase(&state, CatalogueKind::WhatsApp),
        LinkPhase::Working
    );
    assert_eq!(
        shown_phase(&state, CatalogueKind::Telegram),
        LinkPhase::NeedsYou
    );
    assert!(matches!(
        state.whatsapp,
        WhatsAppStatus::Connected { online: false, .. }
    ));
    assert!(matches!(
        state.telegram,
        TelegramStatus::AwaitingOwner { .. }
    ));
    client.shutdown_blocking().expect("shutdown");
}
