//! Beginner Raspberry Pi setup inside the existing MCP service screen.

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    model_connection_editor::field,
    models_view::{ModelsView, caption},
};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    text::TextView,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command,
    mcp::{McpCommand, McpPhase, McpService},
    raspberry_pi::{self, Family, PiResults, PiSetup},
};

pub(crate) struct PiScreen {
    pub(crate) query: Entity<InputState>,
    pub(crate) host: Entity<InputState>,
    pub(crate) user: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
    pub(crate) pairing: raspberry_pi::PairingState,
    family: Family,
    pub(crate) pending_chat: Option<PiSetup>,
    pub(crate) details: std::collections::BTreeSet<String>,
    selected: Option<&'static str>,
    health_open: bool,
    pub(crate) results: PiResults,
    pub(crate) discovery: raspberry_pi::DiscoveryState,
    pub(crate) gpio_maps: std::collections::BTreeSet<String>,
    pub(crate) gpio_pin: Option<(String, &'static str, u8)>,
}

impl PiScreen {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        Self {
            query: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Search the name printed on your Pi")
            }),
            host: cx
                .new(|cx| InputState::new(window, cx).placeholder("For example, gardenpi.local")),
            user: cx.new(|cx| {
                InputState::new(window, cx).placeholder("The account you made on your Pi")
            }),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Your Pi account password")
                    .masked(true)
            }),
            pairing: raspberry_pi::PairingState::default(),
            pending_chat: None,
            details: std::collections::BTreeSet::new(),
            family: Family::Computer,
            selected: None,
            health_open: false,
            results: PiResults::default(),
            discovery: raspberry_pi::DiscoveryState::default(),
            gpio_maps: std::collections::BTreeSet::new(),
            gpio_pin: None,
        }
    }
}

