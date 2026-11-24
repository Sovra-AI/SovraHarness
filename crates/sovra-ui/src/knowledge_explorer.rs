//! Search the local evidence stores and read the wording behind each calculation.
mod map;

use crate::{
    RuntimeClient,
    design::{self, SovraStyled as _, Surface, TextRole},
    shell::Shell,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Selectable as _, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
};
use sovra_core::{
    Command, Event, MemoryEvent, MemoryReviewResult, MemoryStatus,
    artifacts::ArtifactEvent,
    file_index::IndexEvent,
    knowledge_analysis::{AnalysisOptions, Citation, Exploration, ExploreRequest, Language},
};
/// Search options fold away; the JSON rules box folds inside them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Options {
    Closed,
    Open,
    Expert,
}

pub(crate) struct ExplorerView {
    query: Entity<InputState>,
    settings: Entity<InputState>,
    _enter: gpui_kit::Subscription,
    options: Options,
    results_folded: bool,
    language: Language,
    window_seconds: u32,
    sequence: u64,
    pending_request: Option<u64>,
    notice: String,
    result: Option<Exploration>,
    opened: Option<usize>,
    map: map::EvidenceMap,
    pub(crate) reading_personal_source: bool,
}
impl ExplorerView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search files, saved details and artifacts…")
        });
        let settings = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(r#"{"language":"english","window_seconds":86400,"mask_radius":1}"#)
        });
        let enter = cx.subscribe_in(&query, window, |shell, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                shell.explore_sources(cx);
            }
        });
        Self {
            query,
            settings,
            _enter: enter,
            options: Options::Closed,
            results_folded: false,
            language: Language::English,
            window_seconds: 86_400,
            sequence: 0,
            pending_request: None,
            notice: String::new(),
            result: None,
            opened: None,
            map: map::EvidenceMap::default(),
            reading_personal_source: false,
        }
    }
    pub(crate) fn fold_results(&mut self) {
        self.results_folded = true;
        self.options = Options::Closed;
    }

    pub(crate) fn observe(&mut self, event: &Event) {
        if let Event::Explored { request_id, result } = event {
            if self.pending_request == Some(*request_id) {
                self.pending_request = None;
                match result {
                    Ok(report) => {
                        self.notice = if report.readings.is_empty() {
                            "No matching wording found.".into()
                        } else {
                            String::new()
                        };
                        self.map = map::EvidenceMap::for_report(report);
                        self.result = Some(report.clone());
                    }
                    Err(message) => self.notice.clone_from(message),
                }
            }
            return;
        }
        if self.pending_request.is_none() && self.result.is_none() {
            return;
        }
        let changed = matches!(
            event,
            Event::Artifacts(ArtifactEvent::Saved(_) | ArtifactEvent::Deleted { .. })
                | Event::Memory(MemoryEvent::Status(
                    MemoryStatus::Paused | MemoryStatus::Unavailable
                ))
                | Event::FileIndex(IndexEvent::Roots(_))
        ) || matches!(event, Event::FileIndex(IndexEvent::Progress(root)) if root.status != sovra_core::file_index::IndexStatus::Scanning && root.status != sovra_core::file_index::IndexStatus::Pending)
            || matches!(event,Event::Memory(MemoryEvent::Review(e)) if matches!(e.result,Ok(MemoryReviewResult::Changed(_) | MemoryReviewResult::SourceStopped(_))));
        if changed {
            self.sequence = self.sequence.wrapping_add(1);
            self.result = None;
            self.opened = None;
            self.pending_request = None;
            self.notice = "Sources changed. Search again for current wording.".into();
        }
    }
}
fn configured_options(
    language: Language,
    window_seconds: u32,
    overrides: &str,
) -> Result<AnalysisOptions, String> {
    let options = AnalysisOptions {
        language,
        window_seconds,
        ..AnalysisOptions::default()
    };
    if overrides.trim().is_empty() {
        return Ok(options);
    }
    let mut json: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(overrides).map_err(|e| format!("Check the analysis settings. {e}"))?;
    json.entry("language")
        .or_insert_with(|| serde_json::json!(language));
    json.entry("window_seconds")
        .or_insert_with(|| window_seconds.into());
    serde_json::from_value(serde_json::Value::Object(json))
        .map_err(|e| format!("Check the analysis settings. {e}"))
}

