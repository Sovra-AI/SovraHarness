//! Headless UI tests: real views in a GPUI test window, runtime replaced by loopback channels.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{AgUiWidget, Command, Event, NavTab, RuntimeStatus, ScreenRoute};
use sovra_runtime::Client;
use sovra_ui::Shell;

fn confirm_schedule(
    cx: &mut TestAppContext,
    events: &async_channel::Sender<Event>,
    name: &str,
    cadence: &str,
    prompt: &str,
) {
    let mut book = sovra_core::ScheduleBook::default();
    let schedule = sovra_core::create_in_book(
        &mut book,
        name,
        cadence,
        prompt,
        None,
        sovra_core::Timestamp(1),
        0,
    )
    .expect("schedule test fixture is available");
    events
        .send_blocking(Event::Schedule(sovra_core::snapshot_event(&book)))
        .expect("schedule test fixture is available");
    events
        .send_blocking(Event::Schedule(sovra_core::ScheduleEvent::Created {
            schedule: Box::new(schedule),
        }))
        .expect("schedule test fixture is available");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn name_only_signin_waits_for_the_runtime_before_opening_the_workspace(cx: &mut TestAppContext) {
    use sovra_core::auth::{AuthCommand, AuthEvent, AuthProvider, AuthSession, LocalProfile};
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    for event in [
        AuthEvent::Loaded {
            session: None,
            has_accounts: true,
            google_ready: false,
        },
        AuthEvent::LocalProfiles(vec![LocalProfile {
            account_id: "local-sam".into(),
            full_name: "Sam Rivera".into(),
        }]),
    ] {
        events.send_blocking(Event::Auth(event)).unwrap();
    }
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find(("login-local-profile", 0usize))
                .label()
                .unwrap()
                .contains("Sam Rivera")
        );
        window.click(("login-local-profile", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::Login);
    })
    .unwrap();
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command|
        matches!(command, Command::Auth(AuthCommand::ContinueLocalProfile { account_id }) if account_id == "local-sam")
    ));
    events
        .send_blocking(Event::Auth(AuthEvent::SignedIn {
            session: AuthSession {
                account_id: "local-sam".into(),
                email: String::new(),
                full_name: "Sam Rivera".into(),
                provider: AuthProvider::Password,
                signed_in_at: sovra_core::Timestamp(1),
            },
            new_account: false,
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.workspace().route),
        ScreenRoute::MainWorkspace
    );
}

#[gpui_kit::test]
fn status_follows_runtime_events(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.status(), &RuntimeStatus::Starting);
    });

    events
        .send_blocking(Event::Ready {
            version: "1.2.3".into(),
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.status().label(), "Ready · v1.2.3");
    });

    events.send_blocking(Event::Stopped).unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.status(), &RuntimeStatus::Stopped);
    });
}

