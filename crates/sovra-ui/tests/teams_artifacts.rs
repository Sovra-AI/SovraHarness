//! Team and Artifacts, end to end through the shell with a scripted runtime.

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle, px, size};
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource,
};
use sovra_core::models::InterfaceScale;
use sovra_core::teams::{SpeakerState, TeamActivity, TeamCommand, TeamEvent, default_team};
use sovra_core::{
    AgUiEvent, AgUiWidget, Command, Event, LiveActivity, NavTab, ParsedResearchGoal, ResearchBoard,
    ResearchGoalState, Speaker, SubagentOutcome, Timestamp, parse_research_plan,
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    WindowHandle<Root>,
    Entity<Shell>,
    async_channel::Receiver<Command>,
    async_channel::Sender<Event>,
) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    events
        .send_blocking(Event::Teams(TeamEvent::Loaded {
            members: default_team(),
        }))
        .expect("headless test fixture is available");
    settle(cx);
    (
        window,
        shell.expect("headless test fixture is available"),
        commands,
        events,
    )
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn drain(commands: &async_channel::Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn artifact(id: &str, kind: ArtifactKind, content: &str, at: i64) -> Artifact {
    Artifact::from_draft(
        id.into(),
        ArtifactDraft {
            title: format!("{id} title"),
            kind,
            language: if kind == ArtifactKind::Code {
                "python".into()
            } else {
                String::new()
            },
            content: content.into(),
            source: ArtifactSource {
                session_id: "s".into(),
                message_id: "m".into(),
                author: "Priya".into(),
            },
        },
        Timestamp(at),
    )
}

#[gpui_kit::test]
fn ready_loads_the_team_and_library(cx: &mut TestAppContext) {
    let (_window, _shell, commands, events) = open(cx, 1280., 820.);
    drain(&commands);
    events
        .send_blocking(Event::Ready {
            version: "1".into(),
        })
        .unwrap();
    settle(cx);
    let sent = drain(&commands);
    assert!(sent.contains(&Command::Teams(TeamCommand::Load)));
    assert!(sent.contains(&Command::Artifacts(ArtifactCommand::Load)));
}

#[gpui_kit::test]
fn team_page_fits_every_window_and_text_size_and_asks_the_team(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, commands, _events) = open(cx, width, height);
        for theme in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update(|cx| {
                Theme::change(theme, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
                window.render_frame(cx);
                let page = window.find("sovra-team-view").bounds();
                let ask = window.find("team-ask");
                assert!(ask.visible(), "{width}x{height}@{scale}: {ask:?}");
                assert!(ask.bounds().right() <= page.right());
                let card = window.find(("team-member", 0usize));
                assert!(card.bounds().left() >= page.left());
                assert!(
                    card.bounds().right() <= page.right(),
                    "cards stay inside the page"
                );
            })
            .unwrap();
        }
        cx.update_window(window.into(), |_, window, cx| window.click("team-ask", cx))
            .unwrap();
        settle(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(shell.read(cx).workspace().active_tab, NavTab::Chat);
            assert!(shell.read(cx).workspace().team_mode);
            assert!(window.find("composer-team-note").visible());
            assert!(
                window.find("chat-composer-input").visible(),
                "{width}x{height}@{scale}: input {:?} container {:?}",
                window.find("chat-composer-input").bounds(),
                window.find("sovra-composer-container").bounds()
            );
            shell.update(cx, |shell, cx| {
                shell.send_prompt_text("Plan my garden", window, cx);
            });
        })
        .unwrap();
        let sent = drain(&commands);
        let run = sent
            .iter()
            .find_map(|command| match command {
                Command::Teams(TeamCommand::Run {
                    prompt, context, ..
                }) => Some((prompt, context)),
                _ => None,
            })
            .expect("team mode sends a team run");
        assert_eq!(run.0, "Plan my garden");
        assert!(
            run.1.messages().is_empty(),
            "the current request is separate from prior history"
        );
        cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
    }
}

