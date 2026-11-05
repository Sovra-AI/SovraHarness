//! Artifacts page: everything the user kept from Sovra and their team.
//!
//! The library lives in [`sovra_core::WorkspaceState::artifacts`] (the shell
//! folds `Event::Artifacts`); this view owns only what is on screen — the
//! filter, the open artifact and its edit form — and changes the library
//! through `Command::Artifacts`. Kinds come from `sovra_core::artifacts::KINDS`,
//! so a new kind needs only an icon here.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    text::TextView,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, WeakEntity, Window, div,
};
use sovra_core::artifacts::{Artifact, ArtifactCommand, ArtifactFilter, ArtifactKind, KINDS};
use sovra_core::{Command, Timestamp, format_relative_time};

use crate::RuntimeClient;
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

/// The icon for a kind's semantic key (`KindInfo::icon`).
pub(crate) fn kind_icon(kind: ArtifactKind) -> IconName {
    match kind {
        ArtifactKind::Document => IconName::FileText,
        ArtifactKind::Checklist => IconName::ListChecks,
        ArtifactKind::Table => IconName::Table,
        ArtifactKind::Code => IconName::Code,
        ArtifactKind::Diagram => IconName::Workflow,
        ArtifactKind::Webpage => IconName::Globe,
        ArtifactKind::Picture => IconName::Image,
        ArtifactKind::Data => IconName::Braces,
        ArtifactKind::Note => IconName::StickyNote,
    }
}

/// Markdown that previews the artifact: prose as is, everything else as a
/// highlighted code block so nothing is executed or rendered unsafely here.
fn preview_markdown(artifact: &Artifact) -> String {
    match artifact.kind {
        ArtifactKind::Document
        | ArtifactKind::Checklist
        | ArtifactKind::Table
        | ArtifactKind::Note => artifact.content.clone(),
        ArtifactKind::Code
        | ArtifactKind::Diagram
        | ArtifactKind::Webpage
        | ArtifactKind::Picture
        | ArtifactKind::Data => {
            let fence = if artifact.content.contains("```") {
                "~~~~"
            } else {
                "```"
            };
            format!(
                "{fence}{}\n{}\n{fence}",
                artifact.language, artifact.content
            )
        }
    }
}

fn now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Timestamp(millis)
}

pub(crate) struct ArtifactsView {
    shell: WeakEntity<Shell>,
    search: Entity<InputState>,
    kind: Option<ArtifactKind>,
    /// The artifact shown in detail; `None` shows the library.
    open: Option<String>,
    editing: bool,
    confirm_delete: bool,
    title: Entity<InputState>,
    content: Entity<TextareaState>,
    save_dialog: Option<Task<()>>,
    _search_events: Subscription,
}

impl ArtifactsView {
    pub(crate) fn new(
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search your artifacts…"));
        let search_events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            shell,
            search,
            kind: None,
            open: None,
            editing: false,
            confirm_delete: false,
            title: cx.new(|cx| InputState::new(window, cx).placeholder("Name")),
            content: cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 24)),
            save_dialog: None,
            _search_events: search_events,
        }
    }

    /// Opens one artifact (from a chat card or the library).
    pub(crate) fn show(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open = Some(id.to_owned());
        self.editing = false;
        self.confirm_delete = false;
        cx.notify();
    }

    fn library(&self, cx: &Context<Self>) -> sovra_core::artifacts::ArtifactLibrary {
        self.shell
            .upgrade()
            .map(|shell| shell.read(cx).workspace().artifacts.clone())
            .unwrap_or_default()
    }

    fn send(command: ArtifactCommand, cx: &mut Context<Self>) {
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Artifacts(command));
    }

    fn start_edit(&mut self, artifact: &Artifact, window: &mut Window, cx: &mut Context<Self>) {
        let title = artifact.title.clone();
        let content = artifact.content.clone();
        self.title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        self.content
            .update(cx, |input, cx| input.set_value(content, window, cx));
        self.editing = true;
        cx.notify();
    }

    fn save_copy(&mut self, artifact: &Artifact, cx: &mut Context<Self>) {
        let id = artifact.id.clone();
        let home = std::env::home_dir().unwrap_or_default();
        let downloads = home.join("Downloads");
        let directory = if downloads.is_dir() { downloads } else { home };
        let chosen = cx.prompt_for_new_path(&directory, Some(&artifact.file_name()));
        self.save_dialog = Some(cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(path))) = chosen.await {
                cx.update(|cx| {
                    let _ = cx.global::<RuntimeClient>().0.send(Command::Artifacts(
                        ArtifactCommand::Export {
                            id,
                            path: path.to_string_lossy().into_owned(),
                        },
                    ));
                });
            }
        }));
    }
}

