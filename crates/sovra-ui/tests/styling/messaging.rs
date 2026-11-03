//! Messaging reviews are usable through native controls at every supported size.
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px, size};
use sovra_core::communications::{MessageDelivery, OutboundMessage};
use sovra_core::connections::{
    ChannelKind, ConnectionCommand, ConnectionEvent, ConnectionsState, LinkedDevice,
    TelegramStatus, WhatsAppStatus,
};
use sovra_core::models::InterfaceScale;
use sovra_core::{Command, Event, Timestamp};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn messaging_review_and_settings_work_at_minimum_wide_and_double_size(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [
            (640., 420., 100),
            (2560., 1440., 100),
            (640., 420., 200),
            (2560., 1440., 200),
        ] {
            let (client, commands, events) = Client::loopback();
            cx.update(|cx| {
                sovra_ui::init(client, cx);
                Theme::change(mode, None, cx);
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
            });
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.unwrap();
            let session = shell.read_with(cx, |s, _| s.workspace().session_id.clone());
            let draft = OutboundMessage {
                id: "fixture-review".into(),
                session_id: session.clone(),
                channel: ChannelKind::WhatsApp,
                sender: "+447700900123".into(),
                recipient: "447700900123@s.whatsapp.net".into(),
                recipient_label: "Your own chat".into(),
                text: "Bring the shopping list.\nThis is the exact message to send.".into(),
                delivery: MessageDelivery::Review,
                created_at: Timestamp(0),
                expires_at: Timestamp(i64::MAX),
            };
            events
                .send_blocking(Event::Connections(ConnectionEvent::OutboundChanged(
                    draft.clone(),
                )))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                    c,
                    Command::Connections(ConnectionCommand::ReviewMessage { .. })
                )),
                "rendering must not approve"
            );
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("chat-composer-input").visible());
                for _ in 0..100 {
                    window.render_frame(cx);
                    let send = window.find(("message-review-send", 0usize));
                    let pane = window.find("chat-scroll-area").bounds();
                    if send.visible()
                        && send.bounds().top() >= pane.top()
                        && send.bounds().bottom() <= pane.bottom()
                    {
                        break;
                    }
                    let delta = if send.bounds().top() < pane.top() {
                        30.
                    } else {
                        -30.
                    };
                    window.scroll(
                        "chat-scroll-area",
                        ScrollDelta::Pixels(point(px(0.), px(delta))),
                        cx,
                    );
                }
                let send = window.find(("message-review-send", 0usize));
                assert!(
                    send.visible(),
                    "{mode:?} {width}x{height} at {scale}%: send {:?}, pane {:?}",
                    send.bounds(),
                    window.find("chat-scroll-area").bounds()
                );
                assert!(send.bounds().right() <= px(width));
                let pane = window.find("chat-scroll-area").bounds();
                assert!(send.bounds().top() >= pane.top());
                assert!(send.bounds().bottom() <= pane.bottom());
                window.click(("message-review-send", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|c| c
                == Command::Connections(ConnectionCommand::ReviewMessage {
                    id: draft.id.clone(),
                    session_id: session.clone(),
                    send: true
                })));
            let mut receipt = draft;
            receipt.delivery = MessageDelivery::Accepted {
                message_id: "sent-1".into(),
            };
            events
                .send_blocking(Event::Connections(ConnectionEvent::OutboundChanged(
                    receipt,
                )))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find(("message-review-send", 0usize)).is_none());
            })
            .unwrap();
            let state = ConnectionsState {
                whatsapp: WhatsAppStatus::Connected {
                    device: LinkedDevice {
                        phone_display: "+447700900123".into(),
                        device_jid: "447700900123@s.whatsapp.net".into(),
                        companion_name: "fixture".into(),
                        linked_at: Timestamp(0),
                    },
                    online: true,
                },
                telegram: TelegramStatus::Connected {
                    bot_username: "fixture_bot".into(),
                    owner_chat_id: "777".into(),
                    owner_name: "Fixture".into(),
                    linked_at: Timestamp(0),
                    online: true,
                },
                ..Default::default()
            };
            events
                .send_blocking(Event::Connections(ConnectionEvent::Loaded(Box::new(state))))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                // At 200% the collapsed rail stacks Connections on the account
                // button, so a click there signs out. Open the tab directly.
                shell.update(cx, |shell, cx| {
                    shell.select_tab(sovra_core::NavTab::Connections, cx);
                });
                window.render_frame(cx);
                for _ in 0..40 {
                    window.render_frame(cx);
                    let action = window.find("connection-action-whatsapp");
                    let pane = window.find("connections-scroll").bounds();
                    if action.visible()
                        && action.bounds().top() >= pane.top()
                        && action.bounds().bottom() <= pane.bottom()
                    {
                        break;
                    }
                    let delta = if action.bounds().top() < pane.top() {
                        80.
                    } else {
                        -80.
                    };
                    window.scroll(
                        "connections-scroll",
                        ScrollDelta::Pixels(point(px(0.), px(delta))),
                        cx,
                    );
                }
                window.click("connection-action-whatsapp", cx);
                window.render_frame(cx);
                assert!(window.try_find("whatsapp-send-test-btn").is_none());
                assert!(window.try_find("whatsapp-test-msg-input").is_none());
                assert!(window.try_find("telegram-send-test-btn").is_none());
                assert!(window.try_find("telegram-test-msg-input").is_none());
                assert!(window.try_find("whatsapp-toggle-auto-reply").is_some());
                window.remove_window();
            })
            .unwrap();
        }
    }
}