#[gpui_kit::test]
fn main_chat_login_schedules_and_agui_surfaces_work_end_to_end(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell_entity = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell_entity = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell_entity.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("sovra-main-workspace").visible());
        assert_eq!(
            shell.read(cx).workspace().messages,
            Vec::<sovra_core::ChatMessage>::new()
        );
        assert!(window.try_find("chat-hero-banner").is_none());
        assert!(window.try_find("agui-build-progress-card").is_none());
        assert!(window.try_find("sovra-inspector").is_none());

        // Capability discovery belongs with everyday work, before setup options.
        assert!(
            window.find("nav-skills").bounds().origin.y
                < window.find("sidebar-group-ai-setup").bounds().origin.y
        );
        window.click("nav-skills", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Skills);
        assert!(window.find("skill-search").visible());

        // Switch to Schedules tab -> empty invite, then create one and run it.
        window.click("nav-schedules", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Schedules);
        assert!(window.find("sovra-schedules-view").visible());
        assert!(window.find("schedules-empty-placeholder").visible());

        window.click("schedule-empty-create-btn", cx);
        window.render_frame(cx);
        assert!(window.find("schedule-create-form").visible());
        window.click("schedule-name-input", cx);
        window.input("Weekday briefing", cx);
        window.click("schedule-cadence-input", cx);
        window.input("every weekday at 9", cx);
        window.click("schedule-prompt-input", cx);
        window.input("Give me a short briefing of what's on today", cx);
        window.click("schedule-save-btn", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().schedules,
            Vec::<sovra_core::ScheduleItem>::new()
        );
    })
    .unwrap();
    confirm_schedule(
        cx,
        &events,
        "Weekday briefing",
        "every weekday at 9",
        "Give me a short briefing of what's on today",
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().schedules.len(), 1);
        assert!(window.find("schedules-timeline").visible());
        assert!(window.find(("schedule-lane", 0usize)).visible());
        assert!(window.find("schedules-now-marker").visible());

        window.click(("schedule-run", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);

        // The footer names its immediate action instead of promising an account menu.
        assert_eq!(
            window.find("sidebar-user-profile").label(),
            Some("Sign out")
        );
        window.click("sidebar-user-profile", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::Login);
        assert!(window.find("sovra-login-screen").visible());

        window.click("login-submit-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::Login);
    })
    .unwrap();

    // Verify the schedule run dispatched a real Command::RunPrompt to the runtime.
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::RunPrompt { .. }))
    );
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::Schedule(sovra_core::ScheduleCommand::Create { .. })
    )));
    let auth_commands: Vec<_> = sent
        .into_iter()
        .filter_map(|command| match command {
            Command::Auth(auth) => Some(auth),
            _ => None,
        })
        .collect();
    assert!(matches!(
        auth_commands.as_slice(),
        [
            sovra_core::AuthCommand::SignOut,
            sovra_core::AuthCommand::SignIn { .. }
        ]
    ));
    events
        .send_blocking(Event::Auth(sovra_core::AuthEvent::SignedIn {
            session: sovra_core::AuthSession {
                account_id: "test-account".into(),
                email: "test@example.com".into(),
                full_name: "Test User".into(),
                provider: sovra_core::auth::AuthProvider::Password,
                signed_in_at: sovra_core::Timestamp(1),
            },
            new_account: false,
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.workspace().route, ScreenRoute::MainWorkspace);
    });

    // Stream an AG-UI surface and text chunk back from the runtime.
    events
        .send_blocking(Event::AgUiSurface {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
            widget: AgUiWidget::SurfaceCard {
                surface_id: "personal-ai-summary".into(),
                title: "AG-UI Surface · personal-ai-summary".into(),
                summary: "Updated structured AG-UI surface (revision v1).".into(),
            },
        })
        .unwrap();
    events
        .send_blocking(Event::AgentChunk {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
            text: "Morning brief completed.".into(),
        })
        .unwrap();
    events
        .send_blocking(Event::AgentCompleted {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    shell.read_with(cx, |shell, _| {
        assert!(!shell.workspace().is_generating);
        let last = shell.workspace().messages.last().unwrap();
        assert!(last.text.contains("Morning brief completed."));
        assert_eq!(last.widgets.len(), 1);
    });
}

#[gpui_kit::test]
fn my_ais_switches_routing_modes_decision_backends_and_simulates_tree(cx: &mut TestAppContext) {
    let (client, commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell_entity = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell_entity = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell_entity.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-my-ais", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::MyAis);
        assert!(window.find("sovra-my-ais-view").visible());
        assert_eq!(shell.read(cx).workspace().specialists.len(), 23);
        // Engine internals stay folded away until asked for.
        assert!(window.try_find(("decision-backend-btn", 1usize)).is_none());
        assert!(window.try_find(("specialist-tier-btn", 0usize)).is_none());
        window.click("my-ais-advanced-toggle", cx);
        window.render_frame(cx);

        // Switch decision backend from Laya to OpenJev.
        window.click(("decision-backend-btn", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().routing.backend,
            sovra_core::DecisionBackend::OpenJev
        );

        // Simulate a prompt through the routing tree.
        window.click("simulate-route-btn", cx);
        window.render_frame(cx);
        let eval = shell
            .read(cx)
            .workspace()
            .last_route_evaluation
            .as_ref()
            .unwrap();
        assert_eq!(eval.specialist_id.as_deref(), Some("CodingAgent"));

        // Switch selection mode to Per-Agent Models.
        window.click(("routing-mode-btn", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().routing.mode,
            sovra_core::SelectionMode::PerAgentModel
        );
        assert!(window.find(("specialist-tier-btn", 0usize)).visible());
    })
    .unwrap();

    assert!(matches!(
        commands.recv_blocking().unwrap(),
        Command::UpdateRouting {
            backend: sovra_core::DecisionBackend::OpenJev,
            ..
        }
    ));
    assert!(matches!(
        commands.recv_blocking().unwrap(),
        Command::SimulateRoute { .. }
    ));
}

#[gpui_kit::test]
fn sidebar_alignment_and_native_surface_actions_work_in_both_themes(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1000.), px(760.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    events
        .send_blocking(Event::AgUiSurface {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
            widget: AgUiWidget::Components {
                surface_id: "report".into(),
                version: 3,
                components: vec![sovra_core::UiComponent::Card {
                    title: "Report".into(),
                    description: "From the agent".into(),
                    children: vec![
                        sovra_core::UiComponent::Table {
                            headers: vec!["Name".into(), "Status".into()],
                            rows: vec![vec!["Sovra".into(), "Ready".into()]],
                        },
                        sovra_core::UiComponent::Progress {
                            label: "Progress".into(),
                            value: 75,
                        },
                        sovra_core::UiComponent::Button {
                            label: "Continue".into(),
                            action_id: "next".into(),
                            disabled: false,
                        },
                    ],
                }],
            },
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        for mode in [
            gpui_kit::component::ThemeMode::Dark,
            gpui_kit::component::ThemeMode::Light,
        ] {
            gpui_kit::component::Theme::change(mode, Some(window), cx);
            window.render_frame(cx);
            let icon = window.find(("nav-icon", 0usize)).bounds();
            let text = window.find(("nav-label", 0usize)).bounds();
            let button = window.find("nav-chat").bounds();
            if sovra_ui::design::is_rtl(cx) {
                assert!(
                    button.right() - icon.right() < px(20.),
                    "nav content must start at inline-start padding"
                );
                assert!(
                    text.left() < icon.left(),
                    "label sits toward the start in RTL"
                );
            } else {
                assert!(
                    icon.left() - button.left() < px(20.),
                    "nav content must start at left padding"
                );
            }
            for index in 1..NavTab::ALL.len() {
                assert_eq!(
                    window.find(("nav-icon", index)).bounds().left(),
                    icon.left()
                );
                assert_eq!(
                    window.find(("nav-label", index)).bounds().left(),
                    text.left()
                );
            }
            assert!(window.find(("native-surface", 0usize)).visible());
            assert!(window.find("surface-action-0-0-2").visible());
        }
        window.click("surface-action-0-0-2", cx);
    })
    .unwrap();
    let dispatched = std::iter::from_fn(|| commands.try_recv().ok())
        .find(|command| matches!(command, Command::RunPrompt { .. }));
    let Some(Command::RunPrompt {
        session_id, prompt, ..
    }) = dispatched
    else {
        panic!("action must return to agent")
    };
    assert_eq!(session_id, sovra_core::WorkspaceState::new(0).session_id);
    assert!(
        prompt.contains("Continue"),
        "click must send the visible label, got {prompt}"
    );
    assert!(
        !prompt.contains("surface_ref"),
        "click must not dump JSON, got {prompt}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&prompt).is_err(),
        "prompt must be human text, not JSON: {prompt}"
    );
    shell.read_with(cx, |shell, _| {
        let user = shell
            .workspace()
            .messages
            .iter()
            .rev()
            .find(|message| message.role == sovra_core::MessageRole::User)
            .expect("button click stores a user turn");
        assert_eq!(user.text, prompt);
        assert_eq!(user.text, "Continue");
    });
}

#[gpui_kit::test]
fn composer_enter_sends_shift_enter_adds_line_and_stop_clears_queue(cx: &mut TestAppContext) {
    let (client, commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("composer-attach-btn").is_none());
        assert!(window.try_find("composer-voice-btn").is_none());
        shell.update(cx, |shell, cx| {
            shell.fill_composer_draft("Hello", window, cx);
        });
        window.render_frame(cx);
        window.click("chat-composer-input", cx);
        window.press("shift-enter", cx);
        window.input("world", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().messages[0].text, "Hello\nworld");
        assert!(shell.read(cx).workspace().is_generating);
        shell.update(cx, |shell, cx| {
            shell.fill_composer_draft("Follow up", window, cx);
        });
        window.render_frame(cx);
        window.click("composer-send-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().queued_prompts, ["Follow up"]);
        window.click("composer-stop-btn", cx);
        window.render_frame(cx);
        assert!(!shell.read(cx).workspace().is_generating);
        assert_eq!(
            shell.read(cx).workspace().queued_prompts,
            Vec::<String>::new()
        );
    })
    .unwrap();
    // Startup may request model/history data as well.
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(sent.iter().any(
        |command| matches!(command, Command::RunPrompt { prompt, .. } if prompt == "Hello\nworld")
    ));
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::InterruptSession { .. }))
    );
}

