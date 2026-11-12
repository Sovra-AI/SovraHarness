//! Native GPUI renderer for AG-UI surfaces (`AgUiWidget`).
//!
//! Renders the "Building your personal AI..." live progress checklist card matching the
//! reference screenshot, as well as `ClarifyChips` and `SurfaceCard` widgets emitted
//! by `adk-ui`.

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{AgUiWidget, BuildStep, BuildStepState, ClarifyChip};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_agui_widget(
    widget: &AgUiWidget,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    match widget {
        AgUiWidget::CodeWriting { .. } => design::button(("show-code", index), "Show code", cx)
            .on_click(cx.listener(|shell, _, _, cx| shell.show_code_panel(true, cx)))
            .into_any_element(),
        AgUiWidget::Components { components, .. } => {
            crate::native_surface::render_surface(components, index, cx)
        }
        AgUiWidget::BuildProgress {
            title,
            subtitle,
            steps,
            footer_quote,
        } => render_build_progress(title, subtitle, steps, footer_quote, cx).into_any_element(),
        AgUiWidget::ClarifyChips { question, chips } => {
            render_clarify_chips(question, chips, index, cx).into_any_element()
        }
        AgUiWidget::SurfaceCard {
            surface_id,
            title,
            summary,
        } => render_surface_card(surface_id, title, summary, cx).into_any_element(),
        AgUiWidget::TeamActivity(activity) => {
            render_team_activity(activity, index, cx).into_any_element()
        }
        AgUiWidget::ResearchBoard(board) => {
            render_research_board(board, index, cx).into_any_element()
        }
        AgUiWidget::ArtifactSaved {
            artifact_id,
            title,
            artifact_kind,
        } => {
            render_artifact_saved(artifact_id, title, *artifact_kind, index, cx).into_any_element()
        }
    }
}

/// In-chat research kanban (waiting / working / done) for a deep-research run.
fn render_research_board(
    board: &sovra_core::ResearchBoard,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    use sovra_core::{ResearchGoalState, ResearchPhase};
    let t = design::tokens(cx);
    let theme = cx.theme();
    let (primary, success, muted, danger) = (
        theme.primary,
        theme.success,
        theme.muted_foreground,
        theme.danger,
    );
    let lanes = board.lanes();
    let heading = match board.phase {
        ResearchPhase::Planning => "Research · planning".to_owned(),
        ResearchPhase::Researching => "Research · working".to_owned(),
        ResearchPhase::Writing => "Research · writing document".to_owned(),
        ResearchPhase::Done => "Research · done".to_owned(),
    };
    let goal_count = board.goals.len();
    let has_failed = board
        .goals
        .iter()
        .any(|goal| goal.state == ResearchGoalState::Failed);
    let mut card = v_flex()
        .id(("research-board", index))
        .test_support()
        .w_full()
        .min_w_0()
        .max_w(t.layout.card_max_width)
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            h_flex()
                .gap(t.space.small)
                .items_center()
                .child(Icon::new(IconName::Search).small().text_color(primary))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .min_w_0()
                        .text_ellipsis()
                        .child(heading),
                ),
        )
        .child(
            div()
                .id(("research-board-title", index))
                .test_support()
                .sovra_text(TextRole::Body, cx)
                .min_w_0()
                .child(board.title.clone()),
        )
        .child(
            div()
                .id(("research-board-status", index))
                .test_support()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(board.status.clone()),
        )
        .child(
            h_flex()
                .id(("research-board-lanes", index))
                .test_support()
                .w_full()
                .gap(t.space.small)
                .items_start()
                .children([
                    render_research_lane("waiting", "Waiting", &lanes.waiting, muted, index, cx),
                    render_research_lane("working", "Working", &lanes.working, primary, index, cx),
                    render_research_lane("done", "Done", &lanes.done, success, index, cx),
                ]),
        );
    if goal_count > 0 {
        card = card.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!("{goal_count} goals · helpers work side by side")),
        );
    }
    if has_failed {
        card = card.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(danger)
                .child("Some goals could not finish — the document uses what was found."),
        );
    }
    card
}

