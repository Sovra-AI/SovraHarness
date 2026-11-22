//! The uninstall review reads the installed snapshot, including for switched-off
//! plugins. Its guide is plain text. Opening it never runs a publisher's code.

use std::fmt::Write as _;

use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _,
};
use sovra_core::plugins::{InstalledPlugin, PluginPackage};

use crate::{
    design::{self, SovraStyled as _, TextRole},
    shell::Shell,
};

fn plan(plugin: &InstalledPlugin, package: Option<&PluginPackage>) -> String {
    let mut text = format!(
        "Cleanup for {}\n{}\n{}\n\n",
        super::display_title(plugin),
        super::source(plugin),
        plugin.catalog_url
    );
    text.push_str("If you only added this plugin, confirm Remove to finish. If you used it, open the chat where you set it up. Check which computers, accounts and services you changed. Save any work you want to keep.\n\n");
    if let Some(guide) =
        package.and_then(|p| p.skills.iter().find(|s| s.name == "uninstall-plugin"))
    {
        let _ = write!(
            text,
            "From the installed publisher guide\n{}\n\n",
            guide.instructions.trim()
        );
    } else {
        text.push_str("On each affected computer, review the setup steps you actually completed. Stop any background work you started, undo those settings, and remove files or programs you installed solely for this plugin. Check that services stay stopped and the settings have been restored.\n\n");
    }
    if let Some(package) = package.filter(|p| !p.services.is_empty()) {
        text.push_str("Connections this version suggested\n");
        for service in &package.services {
            let _ = writeln!(text, "• {} ({})", service.name, service.address);
        }
        text.push('\n');
    }
    text.push_str("In Connections, remove services you connected only for this plugin. Check the provider account for permissions you also want to revoke. Return to Plugins and confirm Remove when you are ready.");
    text
}

pub(super) fn review(
    index: usize,
    plugin: &InstalledPlugin,
    package: Option<&PluginPackage>,
    shell: &Shell,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let title = super::display_title(plugin);
    let guide = plan(plugin, package);
    let request = format!(
        "Help me undo the changes I made using this plugin. Use the guide below as reference. First identify the changes on each computer with me. Ask before stopping services or deleting anything, preserve shared resources and my work, and verify each change.\n\n{guide}"
    );
    let busy = shell.plugins.state.busy;
    let has_draft = !shell.composer_input.read(cx).value().is_empty();
    let actions = h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap(t.space.small)
        .child(
            design::button(("plugin-cleanup-copy", index), "Copy guide", cx)
                .small()
                .disabled(busy)
                .accessibility_label(format!("Copy cleanup guide for {title}"))
                .on_click({
                    let guide = guide.clone();
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(guide.clone()))
                }),
        )
        .child(
            design::button(("plugin-cleanup-chat", index), "Guide me in chat", cx)
                .small()
                .disabled(busy || has_draft)
                .accessibility_label(format!("Draft a cleanup request for {title}"))
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.plugins.confirm_remove = None;
                    shell
                        .state
                        .set_room(sovra_core::teams::TeamRoom::ordinary());
                    shell.select_tab(sovra_core::NavTab::Chat, cx);
                    shell.fill_composer_draft(&request, window, cx);
                })),
        )
        .child(
            design::button(
                ("plugin-cleanup-connections", index),
                "Open Connections",
                cx,
            )
            .small()
            .accessibility_label(format!("Review connections for {title}"))
            .on_click(cx.listener(|shell, _, window, cx| {
                shell.plugins.confirm_remove = None;
                shell.present_connections(None, false, window, cx);
            })),
        )
        .child(
            design::button(("plugin-cleanup-cancel", index), "Cancel", cx)
                .small()
                .accessibility_label(format!("Cancel removing {title}"))
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.plugins.confirm_remove = None;
                    shell.plugins.state.details = None;
                    cx.notify();
                })),
        );
    v_flex().w_full().min_w_0().gap(t.space.small)
        .child(actions)
        .children(has_draft.then(|| design::paragraph("You have an unsent chat draft. Copy the guide or finish your draft to use guided chat.").sovra_text(TextRole::Caption, cx)))
        .child(design::paragraph(guide).id(("plugin-cleanup-guide", index)).test_support()
            .max_w(t.layout.reading_width).whitespace_normal().sovra_text(TextRole::Body, cx))
}