#[gpui_kit::test]
fn chat_history_rail_and_schedules_placeholder_work_end_to_end(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("chat-history-rail").visible());
        assert!(window.find("chat-filter-pills").visible());
        assert!(
            window.try_find(("chat-thread-row", 0usize)).is_none(),
            "untouched drafts are not conversation history"
        );

        // Send a prompt in the first thread and verify auto-titling in the history rail.
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Analyze Q4 Semiconductor Supply Chain", window, cx);
        });
        window.render_frame(cx);
        assert_eq!(
            window.find(("chat-thread-row", 0usize)).label(),
            Some("Open conversation Analyze Q4 Semiconductor Supply Chain")
        );

        // Start a new chat and send a second prompt.
        window.click("chat-new-btn", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().messages,
            Vec::<sovra_core::ChatMessage>::new()
        );
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Draft Rust async trait guidelines", window, cx);
        });
        window.render_frame(cx);

        // Pin the top thread and verify it stays accessible.
        window.click(("chat-thread-pin", 0usize), cx);
        window.render_frame(cx);
        assert!(shell.read(cx).workspace().active_thread().unwrap().pinned);

        // Switch back to the first thread by clicking row 1 in the rail.
        // The rail scrolls; bring row 1 fully into view like a person would.
        for _ in 0..20 {
            let rail = window.find("chat-history-rail").bounds();
            let row = window.find(("chat-thread-row", 1usize)).bounds();
            if row.bottom() <= rail.bottom() {
                break;
            }
            window.scroll(
                "chat-history-list",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
                cx,
            );
            window.render_frame(cx);
        }
        window.click(("chat-thread-row", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().messages[0].text,
            "Analyze Q4 Semiconductor Supply Chain"
        );

        // Collapse and re-expand the chat history rail.
        window.click("chat-history-collapse-btn", cx);
        window.render_frame(cx);
        assert!(window.find("chat-history-rail-collapsed").visible());
        window.click("chat-history-expand-btn", cx);
        window.render_frame(cx);
        assert!(window.find("chat-history-rail").visible());

        // Switch to the Scheduled filter in the history rail -> shows the Schedules placeholder CTA.
        window.click("chat-filter-scheduled", cx);
        window.render_frame(cx);
        assert!(window.find("chat-history-scheduled-placeholder").visible());

        // Click "Open Schedules" CTA from the Scheduled history placeholder -> navigates to Schedules tab.
        window.click("chat-scheduled-open-schedules-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Schedules);
        assert!(window.find("sovra-schedules-view").visible());
        assert!(window.find("schedules-empty-placeholder").visible());

        // Click "Create schedule" from the empty placeholder -> opens the schedule creation form and creates a schedule.
        window.click("schedule-empty-create-btn", cx);
        window.render_frame(cx);
        assert!(window.find("schedule-create-form").visible());
        window.click("schedule-name-input", cx);
        window.input("Daily Tech Digest", cx);
        window.click("schedule-prompt-input", cx);
        window.input("Summarize top semiconductor and AI news.", cx);
        window.click("schedule-save-btn", cx);
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().schedules,
            Vec::<sovra_core::ScheduleItem>::new()
        );
    })
    .unwrap();
    confirm_schedule(
        cx,
        &events,
        "Daily Tech Digest",
        "",
        "Summarize top semiconductor and AI news.",
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().schedules.len(), 1);
        assert_eq!(
            shell.read(cx).workspace().schedules[0].name,
            "Daily Tech Digest"
        );
    })
    .unwrap();

    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        sent.iter()
            .any(|cmd| matches!(cmd, Command::SaveChatThread { .. }))
    );
}

#[gpui_kit::test]
fn interactive_system_tour_walks_through_every_page_and_state_aware_chat_setup(
    cx: &mut TestAppContext,
) {
    use sovra_core::models::{ModelConnection, ModelEvent, ModelPreferences, ProviderKind};

    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();

    // Simulate user having already set an OpenRouter key, but no model selected yet.
    let mut router = ModelConnection::new("router", ProviderKind::OpenRouter);
    router.has_credential = true;
    router.key_preview = "sk-or-••••1234".into();
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![router],
            preferences: ModelPreferences::default(),
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // The empty chat's one card shows Step 2 (Pick your AI), NOT Step 1 (Connect),
        // and the top strip stays hidden so there is a single thing to press.
        assert_eq!(
            window.find("chat-first-step-action").label(),
            Some("Pick your AI")
        );
        assert!(window.try_find("sovra-setup-coach-strip").is_none());

        // Sending a message in Chat when key is set must ONLY show Step 2 model selection in chat.
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Hi there!", window, cx);
        });
        window.render_frame(cx);
        // Once the chat has a message, the top strip keeps the next step in view.
        assert!(window.find("sovra-setup-coach-strip").visible());
        assert!(window.find("setup-strip-pick-model").visible());
        assert!(window.try_find("setup-strip-connect").is_none());
        assert!(window.find("surface-setup-recommended-models").visible());
        assert!(window.try_find("surface-setup-openrouter-key").is_none());

        // The chat card keeps one job (pick an AI); the tour starts from the setup bar or sidebar.
        assert!(window.try_find("surface-setup-start-tour").is_none());
        assert!(window.find("sidebar-tour-btn").visible());
        assert!(
            window
                .find(("nav-context-badge", NavTab::Models as usize))
                .visible(),
            "Models tab must show a Setup context badge when no LLM is active yet"
        );
        window.click("setup-strip-tour-btn", cx);
        window.render_frame(cx);
        assert!(window.find("sovra-tour-coach-bar").visible());
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
        assert!(
            window
                .find(("nav-spotlight-badge", NavTab::Chat as usize))
                .visible()
        );

        // Every step opens its page, marks it in the menu and says where it is.
        for step in sovra_core::TourStep::ALL {
            assert_eq!(shell.read(cx).workspace().tour_step, Some(step));
            assert_eq!(shell.read(cx).workspace().active_tab, step.target_tab());
            assert_eq!(window.find("tour-step-title").label(), Some(step.title()));
            // The menu marks the page too (it may sit below the fold of a short menu).
            assert!(
                window
                    .try_find(("nav-spotlight-badge", step.target_tab() as usize))
                    .is_some(),
                "{step:?} marks its page in the menu"
            );
            assert!(window.find("tour-next-btn").visible());
            window.click("tour-next-btn", cx);
            window.render_frame(cx);
        }

        // Finishing returns to Chat and closes the tour card.
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
        assert!(window.try_find("sovra-tour-coach-bar").is_none());
        assert!(shell.read(cx).workspace().onboarding_completed);
    })
    .unwrap();

    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(sent.iter().any(|cmd| matches!(
        cmd,
        Command::Models(sovra_core::models::ModelCommand::SetOnboardingCompleted {
            completed: true
        })
    )));
}

