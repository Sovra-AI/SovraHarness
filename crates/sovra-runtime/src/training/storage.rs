//! Where training keeps things on disk, and the bounded readers for them.
//!
//! ```text
//! training/
//!   owner.lock                 one Sovra window owns training
//!   latest.json                the name of the current run's file
//!   run-<64 hex>.train         a run: model, optimizer state and its text
//!   run-<64 hex>.notes.json    that run's chart history and whether it finished
//!   models/<64 hex>.model      a finished model, named by its content
//!   models/<64 hex>.json       that model's card
//! ```
//!
//! Nothing here is ever deleted: a new run gets a new file, and a model that is
//! already in the library is left as it is.

use std::{
    fs::File,
    io::Read as _,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sovra_core::training::{
    LibraryModel, MAX_DELIVERIES, MAX_LIBRARY_MODELS, MAX_MODEL_BYTES, MAX_TEXT_BYTES, ModelId,
    TrainingError, TrainingMetrics, text_from_file_bytes,
};

use crate::private_storage::{private_directory, private_file};

pub(super) const LATEST: &str = "latest.json";
const MODELS: &str = "models";
/// A card or a run's notes is a few kilobytes; anything larger is not ours.
const MAX_SMALL_FILE_BYTES: u64 = 256 * 1024;
/// Most files looked at in the models folder, so a folder someone filled by
/// hand cannot stall the page.
const MAX_SCANNED_FILES: usize = 4096;

/// What the runtime remembers about a run beside the engine's own checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RunNotes {
    #[serde(default)]
    pub metrics: TrainingMetrics,
    /// The person pressed Stop, or the run finished early: it is complete even
    /// though it did not reach its last round.
    #[serde(default)]
    pub stopped: bool,
    /// The library model this run was saved as.
    #[serde(default)]
    pub exported: Option<ModelId>,
}

pub(super) fn run_filename(run: &str) -> String {
    format!("run-{run}.train")
}

/// A run's file name: `run-<64 hex>.train`, or `.json` from the first training
/// release. Anything else could point outside the training folder.
pub(super) fn valid_filename(name: &str) -> bool {
    run_id(name).is_some()
}

fn run_id(name: &str) -> Option<&str> {
    let stem = name.strip_prefix("run-")?;
    let id = stem
        .strip_suffix(".train")
        .or_else(|| stem.strip_suffix(".json"))?;
    is_hex_id(id).then_some(id)
}

/// The notes file that goes with a run's file.
pub(super) fn notes_filename(run_file: &str) -> Option<String> {
    run_id(run_file).map(|id| format!("run-{id}.notes.json"))
}

fn is_hex_id(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn models_directory(root: &Path) -> PathBuf {
    root.join(MODELS)
}

pub(super) fn model_filename(id: ModelId) -> String {
    format!("{}.model", id.to_hex())
}

pub(super) fn card_filename(id: ModelId) -> String {
    format!("{}.json", id.to_hex())
}

pub(super) fn acquire_lock(root: &Path) -> Result<File, String> {
    private_directory(root)?;
    let path = root.join("owner.lock");
    if path.exists() {
        private_file(&path)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| "Cannot open training lock.".to_owned())?;
    private_file(&path)?;
    file.try_lock()
        .map_err(|_| "Training is open in another window.".to_owned())?;
    Ok(file)
}

/// Reads one of training's own files, refusing anything that is not a plain,
/// owner-only file or is longer than `limit`.
fn read_own(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    private_file(path)?;
    let file = File::open(path).map_err(|_| "Could not open a saved training file.".to_owned())?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read a saved training file.".to_owned())?;
    if bytes.len() as u64 > limit {
        return Err("A saved training file is larger than its limit.".into());
    }
    Ok(bytes)
}

/// The notes beside a run, or empty notes when there are none or they cannot be
/// read: the chart then starts again from where the run is now.
pub(super) fn read_notes(root: &Path, run_file: &str) -> RunNotes {
    let Some(name) = notes_filename(run_file) else {
        return RunNotes::default();
    };
    let path = root.join(name);
    if !path.exists() {
        return RunNotes::default();
    }
    match read_own(&path, MAX_SMALL_FILE_BYTES)
        .and_then(|bytes| serde_json::from_slice::<RunNotes>(&bytes).map_err(|e| e.to_string()))
    {
        Ok(notes) => notes,
        Err(error) => {
            tracing::warn!(%error, "training notes could not be read; the chart starts again");
            RunNotes::default()
        }
    }
}

/// Reads a library model's file and checks it is the model its name says.
pub(super) fn read_model(root: &Path, id: ModelId) -> Result<Vec<u8>, String> {
    let directory = models_directory(root);
    private_directory(&directory)?;
    let bytes = read_own(&directory.join(model_filename(id)), MAX_MODEL_BYTES as u64)
        .map_err(|_| "That model's file could not be read. Nothing was changed.".to_owned())?;
    if sovra_core::training::model_id(&bytes) != id {
        return Err(TrainingError::InvalidModel.to_string());
    }
    Ok(bytes)
}

/// Every usable model in the library, newest first, at most
/// [`MAX_LIBRARY_MODELS`]. A card whose model file is missing or the wrong size
/// is left out and logged; the files stay where they are.
pub(super) fn scan_library(root: &Path) -> Result<Vec<LibraryModel>, String> {
    let directory = models_directory(root);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    private_directory(&directory)?;
    let entries = std::fs::read_dir(&directory)
        .map_err(|_| "Could not read your saved models.".to_owned())?;
    let mut models = Vec::new();
    for entry in entries.take(MAX_SCANNED_FILES).flatten() {
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|stem| is_hex_id(stem))
            .and_then(|stem| ModelId::from_hex(stem).ok())
        else {
            continue;
        };
        match read_card(&directory, id) {
            Ok(model) => models.push(model),
            Err(error) => {
                tracing::warn!(model = %id.to_hex(), %error, "a saved model is not listed");
            }
        }
    }
    models.sort_by(|a, b| {
        b.card
            .created_unix_ms
            .cmp(&a.card.created_unix_ms)
            .then(a.card.id.cmp(&b.card.id))
    });
    models.truncate(MAX_LIBRARY_MODELS);
    Ok(models)
}