fn render_research_lane(
    lane_id: &'static str,
    label: &'static str,
    goals: &[&sovra_core::ResearchGoal],
    accent: gpui_kit::Hsla,
    board_index: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    use sovra_core::ResearchGoalState;
    let t = design::tokens(cx);
    v_flex()
        .id(gpui_kit::SharedString::from(format!(
            "research-lane-{lane_id}-{board_index}"
        )))
        .test_support()
        .flex_1()
        .min_w_0()
        .gap(t.space.compact)
        .child(
            h_flex()
                .gap(t.space.compact)
                .items_center()
                .child(
                    div()
                        .size(t.layout.icon_small)
                        .rounded(t.radius.pill)
                        .bg(accent),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!("{label} · {}", goals.len())),
                ),
        )
        .children(goals.iter().enumerate().map(|(goal_index, goal)| {
            let state_label = match goal.state {
                ResearchGoalState::Waiting => "Waiting",
                ResearchGoalState::Working => "Looking into…",
                ResearchGoalState::Done => "Done",
                ResearchGoalState::Failed => "Couldn't finish",
            };
            v_flex()
                .id(gpui_kit::SharedString::from(format!(
                    "research-goal-{lane_id}-{board_index}-{goal_index}"
                )))
                .test_support()
                .w_full()
                .min_w_0()
                .gap(t.space.hair)
                .px(t.space.small)
                .py(t.space.compact)
                .rounded(t.radius.card)
                .border_1()
                .border_color(t.material_border)
                .bg(t.subtle)
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .min_w_0()
                        .child(goal.text.clone()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(accent)
                        .child(state_label),
                )
        }))
}

/// The live team card (AG-UI `ACTIVITY_SNAPSHOT`, `activityType: "sovra.team"`):
/// who is on it, who is speaking, who passed, in plain words.
fn render_team_activity(
    activity: &sovra_core::teams::TeamActivity,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    use sovra_core::teams::SpeakerState;
    let t = design::tokens(cx);
    let theme = cx.theme();
    let (primary, success, muted, danger) = (
        theme.primary,
        theme.success,
        theme.muted_foreground,
        theme.danger,
    );
    v_flex()
        .id(("team-activity", index))
        .test_support()
        .w_full()
        .max_w(t.layout.card_max_width)
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            h_flex()
                .gap(t.space.small)
                .items_center()
                .child(Icon::new(IconName::Users).small().text_color(primary))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(if activity.finished {
                            "Your team".to_owned()
                        } else {
                            format!(
                                "Your team · round {} of {}",
                                activity.round, activity.max_rounds
                            )
                        }),
                ),
        )
        .child(
            div()
                .id(("team-activity-status", index))
                .test_support()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(activity.status.clone()),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .children(activity.speakers.iter().map(|row| {
                    let (label, color) = match row.state {
                        SpeakerState::Waiting => ("Waiting", muted),
                        SpeakerState::Speaking => ("Answering…", primary),
                        SpeakerState::Done => ("Done", success),
                        SpeakerState::Passed => ("Nothing to add", muted),
                        SpeakerState::Failed => ("Couldn't answer", danger),
                    };
                    h_flex()
                        .gap(t.space.compact)
                        .items_center()
                        .px(t.space.small)
                        .py(t.space.compact)
                        .rounded(t.radius.pill)
                        .border_1()
                        .border_color(t.material_border)
                        .child(crate::chat_view::member_avatar(
                            &row.icon,
                            t.layout.icon_medium,
                            cx,
                        ))
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .child(row.name.clone()),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(color)
                                .child(label),
                        )
                })),
        )
}

/// In-chat listing of recalled saved details, driven from parsed citation tokens.
pub(crate) fn render_memory_recall_card(
    refs: &[sovra_core::MemoryCitationRef],
    message_index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    use sovra_core::{MemoryAction, MemoryCitationTarget};
    let t = design::tokens(cx);
    let primary = cx.theme().primary;
    v_flex()
        .id(("memory-recall-card", message_index))
        .test_support()
        .aria_label("Saved details")
        .w_full()
        .min_w_0()
        .max_w(t.layout.card_max_width)
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            h_flex()
                .gap(t.space.small)
                .items_center()
                .child(Icon::new(IconName::Bookmark).small().text_color(primary))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("Saved details"),
                ),
        )
        .child(
            div()
                .id(("memory-recall-status", message_index))
                .test_support()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("Open a row to check the original."),
        )
        .children(refs.iter().enumerate().map(|(number, item)| {
            let action = match &item.target {
                MemoryCitationTarget::Fact(id) => MemoryAction::Inspect(id.clone()),
                MemoryCitationTarget::Source(id) => MemoryAction::InspectSource(id.clone()),
            };
            let title = if item.preview.is_empty() {
                format!("Saved source {}", number + 1)
            } else {
                item.preview.clone()
            };
            let open_label = format!("Open saved source {}", number + 1);
            h_flex()
                .id(("memory-recall-row", message_index * 10 + number))
                .test_support()
                .aria_label(title.clone())
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .px(t.space.small)
                .py(t.space.compact)
                .rounded(t.radius.card)
                .border_1()
                .border_color(t.material_border)
                .bg(t.subtle)
                .child(
                    div()
                        .id(("memory-recall-preview", message_index * 10 + number))
                        .test_support()
                        .aria_label(title.clone())
                        .flex_1()
                        .min_w_0()
                        .sovra_text(TextRole::Caption, cx)
                        .child(title),
                )
                .child(
                    design::button(
                        (
                            gpui_kit::SharedString::from(format!(
                                "memory-citation-{message_index}"
                            )),
                            number,
                        ),
                        "Open",
                        cx,
                    )
                    .small()
                    .accessibility_label(open_label)
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.open_memory_source(action.clone(), cx);
                    })),
                )
        }))
}

