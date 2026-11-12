//! Phone controls remain reachable at minimum, wide and enlarged-text layouts.
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point, px, size};
use sovra_core::{
    Command, Event,
    group::{DeviceId, PeerAddress},
    models::InterfaceScale,
    remote_control::{InvitationSecret, PairingTicket, RemoteCommand, RemoteEvent, RemoteState},
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

#[gpui_kit::test]
fn phone_badge_and_stop_fit_all_supported_sizes(cx: &mut TestAppContext) {
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
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                Root::new(cx.new(|cx| Shell::new(0, window, cx)), window, cx)
            });
            events
                .send_blocking(Event::RemoteControl(Box::new(RemoteEvent::State(
                    RemoteState {
                        generation: Some(DeviceId([1; 32])),
                        invitation: Some(PairingTicket {
                            version: 1,
                            address: PeerAddress {
                                device: DeviceId([2; 32]),
                                relay: None,
                                addrs: Vec::new(),
                            },
                            secret: InvitationSecret([3; 32]),
                        }),
                        qr: vec![vec![false; 21]; 21],
                        ..RemoteState::default()
                    },
                ))))
                .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                let badge = window.find("remote-control-badge");
                assert!(badge.visible());
                assert!(badge.bounds().right() <= px(width));
                window.click("remote-control-badge", cx);
                window.render_frame(cx);
                let qr = window.find("remote-control-qr-grid").bounds();
                let pane = window.find("settings-sections").bounds();
                assert!((f32::from(qr.size.width - qr.size.height)).abs() < 1.);
                assert!(qr.left() >= pane.left());
                assert!(
                    qr.right() <= pane.right(),
                    "pairing QR fits the pane at {width}px and {scale}%"
                );
                for _ in 0..100 {
                    window.render_frame(cx);
                    let button = window.find("remote-control-toggle");
                    let pane = window.find("settings-sections").bounds();
                    if button.visible()
                        && button.bounds().top() >= pane.top()
                        && button.bounds().bottom() <= pane.bottom()
                    {
                        break;
                    }
                    window.scroll(
                        "settings-sections",
                        ScrollDelta::Pixels(point(px(0.), px(-30.))),
                        cx,
                    );
                }
                let button = window.find("remote-control-toggle");
                assert!(button.visible());
                assert!(button.bounds().right() <= px(width));
                window.click("remote-control-toggle", cx);
            })
            .unwrap();
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|command| command == Command::RemoteControl(RemoteCommand::Stop))
            );
            cx.update_window(window.into(), |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}
