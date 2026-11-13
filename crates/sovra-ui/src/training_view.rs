//! Guided native training controls; progress arrives only through runtime events.
use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Role, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command,
    training::{
        HelpingPhase, TrainingCommand, TrainingEvent, TrainingPhase, TrainingSource, TrainingState,
    },
};

pub(crate) struct TrainingView {
    state: TrainingState,
    text: Entity<TextareaState>,
    sample: bool,
    pending: bool,
}

impl TrainingView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: TrainingState::default(),
            text: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Paste text you want the model to learn from (up to 64 KB).")
                    .auto_grow(3, 8)
            }),
            sample: true,
            pending: false,
        }
    }

    pub(crate) fn apply(&mut self, event: &TrainingEvent, cx: &mut Context<Self>) {
        self.state.apply(event);
        self.pending = false;
        cx.notify();
    }

    fn send(&mut self, command: TrainingCommand, cx: &mut Context<Self>) {
        match cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Training(command))
        {
            Ok(()) => {
                self.pending = true;
                self.state.error = None;
            }
            Err(error) => self.state.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn source(&self, cx: &Context<Self>) -> TrainingSource {
        if self.sample {
            TrainingSource::Sample
        } else {
            TrainingSource::Text(self.text.read(cx).value().to_string())
        }
    }

    fn grid_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let busy = self.pending || self.state.busy();
        let unavailable = self.state.phase == TrainingPhase::Unavailable;
        let mut card = v_flex().w_full().min_w_0().gap(t.space.medium).p(t.space.large)
            .sovra_surface(Surface::Matte, cx)
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("Help another computer"))
            .child(div().min_w_0().whitespace_normal().sovra_text(TextRole::Body, cx)
                .child("Choose a computer in your group. It must have started with Start with my group. This uses its chosen text and sends back model updates. Your chats, memories and local files are not added. Helping waits while chat is busy."))
            .child(design::button("training-refresh-peers", "Refresh computers", cx).disabled(self.pending || unavailable)
                .on_click(cx.listener(|view, _, _, cx| view.send(TrainingCommand::Load, cx))));
        if let Some(help) = &self.state.helping {
            let phase = match help.phase {
                HelpingPhase::Working => "Helping",
                HelpingPhase::Waiting => "Waiting to help",
                HelpingPhase::Stopping => "Stopping after the current exchange",
                HelpingPhase::Stopped => "Stopped helping",
                HelpingPhase::Finished => "Training finished",
            };
            let status = format!("{phase} · {} saved updates", help.accepted_rounds);
            card = card.child(
                div()
                    .id("training-helper-status")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(status),
            );
            card = card.child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(help.peer.name.clone()),
            );
            if let Some(notice) = &help.notice {
                card = card.child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .child(notice.clone()),
                );
            }
            card = card.child(
                design::button("training-stop-help", "Stop helping", cx)
                    .disabled(
                        self.pending
                            || !help.phase.active()
                            || help.phase == HelpingPhase::Stopping,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.send(TrainingCommand::Pause, cx))),
            );
        }
        if let Some(group) = &self.state.group {
            card = card.child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(format!("Your group: {}", group.name)),
            );
            if group.peers.is_empty() {
                card = card.child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child("Add another computer in Groups, then refresh this list."),
                );
            }
            for peer in &group.peers {
                let device = peer.device;
                card = card.child(
                    h_flex()
                        .min_w_0()
                        .flex_wrap()
                        .gap(t.space.small)
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .whitespace_normal()
                                .sovra_text(TextRole::Body, cx)
                                .child(peer.name.clone()),
                        )
                        .child(
                            design::button(
                                format!("training-help-{}", device.to_hex()),
                                "Help this computer",
                                cx,
                            )
                            .accessibility_label(format!("Help {}", peer.name))
                            .disabled(busy || unavailable)
                            .on_click(cx.listener(
                                move |view, _, _, cx| view.send(TrainingCommand::Help(device), cx),
                            )),
                        ),
                );
            }
        } else {
            card = card.child(div().min_w_0().whitespace_normal().sovra_text(TextRole::Body, cx).child("Open Groups in the left menu to create or join a group, then return here and refresh the list."));
        }
        card
    }
}

