//! Artifacts page: everything the user kept from Sovra and their team.
//!
//! The library lives in [`sovra_core::WorkspaceState::artifacts`] (the shell
//! folds `Event::Artifacts`); this view owns only what is on screen — the
//! filter, the open artifact and its edit form — and changes the library
//! through `Command::Artifacts`. Kinds come from `sovra_core::artifacts::KINDS`,
//! so a new kind needs only an icon here.
//!
//! When this computer is in a group, the page also lists what the group shared
//! (read from the shell's `GroupsView`), offers **Share with my group** on saved
//! artifacts and **Keep a copy** on shared ones. Which saved artifact belongs to
//! which shared one is decided by `sovra_core::group::sharing_state`.

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
use sovra_core::artifacts::{
    Artifact, ArtifactCommand, ArtifactDraft, ArtifactFilter, ArtifactKind, ArtifactSource, KINDS,
};
use sovra_core::group::{
    DetailStatus, GroupCommand, OpHash, SharedArtifact, SharingState, check_artifact, lineage_ids,
    sharing_state,
};
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
fn preview_markdown(kind: ArtifactKind, language: &str, content: &str) -> String {
    match kind {
        ArtifactKind::Document
        | ArtifactKind::Checklist
        | ArtifactKind::Table
        | ArtifactKind::Note => content.to_owned(),
        ArtifactKind::Code
        | ArtifactKind::Diagram
        | ArtifactKind::Webpage
        | ArtifactKind::Picture
        | ArtifactKind::Data => {
            let fence = if content.contains("```") {
                "~~~~"
            } else {
                "```"
            };
            format!("{fence}{language}\n{content}\n{fence}")
        }
    }
}

/// The first words of a shared artifact, on one line.
fn snippet(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(160).collect()
}

/// A shared artifact as a card shows it, without its (possibly large) content.
struct SharedCard {
    id: OpHash,
    title: String,
    kind: ArtifactKind,
    shared_by: String,
    shared_at: Timestamp,
    snippet: String,
    two_versions: bool,
    /// "Just you and Dad" for a two-person space.
    space_label: Option<String>,
}

