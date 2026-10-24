//! Headless UI tests: real views in a GPUI test window, runtime replaced by loopback channels.

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::{AgUiWidget, Command, Event, NavTab, RuntimeStatus, ScreenRoute};
use sovra_runtime::Client;
use sovra_ui::Shell;

#[gpui_kit::test]
fn status_follows_runtime_events(cx: &mut TestAppContext) {
    let (client, _commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));

    let mut shell = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
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
        let view = cx.new(|cx| Shell::new(window, cx));
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

        // Switch to Schedules tab -> renders Schedules Command Center.
        window.click("nav-schedules", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Schedules);
        assert!(window.find("sovra-schedules-view").visible());

        // Run a schedule in chat -> switches back to Chat and dispatches Command::RunPrompt.
        window.click(("schedule-run", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);

        // Sign out via user profile -> renders Login screen, then sign back in.
        window.click("sidebar-user-profile", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::Login);
        assert!(window.find("sovra-login-screen").visible());

        window.click("login-submit-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().route, ScreenRoute::MainWorkspace);
    })
    .unwrap();

    // Verify the schedule run dispatched a real Command::RunPrompt to the runtime.
    let dispatched = commands.recv_blocking().unwrap();
    assert!(matches!(dispatched, Command::RunPrompt { .. }));

    // Stream an AG-UI surface and text chunk back from the runtime.
    events
        .send_blocking(Event::AgUiSurface {
            session_id: "personal-ai-setup".into(),
            widget: AgUiWidget::SurfaceCard {
                surface_id: "personal-ai-summary".into(),
                title: "AG-UI Surface · personal-ai-summary".into(),
                summary: "Updated structured AG-UI surface (revision v1).".into(),
            },
        })
        .unwrap();
    events
        .send_blocking(Event::AgentChunk {
            session_id: "personal-ai-setup".into(),
            text: "Morning brief completed.".into(),
        })
        .unwrap();
    events
        .send_blocking(Event::AgentCompleted {
            session_id: "personal-ai-setup".into(),
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
        let view = cx.new(|cx| Shell::new(window, cx));
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
        assert_eq!(shell.read(cx).workspace().specialists.len(), 22);
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
    let window = cx.open_window(size(px(1000.), px(760.)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(window, cx));
        Root::new(shell, window, cx)
    });
    events
        .send_blocking(Event::AgUiSurface {
            session_id: "personal-ai-setup".into(),
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
            let left = window.find(("nav-icon", 0usize)).bounds().left();
            let text_left = window.find(("nav-label", 0usize)).bounds().left();
            let button_left = window.find("nav-chat").bounds().left();
            assert!(
                left - button_left < px(20.),
                "nav content must start at left padding"
            );
            for index in 1..NavTab::ALL.len() {
                assert_eq!(window.find(("nav-icon", index)).bounds().left(), left);
                assert_eq!(window.find(("nav-label", index)).bounds().left(), text_left);
            }
            assert!(window.find(("native-surface", 0usize)).visible());
            assert!(window.find("surface-action-0-0-2").visible());
        }
        window.click("surface-action-0-0-2", cx);
    })
    .unwrap();
    let dispatched = std::iter::from_fn(|| commands.try_recv().ok())
        .find(|command| matches!(command, Command::RunPrompt { .. }));
    let Some(Command::RunPrompt { session_id, prompt }) = dispatched else {
        panic!("action must return to agent")
    };
    assert_eq!(session_id, "personal-ai-setup");
    let action: serde_json::Value = serde_json::from_str(&prompt).unwrap();
    assert_eq!(action["surface_ref"]["id"], "report");
    assert_eq!(action["surface_ref"]["version"], 3);
    assert_eq!(action["event"]["action_id"], "next");
}

#[gpui_kit::test]
fn composer_enter_sends_shift_enter_adds_line_and_stop_clears_queue(cx: &mut TestAppContext) {
    let (client, commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
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
    let (client, commands, _events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("chat-history-rail").visible());
        assert!(window.find("chat-filter-pills").visible());

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

        // Delete all 3 seeded schedules -> verifies the Schedules Empty Placeholder ("No schedules yet") appears.
        window.click(("schedule-delete", 0usize), cx);
        window.render_frame(cx);
        window.click(("schedule-delete", 0usize), cx);
        window.render_frame(cx);
        window.click(("schedule-delete", 0usize), cx);
        window.render_frame(cx);
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
fn interactive_system_tour_walks_through_all_five_tabs_and_state_aware_chat_setup(
    cx: &mut TestAppContext,
) {
    use sovra_core::models::{ModelConnection, ModelEvent, ModelPreferences, ProviderKind};

    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
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
        // Top setup coach strip must show Step 2 (Pick Model), NOT Step 1 (Connect Key).
        assert!(window.find("sovra-setup-coach-strip").visible());
        assert!(window.find("setup-strip-pick-model").visible());
        assert!(window.try_find("setup-strip-connect").is_none());

        // Sending a message in Chat when key is set must ONLY show Step 2 model selection in chat.
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Hi there!", window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("surface-setup-recommended-models").visible());
        assert!(window.try_find("surface-setup-openrouter-key").is_none());

        // Launch the 5-step Interactive System Tour from the chat card button.
        window.click("surface-setup-start-tour", cx);
        window.render_frame(cx);
        assert!(window.find("sovra-tour-coach-bar").visible());
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
        assert!(
            window
                .find(("nav-spotlight-badge", NavTab::Chat as usize))
                .visible()
        );

        // Step 1 -> Step 2 (My AI's)
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::MyAis);
        assert!(
            window
                .find(("nav-spotlight-badge", NavTab::MyAis as usize))
                .visible()
        );

        // Step 2 -> Step 3 (Knowledge)
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Knowledge);

        // Step 3 -> Step 4 (Schedules)
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Schedules);

        // Step 4 -> Step 5 (Settings)
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
        assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Settings);

        // Finish Tour -> returns to Chat and closes tour bar
        window.click("tour-next-btn", cx);
        window.render_frame(cx);
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

#[gpui_kit::test]
fn unified_single_sidebar_and_ai_title_generation_work_end_to_end(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(window, cx));
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
        let chat_main_bounds = window.find("sovra-chat-main").bounds();

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
            (chat_main_bounds.left() - sidebar_bounds.right()).abs() < px(4.),
            "chat column must sit directly beside the single sidebar with no second sidebar in between"
        );

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
            session_id: "personal-ai-setup".into(),
            text: "Here is how to build a zero-copy HTTP/3 frame parser in Rust.".into(),
        })
        .unwrap();
    events
        .send_blocking(Event::AgentCompleted {
            session_id: "personal-ai-setup".into(),
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
                if session_id == "personal-ai-setup"
                && prompt.contains("zero-copy parser")
                && reply.contains("HTTP/3 frame parser")
        )),
        "completing the first exchange must dispatch Command::GenerateChatTitle"
    );

    // Deliver the model-generated title and verify it updates the sidebar history row
    events
        .send_blocking(Event::ChatTitleGenerated {
            session_id: "personal-ai-setup".into(),
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
