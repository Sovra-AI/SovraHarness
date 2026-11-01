//! Connect a service by its web address.
//!
//! The view only shows what the runtime reports and sends a command. It never
//! marks a service connected on its own.

use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::Command;
use sovra_core::mcp::{McpCommand, McpPhase, McpService};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, TextRole};
use crate::model_connection_editor::field;
use crate::models_view::{ModelsView, caption, pill};
use crate::settings_view::settings_section;

impl ModelsView {
    pub(crate) fn render_service_connect_section(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let address_empty = self
            .service_address
            .read(cx)
            .value()
            .to_string()
            .trim()
            .is_empty();
        let mut section = settings_section(
            "mcp-connect-section",
            "Connect a service",
            "Paste a web address. Sign in if the service asks.",
            cx,
        )
        .child(field(
            "mcp-service-name",
            "Name",
            &self.service_name,
            "Optional. Leave this blank to use the service's own name.",
            cx,
        ))
        .child(field(
            "mcp-service-address",
            "Web address",
            &self.service_address,
            "Copy it from the service.",
            cx,
        ))
        .child(field(
            "mcp-service-secret",
            "Secret",
            &self.service_secret,
            "Only if the service gave you one. It stays on this computer.",
            cx,
        ))
        .child(
            h_flex().flex_wrap().child(
                design::primary_button("mcp-connect", "Connect", cx)
                    .disabled(address_empty)
                    .on_click(cx.listener(|view, _, window, cx| view.connect_service(window, cx))),
            ),
        );
        if let Some(notice) = &self.services.notice {
            section = section.child(caption(notice.clone(), cx));
        }
        for service in &self.services.services {
            section = section.child(self.render_connected_service(service, cx));
        }
        section
    }

    fn render_connected_service(
        &self,
        service: &McpService,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let id = service.id.clone();
        let mut card = v_flex()
            .id(SharedString::from(format!("mcp-service-{}", service.id)))
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .text_ellipsis()
                    .child(service.name.clone()),
            );
        if service.phase == McpPhase::Connected {
            card = card.child(
                div()
                    .id(SharedString::from(format!("mcp-status-{}", service.id)))
                    .test_support()
                    .aria_label("Connected")
                    .child(pill("Connected", cx)),
            );
        } else if !service.message.is_empty() {
            card = card.child(
                div()
                    .id(SharedString::from(format!("mcp-message-{}", service.id)))
                    .test_support()
                    .aria_label(service.message.clone())
                    .child(caption(service.message.clone(), cx)),
            );
        }
        for tool in &service.tools {
            card = card.child(
                div()
                    .min_w_0()
                    .id(SharedString::from(format!(
                        "mcp-tool-{}-{}",
                        service.id, tool.name
                    )))
                    .test_support()
                    .aria_label(tool.description.clone())
                    .child(caption(tool.description.clone(), cx)),
            );
        }
        if service.phase == McpPhase::NeedsSignIn {
            let sign_in = id.clone();
            card = card.child(
                design::button(
                    SharedString::from(format!("mcp-sign-in-{}", service.id)),
                    "Sign in",
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, cx| Self::sign_in_service(&sign_in, cx))),
            );
        }
        if service.phase == McpPhase::NeedsClientDetails {
            let continue_id = id.clone();
            card = card
                .child(field(
                    "mcp-client-id",
                    "Sign-in name",
                    &self.service_client_id,
                    "Paste the name the service gave you.",
                    cx,
                ))
                .child(field(
                    "mcp-client-secret",
                    "Password",
                    &self.service_client_secret,
                    "Only if the service gave you one. It stays on this computer.",
                    cx,
                ))
                .child(
                    design::primary_button(
                        SharedString::from(format!("mcp-continue-{}", service.id)),
                        "Continue",
                        cx,
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.continue_sign_in(&continue_id, window, cx);
                    })),
                );
        }
        let remove_id = id;
        card.child(
            h_flex().flex_wrap().child(
                design::button(
                    SharedString::from(format!("mcp-remove-{}", service.id)),
                    "Remove",
                    cx,
                )
                .on_click(cx.listener(move |_, _, _, cx| Self::remove_service(&remove_id, cx))),
            ),
        )
    }

    fn connect_service(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self.service_address.read(cx).value().to_string();
        if address.trim().is_empty() {
            return;
        }
        let name = self.service_name.read(cx).value().to_string();
        let secret_text = self.service_secret.read(cx).value().to_string();
        let secret = Some(secret_text.trim().to_owned()).filter(|value| !value.is_empty());
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Mcp(McpCommand::Connect {
                name,
                address,
                secret,
            }));
        self.service_secret.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }

    fn sign_in_service(id: &str, cx: &mut Context<Self>) {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Mcp(McpCommand::SignIn { id: id.to_owned() }));
    }

    fn continue_sign_in(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let client_id = self.service_client_id.read(cx).value().to_string();
        let secret_text = self.service_client_secret.read(cx).value().to_string();
        let client_secret = Some(secret_text.trim().to_owned()).filter(|value| !value.is_empty());
        let _ =
            cx.global::<RuntimeClient>()
                .0
                .send(Command::Mcp(McpCommand::SubmitClientDetails {
                    id: id.to_owned(),
                    client_id,
                    client_secret,
                }));
        self.service_client_secret.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }

    fn remove_service(id: &str, cx: &mut Context<Self>) {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Mcp(McpCommand::Remove { id: id.to_owned() }));
    }
}