fn sign_in_newcomer(cx: &mut TestAppContext, events: &async_channel::Sender<Event>) {
    use sovra_core::models::{ModelEvent, ModelPreferences};
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: Vec::new(),
            preferences: ModelPreferences::default(),
        }))
        .expect("settings delivered");
    events
        .send_blocking(Event::Auth(sovra_core::AuthEvent::SignedIn {
            session: sovra_core::AuthSession {
                account_id: "new-account".into(),
                email: "new@example.com".into(),
                full_name: "New Person".into(),
                provider: sovra_core::auth::AuthProvider::Password,
                signed_in_at: sovra_core::Timestamp(1),
            },
            new_account: true,
        }))
        .expect("sign-in delivered");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_newcomer_is_walked_through_sovra_after_first_signing_in(cx: &mut TestAppContext) {
    use sovra_core::TourStep;

    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    sign_in_newcomer(cx, &events);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().tour_step,
            Some(TourStep::Welcome)
        );
        assert!(window.find("sovra-tour-coach-bar").visible());
        assert_eq!(
            window.find("tour-step-count").label(),
            Some(format!("Guided tour · Step 1 of {}", TourStep::COUNT).as_str())
        );
        assert!(window.find("tour-step-explanation").visible());
        assert!(window.try_find("tour-prev-btn").is_none());

        // Step 2 offers both ways to give Sovra an AI, each with its own buttons.
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Models);
        assert_eq!(
            window.find("tour-step-title").label(),
            Some("Give Sovra a brain")
        );
        for id in [
            "tour-choice-openrouter",
            "tour-open-openrouter",
            "tour-openrouter-key",
            "tour-choice-ollama",
            "tour-get-ollama",
            "tour-setup-ollama",
        ] {
            assert!(window.find(id).visible(), "{id} is shown");
        }

        // Paste my key opens the online AI card on Connections; the tour stays open.
        // The choices fit inside the card at a normal window size, without scrolling.
        let body = window.find("tour-card-body").bounds();
        for id in ["tour-openrouter-key", "tour-setup-ollama"] {
            let button = window.find(id).bounds();
            assert!(
                button.bottom() <= body.bottom(),
                "{id} is cut off by the card ({button:?} in {body:?})"
            );
        }
        window.click("tour-openrouter-key", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Connections);
        assert_eq!(
            shell.read(cx).workspace().tour_step,
            Some(TourStep::ConnectAi)
        );
        assert!(window.find("sovra-tour-coach-bar").visible());

        // Set up Ollama opens the on-this-computer setup in Models.
        window.click("tour-setup-ollama", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Models);

        window.click("tour-skip-btn", cx);
        window.render_frame(cx);
        assert!(window.try_find("sovra-tour-coach-bar").is_none());
    })
    .unwrap();

    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(sent.iter().any(|cmd| matches!(
        cmd,
        Command::Models(sovra_core::models::ModelCommand::SetOnboardingCompleted {
            completed: true
        })
    )));

    // Settings reload on every visit to Models; a tour already ended stays closed.
    sign_in_newcomer(cx, &events);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().tour_step, None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_connected_ai_is_confirmed_instead_of_asking_again(cx: &mut TestAppContext) {
    use sovra_core::models::{ModelConnection, ModelEvent, ModelPreferences, ProviderKind};

    let (client, _commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    let mut router = ModelConnection::new("router", ProviderKind::OpenRouter);
    router.has_credential = true;
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![router],
            preferences: ModelPreferences {
                active: Some(sovra_core::models::ModelKey {
                    connection_id: "router".into(),
                    model_id: "openrouter/free".into(),
                }),
                ..ModelPreferences::default()
            },
        }))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, Shell::start_system_tour);
        window.render_frame(cx);
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
        assert!(window.find("tour-ai-ready").visible());
        assert!(window.try_find("tour-ai-choices").is_none());
    })
    .unwrap();
}

fn assert_tour_fits(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    scale: u16,
    mode: gpui_kit::component::ThemeMode,
) {
    use sovra_core::models::InterfaceScale;

    let (client, _commands, events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        gpui_kit::component::Theme::change(mode, None, cx);
        sovra_ui::design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
    });
    let mut shell = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.expect("window opened");
    sign_in_newcomer(cx, &events);
    cx.update_window(window.into(), |_, window, cx| {
        let viewport = window.viewport_size();
        for step in sovra_core::TourStep::ALL {
            window.render_frame(cx);
            assert_eq!(shell.read(cx).workspace().tour_step, Some(step));
            let card = window.find("sovra-tour-coach-bar").bounds();
            assert!(
                card.size.height <= viewport.height * 0.6,
                "{step:?} at {width}x{height} {scale}%: the card leaves room for the page ({card:?})"
            );
            for id in ["tour-step-title", "tour-skip-btn", "tour-next-btn"] {
                let found = window.find(id);
                let bounds = found.bounds();
                assert!(
                    found.visible()
                        && bounds.right() <= viewport.width
                        && bounds.bottom() <= viewport.height
                        && bounds.left() >= px(0.)
                        && bounds.top() >= px(0.),
                    "{step:?} at {width}x{height} {scale}%: {id} is off screen ({bounds:?})"
                );
            }
            window.click("tour-next-btn", cx);
        }
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().tour_step, None);
    })
    .expect("window exists");
}

#[gpui_kit::test]
fn every_tour_step_fits_the_smallest_window(cx: &mut TestAppContext) {
    assert_tour_fits(cx, 640., 420., 100, gpui_kit::component::ThemeMode::Light);
}

#[gpui_kit::test]
fn every_tour_step_fits_the_smallest_window_with_double_text(cx: &mut TestAppContext) {
    assert_tour_fits(cx, 640., 420., 200, gpui_kit::component::ThemeMode::Dark);
}

#[gpui_kit::test]
fn every_tour_step_fits_a_wide_monitor(cx: &mut TestAppContext) {
    assert_tour_fits(cx, 2560., 1440., 100, gpui_kit::component::ThemeMode::Dark);
}