/// What the page needs from this computer's group, copied small each render.
struct GroupGlance {
    name: String,
    /// This person may add to the group (not a Viewer).
    can_share: bool,
    /// Everyone else, for "Just Dad".
    others: Vec<(sovra_core::group::PersonId, String)>,
    /// The latest version of each shared artifact.
    cards: Vec<SharedCard>,
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
    /// The shared group artifact shown in detail, instead of a saved one.
    open_shared: Option<OpHash>,
    /// A shared artifact waiting for "Click again to stop sharing".
    confirm_withdraw: bool,
    /// Why the last "Share with my group" could not go out, in plain words.
    share_error: Option<String>,
    /// Shown after Share or Keep a copy, until another artifact opens.
    share_notice: Option<String>,
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
            open_shared: None,
            confirm_withdraw: false,
            share_error: None,
            share_notice: None,
            _search_events: search_events,
        }
    }

    /// Opens one artifact (from a chat card or the library).
    pub(crate) fn show(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open = Some(id.to_owned());
        self.open_shared = None;
        self.editing = false;
        self.confirm_delete = false;
        self.share_error = None;
        self.share_notice = None;
        cx.notify();
    }

    fn show_shared(&mut self, id: OpHash, cx: &mut Context<Self>) {
        self.open = None;
        self.open_shared = Some(id);
        self.editing = false;
        self.confirm_withdraw = false;
        self.share_error = None;
        self.share_notice = None;
        cx.notify();
    }

    fn back_to_library(&mut self, cx: &mut Context<Self>) {
        self.open = None;
        self.open_shared = None;
        self.editing = false;
        self.share_error = None;
        self.share_notice = None;
        cx.notify();
    }

    /// The group this computer is in, if any, with its latest shared artifacts.
    fn group_glance(&self, cx: &Context<Self>) -> Option<GroupGlance> {
        let shell = self.shell.upgrade()?;
        let groups = shell.read(cx).groups(cx);
        let group = groups.group.as_ref()?;
        let cards = groups
            .artifacts
            .iter()
            .filter(|artifact| artifact.is_latest())
            .map(|artifact| SharedCard {
                id: artifact.id,
                title: artifact.title.clone(),
                kind: artifact.kind,
                shared_by: artifact.shared_by.clone(),
                shared_at: artifact.shared_at,
                snippet: snippet(&artifact.content),
                two_versions: matches!(artifact.status, DetailStatus::Disputed { .. }),
                space_label: groups.space_label(&artifact.space),
            })
            .collect();
        Some(GroupGlance {
            name: group.name.clone(),
            can_share: groups
                .my_role()
                .is_some_and(sovra_core::group::Role::writes),
            others: groups.others(),
            cards,
        })
    }

    /// How a saved artifact stands with the group (`None` outside a group).
    fn sharing(&self, artifact: &Artifact, cx: &Context<Self>) -> Option<SharingState> {
        let shell = self.shell.upgrade()?;
        let groups = shell.read(cx).groups(cx);
        groups.group.as_ref()?;
        let me = groups.this_computer?;
        Some(sharing_state(
            &artifact.id,
            &artifact.source.shared_from,
            &artifact.title,
            &artifact.content,
            &me,
            &groups.artifacts,
        ))
    }

    /// One shared artifact, and whether this computer's person may stop sharing it.
    fn shared(&self, id: &OpHash, cx: &Context<Self>) -> Option<(SharedArtifact, bool)> {
        let shell = self.shell.upgrade()?;
        let groups = shell.read(cx).groups(cx);
        let artifact = groups
            .artifacts
            .iter()
            .find(|artifact| artifact.id == *id)?;
        let me = groups.this_computer?;
        let my_person = groups
            .group
            .as_ref()
            .and_then(|group| group.owner_of(&me))
            .map(|member| member.person);
        let mine = my_person.is_some() && artifact.person == my_person;
        Some((artifact.clone(), mine || groups.can_manage()))
    }

    /// A saved copy of any version of the shared artifact `id`, if this computer has one.
    fn my_copy_of(&self, id: OpHash, cx: &Context<Self>) -> Option<String> {
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        let versions: Vec<String> = lineage_ids(id, &shell.groups(cx).artifacts)
            .iter()
            .map(OpHash::to_hex)
            .collect();

        shell
            .workspace()
            .artifacts
            .artifacts
            .iter()
            .find(|artifact| versions.contains(&artifact.source.shared_from))
            .map(|artifact| artifact.id.clone())
    }

    fn share_with_group(
        &mut self,
        artifact: &Artifact,
        version_of: Option<OpHash>,
        with: Option<&(sovra_core::group::PersonId, String)>,
        group: &str,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = check_artifact(&artifact.title, &artifact.language, &artifact.content) {
            self.share_error = Some(format!("This can't be shared: {error}."));
            self.share_notice = None;
            cx.notify();
            return;
        }
        let _ = cx
            .global::<RuntimeClient>()
            .0
            .send(Command::Group(GroupCommand::ShareArtifact {
                title: artifact.title.clone(),
                kind: artifact.kind,
                language: artifact.language.clone(),
                content: artifact.content.clone(),
                new_version_of: version_of,
                local_id: artifact.id.clone(),
                with: with.map(|(person, _)| *person),
            }));
        self.share_error = None;
        self.share_notice = Some(match with {
            Some((_, name)) => {
                format!("Shared with just {name}. Nobody else in {group} can see it.")
            }
            None => format!(
                "Shared with {group}. Everyone in the group gets it the next time their computer is on."
            ),
        });
        cx.notify();
    }

    fn keep_copy(&mut self, artifact: &SharedArtifact, cx: &mut Context<Self>) {
        Self::send(
            ArtifactCommand::Save(ArtifactDraft {
                title: artifact.title.clone(),
                kind: artifact.kind,
                language: artifact.language.clone(),
                content: artifact.content.clone(),
                source: ArtifactSource {
                    session_id: String::new(),
                    message_id: String::new(),
                    author: artifact.shared_by.clone(),
                    shared_from: artifact.id.to_hex(),
                },
            }),
            cx,
        );
        self.share_notice = Some(
            "Kept a copy in your artifacts. Edit it, then share your changes back with the group."
                .into(),
        );
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
        let shared = self.open_shared.and_then(|id| self.shared(&id, cx));
        let body = match (self.open.as_ref().and_then(|id| library.get(id)), shared) {
            (Some(artifact), _) => self.detail(artifact, cx).into_any_element(),
            (None, Some((artifact, may_withdraw))) => self
                .shared_detail(&artifact, may_withdraw, cx)
                .into_any_element(),
            (None, None) => self.list(&library, cx).into_any_element(),
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
        let shared_section = self
            .group_glance(cx)
            .filter(|glance| !glance.cards.is_empty())
            .map(|glance| shared_section(&glance, now, cx));
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
            .children(shared_section)
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
        let group = self.group_glance(cx).filter(|glance| glance.can_share);
        let sharing = group.as_ref().and_then(|_| self.sharing(artifact, cx));
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
        let mut share_status = None;
        if let (Some(group), Some(sharing)) = (group, sharing) {
            let label = match &sharing {
                SharingState::NotShared => Some(format!("Share with {}", group.name)),
                SharingState::Changed { .. } => {
                    Some(format!("Share your changes with {}", group.name))
                }
                SharingState::UpToDate => {
                    share_status = Some(format!(
                        "Shared with {}. Everyone has this version.",
                        group.name
                    ));
                    None
                }
            };
            if let Some(label) = label {
                let version_of = match sharing {
                    SharingState::Changed { version_of } => Some(version_of),
                    SharingState::NotShared | SharingState::UpToDate => None,
                };
                let shared = artifact.clone();
                let name = group.name.clone();
                actions = actions.child(
                    design::button("artifact-share-group", label, cx)
                        .icon(Icon::new(IconName::Users))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.share_with_group(&shared, version_of, None, &name, cx);
                        })),
                );
                // A brand-new share may go to just one person instead.
                if version_of.is_none() {
                    for (index, other) in group.others.iter().take(4).enumerate() {
                        let shared = artifact.clone();
                        let name = group.name.clone();
                        let other = other.clone();
                        actions = actions.child(
                            design::button(
                                ("artifact-share-just", index),
                                format!("Just {}", other.1),
                                cx,
                            )
                            .ghost()
                            .accessibility_label(format!("Share only with {}", other.1))
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.share_with_group(
                                        &shared,
                                        None,
                                        Some(&other),
                                        &name,
                                        cx,
                                    );
                                },
                            )),
                        );
                    }
                }
            }
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
                        preview_markdown(artifact.kind, &artifact.language, &artifact.content),
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
                        .on_click(cx.listener(|view, _, _, cx| view.back_to_library(cx))),
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
            .children(self.share_feedback(share_status, cx))
            .child(body)
    }

    /// The line under the actions: an error, what just happened, or where it stands.
    fn share_feedback(
        &self,
        status: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let t = design::tokens(cx);
        let (text, colour) = match (&self.share_error, &self.share_notice, status) {
            (Some(error), _, _) => (error.clone(), cx.theme().danger),
            (None, Some(notice), _) => (notice.clone(), t.text_secondary),
            (None, None, Some(status)) => (status, t.text_secondary),
            (None, None, None) => return None,
        };
        Some(
            div()
                .id("artifact-share-feedback")
                .test_support()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(colour)
                .child(text),
        )
    }

    /// Someone's shared artifact: read it, keep a copy, or stop sharing it.
    fn shared_detail(
        &self,
        artifact: &SharedArtifact,
        may_withdraw: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = design::tokens(cx);
        let info = artifact.kind.info();
        let when = format_relative_time(artifact.shared_at, now());
        let from = if artifact.computer.is_empty() {
            String::new()
        } else {
            format!(" from {}", artifact.computer)
        };
        let note = match artifact.status {
            DetailStatus::Disputed { .. } => Some(
                "Two people changed this at the same time, so both versions are kept. Keep a copy of the one you want, or share one that combines them.",
            ),
            DetailStatus::Replaced { .. } => Some("There's a newer version of this in the group."),
            DetailStatus::Withdrawn => {
                Some("This was taken back. Helpers and the group list no longer show it.")
            }
            DetailStatus::Redacted => Some("Its words were erased on every computer."),
            DetailStatus::Current => None,
        };
        let copy_text = artifact.content.clone();
        let kept = artifact.clone();
        let id = artifact.id;
        let my_copy = self.my_copy_of(id, cx);
        let mut actions = h_flex()
            .id("shared-artifact-actions")
            .flex_wrap()
            .gap(t.space.small)
            .child(
                design::button("shared-artifact-copy", "Copy", cx)
                    .icon(Icon::new(IconName::Copy))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    }),
            );
        actions = match my_copy {
            Some(local) => actions.child(
                design::primary_button("shared-artifact-open-copy", "Open my copy", cx)
                    .icon(Icon::new(IconName::FileText))
                    .on_click(cx.listener(move |view, _, _, cx| view.show(&local, cx))),
            ),
            None => actions.child(
                design::primary_button("shared-artifact-keep", "Keep a copy", cx)
                    .icon(Icon::new(IconName::Download))
                    .on_click(cx.listener(move |view, _, _, cx| view.keep_copy(&kept, cx))),
            ),
        };
        if may_withdraw && artifact.is_latest() {
            actions = actions.child(
                design::button(
                    "shared-artifact-withdraw",
                    if self.confirm_withdraw {
                        "Click again to stop sharing"
                    } else {
                        "Stop sharing"
                    },
                    cx,
                )
                .ghost()
                .on_click(cx.listener(move |view, _, _, cx| {
                    if view.confirm_withdraw {
                        let _ = cx
                            .global::<RuntimeClient>()
                            .0
                            .send(Command::Group(GroupCommand::Withdraw { detail: id }));
                        view.confirm_withdraw = false;
                        view.share_notice =
                            Some("Stopped sharing. Copies people kept stay theirs.".into());
                    } else {
                        view.confirm_withdraw = true;
                    }
                    cx.notify();
                })),
            );
        }
        v_flex()
            .id("shared-artifact-detail")
            .test_support()
            .w_full()
            .max_w(t.chat.column_width)
            .gap(t.space.medium)
            .child(
                h_flex().child(
                    design::button("shared-artifact-back", "← All artifacts", cx)
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| view.back_to_library(cx))),
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
                    .child(format!(
                        "{} · shared by {}{from} · {when}",
                        info.label, artifact.shared_by
                    )),
            )
            .children(note.map(|note| {
                div()
                    .id("shared-artifact-note")
                    .test_support()
                    .whitespace_normal()
                    .sovra_text(TextRole::Body, cx)
                    .text_color(t.text_secondary)
                    .child(note)
            }))
            .child(actions)
            .children(self.share_feedback(None, cx))
            .child(
                div()
                    .id("shared-artifact-preview")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .p(t.space.medium)
                    .sovra_surface(Surface::Subtle, cx)
                    .sovra_text(TextRole::Reading, cx)
                    .child(
                        TextView::markdown(
                            (
                                "shared-artifact-preview-text",
                                artifact.shared_at.millis().unsigned_abs(),
                            ),
                            preview_markdown(artifact.kind, &artifact.language, &artifact.content),
                        )
                        .selectable(true)
                        .style(design::markdown_style(cx)),
                    ),
            )
    }
}

