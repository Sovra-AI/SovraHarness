//! Decisions remain reachable, bounded and readable at every required size.
use super::*;
use gpui_kit::test::TestQueryExt as _;
use sovra_core::decisions::{
    DecisionCommand, DecisionDraft, DecisionEvent, DecisionEvidence, DecisionPerson, DecisionRecord,
};
use sovra_core::{Command, Event, NavTab, Timestamp};

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn reveal(
    window: &mut gpui_kit::Window,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut gpui_kit::App,
) {
    let id = id.into();
    for _ in 0..60 {
        window.render_frame(cx);
        let pane = window.find("sovra-decisions-view").bounds();
        let item = window.find(id.clone()).bounds();
        assert!(
            item.left() >= pane.left() && item.right() <= pane.right(),
            "{id:?}: {item:?} outside {pane:?}"
        );
        if item.top() >= pane.top() && item.bottom() <= pane.bottom() {
            return;
        }
        let delta = if item.top() < pane.top() {
            pane.top() - item.top()
        } else {
            pane.bottom() - item.bottom()
        };
        window.scroll(
            "sovra-decisions-view",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), delta)),
            cx,
        );
    }
    panic!("{id:?} must be reachable");
}

#[gpui_kit::test]
fn a_chat_reply_keeps_its_source_and_changed_stops_need_a_fresh_check(cx: &mut TestAppContext) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.expect("shell created");
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.send_prompt_text("Should we renew our workshop lease?", window, cx);
        });
    })
    .expect("window available");
    let session = shell.read_with(cx, |shell, _| shell.workspace().session_id.clone());
    let reply = "Renew for one year, subject to the company's checks.";
    for event in [
        Event::AgentResponseStarted {
            session_id: session.clone(),
            model: sovra_core::ResponseModel {
                model_id: "test".into(),
                name: "Test model".into(),
                provider: "Test".into(),
            },
        },
        Event::AgentChunk {
            session_id: session.clone(),
            text: reply.into(),
        },
        Event::AgentCompleted {
            session_id: session.clone(),
        },
    ] {
        events
            .send_blocking(event)
            .expect("scripted reply delivered");
    }
    settle(cx);
    let message = shell.read_with(cx, |shell, _| {
        shell
            .workspace()
            .messages
            .last()
            .expect("reply exists")
            .id
            .clone()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(("record-decision", 1usize), cx);
        for (label, value) in [
            ("Question", "Should we renew our workshop lease?"),
            ("Our criteria", "Rent stays within the budget."),
            ("Review on (YYYY-MM-DD, UTC)", "2099-01-01"),
            (
                "What would end this decision?",
                "The workshop is no longer available.",
            ),
            ("Cited passage 1", "The lease can be renewed for one year."),
        ] {
            window.render_frame(cx);
            let id = window
                .find_by_label(label)
                .path()
                .last()
                .expect("input has a native ID")
                .clone();
            reveal(window, id.clone(), cx);
            window.click(id, cx);
            window.input(value, cx);
        }
        reveal(window, ("decision-source", 0usize), cx);
        window.click(("decision-source", 0usize), cx);
        window.input("Lease page 2", cx);
        reveal(window, "decision-add-stop", cx);
        window.click("decision-add-stop", cx);
        reveal(window, ("decision-stop", 0usize), cx);
        window.click(("decision-stop", 0usize), cx);
        window.input("Budget reviewed", cx);
        reveal(window, ("decision-stop-cleared", 0usize), cx);
        window.click(("decision-stop-cleared", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("decision-stop-cleared", 0usize)).checked(),
            Some(true)
        );
        reveal(window, ("decision-stop", 0usize), cx);
        window.click(("decision-stop", 0usize), cx);
        window.input(" plus manager approval", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("decision-stop-cleared", 0usize)).checked(),
            Some(false)
        );
        reveal(window, "decision-save", cx);
        window.click("decision-save", cx);
    })
    .expect("window available");
    let draft = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            Command::Decisions(DecisionCommand::Save {
                id: None,
                expected_revision: 0,
                draft,
            }) => Some(draft),
            _ => None,
        })
        .expect("chat-started decision sends a valid draft");
    assert_eq!(draft.decision, reply);
    assert_eq!(
        draft.chat,
        Some(DecisionEvidence {
            reference: format!("Chat {session} / reply {message}"),
            passage: reply.into()
        })
    );
    assert_eq!(draft.evidence[0].reference, "Lease page 2");
    assert_eq!(draft.hard_stops.len(), 1);
    assert!(draft.hard_stops[0].condition.contains("manager approval"));
    assert!(
        !draft.hard_stops[0].cleared,
        "changing a written condition requires checking it again"
    );
    events
        .send_blocking(Event::Decisions(DecisionEvent::Error(
            "Test save failure".into(),
        )))
        .expect("save failure delivered");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, ("decision-stop-cleared", 0usize), cx);
        window.click(("decision-stop-cleared", 0usize), cx);
        reveal(window, "decision-save", cx);
        window.click("decision-save", cx);
    })
    .expect("window available");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Decisions(DecisionCommand::Save { draft, .. }) if draft.hard_stops[0].cleared
        )),
        "the person can explicitly check the revised wording"
    );
}

