//! Connection and manual-model forms. Credentials stay masked and leave the UI
//! only through the model command protocol.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Theme, ThemeMode,
    button::ButtonVariants as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    searchable_list::SearchableVec,
    select::SelectEvent,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    div,
};
use sovra_core::models::{
    Appearance, Authentication, CredentialChange, ModelCommand, ModelConnection, ModelError,
    ModelInfo, ModelSecret, ProviderKind,
};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{
    Choice, ModelsView, Selector, caption, choices, select_control, selector,
};

pub(crate) struct ConnectionForm {
    original: ModelConnection,
    name: Entity<InputState>,
    url: Entity<InputState>,
    key: Entity<InputState>,
    key_header: Entity<InputState>,
    timeout: Entity<InputState>,
    organization: Entity<InputState>,
    project: Entity<InputState>,
    protocol: Entity<Selector>,
    authentication: Entity<Selector>,
    clear_key: bool,
    more_options: bool,
}

pub(crate) struct ManualModelForm {
    connection_id: String,
    id: Entity<InputState>,
    name: Entity<InputState>,
    context: Entity<InputState>,
    tools: bool,
}

fn input(
    value: &str,
    placeholder: &str,
    secret: bool,
    window: &mut Window,
    cx: &mut Context<ModelsView>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx)
            .placeholder(placeholder.to_owned())
            .masked(secret);
        input.set_value(value.to_owned(), window, cx);
        input
    })
}

fn field(
    id: &'static str,
    label: &'static str,
    state: &Entity<InputState>,
    hint: &'static str,
    cx: &App,
) -> gpui_kit::Div {
    let t = design::tokens(cx);
    v_flex()
        .w_full()
        .gap(t.space.compact)
        .child(caption(label, cx))
        .child(Input::new(state).id(id).aria_label(label).w_full())
        .child(caption(hint, cx))
}

fn protocol_value(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenRouter => "router",
        ProviderKind::Ollama => "ollama",
        ProviderKind::OpenAiCompatible => "compatible",
        ProviderKind::Anthropic => "anthropic",
    }
}
fn read_protocol(value: &str) -> ProviderKind {
    match value {
        "router" => ProviderKind::OpenRouter,
        "ollama" => ProviderKind::Ollama,
        "anthropic" => ProviderKind::Anthropic,
        _ => ProviderKind::OpenAiCompatible,
    }
}
fn auth_value(auth: Authentication) -> &'static str {
    match auth {
        Authentication::None => "none",
        Authentication::Bearer => "bearer",
        Authentication::ApiKey => "api-key",
    }
}

impl ConnectionForm {
    fn read(&self, cx: &App) -> Result<(ModelConnection, CredentialChange), ModelError> {
        let mut connection = self.original.clone();
        connection.name = self.name.read(cx).value().to_string();
        connection.base_url = self.url.read(cx).value().to_string();
        connection.kind = read_protocol(
            self.protocol
                .read(cx)
                .selected_value()
                .map_or("compatible", String::as_str),
        );
        connection.authentication = match self
            .authentication
            .read(cx)
            .selected_value()
            .map_or("none", String::as_str)
        {
            "bearer" => Authentication::Bearer,
            "api-key" => Authentication::ApiKey,
            _ => Authentication::None,
        };
        connection.timeout_seconds = self
            .timeout
            .read(cx)
            .value()
            .parse()
            .map_err(|_| ModelError("Enter a timeout in seconds, between 5 and 600.".into()))?;
        self.organization
            .read(cx)
            .value()
            .trim()
            .clone_into(&mut connection.organization);
        self.project
            .read(cx)
            .value()
            .trim()
            .clone_into(&mut connection.project);
        self.key_header
            .read(cx)
            .value()
            .trim()
            .clone_into(&mut connection.api_key_header);
        let key = self.key.read(cx).value().trim().to_owned();
        let change = if !key.is_empty() {
            CredentialChange::Replace(ModelSecret(key))
        } else if self.clear_key {
            CredentialChange::Remove
        } else {
            CredentialChange::Keep
        };
        Ok((connection, change))
    }
}

