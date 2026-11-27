//! Native first-sign-in controls; credentials never enter a chat or plugin settings.

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, TextRole},
    models_view::{ModelsView, caption},
};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command,
    mcp::McpCommand,
    models::ModelSecret,
    raspberry_pi::{PairingPhase, PiSetup, validate_pairing},
};

impl ModelsView {
    pub(crate) fn start_pi_pairing(
        &mut self,
        setup: PiSetup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let password = ModelSecret(
            self.services_screen
                .pi
                .password
                .read(cx)
                .value()
                .to_string(),
        );
        if let Err(problem) = validate_pairing(&setup, &password) {
            self.services_screen.form_problem = Some(problem);
            cx.notify();
            return;
        }
        if cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Mcp(McpCommand::PairRaspberryPi {
                setup: setup.clone(),
                password,
            }))
            .is_ok()
        {
            self.services_screen.pi.pending_chat = Some(setup.clone());
            self.services_screen.pi.pairing.begin(setup);
            self.services_screen.form_problem = None;
            self.services_screen
                .pi
                .password
                .update(cx, |input, cx| input.set_value("", window, cx));
        } else {
            self.services_screen.form_problem =
                Some("The connection couldn't start. Restart Sovra and try again.".into());
        }
        cx.notify();
    }

    pub(crate) fn render_pi_pairing(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let pairing = &self.services_screen.pi.pairing;
        let (message, fingerprint) = match &pairing.phase {
            PairingPhase::Idle => ("Enter your Pi's account details to connect.", None),
            PairingPhase::Checking => ("Connecting to your Pi…", None),
            PairingPhase::ConfirmIdentity { fingerprint } => {
                ("Trust this Pi for future connections?", Some(fingerprint))
            }
            PairingPhase::Connected => ("Your Pi answered. Opening your chat…", None),
            PairingPhase::Failed(problem) => (problem.as_str(), None),
        };
        let mut panel = v_flex().w_full().min_w_0().gap(t.space.small).child(
            div()
                .id("pi-pairing-status")
                .test_support()
                .aria_label(message.to_owned())
                .whitespace_normal()
                .sovra_text(TextRole::Body, cx)
                .child(message.to_owned()),
        );
        if let Some(fingerprint) = fingerprint {
            let host = pairing
                .setup
                .as_ref()
                .map_or("your Pi", |setup| setup.host.as_str());
            panel = panel.child(caption(format!("You're connecting to {host}. Confirm this is your computer. Sovra will remember its identity and stop if it changes."), cx))
                .child(caption(format!("Connection identity: {fingerprint}"), cx));
        }
        if pairing.busy()
            && let Some(request_id) = pairing.request_id
        {
            let mut actions = h_flex().w_full().flex_wrap().gap(t.space.small);
            if fingerprint.is_some() {
                actions = actions.child(
                    design::primary_button("pi-trust-identity", "Trust and connect", cx).on_click(
                        cx.listener(move |view, _, _, cx| {
                            if cx
                                .global::<RuntimeClient>()
                                .0
                                .send(Command::Mcp(McpCommand::ConfirmPiIdentity {
                                    request_id,
                                    accept: true,
                                }))
                                .is_ok()
                            {
                                view.services_screen.pi.pairing.phase = PairingPhase::Checking;
                            } else {
                                view.services_screen.pi.pairing.phase = PairingPhase::Failed(
                                    "Sovra stopped. Restart it and try again.".into(),
                                );
                                view.services_screen.pi.pending_chat = None;
                            }
                            cx.notify();
                        }),
                    ),
                );
            }
            actions = actions.child(
                design::button("pi-cancel-pairing", "Cancel", cx)
                    .small()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let _ = cx
                            .global::<RuntimeClient>()
                            .0
                            .send(Command::Mcp(McpCommand::CancelPiPairing { request_id }));
                        view.services_screen.pi.pending_chat = None;
                        cx.notify();
                    })),
            );
            panel = panel.child(actions);
        }
        panel
    }
}