impl Render for TrainingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = design::tokens(cx);
        let busy = self.pending || self.state.busy();
        let unavailable = self.state.phase == TrainingPhase::Unavailable;
        let phase = match self.state.phase {
            TrainingPhase::Loading => "Opening saved training…",
            TrainingPhase::Ready => "Ready to start",
            TrainingPhase::Preparing => "Preparing and saving…",
            TrainingPhase::Running => "Training on this computer",
            TrainingPhase::Pausing => "Finishing this short step…",
            TrainingPhase::Paused => "Paused · your progress is saved",
            TrainingPhase::Complete => "Finished · your model is saved",
            TrainingPhase::Unavailable => "Training is unavailable in this session",
        };
        let progress = &self.state.progress;
        let status = if progress.total_rounds > 0 {
            format!(
                "{phase} · {} of {} rounds saved · {} tokens learned from",
                progress.rounds, progress.total_rounds, progress.tokens
            )
        } else {
            phase.to_owned()
        };
        let mut card = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.medium)
            .p(t.space.large)
            .sovra_surface(Surface::Matte, cx)
            .child(
                div()
                    .sovra_text(TextRole::SectionTitle, cx)
                    .child("1. Choose text for a new run"),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button("training-sample", "Try the garden sample", cx)
                            .selected(self.sample)
                            .disabled(busy || unavailable)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.sample = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        design::button("training-own-text", "Use my text", cx)
                            .selected(!self.sample)
                            .disabled(busy || unavailable)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.sample = false;
                                cx.notify();
                            })),
                    ),
            );
        if self.sample {
            card = card.child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .child(sovra_core::training::SAMPLE_TEXT),
            );
        } else {
            card = card.child(
                Textarea::new(&self.text)
                    .disabled(busy || unavailable)
                    .aria_label("Text for training"),
            );
        }
        card = card.child(div().min_w_0().whitespace_normal().sovra_text(TextRole::Caption, cx)
            .child("New training uses only the text you choose here. It stays on this computer, including in the saved training file. Your chats, memories and shared group notes are not added. Helping the group train is a separate choice in Groups and starts off. Answering questions or lending an AI does not turn it on. Resume uses the text already saved with that run."))
            .child(div().sovra_text(TextRole::SectionTitle, cx).child("2. Start when you're ready"))
            .child(h_flex().flex_wrap().gap(t.space.small)
                .child(design::button("training-start", if self.state.saved_at.is_some() { "Start new training" } else { "Start training" }, cx).primary().disabled(busy || unavailable)
                    .on_click(cx.listener(|v, _, _, cx| {
                        let source = v.source(cx);
                        v.send(TrainingCommand::Start(source), cx);
                    })))
                .child(design::button("training-start-shared", "Start with my group", cx).disabled(busy || unavailable || self.state.group.is_none())
                    .on_click(cx.listener(|v, _, _, cx| { let source = v.source(cx); v.send(TrainingCommand::StartShared(source), cx); })))
                .child(design::button("training-pause", "Pause", cx).disabled(self.pending || self.state.phase != TrainingPhase::Running)
                    .on_click(cx.listener(|v, _, _, cx| v.send(TrainingCommand::Pause, cx))))
                .child(design::button("training-resume", "Resume saved training", cx).disabled(busy || self.state.phase != TrainingPhase::Paused)
                    .on_click(cx.listener(|v, _, _, cx| v.send(TrainingCommand::Resume, cx)))))
            .child(div().id("training-status").test_support().role(Role::Status).aria_label(status.clone())
                .min_w_0().whitespace_normal().sovra_text(TextRole::Body, cx).child(status))
            .child(div().min_w_0().whitespace_normal().sovra_text(TextRole::Caption, cx)
                .child("Progress saves after each short round. Closing Sovra pauses training. It waits while chat is busy. Starting again keeps the earlier model file."));
        if self.state.sharing.is_some() {
            let shared = format!(
                "Your group can help while this run is active · {} rounds saved from other computers in this session",
                self.state.helped_rounds
            );
            card = card
                .child(
                    div()
                        .id("training-sharing-status")
                        .test_support()
                        .role(Role::Status)
                        .aria_label(shared.clone())
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Body, cx)
                        .child(shared),
                )
                .child(
                    design::button("training-stop-sharing", "Stop sharing", cx)
                        .disabled(self.pending)
                        .on_click(
                            cx.listener(|v, _, _, cx| v.send(TrainingCommand::StopSharing, cx)),
                        ),
                );
        }
        if let Some(loss) = self.state.progress.loss_milli {
            card = card.child(div().min_w_0().whitespace_normal().sovra_text(TextRole::Caption, cx)
                .child(format!("Latest learning loss: {}.{:03}. Lower means fewer prediction errors on the current text batch; it is not a chat-quality score.", loss / 1000, loss % 1000)));
        }
        if let Some(path) = &self.state.saved_at {
            let path = path.clone();
            card = card
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .sovra_text(TextRole::Caption, cx)
                        .truncate()
                        .child(format!("Saved model and training state: {path}")),
                )
                .child(
                    design::button("training-copy-path", "Copy saved file location", cx).on_click(
                        move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                path.clone(),
                            ));
                        },
                    ),
                );
        }
        if let Some(error) = &self.state.error {
            card = card.child(
                div()
                    .id("training-error")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .min_w_0()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        div().id("training-page").test_support().size_full().min_w_0().min_h_0().overflow_y_scroll()
            .child(v_flex().w_full().items_center().p(t.space.large)
                .child(v_flex().w_full().min_w_0().max_w(t.layout.reading_width).gap(t.space.large)
                    .child(design::page_header("Training", "Teach a tiny practice model with text you choose.", cx))
                    .child(div().min_w_0().whitespace_normal().sovra_text(TextRole::Body, cx)
                        .child("Early preview · Train a small next-byte model on this computer or with your group. It demonstrates training and recovery; it does not replace your chat AI. Larger language models are still being built."))
                    .child(card)
                    .child(self.grid_card(cx))))
    }
}