#[gpui_kit::test]
fn unified_single_sidebar_and_ai_title_generation_work_end_to_end(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // Verify there is ONE unified sidebar containing both navigation and chat history,
        // with "+ New Chat" at the top and the chat column directly adjacent to it.
        let sidebar_bounds = window.find("sovra-sidebar").bounds();
        let history_bounds = window.find("chat-history-rail").bounds();
        let new_chat_bounds = window.find("chat-new-btn").bounds();
        let nav_chat_bounds = window.find("nav-chat").bounds();
        let nav_schedules_bounds = window.find("nav-schedules").bounds();
        let group_header_bounds = window.find("sidebar-group-ai-setup").bounds();
        let nav_models_bounds = window.find("nav-models").bounds();

        assert!(
            history_bounds.left() >= sidebar_bounds.left()
                && history_bounds.right() <= sidebar_bounds.right(),
            "chat history must live inside the single unified sidebar"
        );
        assert!(
            new_chat_bounds.top() < nav_chat_bounds.top(),
            "+ New Chat must be at the top of the sidebar above navigation"
        );
        assert!(
            nav_schedules_bounds.bottom() <= group_header_bounds.top()
                && group_header_bounds.bottom() <= nav_models_bounds.top(),
            "Your AI & Setup section header must separate everyday workspace tabs from AI setup tabs"
        );
        assert!(
            window
                .try_find(("nav-context-badge", NavTab::Schedules as usize))
                .is_none(),
            "Schedules tab has no count badge when the list is empty"
        );
        window.click("nav-schedules", cx);
        window.render_frame(cx);
        window.click("schedule-empty-create-btn", cx);
        window.render_frame(cx);
        window.click("schedule-name-input", cx);
        window.input("Weekday briefing", cx);
        window.click("schedule-prompt-input", cx);
        window.input("Give me a short briefing of what's on today", cx);
        window.click("schedule-save-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().schedules, Vec::<sovra_core::ScheduleItem>::new());
    }).unwrap();
    confirm_schedule(
        cx,
        &events,
        "Weekday briefing",
        "",
        "Give me a short briefing of what's on today",
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-chat", cx);
        window.render_frame(cx);
        assert!(
            window
                .find(("nav-context-badge", NavTab::Schedules as usize))
                .visible(),
            "Schedules tab must display live count badge when schedules exist"
        );
        let sidebar_bounds = window.find("sovra-sidebar").bounds();
        let chat_main_bounds = window.find("sovra-chat-main").bounds();
        if sovra_ui::design::is_rtl(cx) {
            assert!(
                (sidebar_bounds.left() - chat_main_bounds.right()).abs() < px(4.),
                "chat column must sit directly beside the single sidebar with no second sidebar in between"
            );
        } else {
            assert!(
                (chat_main_bounds.left() - sidebar_bounds.right()).abs() < px(4.),
                "chat column must sit directly beside the single sidebar with no second sidebar in between"
            );
        }

        // Send first user prompt
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text(
                "Can you help me design a zero-copy parser in Rust for HTTP/3 frames?",
                window,
                cx,
            );
        });
        window.render_frame(cx);
    })
    .unwrap();

    // Simulate assistant response completing
    events
        .send_blocking(Event::AgentChunk {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
            text: "Here is how to build a zero-copy HTTP/3 frame parser in Rust.".into(),
        })
        .unwrap();
    events
        .send_blocking(Event::AgentCompleted {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    // Verify the shell automatically requested an AI-generated title from the model
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        sent.iter().any(|cmd| matches!(
            cmd,
            Command::GenerateChatTitle { session_id, prompt, reply }
                if session_id == &sovra_core::WorkspaceState::new(0).session_id
                && prompt.contains("zero-copy parser")
                && reply.contains("HTTP/3 frame parser")
        )),
        "completing the first exchange must dispatch Command::GenerateChatTitle"
    );

    // Deliver the model-generated title and verify it updates the sidebar history row
    events
        .send_blocking(Event::ChatTitleGenerated {
            session_id: sovra_core::WorkspaceState::new(0).session_id,
            title: "\"Rust Zero-Copy HTTP/3 Parser\"".into(),
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            shell.read(cx).workspace().active_thread().unwrap().title,
            "Rust Zero-Copy HTTP/3 Parser"
        );
        assert_eq!(
            window.find(("chat-thread-row", 0usize)).label(),
            Some("Open conversation Rust Zero-Copy HTTP/3 Parser")
        );
        // Also verify history stays visible in the unified sidebar when switching to another tab (e.g. Models)
        window.click("nav-models", cx);
        window.render_frame(cx);
        assert!(window.find("chat-history-rail").visible());
        assert_eq!(
            window.find(("chat-thread-row", 0usize)).label(),
            Some("Open conversation Rust Zero-Copy HTTP/3 Parser")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn file_setup_keeps_draft_and_returns_to_send_real_attachments(cx: &mut TestAppContext) {
    file_setup_flow(cx, false, sovra_core::models::Appearance::Dark);
}

#[gpui_kit::test]
fn file_setup_is_reachable_at_minimum_size_in_light_mode(cx: &mut TestAppContext) {
    file_setup_flow(cx, true, sovra_core::models::Appearance::Light);
}

#[gpui_kit::test]
fn file_setup_is_reachable_at_minimum_size_in_dark_mode(cx: &mut TestAppContext) {
    file_setup_flow(cx, true, sovra_core::models::Appearance::Dark);
}

fn file_setup_flow(
    cx: &mut TestAppContext,
    compact: bool,
    appearance: sovra_core::models::Appearance,
) {
    use sovra_core::files::{Attachment, FileCommand, FilePrepared, ModelPurpose};
    use sovra_core::models::*;
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut entity = None;
    let dimensions = if compact {
        size(px(640.), px(420.))
    } else {
        size(px(1280.), px(820.))
    };
    let window = cx.open_window(dimensions, |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        entity = Some(shell.clone());
        Root::new(shell, window, cx)
    });
    let shell = entity.expect("file setup test fixture succeeds");
    let mut connection = ModelConnection::new("pictures", ProviderKind::OpenRouter);
    connection.has_credential = true;
    let model = ModelInfo {
        id: "reader".into(),
        name: "Picture Reader".into(),
        input_modalities: vec!["text".into(), "image".into()],
        output_modalities: vec!["text".into()],
        ..ModelInfo::default()
    };
    let key = model.key("pictures");
    let mut preferences = ModelPreferences {
        active: Some(key.clone()),
        appearance,
        ..ModelPreferences::default()
    };
    events
        .send_blocking(Event::Models(ModelEvent::Loaded {
            connections: vec![connection],
            preferences: preferences.clone(),
        }))
        .expect("file setup test fixture succeeds");
    events
        .send_blocking(Event::Models(ModelEvent::CatalogLoaded {
            connection_id: "pictures".into(),
            snapshot: CatalogSnapshot {
                models: vec![model],
                ..CatalogSnapshot::default()
            },
            cached: false,
        }))
        .expect("file setup test fixture succeeds");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.fill_composer_draft("What does this sign say?", window, cx);
        });
        window.render_frame(cx);
        window.click("composer-add-files", cx);
    })
    .expect("file setup test fixture succeeds");
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec!["sign.png".into()])
    });
    cx.run_until_parked();
    let request = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| {
            if let Command::Files(FileCommand::Prepare { session_id, id, .. }) = command {
                Some((session_id, id))
            } else {
                None
            }
        })
        .expect("file setup test fixture succeeds");
    let attachment = Attachment {
        id: request.1.clone(),
        name: "sign.png".into(),
        mime_type: "image/png".into(),
        size_bytes: 42,
        purpose: ModelPurpose::Images,
        sections: 1,
        note: None,
        thumbnail: Vec::new(),
    };
    events
        .send_blocking(Event::FilePrepared(FilePrepared {
            session_id: request.0,
            id: request.1.clone(),
            result: Ok(attachment.clone()),
        }))
        .expect("file setup test fixture succeeds");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("attachment-model-guidance").visible());
        let guidance = window.find("attachment-choose-model").bounds();
        assert!(guidance.top() >= px(0.) && guidance.bottom() <= dimensions.height);
        assert_eq!(
            shell.read(cx).workspace().messages,
            [] as [sovra_core::ChatMessage; 0]
        );
        window.click("attachment-choose-model", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Models);
        let choose = window.find(("model-use-row", 0usize)).bounds();
        assert!(choose.top() >= px(0.) && choose.bottom() <= dimensions.height);
        window.click(("model-use-row", 0usize), cx);
    })
    .expect("file setup test fixture succeeds");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Models(ModelCommand::AssignPurpose {
                purpose: ModelPurpose::Images,
                key: Some(_)
            })
        ))
    );
    preferences.purposes.insert(ModelPurpose::Images, key);
    events
        .send_blocking(Event::Models(ModelEvent::PreferencesChanged(preferences)))
        .expect("file setup test fixture succeeds");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
        assert!(window.try_find("attachment-model-guidance").is_none());
        window.click("composer-send-btn", cx);
        window.render_frame(cx);
        let message = &shell.read(cx).workspace().messages[0];
        assert_eq!(message.text, "What does this sign say?");
        assert_eq!(message.attachments, vec![attachment]);
    })
    .expect("file setup test fixture succeeds");
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::RunWithFiles { prompt, attachment_ids, .. } if prompt == "What does this sign say?" && attachment_ids == vec![request.1.clone()])));
}

