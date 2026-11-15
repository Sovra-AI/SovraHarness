//! Trains a small model, hands it to the Ollama on this computer, and prints
//! what Sovra itself writes from a beginning, so the two can be compared.
//!
//! ```text
//! cargo run --release -p sovra-app --example ollama_selftest -- <name> <folder> <small|medium|large> [plain]
//! ```
//!
//! `plain` zeroes the model's output offsets first, which an Ollama older than
//! 0.40.15 needs. The model is added to Ollama under `<name>`; remove it
//! afterwards with `ollama rm <name>`.

use std::io::Write as _;
use std::sync::Arc;

use sovra_core::group::ContentHash;
use sovra_core::training::{
    BaseModel, BlockKind, ModelOutlets, OLLAMA_NEEDED, OLLAMA_OLDEST, OllamaModel, SAMPLE_TEXT,
    TrainingBase, TrainingEngine, TrainingPlan, TrainingPreset, TrainingSettings, TrainingSource,
    decode_model, encode_model, gguf_needs_output_offsets, to_gguf,
};
use sovra_providers::{FileCredentialStore, HttpModelOutlets};
use sovra_train::BurnTrainingEngine;

type Failure = Box<dyn std::error::Error>;

const BEGINNING: &str = "The garden";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Failure> {
    let mut arguments = std::env::args().skip(1);
    let name = arguments.next().ok_or("give a name for the model")?;
    let folder = std::path::PathBuf::from(arguments.next().ok_or("give a folder")?);
    let model = match arguments.next().as_deref() {
        Some("medium") => BaseModel::Medium,
        Some("large") => BaseModel::Large,
        _ => BaseModel::Small,
    };
    let plain = arguments.next().as_deref() == Some("plain");
    std::fs::create_dir_all(&folder)?;

    let engine = BurnTrainingEngine::processor();
    let settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        model.architecture(),
        SAMPLE_TEXT.len(),
        &engine.hardware(),
    );
    let plan = TrainingPlan {
        name: "Self test".into(),
        base: TrainingBase::Fresh(model),
        source: TrainingSource::Sample,
        preset: None,
        settings,
        with_group: false,
    };
    let mut session = engine.create(&plan, SAMPLE_TEXT, None, ContentHash::of(b"self test"))?;
    let mut now = 0;
    while !session.progress().is_complete() {
        now += 1;
        session.advance(now)?;
    }
    let mut file = session.export()?;
    if plain {
        let decoded = decode_model(&file)?;
        let mut values = decoded.parameters;
        for block in decoded.header.architecture.blocks() {
            if block.kind == BlockKind::OutputBias {
                values[block.range()].fill(0.0);
            }
        }
        file = encode_model(&decoded.header, &values)?;
    }
    let gguf = to_gguf(&file, &name)?;
    std::fs::write(folder.join("selftest.gguf"), &gguf)?;
    std::fs::write(folder.join("selftest.model"), &file)?;
    // The likeliest byte every time, so there is one right answer to compare.
    let written = engine.generate(&file, BEGINNING, 80, 0, 1)?;
    let needs = if gguf_needs_output_offsets(&file)? {
        OLLAMA_NEEDED
    } else {
        OLLAMA_OLDEST
    };
    let outlets = HttpModelOutlets::new(
        Arc::new(FileCredentialStore::new(folder.join("keys"))),
        None,
    );
    let added = outlets
        .add_to_ollama(OllamaModel {
            name: name.clone(),
            gguf,
            context: model.architecture().context,
            needs,
        })
        .await;
    let mut out = std::io::stdout().lock();
    writeln!(out, "beginning: {BEGINNING:?}")?;
    writeln!(out, "sovra writes: {written:?}")?;
    writeln!(out, "added to ollama as {name}: {added:?}")?;
    Ok(())
}