#[gpui_kit::test]
fn team_members_can_be_switched_off_and_edited(cx: &mut TestAppContext) {
    let (window, shell, commands, _events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Team, cx));
        window.render_frame(cx);
        window.click(("team-member-active", 1usize), cx);
        window.click(("team-member-edit", 2usize), cx);
        window.render_frame(cx);
        assert!(window.find("team-member-form").visible());
        window.click("team-form-save", cx);
    })
    .unwrap();
    let sent = drain(&commands);
    let saved: Vec<_> = sent
        .iter()
        .filter_map(|command| match command {
            Command::Teams(TeamCommand::SaveMember(member)) => Some(member),
            _ => None,
        })
        .collect();
    assert_eq!(saved.len(), 2, "{sent:?}");
    assert_eq!((saved[0].id.as_str(), saved[0].active), ("morgan", false));
    assert_eq!(saved[1].id, "priya");
    assert_eq!(saved[1].name, "Priya", "editing keeps what was there");
}

#[gpui_kit::test]
fn mentions_route_to_the_team_and_agui_events_show_each_member(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 640., 420.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("@priya is coffee healthy?", window, cx);
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    assert!(
        drain(&commands)
            .iter()
            .any(|command| matches!(command, Command::Teams(TeamCommand::Run { .. })))
    );
    let members = default_team();
    let mut activity = TeamActivity::new(1, &members, &["priya".into()]);
    activity.set("priya", SpeakerState::Speaking);
    let priya = Speaker {
        member_id: "priya".into(),
        name: "Priya".into(),
        icon: "🔬".into(),
        role: "Researcher".into(),
    };
    for event in [
        AgUiEvent::RunStarted { run_id: "r".into() },
        AgUiEvent::ActivitySnapshot {
            message_id: "r-team".into(),
            activity: LiveActivity::Team(activity.clone()),
        },
        AgUiEvent::SubagentStarted {
            subagent_run_id: "r-1".into(),
            name: "priya".into(),
            description: "Researcher".into(),
            speaker: priya,
        },
        AgUiEvent::TextMessageStart {
            message_id: "r-1-reply".into(),
            subagent_run_id: Some("r-1".into()),
        },
        AgUiEvent::TextMessageContent {
            message_id: "r-1-reply".into(),
            delta: "In moderation, yes.".into(),
        },
        AgUiEvent::TextMessageEnd {
            message_id: "r-1-reply".into(),
        },
        AgUiEvent::SubagentFinished {
            subagent_run_id: "r-1".into(),
            outcome: SubagentOutcome::Success,
        },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    settle(cx);
    activity.set("priya", SpeakerState::Done);
    activity.finish("Your team is done");
    for event in [
        AgUiEvent::ActivitySnapshot {
            message_id: "r-team".into(),
            activity: LiveActivity::Team(activity),
        },
        AgUiEvent::RunFinished { run_id: "r".into() },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    settle(cx);
    let saved = drain(&commands)
        .into_iter()
        .filter_map(|command| {
            if let Command::SaveChatThread { thread } = command {
                Some(thread)
            } else {
                None
            }
        })
        .next_back()
        .expect("team completion must persist the conversation without navigation");
    assert!(
        saved
            .messages
            .iter()
            .any(|message| message.text == "In moderation, yes." && message.author == "Priya")
    );
    assert!(saved.messages.iter().any(|message| {
        message
            .widgets
            .iter()
            .any(|widget| matches!(widget, AgUiWidget::TeamActivity(activity) if activity.finished))
    }));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = shell.read(cx).workspace();
        assert!(!state.is_generating);
        let reply = state
            .messages
            .iter()
            .find(|m| m.id == "r-1-reply")
            .expect("member reply");
        assert_eq!(reply.author, "Priya");
        assert_eq!(reply.text, "In moderation, yes.");
        let card = state
            .messages
            .iter()
            .position(|m| m.id == "r-team")
            .expect("team card message");
        let status = window.find(("team-activity-status", card * 10));
        assert!(status.visible(), "{status:?}");
        assert!(window.find("chat-composer-input").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn deep_research_board_and_document_card_stay_in_chat(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text(
                "/deep-research Compare heat pumps vs gas boilers",
                window,
                cx,
            );
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    // Deep research is a normal prompt route (skill), not a Team page navigation.
    assert!(
        !drain(&commands)
            .iter()
            .any(|command| matches!(command, Command::Teams(TeamCommand::Run { .. })))
    );
    let mut board = ResearchBoard::planning("Compare heat pumps vs gas boilers");
    let goals: Vec<_> = parse_research_plan(
        "- [RESEARCH] Investigate heat pump efficiency ratings today\n\
         - [RESEARCH] Compare running costs versus gas boilers now\n\
         - [DELIVERABLE] Write a homeowner summary for the family",
    )
    .into_iter()
    .map(ParsedResearchGoal::into_goal)
    .collect();
    board.with_plan(goals);
    board.set("goal-0", ResearchGoalState::Working);
    board.set("goal-1", ResearchGoalState::Working);
    for event in [
        AgUiEvent::RunStarted {
            run_id: "research-1".into(),
        },
        AgUiEvent::ActivitySnapshot {
            message_id: "research-1-board".into(),
            activity: LiveActivity::Research(board.clone()),
        },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    settle(cx);
    board.set("goal-0", ResearchGoalState::Done);
    board.set("goal-1", ResearchGoalState::Done);
    board.begin_writing();
    board.set("goal-2", ResearchGoalState::Done);
    board.finish("Your research document is ready");
    for event in [
        AgUiEvent::ActivitySnapshot {
            message_id: "research-1-board".into(),
            activity: LiveActivity::Research(board),
        },
        AgUiEvent::TextMessageStart {
            message_id: "research-1-closing".into(),
            subagent_run_id: None,
        },
        AgUiEvent::TextMessageContent {
            message_id: "research-1-closing".into(),
            delta: "Your research document is ready. Open it below.".into(),
        },
        AgUiEvent::TextMessageEnd {
            message_id: "research-1-closing".into(),
        },
        AgUiEvent::RunFinished {
            run_id: "research-1".into(),
        },
    ] {
        events
            .send_blocking(Event::AgUi {
                session_id: session.clone(),
                event,
            })
            .unwrap();
    }
    events
        .send_blocking(Event::AgUiSurface {
            session_id: session,
            widget: AgUiWidget::ArtifactSaved {
                artifact_id: "doc-heat-pumps".into(),
                title: "Compare heat pumps vs gas boilers".into(),
                artifact_kind: ArtifactKind::Document,
            },
        })
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = shell.read(cx).workspace();
        assert_eq!(
            state.active_tab,
            NavTab::Chat,
            "no separate Team/Goals window"
        );
        assert!(!state.is_generating);
        let board_msg = state
            .messages
            .iter()
            .position(|m| m.id == "research-1-board")
            .expect("research board message");
        assert!(
            window.find(("research-board", board_msg * 10)).visible(),
            "research board should render in chat"
        );
        assert!(
            window
                .find(("research-board-lanes", board_msg * 10))
                .visible(),
            "kanban lanes should be visible"
        );
        assert!(
            window
                .find(("research-board-status", board_msg * 10))
                .visible(),
            "board status should be visible"
        );
        let closing = state
            .messages
            .iter()
            .find(|m| m.id == "research-1-closing")
            .expect("closing note");
        assert!(closing.text.contains("research document is ready"));
        // Widget ElementIds use message_index × 10 + widget_index (see chat_view).
        let artifact_index = state
            .messages
            .iter()
            .enumerate()
            .find_map(|(message_index, message)| {
                message
                    .widgets
                    .iter()
                    .position(|widget| {
                        matches!(
                            widget,
                            AgUiWidget::ArtifactSaved {
                                artifact_id,
                                artifact_kind: ArtifactKind::Document,
                                ..
                            } if artifact_id == "doc-heat-pumps"
                        )
                    })
                    .map(|widget_index| message_index * 10 + widget_index)
            })
            .expect("document artifact card on a chat message");
        assert!(
            window.find(("artifact-saved", artifact_index)).visible(),
            "document card must render in chat"
        );
        assert!(
            window
                .find(("artifact-saved-open", artifact_index))
                .visible(),
            "Open control on the document card must be visible"
        );
        assert!(window.find("chat-composer-input").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn saving_a_reply_and_browsing_the_library(cx: &mut TestAppContext) {
    let (window, shell, commands, events) = open(cx, 1280., 820.);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Write a shopping list", window, cx);
        });
    })
    .unwrap();
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    drain(&commands);
    for event in [
        Event::AgentResponseStarted {
            session_id: session.clone(),
            model: sovra_core::ResponseModel {
                model_id: "m".into(),
                name: "Model".into(),
                provider: "Test".into(),
            },
        },
        Event::AgentChunk {
            session_id: session.clone(),
            text: "# Shopping\n\n- [ ] Milk\n- [ ] Eggs".into(),
        },
        Event::AgentCompleted {
            session_id: session.clone(),
        },
    ] {
        events.send_blocking(event).unwrap();
    }
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("save-message", 1usize), cx);
    })
    .unwrap();
    let draft = drain(&commands)
        .into_iter()
        .find_map(|command| match command {
            Command::Artifacts(ArtifactCommand::Save(draft)) => Some(draft),
            _ => None,
        })
        .expect("Save sends the reply to Artifacts");
    assert_eq!(draft.kind, ArtifactKind::Checklist);
    assert_eq!(draft.title, "Shopping");
    assert_eq!(draft.source.session_id, session);

    let mut saved = artifact("list", ArtifactKind::Checklist, &draft.content, 3);
    saved.source.message_id = draft.source.message_id;
    events
        .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![
            saved,
            artifact("code", ArtifactKind::Code, "print('hi')", 2),
            artifact("page", ArtifactKind::Webpage, "<h1>Hi</h1>", 1),
        ])))
        .unwrap();
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("save-message", 1usize)).label(),
            Some("Saved to Artifacts")
        );
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        assert!(window.find(("artifact-card", 2usize)).visible());
        window.click(("artifact-kind", 3usize), cx);
        window.render_frame(cx);
        assert!(window.find(("artifact-card", 0usize)).visible());
        assert!(
            window.find_all(("artifact-card", 1usize)).is_empty(),
            "the filter hides other kinds"
        );
        window.click("artifact-kind-all", cx);
        window.render_frame(cx);
        window.click(("artifact-open", 2usize), cx);
        window.render_frame(cx);
        assert!(window.find("artifact-detail").visible());
        assert!(window.find("artifact-open-browser").visible());
        window.click("artifact-open-browser", cx);
        window.click("artifact-delete", cx);
        window.render_frame(cx);
        window.click("artifact-delete", cx);
    })
    .unwrap();
    let sent = drain(&commands);
    assert!(sent.contains(&Command::Artifacts(ArtifactCommand::Open {
        id: "page".into()
    })));
    assert!(sent.contains(&Command::Artifacts(ArtifactCommand::Delete {
        id: "page".into()
    })));
}

#[gpui_kit::test]
fn artifacts_page_fits_minimum_wide_and_double_size(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (1280., 820., 200)] {
        let (window, shell, _commands, events) = open(cx, width, height);
        events
            .send_blocking(Event::Artifacts(ArtifactEvent::Loaded(vec![artifact(
                "doc",
                ArtifactKind::Document,
                "# A long letter\n\nDear friend, this is a long paragraph that must wrap inside the page rather than overflow it on any screen.",
                1,
            )])))
            .unwrap();
        settle(cx);
        cx.update(|cx| design::set_interface_scale(InterfaceScale::from_percent(scale), cx));
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| shell.open_artifact("doc", cx));
            window.render_frame(cx);
            let page = window.find("sovra-artifacts-view").bounds();
            for id in ["artifact-copy", "artifact-preview"] {
                let element = window.find(id);
                assert!(element.visible(), "{width}x{height}@{scale}: {id}");
                assert!(element.bounds().right() <= page.right(), "{id} overflows");
            }
        })
        .unwrap();
        cx.update(|cx| design::set_interface_scale(InterfaceScale::default(), cx));
    }
}