impl ModelsView {
    fn find_raspberry_pis(&mut self, cx: &mut Context<Self>) {
        let event = if cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Mcp(McpCommand::DiscoverRaspberryPis))
            .is_ok()
        {
            raspberry_pi::DiscoveryEvent::Started
        } else {
            raspberry_pi::DiscoveryEvent::Finished(Err(
                "The search couldn't start. Restart Sovra and try again.".into(),
            ))
        };
        self.services_screen.pi.discovery.apply(&event);
        cx.notify();
    }

    fn render_pi_discovery(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let state = &self.services_screen.pi.discovery;
        let message = if state.searching {
            "Looking for nearby computers… This takes about eight seconds."
        } else if let Some(problem) = &state.problem {
            problem.as_str()
        } else if state.searched && state.candidates.is_empty() {
            "No nearby computers answered. Keep your Pi powered on and on the same network. You can still enter its name below."
        } else if !state.candidates.is_empty() {
            "Choose your Pi's computer name. Sovra checks its model when you connect; other computers can appear here too."
        } else {
            "Find your Pi on the same network, or enter its name below."
        };
        let mut panel = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                design::button("pi-find", "Find my Pi", cx)
                    .small()
                    .disabled(state.searching)
                    .on_click(cx.listener(|view, _, _, cx| view.find_raspberry_pis(cx))),
            )
            .child(
                div()
                    .id("pi-discovery-status")
                    .test_support()
                    .aria_label(message.to_owned())
                    .sovra_text(TextRole::Caption, cx)
                    .whitespace_normal()
                    .child(message.to_owned()),
            );
        for candidate in &state.candidates {
            let host = candidate.host().to_owned();
            panel = panel.child(
                design::button(SharedString::from(format!("pi-nearby-{host}")), "", cx)
                    .small()
                    .accessibility_label(host.clone())
                    .w_full()
                    .min_w_0()
                    .h_auto()
                    .min_h(t.layout.compact_control_height)
                    .py(t.space.compact)
                    .selected(self.services_screen.pi.host.read(cx).value().as_ref() == host)
                    .child(div().min_w_0().whitespace_normal().child(host.clone()))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.services_screen
                            .pi
                            .host
                            .update(cx, |input, cx| input.set_value(host.clone(), window, cx));
                        view.services_screen.pi.health_open = true;
                        view.services_screen.form_problem = None;
                        cx.notify();
                    })),
            );
        }
        panel
    }

    pub(crate) fn render_pi_form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let screen = &self.services_screen.pi;
        let mut panel = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Connect your Raspberry Pi"),
            )
            .child(caption(
                "Choose your model, then start chatting. Setup help and projects are included.",
                cx,
            ));
        if let Some(board) = screen.selected.and_then(raspberry_pi::board) {
            panel = panel
                .child(
                    design::button("pi-change-model", "Choose a different model", cx)
                        .small()
                        .ghost()
                        .disabled(screen.pairing.busy())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.services_screen
                                .pi
                                .password
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            view.services_screen.pi.pairing = raspberry_pi::PairingState::default();
                            view.services_screen.pi.selected = None;
                            view.services_screen.form_problem = None;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("pi-selected-model")
                        .test_support()
                        .aria_label(board.name)
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(board.name),
                )
                .child(caption(board.note, cx));
            if screen.pairing.busy() {
                return panel.child(self.render_pi_pairing(cx));
            }
            if matches!(screen.pairing.phase, raspberry_pi::PairingPhase::Failed(_)) {
                panel = panel.child(self.render_pi_pairing(cx));
            }
            if board.family != Family::Pico {
                panel = panel.child(self.render_pi_discovery(cx));
                panel = panel.child(
                    design::button("pi-health-options", "Connection details", cx)
                        .small()
                        .ghost()
                        .selected(screen.health_open)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.services_screen.pi.health_open =
                                !view.services_screen.pi.health_open;
                            cx.notify();
                        })),
                );
                if screen.health_open {
                    panel = panel
                        .child(caption("Enter these once. Sovra remembers your sign-in on this computer. Leave them blank if you need setup help first.", cx))
                        .child(field("pi-host", "Pi's computer name or network address", &screen.host, "Find this in your Pi's network settings. Keep both computers on the same network.", cx))
                        .child(field("pi-user", "Your account on the Pi", &screen.user, "Use the name you chose during setup. There is no shared default account.", cx))
                        .child(field("pi-password", "Your Pi password", &screen.password, "Saved in this computer's credential storage, never in chat. Leave blank to use a saved sign-in or an existing trusted key. Remote access must be enabled on the Pi.", cx))
                        .child(design::button("pi-remote-help", "Open remote access help", cx).small().ghost().on_click(|_, _, cx| cx.open_url("https://www.raspberrypi.com/documentation/computers/remote-access.html")));
                }
            }
            panel = panel.child(caption("Projects you request with Build a project are saved and sent to this Pi. Connecting a Pico pauses its current program.", cx));
            panel = panel.child(
                design::primary_button("pi-add-plugin", "Connect and chat", cx).on_click(
                    cx.listener(move |view, _, window, cx| {
                        let screen = &view.services_screen.pi;
                        let use_host = board.family != Family::Pico;
                        let setup = PiSetup {
                            board_id: board.id.into(),
                            host: if use_host {
                                screen.host.read(cx).value().trim().to_owned()
                            } else {
                                String::new()
                            },
                            user: if use_host {
                                screen.user.read(cx).value().trim().to_owned()
                            } else {
                                String::new()
                            },
                        };
                        if use_host && !screen.password.read(cx).value().is_empty() {
                            view.start_pi_pairing(setup, window, cx);
                            return;
                        }
                        match setup.validate() {
                            Ok(_) => {
                                if let Some(service) = view
                                    .services
                                    .services
                                    .iter()
                                    .find(|service| {
                                        raspberry_pi::service_setup(service).as_ref()
                                            == Some(&setup)
                                    })
                                    .cloned()
                                {
                                    view.services_screen.mode =
                                        crate::service_connect_view::AddMode::Closed;
                                    if service.enabled && service.phase == McpPhase::Connected {
                                        cx.emit(crate::models_view::GoToPiChat {
                                            service_id: service.id,
                                            setup,
                                        });
                                    } else {
                                        view.services_screen.pi.pending_chat = Some(setup);
                                        let command = if service.enabled {
                                            McpCommand::Reconnect { id: service.id }
                                        } else {
                                            McpCommand::SetEnabled {
                                                id: service.id,
                                                enabled: true,
                                            }
                                        };
                                        let _ = cx
                                            .global::<RuntimeClient>()
                                            .0
                                            .send(Command::Mcp(command));
                                    }
                                    cx.notify();
                                    return;
                                }
                                if cx
                                    .global::<RuntimeClient>()
                                    .0
                                    .send(Command::Mcp(McpCommand::AddRaspberryPi(setup.clone())))
                                    .is_ok()
                                {
                                    view.services_screen.pi.pending_chat = Some(setup);
                                    view.services_screen.mode =
                                        crate::service_connect_view::AddMode::Closed;
                                    view.services_screen.form_problem = None;
                                } else {
                                    view.services_screen.form_problem = Some(
                                    "Sovra couldn't add the plugin. Restart the app and try again."
                                        .into(),
                                );
                                }
                            }
                            Err(problem) => view.services_screen.form_problem = Some(problem),
                        }
                        cx.notify();
                    }),
                ),
            );
        } else {
            let query = screen.query.read(cx).value().to_string();
            let found =
                raspberry_pi::search(&query, query.trim().is_empty().then_some(screen.family));
            panel = panel
                .child(
                    Input::new(&screen.query)
                        .id("pi-search")
                        .aria_label("Search Raspberry Pi models")
                        .w_full(),
                )
                .child(h_flex().w_full().flex_wrap().gap(t.space.compact).children(
                    Family::ALL.map(|family| {
                        design::button(
                            SharedString::from(format!("pi-family-{family:?}")),
                            family.label(),
                            cx,
                        )
                        .small()
                        .selected(screen.family == family)
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.services_screen.pi.family = family;
                                view.services_screen
                                    .pi
                                    .query
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            },
                        ))
                    }),
                ));
            if found.is_empty() {
                panel = panel.child(caption(
                    "No model matches. Try a short name, such as Zero, 500 or Pico.",
                    cx,
                ));
            }
            for board in found {
                panel = panel.child(
                    design::button(SharedString::from(format!("pi-model-{}", board.id)), "", cx)
                        .accessibility_label(board.name)
                        .h_auto()
                        .py(t.space.small)
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .whitespace_normal()
                                .child(board.name),
                        )
                        .w_full()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.services_screen.pi.selected = Some(board.id);
                            view.services_screen.pi.health_open = board.family != Family::Pico;
                            view.services_screen.form_problem = None;
                            if board.family != Family::Pico
                                && !view.services_screen.pi.discovery.searching
                                && !view.services_screen.pi.discovery.searched
                            {
                                view.find_raspberry_pis(cx);
                            }
                            cx.notify();
                        })),
                );
            }
        }
        panel
    }

    pub(crate) fn render_pi_actions(
        &self,
        service: &McpService,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let Some(setup) = raspberry_pi::service_setup(service) else {
            return div().into_any_element();
        };
        let Some(board) = raspberry_pi::board(&setup.board_id) else {
            return div().into_any_element();
        };
        let pending = self
            .services_screen
            .pi
            .results
            .pending
            .contains(&service.id);
        let mut panel = v_flex().w_full().min_w_0().gap(t.space.small)
            .child(caption("Plugin tools and device connection are separate. Use a check below to see what your computer can reach.", cx));
        let check = if board.family == Family::Pico {
            ("check_usb_connection", "Check USB connection")
        } else {
            ("check_health", "Check Pi health")
        };
        panel = panel.child(
            h_flex().w_full().flex_wrap().gap(t.space.small).children(
                [
                    ("setup_guide", "Setup steps"),
                    ("starter_project", "First project"),
                    ("gpio_guide", "GPIO pins"),
                    ("led_project", "Blink a light"),
                    ("button_project", "Read a button"),
                    ("troubleshoot", "Fix a problem"),
                    check,
                ]
                .map(|(tool, label)| {
                    let id = service.id.clone();
                    design::button(SharedString::from(format!("pi-{tool}-{id}")), label, cx)
                        .small()
                        .disabled(
                            pending
                                || !service.enabled
                                || service.phase != McpPhase::Connected
                                || service.disabled_tools.iter().any(|name| name == tool)
                                || !service.tools.iter().any(|info| info.name == tool),
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            let result = cx.global::<RuntimeClient>().0.send(Command::Mcp(
                                McpCommand::CallTool {
                                    service_id: id.clone(),
                                    tool: tool.into(),
                                    arguments: serde_json::json!({}),
                                    session_id: None,
                                },
                            ));
                            if result.is_ok() {
                                view.services_screen.pi.results.begin(&id);
                                if tool == "gpio_guide" {
                                    view.services_screen.pi.gpio_maps.insert(id.clone());
                                } else {
                                    view.services_screen.pi.gpio_maps.remove(&id);
                                }
                            }
                            cx.notify();
                        }))
                }),
            ),
        );
        if pending {
            panel = panel.child(caption("Checking…", cx));
        }
        if let Some(text) = self.services_screen.pi.results.text.get(&service.id) {
            let project_title = text
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("# "))
                .map_or_else(
                    || format!("{} first project", board.name),
                    |title| title.chars().take(160).collect(),
                );
            let draft = sovra_core::artifacts::ArtifactDraft {
                title: format!("{} notes", board.name),
                kind: sovra_core::artifacts::ArtifactKind::Document,
                language: String::new(),
                content: text.clone(),
                source: sovra_core::artifacts::ArtifactSource {
                    author: board.name.into(),
                    ..Default::default()
                },
            };
            let copied = text.clone();
            panel = panel.child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button(
                            SharedString::from(format!("pi-save-{}", service.id)),
                            "Save to library",
                            cx,
                        )
                        .small()
                        .on_click(move |_, _, cx| {
                            let _ = cx.global::<RuntimeClient>().0.send(Command::Artifacts(
                                sovra_core::artifacts::ArtifactCommand::Save(draft.clone()),
                            ));
                        }),
                    )
                    .child(
                        design::button(
                            SharedString::from(format!("pi-copy-{}", service.id)),
                            "Copy",
                            cx,
                        )
                        .small()
                        .ghost()
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                copied.clone(),
                            ));
                        }),
                    ),
            );
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("pi-result-{}", service.id)))
                    .test_support()
                    .aria_label(text.clone())
                    .w_full()
                    .min_w_0()
                    .p(t.space.small)
                    .sovra_surface(Surface::Matte, cx)
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(
                        if self.services_screen.pi.gpio_maps.contains(&service.id)
                            && text == &board.gpio_guide()
                        {
                            self.render_pi_pin_maps(board, &service.id, cx)
                        } else {
                            TextView::markdown(
                                SharedString::from(format!("pi-rich-result-{}", service.id)),
                                text.clone(),
                            )
                            .selectable(true)
                            .scrollable(false)
                            .style(design::markdown_style(cx))
                            .code_block_actions(move |code, _, cx| {
                                let copied_code = code.code().to_string();
                                let language =
                                    code.lang().map(|lang| lang.to_string()).unwrap_or_default();
                                let save = sovra_core::artifacts::draft_from_piece(
                                    &language,
                                    &copied_code,
                                    sovra_core::artifacts::ArtifactSource {
                                        author: board.name.into(),
                                        ..Default::default()
                                    },
                                    &project_title,
                                );
                                h_flex()
                                    .w_full()
                                    .flex_wrap()
                                    .gap(design::tokens(cx).space.small)
                                    .child(
                                        design::button("pi-copy-code", "Copy code", cx)
                                            .small()
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(
                                                    gpui_kit::ClipboardItem::new_string(
                                                        copied_code.clone(),
                                                    ),
                                                );
                                            }),
                                    )
                                    .child(
                                        design::button("pi-save-code", "Save code", cx)
                                            .small()
                                            .on_click(move |_, _, cx| {
                                                let _ = cx
                                                    .global::<RuntimeClient>()
                                                    .0
                                                    .send(Command::Artifacts(
                                                    sovra_core::artifacts::ArtifactCommand::Save(
                                                        save.clone(),
                                                    ),
                                                ));
                                            }),
                                    )
                            })
                            .into_any_element()
                        },
                    ),
            );
        }
        panel.into_any_element()
    }
}