impl Shell {
    pub(crate) fn explore_sources(&mut self, cx: &mut Context<Self>) {
        let view = &mut self.explorer;
        view.sequence = view.sequence.wrapping_add(1);
        view.result = None;
        view.results_folded = false;
        view.pending_request = None;
        view.opened = None;
        let options = view.settings.read(cx).value();
        let options = configured_options(view.language, view.window_seconds, &options);
        let result = options.and_then(|options| {
            options.validate().map_err(str::to_owned)?;
            let query = view.query.read(cx).value().trim().to_owned();
            if query.is_empty() {
                return Err("Enter a name or words from a source.".into());
            }
            cx.global::<RuntimeClient>()
                .0
                .send(Command::Explore(ExploreRequest {
                    id: view.sequence,
                    query,
                    options,
                }))
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => {
                view.pending_request = Some(view.sequence);
                view.notice = "Searching connected sources…".into();
            }
            Err(message) => view.notice = message,
        }
        cx.notify();
    }
}
pub(crate) fn render(shell: &Shell, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let view = &shell.explorer;
    let t = design::tokens(cx);
    let mut column = v_flex()
        .id("knowledge-explorer")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .child(
            div()
                .id("knowledge-explore")
                .test_support()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Search your knowledge"),
        )
        .child(
            Input::new(&view.query)
                .id("explore-query")
                .aria_label("Search connected knowledge"),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::primary_button("explore-submit", "Search sources", cx)
                        .on_click(cx.listener(|shell, _, _, cx| shell.explore_sources(cx))),
                )
                .child(
                    design::button("explore-settings", "Search options", cx)
                        .ghost()
                        .icon(if view.options == Options::Closed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .selected(view.options != Options::Closed)
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.explorer.options = match shell.explorer.options {
                                Options::Closed => Options::Open,
                                Options::Open | Options::Expert => Options::Closed,
                            };
                            cx.notify();
                        })),
                ),
        );
    if view.options != Options::Closed {
        let mut languages = h_flex().flex_wrap().gap(t.space.compact);
        for (index, language) in [
            Language::English,
            Language::French,
            Language::German,
            Language::Spanish,
            Language::Italian,
            Language::Portuguese,
            Language::Dutch,
            Language::Arabic,
        ]
        .into_iter()
        .enumerate()
        {
            languages = languages.child(
                design::button(("explorer-language", index), format!("{language:?}"), cx)
                    .small()
                    .selected(view.language == language)
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.explorer.language = language;
                        cx.notify();
                    })),
            );
        }
        let mut windows = h_flex().flex_wrap().gap(t.space.compact);
        for (index, (label, seconds)) in [("Minute", 60), ("Hour", 3600), ("Day", 86_400)]
            .into_iter()
            .enumerate()
        {
            windows = windows.child(
                design::button(("explorer-window", index), label, cx)
                    .small()
                    .selected(view.window_seconds == seconds)
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.explorer.window_seconds = seconds;
                        cx.notify();
                    })),
            );
        }
        let lang_label = format!("Language of your files · {:?}", view.language);
        let time_label = format!(
            "Group dated readings by · {}",
            match view.window_seconds {
                60 => "minute".to_owned(),
                3_600 => "hour".to_owned(),
                86_400 => "day".to_owned(),
                seconds => format!("{seconds} seconds"),
            }
        );
        let mut options = v_flex()
            .id("explore-options")
            .test_support()
            .w_full()
            .min_w_0()
            .gap(t.space.small)
            .p(t.space.medium)
            .sovra_surface(Surface::Subtle, cx)
            .child(
                div()
                    .id("explore-selected-language")
                    .test_support()
                    .aria_label(lang_label.clone())
                    .sovra_text(TextRole::Caption, cx)
                    .child(lang_label),
            )
            .child(languages)
            .child(
                div()
                    .id("explore-selected-window")
                    .test_support()
                    .aria_label(time_label.clone())
                    .sovra_text(TextRole::Caption, cx)
                    .child(time_label),
            )
            .child(windows)
            .child(
                design::button("explore-expert", "Custom rules for experts", cx)
                    .ghost()
                    .small()
                    .self_start()
                    .icon(if view.options == Options::Expert {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.explorer.options = match shell.explorer.options {
                            Options::Expert => Options::Open,
                            Options::Closed | Options::Open => Options::Expert,
                        };
                        cx.notify();
                    })),
            );
        if view.options == Options::Expert {
            options = options
                .child(div().sovra_text(TextRole::Caption, cx).child("Optional JSON overrides individual settings. Set mask_radius for images, profiles for table columns, or properties for the classes a relationship connects. See the Knowledge guide."))
                .child(Input::new(&view.settings).id("explore-settings-json").aria_label("Analysis settings JSON"));
        }
        column = column.child(options);
    }
    if !view.notice.is_empty() {
        column = column.child(
            div()
                .id("explore-status")
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(view.notice.clone())
                .sovra_text(TextRole::Body, cx)
                .child(view.notice.clone()),
        );
    }
    if let Some(result) = &view.result {
        if view.results_folded && !result.readings.is_empty() {
            return column.child(
                design::button(
                    "explore-show-results",
                    format!("Show search results ({})", result.readings.len()),
                    cx,
                )
                .self_start()
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.explorer.results_folded = false;
                    cx.notify();
                })),
            );
        }
        for (i, note) in result.notices.iter().enumerate() {
            column = column.child(
                div()
                    .id(("explore-summary", i))
                    .test_support()
                    .aria_label(note.clone())
                    .sovra_text(TextRole::Body, cx)
                    .child(note.clone()),
            );
        }
        if !result.readings.is_empty() {
            column = column.child(map::render(&view.map, result, cx));
        }
        for (i, reading) in result.readings.iter().enumerate() {
            if !view.map.list && view.map.selected.map(|line| line.reading) != Some(i) {
                continue;
            }
            let excerpt = &reading.excerpt;
            let mut card = v_flex()
                .id(("explore-source", i))
                .test_support()
                .aria_label(format!("{} · {}", excerpt.title, excerpt.provenance))
                .w_full()
                .min_w_0()
                .sovra_surface(Surface::Matte, cx)
                .p(t.space.medium)
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(excerpt.title.clone()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(excerpt.provenance.clone()),
                );
            for f in &reading.analysis.findings {
                let first = excerpt.first_line.saturating_add(f.from.saturating_sub(1));
                let last = excerpt.first_line.saturating_add(f.to.saturating_sub(1));
                card = card.child(div().sovra_text(TextRole::Body, cx).child(format!(
                    "{} · lines {first}–{last} · {}",
                    f.kind.label(),
                    f.detail
                )));
            }
            let open = view.opened == Some(i);
            card = card.child(
                design::button(
                    ("explore-wording", i),
                    if open {
                        "Hide source wording"
                    } else {
                        "Read source wording"
                    },
                    cx,
                )
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.explorer.opened = if open { None } else { Some(i) };
                    cx.notify();
                })),
            );
            if open {
                card = card.child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child(excerpt.wording.clone()),
                );
            }
            match &excerpt.citation {
                Citation::User { source_id, .. } => {
                    let id = source_id.clone();
                    card = card.child(
                        design::button(("explore-record", i), "Open original record", cx).on_click(
                            cx.listener(move |shell, _, _, cx| {
                                shell.open_memory_source(
                                    sovra_core::MemoryAction::InspectSource(id.clone()),
                                    cx,
                                );
                                shell.explorer.reading_personal_source = true;
                            }),
                        ),
                    );
                }
                Citation::Artifact { id, .. } => {
                    let id = id.clone();
                    card = card.child(
                        design::button(("explore-artifact", i), "Open artifact", cx).on_click(
                            cx.listener(move |shell, _, _, cx| shell.open_artifact(&id, cx)),
                        ),
                    );
                }
                Citation::File { reference, .. } => {
                    let root = reference.root_id.clone();
                    card = card.child(
                        design::button(("explore-folder", i), "Browse source folder", cx).on_click(
                            cx.listener(move |shell, _, window, cx| {
                                shell.browse_index_folder(Some(root.clone()), window, cx);
                            }),
                        ),
                    );
                }
            }
            column = column.child(card);
        }
    }
    column
}