/// "Shared in <group>": the latest version of each artifact people shared.
fn shared_section(
    glance: &GroupGlance,
    now: Timestamp,
    cx: &mut Context<ArtifactsView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .id("shared-artifacts")
        .test_support()
        .w_full()
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(format!("Shared in {}", glance.name)),
        )
        .child(
            div()
                .whitespace_normal()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child("Things people in your group shared. Open one to read it or keep a copy."),
        )
        .child(
            h_flex()
                .id("shared-artifact-grid")
                .w_full()
                .flex_wrap()
                .gap(t.space.medium)
                .children(
                    glance
                        .cards
                        .iter()
                        .enumerate()
                        .map(|(index, card)| shared_card(card, index, now, cx)),
                ),
        )
}

fn shared_card(
    card: &SharedCard,
    index: usize,
    now: Timestamp,
    cx: &mut Context<ArtifactsView>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let id = card.id;
    let mut caption = format!(
        "{} · shared by {} · {}",
        card.kind.label(),
        card.shared_by,
        format_relative_time(card.shared_at, now)
    );
    if card.two_versions {
        caption.push_str(" · two versions");
    }
    if let Some(label) = &card.space_label {
        caption.push_str(" · ");
        caption.push_str(&label.to_lowercase());
    }
    v_flex()
        .id(("shared-artifact-card", index))
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
                .child(Icon::new(kind_icon(card.kind)).text_color(cx.theme().primary))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .text_ellipsis()
                        .child(card.title.clone()),
                ),
        )
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(caption),
        )
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .line_clamp(2)
                .child(card.snippet.clone()),
        )
        .child(
            h_flex().child(
                design::button(("shared-artifact-open", index), "Open", cx)
                    .small()
                    .accessibility_label(format!("Open {}", card.title))
                    .on_click(cx.listener(move |view, _, _, cx| view.show_shared(id, cx))),
            ),
        )
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
