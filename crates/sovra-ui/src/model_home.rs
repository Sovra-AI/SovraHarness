//! Guided model setup wizard: clear 3-step progression, state-aware cards that
//! never repeat completed steps, curated model recommendations, and 1-click tour launch.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, button::ButtonVariants as _, h_flex, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::models::{ModelCommand, ModelInfo, ModelKey, RecommendedModelCard, SetupStage};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{GoToChat, ModelsView, StartSystemTour, caption, pill};

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
        self.choosing_purpose = None;
        self.settings_open = false;
        self.connections_open = false;
        self.connection_form = None;
        self.manual_form = None;
        self.form_subscriptions.clear();
        self.selected = None;
        self.compare_open = false;
        cx.notify();
    }

    pub(crate) fn choose_model(&mut self, key: &ModelKey, cx: &mut Context<Self>) {
        self.state.error = None;
        self.pending_activation = Some(key.clone());
        let command = self.choosing_purpose.map_or_else(
            || ModelCommand::Activate { key: key.clone() },
            |purpose| ModelCommand::AssignPurpose {
                purpose,
                key: Some(key.clone()),
            },
        );
        self.send(command, cx);
        if self.state.error.is_some() {
            self.pending_activation = None;
        }
    }

    fn render_setup_stepper(connected: bool, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let (step1, step2) = if connected {
            ("✓ 1 · Connected", "● 2 · Pick your AI")
        } else {
            ("● 1 · Connect an AI service", "2 · Pick your AI")
        };
        h_flex()
            .id("models-setup-stepper")
            .test_support()
            .flex_wrap()
            .items_center()
            .gap(t.space.compact)
            .child(pill(step1, cx))
            .child(caption("→", cx))
            .child(pill(step2, cx))
            .child(caption("→ then you're chatting", cx))
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
            .map_or_else(|| "Price not listed".to_owned(), ModelInfo::plain_price);
        let key = rec.key.clone();
        let button_label = if is_active {
            "Active"
        } else if is_activating {
            "Choosing…"
        } else {
            "Use this model"
        };
        // During setup the cards are the main action; afterwards they are suggestions.
        let button = if self.state.has_active_llm() {
            design::button(("models-home-rec-btn", index), button_label, cx)
        } else {
            design::primary_button(("models-home-rec-btn", index), button_label, cx)
        };

        v_flex()
            .id(("models-home-rec", index))
            .test_support()
            .w_full()
            .p(t.space.medium)
            .gap(t.space.small)
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
                        button
                            .small()
                            .disabled(is_active || self.pending_activation.is_some())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.choose_model(&key, cx);
                            })),
                    ),
            )
            .child(caption(rec.kind.summary(), cx))
            .child(
                div()
                    .id(("models-home-rec-price", index))
                    .test_support()
                    .aria_label(price_text.clone())
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().foreground)
                    .child(price_text),
            )
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
        let stage = self.state.setup_stage();
        let any_connected = matches!(
            stage,
            SetupStage::NeedsModelSelection { .. } | SetupStage::Ready { .. }
        );
        let active = match &stage {
            SetupStage::Ready { active } => Some(active),
            _ => None,
        };
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
                    .child(design::page_header(
                        "Models",
                        "Find the right AI for the way you work.",
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap(t.space.compact)
                            .child(
                                design::button("models-start-tour", "Take a tour", cx)
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
                (!narrow && active.is_none())
                    .then(|| Self::render_setup_stepper(any_connected, cx)),
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
                .child(caption("Your active AI", cx))
                .child(
                    div()
                        .id("models-home-active-name")
                        .test_support()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(name),
                )
                .child(caption(
                    "Ready for your next conversation. You can switch models anytime.",
                    cx,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::primary_button("models-home-chat", "Go to chat", cx)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(GoToChat))),
                        )
                        .child(
                            design::button("models-home-take-tour", "Take a tour", cx)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(StartSystemTour))),
                        ),
                );
        } else if let SetupStage::NeedsModelKey {
            model_id,
            connection_name,
            ..
        } = &stage
        {
            let name = self
                .state
                .preferences
                .active
                .as_ref()
                .and_then(|key| self.state.model(key))
                .map_or_else(|| model_id.clone(), |model| model.name.clone());
            next = next
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(format!("Add your {connection_name} key")),
                )
                .child(caption(
                    format!(
                        "You chose {name}. Paste your {connection_name} key once and you'll go straight to the chat."
                    ),
                    cx,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            design::primary_button(
                                "models-home-add-key",
                                format!("Add {connection_name} key"),
                                cx,
                            )
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_selected_connection(window, cx);
                                view.setup_flow = true;
                            })),
                        )
                        .child(
                            design::button("models-home-pick-other", "Pick a different AI", cx)
                                .ghost()
                                .on_click(cx.listener(|view, _, _, cx| view.open_model_catalog(cx))),
                        ),
                );
        } else if let SetupStage::NeedsModelSelection {
            connected_service,
            key_preview,
        } = &stage
        {
            let key_summary = key_preview.as_ref().map_or_else(
                || format!("✓ {connected_service} is connected"),
                |preview| format!("✓ {connected_service} is connected ({preview})"),
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
                            design::button("models-home-change-key", "Change key", cx)
                                .small()
                                .ghost()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.home_open = false;
                                    view.settings_open = true;
                                    cx.notify();
                                })),
                        ),
                )
                .children(self.state.active_problem.as_ref().map(|problem| {
                    caption(
                        format!("The AI you chose before can't answer right now: {problem}"),
                        cx,
                    )
                    .text_color(cx.theme().warning)
                }))
                .child(caption("STEP 2 OF 2", cx))
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("Pick a chat model"),
                )
                .child(caption(
                    "Any of these can chat with you. Press “Use this model” to start chatting; you can switch anytime here in Models.",
                    cx,
                ));
            if recommendations.is_empty() {
                let loading = self.state.catalogs.values().any(|c| c.loading);
                next = next.child(caption(
                    if loading {
                        "Loading the list of AIs…"
                    } else {
                        "No suggestions yet. Every model in the full list can chat, so you can pick from there, or check again."
                    },
                    cx,
                ));
            } else {
                next = next.children(
                    recommendations
                        .iter()
                        .enumerate()
                        .map(|(idx, rec)| self.render_recommendation_card(idx, rec, cx)),
                );
            }
            next = next.child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button("models-home-choose", "See all chat models", cx)
                            .on_click(cx.listener(|view, _, _, cx| view.open_model_catalog(cx))),
                    )
                    .children(recommendations.is_empty().then(|| {
                        design::button("models-home-refresh", "Check again", cx)
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| view.refresh_visible(cx)))
                    })),
            );
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
                                "Connect OpenRouter",
                                cx,
                            )
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_openrouter_key_editor(window, cx);
                                view.setup_flow = true;
                            })),
                        )
                        .child(
                            design::button(
                                "models-home-open-openrouter",
                                "Get an API key ↗",
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
                    "Create a key on OpenRouter, then choose Connect OpenRouter to add it here.",
                    cx,
                ));
        }

        content = content.child(next);
        // Per-file AIs only matter once chat works; during setup they are a distraction.
        content = content.children(active.is_some().then(|| {
            v_flex()
                .w_full()
                .gap(t.space.small)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child("An AI for each kind of file"),
                )
                .child(caption(
                    "Choose once. Sovra will use the right AI when you add a file in chat.",
                    cx,
                ))
                .children(
                    sovra_core::files::ModelPurpose::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, purpose)| {
                            let key = self.state.preferences.purposes.get(&purpose);
                            let name = key.map_or_else(
                                || {
                                    if purpose == sovra_core::files::ModelPurpose::Documents {
                                        "Uses your chat AI".to_owned()
                                    } else {
                                        "Choose when you need it".to_owned()
                                    }
                                },
                                |key| {
                                    self.state.model(key).map_or_else(
                                        || key.model_id.clone(),
                                        |model| model.name.clone(),
                                    )
                                },
                            );
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .justify_between()
                                .items_center()
                                .gap(t.space.small)
                                .p(t.space.medium)
                                .sovra_surface(Surface::Subtle, cx)
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .gap(t.space.compact)
                                        .child(
                                            div()
                                                .sovra_text(TextRole::Body, cx)
                                                .child(purpose.label()),
                                        )
                                        .child(caption(name, cx)),
                                )
                                .child(
                                    design::button(
                                        ("model-purpose", index),
                                        if key.is_some() { "Change" } else { "Choose AI" },
                                        cx,
                                    )
                                    .accessibility_label(format!(
                                        "Choose AI to {}",
                                        purpose.label().to_lowercase()
                                    ))
                                    .on_click(cx.listener(
                                        move |view, _, window, cx| {
                                            view.open_for_purpose(purpose, window, cx);
                                        },
                                    )),
                                )
                        }),
                )
        }));
        if active.is_some() && !recommendations.is_empty() {
            content = content.child(
                v_flex()
                    .gap(t.space.small)
                    .child(
                        div()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child("Other chat models you can switch to"),
                    )
                    .children(
                        recommendations
                            .iter()
                            .enumerate()
                            .map(|(idx, rec)| self.render_recommendation_card(idx, rec, cx)),
                    ),
            );
        }
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
                                design::button("models-home-local", "Use Ollama", cx)
                                    .ghost()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.open_ollama_setup(window, cx);
                                    })),
                            )
                            .child(
                                design::button(
                                    "models-home-other",
                                    "Connect a server",
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
            .p(t.space.large)
            .overflow_y_scroll()
            .child(content)
    }
}