#[gpui_kit::test]
fn removed_upload_cannot_reappear_after_preparation_finishes(cx: &mut TestAppContext) {
    use sovra_core::files::{FileCommand, FileError, FilePrepared};
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut entity = None;
    let window = cx.open_window(size(px(640.), px(420.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        entity = Some(shell.clone());
        Root::new(shell, window, cx)
    });
    let shell = entity.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.add_files(vec!["notes.txt".into()], cx);
        });
        window.render_frame(cx);
        assert!(window.find("attachment-draft-strip").visible());
        window.click(("remove-attachment", 0usize), cx);
    })
    .unwrap();
    let (session_id, id) = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| {
            if let Command::Files(FileCommand::Prepare { session_id, id, .. }) = command {
                Some((session_id, id))
            } else {
                None
            }
        })
        .unwrap();
    events
        .send_blocking(Event::FilePrepared(FilePrepared {
            session_id,
            id,
            result: Err(FileError("late error".into())),
        }))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("attachment-draft-strip").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn reopening_a_saved_chat_sends_bounded_role_preserving_context(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    let mut thread = sovra_core::ChatThread::new("saved-chat", sovra_core::Timestamp(1));
    thread.title = "Garden app".into();
    let mut reply = sovra_core::ChatMessage::assistant(
        "old-assistant",
        "We chose Rust",
        "",
        vec![sovra_core::default_personal_ai_build_widget()],
    );
    reply.thinking = "private scratchpad".into();
    thread.messages = vec![
        sovra_core::ChatMessage::user("old-user", "I am making a garden app", ""),
        reply,
    ];
    events
        .send_blocking(Event::ChatHistoryLoaded {
            threads: vec![thread],
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("chat-history-section-Older").visible());
        assert!(window.try_find("chat-history-section-Today").is_none());
        shell.update(cx, |shell, cx| {
            shell.select_thread("saved-chat", cx);
            shell.send_prompt_text("What did we choose?", window, cx);
        });
    })
    .unwrap();
    let command = std::iter::from_fn(|| commands.try_recv().ok()).find(|command| matches!(command, Command::RunPrompt { session_id, .. } if session_id == "saved-chat")).unwrap();
    let Command::RunPrompt {
        context: Some(context),
        prompt,
        ..
    } = command
    else {
        panic!("restored chat needs its visible history")
    };
    assert_eq!(prompt, "What did we choose?");
    assert_eq!(context.messages().len(), 2);
    assert_eq!(context.messages()[0].role, sovra_core::MessageRole::User);
    assert_eq!(context.messages()[0].text, "I am making a garden app");
    assert_eq!(
        context.messages()[1].role,
        sovra_core::MessageRole::Assistant
    );
    assert_eq!(context.messages()[1].text, "We chose Rust");
}

