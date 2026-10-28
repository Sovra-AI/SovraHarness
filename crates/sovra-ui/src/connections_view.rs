//! Personal connections inside Settings: link your own `WhatsApp` to this app. Click
//! Connect, scan the code `WhatsApp` issues with your phone, and messages you send
//! yourself on `WhatsApp` are answered by Sovra. The real client runs in the runtime;
//! this view only shows its state and sends plain commands.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputState},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Styled as _, TestSupportExt as _, Window, canvas, div, fill, point,
    size,
};
use sovra_core::Command;
use sovra_core::connections::{
    ConnectionCommand, ConnectionsState, WhatsAppQrTicket, WhatsAppSettings, WhatsAppStatus,
};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption, pill};
use crate::settings_view::settings_section;

pub(crate) struct WhatsAppForm {
    pub(crate) phone_input: Entity<InputState>,
    pub(crate) test_msg_input: Entity<InputState>,
}

impl WhatsAppForm {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        Self {
            phone_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Phone number with country code")
            }),
            test_msg_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Hello from my computer")),
        }
    }
}

impl ModelsView {
    pub(crate) fn send_connection(&mut self, command: ConnectionCommand, cx: &mut Context<Self>) {
        self.connections.notice = None;
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Connections(command));
        cx.notify();
    }

    pub(crate) fn render_connections_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let state = &self.connections;
        let linked = state.whatsapp.is_linked();

        let mut section = settings_section(
            "settings-connections",
            "WhatsApp",
            "Link your own WhatsApp to this app. Click Connect, scan the code with your phone, then message yourself on WhatsApp to talk to Sovra from anywhere.",
            cx,
        )
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(t.space.small)
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap(t.space.compact)
                        .child(
                            Icon::new(IconName::MessageCircle)
                                .small()
                                .text_color(cx.theme().link),
                        )
                        .child(div().sovra_text(TextRole::Body, cx).child("WhatsApp"))
                        .child(
                            div()
                                .id("whatsapp-status-badge")
                                .test_support()
                                .child(pill(state.whatsapp.badge_label().to_owned(), cx)),
                        ),
                )
                .child(if linked {
                    design::button("whatsapp-disconnect-top", "Unlink WhatsApp", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send_connection(ConnectionCommand::DisconnectWhatsApp, cx);
                        }))
                } else {
                    design::primary_button("whatsapp-connect-btn", "Connect WhatsApp", cx)
                        .small()
                        .icon(IconName::MessageCircle)
                        .tooltip("Show a code to scan with WhatsApp on your phone")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send_connection(ConnectionCommand::StartWhatsAppPairing, cx);
                        }))
                }),
        );

        if let Some(notice) = &state.notice {
            section = section.child(
                div()
                    .id("connections-notice")
                    .test_support()
                    .w_full()
                    .p(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .sovra_text(TextRole::Caption, cx)
                    .child(notice.clone()),
            );
        }

        if let WhatsAppStatus::Connected { device, online } = &state.whatsapp {
            let settings = state.settings;
            section = section
                .child(
                    v_flex()
                        .id("whatsapp-connected-details")
                        .test_support()
                        .w_full()
                        .p(t.space.medium)
                        .gap(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(format!("Linked to {}", device.phone_display)),
                        )
                        .child(caption(
                            if *online {
                                format!(
                                    "Shows as “{}” under Linked devices on your phone. Messages are end-to-end encrypted.",
                                    device.companion_name
                                )
                            } else {
                                "Reconnecting to WhatsApp…".to_owned()
                            },
                            cx,
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .gap(t.space.medium)
                                .child(
                                    Switch::new("whatsapp-toggle-self-only")
                                        .checked(settings.only_self_chat)
                                        .label("Only answer in my own chat (“Message yourself”)")
                                        .on_click(cx.listener(
                                            move |view, checked: &bool, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::UpdateWhatsAppSettings {
                                                        settings: WhatsAppSettings {
                                                            only_self_chat: *checked,
                                                            ..settings
                                                        },
                                                    },
                                                    cx,
                                                );
                                            },
                                        )),
                                )
                                .child(
                                    Switch::new("whatsapp-toggle-auto-reply")
                                        .checked(settings.auto_reply_enabled)
                                        .label("Answer WhatsApp messages automatically")
                                        .on_click(cx.listener(
                                            move |view, checked: &bool, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::UpdateWhatsAppSettings {
                                                        settings: WhatsAppSettings {
                                                            auto_reply_enabled: *checked,
                                                            ..settings
                                                        },
                                                    },
                                                    cx,
                                                );
                                            },
                                        )),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            div().flex_1().min_w(t.layout.filter_width).child(
                                Input::new(&self.whatsapp_form.test_msg_input)
                                    .id("whatsapp-test-msg-input")
                                    .aria_label("Message to send to your own WhatsApp")
                                    .small(),
                            ),
                        )
                        .child(
                            design::primary_button(
                                "whatsapp-send-test-btn",
                                "Send to my WhatsApp",
                                cx,
                            )
                            .small()
                            .on_click(cx.listener(|view, _, window, cx| {
                                let text = view
                                    .whatsapp_form
                                    .test_msg_input
                                    .read(cx)
                                    .value()
                                    .to_string();
                                let text = if text.trim().is_empty() {
                                    "Hello from Sovra on my computer.".to_owned()
                                } else {
                                    text
                                };
                                view.send_connection(
                                    ConnectionCommand::SendTestWhatsAppMessage { text },
                                    cx,
                                );
                                view.whatsapp_form.test_msg_input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                            })),
                        ),
                );
        }

        if !state.recent_messages.is_empty() {
            section = section.child(
                v_flex()
                    .id("whatsapp-bridged-messages")
                    .test_support()
                    .w_full()
                    .gap(t.space.small)
                    .child(caption("Recent messages from WhatsApp", cx))
                    .children(state.recent_messages.iter().enumerate().map(|(idx, msg)| {
                        v_flex()
                            .id(("whatsapp-msg-row", idx))
                            .test_support()
                            .w_full()
                            .p(t.space.small)
                            .gap(t.space.hair)
                            .sovra_surface(Surface::Subtle, cx)
                            .child(
                                div()
                                    .sovra_text(TextRole::Caption, cx)
                                    .text_color(cx.theme().link)
                                    .child(msg.sender_name.clone()),
                            )
                            .child(div().sovra_text(TextRole::Body, cx).child(msg.text.clone()))
                            .children(
                                msg.reply_text
                                    .clone()
                                    .map(|reply| caption(format!("Sovra: {reply}"), cx)),
                            )
                    })),
            );
        }

        section
    }

    pub(crate) fn render_whatsapp_qr_modal(
        &self,
        state: &ConnectionsState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);

        let body: gpui_kit::AnyElement = match &state.whatsapp {
            WhatsAppStatus::AwaitingQrScan { ticket } => {
                self.render_scan_step(ticket, cx).into_any_element()
            }
            WhatsAppStatus::VerifyingHandshake { device_label } => caption(
                format!("{device_label} accepted the link. Finishing up…"),
                cx,
            )
            .id("whatsapp-finishing")
            .test_support()
            .into_any_element(),
            WhatsAppStatus::Error { message } => v_flex()
                .id("whatsapp-pairing-error")
                .test_support()
                .gap(t.space.small)
                .child(caption(message.clone(), cx).text_color(cx.theme().danger))
                .child(
                    design::primary_button("whatsapp-retry-btn", "Try again", cx)
                        .small()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send_connection(ConnectionCommand::StartWhatsAppPairing, cx);
                        })),
                )
                .into_any_element(),
            WhatsAppStatus::Disconnected
            | WhatsAppStatus::Connecting { .. }
            | WhatsAppStatus::Connected { .. } => caption("Contacting WhatsApp…", cx)
                .id("whatsapp-contacting")
                .test_support()
                .into_any_element(),
        };

        v_flex()
            .id("whatsapp-qr-modal")
            .test_support()
            .w_full()
            .p(t.space.medium)
            .gap(t.space.medium)
            .sovra_surface(Surface::Elevated, cx)
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(t.space.small)
                    .child(
                        v_flex()
                            .gap(t.space.hair)
                            .child(
                                div()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child("Link WhatsApp to this computer"),
                            )
                            .child(caption(
                                "On your phone, open WhatsApp → Settings (or ⋮) → Linked devices → Link a device, then point the camera at the code.",
                                cx,
                            )),
                    )
                    .child(
                        design::button("whatsapp-qr-close", "Cancel", cx)
                            .small()
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send_connection(ConnectionCommand::CancelPairing, cx);
                            })),
                    ),
            )
            .child(body)
    }

    fn render_scan_step(
        &self,
        ticket: &WhatsAppQrTicket,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let code_card: gpui_kit::AnyElement = if ticket.has_qr() {
            render_qr_card(ticket, cx).into_any_element()
        } else {
            caption("Waiting for WhatsApp to issue a code…", cx)
                .id("whatsapp-qr-pending")
                .test_support()
                .into_any_element()
        };
        h_flex()
            .w_full()
            .flex_wrap()
            .items_start()
            .gap(t.space.medium)
            .child(code_card)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(t.layout.card_min_width)
                    .gap(t.space.small)
                    .child(
                        v_flex()
                            .gap(t.space.small)
                            .p(t.space.small)
                            .sovra_surface(Surface::Subtle, cx)
                            .child(
                                div()
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .child("Can't scan? Link with your phone number"),
                            )
                            .child(caption(
                                "Enter your WhatsApp number. Then on the phone choose “Link with phone number instead” and type the code shown here.",
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .w_full()
                                    .flex_wrap()
                                    .gap(t.space.small)
                                    .child(
                                        div().flex_1().min_w(t.layout.filter_width).child(
                                            Input::new(&self.whatsapp_form.phone_input)
                                                .id("whatsapp-phone-input")
                                                .aria_label("Phone number for the linking code")
                                                .small(),
                                        ),
                                    )
                                    .child(
                                        design::button("whatsapp-get-pair-code", "Get a code", cx)
                                            .small()
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                let phone_number = view
                                                    .whatsapp_form
                                                    .phone_input
                                                    .read(cx)
                                                    .value()
                                                    .to_string();
                                                view.send_connection(
                                                    ConnectionCommand::RequestPairingCode {
                                                        phone_number,
                                                    },
                                                    cx,
                                                );
                                            })),
                                    ),
                            )
                            .children(ticket.pairing_code.clone().map(|code| {
                                div()
                                    .id("whatsapp-pairing-code-display")
                                    .test_support()
                                    .px(t.space.small)
                                    .py(t.space.hair)
                                    .rounded(t.radius.input)
                                    .bg(t.nav_active_background)
                                    .sovra_text(TextRole::SectionTitle, cx)
                                    .text_color(cx.theme().link)
                                    .child(format!("Type this code on your phone: {code}"))
                            })),
                    )
                    .child(
                        design::button("whatsapp-refresh-qr-btn", "New code", cx)
                            .small()
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send_connection(ConnectionCommand::RefreshWhatsAppQr, cx);
                            })),
                    ),
            )
    }
}

