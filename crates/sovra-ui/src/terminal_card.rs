//! The terminal card in the chat: what Sovra wants to run, why, how risky it is,
//! the three choices (Allow once / Always allow / Don't allow), then the live
//! output and the result. Only these real buttons send decisions; nothing the
//! model writes can press them.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, button::ButtonVariants as _, h_flex,
    shimmer::ShimmerText, v_flex,
};
use gpui_kit::{
    ClipboardItem, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::Command;
use sovra_core::terminal::{
    CallStatus, CallView, Risk, TerminalCommand, TerminalDecision, quoted_rules,
};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::{RuntimeClient, Shell};

/// Lines shown while the card is collapsed: enough to follow progress at a glance.
const COLLAPSED_LINES: usize = 8;

fn send(cx: &mut gpui_kit::App, command: TerminalCommand) {
    let _ = cx
        .global::<RuntimeClient>()
        .0
        .send(Command::Terminal(command));
}

pub(crate) fn render(
    view: &CallView,
    index: usize,
    expanded: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let call = &view.call;
    let theme = cx.theme();
    let (icon, tone): (IconName, Hsla) = match &call.status {
        CallStatus::AwaitingApproval => (IconName::SquareTerminal, theme.warning),
        CallStatus::Running => (IconName::LoaderCircle, t.accent.violet),
        status if status.is_failure() => (IconName::CircleX, theme.danger),
        CallStatus::Finished { .. } => (IconName::CircleCheck, theme.success),
        _ => (IconName::Ban, t.text_secondary),
    };
    let title = match call.status {
        CallStatus::AwaitingApproval => "Sovra wants to run a command",
        CallStatus::Running => "Running a command",
        _ => "Terminal command",
    };

    let header = h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(t.space.small)
        .child(Icon::new(icon).small().text_color(tone).flex_shrink_0())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(title),
        )
        .child(status_pill(
            index,
            call.status.label(),
            tone,
            call.status == CallStatus::Running,
            cx,
        ));

    let mut card = v_flex()
        .id(("terminal-card", index))
        .test_support()
        .aria_label(format!("{title}: {}", call.command))
        .w_full()
        .min_w_0()
        .sovra_surface(Surface::Subtle, cx)
        .p(t.space.medium)
        .gap(t.space.small)
        .child(header);
    if !call.description.is_empty() {
        card = card.child(
            div()
                .sovra_text(TextRole::Body, cx)
                .whitespace_normal()
                .child(call.description.clone()),
        );
    }
    card = card.child(terminal_pane(view, index, expanded, cx));

    if !call.prompt.risks.is_empty() && call.status == CallStatus::AwaitingApproval {
        card = card.child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.compact)
                .children(call.prompt.risks.iter().map(|risk| risk_chip(*risk, cx))),
        );
    }

    match &call.status {
        CallStatus::AwaitingApproval => card = card.child(approval_actions(view, index, cx)),
        CallStatus::Running => {
            let call_id = call.call_id.clone();
            card = card.child(
                h_flex().w_full().flex_wrap().gap(t.space.small).child(
                    design::button(("terminal-stop", index), "Stop", cx)
                        .small()
                        .icon(IconName::Square)
                        .accessibility_label("Stop this command")
                        .on_click(move |_, _, cx| {
                            send(
                                cx,
                                TerminalCommand::Stop {
                                    call_id: call_id.clone(),
                                },
                            );
                        }),
                ),
            );
        }
        _ => {
            if let Some(reason) = call.permission.label() {
                card = card.child(
                    div()
                        .id(("terminal-permission", index))
                        .test_support()
                        .sovra_text(TextRole::Caption, cx)
                        .text_color(t.text_secondary)
                        .whitespace_normal()
                        .child(reason),
                );
            }
        }
    }
    card
}

fn status_pill(
    index: usize,
    label: String,
    tone: Hsla,
    live: bool,
    cx: &Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    div()
        .id(("terminal-status", index))
        .test_support()
        .flex_shrink_0()
        .max_w(t.layout.content_min_width)
        .truncate()
        .px(t.space.small)
        .py(t.space.hair)
        .rounded(t.radius.pill)
        .bg(tone.alpha(0.12))
        .text_color(tone)
        .sovra_text(TextRole::Caption, cx)
        .aria_label(label.clone())
        .child(if live {
            ShimmerText::new(label).into_any_element()
        } else {
            div().child(label).into_any_element()
        })
}