#[gpui_kit::test]
fn whatsapp_personal_qr_pairing_and_message_bridge_work_end_to_end(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    use sovra_core::connections::{
        BridgedMessage, ConnectionCommand, ConnectionEvent, LinkedDevice, WhatsAppQrTicket,
        WhatsAppStatus,
    };

    fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
        for _ in 0..40 {
            window.render_frame(cx);
            let item = window.find(id);
            if item.visible() {
                return;
            }
            let delta = if item.bounds().top() < window.find("connections-scroll").bounds().top() {
                80.
            } else {
                -80.
            };
            window.scroll(
                "connections-scroll",
                ScrollDelta::Pixels(point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("{id} must be reachable in Connections");
    }

    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-connections", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Connections);
        assert!(window.find("connections-page").visible());
        reveal(window, "connection-action-whatsapp", cx);
        window.click("connection-action-whatsapp", cx);
        window.render_frame(cx);
        assert!(window.find("settings-connections").visible());
        assert!(window.find("whatsapp-connect-btn").visible());

        // Click Connect WhatsApp -> dispatches StartWhatsAppPairing
        window.click("whatsapp-connect-btn", cx);
    })
    .unwrap();

    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|cmd| matches!(
            cmd,
            Command::Connections(ConnectionCommand::StartWhatsAppPairing)
        )),
        "clicking Connect WhatsApp must dispatch StartWhatsAppPairing"
    );

    // WhatsApp issues a code -> the QR window opens and paints the grid
    let ticket = WhatsAppQrTicket {
        qr_payload: "2@ref,noise,identity,adv,7".into(),
        qr_matrix: (0..29)
            .map(|row| (0..29).map(|col| (row + col) % 3 == 0).collect())
            .collect(),
        expires_in_secs: 60,
        pairing_code: None,
    };
    events
        .send_blocking(Event::Connections(ConnectionEvent::StatusChanged(
            WhatsAppStatus::AwaitingQrScan { ticket },
        )))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("whatsapp-qr-modal").visible());
        assert!(window.find("whatsapp-qr-matrix").visible());
        assert!(
            window.try_find("whatsapp-confirm-scan-btn").is_none(),
            "linking completes when the phone scans; there is nothing to confirm by hand"
        );
        reveal(window, "whatsapp-refresh-qr-btn", cx);
        window.click("whatsapp-refresh-qr-btn", cx);
    })
    .unwrap();

    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|cmd| matches!(
            cmd,
            Command::Connections(ConnectionCommand::RefreshWhatsAppQr)
        )),
        "asking for a new code must dispatch RefreshWhatsAppQr"
    );

    // The phone scans, the link completes, and a message arrives with Sovra's answer
    events
        .send_blocking(Event::Connections(ConnectionEvent::StatusChanged(
            WhatsAppStatus::Connected {
                device: LinkedDevice {
                    phone_display: "+1 555-0142".into(),
                    device_jid: "15550142:1@s.whatsapp.net".into(),
                    companion_name: "Sovra".into(),
                    linked_at: sovra_core::Timestamp(10_000),
                },
                online: true,
            },
        )))
        .unwrap();
    events
        .send_blocking(Event::Connections(ConnectionEvent::MessageBridged(
            BridgedMessage {
                id: "wa-1".into(),
                channel: sovra_core::connections::ChannelKind::WhatsApp,
                chat_jid: "15550142@s.whatsapp.net".into(),
                sender_name: "You (WhatsApp)".into(),
                sender_jid: "15550142@s.whatsapp.net".into(),
                text: "Summarize my afternoon schedule".into(),
                reply_text: None,
                received_at: sovra_core::Timestamp(11_000),
            },
        )))
        .unwrap();
    events
        .send_blocking(Event::Connections(ConnectionEvent::ReplySent {
            message_id: "wa-1".into(),
            reply: "Two meetings and a dentist appointment.".into(),
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("whatsapp-qr-modal").is_none());
        reveal(window, "whatsapp-connected-details", cx);
        assert!(window.find("whatsapp-connected-details").visible());
        reveal(window, "whatsapp-bridged-messages", cx);
        assert!(window.find("whatsapp-bridged-messages").visible());
        assert!(
            window
                .find(("nav-context-badge", NavTab::Connections as usize))
                .visible(),
            "Connections tab in the sidebar shows a mark when WhatsApp is linked"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn telegram_bot_connects_with_a_token_and_shows_the_link_code(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    use sovra_core::connections::{ConnectionCommand, ConnectionEvent, TelegramStatus};

    fn reveal(window: &mut gpui_kit::Window, id: &'static str, cx: &mut gpui_kit::App) {
        for _ in 0..40 {
            window.render_frame(cx);
            let item = window.find(id);
            if item.visible() {
                return;
            }
            let delta = if item.bounds().top() < window.find("connections-scroll").bounds().top() {
                80.
            } else {
                -80.
            };
            window.scroll(
                "connections-scroll",
                ScrollDelta::Pixels(point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("{id} must be reachable in Connections");
    }

    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(2600.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("nav-connections", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Connections);
        reveal(window, "connection-action-telegram", cx);
        window.click("connection-action-telegram", cx);
        window.render_frame(cx);
        reveal(window, "settings-telegram", cx);
        reveal(window, "telegram-connect-btn", cx);
        // Nothing typed yet: Connect still dispatches, and the runtime answers with a notice.
        window.click("telegram-connect-btn", cx);
    })
    .unwrap();
    let dispatched: Vec<Command> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        dispatched.iter().any(|cmd| matches!(
            cmd,
            Command::Connections(ConnectionCommand::ConnectTelegram { .. })
        )),
        "Connect Telegram must dispatch ConnectTelegram; saw {dispatched:?}"
    );

    events
        .send_blocking(Event::Connections(ConnectionEvent::TelegramStatusChanged(
            TelegramStatus::AwaitingOwner {
                bot_username: "sovra_test_bot".into(),
                code: "482913".into(),
            },
        )))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        reveal(window, "telegram-link-code", cx);
        assert!(window.find("telegram-link-code").visible());
        assert!(
            window.try_find("telegram-token-input").is_none(),
            "the token field goes away once the bot is live"
        );
    })
    .unwrap();

    events
        .send_blocking(Event::Connections(ConnectionEvent::TelegramStatusChanged(
            TelegramStatus::Connected {
                bot_username: "sovra_test_bot".into(),
                owner_chat_id: "777".into(),
                owner_name: "Sam".into(),
                linked_at: sovra_core::Timestamp(10_000),
                online: true,
            },
        )))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        reveal(window, "telegram-connected-details", cx);
        assert!(window.find("telegram-connected-details").visible());
        reveal(window, "telegram-disconnect-btn", cx);
        window.click("telegram-disconnect-btn", cx);
    })
    .unwrap();
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|cmd| matches!(
            cmd,
            Command::Connections(ConnectionCommand::DisconnectTelegram)
        )),
        "Disconnect Telegram must dispatch DisconnectTelegram"
    );
}

#[gpui_kit::test]
fn quitting_requests_runtime_shutdown(cx: &mut TestAppContext) {
    let (client, commands, _events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        cx.shutdown();
    });
    assert_eq!(commands.try_recv().unwrap(), Command::Shutdown);
}

