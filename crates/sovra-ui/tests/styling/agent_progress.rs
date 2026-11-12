//! Live agent progress: the work log at the top of a reply and the team kanban
//! board must fit, and stay usable, at every supported size in both themes.

use super::*;
use sovra_core::teams::{TeamActivity, default_team};
use sovra_core::work::{StepCall, WorkActivity};
use sovra_core::{AgUiEvent, Event, LiveActivity};

const SIZES: [(f32, f32, u16); 3] = [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)];

fn open(
    cx: &mut TestAppContext,
    mode: ThemeMode,
    (width, height, percent): (f32, f32, u16),
) -> (
    gpui_kit::WindowHandle<Root>,
    async_channel::Sender<Event>,
    String,
) {
    let (client, _commands, events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        Theme::change(mode, None, cx);
        design::set_interface_scale(
            sovra_core::models::InterfaceScale::from_percent(percent),
            cx,
        );
    });
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let shell = cx.new(|cx| Shell::new(0, window, cx));
        Root::new(shell, window, cx)
    });
    (
        window,
        events,
        sovra_core::WorkspaceState::new(0).session_id,
    )
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

fn reset(cx: &mut TestAppContext) {
    cx.update(|cx| {
        design::set_interface_scale(sovra_core::models::InterfaceScale::default(), cx);
    });
}

fn snapshot(events: &async_channel::Sender<Event>, session: &str, activity: LiveActivity) {
    events
        .send_blocking(Event::AgUi {
            session_id: session.to_owned(),
            event: AgUiEvent::ActivitySnapshot {
                message_id: "turn-activity".into(),
                activity,
            },
        })
        .expect("headless test fixture is available");
}

/// Sovra asks two helpers at once; one of them is searching the web.
fn parallel_work() -> WorkActivity {
    let mut work = WorkActivity::default();
    let request =
        serde_json::json!({"request": "compare heat pumps and gas boilers for a small house"});
    for (id, tool, name) in [
        ("a", "WebSearchAgent", "Web Search helper"),
        ("b", "CodingAgent", "Coding helper"),
    ] {
        work.start(StepCall {
            id,
            helper: "",
            helper_name: "",
            tool,
            args: &request,
            asks_helper: Some(name),
            at_ms: 0,
        });
    }
    let query = serde_json::json!({"query": "heat pump running costs per year in a cold climate"});
    work.start(StepCall {
        id: "a.1",
        helper: "WebSearchAgent",
        helper_name: "Web Search helper",
        tool: "web_search",
        args: &query,
        asks_helper: None,
        at_ms: 400,
    });
    work
}

fn assert_inside(window: &mut gpui_kit::Window, id: impl Into<gpui_kit::ElementId>, width: f32) {
    let id = id.into();
    let bounds = window.find(id.clone()).bounds();
    assert!(
        bounds.left() >= px(0.) && bounds.right() <= px(width),
        "{id:?} overflows the window: {bounds:?}"
    );
    assert!(bounds.size.height > px(0.), "{id:?} has no height");
}

#[gpui_kit::test]
fn the_work_log_shows_parallel_helpers_then_folds_when_done(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for case in SIZES {
            let width = case.0;
            let (window, events, session) = open(cx, mode, case);
            let mut work = parallel_work();
            snapshot(&events, &session, LiveActivity::Work(work.clone()));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_inside(
                    window,
                    gpui_kit::SharedString::from("work-helper-0-0"),
                    width,
                );
                assert_inside(
                    window,
                    gpui_kit::SharedString::from("work-helper-0-1"),
                    width,
                );
                assert_inside(window, gpui_kit::SharedString::from("work-step-0-2"), width);
                assert!(window.try_find(("work-log-status", 0_usize)).is_some());
                assert!(
                    window.try_find(("work-log-steps", 0_usize)).is_some(),
                    "live logs are open"
                );
            })
            .unwrap();

            work.finish_step("a.1", true, 1_400);
            work.finish_step("a", true, 2_000);
            work.finish_step("b", true, 2_600);
            work.finish(true, 3_000);
            snapshot(&events, &session, LiveActivity::Work(work));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.try_find(("work-log-steps", 0_usize)).is_none(),
                    "a finished log folds to one line"
                );
                assert_inside(window, ("work-log-toggle", 0_usize), width);
                window.click(("work-log-toggle", 0_usize), cx);
            })
            .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find(("work-log-steps", 0_usize)).is_some());
                assert_inside(window, gpui_kit::SharedString::from("work-step-0-0"), width);
            })
            .unwrap();
            reset(cx);
        }
    }
}

