//! The live work log at the top of a reply (AG-UI `ACTIVITY_SNAPSHOT`,
//! `activityType: "sovra.work"`): what Sovra is doing right now, the helpers
//! working side by side, and each step with how long it took. Open while the
//! reply is being worked on, folded to one line once it is done.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, button::ButtonVariants as _, h_flex,
    shimmer::ShimmerText, v_flex,
};
use gpui_kit::{
    Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::work::{StepKind, StepState, WorkActivity, WorkHelper, WorkStep, format_duration};

use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

/// Steps shown while the work is live; the newest ones, so the log never jumps.
const LIVE_STEPS: usize = 6;

/// `toggled` is the reader's choice for message `index`: it flips the default
/// (open while working, folded when finished), like the reasoning disclosure.
pub(crate) fn render(
    work: &WorkActivity,
    index: usize,
    toggled: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let (primary, success, danger) = (theme.primary, theme.success, theme.danger);
    let live = !work.finished;
    let open = live != toggled;
    let (icon, accent) = match (live, work.stopped) {
        (true, _) => (IconName::LoaderCircle, primary),
        (false, false) => (IconName::CircleCheck, success),
        (false, true) => (IconName::CircleAlert, danger),
    };
    let total = work.total_steps();
    let mut caption = format!("{total} {}", if total == 1 { "step" } else { "steps" });
    if work.elapsed_ms > 0 {
        caption.push_str(" · ");
        caption.push_str(&format_duration(work.elapsed_ms));
    }
    let status = work.status.clone();
    let header = h_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .items_center()
        .child(Icon::new(icon).small().text_color(accent))
        .child(
            div()
                .id(("work-log-status", index))
                .test_support()
                .flex_1()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .text_ellipsis()
                .child(if live {
                    ShimmerText::new(status).into_any_element()
                } else {
                    div().child(status).into_any_element()
                }),
        );
    // Counts and the toggle sit on their own wrapping line, so a large text size
    // on a narrow window never pushes the toggle out of the card.
    let mut meta = h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap(t.space.small)
        .items_center()
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(caption),
        );
    if !work.steps.is_empty() {
        let label = if open { "Hide steps" } else { "Show steps" };
        meta = meta.child(
            design::button(("work-log-toggle", index), label, cx)
                .ghost()
                .xsmall()
                .accessibility_label(label)
                .icon(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .on_click(cx.listener(move |shell, _, _, cx| {
                    if !shell.chat.work_toggled.remove(&index) {
                        shell.chat.work_toggled.insert(index);
                    }
                    shell.chat.remeasure_message(index);
                    cx.notify();
                })),
        );
    }
    let mut card = v_flex()
        .id(("work-log", index))
        .test_support()
        .aria_label(SharedString::from(format!("Progress: {}", work.status)))
        .w_full()
        .min_w_0()
        .max_w(t.layout.card_max_width)
        .sovra_surface(Surface::Matte, cx)
        .px(t.space.medium)
        .py(t.space.small)
        .gap(t.space.small)
        .child(header)
        .child(meta);
    if !open {
        return card;
    }
    if !work.helpers.is_empty() {
        card = card.child(
            h_flex()
                .id(("work-log-helpers", index))
                .test_support()
                .w_full()
                .flex_wrap()
                .gap(t.space.small)
                .items_start()
                .children(
                    work.helpers
                        .iter()
                        .enumerate()
                        .map(|(lane, helper)| render_helper(helper, index, lane, cx)),
                ),
        );
    }
    let shown = if live {
        work.steps.len().saturating_sub(LIVE_STEPS)
    } else {
        0
    };
    let earlier = u64::from(work.hidden_steps) + shown as u64;
    let mut list = v_flex()
        .id(("work-log-steps", index))
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.hair);
    if earlier > 0 {
        list = list.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!(
                    "{earlier} earlier {} not shown",
                    if earlier == 1 { "step" } else { "steps" }
                )),
        );
    }
    list = list.children(
        work.steps
            .iter()
            .enumerate()
            .skip(shown)
            .map(|(step_index, step)| render_step(step, index, step_index, cx)),
    );
    card.child(list)
}

