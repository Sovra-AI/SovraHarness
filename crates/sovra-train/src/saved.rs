//! The saved form of a run: a small JSON header, the text, and the merge engine's
//! compact binary state.
//!
//! | Bytes | Field |
//! |---|---|
//! | 0..8 | `SOVRARUN` |
//! | 8..12 | format version (4) |
//! | 12..16 | header length |
//! | 16..24 | text length |
//! | 24..32 | merge state length |
//! | 32..40 | best model's length (version 4 only; zero when there is none) |
//! | then | header (JSON), text (UTF-8), merge state, best model (f32 values) |
//!
//! Every length is checked against its bound and the file's real size before
//! anything is allocated. Version 3 is the same without the best model, and
//! still read. Versions 1 and 2 were all-JSON files for the Starter table; they
//! are still read, so a run saved by the first release carries on.

use serde::{Deserialize, Serialize};
use sovra_core::group::ContentHash;
use sovra_core::swarm::learn::scavenger::GridCheckpoint;
use sovra_core::training::{
    Architecture, BaseModel, HardwareInfo, LrSchedule, MAX_CHECKPOINT_BYTES, MAX_SAMPLE_TEXT_BYTES,
    MAX_TEXT_BYTES, MetricPoint, SourceKind, SourceSummary, TrainingBase, TrainingError,
    TrainingPlanSummary, TrainingPreset, TrainingSettings,
};

const MAGIC: &[u8; 8] = b"SOVRARUN";
const VERSION: u32 = 4;
/// The version before the best model was kept, still read.
const VERSION_WITHOUT_BEST: u32 = 3;
const PREFIX_BYTES: usize = 40;
const PREFIX_BYTES_WITHOUT_BEST: usize = 32;
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// The first release's limits, for reading its files.
const LEGACY_MAX_BYTES: usize = 8 * 1024 * 1024;
const LEGACY_MAX_TEXT_BYTES: usize = 65_536;
const LEGACY_MIN_ROUNDS: u32 = 80;

/// Everything about a run that is not the text or the model values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Header {
    pub(crate) plan: TrainingPlanSummary,
    /// The family the finished model is shown as.
    pub(crate) family: BaseModel,
    /// What the starting model had already learned, when continuing one.
    pub(crate) base_rounds: u32,
    pub(crate) base_tokens: u64,
    /// Ties the header to the text saved beside it.
    pub(crate) text: ContentHash,
    pub(crate) cursor: u64,
    pub(crate) loss_milli: Option<u32>,
    pub(crate) check_loss_milli: Option<u32>,
    /// Best check so far and the round it was measured on.
    pub(crate) best_check: Option<(u32, u32)>,
    pub(crate) stalled: bool,
    pub(crate) sample: Option<(u32, String)>,
    pub(crate) point: Option<MetricPoint>,
    /// What the model had learned when the best check was measured, kept with
    /// the model's values at that moment (the file's last part).
    #[serde(default)]
    pub(crate) best: Option<BestPoint>,
}

/// The progress that goes with the best model kept beside a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BestPoint {
    pub(crate) round: u32,
    pub(crate) tokens: u64,
    pub(crate) loss_milli: Option<u32>,
    pub(crate) check_loss_milli: u32,
}

pub(crate) struct Saved {
    pub(crate) header: Header,
    pub(crate) text: String,
    pub(crate) grid: GridCheckpoint,
    /// Set for the oldest files only, which were tied to their text by the run's
    /// own name: the restored merge state must carry exactly this name.
    pub(crate) job: Option<ContentHash>,
    /// The model's values when the best check was measured, if one was kept.
    pub(crate) best: Option<Vec<f32>>,
}

#[cfg(test)]
impl Saved {
    /// The model's values as the merge engine holds them.
    pub(crate) fn grid_params(&self) -> Vec<f32> {
        sovra_core::swarm::learn::scavenger::GridSyncer::restore(self.grid.clone())
            .map(|grid| grid.params().to_vec())
            .unwrap_or_default()
    }
}

