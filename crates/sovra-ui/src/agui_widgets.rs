//! Native GPUI renderer for AG-UI surfaces (`AgUiWidget`).
//!
//! Renders the "Building your personal AI..." live progress checklist card matching the
//! reference screenshot, as well as `ClarifyChips` and `SurfaceCard` widgets emitted
//! by `adk-ui`.

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, px,
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
        AgUiWidget::Components {
            surface_id,
            version,
            components,
        } => crate::native_surface::render_surface(surface_id, *version, components, index, cx),
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
    }
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
        .max_w(px(560.))
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
                        .gap(px(2.))
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
        .child(div().h(px(1.)).w_full().bg(theme.border))
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
        .size(px(36.))
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
        .py(px(2.))
        .child(
            h_flex()
                .size(px(20.))
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
                        .py(px(4.))
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
        .gap(px(4.))
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