#[gpui_kit::test]
fn the_team_card_is_a_kanban_board_of_members_working_side_by_side(cx: &mut TestAppContext) {
    let members = default_team();
    let ids: Vec<String> = members.iter().take(3).map(|m| m.id.clone()).collect();
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for case in SIZES {
            let width = case.0;
            let (window, events, session) = open(cx, mode, case);
            let mut activity = TeamActivity::new(1, &members, &ids);
            activity.start_together(&ids[..2]);
            activity.set_doing(&ids[0], "Searching the web for “soup”");
            snapshot(&events, &session, LiveActivity::Team(activity));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                for lane in ["waiting", "working", "done"] {
                    assert_inside(
                        window,
                        gpui_kit::SharedString::from(format!("team-lane-{lane}-0")),
                        width,
                    );
                }
                for id in &ids[..2] {
                    assert_inside(
                        window,
                        gpui_kit::SharedString::from(format!("team-member-working-0-{id}")),
                        width,
                    );
                }
                assert_inside(
                    window,
                    gpui_kit::SharedString::from(format!("team-member-waiting-0-{}", ids[2])),
                    width,
                );
                assert!(
                    window
                        .try_find(("team-activity-parallel", 0_usize))
                        .is_some()
                );
            })
            .unwrap();
            reset(cx);
        }
    }
}

/// Replies stream into the chat and push the in-chat board out of view, so while
/// a team turn runs the board is also docked above the chat box, fully on screen.
#[gpui_kit::test]
fn the_live_team_board_stays_in_view_above_the_chat_box(cx: &mut TestAppContext) {
    let members = default_team();
    let ids: Vec<String> = members.iter().take(3).map(|m| m.id.clone()).collect();
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for case in SIZES {
            let (width, height, _) = case;
            let (window, events, session) = open(cx, mode, case);
            events
                .send_blocking(Event::AgUi {
                    session_id: session.clone(),
                    event: AgUiEvent::RunStarted {
                        run_id: "run".into(),
                    },
                })
                .expect("headless test fixture is available");
            let mut activity = TeamActivity::new(1, &members, &ids);
            activity.start_together(&ids[..2]);
            activity.set_doing(&ids[0], "Searching the web for “soup”");
            snapshot(&events, &session, LiveActivity::Team(activity.clone()));
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                let dock = window.find("team-dock").bounds();
                assert!(
                    dock.top() >= px(0.) && dock.bottom() <= px(height),
                    "the live board is on screen: {dock:?}"
                );
                assert_inside(window, "team-dock", width);
                for lane in ["waiting", "working", "done"] {
                    assert!(
                        window
                            .try_find(gpui_kit::SharedString::from(format!(
                                "team-dock-lane-{lane}"
                            )))
                            .is_some()
                    );
                }
                assert!(
                    window
                        .try_find(gpui_kit::SharedString::from(format!(
                            "team-dock-working-{}",
                            ids[0]
                        )))
                        .is_some(),
                    "a working member is in the Working lane"
                );
            })
            .unwrap();
            // When the turn finishes, the dock gives the space back to the chat.
            activity.finish("Your team is done");
            snapshot(&events, &session, LiveActivity::Team(activity));
            events
                .send_blocking(Event::AgUi {
                    session_id: session.clone(),
                    event: AgUiEvent::RunFinished {
                        run_id: "run".into(),
                    },
                })
                .expect("headless test fixture is available");
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("team-dock").is_none());
            })
            .unwrap();
            reset(cx);
        }
    }
}
