//! The Connections tab. The first screen is one catalogue of the links this app
//! can use. Opening a card shows that link's existing setup. The runtime owns
//! the real clients; this view only shows state and sends plain commands.

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
    AnyElement, AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, canvas, div, fill, point, size,
};
use sovra_core::Command;
use sovra_core::connection_catalogue::{
    CatalogueKind, CatalogueSources, ConnectionCatalogue, LinkPhase, catalogue_from,
};
use sovra_core::connections::{
    ConnectionCommand, ConnectionsState, TelegramSettings, TelegramStatus, WhatsAppQrTicket,
    WhatsAppSettings, WhatsAppStatus,
};
use sovra_core::models::ProviderKind;

use crate::RuntimeClient;
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, ShowLocalAi, caption, pill};
use crate::settings_view::settings_section;

pub(crate) struct WhatsAppForm {
    pub(crate) phone: Entity<InputState>,
    pub(crate) telegram_token: Entity<InputState>,
}

impl WhatsAppForm {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ModelsView>) -> Self {
        Self {
            phone: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Phone number with country code")
            }),
            telegram_token: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Paste the token from @BotFather")
                    .masked(true)
            }),
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
            "Link your own WhatsApp to this app. Click Connect, scan the code with your phone, then message yourself on WhatsApp to talk to Sovra. You can also ask in Chat to prepare a WhatsApp message and review it before sending.",
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
                );
        }

        if !state.recent_messages.is_empty() {
            section = section.child(
                v_flex()
                    .id("whatsapp-bridged-messages")
                    .test_support()
                    .w_full()
                    .gap(t.space.small)
                    .child(caption("Recent messages from your linked apps", cx))
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
                                    .child(format!(
                                        "{} · {}",
                                        msg.sender_name,
                                        msg.channel.label()
                                    )),
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

    /// Telegram: a bot of your own, connected with the token `@BotFather` gives you. You
    /// prove it is yours by sending it a code, then it answers you.
    pub(crate) fn render_telegram_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let state = &self.connections;
        let status = &state.telegram;
        let linked = status.is_linked();

        let mut section = settings_section(
            "settings-telegram",
            "Telegram",
            "Talk to Sovra from Telegram through a bot of your own. In Telegram, open @BotFather, send /newbot, and paste the token it gives you here.",
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
                        .child(Icon::new(IconName::Send).small().text_color(cx.theme().link))
                        .child(div().sovra_text(TextRole::Body, cx).child("Telegram"))
                        .child(
                            div()
                                .id("telegram-status-badge")
                                .test_support()
                                .child(pill(status.badge_label().to_owned(), cx)),
                        ),
                )
                .children(linked.then(|| {
                    design::button("telegram-disconnect-btn", "Disconnect Telegram", cx)
                        .small()
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send_connection(ConnectionCommand::DisconnectTelegram, cx);
                        }))
                })),
        );

        match status {
            TelegramStatus::Disconnected | TelegramStatus::Error { .. } => {
                if let TelegramStatus::Error { message } = status {
                    section = section.child(
                        caption(message.clone(), cx)
                            .id("telegram-error")
                            .test_support()
                            .text_color(cx.theme().danger),
                    );
                }
                section = section.child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            div().prefer_width(t.layout.filter_width).child(
                                Input::new(&self.whatsapp_form.telegram_token)
                                    .id("telegram-token-input")
                                    .aria_label("Telegram bot token")
                                    .small(),
                            ),
                        )
                        .child(
                            design::primary_button("telegram-connect-btn", "Connect Telegram", cx)
                                .small()
                                .icon(IconName::Send)
                                .on_click(cx.listener(|view, _, window, cx| {
                                    let token = view
                                        .whatsapp_form
                                        .telegram_token
                                        .read(cx)
                                        .value()
                                        .to_string();
                                    view.send_connection(
                                        ConnectionCommand::ConnectTelegram { token },
                                        cx,
                                    );
                                    view.whatsapp_form
                                        .telegram_token
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                })),
                        ),
                );
            }
            TelegramStatus::Connecting => {
                section = section.child(
                    caption("Checking the token with Telegram…", cx)
                        .id("telegram-connecting")
                        .test_support(),
                );
            }
            TelegramStatus::AwaitingOwner { bot_username, code } => {
                section = section.child(
                    v_flex()
                        .id("telegram-link-code")
                        .test_support()
                        .w_full()
                        .p(t.space.medium)
                        .gap(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(format!("Your bot @{bot_username} is live")),
                        )
                        .child(caption(
                            format!(
                                "In Telegram, open @{bot_username}, press Start, and send it this code so it knows you are its owner:"
                            ),
                            cx,
                        ))
                        .child(
                            div()
                                .px(t.space.small)
                                .py(t.space.hair)
                                .rounded(t.radius.input)
                                .bg(t.nav_active_background)
                                .sovra_text(TextRole::SectionTitle, cx)
                                .text_color(cx.theme().link)
                                .child(code.clone()),
                        ),
                );
            }
            TelegramStatus::Connected {
                bot_username,
                owner_name,
                online,
                ..
            } => {
                let settings = state.telegram_settings;
                section = section.child(
                    v_flex()
                        .id("telegram-connected-details")
                        .test_support()
                        .w_full()
                        .p(t.space.medium)
                        .gap(t.space.small)
                        .sovra_surface(Surface::Subtle, cx)
                        .child(
                            div()
                                .sovra_text(TextRole::SectionTitle, cx)
                                .child(format!("@{bot_username} answers {owner_name}")),
                        )
                        .child(caption(
                            if *online {
                                "Message your bot in Telegram and Sovra replies there.".to_owned()
                            } else {
                                "Reconnecting to Telegram…".to_owned()
                            },
                            cx,
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .gap(t.space.medium)
                                .child(
                                    Switch::new("telegram-toggle-owner-only")
                                        .checked(settings.only_owner)
                                        .label("Only answer me, not other people who find the bot")
                                        .on_click(cx.listener(
                                            move |view, checked: &bool, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::UpdateTelegramSettings {
                                                        settings: TelegramSettings {
                                                            only_owner: *checked,
                                                            ..settings
                                                        },
                                                    },
                                                    cx,
                                                );
                                            },
                                        )),
                                )
                                .child(
                                    Switch::new("telegram-toggle-auto-reply")
                                        .checked(settings.auto_reply_enabled)
                                        .label("Answer Telegram messages automatically")
                                        .on_click(cx.listener(
                                            move |view, checked: &bool, _, cx| {
                                                view.send_connection(
                                                    ConnectionCommand::UpdateTelegramSettings {
                                                        settings: TelegramSettings {
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
                );
            }
        }

        section
    }

    pub(crate) fn render_whatsapp_qr_modal(
        &self,
        state: &ConnectionsState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);

        let body: AnyElement = match &state.whatsapp {
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
                                "On your phone, open WhatsApp → Settings (or ⋮) → Linked devices → Link a device, then point the camera at the code. It will show as Sovra connection.",
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
        let code_card: AnyElement = if ticket.has_qr() {
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
                    .prefer_width(t.layout.card_min_width)
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
                                        div().prefer_width(t.layout.filter_width).child(
                                            Input::new(&self.whatsapp_form.phone)
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
                                                    .phone
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
    v_flex()
        .items_center()
        .gap(t.space.hair)
        .child(scan_code(
            "whatsapp-qr-matrix",
            ticket.qr_matrix.clone(),
            cx,
        ))
        .child(caption(
            format!(
                "The code changes every {} seconds while this window is open.",
                ticket.expires_in_secs
            ),
            cx,
        ))
}

/// A scannable code drawn from its module grid, framed; shared by `WhatsApp` linking and
/// group invites.
pub(crate) fn scan_code(
    id: &'static str,
    matrix: Vec<Vec<bool>>,
    cx: &Context<ModelsView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
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
            let (module, inset) = design::scan_code::geometry(
                bounds.size.width,
                u16::try_from(modules).expect("QR dimensions fit u16"),
            );
            let origin = bounds.origin + point(inset, inset);
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
    .size_full()
    .flex_shrink_0();

    div()
        .id(id)
        .test_support()
        .aria_label("Pairing QR code")
        .w_full()
        .max_w(side)
        .flex_shrink_0()
        .rounded(t.radius.card)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .id(format!("{id}-grid"))
                .test_support()
                .w_full()
                .aspect_ratio(1.)
                .child(grid),
        )
}

impl ModelsView {
    pub(crate) fn open_catalogue_kind(
        &mut self,
        kind: CatalogueKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.manual_form = None;
        match kind {
            CatalogueKind::OnlineAi => self.open_openrouter_key_editor(window, cx),
            CatalogueKind::CompatibleService => self.open_openai_compatible_editor(window, cx),
            CatalogueKind::LocalAi
            | CatalogueKind::WhatsApp
            | CatalogueKind::Telegram
            | CatalogueKind::Google
            | CatalogueKind::GitHub
            | CatalogueKind::WebAddress => {
                self.connections_page = true;
                self.settings_open = false;
                self.home_open = false;
                self.open_kind = Some(kind);
                self.connection_form = None;
                self.form_subscriptions.clear();
                cx.notify();
            }
        }
    }

    pub(crate) fn render_connections_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.connections.qr_modal_open {
            let modal = self
                .render_whatsapp_qr_modal(&self.connections, cx)
                .into_any_element();
            return self.connections_frame(cx, modal);
        }
        if let Some(kind) = self.open_kind {
            let editor = self.render_open_connection(kind, cx);
            return self.connections_frame(cx, editor);
        }
        let catalogue = self.render_catalogue(cx);
        self.connections_frame(cx, catalogue)
    }

    fn current_catalogue(&self) -> ConnectionCatalogue {
        catalogue_from(&CatalogueSources {
            connections: &self.state.connections,
            catalogs: &self.state.catalogs,
            whatsapp: &self.connections.whatsapp,
            telegram: &self.connections.telegram,
            google: &self.google.state,
            github: &self.github.state,
            services: &self.services.services,
            probed: &self.connections.probed,
        })
    }

    fn connections_frame(&self, cx: &mut Context<Self>, body: AnyElement) -> AnyElement {
        let t = design::tokens(cx);
        let narrow = !self.width_class.fits_detail_pane();
        let column = if self.open_kind.is_some() {
            t.layout.reading_width
        } else {
            t.layout.page_width
        };
        v_flex()
            .id("connections-page")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .p(if narrow {
                t.space.small
            } else {
                t.space.medium
            })
            .child(
                v_flex()
                    .id("connections-scroll")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .overflow_y_scroll()
                    .test_support()
                    .child(
                        v_flex()
                            .id("connections-column")
                            .test_support()
                            .w_full()
                            .max_w(column)
                            .min_w_0()
                            .mx_auto()
                            .gap(t.space.medium)
                            .child(body),
                    ),
            )
            .into_any_element()
    }

    fn render_catalogue(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        let catalogue = self.current_catalogue();
        let mut rows = Vec::new();
        for card in catalogue.cards() {
            let slug = card.kind.slug();
            let kind = card.kind;
            let phase = card.phase.label();
            let color = match card.phase {
                LinkPhase::Linked => cx.theme().success,
                LinkPhase::NeedsYou => cx.theme().danger,
                LinkPhase::NotLinked | LinkPhase::Working => t.text_secondary,
            };
            rows.push(
                v_flex()
                    .id(gpui_kit::SharedString::from(format!(
                        "connection-card-{slug}"
                    )))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.medium)
                    .gap(t.space.small)
                    .sovra_surface(Surface::Matte, cx)
                    .child(
                        div()
                            .id(gpui_kit::SharedString::from(format!(
                                "connection-title-{slug}"
                            )))
                            .aria_label(card.title)
                            .test_support()
                            .sovra_text(TextRole::SectionTitle, cx)
                            .child(card.title),
                    )
                    .child(
                        div()
                            .id(gpui_kit::SharedString::from(format!(
                                "connection-sentence-{slug}"
                            )))
                            .aria_label(card.sentence)
                            .test_support()
                            .min_w_0()
                            .sovra_text(TextRole::Body, cx)
                            .child(card.sentence),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap(t.space.small)
                            .child(
                                div()
                                    .id(gpui_kit::SharedString::from(format!(
                                        "connection-status-{slug}"
                                    )))
                                    .aria_label(phase)
                                    .test_support()
                                    .text_color(color)
                                    .child(phase),
                            )
                            .child(
                                design::primary_button(
                                    gpui_kit::SharedString::from(format!(
                                        "connection-action-{slug}"
                                    )),
                                    card.action,
                                    cx,
                                )
                                .flex_shrink_0()
                                .on_click(cx.listener(
                                    move |view, _, window, cx| {
                                        view.open_catalogue_kind(kind, window, cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element(),
            );
        }
        v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .child(design::page_header(
                "Connections",
                "Each link is separate. Open one when you want to set it up.",
                cx,
            ))
            .child(
                design::button("connections-refresh", "Check links", cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.send_connection(ConnectionCommand::Refresh, cx);
                    },
                )),
            )
            .children(rows)
            .into_any_element()
    }

    fn render_open_connection(
        &mut self,
        kind: CatalogueKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = design::tokens(cx);
        let editor = match kind {
            CatalogueKind::OnlineAi => self.render_online_editor(cx),
            CatalogueKind::CompatibleService => self.render_connection_form(cx).into_any_element(),
            CatalogueKind::LocalAi => Self::render_local_editor(cx),
            CatalogueKind::WhatsApp => self.render_connections_section(cx).into_any_element(),
            CatalogueKind::Telegram => self.render_telegram_section(cx).into_any_element(),
            CatalogueKind::Google => self.google_card.clone().map_or_else(
                || caption("Google is not available in this window.", cx).into_any_element(),
                |card| div().w_full().min_w_0().child(card).into_any_element(),
            ),
            CatalogueKind::GitHub => self.github_card.clone().map_or_else(
                || caption("GitHub is not available in this window.", cx).into_any_element(),
                |card| div().w_full().min_w_0().child(card).into_any_element(),
            ),
            CatalogueKind::WebAddress => self.render_service_connect_section(cx).into_any_element(),
        };
        let feedback = matches!(
            kind,
            CatalogueKind::OnlineAi | CatalogueKind::CompatibleService
        )
        .then(|| {
            if let Some(error) = &self.state.error {
                Some(
                    div()
                        .id("connection-model-error")
                        .test_support()
                        .aria_label(error.clone())
                        .child(caption(error.clone(), cx).text_color(cx.theme().danger)),
                )
            } else {
                self.state.notice.as_ref().map(|notice| {
                    div()
                        .id("connection-model-notice")
                        .test_support()
                        .aria_label(notice.clone())
                        .child(caption(notice.clone(), cx))
                })
            }
        })
        .flatten();
        v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .child(
                design::button("connections-back", "All connections", cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.open_kind = None;
                        view.connection_form = None;
                        view.form_subscriptions.clear();
                        view.manual_form = None;
                        cx.notify();
                    },
                )),
            )
            .children(feedback)
            .child(editor)
            .into_any_element()
    }

    fn render_online_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // The key form needs the draft and the view together. Take the draft out
        // so both borrows can exist, then put it back for the next edit.
        let Some(form) = self.connection_form.take() else {
            return caption("Open this link again to add an online AI.", cx).into_any_element();
        };
        if form.original.kind == ProviderKind::OpenRouter {
            let element = self.render_key_form(&form, cx).into_any_element();
            self.connection_form = Some(form);
            element
        } else {
            self.connection_form = Some(form);
            self.render_connection_form(cx).into_any_element()
        }
    }

    fn render_local_editor(cx: &mut Context<Self>) -> AnyElement {
        let t = design::tokens(cx);
        v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .child(caption(
                "A free AI can stay on this computer. Setting it up opens the model list so you can download one.",
                cx,
            ))
            .child(
                design::primary_button("connection-local-setup", "Set up on this computer", cx)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(ShowLocalAi);
                    })),
            )
            .into_any_element()
    }
}
