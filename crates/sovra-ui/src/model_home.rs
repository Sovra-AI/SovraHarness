//! Guided model setup wizard: clear 3-step progression, state-aware cards that
//! never repeat completed steps, curated model recommendations, and 1-click tour launch.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, button::ButtonVariants as _, h_flex, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::models::{ModelCommand, ModelKey, RecommendedModelCard, SetupStage};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{GoToChat, ModelsView, StartSystemTour, caption, pill, price_summary};

impl ModelsView {
    pub(crate) fn open_selected_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let connection = self
            .state
            .preferences
            .active
            .as_ref()
            .and_then(|key| {
                self.state
                    .connections
                    .iter()
                    .find(|connection| connection.id == key.connection_id)
            })
            .cloned();
        if let Some(connection) = connection {
            self.home_open = false;
            self.settings_open = true;
            self.open_connection(Some(connection), window, cx);
        } else {
            self.open_model_home(cx);
        }
    }

    pub(crate) fn open_model_home(&mut self, cx: &mut Context<Self>) {
        self.home_open = true;
        self.settings_open = false;
        self.connections_open = false;
        self.connection_form = None;
        self.manual_form = None;
        self.form_subscriptions.clear();
        self.selected = None;
        self.compare_open = false;
        cx.notify();
    }

    pub(crate) fn choose_model(&mut self, key: ModelKey, cx: &mut Context<Self>) {
        self.state.error = None;
        self.pending_activation = Some(key.clone());
        self.send(ModelCommand::Activate { key }, cx);
        if self.state.error.is_some() {
            self.pending_activation = None;
        }
    }

    fn render_setup_stepper(
        connected: bool,
        has_active_model: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let step1_label = if connected {
            "✓ Step 1 · Key Connected"
        } else {
            "● Step 1 · Connect AI Service"
        };
        let step2_label = if has_active_model {
            "✓ Step 2 · Model Selected"
        } else if connected {
            "● Step 2 · Pick Your Model"
        } else {
            "2 · Pick Your Model"
        };
        let step3_label = if has_active_model {
            "● Step 3 · Chat & System Tour"
        } else {
            "3 · Chat & System Tour"
        };

        h_flex()
            .id("models-setup-stepper")
            .test_support()
            .flex_wrap()
            .items_center()
            .gap(t.space.compact)
            .child(pill(step1_label, cx))
            .child(caption("→", cx))
            .child(pill(step2_label, cx))
            .child(caption("→", cx))
            .child(pill(step3_label, cx))
    }

    fn render_recommendation_card(
        &self,
        index: usize,
        rec: &RecommendedModelCard,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let is_active = self.state.preferences.active.as_ref() == Some(&rec.key);
        let is_activating = self.pending_activation.as_ref() == Some(&rec.key);
        let price_text = self
            .state
            .model(&rec.key)
            .map_or_else(|| "Ready to use".to_owned(), price_summary);
        let key = rec.key.clone();
        let button_label = if is_active {
            "✓ Active Now"
        } else if is_activating {
            "Choosing…"
        } else {
            "Use This Model"
        };

        v_flex()
            .id(("models-home-rec", index))
            .test_support()
            .w_full()
            .p(t.space.small)
            .gap(t.space.compact)
            .sovra_surface(
                if is_active {
                    Surface::NavActive
                } else {
                    Surface::Matte
                },
                cx,
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap(t.space.compact)
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_center()
                            .gap(t.space.compact)
                            .child(pill(rec.kind.badge(), cx))
                            .child(div().sovra_text(TextRole::Body, cx).child(rec.name.clone())),
                    )
                    .child(
                        design::primary_button(("models-home-rec-btn", index), button_label, cx)
                            .small()
                            .disabled(is_active || self.pending_activation.is_some())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.choose_model(key.clone(), cx);
                            })),
                    ),
            )
            .child(caption(rec.kind.summary(), cx))
            .child(caption(
                format!("{} · Price: {price_text}", rec.connection_name),
                cx,
            ))
    }

    pub(crate) fn render_model_home(
        &self,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let spacing = if narrow {
            t.space.small
        } else {
            t.space.medium
        };
        let openrouter_conn = self.state.openrouter_connection();
        let stage = self.state.setup_stage();
        let any_connected = matches!(
            stage,
            SetupStage::NeedsModelSelection { .. } | SetupStage::Ready { .. }
        );
        let active = match &stage {
            SetupStage::Ready { active } => Some(active),
            _ => None,
        };
        let suggestion = self.state.recommended_chat_model();
        let recommendations = self.state.recommended_model_cards();

        let mut content = v_flex()
            .w_full()
            .max_w(t.layout.reading_width)
            .gap(spacing)
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap(t.space.small)
                    .child(
                        div()
                            .sovra_text(
                                if narrow {
                                    TextRole::SectionTitle
                                } else {
                                    TextRole::PageTitle
                                },
                                cx,
                            )
                            .child(if narrow { "Models" } else { "Choose your AI" }),
                    )
                    .child(
                        h_flex()
                            .gap(t.space.compact)
                            .child(
                                design::button("models-start-tour", "Interactive Tour", cx)
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(StartSystemTour))),
                            )
                            .child(
                                design::button("models-browse-all", "Browse all models", cx)
                                    .small()
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.open_model_catalog(cx)),
                                    ),
                            ),
                    ),
            )
            .children(
                (!narrow).then(|| Self::render_setup_stepper(any_connected, active.is_some(), cx)),
            );

        if let Some(error) = &self.state.error {
            content = content.child(caption(error.clone(), cx).text_color(cx.theme().danger));
        }

        let mut next = v_flex()
            .w_full()
            .p(spacing)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx);
        if !self.state.loaded {
            next = next.child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Checking your saved setup…"),
            );
        } else if let Some(key) = active {
            let name = self
                .state
                .model(key)
                .map_or_else(|| key.model_id.clone(), |model| model.name.clone());
            next = next
                .child(caption("✓ SETUP COMPLETE · YOUR SELECTED AI", cx))
                .child(
                    div()
                        .id("models-home-active-name")
                        .test_support()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(name),
                )
                .child(caption(
                    "Your choice is saved. Go to Chat to send a message, or take a short tour of Sovra.",
                    cx,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::primary_button("models-home-chat", "Go to Chat →", cx)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(GoToChat))),
                        )
                        .child(
                            design::button(
                                "models-home-take-tour",
                                "Take 1-Minute System Tour",
                                cx,
                            )
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(StartSystemTour))),
                        ),
                );
            if !recommendations.is_empty() {
                next = next
                    .child(caption(
                        "Switch to another recommended model in 1 click:",
                        cx,
                    ))
                    .children(
                        recommendations
                            .iter()
                            .enumerate()
                            .map(|(idx, rec)| self.render_recommendation_card(idx, rec, cx)),
                    );
            }
        } else if let SetupStage::NeedsModelKey {
            model_id,
            connection_name,
            ..
        } = &stage
        {
            next = next
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(format!("Add your {connection_name} key")),
                )
                .child(caption(
                    format!(
                        "You chose {model_id}. Add the key for {connection_name} to finish setup."
                    ),
                    cx,
                ))
                .child(
                    design::primary_button("models-home-add-key", "Add this service’s key", cx)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.open_selected_connection(window, cx);
                        })),
                );
        } else if any_connected {
            let key_summary = openrouter_conn
                .and_then(|c| self.active_key_preview(c))
                .map_or_else(
                    || "✓ Step 1 Complete · AI service connected".to_owned(),
                    |preview| format!("✓ Step 1 Complete · OpenRouter Key Connected ({preview})"),
                );
            next = next
                .child(
                    h_flex()
                        .id("models-home-step1-done")
                        .test_support()
                        .flex_wrap()
                        .justify_between()
                        .items_center()
                        .gap(t.space.compact)
                        .child(
                            div()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(cx.theme().success)
                                .child(key_summary),
                        )
                        .child(
                            design::button("models-home-change-key", "Manage connections", cx)
                                .small()
                                .ghost()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.home_open = false;
                                    view.settings_open = true;
                                    cx.notify();
                                })),
                        ),
                )
                .child(caption("STEP 2 OF 2 · ONLY ONE STEP LEFT", cx))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("Pick Your AI Model to Start Chatting"),
                )
                .child(caption(
                    "Your connection is saved. Choose an AI to answer your messages. The price is shown with each choice.",
                    cx,
                ));
            if let Some(key) = suggestion.clone() {
                let name = self
                    .state
                    .model(&key)
                    .map_or_else(|| key.model_id.clone(), |model| model.name.clone());
                next = next
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_center()
                            .gap(t.space.small)
                            .child(
                                design::primary_button(
                                    "models-home-use-free",
                                    if self.pending_activation.is_some() {
                                        "Choosing…"
                                    } else {
                                        "Use Recommended Free Model →"
                                    },
                                    cx,
                                )
                                .disabled(self.pending_activation.is_some())
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.choose_model(key.clone(), cx);
                                    },
                                )),
                            )
                            .child(div().sovra_text(TextRole::Body, cx).child(name)),
                    )
                    .child(caption(
                        "Listed at $0 by OpenRouter. Usage limits apply.",
                        cx,
                    ));
            }
            if !recommendations.is_empty() {
                next = next.children(
                    recommendations
                        .iter()
                        .enumerate()
                        .map(|(idx, rec)| self.render_recommendation_card(idx, rec, cx)),
                );
            } else if suggestion.is_none() {
                next = next
                    .child(caption(
                        "Pick any model from the catalog below, or refresh to load the latest list.",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(t.space.small)
                            .child(
                                design::primary_button(
                                    "models-home-choose",
                                    "Choose from the library",
                                    cx,
                                )
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.open_model_catalog(cx)),
                                ),
                            )
                            .child(
                                design::button("models-home-refresh", "Check again", cx).on_click(
                                    cx.listener(|view, _, _, cx| view.refresh_visible(cx)),
                                ),
                            ),
                    );
            }
        } else {
            next = next
                .child(caption("STEP 1 OF 2 · CONNECT AN AI SERVICE", cx))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("Connect your AI account"),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::primary_button(
                                "models-home-connect",
                                "Connect OpenRouter Key →",
                                cx,
                            )
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_openrouter_key_editor(window, cx);
                            })),
                        )
                        .child(
                            design::button(
                                "models-home-open-openrouter",
                                "Get Free Key at openrouter.ai ↗",
                                cx,
                            )
                            .on_click(|_, _, cx| cx.open_url("https://openrouter.ai/keys")),
                        ),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child("OpenRouter supplies the AI answers. Create an account and copy its connection key, then paste the key here. Some models are free; others charge for use."),
                )
                .child(caption(
                    "1) Click “Get Free Key at openrouter.ai” to open the website and create a key (starts with sk-or-).  2) Click “Connect OpenRouter Key” to paste it.",
                    cx,
                ));
        }

        content = content.child(next);
        if matches!(stage, SetupStage::NeedsConnection) {
            content = content.child(
                v_flex()
                    .gap(t.space.small)
                    .child(
                        div()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child("Prefer local offline AI or another server?"),
                    )
                    .child(caption(
                        "Run free private models on your computer with Ollama, or connect any OpenAI-compatible server URL.",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(t.space.small)
                            .child(
                                design::button("models-home-local", "Use Ollama (Free Local)", cx)
                                    .ghost()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.open_ollama_setup(window, cx);
                                    })),
                            )
                            .child(
                                design::button(
                                    "models-home-other",
                                    "Connect OpenAI-Compatible URL",
                                    cx,
                                )
                                .ghost()
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.open_openai_compatible_editor(window, cx);
                                })),
                            ),
                    ),
            );
        }

        v_flex()
            .id("models-setup-home")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .p(spacing)
            .overflow_y_scroll()
            .child(content)
    }
}