fn credential_status(connection: &ModelConnection) -> &'static str {
    if connection.has_credential {
        "Connected · your key is saved in this computer's keychain"
    } else if connection.environment_credential {
        "Connected · using a key from this computer's system settings"
    } else {
        "Not connected yet"
    }
}

fn back_button(cx: &mut Context<ModelsView>) -> gpui_kit::component::button::Button {
    design::button("connection-cancel", "Back", cx).on_click(cx.listener(|view, _, _, cx| {
        view.connection_form = None;
        view.form_subscriptions.clear();
        cx.notify();
    }))
}

fn settings_section(
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
        .sovra_surface(Surface::Subtle, cx)
        .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
        .child(caption(description, cx))
}

impl ModelsView {
    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut page = v_flex()
            .id("sovra-settings-view")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .p(t.space.medium)
            .gap(t.space.medium)
            .child(
                v_flex()
                    .gap(t.space.compact)
                    .child(div().sovra_text(TextRole::PageTitle, cx).child("Settings"))
                    .child(caption("Set up Sovra and make it yours.", cx)),
            );
        if let Some(error) = &self.state.error {
            page = page.child(caption(error.clone(), cx).text_color(cx.theme().danger));
        } else if let Some(notice) = &self.state.notice {
            page = page.child(caption(notice.clone(), cx));
        }
        if self.manual_form.is_some() {
            return page.child(self.render_manual_form(cx)).into_any_element();
        }
        if let Some(form) = &self.connection_form {
            if form.original.kind == ProviderKind::OpenRouter {
                return page
                    .child(self.render_key_form(form, cx))
                    .into_any_element();
            }
            return page
                .child(self.render_connection_form(cx))
                .into_any_element();
        }
        let dark = cx.theme().is_dark();
        let appearance = settings_section(
            "settings-appearance",
            "Appearance",
            "Choose how Sovra looks.",
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
        ));

        let router = self
            .state
            .connections
            .iter()
            .find(|c| c.kind == ProviderKind::OpenRouter);
        let connected = router.is_some_and(|c| c.has_credential || c.environment_credential);
        let status = router.map_or("Not connected yet", credential_status);
        let ai = settings_section(
            "settings-ai",
            "AI connection",
            "Sovra uses an online AI service called OpenRouter to understand you and reply. Add your OpenRouter key once and you're ready to chat.",
            cx,
        )
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
            h_flex().child(
                design::primary_button(
                    "settings-openrouter",
                    if connected { "Manage key" } else { "Add your key" },
                    cx,
                )
                .on_click(cx.listener(|view, _, window, cx| {
                    let connection = view
                        .state
                        .connections
                        .iter()
                        .find(|connection| connection.kind == ProviderKind::OpenRouter)
                        .cloned();
                    let is_new = connection.is_none();
                    view.open_connection(connection, window, cx);
                    if is_new {
                        if let Some(form) = &mut view.connection_form {
                            form.original.kind = ProviderKind::OpenRouter;
                        }
                        view.preset(
                            "OpenRouter",
                            ProviderKind::OpenRouter,
                            ProviderKind::OpenRouter.default_url(),
                            Authentication::Bearer,
                            window,
                            cx,
                        );
                    }
                })),
            ),
        );

        let mut advanced = settings_section(
            "settings-advanced",
            "Advanced",
            "Run AI on your own computer (for example with Ollama or LM Studio) or connect a different service. Most people can skip this.",
            cx,
        )
        .child(
            h_flex().child(
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
            ),
        );
        if self.settings_advanced_open {
            advanced = advanced
                .child(Self::connections_header(cx))
                .children(self.connection_cards(cx));
        }

        page.child(
            v_flex()
                .id("settings-sections")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(t.layout.reading_width)
                        .gap(t.space.medium)
                        .child(appearance)
                        .child(ai)
                        .child(advanced),
                ),
        )
        .into_any_element()
    }

    fn render_key_form(
        &self,
        form: &ConnectionForm,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let saved = form.original.has_credential;
        let has_draft = !form.key.read(cx).value().trim().is_empty();
        let saving = self.pending_save.is_some();
        let status = credential_status(&form.original);
        let remove = form.original.clone();
        let mut steps = v_flex().gap(t.space.compact);
        if !saved {
            steps = steps
                .child(caption(
                    "1. Go to openrouter.ai and sign in. Signing up is free.",
                    cx,
                ))
                .child(caption("2. Open “Keys” and create a new key.", cx))
                .child(caption("3. Copy the key and paste it below.", cx));
        }
        v_flex()
            .id("settings-key-form")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .max_w(t.layout.reading_width)
            .gap(t.space.medium)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("Your OpenRouter key"))
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
            .child(steps)
            .child(field(
                "connection-key",
                if saved { "New key" } else { "Paste your key" },
                &form.key,
                if saved {
                    "Only fill this in if you want to swap your key for a different one."
                } else {
                    "It starts with “sk-or-”. Sovra keeps it locked in this computer's keychain and never shows it again."
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
                                    || (!saved && !form.original.environment_credential && !has_draft),
                            )
                            .on_click(cx.listener(|view, _, _, cx| view.submit_connection(true, cx))),
                    )
                    .children(saved.then(|| {
                        design::button("settings-remove-key", "Remove key", cx)
                            .disabled(saving)
                            .on_click(cx.listener(move |view, _, _, cx| {
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
                    .child(back_button(cx)),
            )
            .child(caption(
                "“Check key” only makes sure the key works. It doesn't send a message or cost anything.",
                cx,
            ))
    }

    pub(crate) fn open_connection(
        &mut self,
        connection: Option<ModelConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = connection.unwrap_or_else(|| {
            // Bounded in practice: there are finitely many connections, so a free
            // suffix is always found; the lint cannot see that.
            #[allow(
                clippy::maybe_infinite_iter,
                reason = "terminates at first free suffix"
            )]
            let sequence = (1u64..)
                .find(|n| {
                    !self
                        .state
                        .connections
                        .iter()
                        .any(|c| c.id == format!("connection-{n}"))
                })
                .expect("connection identifiers are finite");
            let mut c = ModelConnection::new(
                &format!("connection-{sequence}"),
                ProviderKind::OpenAiCompatible,
            );
            c.name = "My model server".into();
            c
        });
        let protocol = selector(
            choices(&[
                ("router", "OpenRouter"),
                ("ollama", "Ollama"),
                ("compatible", "OpenAI compatible"),
                ("anthropic", "Anthropic Messages"),
            ]),
            window,
            cx,
        );
        protocol.update(cx, |s, cx| {
            s.set_selected_value(&protocol_value(connection.kind).to_owned(), window, cx);
        });
        let authentication = selector(
            choices(&[
                ("none", "No authentication"),
                ("bearer", "Bearer API key"),
                ("api-key", "x-api-key header"),
            ]),
            window,
            cx,
        );
        authentication.update(cx, |s, cx| {
            s.set_selected_value(
                &auth_value(connection.authentication).to_owned(),
                window,
                cx,
            );
        });
        let form = ConnectionForm {
            name: input(&connection.name, "Connection name", false, window, cx),
            url: input(
                &connection.base_url,
                "https://your-server.example/v1",
                false,
                window,
                cx,
            ),
            key: input("", "Enter API key", true, window, cx),
            key_header: input(&connection.api_key_header, "x-api-key", false, window, cx),
            timeout: input(
                &connection.timeout_seconds.to_string(),
                "60",
                false,
                window,
                cx,
            ),
            organization: input(
                &connection.organization,
                "Optional organization ID",
                false,
                window,
                cx,
            ),
            project: input(
                &connection.project,
                "Optional project ID",
                false,
                window,
                cx,
            ),
            protocol: protocol.clone(),
            authentication,
            original: connection,
            clear_key: false,
            more_options: false,
        };
        self.form_subscriptions.clear();
        for state in [
            &form.name,
            &form.url,
            &form.key,
            &form.key_header,
            &form.timeout,
            &form.organization,
            &form.project,
        ] {
            self.form_subscriptions.push(cx.subscribe_in(
                state,
                window,
                |_, _, _: &InputEvent, _, cx| cx.notify(),
            ));
        }
        self.form_subscriptions.push(cx.subscribe_in(
            &protocol,
            window,
            |view, _, event: &SelectEvent<SearchableVec<Choice>>, window, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let kind = read_protocol(value);
                if let Some(form) = &view.connection_form {
                    form.url
                        .update(cx, |s, cx| s.set_value(kind.default_url(), window, cx));
                    let auth = ModelConnection::new("draft", kind).authentication;
                    form.authentication.update(cx, |s, cx| {
                        s.set_selected_value(&auth_value(auth).to_owned(), window, cx);
                    });
                }
                cx.notify();
            },
        ));
        self.connection_form = Some(form);
        self.manual_form = None;
        self.pending_save = None;
        self.state.error = None;
        self.state.notice = None;
        cx.notify();
    }

    fn preset(
        &mut self,
        name: &str,
        kind: ProviderKind,
        url: &str,
        auth: Authentication,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &self.connection_form {
            form.protocol.update(cx, |s, cx| {
                s.set_selected_value(&protocol_value(kind).to_owned(), window, cx);
            });
            form.name
                .update(cx, |s, cx| s.set_value(name.to_owned(), window, cx));
            form.url
                .update(cx, |s, cx| s.set_value(url.to_owned(), window, cx));
            form.authentication.update(cx, |s, cx| {
                s.set_selected_value(&auth_value(auth).to_owned(), window, cx);
            });
        }
        cx.notify();
    }

    fn submit_connection(&mut self, test: bool, cx: &mut Context<Self>) {
        let Some(form) = &self.connection_form else {
            return;
        };
        match form.read(cx) {
            Ok((connection, credential)) => {
                self.state.error = None;
                if test {
                    self.send(
                        ModelCommand::TestConnection {
                            connection,
                            credential,
                        },
                        cx,
                    );
                } else {
                    self.pending_save = Some(connection.id.clone());
                    self.send(
                        ModelCommand::SaveConnection {
                            connection,
                            credential,
                        },
                        cx,
                    );
                }
            }
            Err(error) => {
                self.state.error = Some(error.0);
                cx.notify();
            }
        }
    }

    pub(crate) fn render_connection_form(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let form = self
            .connection_form
            .as_ref()
            .expect("rendered only when open");
        let kind = read_protocol(
            form.protocol
                .read(cx)
                .selected_value()
                .map_or("compatible", String::as_str),
        );
        let hint = match kind {
            ProviderKind::OpenRouter => "Usually https://openrouter.ai/api/v1.",
            ProviderKind::Ollama => {
                "If Ollama runs on this computer, this is usually http://localhost:11434."
            }
            ProviderKind::OpenAiCompatible => {
                "Copy this from the program you're connecting to. It usually ends in /v1, for example http://localhost:1234/v1 for LM Studio."
            }
            ProviderKind::Anthropic => "Usually https://api.anthropic.com.",
        };
        let credential_hint = if form.original.has_credential {
            "A key is already saved. Leave this empty to keep it, or paste a new one to replace it."
        } else if form.original.environment_credential {
            "A key from this computer's system settings is being used. Paste one here only to override it."
        } else {
            "Leave empty for programs on your own computer. Keys are kept in this computer's keychain."
        };
        let title = if self
            .state
            .connections
            .iter()
            .any(|c| c.id == form.original.id)
        {
            "Edit connection"
        } else {
            "Connect a service"
        };
        let mut content = v_flex()
            .id("model-connection-form")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(t.space.medium)
            .max_w(t.layout.reading_width)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().sovra_text(TextRole::SectionTitle, cx).child(title))
                    .child(back_button(cx)),
            )
            .child(caption(
                "What are you connecting to? Pick one to fill in the details for you.",
                cx,
            ))
            .child(
                h_flex().flex_wrap().gap(t.space.compact).children(
                    [
                        (
                            "OpenRouter",
                            ProviderKind::OpenRouter,
                            "https://openrouter.ai/api/v1",
                            Authentication::Bearer,
                        ),
                        (
                            "Ollama",
                            ProviderKind::Ollama,
                            "http://localhost:11434",
                            Authentication::None,
                        ),
                        (
                            "LM Studio",
                            ProviderKind::OpenAiCompatible,
                            "http://localhost:1234/v1",
                            Authentication::None,
                        ),
                        (
                            "vLLM",
                            ProviderKind::OpenAiCompatible,
                            "http://localhost:8000/v1",
                            Authentication::None,
                        ),
                        (
                            "llama.cpp",
                            ProviderKind::OpenAiCompatible,
                            "http://localhost:8080/v1",
                            Authentication::None,
                        ),
                        (
                            "Anthropic",
                            ProviderKind::Anthropic,
                            "https://api.anthropic.com",
                            Authentication::ApiKey,
                        ),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, kind, url, auth))| {
                        design::button(("connection-preset", index), name, cx)
                            .small()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.preset(name, kind, url, auth, window, cx);
                            }))
                    }),
                ),
            )
            .child(field(
                "connection-name",
                "Name",
                &form.name,
                "Anything that helps you recognise it, like “Home computer”.",
                cx,
            ))
            .child(field("connection-url", "Address", &form.url, hint, cx))
            .child(field(
                "connection-key",
                "Key (only if the service gave you one)",
                &form.key,
                credential_hint,
                cx,
            ));
        if form.original.has_credential {
            content = content.child(
                Checkbox::new("connection-clear-key")
                    .cursor_pointer()
                    .label("Remove the saved key")
                    .checked(form.clear_key)
                    .on_click(cx.listener(|view, checked, _, cx| {
                        if let Some(form) = &mut view.connection_form {
                            form.clear_key = *checked;
                        }
                        cx.notify();
                    })),
            );
        }
        content = content.child(
            h_flex().child(
                design::button(
                    "connection-more-options",
                    if form.more_options {
                        "Hide technical options"
                    } else {
                        "Show technical options"
                    },
                    cx,
                )
                .ghost()
                .small()
                .on_click(cx.listener(|view, _, _, cx| {
                    if let Some(form) = &mut view.connection_form {
                        form.more_options = !form.more_options;
                    }
                    cx.notify();
                })),
            ),
        );
        if form.more_options {
            content = content
                .child(
                    v_flex()
                        .gap(t.space.compact)
                        .child(caption("Service type", cx))
                        .child(select_control(
                            &form.protocol,
                            "Service type",
                            "Choose a type",
                            cx,
                        )),
                )
                .child(
                    v_flex()
                        .gap(t.space.compact)
                        .child(caption("How the key is sent", cx))
                        .child(select_control(
                            &form.authentication,
                            "How the key is sent",
                            "Choose how the key is sent",
                            cx,
                        )),
                )
                .child(field(
                    "connection-timeout",
                    "Wait time (seconds)",
                    &form.timeout,
                    "How long to wait for a reply before giving up. Between 5 and 600.",
                    cx,
                ));
            if matches!(kind, ProviderKind::OpenAiCompatible | ProviderKind::Ollama) {
                content = content.child(field(
                    "connection-key-header",
                    "Key header name",
                    &form.key_header,
                    "Only change this if the service's instructions tell you to. Usually x-api-key.",
                    cx,
                ));
            }
            if matches!(kind, ProviderKind::OpenAiCompatible) {
                content = content
                    .child(field(
                        "connection-organization",
                        "Organization ID",
                        &form.organization,
                        "Only if your service asks for one.",
                        cx,
                    ))
                    .child(field(
                        "connection-project",
                        "Project ID",
                        &form.project,
                        "Only if your service asks for one.",
                        cx,
                    ));
            }
        }
        content
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::primary_button("connection-save", "Save", cx)
                            .disabled(self.pending_save.is_some())
                            .on_click(cx.listener(|view, _, _, cx| view.submit_connection(false, cx))),
                    )
                    .child(
                        design::button("connection-test", "Check connection", cx)
                            .disabled(self.state.busy)
                            .on_click(cx.listener(|view, _, _, cx| view.submit_connection(true, cx))),
                    ),
            )
            .child(caption(
                "“Check connection” only makes sure Sovra can reach the service. It doesn't send a message or cost anything.",
                cx,
            ))
    }

    pub(crate) fn render_connections(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        v_flex()
            .id("model-connections")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(t.space.small)
            .child(Self::connections_header(cx))
            .children(self.connection_cards(cx))
    }

    fn connections_header(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        v_flex()
            .gap(t.space.compact)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().sovra_text(TextRole::SectionTitle, cx).child("Connected services"))
                    .child(
                        design::primary_button("connection-add", "Add connection", cx).on_click(
                            cx.listener(|view, _, window, cx| view.open_connection(None, window, cx)),
                        ),
                    ),
            )
            .child(caption(
                "Each connection is a place Sovra can get AI from. Removing one never deletes anything from that service.",
                cx,
            ))
    }

    fn connection_cards(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let t = design::tokens(cx);
        let mut cards = Vec::new();
        for (index, connection) in self.state.connections.iter().enumerate() {
            let edit = connection.clone();
            let refresh_id = connection.id.clone();
            let remove_id = connection.id.clone();
            let manual_id = connection.id.clone();
            let status = self.state.catalogs.get(&connection.id).map_or_else(
                || "Not checked yet".to_owned(),
                |catalog| {
                    if catalog.loading {
                        "Checking…".to_owned()
                    } else if catalog.error.is_some() {
                        "Can't connect right now · choose Edit to fix it".to_owned()
                    } else {
                        let count = catalog.snapshot.models.len();
                        format!(
                            "{count} AI model{} available{}",
                            if count == 1 { "" } else { "s" },
                            if catalog.cached {
                                " · last known list"
                            } else {
                                ""
                            }
                        )
                    }
                },
            );
            let key_status = if connection.has_credential {
                "Key saved on this computer"
            } else if connection.environment_credential {
                "Key from this computer's system settings"
            } else {
                "No key"
            };
            let mut card = v_flex()
                .p(t.space.medium)
                .gap(t.space.small)
                .sovra_surface(Surface::Matte, cx)
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.small)
                        .justify_between()
                        .child(
                            v_flex()
                                .gap(t.space.compact)
                                .child(
                                    div()
                                        .sovra_text(TextRole::SectionTitle, cx)
                                        .child(connection.name.clone()),
                                )
                                .child(caption(status, cx))
                                .child(caption(key_status, cx)),
                        )
                        .child(
                            design::button(("connection-edit", index), "Edit", cx).on_click(
                                cx.listener(move |view, _, window, cx| {
                                    view.open_connection(Some(edit.clone()), window, cx);
                                }),
                            ),
                        ),
                )
                .child(caption(connection.base_url.clone(), cx));
            let mut actions = h_flex().flex_wrap().gap(t.space.compact).child(
                design::button(("connection-refresh", index), "Refresh list", cx).on_click(
                    cx.listener(move |view, _, _, cx| {
                        view.send(
                            ModelCommand::Refresh {
                                connection_id: refresh_id.clone(),
                            },
                            cx,
                        );
                    }),
                ),
            );
            if connection.kind != ProviderKind::OpenRouter {
                actions = actions.child(
                    design::button(("manual-model-add", index), "Add a model by name", cx)
                        .ghost()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_manual_model(&manual_id, window, cx);
                        })),
                );
            }
            actions = actions.child(
                design::button(("connection-remove", index), "Remove", cx)
                    .ghost()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.delete_confirmation = Some(remove_id.clone());
                        cx.notify();
                    })),
            );
            card = card.child(actions);
            if self.delete_confirmation.as_deref() == Some(&connection.id) {
                let id = connection.id.clone();
                card = card
                    .child(
                        caption(
                            "Remove this connection and its saved key? Nothing is deleted from the service itself.",
                            cx,
                        )
                        .text_color(cx.theme().warning),
                    )
                    .child(
                        h_flex()
                            .gap(t.space.small)
                            .child(
                                design::button(("connection-confirm-remove", index), "Remove", cx)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.send(ModelCommand::DeleteConnection { id: id.clone() }, cx);
                                        view.delete_confirmation = None;
                                    })),
                            )
                            .child(
                                design::button(("connection-cancel-remove", index), "Keep it", cx)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.delete_confirmation = None;
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            cards.push(card.into_any_element());
        }
        cards
    }

    pub(crate) fn open_manual_model(
        &mut self,
        connection_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.manual_form = Some(ManualModelForm {
            connection_id: connection_id.to_owned(),
            id: input("", "Exact ID from your server", false, window, cx),
            name: input("", "Optional display name", false, window, cx),
            context: input("", "Unknown", false, window, cx),
            tools: false,
        });
        self.state.error = None;
        cx.notify();
    }

    fn save_manual_model(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.manual_form else {
            return;
        };
        let context = form.context.read(cx).value();
        let context_length = if context.trim().is_empty() {
            None
        } else {
            let Some(context) = context
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
            else {
                self.state.error = Some(
                    "Context length must be a positive number of tokens, or left blank.".into(),
                );
                cx.notify();
                return;
            };
            Some(context)
        };
        let model = ModelInfo {
            id: form.id.read(cx).value().trim().to_owned(), name: form.name.read(cx).value().trim().to_owned(),
            author: "Custom".into(), description: "Manually configured chat model. Capabilities and context are supplied by you; pricing is not reported.".into(),
            context_length, input_modalities:vec!["text".into()],output_modalities:vec!["text".into()],
            supported_parameters: if form.tools { vec!["tools".into()] } else { Vec::new() }, manual:true, details_loaded:true,
            ..ModelInfo::default()
        };
        self.pending_save = Some(form.connection_id.clone());
        self.send(
            ModelCommand::AddManualModel {
                connection_id: form.connection_id.clone(),
                model: Box::new(model),
            },
            cx,
        );
    }

    pub(crate) fn render_manual_form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let form = self.manual_form.as_ref().expect("manual form is open");
        v_flex().id("manual-model-form").test_support().flex_1().min_h_0().overflow_y_scroll().gap(t.space.medium).max_w(t.layout.reading_width)
            .child(div().sovra_text(TextRole::SectionTitle,cx).child("Add a model by name"))
            .child(caption("Use this if a model you know is there doesn't show up in the list. Type its name exactly as the service shows it.",cx))
            .child(field("manual-model-id","Model name on the service",&form.id,"Copy it exactly, for example qwen3:8b.",cx))
            .child(field("manual-model-name","Friendly name",&form.name,"Optional. What Sovra should call it.",cx))
            .child(field("manual-model-context","Memory size (tokens)",&form.context,"Optional. How much text the model can keep in mind at once. Leave empty if you don't know.",cx))
            .child(Checkbox::new("manual-model-tools").cursor_pointer().label("This model can use tools").checked(form.tools)
                .on_click(cx.listener(|view,checked,_,cx| { if let Some(form) = &mut view.manual_form { form.tools = *checked; } cx.notify(); })))
            .child(h_flex().gap(t.space.small)
                .child(design::primary_button("manual-model-save","Add model",cx).disabled(self.pending_save.is_some()).on_click(cx.listener(|view,_,_,cx| view.save_manual_model(cx))))
                .child(design::button("manual-model-cancel","Cancel",cx).on_click(cx.listener(|view,_,_,cx| { view.manual_form = None; cx.notify(); }))))
    }

    pub(crate) fn render_download(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex().p(t.space.small).gap(t.space.compact).sovra_surface(Surface::Subtle,cx)
            .child(caption("Download a model to run on your own computer",cx))
            .child(h_flex().flex_wrap().gap(t.space.compact)
                .child(select_control(&self.download_connection,"Download to","Choose where to download",cx).w(t.layout.filter_width))
                .child(Input::new(&self.download_name).id("ollama-model-tag").aria_label("Name of the model to download").flex_1())
                .child(design::button("ollama-pull", "Download", cx).disabled(self.state.pull_progress.is_some())
                    .on_click(cx.listener(|view,_,_,cx| {
                        if let Some(connection_id) = view.download_connection.read(cx).selected_value().cloned() {
                            let model = view.download_name.read(cx).value().to_string();
                            view.send(ModelCommand::PullOllama { connection_id,model },cx);
                        } else { view.state.error = Some("Set up Ollama first: open Settings → Advanced and add an Ollama connection.".into()); cx.notify(); }
                    }))))
            .child(caption("Find model names at ollama.com/library. Downloads can be several gigabytes, so this may take a while. Finished models appear below.",cx));
        if let Some(progress) = &self.state.pull_progress {
            panel = panel.child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(caption(progress.clone(), cx))
                    .child(
                        design::button("ollama-cancel-pull", "Cancel download", cx)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send(ModelCommand::CancelPull, cx);
                            })),
                    ),
            );
        }
        panel
    }
}
