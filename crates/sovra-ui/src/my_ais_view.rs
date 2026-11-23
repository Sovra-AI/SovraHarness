//! The saved chat AI, its thinking level, and a different AI per question difficulty.

use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::{
    Context, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::{NavTab, models::SetupStage};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render_my_ais_view(
    models: &Entity<crate::ModelsView>,
    tiers: &Entity<crate::ai_tiers_view::AiTiersView>,
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
                .child(tiers.clone()),
        )
}
