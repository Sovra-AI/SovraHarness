//! Guided, per-person Microsoft registration. Only public IDs leave this form.

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, TextRole},
    model_connection_editor::field,
};
use gpui_kit::component::{
    Disableable as _,
    checkbox::Checkbox,
    h_flex,
    input::{InputEvent, InputState},
    radio::{Radio, RadioGroup},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command,
    microsoft::{
        MICROSOFT_SETUP_HELP, MICROSOFT_SETUP_URL, MicrosoftClient, MicrosoftCommand,
        MicrosoftError, MicrosoftSetupKind, RETURN_ADDRESS,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Account,
    Registration,
    ReturnAddress,
    SignIn,
}

impl Step {
    fn title(self) -> &'static str {
        match self {
            Self::Account => "Step 1 of 4 · Choose your account",
            Self::Registration => "Step 2 of 4 · Create your connection",
            Self::ReturnAddress => "Step 3 of 4 · Set the return address",
            Self::SignIn => "Step 4 of 4 · Check and sign in",
        }
    }
}

pub(crate) struct MicrosoftSetup {
    kind: MicrosoftSetupKind,
    step: Step,
    app_id: Entity<InputState>,
    tenant_id: Entity<InputState>,
    return_confirmed: bool,
    copied: Option<&'static str>,
    scroll: gpui_kit::ScrollHandle,
    _inputs: Vec<Subscription>,
}

impl MicrosoftSetup {
    pub(crate) fn new(
        scroll: gpui_kit::ScrollHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_id =
            cx.new(|cx| InputState::new(window, cx).placeholder("Paste Application (client) ID"));
        let tenant_id =
            cx.new(|cx| InputState::new(window, cx).placeholder("Paste Directory (tenant) ID"));
        let inputs = [&app_id, &tenant_id]
            .into_iter()
            .map(|input| cx.subscribe_in(input, window, |_, _, _: &InputEvent, _, cx| cx.notify()))
            .collect();
        Self {
            kind: MicrosoftSetupKind::Work,
            step: Step::Account,
            app_id,
            tenant_id,
            return_confirmed: false,
            copied: None,
            scroll,
            _inputs: inputs,
        }
    }

    pub(crate) fn edit(
        &mut self,
        client: Option<&MicrosoftClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kind = if client.is_some_and(|c| c.tenant == "consumers") {
            MicrosoftSetupKind::Personal
        } else {
            MicrosoftSetupKind::Work
        };
        self.app_id.update(cx, |s, cx| {
            s.set_value(client.map_or("", |c| c.client_id.as_str()), window, cx);
        });
        self.tenant_id.update(cx, |s, cx| {
            s.set_value(
                client
                    .filter(|c| c.tenant != "consumers")
                    .map_or("", |c| c.tenant.as_str()),
                window,
                cx,
            );
        });
        self.step = Step::Account;
        self.return_confirmed = false;
        self.copied = None;
        cx.notify();
    }

    fn read(&self, cx: &App) -> Result<MicrosoftClient, MicrosoftError> {
        MicrosoftClient::from_setup(
            self.kind,
            &self.app_id.read(cx).value(),
            &self.tenant_id.read(cx).value(),
        )
    }

    fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        self.scroll.set_offset(gpui_kit::Point::default());
        self.copied = None;
        cx.notify();
    }

    fn confirmation(&self, cx: &mut Context<Self>) -> Checkbox {
        Checkbox::new("microsoft-return-confirmed")
            .cursor_pointer()
            .label("I saved this return address")
            .checked(self.return_confirmed)
            .on_click(cx.listener(|view, checked, _, cx| {
                view.return_confirmed = *checked;
                cx.notify();
            }))
    }

    fn account(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        v_flex().gap(t.space.medium).min_w_0()
            .child("Connect your own Outlook, files, calendar, notes and to-dos. This one-time setup uses your own Microsoft app registration. You do not need a new Windows login.")
            .child(RadioGroup::new("microsoft-account-kind").gap(t.space.small)
                .child(Radio::new(0usize).label("Work or school").cursor_pointer())
                .child(Radio::new(1usize).label("Personal").cursor_pointer())
                .selected_index(Some(match self.kind { MicrosoftSetupKind::Work => 0, MicrosoftSetupKind::Personal => 1 }))
                .on_change(cx.listener(|view, index: &usize, _, cx| {
                    view.kind = if *index == 0 { MicrosoftSetupKind::Work } else { MicrosoftSetupKind::Personal };
                    cx.notify();
                })))
            .child(match self.kind {
                MicrosoftSetupKind::Work => "Use the account your workplace or school gave you. You need permission to register an app in its Microsoft Entra directory. If this is blocked, ask your IT administrator using the message below.",
                MicrosoftSetupKind::Personal => "For Outlook.com, Hotmail or Live accounts. You still need access to a Microsoft Entra directory to register the connection. A personal email account alone may not provide this. Read Microsoft's requirements or ask someone who manages your directory for help.",
            })
            .child(h_flex().flex_wrap().gap(t.space.small)
                .child(design::button("microsoft-setup-help", "Microsoft's requirements", cx)
                    .on_click(|_, _, cx| cx.open_url(MICROSOFT_SETUP_HELP)))
                .child(design::button("microsoft-copy-help", "Copy help request", cx)
                    .on_click(cx.listener(|view, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(view.kind.help_request().into()));
                        view.copied = Some("Help request copied. You can paste it into a message to your administrator or helper.");
                        cx.notify();
                    }))))
            .child(h_flex().flex_wrap().gap(t.space.small)
                .child(design::primary_button("microsoft-setup-next", "Start setup", cx)
                    .on_click(cx.listener(|view, _, _, cx| view.go(Step::Registration, cx))))
                .child(design::button("microsoft-have-ids", "I already have the IDs", cx)
                    .on_click(cx.listener(|view, _, _, cx| view.go(Step::SignIn, cx)))))
    }

    fn registration(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        v_flex().gap(t.space.medium).min_w_0()
            .child("Open Microsoft Entra. If asked, sign in with the account that can manage your directory.")
            .child(design::button("microsoft-open-setup", "Open Microsoft setup", cx).on_click(|_, _, cx| cx.open_url(MICROSOFT_SETUP_URL)))
            .child("1. In App registrations, choose New registration.")
            .child("2. Name it Sovra on my computer.")
            .child("3. Under Supported account types, choose:")
            .child(note("microsoft-account-choice", self.kind.account_choice(), cx))
            .child("4. Leave Redirect URI empty for now and choose Register. Keep that page open for the next step.")
            .child("If you cannot register apps, go Back and copy the help request. Your administrator can do these steps for you.")
            .child(design::primary_button("microsoft-setup-next", "I registered the app", cx)
                .on_click(cx.listener(|view, _, _, cx| view.go(Step::ReturnAddress, cx))))
    }

    fn return_address(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        v_flex().gap(t.space.medium).min_w_0()
            .child("In your app's Microsoft page:")
            .child("1. Choose Authentication, then Add a platform (or Add redirect URI).")
            .child("2. Choose Mobile and desktop applications.")
            .child("3. Add this exact Redirect URI, then choose Configure or Save:")
            .child(note("microsoft-return-address", RETURN_ADDRESS, cx))
            .child(design::button("microsoft-copy-return", "Copy return address", cx)
                .on_click(cx.listener(|view, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(RETURN_ADDRESS.into()));
                    view.copied = Some("Return address copied: http://localhost"); cx.notify();
                })))
            .child("Use Mobile and desktop, not Web. Do not create a client secret. Your browser returns to Sovra on this computer after sign-in.")
            .child(self.confirmation(cx))
            .child(design::primary_button("microsoft-setup-next", "Continue to IDs", cx)
                .disabled(!self.return_confirmed)
                .on_click(cx.listener(|view, _, _, cx| view.go(Step::SignIn, cx))))
    }

    fn sign_in(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = design::tokens(cx);
        let result = self.read(cx);
        let mut content = v_flex().gap(t.space.medium).min_w_0()
            .child("In Microsoft's app registration, open Overview. Copy the IDs with these exact labels. Do not use Object ID.")
            .child(field("microsoft-app-id", "Application (client) ID", &self.app_id, "A public ID, not an email address, password or client secret.", cx));
        if self.kind == MicrosoftSetupKind::Work {
            content = content.child(field(
                "microsoft-tenant-id",
                "Directory (tenant) ID",
                &self.tenant_id,
                "Copy this from the same Overview page. It is different from the Application ID.",
                cx,
            ));
        }
        content = content.child(note("microsoft-setup-review", format!("Account type: {}. Return address: http://localhost under Mobile and desktop applications.", match self.kind { MicrosoftSetupKind::Work => "work or school in your directory", MicrosoftSetupKind::Personal => "personal Microsoft account" }), cx))
            .child(self.confirmation(cx));
        if let Err(error) = &result {
            content = content.child(note("microsoft-setup-validation", error.to_string(), cx));
        } else if !self.return_confirmed {
            content = content.child(note(
                "microsoft-setup-validation",
                "Confirm the saved return address above. Use Back to see the instructions.",
                cx,
            ));
        }
        content
            .child("Sovra checks the ID format here. Microsoft checks the registration when you sign in. You will review access on Microsoft's page; your workplace may need to approve it.")
            .child("Sovra can read your content, create email drafts for you to send, add appointments without guests and add to-dos when you ask. Teams and other work services stay off until you choose them.")
            .child(design::primary_button("microsoft-save-connect", "Save and sign in", cx)
                .disabled(result.is_err() || !self.return_confirmed)
                .on_click(cx.listener(|view, _, _, cx| {
                    if view.return_confirmed && let Ok(client) = view.read(cx) {
                        let _ = cx.global::<RuntimeClient>().0.send(Command::Microsoft(MicrosoftCommand::SaveAndConnect(client)));
                    }
                })))
    }
}

fn note<T: Into<gpui_kit::SharedString>>(
    id: &'static str,
    text: T,
    cx: &App,
) -> impl IntoElement + use<T> {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .min_w_0()
        .sovra_text(TextRole::Body, cx)
        .child(text)
}

impl Render for MicrosoftSetup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = design::tokens(cx);
        let step = self.step;
        let body = match step {
            Step::Account => self.account(cx),
            Step::Registration => self.registration(cx),
            Step::ReturnAddress => self.return_address(cx),
            Step::SignIn => self.sign_in(cx),
        };
        let mut content = v_flex().w_full().min_w_0().gap(t.space.medium).child(note(
            "microsoft-setup-step",
            step.title(),
            cx,
        ));
        if step != Step::Account {
            content = content.child(design::button("microsoft-setup-back", "Back", cx).on_click(
                cx.listener(|view, _, _, cx| {
                    view.go(
                        match view.step {
                            Step::Account | Step::Registration => Step::Account,
                            Step::ReturnAddress => Step::Registration,
                            Step::SignIn => Step::ReturnAddress,
                        },
                        cx,
                    );
                }),
            ));
        }
        content = content.child(body);
        if let Some(copied) = self.copied {
            content = content.child(note("microsoft-setup-copied", copied, cx));
        }
        content
    }
}