impl Render for ArtifactsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = design::tokens(cx);
        let library = self.library(cx);
        let body = match self.open.as_ref().and_then(|id| library.get(id)) {
            Some(artifact) => self.detail(artifact, cx).into_any_element(),
            None => self.list(&library, cx).into_any_element(),
        };
        v_flex()
            .id("sovra-artifacts-view")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p(t.space.large)
            .child(
                v_flex()
                    .w_full()
                    .max_w(t.layout.page_width)
                    .mx_auto()
                    .gap(t.space.large)
                    .children(library.notice.clone().map(|notice| {
                        div()
                            .id("artifacts-notice")
                            .test_support()
                            .sovra_text(TextRole::Caption, cx)
                            .text_color(t.text_secondary)
                            .child(notice)
                    }))
                    .child(body),
            )
    }
}

impl ArtifactsView {
    fn list(
        &self,
        library: &sovra_core::artifacts::ArtifactLibrary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let filter = ArtifactFilter {
            kind: self.kind,
            query: self.search.read(cx).value().to_string(),
        };
        let shown: Vec<Artifact> = library.filtered(&filter).into_iter().cloned().collect();
        let now = now();
        let mut chips = h_flex()
            .id("artifact-kind-filters")
            .w_full()
            .flex_wrap()
            .gap(t.space.compact)
            .child(
                design::button(
                    "artifact-kind-all",
                    format!("All ({})", library.artifacts.len()),
                    cx,
                )
                .small()
                .selected(self.kind.is_none())
                .on_click(cx.listener(|view, _, _, cx| {
                    view.kind = None;
                    cx.notify();
                })),
            );
        for (kind, count) in library.kind_counts() {
            let index = KINDS
                .iter()
                .position(|info| info.kind == kind)
                .unwrap_or_default();
            chips = chips.child(
                design::button(
                    ("artifact-kind", index),
                    format!("{} ({count})", kind.info().plural),
                    cx,
                )
                .small()
                .icon(Icon::new(kind_icon(kind)))
                .selected(self.kind == Some(kind))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.kind = Some(kind);
                    cx.notify();
                })),
            );
        }
        let content = if library.artifacts.is_empty() {
            empty_state(cx).into_any_element()
        } else if shown.is_empty() {
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child("Nothing matches. Try a different word, or choose All.")
                .into_any_element()
        } else {
            h_flex()
                .id("artifact-grid")
                .test_support()
                .w_full()
                .flex_wrap()
                .gap(t.space.medium)
                .children(
                    shown
                        .iter()
                        .enumerate()
                        .map(|(index, artifact)| artifact_card(artifact, index, now, cx)),
                )
                .into_any_element()
        };
        v_flex()
            .w_full()
            .gap(t.space.medium)
            .child(design::page_header(
                "Artifacts",
                "Everything you and your AI made and kept — letters, lists, tables, code, diagrams and more. Open one to read it, copy it or save it to your computer.",
                cx,
            ))
            .child(
                div()
                    .w_full()
                    .max_w(t.layout.reading_width)
                    .child(Input::new(&self.search).aria_label("Search your artifacts")),
            )
            .child(chips)
            .child(content)
    }

    fn detail(&self, artifact: &Artifact, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let info = artifact.kind.info();
        let made_by = if artifact.source.author.is_empty() {
            "You".to_owned()
        } else {
            artifact.source.author.clone()
        };
        let when = format_relative_time(artifact.updated_at, now());
        let copy_text = artifact.content.clone();
        let opened = artifact.clone();
        let exported = artifact.clone();
        let edited = artifact.clone();
        let id = artifact.id.clone();
        let mut actions = h_flex()
            .id("artifact-actions")
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::button("artifact-copy", "Copy", cx)
                    .icon(Icon::new(IconName::Copy))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    }),
            )
            .child(
                design::button("artifact-save-copy", "Save to my computer", cx)
                    .icon(Icon::new(IconName::Download))
                    .on_click(cx.listener(move |view, _, _, cx| view.save_copy(&exported, cx))),
            );
        if info.opens_in_browser {
            actions = actions.child(
                design::primary_button("artifact-open-browser", "Open in browser", cx)
                    .icon(Icon::new(IconName::ExternalLink))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        Self::send(
                            ArtifactCommand::Open {
                                id: opened.id.clone(),
                            },
                            cx,
                        );
                    })),
            );
        }
        if !self.editing {
            actions = actions.child(
                design::button("artifact-edit", "Edit", cx)
                    .icon(Icon::new(IconName::Pencil))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.start_edit(&edited, window, cx);
                    })),
            );
        }
        actions = actions.child(
            design::button(
                "artifact-delete",
                if self.confirm_delete {
                    "Click again to delete"
                } else {
                    "Delete"
                },
                cx,
            )
            .ghost()
            .on_click(cx.listener(move |view, _, _, cx| {
                if view.confirm_delete {
                    Self::send(ArtifactCommand::Delete { id: id.clone() }, cx);
                    view.open = None;
                    view.confirm_delete = false;
                } else {
                    view.confirm_delete = true;
                }
                cx.notify();
            })),
        );
        let body = if self.editing {
            let id = artifact.id.clone();
            v_flex()
                .id("artifact-edit-form")
                .test_support()
                .gap(t.space.small)
                .child(Input::new(&self.title).aria_label("Artifact name"))
                .child(Textarea::new(&self.content).aria_label("Artifact content"))
                .child(
                    h_flex()
                        .gap(t.space.small)
                        .child(
                            design::primary_button("artifact-edit-save", "Save changes", cx)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    Self::send(
                                        ArtifactCommand::Edit {
                                            id: id.clone(),
                                            title: view.title.read(cx).value().to_string(),
                                            content: view.content.read(cx).value().to_string(),
                                        },
                                        cx,
                                    );
                                    view.editing = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            design::button("artifact-edit-cancel", "Cancel", cx).on_click(
                                cx.listener(|view, _, _, cx| {
                                    view.editing = false;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .id("artifact-preview")
                .test_support()
                .w_full()
                .min_w_0()
                .p(t.space.medium)
                .sovra_surface(Surface::Subtle, cx)
                .sovra_text(TextRole::Reading, cx)
                .child(
                    TextView::markdown(
                        (
                            "artifact-preview-text",
                            artifact.updated_at.millis().unsigned_abs(),
                        ),
                        preview_markdown(artifact),
                    )
                    .selectable(true)
                    .style(design::markdown_style(cx)),
                )
                .into_any_element()
        };
        v_flex()
            .id("artifact-detail")
            .test_support()
            .w_full()
            .max_w(t.chat.column_width)
            .gap(t.space.medium)
            .child(
                h_flex().child(
                    design::button("artifact-back", "← All artifacts", cx)
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open = None;
                            view.editing = false;
                            cx.notify();
                        })),
                ),
            )
            .child(
                h_flex()
                    .gap(t.space.small)
                    .items_center()
                    .min_w_0()
                    .child(Icon::new(kind_icon(artifact.kind)).text_color(cx.theme().primary))
                    .child(
                        div()
                            .min_w_0()
                            .sovra_text(TextRole::PageTitle, cx)
                            .child(artifact.title.clone()),
                    ),
            )
            .child(
                div()
                    .sovra_text(TextRole::Caption, cx)
                    .text_color(t.text_secondary)
                    .child(format!("{} · made by {made_by} · {when}", info.label)),
            )
            .child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .text_color(t.text_secondary)
                    .child(if artifact.versions.is_empty() {
                        info.explanation.to_owned()
                    } else {
                        format!(
                            "{} Edited {} time{} — earlier versions are kept.",
                            info.explanation,
                            artifact.versions.len(),
                            if artifact.versions.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        )
                    }),
            )
            .child(actions)
            .child(body)
    }
}

