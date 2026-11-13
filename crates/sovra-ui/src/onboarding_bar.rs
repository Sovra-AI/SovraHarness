//! Top guided walkthrough and setup coach bar:
//! 1. While the guided tour (`TourStep`) runs, shows a card above the page it is
//!    explaining: progress, what the page is for in everyday words, one thing to try,
//!    step-specific buttons (the AI choices on "Give Sovra a brain"), Back, Next and
//!    End tour. It starts on its own after a newcomer first signs in.
//! 2. When setup is incomplete (`!has_active_llm` after models load), displays a
//!    concise state-aware setup strip showing ONLY the next remaining step
//!    (e.g. if the API key is already set, it only prompts to pick a model).

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, button::ButtonVariants as _, h_flex, v_flex,
};
use gpui_kit::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div, relative,
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
        return Some(render_tour_coach_bar(state, step, narrow, cx).into_any_element());
    }
    if narrow
        || matches!(
            state.active_tab,
            NavTab::Models | NavTab::Settings | NavTab::Connections
        )
    {
        // On minimum 640x420 windows, keep vertical space unobstructed unless the user
        // explicitly opens the interactive tour.
        return None;
    }
    // An empty chat already shows the first step as a card; one thing to press is enough.
    if state.active_tab == NavTab::Chat && state.messages.is_empty() {
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

/// Most of the main column the tour card may cover, so the page it explains stays
/// visible beside it even in a short window with large text.
const TOUR_CARD_SHARE: f32 = 0.55;

fn step_icon(step: TourStep) -> IconName {
    match step {
        TourStep::Welcome => IconName::Sparkles,
        TourStep::ConnectAi => IconName::Brain,
        TourStep::AskAnything => IconName::MessageCircle,
        TourStep::Skills => IconName::LayoutDashboard,
        TourStep::Knowledge => IconName::BookOpen,
        TourStep::Connections => IconName::Plug,
        TourStep::Team => IconName::Users,
        TourStep::Schedules => IconName::Calendar,
        TourStep::SavedWork => IconName::Archive,
        TourStep::Groups => IconName::House,
        TourStep::MakeItYours => IconName::Settings,
    }
}

fn step_tone(step: TourStep, cx: &App) -> design::TileTone {
    let tiles = &design::tokens(cx).tiles;
    tiles.categories[(step.step_number() - 1) % tiles.categories.len()]
}

/// The tour card: where you are, what this page is for, one thing to try, and
/// the way on. It sits above the page it explains so both stay in view.
fn render_tour_coach_bar(
    state: &WorkspaceState,
    step: TourStep,
    narrow: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let step_num = step.step_number();
    let total = TourStep::COUNT;
    let tone = step_tone(step, cx);

    let progress = h_flex()
        .id("tour-progress")
        .test_support()
        .aria_label(format!("Step {step_num} of {total}"))
        .w_full()
        .gap(t.space.hair)
        .children(TourStep::ALL.iter().map(|other| {
            div()
                .flex_1()
                .h(t.layout.tour_progress)
                .rounded(t.radius.pill)
                .bg(if other.step_number() <= step_num {
                    tone.foreground
                } else {
                    t.subtle
                })
        }));

    let heading = h_flex()
        .w_full()
        .items_center()
        .gap(t.space.small)
        .child(crate::skill_library::tile(step_icon(step), tone, cx))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .id("tour-step-count")
                        .test_support()
                        .aria_label(format!("Guided tour · Step {step_num} of {total}"))
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(format!("Guided tour · Step {step_num} of {total}")),
                )
                .child(
                    div()
                        .id("tour-step-title")
                        .test_support()
                        .aria_label(step.title())
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(step.title()),
                ),
        );

    let explanation = div()
        .id("tour-step-explanation")
        .test_support()
        .aria_label(step.explanation())
        .w_full()
        .max_w(t.layout.reading_width)
        .sovra_text(TextRole::Body, cx)
        .child(step.explanation());

    let tip = step.try_this().map(|tip| {
        h_flex()
            .id("tour-step-tip")
            .test_support()
            .w_full()
            .max_w(t.layout.reading_width)
            .items_start()
            .gap(t.space.compact)
            .px(t.space.small)
            .py(t.space.compact)
            .rounded(t.radius.card)
            .bg(tone.background)
            .child(
                Icon::new(IconName::Info)
                    .size(t.layout.icon_small)
                    .flex_shrink_0()
                    .text_color(tone.foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(tone.foreground)
                    .child(tip),
            )
    });

    let choices = (step == TourStep::ConnectAi).then(|| render_ai_choices(state, cx));

    // Back, Next and End tour stay outside the scrolling part, so the way on is
    // always in view however much a step has to say.
    let body = v_flex()
        .id("tour-card-body")
        .test_support()
        .w_full()
        .min_h_0()
        .max_h(t.layout.tour_card_max_height)
        .overflow_y_scroll()
        .gap(t.space.small)
        .child(progress)
        .child(heading)
        .child(explanation)
        .children(choices)
        .children(tip);

    v_flex()
        .id("sovra-tour-coach-bar")
        .test_support()
        .aria_label(format!(
            "Guided tour, step {step_num} of {total}: {}",
            step.title()
        ))
        .w_full()
        .flex_shrink_0()
        .max_h(relative(TOUR_CARD_SHARE))
        .px(if narrow {
            t.space.small
        } else {
            t.space.medium
        })
        .py(t.space.small)
        .gap(t.space.small)
        .sovra_surface(Surface::Subtle, cx)
        .border_b_1()
        .border_color(tone.foreground)
        .child(body)
        .child(render_tour_actions(step, cx))
}