/// The command as a terminal shows it, with its output underneath.
fn terminal_pane(
    view: &CallView,
    index: usize,
    expanded: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let call = &view.call;
    let mono = cx.theme().mono_font_family.clone();
    let folder = short_folder(&call.working_dir);
    let total = view.lines.len() + usize::from(!view.partial.is_empty());
    let shown: Vec<&str> = view
        .lines
        .iter()
        .map(String::as_str)
        .chain((!view.partial.is_empty()).then_some(view.partial.as_str()))
        .collect();
    let hidden_above = if expanded {
        0
    } else {
        shown.len().saturating_sub(COLLAPSED_LINES)
    };
    let output = shown[hidden_above..].join("\n");
    let omitted = view.omitted_lines + hidden_above as u64;

    // The folder gets its own line so a long path can never squeeze the command
    // itself in a narrow pane or at large text sizes.
    let command_line = v_flex()
        .w_full()
        .min_w_0()
        .child(
            div()
                .w_full()
                .min_w_0()
                .truncate()
                .text_color(t.text_secondary)
                .child(folder),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(t.accent.violet_light)
                        .child("$"),
                )
                .child(
                    div()
                        .id(("terminal-command", index))
                        .test_support()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .text_color(cx.theme().foreground)
                        .child(call.command.clone()),
                ),
        );

    let mut pane = v_flex()
        .w_full()
        .min_w_0()
        .p(t.space.small)
        .gap(t.space.compact)
        .rounded(t.chat.code_radius)
        .bg(t.code_background)
        .font_family(mono)
        .sovra_text(TextRole::Caption, cx)
        .child(command_line);

    if total > 0 {
        if omitted > 0 {
            pane = pane.child(
                div()
                    .text_color(t.text_secondary)
                    .child(format!("… {omitted} earlier lines")),
            );
        }
        let body = div()
            .id(("terminal-output", index))
            .test_support()
            .w_full()
            .min_w_0()
            .whitespace_normal()
            .text_color(cx.theme().foreground.alpha(0.85))
            .child(output);
        pane = pane.child(if expanded {
            body.max_h(t.chat.terminal_output_max_height)
                .overflow_y_scroll()
                .into_any_element()
        } else {
            body.into_any_element()
        });
    }

    let mut footer = h_flex().w_full().flex_wrap().gap(t.space.hair);
    if shown.len() > COLLAPSED_LINES {
        let call_id = call.call_id.clone();
        footer = footer.child(
            design::button(
                ("terminal-expand", index),
                if expanded {
                    "Show less"
                } else {
                    "Show all output"
                },
                cx,
            )
            .ghost()
            .xsmall()
            .icon(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .on_click(cx.listener(move |shell, _, _, cx| {
                if !shell.chat.terminal_expanded.remove(&call_id) {
                    shell.chat.terminal_expanded.insert(call_id.clone());
                }
                shell.chat.remeasure_terminal();
                shell.refresh_transcript(cx);
            })),
        );
    }
    let copy_command = call.command.clone();
    footer = footer.child(
        design::button(("terminal-copy-command", index), "Copy command", cx)
            .ghost()
            .xsmall()
            .icon(IconName::Copy)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_command.clone()));
            }),
    );
    if total > 0 {
        let copy_output = shown.join("\n");
        footer = footer.child(
            design::button(("terminal-copy-output", index), "Copy output", cx)
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_output.clone()));
                }),
        );
    }
    v_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.hair)
        .child(pane)
        .child(footer)
}

/// `~/projects/app` for paths under the home folder; otherwise the last two parts.
fn short_folder(path: &str) -> String {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [] => path.to_owned(),
        // `/home/<name>` and `/Users/<name>` are the home folder.
        ["home" | "Users", _] => "~".to_owned(),
        ["home" | "Users", _, rest @ ..] if rest.len() <= 2 => format!("~/{}", rest.join("/")),
        [.., parent, last] => format!("…/{parent}/{last}"),
        [only] => (*only).to_owned(),
    }
}

fn risk_chip(risk: Risk, cx: &Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let tone = if risk.is_serious() {
        cx.theme().danger
    } else {
        cx.theme().warning
    };
    h_flex()
        .items_center()
        .gap(t.space.hair)
        .px(t.space.small)
        .py(t.space.hair)
        .rounded(t.radius.pill)
        .border_1()
        .border_color(tone.alpha(0.4))
        .text_color(tone)
        .sovra_text(TextRole::Caption, cx)
        .child(
            Icon::new(if risk.is_serious() {
                IconName::TriangleAlert
            } else {
                IconName::CircleAlert
            })
            .xsmall(),
        )
        .child(risk.label())
}

fn approval_actions(
    view: &CallView,
    index: usize,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let call = &view.call;
    let decide = |decision: TerminalDecision| {
        let call_id = call.call_id.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            send(
                cx,
                TerminalCommand::Decide {
                    call_id: call_id.clone(),
                    decision,
                },
            );
        }
    };
    let rules = &call.prompt.suggested_rules;
    let mut buttons = h_flex().w_full().flex_wrap().gap(t.space.small).child(
        design::primary_button(("terminal-allow-once", index), "Allow once", cx)
            .accessibility_label(format!("Allow this command once: {}", call.command))
            .on_click(decide(TerminalDecision::AllowOnce)),
    );
    if !rules.is_empty() {
        let label = match rules.as_slice() {
            [rule] => format!("Always allow “{rule}”"),
            _ => "Always allow these".to_owned(),
        };
        buttons = buttons.child(
            design::button(("terminal-always-allow", index), label, cx)
                .accessibility_label(format!(
                    "Always allow commands starting with {}",
                    quoted_rules(rules)
                ))
                .on_click(decide(TerminalDecision::AlwaysAllow)),
        );
    }
    buttons = buttons.child(
        design::button(("terminal-deny", index), "Don't allow", cx)
            .ghost()
            .accessibility_label("Don't run this command")
            .on_click(decide(TerminalDecision::Deny)),
    );
    let hint = if rules.is_empty() {
        "This kind of command is checked every time, so there's no “always” option.".to_owned()
    } else {
        format!(
            "“Always allow” lets commands starting with {} run without asking. You can undo it in Settings.",
            quoted_rules(rules)
        )
    };
    v_flex().w_full().gap(t.space.compact).child(buttons).child(
        div()
            .sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary)
            .whitespace_normal()
            .child(hint),
    )
}

#[cfg(test)]
mod tests {
    use super::short_folder;

    #[test]
    fn folders_are_shortened_for_the_card() {
        assert_eq!(short_folder("/home/ada"), "~");
        assert_eq!(short_folder("/Users/ada/code/app"), "~/code/app");
        assert_eq!(short_folder("/srv/data/projects/app"), "…/projects/app");
        assert_eq!(short_folder("C:\\Users\\ada"), "…/Users/ada");
        assert_eq!(short_folder("/"), "/");
        assert_eq!(short_folder("/tmp"), "tmp");
    }
}
