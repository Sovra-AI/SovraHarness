//! Top guided walkthrough and setup coach bar:
//! 1. When the 5-step Interactive System Tour (`TourStep`) is running, displays a
//!    spotlight coach banner explaining the current tab with `Previous`, `Next Step →`,
//!    a step-specific helper action, and `End Tour`.
//! 2. When setup is incomplete (`!has_active_llm` after models load), displays a
//!    concise state-aware setup strip showing ONLY the next remaining step
//!    (e.g. if the API key is already set, it only prompts to pick a model).

use gpui_kit::component::{Sizable as _, button::ButtonVariants as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::models::SetupStage;
use sovra_core::{NavTab, TourStep, WorkspaceState};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{caption, pill};
use crate::shell::Shell;

pub(crate) fn render_onboarding_bar(
    state: &WorkspaceState,
    narrow: bool,
    cx: &mut Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    if let Some(step) = state.tour_step {
        return Some(render_tour_coach_bar(step, narrow, cx).into_any_element());
    }
    if narrow || matches!(state.active_tab, NavTab::Models | NavTab::Settings) {
        // On minimum 640x420 windows, keep vertical space unobstructed unless the user
        // explicitly opens the interactive tour.
        return None;
    }
    if state.models_loaded && !state.has_active_llm {
        return Some(
            render_setup_coach_strip(&state.setup_stage, state.llm_setup_hint.as_deref(), cx)
                .into_any_element(),
        );
    }
    None
}

fn render_tour_coach_bar(
    step: TourStep,
    narrow: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let step_num = step.step_number();
    let badge_text = format!("Quick tour · {step_num} of 5");
    let next_label = match step.next() {
        Some(next) => format!("Next: {}", next.title()),
        None => "Finish tour".to_owned(),
    };

    let mut actions = h_flex().flex_wrap().items_center().gap(t.space.compact);
    if step == TourStep::WelcomeChat {
        actions = actions.child(
            design::button("tour-try-sample-btn", "Try a question", cx)
                .small()
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell.fill_composer_draft(
                        "Explain what Sovra can help me do in 3 simple bullet points.",
                        window,
                        cx,
                    );
                })),
        );
    } else if step == TourStep::AutomaticRouting {
        actions = actions.child(
            design::button("tour-try-sim-btn", "Try routing simulation", cx)
                .small()
                .on_click(cx.listener(|shell, _, _, cx| {
                    if !shell.my_ais_advanced_open {
                        shell.toggle_my_ais_advanced(cx);
                    }
                    shell.simulate_route_from_input(cx);
                })),
        );
    }
    if step.previous().is_some() {
        actions = actions.child(
            design::button("tour-prev-btn", "← Back", cx)
                .small()
                .ghost()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.prev_system_tour_step(cx);
                })),
        );
    }
    actions = actions
        .child(
            design::primary_button("tour-next-btn", next_label, cx)
                .small()
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell.next_system_tour_step(window, cx);
                })),
        )
        .child(
            design::button("tour-skip-btn", "End tour", cx)
                .small()
                .ghost()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.end_system_tour(cx);
                })),
        );

    v_flex()
        .id("sovra-tour-coach-bar")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .px(if narrow {
            t.space.small
        } else {
            t.space.medium
        })
        .py(t.space.small)
        .gap(t.space.compact)
        .sovra_surface(Surface::Subtle, cx)
        .border_b_1()
        .border_color(t.accent.violet)
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .justify_between()
                .items_center()
                .gap(t.space.small)
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(t.space.compact)
                        .child(pill(badge_text, cx))
                        .child(
                            div()
                                .id("tour-step-title")
                                .test_support()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(step.title()),
                        ),
                )
                .child(actions),
        )
        .children((!narrow).then(|| caption(step.explanation(), cx)))
}

fn render_setup_coach_strip(
    stage: &SetupStage,
    hint: Option<&str>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let (badge, message, cta_id, cta_label, target) = match stage {
        SetupStage::NeedsModelSelection {
            connected_service, ..
        } => (
            "STEP 2 OF 2",
            hint.map_or_else(
                || {
                    format!(
                        "{connected_service} is connected. Now pick the AI you want to talk to."
                    )
                },
                str::to_owned,
            ),
            "setup-strip-pick-model",
            "Pick your AI →",
            "recommended-models",
        ),
        SetupStage::NeedsModelKey {
            connection_name, ..
        } => (
            "ALMOST READY",
            hint.map_or_else(
                || format!("Your AI is chosen. Add your {connection_name} key to start chatting."),
                str::to_owned,
            ),
            "setup-strip-add-key",
            "Add your key →",
            "selected-key",
        ),
        SetupStage::Loading | SetupStage::NeedsConnection | SetupStage::Ready { .. } => (
            "STEP 1 OF 2",
            "Connect an AI service to start chatting. A free OpenRouter key is the easiest."
                .to_owned(),
            "setup-strip-connect",
            "Connect an AI service →",
            "openrouter-key",
        ),
    };

    h_flex()
        .id("sovra-setup-coach-strip")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .flex_wrap()
        .justify_between()
        .items_center()
        .gap(t.space.small)
        .px(t.space.medium)
        .py(t.space.compact)
        .sovra_surface(Surface::Subtle, cx)
        .border_b_1()
        .border_color(t.material_border)
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.compact)
                .child(pill(badge, cx))
                .child(caption(message, cx)),
        )
        .child(
            h_flex()
                .items_center()
                .gap(t.space.compact)
                .child(
                    design::primary_button(cta_id, cta_label, cx)
                        .small()
                        .on_click(cx.listener(move |shell, _, window, cx| {
                            shell.handle_llm_setup_action(target, window, cx);
                        })),
                )
                .child(
                    design::button("setup-strip-tour-btn", "System Tour", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.start_system_tour(cx);
                        })),
                ),
        )
}