/// Under server-side decorations (i3, X11 without a compositor) the window
/// manager already frames the window, so the shell must not stack an empty
/// in-app bar beneath it.
#[cfg(target_os = "linux")]
#[gpui_kit::test]
fn no_in_app_title_bar_under_window_manager_decorations(cx: &mut TestAppContext) {
    let (client, _commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(view, window, cx)
    });

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("sovra-main-workspace").visible());
        assert!(window.try_find("title-bar").is_none());
        assert!(window.try_find("title-bar-page").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn new_chat_dates_are_real_before_storage_and_navigation_keeps_them(cx: &mut TestAppContext) {
    let (client, commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let earliest = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut view = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(42, window, cx));
        view = Some(shell.clone());
        Root::new(shell, window, cx)
    });
    let shell = view.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Fictional date fixture.", window, cx);
        });
        let first = shell.read(cx).workspace().active_thread().unwrap().clone();
        window.render_frame(cx);
        assert!(window.find("chat-history-section-Today").visible());
        assert!(window.try_find("chat-history-section-Older").is_none());
        assert!(
            u128::try_from(first.created_at.millis()).unwrap() >= earliest,
            "a live chat must not display the message counter as a date"
        );
        assert!(first.updated_at >= first.created_at);
        assert_eq!(
            sovra_core::recency_bucket(first.updated_at, first.updated_at),
            sovra_core::RecencyBucket::Today
        );
        shell.update(cx, |shell, cx| shell.start_new_chat(window, cx));
        assert_eq!(
            shell
                .read(cx)
                .workspace()
                .threads
                .iter()
                .find(|t| t.id == first.id)
                .unwrap()
                .updated_at,
            first.updated_at,
            "navigation is not a new conversation event"
        );
        let saved = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|command| {
                if let Command::SaveChatThread { thread } = command {
                    Some(thread)
                } else {
                    None
                }
            })
            .filter(|thread| thread.id == first.id)
            .collect::<Vec<_>>();
        assert_ne!(saved, Vec::new());
        assert!(
            saved
                .iter()
                .all(|thread| thread.created_at == first.created_at
                    && thread.updated_at == first.updated_at)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn background_completion_saves_and_dispatches_the_originating_chat(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("First context only", window, cx);
        });
    })
    .unwrap();
    let first = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    events
        .send_blocking(Event::AgentChunk {
            session_id: first.clone(),
            text: "First half, ".into(),
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    while commands.try_recv().is_ok() {}
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("First follow-up", window, cx);
            shell.start_new_chat(window, cx);
            shell.send_prompt_text("Second context only", window, cx);
        });
    })
    .unwrap();
    let second = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    let before = std::iter::from_fn(|| commands.try_recv().ok()).collect::<Vec<_>>();
    assert!(before.iter().any(|command| matches!(command, Command::SaveChatThread { thread }
        if thread.id == first && thread.messages.iter().any(|message| message.text == "First half, "))));
    for event in [
        Event::AgentChunk {
            session_id: first.clone(),
            text: "second half.".into(),
        },
        Event::AgentCompleted {
            session_id: first.clone(),
        },
    ] {
        events.send_blocking(event).unwrap();
    }
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    let emitted = std::iter::from_fn(|| commands.try_recv().ok()).collect::<Vec<_>>();
    assert!(emitted.iter().any(|command| matches!(command, Command::SaveChatThread { thread }
        if thread.id == first && thread.messages.iter().any(|message| message.text == "First half, second half."))));
    let command = emitted.iter().find(|command| matches!(command, Command::RunPrompt { prompt, .. } if prompt == "First follow-up")).unwrap();
    let Command::RunPrompt {
        session_id,
        context: Some(context),
        ..
    } = command
    else {
        unreachable!()
    };
    assert_eq!(session_id, &first);
    assert!(
        context
            .messages()
            .iter()
            .any(|message| message.text == "First half, second half.")
    );
    assert!(
        context
            .messages()
            .iter()
            .all(|message| !message.text.contains("Second context"))
    );
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.workspace().session_id, second);
        assert!(shell.workspace().is_generating);
        assert_eq!(shell.workspace().messages.len(), 1);
    });
}

#[gpui_kit::test]
fn late_source_receipt_saves_the_originating_transcript_without_changing_selection(
    cx: &mut TestAppContext,
) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut view = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        view = Some(shell.clone());
        Root::new(shell, window, cx)
    });
    let shell = view.unwrap();
    let first = cx
        .update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.send_prompt_text("Receipt fixture", window, cx);
            });
            let thread = shell.read(cx).workspace().active_thread().unwrap().clone();
            shell.update(cx, |shell, cx| shell.start_new_chat(window, cx));
            thread
        })
        .unwrap();
    let selected = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    while commands.try_recv().is_ok() {}
    events
        .send_blocking(Event::Memory(sovra_core::MemoryEvent::SourceSaved {
            session_id: first.id.clone(),
            message: Some(sovra_core::MemorySourceMessage {
                id: Some(first.messages[0].id.clone()),
                in_reply_to: None,
                role: sovra_core::MessageRole::User,
                text: first.messages[0].text.clone(),
            }),
            receipt: sovra_core::MemorySourceReceipt {
                source_id: sovra_core::EpisodeId("episode:receipt".into()),
                version: 1,
                has_processing_job: true,
            },
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.workspace().session_id.clone()),
        selected
    );
    let saves = std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::SaveChatThread { thread } => Some(thread),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(saves.iter().any(|thread| thread.id == first.id
        && thread.updated_at == first.updated_at
        && thread.messages[0].source == Some(sovra_core::EpisodeId("episode:receipt".into()))));
}

#[gpui_kit::test]
fn schedule_save_keeps_the_draft_until_confirmed_and_can_retry(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Schedules, cx));
        window.render_frame(cx);
        window.click("schedule-empty-create-btn", cx);
        window.click("schedule-name-input", cx);
        window.input("Garden reminder", cx);
        window.click("schedule-cadence-input", cx);
        window.input("every day at 9", cx);
        window.click("schedule-prompt-input", cx);
        window.input("Check the garden", cx);
        window.click("schedule-save-btn", cx);
        assert!(window.find("schedule-create-form").visible());
        assert_eq!(
            shell.read(cx).workspace().schedules,
            Vec::<sovra_core::ScheduleItem>::new()
        );
    })
    .unwrap();
    let first = std::iter::from_fn(|| commands.try_recv().ok())
        .find(|command| {
            matches!(
                command,
                Command::Schedule(sovra_core::ScheduleCommand::Create { .. })
            )
        })
        .unwrap();
    events
        .send_blocking(Event::Schedule(sovra_core::ScheduleEvent::Error(
            "Test storage unavailable".into(),
        )))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("schedule-error").visible());
        window.click("schedule-save-btn", cx);
        assert!(window.find("schedule-create-form").visible());
    })
    .unwrap();
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| command == first));
    let mut book = sovra_core::ScheduleBook::default();
    let saved = sovra_core::create_in_book(
        &mut book,
        "Garden reminder",
        "every day at 9",
        "Check the garden",
        None,
        sovra_core::Timestamp(1),
        0,
    )
    .unwrap();
    events
        .send_blocking(Event::Schedule(sovra_core::snapshot_event(&book)))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("schedule-create-form").visible(),
            "a list refresh is not a save acknowledgement"
        );
    })
    .unwrap();
    events
        .send_blocking(Event::Schedule(sovra_core::ScheduleEvent::Created {
            schedule: Box::new(saved),
        }))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("schedule-create-form").is_none());
        assert_eq!(shell.read(cx).workspace().schedules.len(), 1);
        window.click("schedule-create-toggle-btn", cx);
        window.click("schedule-save-btn", cx);
        assert!(
            window.find("schedule-error").visible(),
            "the next task must start with blank fields"
        );
    })
    .unwrap();
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Schedule(sovra_core::ScheduleCommand::Create { .. })
        ))
    );
}
