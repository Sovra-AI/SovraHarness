//! A newly registered activity variant must render at every supported width.

use super::*;
use sovra_core::{
    AgUiWidget, Event, ResearchBoard, ResearchGoal, ResearchGoalKind, ResearchGoalState,
};

#[gpui_kit::test]
fn research_activity_renders_goals_at_minimum_wide_and_double_text(cx: &mut TestAppContext) {
    for (width, height, percent) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
        let (client, _commands, events) = Client::loopback();
        cx.update(|cx| {
            sovra_ui::init(client, cx);
            design::set_interface_scale(
                sovra_core::models::InterfaceScale::from_percent(percent),
                cx,
            );
        });
        let window = cx.open_window(size(px(width), px(height)), |window, cx| {
            let shell = cx.new(|cx| Shell::new(0, window, cx));
            Root::new(shell, window, cx)
        });
        let mut board = ResearchBoard::planning("Compare home heating choices");
        board.goals = [
            ResearchGoalState::Waiting,
            ResearchGoalState::Working,
            ResearchGoalState::Done,
            ResearchGoalState::Failed,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, state)| ResearchGoal {
            id: format!("goal-{index}"),
            kind: ResearchGoalKind::Research,
            text: "Compare installation cost and ongoing household expenses".into(),
            state,
            note: String::new(),
        })
        .collect();
        events
            .send_blocking(Event::AgUiSurface {
                session_id: sovra_core::WorkspaceState::new(0).session_id,
                widget: AgUiWidget::ResearchBoard(board),
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            for target in ["waiting-0-0", "working-0-0", "done-0-0", "done-0-1"] {
                let row = window.find(gpui_kit::SharedString::from(format!(
                    "research-goal-{target}"
                )));
                let bounds = row.bounds();
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                assert!(bounds.size.height > px(0.));
            }
        })
        .unwrap();
        cx.update(|cx| {
            design::set_interface_scale(sovra_core::models::InterfaceScale::default(), cx);
        });
    }
}