fn record() -> DecisionRecord {
    DecisionRecord::create(
        "lease".into(),
        DecisionDraft {
            question: "Should we renew our workshop lease?".into(),
            decision: "Renew for one year.".into(),
            criteria: "Within our budget and enough space for the team.".into(),
            evidence: vec![DecisionEvidence {
                reference: "Lease page 2".into(),
                passage: "Renewal is offered for one year.".into(),
            }],
            review_on: "2099-01-01".into(),
            ends_when: "The workshop is no longer available.".into(),
            ..DecisionDraft::default()
        },
        DecisionPerson {
            account_id: "writer".into(),
            name: "Writer".into(),
        },
        Timestamp(1),
    )
    .expect("decision test operation succeeds")
}

#[gpui_kit::test]
fn acceptance_requires_a_second_account_and_a_visible_confirmation(cx: &mut TestAppContext) {
    use sovra_core::auth::{AuthEvent, AuthProvider, AuthSession};
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| sovra_ui::init(client, cx));
    let mut shell = None;
    let window = cx.open_window(size(px(1280.), px(820.)), |window, cx| {
        let view = cx.new(|cx| Shell::new(0, window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.expect("shell created");
    let session = |id: &str| AuthSession {
        account_id: id.into(),
        full_name: id.into(),
        email: String::new(),
        provider: AuthProvider::Password,
        signed_in_at: Timestamp(1),
    };
    events
        .send_blocking(Event::Auth(AuthEvent::Loaded {
            session: Some(session("writer")),
            has_accounts: true,
            google_ready: false,
        }))
        .expect("writer signed in");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
        window.render_frame(cx);
        window.click("saved-work-decisions", cx);
    })
    .expect("window available");
    events
        .send_blocking(Event::Decisions(DecisionEvent::Opened(Box::new(record()))))
        .expect("record opened");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "decision-accept", cx);
        assert_eq!(window.find("decision-accept").disabled(), Some(true));
    })
    .expect("window available");
    events
        .send_blocking(Event::Auth(AuthEvent::SignedIn {
            session: session("reviewer"),
            new_account: false,
        }))
        .expect("reviewer signed in");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "decision-accept", cx);
        assert_ne!(window.find("decision-accept").disabled(), Some(true));
        window.click("decision-accept", cx);
    })
    .expect("window available");
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::Decisions(DecisionCommand::Accept { .. })))
    );
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "decision-confirm", cx);
        window.click("decision-confirm", cx);
    })
    .expect("window available");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| command
            == Command::Decisions(DecisionCommand::Accept {
                id: "lease".into(),
                expected_revision: 1
            }))
    );
    let accepted = record()
        .accept(
            1,
            DecisionPerson {
                account_id: "reviewer".into(),
                name: "reviewer".into(),
            },
            Timestamp(2),
        )
        .expect("second person accepts");
    events
        .send_blocking(Event::Decisions(DecisionEvent::Saved(Box::new(accepted))))
        .expect("save confirmed");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        reveal(window, "decision-status", cx);
        assert_eq!(
            window.find("decision-status").label(),
            Some("Accepted company position")
        );
    })
    .expect("window available");
}

#[gpui_kit::test]
fn decision_forms_and_records_fit_and_keep_failed_drafts(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(
                    sovra_core::models::InterfaceScale::from_percent(scale),
                    cx,
                );
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.expect("decision test operation succeeds");
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Artifacts, cx));
                window.render_frame(cx);
                window.click("saved-work-decisions", cx);
            })
            .expect("decision test operation succeeds");
            events
                .send_blocking(Event::Decisions(DecisionEvent::Listed {
                    records: vec![],
                    offset: 0,
                    more: false,
                }))
                .expect("decision test operation succeeds");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "decision-new", cx);
                window.click("decision-new", cx);
                reveal(window, "decision-save", cx);
                window.click("decision-save", cx);
                reveal(window, "decision-error", cx);
            })
            .expect("decision test operation succeeds");
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                    command,
                    Command::Decisions(DecisionCommand::Save { .. })
                ))
            );
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "decision-back", cx);
                window.click("decision-back", cx);
                window.render_frame(cx);
                reveal(window, "decision-confirm", cx);
                window.click("decision-confirm", cx);
            })
            .expect("decision test operation succeeds");
            events
                .send_blocking(Event::Decisions(DecisionEvent::Opened(Box::new(record()))))
                .expect("decision test operation succeeds");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "decision-status", cx);
                reveal(window, "decision-edit", cx);
                window.click("decision-edit", cx);
                reveal(window, "decision-save", cx);
                window.click("decision-save", cx);
            })
            .expect("decision test operation succeeds");
            let first = std::iter::from_fn(|| commands.try_recv().ok())
                .find(|command| matches!(command, Command::Decisions(DecisionCommand::Save { .. })))
                .expect("valid edit sends save");
            events
                .send_blocking(Event::Decisions(DecisionEvent::Error(
                    "Test disk full".into(),
                )))
                .expect("decision test operation succeeds");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "decision-error", cx);
                reveal(window, "decision-save", cx);
                window.click("decision-save", cx);
            })
            .expect("decision test operation succeeds");
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok()).any(|command| command == first),
                "failed save keeps every field"
            );
            events
                .send_blocking(Event::Decisions(DecisionEvent::Saved(Box::new(record()))))
                .expect("decision test operation succeeds");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "decision-status", cx);
                assert!(window.try_find("decision-save").is_none());
                reveal(window, "decision-history", cx);
                window.click("decision-history", cx);
            })
            .expect("decision test operation succeeds");
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .expect("decision test operation succeeds");
        }
    }
}