fn state_colour(state: StepState, cx: &Context<Shell>) -> Hsla {
    let theme = cx.theme();
    match state {
        StepState::Running => theme.primary,
        StepState::Done => theme.success,
        StepState::Failed => theme.danger,
    }
}

fn kind_icon(kind: StepKind) -> IconName {
    match kind {
        StepKind::Search => IconName::Search,
        StepKind::Read => IconName::BookOpen,
        StepKind::Write => IconName::PenLine,
        StepKind::Save => IconName::Bookmark,
        StepKind::Run => IconName::SquareTerminal,
        StepKind::Remember => IconName::Brain,
        StepKind::Message => IconName::MessageSquare,
        StepKind::Plan => IconName::ListChecks,
        StepKind::Ask => IconName::Users,
        StepKind::Other => IconName::Wrench,
    }
}

fn render_helper(
    helper: &WorkHelper,
    log: usize,
    lane: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let accent = state_colour(helper.state, cx);
    let state = match helper.state {
        StepState::Running => "Working…",
        StepState::Done => "Done",
        StepState::Failed => "Couldn't finish",
    };
    let mut lane_card = v_flex()
        .id(SharedString::from(format!("work-helper-{log}-{lane}")))
        .test_support()
        .aria_label(SharedString::from(format!("{}: {state}", helper.name)))
        .prefer_width(t.layout.kanban_lane_width)
        .gap(t.space.hair)
        .px(t.space.small)
        .py(t.space.compact)
        .rounded(t.radius.card)
        .border_1()
        .border_color(t.material_border)
        .bg(t.subtle)
        .child(
            h_flex()
                .gap(t.space.compact)
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .flex_shrink_0()
                        .size(t.space.small)
                        .rounded(t.radius.pill)
                        .bg(accent),
                )
                .child(
                    div()
                        .min_w_0()
                        .sovra_text(TextRole::Caption, cx)
                        .text_ellipsis()
                        .child(helper.name.clone()),
                ),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(accent)
                .child(state),
        );
    if let Some(doing) = &helper.doing {
        lane_card = lane_card.child(
            div()
                .min_w_0()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .text_ellipsis()
                .child(doing.clone()),
        );
    }
    if helper.steps > 0 {
        lane_card = lane_card.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!(
                    "{} {}",
                    helper.steps,
                    if helper.steps == 1 { "step" } else { "steps" }
                )),
        );
    }
    lane_card
}

fn render_step(
    step: &WorkStep,
    log: usize,
    step_index: usize,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let accent = state_colour(step.state, cx);
    let trailing = match (step.state, step.duration_ms()) {
        (StepState::Running, _) => "Working…".to_owned(),
        (StepState::Failed, _) => "Didn't work".to_owned(),
        (StepState::Done, Some(ms)) => format_duration(ms),
        (StepState::Done, None) => "Done".to_owned(),
    };
    let mut row = h_flex()
        .id(SharedString::from(format!("work-step-{log}-{step_index}")))
        .test_support()
        .aria_label(SharedString::from(format!("{}: {trailing}", step.label)))
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .items_center();
    if !step.helper.is_empty() {
        row = row.pl(t.space.medium);
    }
    row.child(Icon::new(kind_icon(step.kind)).xsmall().text_color(
        if step.state == StepState::Done {
            t.text_secondary
        } else {
            accent
        },
    ))
    .child(
        div()
            .flex_1()
            .min_w_0()
            .sovra_text(TextRole::Caption, cx)
            .text_ellipsis()
            .child(step.label.clone()),
    )
    .child(
        div()
            .flex_shrink_0()
            .sovra_text(TextRole::Caption, cx)
            .text_color(accent)
            .child(trailing),
    )
}
