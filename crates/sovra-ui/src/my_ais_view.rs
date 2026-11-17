//! The saved chat AI, and three preview choices of equal height.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    Context, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{Command, NavTab, SelectionMode, models::SetupStage};

use crate::RuntimeClient;
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_my_ais_view(
    models: &Entity<crate::ModelsView>,
    preview_mode: SelectionMode,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let models = models.read(cx).state();
    let stage = models.setup_stage();
    let ready = matches!(stage, SetupStage::Ready { .. });
    let loading = matches!(stage, SetupStage::Loading);
    let name = models.preferences.active.as_ref().map_or_else(
        || "Choose your AI".to_owned(),
        |key| {
            models
                .model(key)
                .map_or_else(|| key.model_id.clone(), |model| model.name.clone())
        },
    );
    let description = if loading {
        "Loading your AI…".to_owned()
    } else {
        models
            .setup_status_hint()
            .unwrap_or_else(|| "This AI answers your chat messages.".to_owned())
    };
    let action = match stage {
        SetupStage::Ready { .. } => "Change AI",
        SetupStage::NeedsModelKey { .. } => "Connect AI",
        SetupStage::Loading
        | SetupStage::NeedsConnection
        | SetupStage::NeedsModelSelection { .. } => "Choose AI",
    };
    let choose = if ready {
        design::button("open-models-catalog-btn", action, cx)
    } else {
        design::primary_button("open-models-catalog-btn", action, cx)
    }
    .disabled(loading)
    .on_click(cx.listener(move |shell, _, window, cx| {
        shell.navigate_to_tab(NavTab::Models, window, cx);
        if ready {
            shell.models().update(cx, |models, cx| {
                models.changing_active = true;
                models
                    .change_search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            });
        }
    }));

    let mut actions = h_flex().w_full().flex_wrap().gap(t.space.small);
    if ready {
        actions = actions.child(
            design::primary_button("my-ais-chat-btn", "Chat", cx).on_click(cx.listener(
                |shell, _, window, cx| {
                    shell.select_tab(NavTab::Chat, cx);
                    shell
                        .composer_input
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                },
            )),
        );
    }
    actions = actions.child(choose);

    v_flex()
        .id("sovra-my-ais-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p(t.space.large)
        .child(
            v_flex()
                .id("my-ais-column")
                .test_support()
                .w_full()
                .min_w_0()
                .max_w(t.layout.reading_width)
                .mx_auto()
                .gap(t.space.large)
                .child(design::page_header(
                    "My AIs",
                    "Choose an AI, then start chatting.",
                    cx,
                ))
                .child(
                    v_flex()
                        .id("my-ais-current-choice")
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .p(t.space.medium)
                        .gap(t.space.small)
                        .sovra_surface(Surface::Matte, cx)
                        .child(
                            div()
                                .id("my-ais-name")
                                .test_support()
                                .aria_label(name.clone())
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(name),
                        )
                        .child(
                            div()
                                .id("my-ais-status")
                                .test_support()
                                .aria_label(description.clone())
                                .min_w_0()
                                .whitespace_normal()
                                .sovra_text(TextRole::Body, cx)
                                .child(description),
                        )
                        .child(actions),
                )
                .child(render_mode_selector(preview_mode, cx)),
        )
}

/// Plain titles. The core names stay technical and are not shown here.
fn mode_copy(mode: SelectionMode) -> (&'static str, &'static str) {
    match mode {
        SelectionMode::SingleGlobalModel => (
            "Use my chat model",
            "Preview with the model you picked in Models.",
        ),
        SelectionMode::PerAgentModel => (
            "Preview effort by task",
            "Compare Quick, Balanced or Thorough preferences for each helper. A chosen model takes priority in this preview.",
        ),
        SelectionMode::DynamicTreeRouting => (
            "Preview automatic choice",
            "See which helper and model tier the routing rules suggest for a question. This does not check whether that model is connected.",
        ),
    }
}

/// One row of choices. Stretch keeps the short card as tall as the long ones.
fn render_mode_selector(
    active_mode: SelectionMode,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .id("routing-mode-selector")
        .test_support()
        .w_full()
        .min_w_0()
        .items_stretch()
        .flex_wrap()
        .gap(t.space.small)
        .children(
            SelectionMode::ALL
                .into_iter()
                .enumerate()
                .map(|(idx, mode)| {
                    let selected = mode == active_mode;
                    let (title, description) = mode_copy(mode);
                    design::button(("routing-mode-btn", idx), "", cx)
                        .accessibility_label(format!("{title}. {description}"))
                        .selected(selected)
                        .toggled(selected)
                        .prefer_width(t.layout.card_min_width)
                        .self_stretch()
                        .h_auto()
                        .items_start()
                        .justify_start()
                        .whitespace_normal()
                        .p(t.space.large)
                        .rounded(t.radius.card)
                        .child(
                            Icon::new(if selected {
                                IconName::CircleCheck
                            } else {
                                IconName::Circle
                            })
                            .small()
                            .text_color(if selected {
                                cx.theme().link
                            } else {
                                t.text_secondary
                            }),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .w_full()
                                .items_start()
                                .whitespace_normal()
                                .gap(t.space.small)
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .whitespace_normal()
                                        .sovra_text(TextRole::SectionTitle, cx)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .whitespace_normal()
                                        .sovra_text(TextRole::Caption, cx)
                                        .text_color(t.text_secondary)
                                        .child(description),
                                ),
                        )
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.state.set_routing_mode(mode);
                            let backend = shell.state.routing.backend;
                            let global_model = shell.state.routing.global_model.clone();
                            let _ = cx.global::<RuntimeClient>().0.send(Command::UpdateRouting {
                                mode,
                                backend,
                                global_model,
                            });
                            cx.notify();
                        }))
                }),
        )
}
