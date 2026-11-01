//! Settings and AI setup: appearance, `OpenRouter` key management with
//! visible active-key preview, guided Ollama & OpenAI-compatible URL setup, and explained
//! Agentic AI settings (always-on context, reply style, memory, tools, and guardrails).

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Theme, ThemeMode,
    button::ButtonVariants as _, checkbox::Checkbox, h_flex, input::Input, v_flex,
};
use gpui_kit::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::models::{
    AgentSettings, Appearance, Authentication, CredentialChange, InterfaceScale, ModelCommand,
    ProviderKind, ResponseStyle,
};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::model_connection_editor::{ConnectionForm, back_button, credential_status, field};
use crate::models_view::{ModelsView, caption, pill};

const CONTEXT_STARTERS: [(&str, &str); 3] = [
    (
        "Explain simply (no jargon)",
        "Please explain everything in plain, everyday English without technical jargon. Use simple, relatable examples.",
    ),
    (
        "Short bullet points",
        "Keep answers short, warm, and organized into easy-to-read bullet points.",
    ),
    (
        "Step-by-step helper",
        "When helping me do something on my computer, guide me patiently one small numbered step at a time.",
    ),
];

pub(crate) fn settings_section(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    cx: &App,
) -> impl gpui_kit::ParentElement + IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .p(t.space.medium)
        .gap(t.space.small)
        .sovra_surface(Surface::Matte, cx)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
        .child(caption(description, cx))
}

fn settings_page(content: gpui_kit::AnyElement, cx: &App) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("sovra-settings-view")
        .test_support()
        .size_full()
        .min_w_0()
        .min_h_0()
        .p(t.space.large)
        .child(
            v_flex()
                .id("settings-sections")
                .test_support()
                .w_full()
                .flex_1()
                .min_h_0()
                .items_center()
                .overflow_y_scroll()
                .child(content),
        )
}