/// An agent saved something with `save_artifact`: a card to open it.
fn render_artifact_saved(
    artifact_id: &str,
    title: &str,
    kind: sovra_core::artifacts::ArtifactKind,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let id = artifact_id.to_owned();
    h_flex()
        .id(("artifact-saved", index))
        .test_support()
        .w_full()
        .max_w(t.layout.card_max_width)
        .flex_wrap()
        .items_center()
        .gap(t.space.small)
        .p(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .child(Icon::new(crate::artifacts_view::kind_icon(kind)).text_color(cx.theme().primary))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .text_ellipsis()
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!("{} saved to Artifacts", kind.label())),
                ),
        )
        .child(
            design::button(("artifact-saved-open", index), "Open", cx)
                .small()
                .on_click(cx.listener(move |shell, _, _, cx| shell.open_artifact(&id, cx))),
        )
}

fn render_build_progress(
    title: &str,
    subtitle: &str,
    steps: &[BuildStep],
    footer_quote: &str,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let all_done = steps.iter().all(|s| s.state == BuildStepState::Completed);

    v_flex()
        .id("agui-build-progress-card")
        .test_support()
        .w_full()
        .max_w(t.layout.card_max_width)
        .sovra_surface(Surface::Matte, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(
            h_flex()
                .items_center()
                .gap(t.space.small)
                .child(progress_ring_badge(all_done, cx))
                .child(
                    v_flex()
                        .gap(t.space.hair)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(title.to_owned()),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(subtitle.to_owned()),
                        ),
                ),
        )
        .child(div().h(t.layout.hairline).w_full().bg(theme.border))
        .child(
            v_flex()
                .gap(t.space.compact)
                .children(steps.iter().map(|step| render_step_row(step, cx))),
        )
        .child(
            div()
                .w_full()
                .pt(t.space.compact)
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(footer_quote.to_owned()),
        )
}

fn progress_ring_badge(all_done: bool, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let ring_color = if all_done {
        t.accent.green
    } else {
        t.accent.violet
    };
    let icon = if all_done {
        IconName::Check
    } else {
        IconName::LoaderCircle
    };
    h_flex()
        .size(t.layout.control_height)
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(t.radius.pill)
        .border_2()
        .border_color(ring_color)
        .bg(t.subtle)
        .child(Icon::new(icon).small().text_color(ring_color))
}

fn render_step_row(step: &BuildStep, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (icon_name, badge_color, text_color) = match step.state {
        BuildStepState::Completed => (IconName::Check, t.accent.violet, cx.theme().foreground),
        BuildStepState::InProgress => (IconName::LoaderCircle, t.accent.indigo, t.text_secondary),
        BuildStepState::Pending => (
            IconName::Circle,
            cx.theme().muted_foreground,
            cx.theme().muted_foreground,
        ),
    };

    h_flex()
        .items_center()
        .gap(t.space.small)
        .py(t.space.hair)
        .child(
            h_flex()
                .size(t.layout.icon_small)
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(t.radius.pill)
                .bg(t.nav_active_background)
                .child(Icon::new(icon_name).xsmall().text_color(badge_color)),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(text_color)
                .child(step.label.clone()),
        )
}

fn render_clarify_chips(
    question: &str,
    chips: &[ClarifyChip],
    widget_idx: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .sovra_surface(Surface::Subtle, cx)
        .p(t.space.small)
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(question.to_owned()),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.compact)
                .children(chips.iter().enumerate().map(|(chip_idx, chip)| {
                    let prompt = chip.prompt.clone();
                    let label = chip.label.clone();
                    div()
                        .id(("agui-chip", widget_idx * 100 + chip_idx))
                        .cursor_pointer()
                        .px(t.space.small)
                        .py(t.space.compact)
                        .rounded(t.radius.pill)
                        .bg(t.nav_active_background)
                        .sovra_text(TextRole::Caption, cx)
                        .child(label)
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.send_prompt_text(&prompt, window, cx);
                        }))
                })),
        )
}

fn render_surface_card(
    surface_id: &str,
    title: &str,
    summary: &str,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .sovra_surface(Surface::Subtle, cx)
        .p(t.space.small)
        .gap(t.space.compact)
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(cx.theme().link)
                        .child(format!("ag_ui · {surface_id}")),
                ),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(summary.to_owned()),
        )
}
