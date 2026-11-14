//! Before the choices: what this computer has, and the text to learn from.

use super::{TrainingView, form::SourceChoice, parts, words};
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, h_flex, input::Textarea,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    TestSupportExt as _, div,
};
use sovra_core::training::{SAMPLE_TEXT, TrainingCommand};

/// How much of the built-in sample is shown before it trails off.
const SAMPLE_PREVIEW_CHARS: usize = 220;

fn known(text: &str) -> String {
    if text.trim().is_empty() {
        "Not known".to_owned()
    } else {
        text.trim().to_owned()
    }
}

impl TrainingView {
    pub(super) fn hardware_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let mut card = parts::card(self.compact(), cx).child(
            h_flex()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(t.space.small)
                .child(parts::title("This computer", cx))
                .child(
                    design::button("training-refresh", "Check again", cx)
                        .small()
                        .disabled(self.pending)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send(TrainingCommand::Load, cx);
                        })),
                ),
        );
        if let Some(hardware) = &self.state.hardware {
            let computer = match (hardware.os.trim(), hardware.arch.trim()) {
                ("", "") => "Not known".to_owned(),
                (os, "") => os.to_owned(),
                ("", arch) => arch.to_owned(),
                (os, arch) => format!("{os} · {arch}"),
            };
            let memory = if hardware.memory_total_bytes == 0 {
                "Not known".to_owned()
            } else {
                format!(
                    "{}, {} free",
                    words::gigabytes(hardware.memory_total_bytes),
                    words::gigabytes(hardware.memory_available_bytes)
                )
            };
            let mut tiles = h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_stretch()
                .gap(t.space.small)
                .child(parts::tile(
                    "training-hardware-computer",
                    "Computer",
                    computer,
                    cx,
                ))
                .child(parts::tile(
                    "training-hardware-processor",
                    "Processor",
                    known(&hardware.cpu),
                    cx,
                ))
                .child(parts::tile(
                    "training-hardware-cores",
                    "Cores",
                    words::cores(hardware.physical_cores, hardware.logical_cores),
                    cx,
                ))
                .child(parts::tile(
                    "training-hardware-memory",
                    "Memory",
                    memory,
                    cx,
                ))
                .child(parts::tile(
                    "training-hardware-engine",
                    "Training engine",
                    known(&hardware.engine_name),
                    cx,
                ));
            if !hardware.gpus.is_empty() {
                tiles = tiles.child(parts::tile(
                    "training-hardware-graphics",
                    "Graphics",
                    hardware.gpus.join(", "),
                    cx,
                ));
            }
            card = card.child(tiles).child(parts::status(
                "training-hardware-advice",
                hardware.recommendation(),
                cx,
            ));
        } else {
            card = card.child(parts::status(
                "training-hardware-advice",
                "Checking this computer…",
                cx,
            ));
        }
        card.id("training-hardware").test_support()
    }

    pub(super) fn source_card(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let locked = self.locked();
        let source = self.form.source;
        let mut card = parts::card(self.compact(), cx)
            .child(parts::title("What should it learn?", cx))
            .child(
                h_flex()
                    .min_w_0()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button("training-source-sample", "Try the sample", cx)
                            .selected(source == SourceChoice::Sample)
                            .disabled(locked)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.choose_source(SourceChoice::Sample, cx);
                            })),
                    )
                    .child(
                        design::button("training-source-text", "Use my text", cx)
                            .selected(source == SourceChoice::Text)
                            .disabled(locked)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.choose_source(SourceChoice::Text, cx);
                            })),
                    )
                    .child(
                        design::button("training-source-file", "Use a text file", cx)
                            .selected(source == SourceChoice::File)
                            .disabled(locked)
                            .on_click(cx.listener(|view, _, window, cx| {
                                // The first press goes straight to the file dialog.
                                if view.form.file.is_none() {
                                    view.choose_file(window, cx);
                                } else {
                                    view.choose_source(SourceChoice::File, cx);
                                }
                            })),
                    ),
            );
        match source {
            SourceChoice::Sample => {
                let preview: String = SAMPLE_TEXT.chars().take(SAMPLE_PREVIEW_CHARS).collect();
                card = card
                    .child(parts::body(
                        "A short piece about a garden, built in. Good for a first try.",
                        cx,
                    ))
                    .child(
                        div()
                            .id("training-sample-preview")
                            .test_support()
                            .min_w_0()
                            .p(t.space.small)
                            .whitespace_normal()
                            .sovra_surface(Surface::Subtle, cx)
                            .sovra_text(TextRole::Caption, cx)
                            .child(format!("{}…", preview.trim_end())),
                    );
            }
            SourceChoice::Text => {
                card = card
                    .child(
                        div()
                            .id("training-text")
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .child(
                                Textarea::new(&self.text)
                                    .disabled(locked)
                                    .aria_label("Text for training")
                                    .w_full(),
                            ),
                    )
                    .child(parts::caption(
                        format!(
                            "{} so far. Any language works. Use between a few sentences and 4 MB.",
                            words::file_size(self.pasted.bytes as u64)
                        ),
                        cx,
                    ));
            }
            SourceChoice::File => {}
        }
        if source == SourceChoice::File || self.file_notice.is_some() {
            let chosen = self.form.file.as_ref().and_then(|path| {
                sovra_core::training::TrainingSource::File(path.clone()).file_name()
            });
            if source == SourceChoice::File {
                card = card.child(parts::status(
                    "training-file-name",
                    match &chosen {
                        Some(name) => format!("Chosen file: {name}"),
                        None => "No file chosen yet.".to_owned(),
                    },
                    cx,
                ));
                card = card
                    .child(
                        h_flex().min_w_0().flex_wrap().child(
                            design::button(
                                "training-choose-file",
                                if chosen.is_some() {
                                    "Choose a different file"
                                } else {
                                    "Choose a file"
                                },
                                cx,
                            )
                            .disabled(locked)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.choose_file(window, cx);
                            })),
                        ),
                    )
                    .child(parts::caption(
                        "Plain text files ending in .txt or .md, up to 4 MB. The file is read when you press Start.",
                        cx,
                    ));
            }
            if let Some(notice) = &self.file_notice {
                card = card.child(parts::alert("training-file-notice", notice.clone(), cx));
            }
        }
        card.child(parts::caption(
            "Training uses only the text you choose here. Your chats, memories, folders and shared notes are never used.",
            cx,
        ))
        .id("training-source")
        .test_support()
    }
}