impl ModelsView {
    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut page = v_flex()
            .w_full()
            .max_w(t.layout.reading_width)
            .min_w_0()
            .flex_shrink_0()
            .gap(t.space.large)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(design::page_header(
                        "Settings",
                        "Make Sovra feel like your own.",
                        cx,
                    ))
                    .child(
                        design::button("settings-restart-tour", "Take a tour", cx)
                            .small()
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(crate::models_view::StartSystemTour);
                            })),
                    ),
            );
        if let Some(error) = &self.state.error {
            page = page.child(caption(error.clone(), cx).text_color(cx.theme().danger));
        } else if let Some(notice) = &self.state.notice {
            page = page.child(caption(notice.clone(), cx));
        }
        if self.connections.qr_modal_open {
            return settings_page(
                page.child(self.render_whatsapp_qr_modal(&self.connections, cx))
                    .into_any_element(),
                cx,
            )
            .into_any_element();
        }
        if self.connections.qr_modal_open {
            return settings_page(
                page.child(self.render_whatsapp_qr_modal(&self.connections, cx))
                    .into_any_element(),
                cx,
            )
            .into_any_element();
        }
        if self.manual_form.is_some() {
            return settings_page(
                page.child(self.render_manual_form(cx)).into_any_element(),
                cx,
            )
            .into_any_element();
        }
        if let Some(form) = &self.connection_form {
            if form.original.kind == ProviderKind::OpenRouter {
                return settings_page(
                    page.child(self.render_key_form(form, cx))
                        .into_any_element(),
                    cx,
                )
                .into_any_element();
            }
            return settings_page(
                page.child(self.render_connection_form(cx))
                    .into_any_element(),
                cx,
            )
            .into_any_element();
        }

        settings_page(
            page.child(
                v_flex()
                    .w_full()
                    .gap(t.space.small)
                    .child(Self::render_appearance_section(cx))
                    .child(self.render_ai_connection_section(cx))
                    .child(self.render_connections_section(cx))
                    .child(self.render_telegram_section(cx))
                    .child(self.render_development_section(cx))
                    .child(self.render_other_connections_section(cx))
                    .children(self.render_google_section(cx))
                    .child(self.render_always_context_section(cx))
                    .child(self.render_agent_behavior_section(cx)),
            )
            .into_any_element(),
            cx,
        )
        .into_any_element()
    }

    /// The person's own accounts: Google now, and `WhatsApp` joins this section.
    fn render_google_section(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let card = self.google_card.clone()?;
        Some(
            settings_section(
                "settings-google-account",
                "Connections",
                "Let Sovra help with your own accounts. You choose what it can use, and you \
                 can switch anything off at any time.",
                cx,
            )
            .child(card),
        )
    }

    fn render_appearance_section(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let dark = cx.theme().is_dark();
        settings_section(
            "settings-appearance",
            "Appearance",
            "Choose a light or dark workspace.",
            cx,
        )
        .child(h_flex().gap(t.space.compact).children(
            [(ThemeMode::Light, "Light"), (ThemeMode::Dark, "Dark")].map(|(mode, label)| {
                design::button(
                    match mode {
                        ThemeMode::Light => "settings-theme-light",
                        ThemeMode::Dark => "settings-theme-dark",
                    },
                    label,
                    cx,
                )
                .accessibility_label(format!("{label} appearance"))
                .selected(mode.is_dark() == dark)
                .on_click(cx.listener(move |view, _, _, cx| {
                    Theme::change(mode, None, cx);
                    view.send(
                        ModelCommand::SetAppearance {
                            appearance: if mode.is_dark() {
                                Appearance::Dark
                            } else {
                                Appearance::Light
                            },
                        },
                        cx,
                    );
                }))
            }),
        ))
        .child(Self::render_text_size(cx))
    }

    /// Interface size (zoom); the same levels as View → Zoom In/Out and ⌘=/⌘-/⌘0.
    fn render_text_size(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let scale = design::interface_scale(cx);
        h_flex()
            .id("settings-text-size")
            .flex_wrap()
            .items_center()
            .gap(t.space.compact)
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .text_color(t.text_secondary)
                    .child("Text size"),
            )
            .child(
                design::button("settings-text-smaller", "A−", cx)
                    .accessibility_label("Make text smaller")
                    .tooltip("Make text smaller (⌘−)")
                    .disabled(scale == InterfaceScale::SMALLEST)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::app_menu::zoom(scale.smaller(), cx);
                    })),
            )
            .child(
                design::button("settings-text-reset", format!("{}%", scale.percent()), cx)
                    .accessibility_label(format!("Text size {}%, reset to 100%", scale.percent()))
                    .tooltip("Actual size (⌘0)")
                    .on_click(cx.listener(|_, _, _, cx| {
                        crate::app_menu::zoom(InterfaceScale::default(), cx);
                    })),
            )
            .child(
                design::button("settings-text-larger", "A+", cx)
                    .accessibility_label("Make text larger")
                    .tooltip("Make text larger (⌘=)")
                    .disabled(scale == InterfaceScale::LARGEST)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::app_menu::zoom(scale.larger(), cx);
                    })),
            )
    }

    fn render_ai_connection_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let router = self.state.openrouter_connection();
        let connected = router.is_some_and(|c| c.has_credential || c.environment_credential);
        let status = router.map_or("Not connected yet", credential_status);
        let key_preview = router.and_then(|c| self.active_key_preview(c)).map_or_else(
            || "Active key: None".to_owned(),
            |preview| format!("Active key: {preview}"),
        );
        let active_model_label = match &self.state.preferences.active {
            Some(key) => {
                let name = self
                    .state
                    .model(key)
                    .map_or_else(|| key.model_id.clone(), |m| m.name.clone());
                format!("Active model: {name}")
            }
            None => "Active model: None selected".to_owned(),
        };

        settings_section(
            "settings-ai",
            "AI connection",
            "Connect OpenRouter once, then choose from its online models.",
            cx,
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.compact)
                .items_center()
                .child(
                    div()
                        .id("settings-openrouter-status")
                        .test_support()
                        .aria_label(status)
                        .text_color(if connected {
                            cx.theme().success
                        } else {
                            t.text_secondary
                        })
                        .child(status),
                )
                .child(
                    div()
                        .id("settings-active-key-badge")
                        .test_support()
                        .aria_label(key_preview.clone())
                        .child(pill(key_preview, cx)),
                )
                .child(
                    div()
                        .id("settings-active-model-status")
                        .test_support()
                        .aria_label(active_model_label.clone())
                        .child(pill(active_model_label, cx)),
                ),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::primary_button(
                        "settings-openrouter",
                        if connected {
                            "Manage OpenRouter key"
                        } else {
                            "Add OpenRouter key"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_openrouter_key_editor(window, cx);
                    })),
                )
                .child(
                    design::button("settings-choose-model", "Choose a model", cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.open_model_home(cx);
                        }),
                    ),
                ),
        )
    }

    fn render_always_context_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let saved_context = self.state.preferences.agent_settings.always_context.trim();
        let status_label = if saved_context.is_empty() {
            "No standing instructions saved yet — Sovra will use its friendly default.".to_owned()
        } else {
            format!(
                "Active in every chat ({} characters saved)",
                saved_context.len()
            )
        };
        settings_section(
            "settings-always-context",
            "Personal instructions",
            "Tell Sovra what to keep in mind in every chat: your work, preferences, or how you like answers.",
            cx,
        )
        .child(caption(status_label, cx))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.compact)
                .children(
                    CONTEXT_STARTERS
                        .iter()
                        .enumerate()
                        .map(|(idx, (label, sample))| {
                            let text = (*sample).to_owned();
                            design::button(("settings-context-preset", idx), *label, cx)
                                .small()
                                .ghost()
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.always_context_input.update(cx, |input, cx| {
                                        input.set_value(text.clone(), window, cx);
                                    });
                                    cx.notify();
                                }))
                        }),
                ),
        )
        .child(
            Input::new(&self.always_context_input)
                .id("settings-always-context-input")
                .aria_label("Always-on context and custom instructions")
                .cleanable(true)
                .w_full(),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::primary_button("settings-save-context", "Save instructions", cx)
                        .on_click(cx.listener(|view, _, _, cx| {
                            let mut settings = view.state.preferences.agent_settings.clone();
                            view.always_context_input.read(cx).value().trim().clone_into(&mut settings.always_context);
                            view.save_agent_settings(settings, cx);
                        })),
                ),
        )
    }

    fn render_agent_behavior_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let current = &self.state.preferences.agent_settings;
        let style_desc = current.response_style.description();
        settings_section(
            "settings-agent-behavior",
            "Memory & tools",
            "Choose what Sovra can use to help you.",
            cx,
        )
        .child(
            v_flex()
                .gap(t.space.compact)
                .child(caption(
                    "Reply style",
                    cx,
                ))
                .child(
                    h_flex().flex_wrap().gap(t.space.compact).children(
                        ResponseStyle::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(idx, style)| {
                                design::button(("settings-style-btn", idx), style.label(), cx)
                                    .small()
                                    .selected(current.response_style == style)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        let mut next =
                                            view.state.preferences.agent_settings.clone();
                                        next.response_style = style;
                                        view.save_agent_settings(next, cx);
                                    }))
                            }),
                    ),
                )
                .child(caption(style_desc, cx)),
        )
        .child(
            v_flex()
                .gap(t.space.hair)
                .child(
                    Checkbox::new("settings-memory-toggle")
                        .cursor_pointer()
                        .label("Remember across conversations")
                        .checked(current.memory_enabled)
                        .on_click(cx.listener(|view, checked, _, cx| {
                            let mut next = view.state.preferences.agent_settings.clone();
                            next.memory_enabled = *checked;
                            view.save_agent_settings(next, cx);
                        })),
                )
                .child(caption(
                    "Your selected AI checks your messages for useful details, which are saved on this computer. Messages and earlier memories may be sent to that AI service. Turning this off stops new remembering and cancels pending work; saved history is kept.",
                    cx,
                )),
        )
        .child(
            v_flex()
                .gap(t.space.hair)
                .child(
                    Checkbox::new("settings-tools-toggle")
                        .cursor_pointer()
                        .label("Interactive answers")
                        .checked(current.tools_enabled)
                        .on_click(cx.listener(|view, checked, _, cx| {
                            let mut next = view.state.preferences.agent_settings.clone();
                            next.tools_enabled = *checked;
                            view.save_agent_settings(next, cx);
                        })),
                )
                .child(caption(
                    "Lets your AI create clickable buttons, progress checklists, and clean summary cards inside your conversation when helpful.",
                    cx,
                )),
        )
        .child(
            v_flex()
                .gap(t.space.hair)
                .child(
                    Checkbox::new("settings-web-search-toggle")
                        .cursor_pointer()
                        .label("Web search")
                        .checked(current.web_search_enabled)
                        .on_click(cx.listener(|view, checked, _, cx| {
                            let mut next = view.state.preferences.agent_settings.clone();
                            next.web_search_enabled = *checked;
                            view.save_agent_settings(next, cx);
                        })),
                )
                .child(caption(
                    "Lets compatible models search the internet when you ask about today's news, weather, or recent events.",
                    cx,
                )),
        )
        .child(
            v_flex()
                .gap(t.space.hair)
                .child(
                    Checkbox::new("settings-confirm-actions-toggle")
                        .cursor_pointer()
                        .label("Confirm important actions")
                        .checked(current.confirm_actions)
                        .on_click(cx.listener(|view, checked, _, cx| {
                            let mut next = view.state.preferences.agent_settings.clone();
                            next.confirm_actions = *checked;
                            view.save_agent_settings(next, cx);
                        })),
                )
                .child(caption(
                    "Keeps you in charge: Sovra will explain what it plans to do and ask for your go-ahead before making changes.",
                    cx,
                )),
        )
    }

    fn render_other_connections_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut advanced = settings_section(
            "settings-advanced",
            "Other connections",
            "Run free AI on your computer with Ollama (no key needed), or connect any OpenAI-compatible URL (LM Studio, vLLM, llama.cpp).",
            cx,
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button(
                        "settings-advanced-toggle",
                        if self.settings_advanced_open {
                            "Hide advanced options"
                        } else {
                            "Show advanced options"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.settings_advanced_open = !view.settings_advanced_open;
                        cx.notify();
                    })),
                )
                .child(
                    design::button(
                        "settings-ollama-btn",
                        "Set up Ollama",
                        cx,
                    )
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_ollama_setup(window, cx);
                    })),
                )
                .child(
                    design::button(
                        "settings-openai-url-btn",
                        "Add a server",
                        cx,
                    )
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_openai_compatible_editor(window, cx);
                    })),
                ),
        );
        if self.settings_advanced_open {
            advanced = advanced
                .child(Self::connections_header(cx))
                .children(self.connection_cards(cx));
        }
        advanced
    }

    pub(crate) fn render_key_form(
        &self,
        form: &ConnectionForm,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let saved = form.original.has_credential;
        let has_draft = !form.key.read(cx).value().trim().is_empty();
        let saving = self.pending_save.is_some();
        let status = credential_status(&form.original);
        let active_preview = self.active_key_preview(&form.original).map_or_else(
            || "Active OpenRouter key: None saved yet".to_owned(),
            |preview| format!("Active OpenRouter key: {preview}"),
        );
        let remove = form.original.clone();
        let mut steps = v_flex().gap(t.space.compact);
        if !saved {
            steps = steps
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(t.space.small)
                        .child(caption(
                            "1. Go to openrouter.ai in your web browser and sign in (signing up is free).",
                            cx,
                        ))
                        .child(
                            design::button(
                                "openrouter-open-browser",
                                "Open openrouter.ai/keys in Browser ↗",
                                cx,
                            )
                            .small()
                            .on_click(|_, _, cx| cx.open_url("https://openrouter.ai/keys")),
                        ),
                )
                .child(caption(
                    "2. Click “Keys” in the menu and press “Create Key”.",
                    cx,
                ))
                .child(caption(
                    "3. Copy the key (it starts with “sk-or-”) and paste it in the box below.",
                    cx,
                ))
                .child(caption(
                    "4. Save your key, then choose “Next: pick a chat model”. Free and paid models are shown with their price.",
                    cx,
                ));
        }
        v_flex()
            .id("settings-key-form")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .max_w(t.layout.reading_width)
            .gap(t.space.medium)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Your OpenRouter API Key"),
            )
            .child(
                div()
                    .id("settings-key-status")
                    .test_support()
                    .aria_label(status)
                    .sovra_surface(Surface::Subtle, cx)
                    .p(t.space.small)
                    .text_color(if saved || form.original.environment_credential {
                        cx.theme().success
                    } else {
                        t.text_secondary
                    })
                    .child(status),
            )
            .child(
                div()
                    .id("settings-key-active-preview")
                    .test_support()
                    .aria_label(active_preview.clone())
                    .child(pill(active_preview, cx)),
            )
            .children((saved && self.state.preferences.active.is_none()).then(|| {
                div()
                    .id("settings-key-next-step")
                    .test_support()
                    .sovra_text(TextRole::Body, cx)
                    .child("Key saved. Now pick the model you want to chat with — free ones are listed first.")
            }))
            .child(steps)
            .child(field(
                "connection-key",
                if saved {
                    "Replace OpenRouter key"
                } else {
                    "Paste your OpenRouter key"
                },
                &form.key,
                if saved {
                    "Your active key is shown above. Only fill this in if you want to replace it with a different OpenRouter key."
                } else {
                    "It starts with “sk-or-”. Sovra locks it inside this computer's keychain and shows a masked preview above."
                },
                cx,
            ))
            .child(
                h_flex()
                    .gap(t.space.small)
                    .flex_wrap()
                    .child(
                        design::primary_button(
                            "connection-save",
                            if saving {
                                "Saving…"
                            } else if saved {
                                "Replace key"
                            } else {
                                "Save key"
                            },
                            cx,
                        )
                        .disabled(saving || !has_draft)
                        .on_click(cx.listener(|view, _, _, cx| view.submit_connection(false, cx))),
                    )
                    .child(
                        design::button("connection-test", "Check key", cx)
                            .disabled(
                                self.state.busy
                                    || saving
                                    || (!saved
                                        && !form.original.environment_credential
                                        && !has_draft),
                            )
                            .on_click(cx.listener(|view, _, _, cx| view.submit_connection(true, cx))),
                    )
                    .children(saved.then(|| {
                        design::button("settings-remove-key", "Remove key", cx)
                            .disabled(saving)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.last_saved_previews.remove(&remove.id);
                                view.pending_save = Some(remove.id.clone());
                                view.send(
                                    ModelCommand::SaveConnection {
                                        connection: remove.clone(),
                                        credential: CredentialChange::Remove,
                                    },
                                    cx,
                                );
                            }))
                    }))
                    .child(if saved && self.state.preferences.active.is_none() {
                        design::primary_button(
                            "settings-key-choose-model",
                            "Next: pick a chat model →",
                            cx,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open_model_home(cx);
                        }))
                    } else {
                        design::button(
                            "settings-key-choose-model",
                            "Pick a chat model",
                            cx,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open_model_home(cx);
                        }))
                    })
                    .child(back_button(cx)),
            )
            .child(caption(
                "“Check key” only verifies that OpenRouter accepts your key. It doesn't send a chat message or cost anything.",
                cx,
            ))
    }

    pub(crate) fn save_agent_settings(&mut self, settings: AgentSettings, cx: &mut Context<Self>) {
        self.state.preferences.agent_settings = settings.clone();
        self.send(ModelCommand::SetAgentSettings { settings }, cx);
    }

    pub fn open_openrouter_key_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        let connection = self
            .state
            .connections
            .iter()
            .find(|connection| connection.kind == ProviderKind::OpenRouter)
            .cloned();
        let is_new = connection.is_none();
        self.open_connection(connection, window, cx);
        if is_new {
            if let Some(form) = &mut self.connection_form {
                form.original.id = "openrouter".into();
                form.original.kind = ProviderKind::OpenRouter;
            }
            self.preset(
                "OpenRouter",
                ProviderKind::OpenRouter,
                ProviderKind::OpenRouter.default_url(),
                Authentication::Bearer,
                window,
                cx,
            );
        }
    }

    pub fn open_ollama_setup(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.home_open = false;
        self.settings_open = false;
        self.connections_open = false;
        self.connection_form = None;
        self.manual_form = None;
        self.filter.scope = sovra_core::models::CatalogScope::Ollama;
        self.download_open = true;
        self.controls_dirty = true;
        self.rebuild_rows();
        cx.notify();
    }

    pub fn open_openai_compatible_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.settings_advanced_open = true;
        self.open_connection(None, window, cx);
        self.preset(
            "OpenAI compatible server",
            ProviderKind::OpenAiCompatible,
            ProviderKind::OpenAiCompatible.default_url(),
            Authentication::None,
            window,
            cx,
        );
    }

    pub fn open_model_catalog(&mut self, cx: &mut Context<Self>) {
        self.home_open = false;
        self.settings_open = false;
        self.connections_open = false;
        self.connection_form = None;
        self.manual_form = None;
        self.form_subscriptions.clear();
        self.controls_dirty = true;
        self.rebuild_rows();
        cx.notify();
    }
}
