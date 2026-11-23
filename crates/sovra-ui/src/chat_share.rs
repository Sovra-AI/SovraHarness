//! "Share with group" on a chat message: one press sends a reply, or something
//! the person wrote, to a group they are in, without leaving the chat. With one
//! group the button names it; with several it opens a choice. What is sent, a
//! note or a document, is decided in `sovra_core::group::share_from_chat`.

use std::collections::HashMap;

use gpui_kit::component::{ActiveTheme as _, Sizable as _, button::ButtonVariants as _, h_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
};
use sovra_core::Command;
use sovra_core::group::{GroupCommand, GroupEvent, GroupId, GroupSummary, share_from_chat};

use crate::design;
use crate::models_view::caption;
use crate::shell::Shell;

/// What the chat knows about sharing with groups.
#[derive(Default)]
pub(crate) struct ChatShare {
    /// The groups to offer, the one on the Groups page first.
    groups: Vec<GroupSummary>,
    /// The message whose choice of groups is open.
    choosing: Option<String>,
    /// The message being shared, until its group says it is saved.
    waiting: Option<String>,
    /// Messages already shared, and the group each went to.
    shared: HashMap<String, String>,
    /// The message that could not be shared, and why.
    failed: Option<(String, String)>,
}

impl ChatShare {
    /// Follows what the groups say: which exist, and how a share ended.
    pub(crate) fn apply(&mut self, event: &GroupEvent, groups: Vec<GroupSummary>) {
        self.groups = groups;
        match event {
            GroupEvent::Shared { group_name, .. } => {
                if let Some(message) = self.waiting.take() {
                    self.shared.insert(message, group_name.clone());
                    self.failed = None;
                }
            }
            GroupEvent::Problem(problem) => {
                if let Some(message) = self.waiting.take() {
                    self.failed = Some((message, problem.message().to_owned()));
                }
            }
            _ => {}
        }
    }

    /// The share control for one message, or `None` when there is no group to
    /// share with or nothing to share.
    pub(crate) fn control(
        &self,
        message: &str,
        text: &str,
        index: usize,
        cx: &Context<Shell>,
    ) -> Option<AnyElement> {
        if self.groups.is_empty() || text.trim().is_empty() {
            return None;
        }
        let t = design::tokens(cx);
        let row = h_flex()
            .id(("share-message-row", index))
            .test_support()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap(t.space.compact);
        if let Some(group) = self.shared.get(message) {
            return Some(
                row.child(
                    caption(format!("✓ Shared with {group}"), cx)
                        .id(("share-message-done", index))
                        .aria_label(format!("Shared with {group}"))
                        .test_support(),
                )
                .into_any_element(),
            );
        }
        if self.waiting.as_deref() == Some(message) {
            return Some(row.child(caption("Sharing…", cx)).into_any_element());
        }
        let send = |group: GroupId| {
            let (message, text) = (message.to_owned(), text.to_owned());
            cx.listener(move |shell: &mut Shell, _, _, cx| {
                shell.share_with_group(group, &message, &text, cx);
            })
        };
        let mut row = match self.groups.as_slice() {
            [only] => row.child(
                design::button(
                    ("share-message", index),
                    format!("Share with {}", only.name),
                    cx,
                )
                .ghost()
                .xsmall()
                .px(t.space.compact)
                .accessibility_label(format!("Share this with your group {}", only.name))
                .tooltip("Everyone in the group gets it, and their Sovra can use it")
                .on_click(send(only.group)),
            ),
            groups => {
                let choosing = self.choosing.as_deref() == Some(message);
                let toggled = message.to_owned();
                let row = row.child(
                    design::button(("share-message", index), "Share with a group", cx)
                        .ghost()
                        .xsmall()
                        .px(t.space.compact)
                        .accessibility_label("Share this with one of your groups")
                        .on_click(cx.listener(move |shell: &mut Shell, _, _, cx| {
                            let share = &mut shell.chat.share;
                            share.choosing = if share.choosing.as_deref() == Some(&toggled) {
                                None
                            } else {
                                Some(toggled.clone())
                            };
                            cx.notify();
                        })),
                );
                if choosing {
                    row.children(groups.iter().enumerate().map(|(place, group)| {
                        design::button(
                            ("share-message-to", index * 100 + place),
                            format!("🏠 {}", group.name),
                            cx,
                        )
                        .xsmall()
                        .accessibility_label(format!("Share this with {}", group.name))
                        .on_click(send(group.group))
                    }))
                } else {
                    row
                }
            }
        };
        if let Some((failed, why)) = &self.failed
            && failed == message
        {
            row = row.child(
                caption(format!("Couldn't share: {why}"), cx)
                    .id(("share-message-failed", index))
                    .aria_label(format!("Couldn't share: {why}"))
                    .test_support()
                    .text_color(cx.theme().danger),
            );
        }
        Some(row.into_any_element())
    }
}

impl Shell {
    /// Sends one chat message to `group` and waits to hear it was saved there.
    pub(crate) fn share_with_group(
        &mut self,
        group: GroupId,
        message: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let command = GroupCommand::In {
            group,
            command: Box::new(share_from_chat(text, message)),
        };
        let sent = cx
            .global::<crate::RuntimeClient>()
            .0
            .send(Command::Group(command))
            .is_ok();
        let share = &mut self.chat.share;
        share.choosing = None;
        if sent {
            share.waiting = Some(message.to_owned());
            share.failed = None;
        } else {
            share.failed = Some((
                message.to_owned(),
                "Sovra is busy. Try again in a moment.".to_owned(),
            ));
        }
        cx.notify();
    }
}
