//! Connected services: add web addresses, programs on this computer, or pasted
//! settings, then manage many of them: pause, choose which tools Sovra may use,
//! reconnect, rename and remove.
//!
//! The view only shows what the runtime reports and sends commands. It never
//! marks a service connected on its own, and a program never runs until the
//! person has read its exact command and pressed **Run it**.

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div,
};
use sovra_core::Command;
use sovra_core::mcp::{
    DraftSetup, McpCommand, McpPhase, McpService, McpTransport, parse_server_settings,
    program_from_command_line, program_notes,
};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::model_connection_editor::field;
use crate::models_view::{ModelsView, caption};
use crate::settings_view::settings_section;

/// Show the search box once the list is longer than this.
const SEARCH_AFTER: usize = 4;

/// Which way of adding a service is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddMode {
    Closed,
    Web,
    Program,
    Paste,
}

/// The services screen's own form state. Service state comes from the runtime.
pub(crate) struct ServicesScreen {
    pub(crate) mode: AddMode,
    pub(crate) query: Entity<InputState>,
    pub(crate) program_name: Entity<InputState>,
    pub(crate) program_command: Entity<InputState>,
    pub(crate) program_settings: Entity<TextareaState>,
    pub(crate) program_folder: Entity<InputState>,
    pub(crate) paste: Entity<TextareaState>,
    pub(crate) rename: Entity<InputState>,
    pub(crate) renaming: Option<String>,
    pub(crate) confirm_remove: Option<String>,
    pub(crate) expanded: HashSet<String>,
    /// A problem with what was typed, shown under the add form.
    pub(crate) form_problem: Option<String>,
}

impl ServicesScreen {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        Self {
            mode: AddMode::Closed,
            query: cx.new(|cx| InputState::new(window, cx).placeholder("Search servers and tools")),
            program_name: cx
                .new(|cx| InputState::new(window, cx).placeholder("Name, if you want one")),
            program_command: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("npx -y @modelcontextprotocol/server-filesystem ~/Documents")
            }),
            program_settings: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("One per line, like GITHUB_TOKEN=ghp_…")
                    .auto_grow(2, 6)
            }),
            program_folder: cx.new(|cx| InputState::new(window, cx).placeholder("Optional")),
            paste: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("{ \"mcpServers\": { … } }")
                    .auto_grow(4, 14)
            }),
            rename: cx.new(|cx| InputState::new(window, cx).placeholder("New name")),
            renaming: None,
            confirm_remove: None,
            expanded: HashSet::new(),
            form_problem: None,
        }
    }

    /// Inputs whose edits should re-render the screen (search, enabled buttons).
    pub(crate) fn live_inputs(&self) -> [Entity<InputState>; 2] {
        [self.query.clone(), self.program_command.clone()]
    }
}

fn send(cx: &mut gpui_kit::App, command: McpCommand) {
    let _ = cx.global::<RuntimeClient>().0.send(Command::Mcp(command));
}

fn sid(prefix: &str, id: &str) -> SharedString {
    SharedString::from(format!("{prefix}-{id}"))
}

/// `KEY=value` lines; blank lines and `#` comments are skipped.
fn settings_lines(text: &str) -> Result<Vec<(String, String)>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            line.split_once('=')
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .filter(|(name, _)| sovra_core::mcp::env_name_allowed(name))
                .ok_or_else(|| format!("“{line}” isn't a setting. Write each one as NAME=value."))
        })
        .collect()
}