/// The two ways to give Sovra an AI, each with the button that opens it. Once one
/// works this says so, so nobody sets up a second by mistake.
fn render_ai_choices(state: &WorkspaceState, cx: &mut Context<Shell>) -> gpui_kit::AnyElement {
    let t = design::tokens(cx);
    if state.has_active_llm {
        let name = state
            .routing
            .global_model
            .as_ref()
            .map_or_else(String::new, |key| format!(" ({})", key.model_id));
        return h_flex()
            .id("tour-ai-ready")
            .test_support()
            .items_center()
            .gap(t.space.compact)
            .child(
                Icon::new(IconName::CircleCheck)
                    .size(t.layout.icon_small)
                    .text_color(cx.theme().success),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .sovra_text(TextRole::Body, cx)
                    .child(format!(
                        "Sovra already has an AI{name}. You are ready: press Next."
                    )),
            )
            .into_any_element();
    }
    let tiles = &t.tiles;
    let online = choice_card(
        "tour-choice-openrouter",
        IconName::Cloud,
        tiles.categories[1],
        "OpenRouter: online",
        "1. Press Open OpenRouter and make a free account.\n2. Press Create key and copy it.\n3. Come back and press Paste my key.",
        [
            (
                "tour-open-openrouter",
                "Open OpenRouter",
                "open-openrouter-web",
                false,
            ),
            (
                "tour-openrouter-key",
                "Paste my key",
                "openrouter-key",
                true,
            ),
        ],
        cx,
    );
    let local = choice_card(
        "tour-choice-ollama",
        IconName::Cpu,
        tiles.categories[2],
        "Ollama: on this computer",
        "1. Press Get Ollama, then download and install it.\n2. Come back and press Set up Ollama.\n3. Sovra helps you download a free AI.",
        [
            ("tour-get-ollama", "Get Ollama", "open-ollama-web", false),
            ("tour-setup-ollama", "Set up Ollama", "ollama", true),
        ],
        cx,
    );
    h_flex()
        .id("tour-ai-choices")
        .test_support()
        .w_full()
        .flex_wrap()
        .items_stretch()
        .gap(t.space.small)
        .child(online)
        .child(local)
        .into_any_element()
}

fn choice_card(
    id: &'static str,
    icon: IconName,
    tone: design::TileTone,
    title: &'static str,
    steps: &'static str,
    buttons: [(&'static str, &'static str, &'static str, bool); 2],
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut actions = h_flex().flex_wrap().items_center().gap(t.space.compact);
    for (button_id, label, target, primary) in buttons {
        let button = if primary {
            design::primary_button(button_id, label, cx)
        } else {
            design::button(button_id, label, cx)
        };
        actions = actions.child(button.small().on_click(cx.listener(
            move |shell, _, window, cx| {
                shell.handle_llm_setup_action(target, window, cx);
            },
        )));
    }
    v_flex()
        .id(id)
        .test_support()
        .flex_1()
        .min_w(t.layout.card_min_width)
        .gap(t.space.compact)
        .p(t.space.small)
        .rounded(t.radius.card)
        .sovra_surface(Surface::Glass, cx)
        .border_1()
        .border_color(t.material_border)
        .child(
            h_flex()
                .items_center()
                .gap(t.space.compact)
                .child(
                    Icon::new(icon)
                        .size(t.layout.icon_small)
                        .text_color(tone.foreground),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(title),
                ),
        )
        .children(steps.lines().map(|line| caption(line, cx)))
        .child(actions)
}

fn render_tour_actions(step: TourStep, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    // The way on never shrinks or scrolls away.
    let next_label = match step.next() {
        Some(_) => "Next →".to_owned(),
        None => "Start using Sovra".to_owned(),
    };
    let mut actions = h_flex()
        .w_full()
        .flex_shrink_0()
        .flex_wrap()
        .items_center()
        .gap(t.space.compact);
    if step == TourStep::AskAnything {
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
    }
    actions = actions.child(div().flex_1());
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
    actions
        .child(
            design::button("tour-skip-btn", "End tour", cx)
                .small()
                .ghost()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.end_system_tour(cx);
                })),
        )
        .child(
            design::primary_button("tour-next-btn", next_label, cx)
                .small()
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell.next_system_tour_step(window, cx);
                })),
        )
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
                || format!("{connected_service} is set up. Now pick the AI you want to talk to."),
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
            "Connect an AI service to start chatting. OpenRouter offers free and paid AI choices."
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
                    design::button("setup-strip-tour-btn", "Quick tour", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.start_system_tour(cx);
                        })),
                ),
        )
}
