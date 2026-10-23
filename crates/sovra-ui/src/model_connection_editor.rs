//! Connection and manual-model forms. Credentials stay masked and leave the UI
//! only through the model command protocol.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
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
    Authentication, CredentialChange, ModelCommand, ModelConnection, ModelError, ModelInfo,
    ModelSecret, ProviderKind,
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

impl ModelsView {
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
            ProviderKind::OpenRouter => {
                "https://openrouter.ai/api/v1 · API key required for inference; catalog browsing is public."
            }
            ProviderKind::Ollama => {
                "Server root, usually http://localhost:11434. Local Ollama needs no key. Remote proxies may require Bearer authentication."
            }
            ProviderKind::OpenAiCompatible => {
                "Include the API prefix, usually /v1. LM Studio: localhost:1234/v1 · vLLM: localhost:8000/v1 · llama.cpp: localhost:8080/v1."
            }
            ProviderKind::Anthropic => {
                "Server root, usually https://api.anthropic.com. Sovra adds /v1/models and /v1/messages plus the required version header."
            }
        };
        let credential_hint = if form.original.environment_credential {
            "An environment key is available for the original endpoint. Enter a key here to save an override in the OS credential store."
        } else if form.original.has_credential {
            "A key is saved in the OS credential store. Leave blank to keep it, or enter a replacement."
        } else {
            "Optional for unauthenticated local servers. Keys are saved in the operating system credential store, never in config files."
        };
        let mut content = v_flex().id("model-connection-form").test_support().flex_1().min_h_0().overflow_y_scroll().gap(t.space.medium).max_w(t.layout.reading_width)
            .child(h_flex().justify_between().items_center()
                .child(div().sovra_text(TextRole::SectionTitle,cx).child("Connect a model provider"))
                .child(design::button("connection-cancel","Back",cx).on_click(cx.listener(|view,_,_,cx| { view.connection_form = None; view.form_subscriptions.clear(); cx.notify(); }))))
            .child(caption("Start with a preset, then adjust it for your server.",cx))
            .child(h_flex().flex_wrap().gap(t.space.compact).children([
                ("OpenRouter",ProviderKind::OpenRouter,"https://openrouter.ai/api/v1",Authentication::Bearer),
                ("Ollama",ProviderKind::Ollama,"http://localhost:11434",Authentication::None),
                ("LM Studio",ProviderKind::OpenAiCompatible,"http://localhost:1234/v1",Authentication::None),
                ("vLLM",ProviderKind::OpenAiCompatible,"http://localhost:8000/v1",Authentication::None),
                ("llama.cpp",ProviderKind::OpenAiCompatible,"http://localhost:8080/v1",Authentication::None),
                ("Anthropic",ProviderKind::Anthropic,"https://api.anthropic.com",Authentication::ApiKey),
            ].into_iter().enumerate().map(|(index,(name,kind,url,auth))| design::button(("connection-preset",index), name, cx).small()
                .on_click(cx.listener(move |view,_,window,cx| view.preset(name,kind,url,auth,window,cx))))))
            .child(field("connection-name","Connection name",&form.name,"A recognizable name, such as Work GPU or Home Ollama.",cx))
            .child(v_flex().gap(t.space.compact).child(caption("API protocol",cx)).child(select_control(&form.protocol,"API protocol","Choose a protocol",cx)))
            .child(field("connection-url","Base URL",&form.url,hint,cx))
            .child(v_flex().gap(t.space.compact).child(caption("Authentication",cx)).child(select_control(&form.authentication,"Authentication mode","Choose authentication",cx)))
            .child(field("connection-key","API key",&form.key,credential_hint,cx))
            .child(Checkbox::new("connection-clear-key").label("Clear the saved key").checked(form.clear_key).on_click(cx.listener(|view,checked,_,cx| {
                if let Some(form) = &mut view.connection_form { form.clear_key = *checked; } cx.notify();
            })))
            .child(field("connection-timeout","Timeout in seconds",&form.timeout,"5–600 seconds. Applies to discovery requests and the complete chat response; downloads use their own progress timeout.",cx));
        if matches!(kind, ProviderKind::OpenAiCompatible | ProviderKind::Ollama) {
            content = content.child(field("connection-key-header", "API key header name", &form.key_header, "Used only for API key header authentication. Defaults to x-api-key; some proxies use api-key.", cx));
        }
        if matches!(kind, ProviderKind::OpenAiCompatible) {
            content = content
                .child(field(
                    "connection-organization",
                    "Organization ID",
                    &form.organization,
                    "Optional OpenAI-Organization header, only when your provider requires it.",
                    cx,
                ))
                .child(field(
                    "connection-project",
                    "Project ID",
                    &form.project,
                    "Optional OpenAI-Project header, only when your provider requires it.",
                    cx,
                ));
        }
        content.child(h_flex().flex_wrap().gap(t.space.small)
            .child(design::button("connection-test","Test connection",cx).disabled(self.state.busy).on_click(cx.listener(|view,_,_,cx| view.submit_connection(true,cx))))
            .child(design::primary_button("connection-save","Save connection",cx).disabled(self.pending_save.is_some()).on_click(cx.listener(|view,_,_,cx| view.submit_connection(false,cx)))))
            .child(caption("Testing verifies the connection and model discovery without sending a paid inference request. Servers without model discovery can still be saved and configured with a manual model ID.",cx))
    }

    pub(crate) fn render_connections(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut list = v_flex().id("model-connections").test_support().flex_1().min_h_0().overflow_y_scroll().gap(t.space.small)
            .child(h_flex().justify_between().items_center()
                .child(div().sovra_text(TextRole::SectionTitle,cx).child("Your connections"))
                .child(design::primary_button("connection-add","Add connection",cx).on_click(cx.listener(|view,_,window,cx| view.open_connection(None,window,cx)))))
            .child(caption("Local and remote servers share one library. Credentials stay in your OS credential store; removing a connection never removes models from its server.",cx));
        for (index, connection) in self.state.connections.iter().enumerate() {
            let edit = connection.clone();
            let refresh_id = connection.id.clone();
            let remove_id = connection.id.clone();
            let manual_id = connection.id.clone();
            let status =
                self.state
                    .catalogs
                    .get(&connection.id)
                    .map_or("Not loaded".into(), |catalog| {
                        if catalog.loading {
                            "Refreshing…".into()
                        } else if catalog.error.is_some() {
                            "Connection needs attention".into()
                        } else {
                            format!(
                                "{} models · {}",
                                catalog.snapshot.models.len(),
                                if catalog.cached {
                                    "cached catalog"
                                } else {
                                    "live catalog"
                                }
                            )
                        }
                    });
            let mut card = v_flex()
                .p(t.space.medium)
                .gap(t.space.small)
                .sovra_surface(Surface::Subtle, cx)
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
                                .child(caption(
                                    format!("{} · {status}", connection.kind.label()),
                                    cx,
                                )),
                        )
                        .child(
                            design::button(("connection-edit", index), "Edit", cx).on_click(
                                cx.listener(move |view, _, window, cx| {
                                    view.open_connection(Some(edit.clone()), window, cx);
                                }),
                            ),
                        ),
                )
                .child(caption(connection.base_url.clone(), cx))
                .child(caption(
                    if connection.has_credential {
                        "API key saved in OS credential store"
                    } else if connection.environment_credential {
                        "API key supplied by environment"
                    } else {
                        "No saved API key"
                    },
                    cx,
                ))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(t.space.compact)
                        .child(
                            design::button(("connection-refresh", index), "Discover models", cx)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.send(
                                        ModelCommand::Refresh {
                                            connection_id: refresh_id.clone(),
                                        },
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            design::button(("connection-remove", index), "Remove", cx)
                                .ghost()
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.delete_confirmation = Some(remove_id.clone());
                                    cx.notify();
                                })),
                        ),
                );
            if connection.kind != ProviderKind::OpenRouter {
                card = card.child(
                    design::button(("manual-model-add", index), "Add model by ID", cx)
                        .ghost()
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_manual_model(&manual_id, window, cx);
                        })),
                );
            }
            if self.delete_confirmation.as_deref() == Some(&connection.id) {
                let id = connection.id.clone();
                card = card.child(caption("Remove this connection and its saved key? Models on the server remain untouched.",cx).text_color(cx.theme().warning))
                    .child(h_flex().gap(t.space.small)
                        .child(design::button(("connection-confirm-remove",index), "Remove connection", cx).on_click(cx.listener(move |view,_,_,cx| { view.send(ModelCommand::DeleteConnection { id:id.clone() },cx); view.delete_confirmation = None; })))
                        .child(design::button(("connection-cancel-remove",index), "Keep connection", cx).on_click(cx.listener(|view,_,_,cx| { view.delete_confirmation = None; cx.notify(); }))));
            }
            list = list.child(card);
        }
        list
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
            .child(div().sovra_text(TextRole::SectionTitle,cx).child("Add a chat model by ID"))
            .child(caption("Use this when a server does not expose model discovery, or to declare capabilities for a discovered model. The ID must match the server exactly.",cx))
            .child(field("manual-model-id","Model ID",&form.id,"Use the server's deployment/model identifier, not a friendly marketing name.",cx))
            .child(field("manual-model-name","Display name",&form.name,"Optional; defaults to the model ID.",cx))
            .child(field("manual-model-context","Context length",&form.context,"Optional maximum context in tokens. Leave blank when unknown.",cx))
            .child(Checkbox::new("manual-model-tools").label("This model and server support tool calling").checked(form.tools)
                .on_click(cx.listener(|view,checked,_,cx| { if let Some(form) = &mut view.manual_form { form.tools = *checked; } cx.notify(); })))
            .child(h_flex().gap(t.space.small)
                .child(design::primary_button("manual-model-save","Add model",cx).disabled(self.pending_save.is_some()).on_click(cx.listener(|view,_,_,cx| view.save_manual_model(cx))))
                .child(design::button("manual-model-cancel","Cancel",cx).on_click(cx.listener(|view,_,_,cx| { view.manual_form = None; cx.notify(); }))))
    }

    pub(crate) fn render_download(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut panel = v_flex().p(t.space.small).gap(t.space.compact).sovra_surface(Surface::Subtle,cx)
            .child(caption("OLLAMA · INSTALL A MODEL",cx))
            .child(h_flex().flex_wrap().gap(t.space.compact)
                .child(select_control(&self.download_connection,"Ollama download destination","Choose Ollama server",cx).w(t.layout.filter_width))
                .child(Input::new(&self.download_name).id("ollama-model-tag").aria_label("Ollama model tag to download").flex_1())
                .child(design::button("ollama-pull", "Download", cx).disabled(self.state.pull_progress.is_some())
                    .on_click(cx.listener(|view,_,_,cx| {
                        if let Some(connection_id) = view.download_connection.read(cx).selected_value().cloned() {
                            let model = view.download_name.read(cx).value().to_string();
                            view.send(ModelCommand::PullOllama { connection_id,model },cx);
                        } else { view.state.error = Some("Add an Ollama connection first.".into()); cx.notify(); }
                    }))))
            .child(caption("Downloads run on the selected Ollama server and can be several GB. Already installed models appear below after discovery.",cx));
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
