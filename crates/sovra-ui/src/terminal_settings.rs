//! Settings → Terminal: switch the assistant's terminal access on or off, see the
//! computer it runs on, and take back any "Always allow" choice.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, button::ButtonVariants as _, checkbox::Checkbox, h_flex, v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::Command;
use sovra_core::terminal::TerminalCommand;

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::models_view::{ModelsView, caption};
use crate::settings_view::settings_section;

impl ModelsView {
    pub(crate) fn render_terminal_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let policy = &self.terminal.policy;
        let enabled = policy.enabled;
        let mut section = settings_section(
            "settings-terminal",
            "Terminal",
            "Let Sovra run commands on this computer, like a helper typing into the terminal for you. Anything that changes files, installs software or uses the internet waits for your OK. Commands that only look at things run straight away.",
            cx,
        )
        .child(
            Checkbox::new("terminal-access-toggle")
                .cursor_pointer()
                .label("Let Sovra use the terminal")
                .checked(enabled)
                .on_click(cx.listener(|view, checked: &bool, _, cx| {
                    // Show the change at once; the runtime confirms it.
                    view.terminal.policy.enabled = *checked;
                    let _ = cx
                        .global::<RuntimeClient>()
                        .0
                        .send(Command::Terminal(TerminalCommand::SetEnabled(*checked)));
                    cx.notify();
                })),
        );
        if let Some(host) = &self.terminal.host {
            section = section.child(
                div()
                    .id("terminal-host")
                    .test_support()
                    .min_w_0()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .whitespace_normal()
                    .child(format!(
                        "This computer: {} · {} · starts in {}",
                        host.os_version,
                        host.shell.label(),
                        host.working_dir
                    )),
            );
        }
        section = section.child(
            div()
                .pt(t.space.small)
                .sovra_text(TextRole::Body, cx)
                .child("Always allowed"),
        );
        if policy.rules.is_empty() {
            return section.child(caption(
                "Nothing yet. Choose “Always allow” on a command card and similar commands will run without asking.",
                cx,
            ));
        }
        let mono = cx.theme().mono_font_family.clone();
        section.child(
            v_flex()
                .id("terminal-rules")
                .test_support()
                .w_full()
                .gap(t.space.compact)
                .children(policy.rules.iter().enumerate().map(|(index, rule)| {
                    let remove = rule.clone();
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(t.space.small)
                        .p(t.space.compact)
                        .sovra_surface(Surface::Subtle, cx)
                        .child(
                            div()
                                .id(("terminal-rule", index))
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(mono.clone())
                                .sovra_text(TextRole::Caption, cx)
                                .child(format!("{rule} …")),
                        )
                        .child(
                            design::button(("terminal-rule-remove", index), "Remove", cx)
                                .ghost()
                                .xsmall()
                                .accessibility_label(format!(
                                    "Stop always allowing commands starting with {rule}"
                                ))
                                .on_click(move |_, _, cx| {
                                    let _ = cx.global::<RuntimeClient>().0.send(Command::Terminal(
                                        TerminalCommand::RemoveRule(remove.clone()),
                                    ));
                                }),
                        )
                })),
        )
    }
}