fn read_card(directory: &Path, id: ModelId) -> Result<LibraryModel, String> {
    let bytes = read_own(&directory.join(card_filename(id)), MAX_SMALL_FILE_BYTES)?;
    let mut model: LibraryModel =
        serde_json::from_slice(&bytes).map_err(|_| "its card is damaged".to_owned())?;
    model.card.validate().map_err(|error| error.to_string())?;
    if model.card.id != id {
        return Err("its card names another model".into());
    }
    let path = directory.join(model_filename(id));
    private_file(&path)?;
    let length = std::fs::metadata(&path)
        .map_err(|_| "its file is missing".to_owned())?
        .len();
    if length != model.card.bytes {
        return Err("its file is not the size its card says".into());
    }
    // The folder may have moved since the card was written.
    model.saved_at = path.display().to_string();
    model.delivery.truncate(MAX_DELIVERIES);
    Ok(model)
}

/// Reads the text file a person chose. It is an ordinary file of theirs, so no
/// owner-only rule applies; it must be a real file of plain UTF-8 within the
/// text limit. Every refusal names the file in plain words.
pub(super) fn read_text_file(path: &Path) -> Result<String, String> {
    let name = path.file_name().map_or_else(
        || "That file".to_owned(),
        |name| {
            name.to_string_lossy()
                .chars()
                .filter(|c| !c.is_control())
                .take(sovra_core::training::MAX_FILE_NAME_CHARS)
                .collect()
        },
    );
    let unreadable = || {
        format!("Sovra could not open {name}. Check that it still exists and that you may read it.")
    };
    let too_large = || format!("{name} is larger than 4 MB. Choose a smaller text file.");
    let metadata = std::fs::metadata(path).map_err(|_| unreadable())?;
    if !metadata.is_file() {
        return Err(format!(
            "{name} is not a file. Choose a plain text file ending in .txt or .md."
        ));
    }
    // Room for a byte-order mark, and one byte more to notice an over-long file.
    let limit = MAX_TEXT_BYTES as u64 + 4;
    if metadata.len() >= limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| unreadable())?
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 >= limit {
        return Err(too_large());
    }
    text_from_file_bytes(bytes).map_err(|error| match error {
        TrainingError::InvalidInput => {
            format!("{name} needs between 32 bytes and 4 MB of text to learn from.")
        }
        TrainingError::InvalidFile(_) => {
            format!("{name} is not plain text. Choose a .txt or .md file saved as UTF-8.")
        }
        TrainingError::InvalidWork
        | TrainingError::InvalidSetting(_)
        | TrainingError::InvalidModel
        | TrainingError::InvalidCheckpoint
        | TrainingError::Step => error.to_string(),
    })
}
