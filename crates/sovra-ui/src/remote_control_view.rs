//! Native phone pairing controls and dispatch using the shared observed UI primitives.
use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, TextRole},
    models_view::{ModelsView, caption},
    settings_view::{settings_page, settings_section},
    shell::Shell,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{App, Context, IntoElement, ParentElement as _, Styled as _, Window, div};
use sovra_core::{
    Command, NavTab,
    development::UiRequest,
    remote_control::{ErrorCode, RemoteCommand, RemoteFailure},
};
use std::hash::{Hash as _, Hasher as _};

#[derive(Default)]
struct RemoteInput(bool);
impl gpui_kit::Global for RemoteInput {}

pub(crate) fn is_remote_input(cx: &App) -> bool {
    cx.try_global::<RemoteInput>().is_some_and(|input| input.0)
}

impl Shell {
    pub(crate) fn open_remote_control(&mut self, cx: &mut Context<Self>) {
        self.select_tab(NavTab::Settings, cx);
        self.models().update(cx, |view, cx| {
            view.remote_only = true;
            cx.notify();
        });
    }
}

impl ModelsView {
    fn send_remote(&mut self, command: RemoteCommand, cx: &mut Context<Self>) {
        if is_remote_input(cx) {
            return;
        }
        match &command {
            RemoteCommand::Stop => {
                if let Some(generation) = self.remote_control.generation {
                    self.remote_revoked_generation = Some(generation);
                }
                self.remote_control.generation = None;
            }
            RemoteCommand::Forget { device } => {
                self.remote_forgotten_phone = Some(*device);
                self.remote_control
                    .phones
                    .retain(|phone| phone.device != *device);
            }
            RemoteCommand::Start | RemoteCommand::PairAnother | RemoteCommand::UiReply { .. } => {}
        }
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::RemoteControl(command));
        cx.notify();
    }

    pub(crate) fn render_remote_page(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        settings_page(
            v_flex()
                .w_full()
                .min_w_0()
                .max_w(t.layout.reading_width)
                .mx_auto()
                .gap(t.space.medium)
                .child(
                    design::button("remote-back-settings", "Back to Settings", cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.remote_only = false;
                            cx.notify();
                        }),
                    ),
                )
                .child(self.render_remote_section(cx))
                .into_any_element(),
            cx,
        )
    }

    pub(crate) fn render_remote_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let enabled = self.remote_control.generation.is_some();
        let mut card = settings_section("remote-control-section", "Use Sovra from your phone",
            "Pair your phone once. While this computer is awake and Sovra is open, your paired phone can continue chats and use this window.", cx)
            .child(caption("The phone app is not available yet. This connection is ready for app developers to integrate.", cx))
            .child(caption(if enabled { "On · open Sovra on your paired phone" } else { "Off · phones cannot control Sovra" }, cx))
            .child(h_flex().flex_wrap().gap(t.space.small)
                .child(design::button("remote-control-toggle", if enabled { "Stop phone control" } else { "Start phone control" }, cx)
                    .on_click(cx.listener(move |view, _, _, cx| view.send_remote(if enabled { RemoteCommand::Stop } else { RemoteCommand::Start }, cx))))
                .children(enabled.then(|| design::button("remote-control-pair", "Pair another phone", cx)
                    .on_click(cx.listener(|view, _, _, cx| view.send_remote(RemoteCommand::PairAnother, cx))))));
        if self.remote_control.invitation.is_some() {
            card = card.child(caption(
                "Control from other phones pauses while this pairing code is available.",
                cx,
            ));
            card = card.child(caption("Scan this code in the Sovra phone app. It expires after five minutes and can be used once.", cx));
            if !self.remote_control.qr.is_empty() {
                card = card.child(crate::connections_view::scan_code(
                    "remote-control-qr",
                    self.remote_control.qr.clone(),
                    cx,
                ));
            }
            card = card.child(
                design::button("remote-control-copy", "Copy pairing link", cx).on_click(
                    cx.listener(|view, _, _, cx| {
                        if is_remote_input(cx) {
                            return;
                        }
                        if let Some(ticket) = &view.remote_control.invitation
                            && let Ok(link) = ticket.encode()
                        {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(link));
                        }
                    }),
                ),
            );
        } else if enabled && self.remote_control.phones.is_empty() {
            card = card.child(caption(
                "Choose Pair another phone to create a new code.",
                cx,
            ));
        }
        for phone in &self.remote_control.phones {
            let id = phone.device;
            let connected = self.remote_control.connected_phone == Some(id);
            card = card.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .sovra_text(TextRole::Body, cx)
                            .child(format!(
                                "{} · {}",
                                phone.name,
                                if connected { "Connected" } else { "Paired" }
                            )),
                    )
                    .child(
                        design::button(
                            gpui_kit::SharedString::from(format!(
                                "remote-control-forget-{}",
                                id.to_hex()
                            )),
                            "Forget",
                            cx,
                        )
                        .accessibility_label(format!("Forget {}", phone.name))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send_remote(RemoteCommand::Forget { device: id }, cx);
                        })),
                    ),
            );
        }
        card.children(
            self.remote_control
                .error
                .as_ref()
                .map(|error| caption(error.clone(), cx).text_color(cx.theme().danger)),
        )
    }
}

pub(crate) fn failure(code: ErrorCode, message: &str) -> RemoteFailure {
    RemoteFailure {
        code,
        message: message.into(),
    }
}

/// Snapshot and focus are compared on the UI thread immediately before mutation.
/// Endpoint/session-specific target prefixes prevent reusing targets after reconnect.
pub(crate) fn execute(
    request: UiRequest,
    prefix: &str,
    revision: Option<&str>,
    viewport: gpui_kit::Bounds<gpui_kit::Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Result<crate::development_view::Output, RemoteFailure> {
    use crate::development_view::{Output, execute as native};
    let Output::Data(mut snapshot) = native(UiRequest::Inspect {}, prefix, viewport, window, cx)
        .map_err(|error| failure(ErrorCode::InvalidRequest, &error))?
    else {
        return Err(failure(
            ErrorCode::InvalidRequest,
            "Could not inspect this window.",
        ));
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    snapshot.to_string().hash(&mut hash);
    format!("{:?}", window.focused(cx)).hash(&mut hash);
    let current = format!("{:016x}", hash.finish());
    match &request {
        UiRequest::Inspect {} => {
            snapshot["revision"] = current.into();
            return Ok(Output::Data(snapshot));
        }
        UiRequest::Screenshot {} => {}
        UiRequest::Click { .. }
        | UiRequest::Type { .. }
        | UiRequest::Key { .. }
        | UiRequest::Scroll { .. } => {
            if revision != Some(current.as_str()) {
                return Err(failure(
                    ErrorCode::StaleObservation,
                    "The window changed. Inspect again before acting.",
                ));
            }
        }
    }
    cx.set_global(RemoteInput(true));
    let output = native(request, prefix, viewport, window, cx);
    cx.set_global(RemoteInput(false));
    output.map_err(|error| failure(ErrorCode::OutcomeUnknown, &error))
}
