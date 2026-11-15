//! One Microsoft connection with service choices disclosed after sign-in.

use gpui_kit::component::{Disableable as _, h_flex, switch::Switch, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, WeakEntity, Window,
    div,
};
use sovra_core::Command;
use sovra_core::microsoft::{MicrosoftCommand, MicrosoftState, SERVICES};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, TextRole};
use crate::shell::Shell;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirmation {
    Disconnect,
    Forget,
}

pub(crate) struct MicrosoftCard {
    shell: WeakEntity<Shell>,
    show_services: bool,
    confirmation: Option<Confirmation>,
    editing: bool,
    setup: Entity<crate::microsoft_setup::MicrosoftSetup>,
}

impl MicrosoftCard {
    pub(crate) fn new(
        shell: WeakEntity<Shell>,
        scroll: gpui_kit::ScrollHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let setup = cx.new(|cx| crate::microsoft_setup::MicrosoftSetup::new(scroll, window, cx));
        Self {
            shell,
            show_services: false,
            confirmation: None,
            editing: false,
            setup,
        }
    }
}

fn send(command: MicrosoftCommand, cx: &mut Context<MicrosoftCard>) {
    let _ = cx
        .global::<RuntimeClient>()
        .0
        .send(Command::Microsoft(command));
}

impl Render for MicrosoftCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connection = self
            .shell
            .upgrade()
            .map(|shell| shell.read(cx).workspace().microsoft.clone())
            .unwrap_or_default();
        let t = design::tokens(cx);
        let mut content = v_flex()
            .w_full()
            .min_w_0()
            .max_w(t.layout.reading_width)
            .gap(t.space.medium)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("Microsoft 365"),
            );
        if !matches!(
            connection.state,
            MicrosoftState::Connected(_) | MicrosoftState::NeedsSignIn(_)
        ) && self.confirmation == Some(Confirmation::Disconnect)
        {
            self.confirmation = None;
        }
        match &connection.state {
            MicrosoftState::Loading => {
                content = content.child("Checking your Microsoft connection…");
            }
            MicrosoftState::NeedsSetup => {
                self.editing = false;
                content = content.child(self.setup.clone());
            }
            MicrosoftState::NotConnected => {
                if self.editing {
                    content = content.child(self.setup.clone()).child(
                        design::button("microsoft-edit-cancel", "Keep saved setup", cx).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.editing = false;
                                cx.notify();
                            }),
                        ),
                    );
                } else {
                    content = content.child("Your own Microsoft setup is saved on this computer. Sign in to choose your account and approve access.")
                        .child(design::primary_button("microsoft-connect", "Connect Microsoft 365", cx)
                            .on_click(cx.listener(|_, _, _, cx| send(MicrosoftCommand::Connect, cx))))
                        .child(design::button("microsoft-edit-setup", "Edit setup", cx)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.setup.update(cx, |setup, cx| setup.edit(connection.client.as_ref(), window, cx));
                                view.editing = true; view.confirmation = None; cx.notify();
                            })));
                    if self.confirmation == Some(Confirmation::Forget) {
                        content = content.child("Forget the saved IDs on this computer? You will need the setup steps again. This does not delete the registration at Microsoft.")
                            .child(h_flex().flex_wrap().gap(t.space.small)
                                .child(design::button("microsoft-forget-confirm", "Forget setup", cx)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.confirmation = None;
                                        view.setup.update(cx, |setup, cx| setup.edit(None, window, cx));
                                        send(MicrosoftCommand::ForgetSetup, cx); cx.notify();
                                    })))
                                .child(design::button("microsoft-forget-cancel", "Keep setup", cx)
                                    .on_click(cx.listener(|view, _, _, cx| { view.confirmation = None; cx.notify(); }))));
                    } else {
                        content = content.child(
                            design::button("microsoft-forget", "Forget setup", cx).on_click(
                                cx.listener(|view, _, _, cx| {
                                    view.confirmation = Some(Confirmation::Forget);
                                    cx.notify();
                                }),
                            ),
                        );
                    }
                }
            }
            MicrosoftState::Connecting { url } => {
                self.editing = false;
                self.confirmation = None;
                let url = url.clone();
                let disabled = url.is_empty();
                content = content
                    .child("Finish signing in on Microsoft's page, then return here.")
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(t.space.small)
                            .child(
                                design::button(
                                    "microsoft-open-browser",
                                    "Open Microsoft again",
                                    cx,
                                )
                                .disabled(disabled)
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                            )
                            .child(design::button("microsoft-cancel", "Cancel", cx).on_click(
                                cx.listener(|_, _, _, cx| send(MicrosoftCommand::Cancel, cx)),
                            )),
                    );
            }
            MicrosoftState::Connected(account) | MicrosoftState::NeedsSignIn(account) => {
                let connected = matches!(connection.state, MicrosoftState::Connected(_));
                content = content
                    .child(
                        div()
                            .sovra_text(TextRole::Body, cx)
                            .child(format!("{} · {}", account.name, account.email)),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(connection.state.status_label()),
                    );
                if connected {
                    content = content.child(
                        design::button(
                            "microsoft-services",
                            if self.show_services {
                                "Hide service choices"
                            } else {
                                "Choose what Sovra can use"
                            },
                            cx,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.show_services = !view.show_services;
                            cx.notify();
                        })),
                    );
                    if self.show_services {
                        for (index, info) in SERVICES.iter().enumerate() {
                            let service = info.service;
                            content = content.child(
                                v_flex()
                                    .min_w_0()
                                    .gap(t.space.small)
                                    .child(
                                        Switch::new(("microsoft-service", index))
                                            .label(info.label)
                                            .checked(account.active(service))
                                            .on_click(cx.listener(move |_, on: &bool, _, cx| {
                                                send(
                                                    MicrosoftCommand::SetService {
                                                        service,
                                                        on: *on,
                                                    },
                                                    cx,
                                                );
                                            })),
                                    )
                                    .child(
                                        div()
                                            .sovra_text(TextRole::Caption, cx)
                                            .child(info.explanation),
                                    ),
                            );
                        }
                    }
                } else {
                    content = content.child(
                        design::primary_button("microsoft-reconnect", "Sign in again", cx)
                            .on_click(
                                cx.listener(|_, _, _, cx| send(MicrosoftCommand::Connect, cx)),
                            ),
                    );
                }
                if self.confirmation == Some(Confirmation::Disconnect) {
                    content = content.child("Disconnect Microsoft 365 from this computer? Sovra will forget its saved access. Your Microsoft files and messages stay where they are.")
                        .child(h_flex().flex_wrap().gap(t.space.small)
                            .child(design::button("microsoft-disconnect-confirm", "Disconnect", cx)
                                .on_click(cx.listener(|view, _, _, cx| { view.confirmation = None; send(MicrosoftCommand::Disconnect, cx); cx.notify(); })))
                            .child(design::button("microsoft-disconnect-cancel", "Keep connected", cx)
                                .on_click(cx.listener(|view, _, _, cx| { view.confirmation = None; cx.notify(); }))));
                } else {
                    content = content.child(
                        design::button("microsoft-disconnect", "Disconnect Microsoft 365", cx)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.confirmation = Some(Confirmation::Disconnect);
                                cx.notify();
                            })),
                    );
                }
            }
        }
        if let Some(notice) = connection.notice {
            content = content.child(
                div()
                    .id("microsoft-notice")
                    .test_support()
                    .aria_label(notice.clone())
                    .sovra_text(TextRole::Body, cx)
                    .child(notice),
            );
        }
        content
    }
}