fn empty_state(cx: &mut Context<ArtifactsView>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("artifacts-empty")
        .test_support()
        .w_full()
        .max_w(t.layout.reading_width)
        .gap(t.space.small)
        .p(t.space.large)
        .sovra_surface(Surface::Subtle, cx)
        .child(Icon::new(IconName::LibraryBig).text_color(cx.theme().primary))
        .child(div().sovra_text(TextRole::SectionTitle, cx).child("Nothing saved yet"))
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child("When Sovra or your team writes something worth keeping, press Save under the answer. You can also ask: “save this as a document”."),
        )
}

fn artifact_card(
    artifact: &Artifact,
    index: usize,
    now: Timestamp,
    cx: &mut Context<ArtifactsView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let id = artifact.id.clone();
    let made_by = if artifact.source.author.is_empty() {
        "You".to_owned()
    } else {
        artifact.source.author.clone()
    };
    v_flex()
        .id(("artifact-card", index))
        .test_support()
        .prefer_width(t.layout.card_min_width)
        .max_w(t.layout.card_max_width)
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Glass, cx)
        .child(
            h_flex()
                .gap(t.space.small)
                .items_center()
                .min_w_0()
                .child(Icon::new(kind_icon(artifact.kind)).text_color(cx.theme().primary))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .text_ellipsis()
                        .child(artifact.title.clone()),
                ),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!(
                    "{} · {made_by} · {}",
                    artifact.kind.label(),
                    format_relative_time(artifact.updated_at, now)
                )),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .line_clamp(2)
                .child(artifact.preview()),
        )
        .child(
            h_flex().child(
                design::button(("artifact-open", index), "Open", cx)
                    .small()
                    .accessibility_label(format!("Open {}", artifact.title))
                    .on_click(cx.listener(move |view, _, _, cx| view.show(&id, cx))),
            ),
        )
}
