//! Terminal cards: approval through real buttons, live output and Settings rules,
//! at the minimum window, a wide window and 200% interface size, in both themes.
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, Window, point, px, size};
use sovra_core::models::InterfaceScale;
use sovra_core::terminal::{
    AllowRule, ApprovalPrompt, AutoApproval, CallStatus, HostOs, HostSystem, Permission, Risk,
    ShellKind, TerminalCall, TerminalCommand, TerminalDecision, TerminalEvent, TerminalPolicy,
};
use sovra_core::{Command, Event, NavTab};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

fn rule(text: &str) -> AllowRule {
    AllowRule::new(text.split(' ').map(str::to_owned).collect()).expect("rule")
}

fn call(session_id: &str, status: CallStatus, permission: Permission) -> TerminalCall {
    TerminalCall {
        call_id: "call_0-1".into(),
        session_id: session_id.into(),
        command: "npm install --save-dev prettier && npm test".into(),
        description: "Install the formatter and run the tests".into(),
        working_dir: "/home/ada/projects/garden-planner".into(),
        status,
        permission,
        prompt: ApprovalPrompt {
            risks: vec![Risk::InstallsSoftware, Risk::UsesNetwork],
            suggested_rules: vec![rule("npm install"), rule("npm test")],
        },
    }
}

/// Scrolls `pane` until `target` is fully inside it, then returns whether it is.
fn reveal(
    window: &mut Window,
    pane: &'static str,
    target: (&'static str, usize),
    cx: &mut gpui_kit::App,
) -> bool {
    for _ in 0..600 {
        window.render_frame(cx);
        let Some(found) = window.try_find(target) else {
            return false;
        };
        let bounds = window.find(pane).bounds();
        let inside =
            found.bounds().top() >= bounds.top() && found.bounds().bottom() <= bounds.bottom();
        // Text taller than a tiny pane (200% in the minimum window) only needs to be
        // on screen; buttons are always shorter than the pane and must fit.
        let taller = found.bounds().size.height > bounds.size.height;
        let overlaps =
            found.bounds().top() < bounds.bottom() && found.bounds().bottom() > bounds.top();
        if found.visible() && (inside || (taller && overlaps)) {
            return true;
        }
        let step = px(24.);
        let delta = if found.bounds().top() < bounds.top() {
            step
        } else {
            -step
        };
        window.scroll(pane, ScrollDelta::Pixels(point(px(0.), delta)), cx);
    }
    false
}

fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(16));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn terminal_cards_and_settings_work_at_every_size(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
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
            let label = format!("{mode:?} {width}x{height} at {scale}%");

            events
                .send_blocking(Event::Terminal(TerminalEvent::Call(call(
                    &session,
                    CallStatus::AwaitingApproval,
                    Permission::Pending,
                ))))
                .unwrap();
            settle(cx);
            assert!(
                !std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|c| matches!(c, Command::Terminal(_))),
                "rendering must not decide"
            );
            assert_eq!(
                shell.read_with(cx, |s, _| s.workspace().terminal.calls().count()),
                1,
                "{label}: the call was folded"
            );
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.try_find(("terminal-card", 0usize)).is_some(),
                    "{label}: card rendered"
                );
                for button in [
                    "terminal-allow-once",
                    "terminal-always-allow",
                    "terminal-deny",
                ] {
                    assert!(
                        reveal(window, "chat-scroll-area", (button, 0), cx),
                        "{label}: {button} not reachable"
                    );
                    assert!(
                        window.find((button, 0usize)).bounds().right() <= px(width),
                        "{label}: {button} overflows"
                    );
                }
                assert!(
                    reveal(window, "chat-scroll-area", ("terminal-command", 0), cx),
                    "{label}: command {:?} visible={} in pane {:?}",
                    window.find(("terminal-command", 0usize)).bounds(),
                    window.find(("terminal-command", 0usize)).visible(),
                    window.find("chat-scroll-area").bounds()
                );
                assert!(reveal(
                    window,
                    "chat-scroll-area",
                    ("terminal-always-allow", 0),
                    cx
                ));
                window.click(("terminal-always-allow", 0usize), cx);
            })
            .unwrap();
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok()).any(|c| c
                    == Command::Terminal(TerminalCommand::Decide {
                        call_id: "call_0-1".into(),
                        decision: TerminalDecision::AlwaysAllow,
                    })),
                "{label}: always allow was not sent"
            );

            // Running: live output and Stop; approval buttons are gone.
            events
                .send_blocking(Event::Terminal(TerminalEvent::Call(call(
                    &session,
                    CallStatus::Running,
                    Permission::Saved(vec![rule("npm install"), rule("npm test")]),
                ))))
                .unwrap();
            events
                .send_blocking(Event::Terminal(TerminalEvent::Output {
                    call_id: "call_0-1".into(),
                    lines: (1..=30).map(|n| format!("added package {n}")).collect(),
                    partial: "testing…".into(),
                    skipped_lines: 0,
                }))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find(("terminal-allow-once", 0usize)).is_none());
                assert!(
                    reveal(window, "chat-scroll-area", ("terminal-expand", 0), cx),
                    "{label}: expand"
                );
                assert!(
                    reveal(window, "chat-scroll-area", ("terminal-stop", 0), cx),
                    "{label}: stop"
                );
                window.click(("terminal-stop", 0usize), cx);
            })
            .unwrap();
            assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|c| c
                == Command::Terminal(TerminalCommand::Stop {
                    call_id: "call_0-1".into()
                })));

            // Finished: the reason it ran is shown instead of buttons.
            events
                .send_blocking(Event::Terminal(TerminalEvent::Call(call(
                    &session,
                    CallStatus::Finished {
                        exit_code: Some(0),
                        duration_ms: 4_200,
                    },
                    Permission::Auto(AutoApproval::ReadOnly),
                ))))
                .unwrap();
            events
                .send_blocking(Event::Terminal(TerminalEvent::Ready {
                    host: HostSystem {
                        os: HostOs::Linux,
                        os_version: "Ubuntu 24.04.1 LTS".into(),
                        arch: "x86_64".into(),
                        shell: ShellKind::Bash,
                        shell_path: "/usr/bin/bash".into(),
                        working_dir: "/home/ada".into(),
                    },
                    policy: TerminalPolicy {
                        enabled: true,
                        rules: vec![rule("npm test"), rule("cargo build")],
                    },
                }))
                .unwrap();
            settle(cx);
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find(("terminal-stop", 0usize)).is_none());
                assert!(
                    reveal(window, "chat-scroll-area", ("terminal-permission", 0), cx),
                    "{label}: reason"
                );

                shell.update(cx, |shell, cx| shell.select_tab(NavTab::Settings, cx));
                window.render_frame(cx);
                let found = window
                    .try_find(("terminal-rule-remove", 1usize))
                    .map(|e| (e.bounds(), e.visible()));
                assert!(
                    reveal(window, "settings-sections", ("terminal-rule-remove", 1), cx),
                    "{label}: remove rule button {found:?}, pane {:?}",
                    window.try_find("settings-sections").map(|e| e.bounds())
                );
                assert!(
                    window
                        .find(("terminal-rule-remove", 1usize))
                        .bounds()
                        .right()
                        <= px(width)
                );
                window.click(("terminal-rule-remove", 1usize), cx);
                window.remove_window();
            })
            .unwrap();
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|c| c
                        == Command::Terminal(TerminalCommand::RemoveRule(rule("cargo build")))),
                "{label}: remove rule was not sent"
            );
        }
    }
}