pub(crate) fn encode(
    header: &Header,
    text: &str,
    grid: &GridCheckpoint,
    best: Option<&[f32]>,
) -> Result<Vec<u8>, TrainingError> {
    let header = serde_json::to_vec(header).map_err(|_| TrainingError::Step)?;
    let grid = grid.to_bytes();
    let best: Vec<u8> = best
        .unwrap_or_default()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let total = PREFIX_BYTES + header.len() + text.len() + grid.len() + best.len();
    let header_len = u32::try_from(header.len()).map_err(|_| TrainingError::Step)?;
    if header.len() > MAX_HEADER_BYTES || total > MAX_CHECKPOINT_BYTES {
        return Err(TrainingError::Step);
    }
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&header_len.to_le_bytes());
    bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(grid.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(best.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(&grid);
    bytes.extend_from_slice(&best);
    Ok(bytes)
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], TrainingError> {
    bytes
        .get(at..at + N)
        .and_then(|part| part.try_into().ok())
        .ok_or(TrainingError::InvalidCheckpoint)
}

fn length(bytes: &[u8], at: usize, most: usize) -> Result<usize, TrainingError> {
    usize::try_from(u64::from_le_bytes(field(bytes, at)?))
        .ok()
        .filter(|len| *len <= most)
        .ok_or(TrainingError::InvalidCheckpoint)
}

/// Reads a saved run of any supported version. The parts are returned as read:
/// the caller still checks them against each other and restores the merge state
/// through its own validation.
pub(crate) fn decode(bytes: &[u8]) -> Result<Saved, TrainingError> {
    let invalid = TrainingError::InvalidCheckpoint;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(invalid);
    }
    if !bytes.starts_with(MAGIC) {
        return legacy(bytes);
    }
    let (prefix, with_best) = match u32::from_le_bytes(field(bytes, 8)?) {
        VERSION => (PREFIX_BYTES, true),
        VERSION_WITHOUT_BEST => (PREFIX_BYTES_WITHOUT_BEST, false),
        _ => return Err(invalid),
    };
    let header_len = usize::try_from(u32::from_le_bytes(field(bytes, 12)?))
        .ok()
        .filter(|len| *len <= MAX_HEADER_BYTES)
        .ok_or(TrainingError::InvalidCheckpoint)?;
    let text_len = length(bytes, 16, MAX_TEXT_BYTES)?;
    let grid_len = length(bytes, 24, MAX_CHECKPOINT_BYTES)?;
    let best_len = if with_best {
        length(bytes, 32, MAX_CHECKPOINT_BYTES)?
    } else {
        0
    };
    if bytes.len() != prefix + header_len + text_len + grid_len + best_len {
        return Err(invalid);
    }
    let (header, rest) = bytes[prefix..].split_at(header_len);
    let (text, rest) = rest.split_at(text_len);
    let (grid, best) = rest.split_at(grid_len);
    let header: Header = serde_json::from_slice(header).map_err(|_| invalid.clone())?;
    // The shape is checked before its size is used to accept the merge state.
    header.plan.validate().map_err(|_| invalid.clone())?;
    // A best model is the model's full size and goes with its progress, or
    // neither is there.
    let parameters = header.plan.architecture.parameters();
    if (best_len != 0) != header.best.is_some() || (best_len != 0 && best_len != parameters * 4) {
        return Err(invalid);
    }
    let best: Option<Vec<f32>> = (best_len != 0).then(|| {
        best.as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect()
    });
    if best
        .as_ref()
        .is_some_and(|values| values.iter().any(|value| !value.is_finite()))
    {
        return Err(invalid);
    }
    if grid.len() != GridCheckpoint::byte_len(parameters)
        || header
            .sample
            .as_ref()
            .is_some_and(|(_, text)| text.len() > MAX_SAMPLE_TEXT_BYTES)
    {
        return Err(invalid);
    }
    let text = std::str::from_utf8(text).map_err(|_| invalid.clone())?;
    if ContentHash::of(text.as_bytes()) != header.text {
        return Err(invalid);
    }
    Ok(Saved {
        header,
        text: text.to_owned(),
        grid: GridCheckpoint::from_bytes(grid).map_err(|_| invalid)?,
        job: None,
        best,
    })
}

/// The first release's all-JSON file.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Legacy {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) recipe: Option<ContentHash>,
    pub(crate) text: String,
    pub(crate) cursor: usize,
    pub(crate) grid: GridCheckpoint,
    pub(crate) loss_milli: Option<u32>,
}

pub(crate) fn legacy_recipe(text: &str) -> ContentHash {
    let mut bytes = b"sovra-byte-grid-session-v1\0".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    ContentHash::of(&bytes)
}

/// Rounds the first release gave a text: at least 80, and enough to read it all.
fn legacy_rounds(text: &str) -> u32 {
    LEGACY_MIN_ROUNDS.max(u32::try_from(text.len().div_ceil(508)).unwrap_or(u32::MAX))
}

fn legacy(bytes: &[u8]) -> Result<Saved, TrainingError> {
    let invalid = TrainingError::InvalidCheckpoint;
    if bytes.len() > LEGACY_MAX_BYTES || bytes.first() != Some(&b'{') {
        return Err(invalid);
    }
    let saved: Legacy = serde_json::from_slice(bytes).map_err(|_| invalid.clone())?;
    let recipe_valid = match saved.version {
        1 => saved.recipe.is_none(),
        2 => saved.recipe == Some(legacy_recipe(&saved.text)),
        _ => false,
    };
    if !recipe_valid
        || saved.text.len() > LEGACY_MAX_TEXT_BYTES
        || saved.grid.parameters() != Architecture::STARTER.parameters()
    {
        return Err(invalid);
    }
    // The run it describes, in today's terms: the Starter table on the first
    // release's fixed settings, learning from the whole text.
    let architecture = Architecture::STARTER;
    let mut settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        architecture,
        saved.text.len(),
        &HardwareInfo::default(),
    );
    settings.rounds = legacy_rounds(&saved.text);
    settings.schedule = LrSchedule::Constant;
    settings.warmup_rounds = 0;
    settings.check_share_percent = 0;
    settings.stop_when_stalled = 0;
    let header = Header {
        plan: TrainingPlanSummary {
            name: "My Starter model".into(),
            base: TrainingBase::Fresh(BaseModel::Starter),
            architecture,
            source: SourceSummary {
                kind: SourceKind::Text,
                bytes: saved.text.len() as u64,
                file_name: None,
            },
            preset: None,
            settings,
            with_group: false,
        },
        family: BaseModel::Starter,
        base_rounds: 0,
        base_tokens: 0,
        text: ContentHash::of(saved.text.as_bytes()),
        cursor: saved.cursor as u64,
        loss_milli: saved.loss_milli,
        check_loss_milli: None,
        best_check: None,
        stalled: false,
        sample: None,
        point: None,
        best: None,
    };
    Ok(Saved {
        job: (saved.version == 1).then(|| legacy_recipe(&saved.text)),
        header,
        text: saved.text,
        grid: saved.grid,
        best: None,
    })
}
