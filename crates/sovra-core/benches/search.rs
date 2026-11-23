//! Search and filter paths that run on every keystroke or redraw, at realistic sizes.
//!
//! Run with `cargo bench -p sovra-core --bench search`. Compare before and after a
//! change on the same machine; see `docs/performance.md`.

use divan::{Bencher, black_box};
use sovra_core::artifacts::{
    Artifact, ArtifactFilter, ArtifactKind, ArtifactLibrary, ArtifactSort, ArtifactSource,
};
use sovra_core::chat_history::{ChatHistoryFilter, ChatThread, group_threads};
use sovra_core::models::{
    CatalogScope, CatalogSnapshot, ModelConnection, ModelEvent, ModelFilter, ModelInfo,
    ModelPreferences, ModelSort, ModelsState, ProviderKind,
};
use sovra_core::{ChatMessage, Timestamp};

fn main() {
    divan::main();
}

/// A paragraph like the ones in model descriptions and chat replies.
const PROSE: &str = "Sovra keeps your notes, files and chats on this computer. It answers \
questions, drafts letters, plans trips and explains documents in plain words — and it \
remembers what matters to you, so you never have to repeat yourself. ";

fn models(count: usize) -> ModelsState {
    let authors = [
        "openai",
        "anthropic",
        "google",
        "meta",
        "mistral",
        "qwen",
        "deepseek",
    ];
    let models = (0..count)
        .map(|i| ModelInfo {
            id: format!("{}/model-{i:04}", authors[i % authors.len()]),
            name: format!("Model {i:04} Turbo Preview"),
            author: authors[i % authors.len()].into(),
            description: PROSE.repeat(3),
            context_length: Some(8_192 * (1 + (i as u64 % 16))),
            input_modalities: vec!["text".into()],
            output_modalities: vec!["text".into()],
            supported_parameters: vec!["tools".into()],
            details_loaded: true,
            ..ModelInfo::default()
        })
        .collect();
    let mut state = ModelsState::default();
    state.apply(&ModelEvent::Loaded {
        connections: vec![ModelConnection::new("router", ProviderKind::OpenRouter)],
        preferences: ModelPreferences::default(),
    });
    state.apply(&ModelEvent::CatalogLoaded {
        connection_id: "router".into(),
        snapshot: CatalogSnapshot {
            models,
            fetched_at: 1,
        },
        cached: false,
    });
    state
}

/// Typing a search in the Models page.
#[divan::bench]
fn models_search(bencher: Bencher) {
    let state = models(2_000);
    let filter = ModelFilter {
        search: "mistral turbo".into(),
        ..ModelFilter::default()
    };
    bencher.bench(|| black_box(&state).filtered(black_box(&filter)));
}

/// Opening the Models page sorted by name, with no search.
#[divan::bench]
fn models_sort_by_name(bencher: Bencher) {
    let state = models(2_000);
    let filter = ModelFilter {
        sort: ModelSort::Name,
        ..ModelFilter::default()
    };
    bencher.bench(|| black_box(&state).filtered(black_box(&filter)));
}

/// The Models page's filter choices, rebuilt when the catalog or preferences change.
#[divan::bench]
fn models_facets(bencher: Bencher) {
    let state = models(2_000);
    bencher.bench(|| black_box(&state).facets(black_box(CatalogScope::All)));
}

fn threads(count: i64, messages: usize) -> Vec<ChatThread> {
    (0..count)
        .map(|t| {
            let mut thread = ChatThread::new(format!("thread-{t}"), Timestamp(t));
            thread.title = format!("Planning conversation {t}");
            for m in 0..messages {
                let text = format!("{PROSE}{PROSE}Message {m} of chat {t}.");
                thread.messages.push(if m % 2 == 0 {
                    ChatMessage::user(format!("u{m}"), text, "Earlier")
                } else {
                    ChatMessage::assistant(format!("a{m}"), text, "Earlier", Vec::new())
                });
            }
            thread.refresh_summary(Timestamp(t));
            thread
        })
        .collect()
}

/// Typing in the sidebar's chat search; the sidebar redraws on every event batch.
#[divan::bench]
fn history_search(bencher: Bencher) {
    let threads = threads(120, 40);
    bencher.bench(|| {
        group_threads(
            black_box(&threads),
            ChatHistoryFilter::Chats,
            black_box("passport renewal"),
            Timestamp(1_000),
        )
        .groups
        .len()
    });
}

fn library(count: i64) -> ArtifactLibrary {
    let artifacts = (0..count)
        .map(|i| Artifact {
            id: format!("artifact-{i}"),
            title: format!("Letter to the council {i}"),
            kind: ArtifactKind::Document,
            language: String::new(),
            content: PROSE.repeat(16),
            source: ArtifactSource::default(),
            collection: if i % 3 == 0 {
                "Home".into()
            } else {
                String::new()
            },
            pinned: false,
            created_at: Timestamp(i),
            updated_at: Timestamp(i),
            versions: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut library = ArtifactLibrary::default();
    library.apply(&sovra_core::artifacts::ArtifactEvent::Loaded(artifacts));
    library
}

/// Typing in the Artifacts search, sorted by name.
#[divan::bench]
fn artifacts_search(bencher: Bencher) {
    let library = library(300);
    let filter = ArtifactFilter {
        query: "parking permit".into(),
        sort: ArtifactSort::Name,
        ..ArtifactFilter::default()
    };
    bencher.bench(|| black_box(&library).filtered(black_box(&filter)).len());
}

/// A small sensor export with mixed units and unordered event times.
#[divan::bench]
fn knowledge_sensor_table(bencher: Bencher) {
    use std::fmt::Write as _;
    let mut data = String::from("timestamp,value,unit,instrument\n");
    for i in 0..64 {
        writeln!(data, "2025-10-{:02},0.5,kg,scale-1", 28 - i % 28).expect("write to string");
    }
    bencher.bench(|| {
        sovra_core::knowledge_analysis::analyze(
            black_box(&data),
            &sovra_core::knowledge_analysis::AnalysisOptions::default(),
        )
        .expect("sensor fixture")
    });
}
#[divan::bench(args=[16,64])]
fn knowledge_point_cloud(bencher: Bencher, n: u32) {
    let points: Vec<_> = (0..n)
        .map(|i| {
            let angle = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            vec![angle.cos(), angle.sin()]
        })
        .collect();
    bencher.bench(|| {
        sovra_core::knowledge_analysis::topology::persistence(black_box(&points))
            .expect("finite fixture points")
    });
}