impl ModelsView {
    pub(crate) fn render_service_connect_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let state = &self.services;
        let summary = state.summary();
        let screen = &self.services_screen;
        let mode = if state.services.is_empty() && screen.mode == AddMode::Closed {
            // With nothing added yet, the form is the whole point of the screen.
            AddMode::Web
        } else {
            screen.mode
        };
        let mut status_bits = Vec::new();
        if summary.working > 0 {
            status_bits.push(format!("{} working", summary.working));
        }
        if summary.needs_you > 0 {
            status_bits.push(format!("{} need you", summary.needs_you));
        }
        if summary.paused > 0 {
            status_bits.push(format!("{} paused", summary.paused));
        }
        let header = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(t.space.small)
            .child(
                div()
                    .id("mcp-summary")
                    .test_support()
                    .min_w_0()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(if status_bits.is_empty() {
                        "No MCP servers yet.".to_owned()
                    } else {
                        status_bits.join(" · ")
                    }),
            )
            .child(if mode == AddMode::Closed {
                design::primary_button("mcp-add-open", "Add MCP server", cx)
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.services_screen.mode = AddMode::Web;
                        cx.notify();
                    }))
                    .into_any_element()
            } else if state.services.is_empty() {
                div().into_any_element()
            } else {
                design::button("mcp-add-close", "Close", cx)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.services_screen.mode = AddMode::Closed;
                        view.services_screen.form_problem = None;
                        cx.notify();
                    }))
                    .into_any_element()
            });
        let mut section = settings_section(
            "mcp-connect-section",
            "MCP servers",
            "MCP servers give Sovra extra tools, like reading your files, checking a calendar or searching a database. Add one, then choose which of its tools Sovra may use.",
            cx,
        )
        .child(header);
        let has_sample = state.services.iter().any(|service| {
            service
                .address
                .ends_with(sovra_core::mcp::SAMPLE_SERVER_FLAG)
        });
        if !has_sample && (state.services.is_empty() || mode != AddMode::Closed) {
            section = section.child(Self::render_sample_offer(cx));
        }
        if mode != AddMode::Closed {
            section = section.child(self.render_add_panel(mode, cx));
        }
        if let Some(notice) = &state.notice {
            section = section.child(
                div()
                    .id("mcp-notice")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .child(notice.clone()),
            );
        }
        if state.services.len() > SEARCH_AFTER {
            section = section.child(
                Input::new(&screen.query)
                    .id("mcp-search")
                    .aria_label("Search servers and tools")
                    .w_full(),
            );
        }
        let query = screen.query.read(cx).value().to_string();
        let shown = state.matching(&query);
        if shown.is_empty() && !state.services.is_empty() {
            section = section.child(caption("No server or tool matches that search.", cx));
        }
        let cards: Vec<_> = shown
            .into_iter()
            .map(|service| self.render_service_card(service, cx).into_any_element())
            .collect();
        section.child(
            v_flex()
                .id("mcp-service-list")
                .test_support()
                .w_full()
                .min_w_0()
                .gap(t.space.small)
                .children(cards),
        )
    }

    /// A no-setup way to see what an MCP server does: Sovra's own sample.
    fn render_sample_offer(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        h_flex()
            .id("mcp-sample-offer")
            .test_support()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(t.space.hair)
                    .child(
                        div()
                            .sovra_text(TextRole::Body, cx)
                            .child("New to MCP servers? Try the sample"),
                    )
                    .child(
                        div()
                            .whitespace_normal()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(t.text_secondary)
                            .child(
                                "It's built into Sovra, needs no setup and can't touch your files. \
                                 It adds four simple tools: roll a dice, tell the time, make a checklist \
                                 and count words.",
                            ),
                    ),
            )
            .child(
                design::button("mcp-add-sample", "Try the sample MCP server", cx)
                    .icon(IconName::Sparkles)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.services.notice = None;
                        view.services_screen.mode = AddMode::Closed;
                        send(cx, McpCommand::AddSample);
                        cx.notify();
                    })),
            )
    }

    fn render_add_panel(&self, mode: AddMode, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let tabs = h_flex().w_full().flex_wrap().gap(t.space.compact).children(
            [
                (AddMode::Web, "mcp-mode-web", "From a web address"),
                (
                    AddMode::Program,
                    "mcp-mode-program",
                    "From a program on this computer",
                ),
                (
                    AddMode::Paste,
                    "mcp-mode-paste",
                    "Paste setup from instructions",
                ),
            ]
            .map(|(tab, id, label)| {
                design::button(id, label, cx)
                    .small()
                    .selected(tab == mode)
                    .accessibility_label(format!("Add: {label}"))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.services_screen.mode = tab;
                        view.services_screen.form_problem = None;
                        cx.notify();
                    }))
            }),
        );
        let body = match mode {
            AddMode::Web | AddMode::Closed => self.render_web_form(cx).into_any_element(),
            AddMode::Program => self.render_program_form(cx).into_any_element(),
            AddMode::Paste => self.render_paste_form(cx).into_any_element(),
        };
        let mut panel = v_flex()
            .id("mcp-add-panel")
            .test_support()
            .w_full()
            .min_w_0()
            .p(t.space.medium)
            .gap(t.space.small)
            .sovra_surface(Surface::Subtle, cx)
            .child(tabs)
            .child(body);
        if let Some(problem) = &self.services_screen.form_problem {
            panel = panel.child(
                div()
                    .id("mcp-form-problem")
                    .test_support()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        panel
    }

    fn render_web_form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let address_empty = self
            .service_address
            .read(cx)
            .value()
            .to_string()
            .trim()
            .is_empty();
        v_flex()
            .w_full()
            .gap(t.space.small)
            .child(field(
                "mcp-service-name",
                "Name",
                &self.service_name,
                "What you'd like to call it. Leave blank to use the server's own name.",
                cx,
            ))
            .child(field(
                "mcp-service-address",
                "Server address (URL)",
                &self.service_address,
                "Starts with https://. Copy it from the server's instructions. If it asks you to sign in, a browser window opens.",
                cx,
            ))
            .child(field(
                "mcp-service-secret",
                "Key or token (optional)",
                &self.service_secret,
                "Only if the instructions gave you one. It's kept in your keychain.",
                cx,
            ))
            .child(
                h_flex().flex_wrap().child(
                    design::primary_button("mcp-connect", "Add server", cx)
                        .disabled(address_empty)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.connect_service(window, cx)),
                        ),
                ),
            )
    }

    fn render_program_form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let screen = &self.services_screen;
        let command_empty = screen.program_command.read(cx).value().trim().is_empty();
        v_flex()
            .w_full()
            .gap(t.space.small)
            .child(field(
                "mcp-program-name",
                "Name",
                &screen.program_name,
                "Leave blank and Sovra names it for you.",
                cx,
            ))
            .child(field(
                "mcp-program-command",
                "Command to start it",
                &screen.program_command,
                "Copy it from the server's instructions (it often starts with npx or uvx). Nothing runs until you've checked it.",
                cx,
            ))
            .child(
                v_flex()
                    .w_full()
                    .gap(t.space.compact)
                    .child(caption("Keys or tokens (optional)", cx))
                    .child(
                        div().id("mcp-program-settings").test_support().w_full().child(
                            Textarea::new(&screen.program_settings)
                                .aria_label("Private settings")
                                .w_full(),
                        ),
                    )
                    .child(caption(
                        "Only if the instructions ask for them, one per line like NAME=value. Kept in your keychain, never shown again.",
                        cx,
                    )),
            )
            .child(field(
                "mcp-program-folder",
                "Folder to start in",
                &screen.program_folder,
                "Optional. Most programs don't need one.",
                cx,
            ))
            .child(
                h_flex().flex_wrap().child(
                    design::primary_button("mcp-add-program", "Add server", cx)
                        .disabled(command_empty)
                        .on_click(cx.listener(|view, _, window, cx| view.add_program(window, cx))),
                ),
            )
    }

    fn render_paste_form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        v_flex()
            .w_full()
            .gap(t.space.small)
            .child(caption(
                "Paste the setup from the server's instructions (the part with “mcpServers”), or settings copied from Claude, Cursor or VS Code. Sovra adds every server it finds.",
                cx,
            ))
            .child(
                div().id("mcp-paste").test_support().w_full().child(
                    Textarea::new(&self.services_screen.paste)
                        .aria_label("Settings to paste")
                        .w_full(),
                ),
            )
            .child(
                h_flex().flex_wrap().child(
                    design::primary_button("mcp-import", "Add servers", cx)
                        .on_click(cx.listener(|view, _, window, cx| view.import_services(window, cx))),
                ),
            )
    }

    fn status_tone(service: &McpService, cx: &Context<Self>) -> Hsla {
        let theme = cx.theme();
        if !service.enabled {
            return design::tokens(cx).text_secondary;
        }
        match service.phase {
            McpPhase::Connected => theme.success,
            McpPhase::Failed => theme.danger,
            McpPhase::NeedsSignIn | McpPhase::NeedsClientDetails | McpPhase::NeedsApproval => {
                theme.warning
            }
            McpPhase::Saved | McpPhase::Connecting => design::tokens(cx).text_secondary,
        }
    }

    fn render_service_card(
        &self,
        service: &McpService,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let screen = &self.services_screen;
        let id = service.id.clone();
        let tone = Self::status_tone(service, cx);
        let mono = cx.theme().mono_font_family.clone();
        let label = service.label();

        let enabled_id = id.clone();
        let title_row = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(t.space.small)
            .child(
                Icon::new(if service.transport.is_program() {
                    IconName::SquareTerminal
                } else {
                    IconName::Globe
                })
                .small()
                .text_color(t.text_secondary)
                .flex_shrink_0(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .sovra_text(TextRole::Body, cx)
                    .child(service.name.clone()),
            )
            .child(
                div()
                    .id(sid("mcp-status", &id))
                    .test_support()
                    .aria_label(label)
                    .flex_shrink_0()
                    .px(t.space.small)
                    .py(t.space.hair)
                    .rounded(t.radius.pill)
                    .bg(tone.alpha(0.12))
                    .text_color(tone)
                    .sovra_text(TextRole::Caption, cx)
                    .child(label),
            )
            .child(
                Switch::new(sid("mcp-enabled", &id))
                    .checked(service.enabled)
                    .small()
                    .tooltip(if service.enabled {
                        "Pause this service"
                    } else {
                        "Turn this service back on"
                    })
                    .on_click(cx.listener(move |_, checked: &bool, _, cx| {
                        send(
                            cx,
                            McpCommand::SetEnabled {
                                id: enabled_id.clone(),
                                enabled: *checked,
                            },
                        );
                    })),
            );

        let reviewing = service.enabled && service.phase == McpPhase::NeedsApproval;
        let mut card = v_flex()
            .id(sid("mcp-service", &id))
            .test_support()
            .w_full()
            .min_w_0()
            .p(t.space.medium)
            .gap(t.space.compact)
            .sovra_surface(Surface::Subtle, cx)
            .child(title_row)
            .when(!reviewing, |card| {
                card.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .truncate()
                        .font_family(mono)
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .child(service.location()),
                )
            });

        let show_message = service.enabled
            && service.phase != McpPhase::Connected
            && service.phase != McpPhase::NeedsApproval
            && !service.message.is_empty();
        if show_message {
            card = card.child(
                div()
                    .id(sid("mcp-message", &id))
                    .test_support()
                    .aria_label(service.message.clone())
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(if service.phase == McpPhase::Failed {
                        cx.theme().danger
                    } else {
                        t.text_secondary
                    })
                    .child(service.message.clone()),
            );
        }

        if service.enabled && service.phase == McpPhase::NeedsApproval {
            card = card.child(Self::render_review(service, cx));
        }
        if service.enabled && service.phase == McpPhase::NeedsSignIn {
            let sign_in = id.clone();
            card = card.child(
                h_flex().child(
                    design::primary_button(sid("mcp-sign-in", &id), "Sign in", cx)
                        .small()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            send(
                                cx,
                                McpCommand::SignIn {
                                    id: sign_in.clone(),
                                },
                            );
                        })),
                ),
            );
        }
        if service.enabled && service.phase == McpPhase::NeedsClientDetails {
            let continue_id = id.clone();
            card = card
                .child(field(
                    "mcp-client-id",
                    "Sign-in name",
                    &self.service_client_id,
                    "Paste the name the server gave you.",
                    cx,
                ))
                .child(field(
                    "mcp-client-secret",
                    "Password",
                    &self.service_client_secret,
                    "Only if the server gave you one. It's kept in your keychain.",
                    cx,
                ))
                .child(
                    h_flex().child(
                        design::primary_button(sid("mcp-continue", &id), "Continue", cx)
                            .small()
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.continue_sign_in(&continue_id, window, cx);
                            })),
                    ),
                );
        }

        if service.phase == McpPhase::Connected && !service.tools.is_empty() {
            card = card.child(self.render_tools(service, cx));
        }

        if screen.renaming.as_deref() == Some(id.as_str()) {
            let save_id = id.clone();
            card = card.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.compact)
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&screen.rename)
                                .id("mcp-rename-input")
                                .aria_label("New name")
                                .w_full(),
                        ),
                    )
                    .child(
                        design::button(sid("mcp-rename-save", &id), "Save", cx)
                            .small()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let name = view.services_screen.rename.read(cx).value().to_string();
                                send(
                                    cx,
                                    McpCommand::Rename {
                                        id: save_id.clone(),
                                        name,
                                    },
                                );
                                view.services_screen.renaming = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        design::button(sid("mcp-rename-cancel", &id), "Cancel", cx)
                            .small()
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.services_screen.renaming = None;
                                cx.notify();
                            })),
                    ),
            );
        }

        card.child(self.render_card_actions(service, cx))
    }

    /// Shown before a program first runs: the exact, untruncated command and
    /// what it means, then **Run it**.
    fn render_review(service: &McpService, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let id = service.id.clone();
        let mono = cx.theme().mono_font_family.clone();
        let notes = program_notes(&service.transport);
        let McpTransport::Program {
            env_names,
            working_dir,
            ..
        } = &service.transport
        else {
            return div().into_any_element();
        };
        let mut review = v_flex()
            .id(sid("mcp-review", &id))
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child("Check this program before it runs"),
            )
            .child(
                div()
                    .id(sid("mcp-review-command", &id))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.small)
                    .rounded(t.chat.code_radius)
                    .bg(t.code_background)
                    .font_family(mono)
                    .sovra_text(TextRole::Caption, cx)
                    .whitespace_normal()
                    .child(service.address.clone()),
            );
        for note in notes {
            let warning = note.is_warning();
            review = review.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap(t.space.compact)
                    .text_color(if warning {
                        cx.theme().warning
                    } else {
                        t.text_secondary
                    })
                    .child(
                        Icon::new(if warning {
                            IconName::TriangleAlert
                        } else {
                            IconName::Info
                        })
                        .xsmall()
                        .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .sovra_text(TextRole::Caption, cx)
                            .child(note.text()),
                    ),
            );
        }
        if !env_names.is_empty() {
            review = review.child(caption(
                format!("Private settings it gets: {}", env_names.join(", ")),
                cx,
            ));
        }
        if let Some(dir) = working_dir {
            review = review.child(caption(format!("Starts in: {dir}"), cx));
        }
        let run_id = id.clone();
        review
            .child(
                h_flex().flex_wrap().gap(t.space.small).child(
                    design::primary_button(sid("mcp-approve", &id), "Run it", cx)
                        .small()
                        .accessibility_label(format!("Run {}: {}", service.name, service.address))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            send(cx, McpCommand::Approve { id: run_id.clone() });
                        })),
                ),
            )
            .child(caption(
                "Only run programs from people you trust. You can pause or remove it any time.",
                cx,
            ))
            .into_any_element()
    }

    fn render_tools(
        &self,
        service: &McpService,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let id = service.id.clone();
        let open = self.services_screen.expanded.contains(&id);
        let toggle_id = id.clone();
        let mut block = v_flex().w_full().min_w_0().gap(t.space.compact).child(
            h_flex().child(
                design::button(
                    sid("mcp-tools-toggle", &id),
                    format!("{} of {} tools on", service.tools_on(), service.tools.len()),
                    cx,
                )
                .ghost()
                .xsmall()
                .icon(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    let expanded = &mut view.services_screen.expanded;
                    if !expanded.remove(&toggle_id) {
                        expanded.insert(toggle_id.clone());
                    }
                    cx.notify();
                })),
            ),
        );
        if !open {
            return block;
        }
        for tool in &service.tools {
            let tool_id = id.clone();
            let name = tool.name.clone();
            block = block.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap(t.space.small)
                    .pl(t.space.small)
                    .child(
                        Switch::new(SharedString::from(format!(
                            "mcp-tool-switch-{id}-{}",
                            tool.name
                        )))
                        .checked(service.tool_is_on(&tool.name))
                        .small()
                        .on_click(cx.listener(
                            move |_, checked: &bool, _, cx| {
                                send(
                                    cx,
                                    McpCommand::SetToolEnabled {
                                        id: tool_id.clone(),
                                        tool: name.clone(),
                                        enabled: *checked,
                                    },
                                );
                            },
                        )),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .sovra_text(TextRole::Caption, cx)
                                    .child(tool.name.clone()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("mcp-tool-{id}-{}", tool.name)))
                                    .test_support()
                                    .aria_label(tool.description.clone())
                                    .min_w_0()
                                    .whitespace_normal()
                                    .sovra_text(TextRole::Caption, cx)
                                    .text_color(t.text_secondary)
                                    .child(tool.description.clone()),
                            ),
                    ),
            );
        }
        block
    }

    fn render_card_actions(
        &self,
        service: &McpService,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let id = service.id.clone();
        let screen = &self.services_screen;
        if screen.confirm_remove.as_deref() == Some(id.as_str()) {
            let confirm_id = id.clone();
            return h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap(t.space.small)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .child(format!(
                            "Remove {}? Its saved settings and sign-in are deleted.",
                            service.name
                        )),
                )
                .child(
                    design::button(sid("mcp-remove-confirm", &id), "Yes, remove", cx)
                        .small()
                        .danger()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            send(
                                cx,
                                McpCommand::Remove {
                                    id: confirm_id.clone(),
                                },
                            );
                            view.services_screen.confirm_remove = None;
                            cx.notify();
                        })),
                )
                .child(
                    design::button(sid("mcp-remove-keep", &id), "Keep it", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.services_screen.confirm_remove = None;
                            cx.notify();
                        })),
                );
        }
        let mut row = h_flex().w_full().flex_wrap().gap(t.space.hair);
        let can_retry = service.enabled
            && matches!(
                service.phase,
                McpPhase::Connected | McpPhase::Failed | McpPhase::Saved
            );
        if can_retry {
            let retry_id = id.clone();
            row = row.child(
                design::button(
                    sid("mcp-reconnect", &id),
                    if service.transport.is_program() {
                        "Restart"
                    } else {
                        "Reconnect"
                    },
                    cx,
                )
                .ghost()
                .xsmall()
                .on_click(cx.listener(move |_, _, _, cx| {
                    send(
                        cx,
                        McpCommand::Reconnect {
                            id: retry_id.clone(),
                        },
                    );
                })),
            );
        }
        let rename_id = id.clone();
        let current = service.name.clone();
        row = row.child(
            design::button(sid("mcp-rename", &id), "Rename", cx)
                .ghost()
                .xsmall()
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.services_screen.renaming = Some(rename_id.clone());
                    view.services_screen.rename.update(cx, |input, cx| {
                        input.set_value(current.clone(), window, cx);
                    });
                    cx.notify();
                })),
        );
        let remove_id = id.clone();
        row.child(
            design::button(sid("mcp-remove", &id), "Remove", cx)
                .ghost()
                .xsmall()
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.services_screen.confirm_remove = Some(remove_id.clone());
                    cx.notify();
                })),
        )
    }

    fn connect_service(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.services.notice = None;
        let address = self.service_address.read(cx).value().to_string();
        if address.trim().is_empty() {
            return;
        }
        let name = self.service_name.read(cx).value().to_string();
        let secret_text = self.service_secret.read(cx).value().to_string();
        let secret = Some(secret_text.trim().to_owned()).filter(|value| !value.is_empty());
        send(
            cx,
            McpCommand::Connect {
                name,
                address,
                secret,
            },
        );
        for input in [
            &self.service_secret,
            &self.service_address,
            &self.service_name,
        ] {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        self.services_screen.form_problem = None;
        self.services_screen.mode = AddMode::Closed;
        cx.notify();
    }

    fn add_program(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.services.notice = None;
        let screen = &self.services_screen;
        let typed = screen.program_command.read(cx).value().to_string();
        let draft = match program_from_command_line(&typed) {
            Ok(draft) => draft,
            Err(problem) => {
                self.services_screen.form_problem = Some(problem.text().to_owned());
                cx.notify();
                return;
            }
        };
        let DraftSetup::Program {
            command,
            args,
            env: mut typed_env,
            ..
        } = draft.setup
        else {
            return;
        };
        let settings = screen.program_settings.read(cx).value().to_string();
        match settings_lines(&settings) {
            Ok(lines) => typed_env.extend(lines),
            Err(problem) => {
                self.services_screen.form_problem = Some(problem);
                cx.notify();
                return;
            }
        }
        let folder = screen.program_folder.read(cx).value().trim().to_owned();
        let name = screen.program_name.read(cx).value().to_string();
        send(
            cx,
            McpCommand::AddProgram {
                name,
                command,
                args,
                env: typed_env,
                working_dir: Some(folder).filter(|dir| !dir.is_empty()),
            },
        );
        let screen = &self.services_screen;
        for input in [
            &screen.program_name,
            &screen.program_command,
            &screen.program_folder,
        ] {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        screen
            .program_settings
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.services_screen.form_problem = None;
        self.services_screen.mode = AddMode::Closed;
        cx.notify();
    }

    fn import_services(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.services.notice = None;
        let text = self.services_screen.paste.read(cx).value().to_string();
        // Check here too, so a typo is explained before anything is sent.
        if let Err(problem) = parse_server_settings(&text) {
            self.services_screen.form_problem = Some(problem.text().to_owned());
            cx.notify();
            return;
        }
        send(cx, McpCommand::Import { text });
        self.services_screen
            .paste
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.services_screen.form_problem = None;
        self.services_screen.mode = AddMode::Closed;
        cx.notify();
    }

    fn continue_sign_in(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let client_id = self.service_client_id.read(cx).value().to_string();
        let secret_text = self.service_client_secret.read(cx).value().to_string();
        let client_secret = Some(secret_text.trim().to_owned()).filter(|value| !value.is_empty());
        send(
            cx,
            McpCommand::SubmitClientDetails {
                id: id.to_owned(),
                client_id,
                client_secret,
            },
        );
        self.service_client_secret.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }
}

/// Plain note text for a program, exposed for tests.
#[cfg(test)]
fn note_texts(transport: &McpTransport) -> Vec<&'static str> {
    program_notes(transport)
        .into_iter()
        .map(sovra_core::mcp::ProgramNote::text)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_lines_read_name_value_pairs() {
        assert_eq!(
            settings_lines("GITHUB_TOKEN=abc\n\n# note\n REGION = eu ").expect("lines"),
            [
                ("GITHUB_TOKEN".to_owned(), "abc".to_owned()),
                ("REGION".to_owned(), "eu".to_owned())
            ]
        );
        assert!(settings_lines("not a setting").is_err());
        assert!(settings_lines("BAD-NAME=1").is_err());
        assert_eq!(settings_lines("  ").expect("empty"), []);
    }

    #[test]
    fn review_notes_are_plain_words() {
        let notes = note_texts(&McpTransport::Program {
            command: "npx".into(),
            args: vec!["-y".into(), "pkg".into()],
            env_names: Vec::new(),
            working_dir: None,
        });
        assert!(notes[0].contains("Downloads"));
        assert!(
            notes
                .last()
                .is_some_and(|note| note.contains("same access"))
        );
    }
}
