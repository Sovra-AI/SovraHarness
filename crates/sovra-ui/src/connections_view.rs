//! Personal channel connections (`Connections` section inside Settings, plus the
//! interactive `WhatsApp` Personal QR code pairing modal): link your personal `WhatsApp`
//! directly inside the single Sovra Rust desktop binary using the Multi-Device companion
//! protocol (`Noise_XX_25519_AESGCM_SHA256`, `Curve25519` keys, `ADV` HMAC-SHA256, and
//! a native GPUI QR code matrix / 8-character Crockford Base32 phone pairing code).

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
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::Command;
use sovra_core::connections::{
    ConnectionCommand, ConnectionsState, WhatsAppQrTicket, WhatsAppStatus,
};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption, pill};
use crate::settings_view::settings_section;

pub(crate) struct WhatsAppForm {
    pub(crate) phone_input: Entity<InputState>,
    pub(crate) test_msg_input: Entity<InputState>,
    pub(crate) arch_open: bool,
}

impl WhatsAppForm {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        Self {
            phone_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder("e.g. +1 555 234 5678 (optional)")
            }),
            test_msg_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("e.g. Summarize my schedule for this afternoon")
            }),
            arch_open: false,
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
        let is_connected = state.whatsapp.is_connected();

        let mut section = settings_section(
            "settings-connections",
            "Connections · WhatsApp Personal",
            "Link your personal WhatsApp directly inside this app — click Connect, scan the QR code with your phone, and chat with Sovra from WhatsApp.",
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
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .child("WhatsApp Personal (Multi-Device Companion)"),
                        )
                        .child(
                            div()
                                .id("whatsapp-status-badge")
                                .test_support()
                                .child(pill(state.whatsapp.badge_label().to_owned(), cx)),
                        ),
                )
                .child(if is_connected {
                    design::button("whatsapp-disconnect-top", "Disconnect WhatsApp", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send_connection(ConnectionCommand::DisconnectWhatsApp, cx);
                        }))
                } else {
                    design::primary_button("whatsapp-connect-btn", "Connect WhatsApp", cx)
                        .small()
                        .icon(IconName::MessageCircle)
                        .tooltip("Open a QR code to scan with your phone's WhatsApp app")
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

        if let WhatsAppStatus::Connected {
            phone_display,
            device_jid,
            companion_name,
            auto_reply_enabled,
            only_self_chat,
            ..
        } = &state.whatsapp
        {
            let current_auto = *auto_reply_enabled;
            let current_self = *only_self_chat;

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
                                .child(format!("✓ Linked to {phone_display}")),
                        )
                        .child(caption(
                            format!(
                                "Companion: {companion_name} · JID: {device_jid} · Signal E2E active"
                            ),
                            cx,
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .gap(t.space.medium)
                                .child(
                                    Switch::new("whatsapp-toggle-self-only")
                                        .checked(current_self)
                                        .label("Only reply in “Message Yourself” (safe default)")
                                        .on_click(cx.listener(
                                            move |view, checked: &bool, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::UpdateWhatsAppSettings {
                                                        auto_reply_enabled: current_auto,
                                                        only_self_chat: *checked,
                                                    },
                                                    cx,
                                                );
                                            },
                                        )),
                                )
                                .child(
                                    Switch::new("whatsapp-toggle-auto-reply")
                                        .checked(current_auto)
                                        .label("Automatically answer my WhatsApp messages")
                                        .on_click(cx.listener(
                                            move |view, checked: &bool, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::UpdateWhatsAppSettings {
                                                        auto_reply_enabled: *checked,
                                                        only_self_chat: current_self,
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
                                    .aria_label("Test WhatsApp message")
                                    .small(),
                            ),
                        )
                        .child(
                            design::primary_button(
                                "whatsapp-send-test-btn",
                                "Send via WhatsApp bridge",
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
                                let trimmed = if text.trim().is_empty() {
                                    "What's on my schedule today?".to_owned()
                                } else {
                                    text
                                };
                                view.send_connection(
                                    ConnectionCommand::SendTestWhatsAppMessage { text: trimmed },
                                    cx,
                                );
                                view.whatsapp_form.test_msg_input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                            })),
                        ),
                );

            if !state.recent_messages.is_empty() {
                section = section.child(
                    v_flex()
                        .id("whatsapp-bridged-messages")
                        .test_support()
                        .w_full()
                        .gap(t.space.small)
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
                                .children(msg.reply_text.clone().map(|reply| caption(reply, cx)))
                        })),
                );
            }
        }

        section.child(self.render_whatsapp_architecture(state, cx))
    }

    pub(crate) fn render_whatsapp_qr_modal(
        &self,
        state: &ConnectionsState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let ticket = match &state.whatsapp {
            WhatsAppStatus::AwaitingQrScan { ticket } => ticket.clone(),
            _ => WhatsAppQrTicket::new(&state.keys, 1_793_080_000_000, 1),
        };
        let qr_payload_copy = ticket.qr_payload.clone();

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
                                    .child("Scan with your phone to link WhatsApp"),
                            )
                            .child(caption(
                                "Open WhatsApp on your phone → Settings (or ⋮) → Linked Devices → Link a Device.",
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
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .items_start()
                    .gap(t.space.medium)
                    .child(render_qr_matrix_card(&ticket, cx))
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
                                            .child("Or link with your phone number"),
                                    )
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .flex_wrap()
                                            .gap(t.space.small)
                                            .child(
                                                div().flex_1().min_w(t.layout.filter_width).child(
                                                    Input::new(&self.whatsapp_form.phone_input)
                                                        .id("whatsapp-phone-input")
                                                        .aria_label("Phone number for pairing code")
                                                        .small(),
                                                ),
                                            )
                                            .child(
                                                design::button(
                                                    "whatsapp-get-pair-code",
                                                    "Get 8-digit code",
                                                    cx,
                                                )
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
                                            .child(format!("Your phone linking code: {code}"))
                                    })),
                            )
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .gap(t.space.small)
                                    .child(
                                        design::primary_button(
                                            "whatsapp-confirm-scan-btn",
                                            "I've scanned the QR code",
                                            cx,
                                        )
                                        .small()
                                        .icon(IconName::Check)
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            let phone_display = view
                                                .whatsapp_form
                                                .phone_input
                                                .read(cx)
                                                .value()
                                                .to_string();
                                            view.send_connection(
                                                ConnectionCommand::ConfirmWhatsAppScan {
                                                    phone_display,
                                                },
                                                cx,
                                            );
                                        })),
                                    )
                                    .child(
                                        design::button("whatsapp-refresh-qr-btn", "New QR code", cx)
                                            .small()
                                            .ghost()
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::RefreshWhatsAppQr,
                                                    cx,
                                                );
                                            })),
                                    )
                                    .child(
                                        design::button(
                                            "whatsapp-copy-qr-payload",
                                            "Copy QR payload",
                                            cx,
                                        )
                                        .small()
                                        .ghost()
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                qr_payload_copy.clone(),
                                            ));
                                        }),
                                    ),
                            ),
                    ),
            )
    }

    fn render_whatsapp_architecture(
        &self,
        state: &ConnectionsState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let open = self.whatsapp_form.arch_open;
        let mut col = v_flex().w_full().gap(t.space.hair).child(
            h_flex().child(
                design::button(
                    "whatsapp-arch-toggle",
                    if open {
                        "Hide how WhatsApp companion works"
                    } else {
                        "How in-binary WhatsApp Personal works"
                    },
                    cx,
                )
                .xsmall()
                .ghost()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.whatsapp_form.arch_open = !view.whatsapp_form.arch_open;
                    cx.notify();
                })),
            ),
        );
        if open {
            col = col
                .child(caption(
                    "Embedded Multi-Device Companion (Noise_XX_25519_AESGCM_SHA256 + Curve25519 Signal Identity + ADV HMAC-SHA256). 5-field QR payload (ref,noise_pub,identity_pub,adv_secret,7) runs inside the native Rust binary with no external sidecar.",
                    cx,
                ))
                .child(caption(
                    format!(
                        "Companion Noise pubkey: {} · Identity pubkey: {}",
                        state.keys.noise_pub_b64, state.keys.identity_pub_b64
                    ),
                    cx,
                ));
        }
        col
    }
}

fn render_qr_matrix_card(
    ticket: &WhatsAppQrTicket,
    cx: &Context<ModelsView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let dark_module = cx.theme().foreground;
    let light_module = t.canvas;

    let mut grid = v_flex()
        .id("whatsapp-qr-matrix")
        .test_support()
        .p(t.space.small)
        .gap(t.space.hair)
        .rounded(t.radius.card)
        .bg(light_module)
        .border_1()
        .border_color(cx.theme().border);

    for row in &ticket.qr_matrix {
        let mut line = h_flex().gap(t.space.hair);
        for &is_dark in row {
            line = line.child(div().size(t.space.compact).bg(if is_dark {
                dark_module
            } else {
                light_module
            }));
        }
        grid = grid.child(line);
    }

    v_flex()
        .items_center()
        .gap(t.space.hair)
        .child(grid)
        .child(caption(
            format!(
                "Ref #{}/6 · {}s · Noise_XX_25519",
                ticket.rotation_index, ticket.expires_in_secs
            ),
            cx,
        ))
}