/// QR codes have at most 177 modules a side, so the count always fits exactly.
fn cells_f32(count: usize) -> f32 {
    u16::try_from(count).map_or(f32::MAX, f32::from)
}

/// Paints the QR module grid on a canvas: one quad per run of dark modules, black on
/// white in both themes so phone cameras read it.
fn render_qr_card(ticket: &WhatsAppQrTicket, cx: &Context<ModelsView>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let matrix = ticket.qr_matrix.clone();
    let dark = t.scan_code_dark;
    let light = t.scan_code_light;
    let side = t.layout.scan_code;

    let grid = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, _| {
            window.paint_quad(fill(bounds, light));
            let modules = matrix.len();
            if modules == 0 {
                return;
            }
            // Leave one module of quiet zone on every side, as the QR standard asks.
            let module = bounds.size.width / (cells_f32(modules) + 2.);
            let origin = bounds.origin + point(module, module);
            for (row, cells) in matrix.iter().enumerate() {
                let mut col = 0;
                while col < cells.len() {
                    if !cells[col] {
                        col += 1;
                        continue;
                    }
                    let start = col;
                    while col < cells.len() && cells[col] {
                        col += 1;
                    }
                    let run = cells_f32(col - start);
                    window.paint_quad(fill(
                        Bounds::new(
                            origin + point(module * cells_f32(start), module * cells_f32(row)),
                            size(module * run, module),
                        ),
                        dark,
                    ));
                }
            }
        },
    )
    .size(side)
    .flex_shrink_0();

    v_flex()
        .items_center()
        .gap(t.space.hair)
        .child(
            div()
                .id("whatsapp-qr-matrix")
                .test_support()
                .rounded(t.radius.card)
                .border_1()
                .border_color(cx.theme().border)
                .child(grid),
        )
        .child(caption(
            format!(
                "The code changes every {} seconds while this window is open.",
                ticket.expires_in_secs
            ),
            cx,
        ))
}
