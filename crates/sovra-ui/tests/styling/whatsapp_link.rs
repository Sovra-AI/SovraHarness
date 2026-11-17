//! Link instructions stay inside the pairing card.

#![allow(
    clippy::unwrap_used,
    reason = "headless UI fixtures fail loudly when the scripted runtime is gone"
)]

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use sovra_core::connections::{ConnectionEvent, WhatsAppQrTicket, WhatsAppStatus};
use sovra_core::models::InterfaceScale;
use sovra_core::{Event, NavTab};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn whatsapp_link_instructions_stay_inside_the_card(cx: &mut TestAppContext) {
    for (width, height, scale) in [(640., 420., 100u16), (2560., 1440., 100), (640., 420., 200)] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            let (client, _commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                Theme::change(mode, None, cx);
            });
            let mut shell_entity = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let shell = cx.new(|cx| Shell::new(0, window, cx));
                shell_entity = Some(shell.clone());
                Root::new(shell, window, cx)
            });
            let shell = shell_entity.unwrap();
            let ticket = WhatsAppQrTicket {
                qr_payload: "2@ref,noise,identity,adv,7".into(),
                qr_matrix: vec![vec![true, false], vec![false, true]],
                expires_in_secs: 59,
                pairing_code: None,
            };
            events
                .send_blocking(Event::Connections(ConnectionEvent::StatusChanged(
                    WhatsAppStatus::AwaitingQrScan { ticket },
                )))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Connections, cx));
                for _ in 0..4 {
                    window.render_frame(cx);
                }
                let where_ = format!("{width}×{height} at {scale}% {mode:?}");
                let modal = window.find("whatsapp-qr-modal").bounds();
                let instructions = window.find("whatsapp-link-instructions");
                let label = instructions.label().unwrap_or_default();
                assert!(
                    label.contains("Linked devices") && label.contains("Sovra connection"),
                    "instructions lost their sentence at {where_}: {label}"
                );
                let text = instructions.bounds();
                assert!(
                    text.left() >= modal.left() - px(1.) && text.right() <= modal.right() + px(1.),
                    "WhatsApp instructions run past the card at {where_}: text {text:?} card {modal:?}"
                );
                assert!(
                    text.top() >= modal.top() - px(1.) && text.bottom() <= modal.bottom() + px(1.),
                    "WhatsApp instructions run past the card vertically at {where_}: text {text:?} card {modal:?}"
                );
                let cancel = window.find("whatsapp-qr-close").bounds();
                assert!(
                    cancel.left() >= modal.left() - px(1.)
                        && cancel.right() <= modal.right() + px(1.),
                    "Cancel runs past the card at {where_}: {cancel:?} card {modal:?}"
                );
                let code = window.find("whatsapp-qr-matrix").bounds();
                assert!(
                    text.bottom() <= code.top() + px(1.),
                    "instructions cover the code at {where_}: text {text:?} code {code:?}"
                );
                if width <= 640. {
                    let line = window.rem_size() * 0.75 * 1.5;
                    assert!(
                        text.size.height > line * 1.4,
                        "instructions stayed one line at {where_}: {text:?} line {line:?}"
                    );
                }
                window.remove_window();
            })
            .unwrap();
        }
    }
}
