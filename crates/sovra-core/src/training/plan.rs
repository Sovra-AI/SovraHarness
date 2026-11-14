//! What a person asks a training run to do: the text, the starting point, how
//! thorough to be, and every expert setting with its bounds.
//!
//! All values are whole numbers (thousandths or millionths where a fraction is
//! meant) so plans compare exactly and read the same on every computer.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::swarm::learn::scavenger::GridSettings;

use super::{
    Architecture, BaseModel, HardwareInfo, ModelId, TrainingEngineKind, TrainingError, sample,
};

/// Most text one run learns from.
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Least text one run needs.
pub const MIN_TEXT_BYTES: usize = 32;
/// Longest model name, in characters.
pub const MAX_NAME_CHARS: usize = 60;
/// Longest file name shown for a chosen text file, in characters.
pub const MAX_FILE_NAME_CHARS: usize = 120;
/// Most text one work packet carries.
pub const MAX_WORK_BATCH_BYTES: usize = MAX_BATCH as usize;
/// Least text one work packet carries.
pub const MIN_WORK_BATCH_BYTES: usize = MIN_BATCH as usize;
/// Most text an old Starter helper accepts in one packet.
pub const LEGACY_WORK_BATCH_BYTES: usize = 512;
const MAX_BATCH: u32 = 4096;
const MIN_BATCH: u32 = 64;
/// Most learning steps a helper takes on one packet.
pub const MAX_LOCAL_STEPS: u32 = 16;
/// Most windows read from one packet in each step.
pub const MAX_WINDOWS: u16 = 64;
/// Most bytes of generated sample text.
pub const MAX_SAMPLE_BYTES: u16 = 512;

/// The built-in sample: a few thousand bytes, enough for every starting point.
pub const SAMPLE_TEXT: &str = sample::SAMPLE_TEXT;

/// How thorough a run is. Each is a complete set of recommended settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingPreset {
    Quick,
    Balanced,
    Thorough,
}

impl TrainingPreset {
    pub const ALL: [Self; 3] = [Self::Quick, Self::Balanced, Self::Thorough];

    pub fn title(self) -> &'static str {
        match self {
            Self::Quick => "Quick try",
            Self::Balanced => "Balanced",
            Self::Thorough => "Thorough",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Quick => "A short run to see it learn.",
            Self::Balanced => "A good model in a reasonable time.",
            Self::Thorough => "The best result. Leave it running, and pause whenever you like.",
        }
    }
}

/// How the step size changes over a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LrSchedule {
    /// The same step size throughout.
    Constant,
    /// Starts full and eases down to a tenth.
    Cosine,
    /// Ramps up over the warm-up rounds, then eases down to a tenth.
    WarmupCosine,
}

/// The rule a computer uses for its learning steps on one packet. Its memory
/// starts fresh with every packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalOptimizer {
    Sgd,
    Adam,
}

/// Every expert setting. `recommended` fills them from a preset, so nobody has to
/// open Advanced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingSettings {
    /// Total rounds. Each round folds one packet's learning into the model.
    pub rounds: u32,
    /// Bytes of text in one work packet.
    pub batch_bytes: u32,
    /// Windows read from the packet in each step. The Starter table reads the whole
    /// packet at once, so it is always 1 there.
    pub windows: u16,
    /// Learning steps taken on one packet before its result is folded in.
    pub local_steps: u32,
    /// Step size for those steps, in millionths, the same in every round. It
    /// mostly sets the direction: `outer_learning_rate_micro` sets how far the
    /// model moves each round, and the schedule scales that one only.
    pub learning_rate_micro: u32,
    pub schedule: LrSchedule,
    pub warmup_rounds: u32,
    pub optimizer: LocalOptimizer,
    pub weight_decay_micro: u32,
    /// Largest size of one local step's correction, in thousandths; 0 = no limit.
    /// (Each round's folded-in update is always limited as well.)
    pub clip_milli: u32,
    /// How far the model moves each round, in millionths. The schedule scales it.
    pub outer_learning_rate_micro: u32,
    /// Share of the text's end kept back to check the model on text it has not
    /// learned from; 0 = none.
    pub check_share_percent: u8,
    /// Rounds between checks and fresh sample text.
    pub check_every: u32,
    /// How adventurous the sample text is, in thousandths; 0 = always the likeliest.
    pub sample_temperature_milli: u32,
    pub sample_bytes: u16,
    pub seed: u64,
    /// Finish early after this many rounds without a better check result; 0 = never.
    pub stop_when_stalled: u32,
    /// Processor threads to use; 0 = let the computer decide.
    pub threads: u8,
    /// A helper's result is refused once the run has moved this many rounds past
    /// the model the helper started from.
    pub cutoff_rounds: u32,
    /// How quickly an older result counts for less, in thousandths per round.
    pub age_decay_milli: u32,
    /// How long a helper may hold one packet before it is given to another.
    pub lease_seconds: u32,
}

fn number(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn within(name: &str, value: u64, low: u64, high: u64) -> Result<(), TrainingError> {
    if (low..=high).contains(&value) {
        return Ok(());
    }
    Err(TrainingError::InvalidSetting(format!(
        "{name} must be between {} and {}.",
        number(low),
        number(high)
    )))
}

/// Most times a preset lets a transformer read the text it learns from.
const MAX_READINGS: usize = 120;

struct PresetNumbers {
    rounds: u32,
    check_every: u32,
    stop_when_stalled: u32,
}

impl TrainingSettings {
    pub const ROUNDS: (u32, u32) = (20, 100_000);
    pub const LEARNING_RATE_MICRO: (u32, u32) = (1, 10_000_000);
    pub const OUTER_LEARNING_RATE_MICRO: (u32, u32) = (1, 1_000_000);
    pub const MAX_WEIGHT_DECAY_MICRO: u32 = 1_000_000;
    pub const MAX_CLIP_MILLI: u32 = 1_000_000;
    pub const MAX_CHECK_SHARE_PERCENT: u8 = 30;
    pub const CHECK_EVERY: (u32, u32) = (1, 10_000);
    pub const MAX_SAMPLE_TEMPERATURE_MILLI: u32 = 5_000;
    pub const SAMPLE_BYTES: (u16, u16) = (16, MAX_SAMPLE_BYTES);
    pub const MAX_STOP_WHEN_STALLED: u32 = 100_000;
    pub const MAX_THREADS: u8 = 64;
    pub const CUTOFF_ROUNDS: (u32, u32) = (1, 1024);
    pub const MAX_AGE_DECAY_MILLI: u32 = 100_000;
    pub const LEASE_SECONDS: (u32, u32) = (5, 3600);

    /// The least text one packet may carry for this shape: a transformer needs one
    /// full window and the byte after it.
    pub fn min_batch_bytes(architecture: Architecture) -> u32 {
        if architecture.is_table() {
            MIN_BATCH
        } else {
            (u32::from(architecture.context) + 1).max(MIN_BATCH)
        }
    }

    /// Checks every value against its range, naming the first one that is out.
    pub fn validate(&self, architecture: Architecture) -> Result<(), TrainingError> {
        architecture.validate()?;
        let wide = u64::from;
        within(
            "Rounds",
            wide(self.rounds),
            wide(Self::ROUNDS.0),
            wide(Self::ROUNDS.1),
        )?;
        within(
            "Text per packet",
            wide(self.batch_bytes),
            wide(Self::min_batch_bytes(architecture)),
            MAX_WORK_BATCH_BYTES as u64,
        )?;
        if architecture.is_table() {
            within("Windows per step", wide(u32::from(self.windows)), 1, 1)?;
        } else {
            within(
                "Windows per step",
                wide(u32::from(self.windows)),
                1,
                wide(u32::from(MAX_WINDOWS)),
            )?;
        }
        within(
            "Steps per round",
            wide(self.local_steps),
            1,
            wide(MAX_LOCAL_STEPS),
        )?;
        within(
            "Learning rate",
            wide(self.learning_rate_micro),
            wide(Self::LEARNING_RATE_MICRO.0),
            wide(Self::LEARNING_RATE_MICRO.1),
        )?;
        within(
            "Warm-up rounds",
            wide(self.warmup_rounds),
            0,
            wide(self.rounds),
        )?;
        within(
            "Weight decay",
            wide(self.weight_decay_micro),
            0,
            wide(Self::MAX_WEIGHT_DECAY_MICRO),
        )?;
        within(
            "Step limit",
            wide(self.clip_milli),
            0,
            wide(Self::MAX_CLIP_MILLI),
        )?;
        within(
            "Round step size",
            wide(self.outer_learning_rate_micro),
            wide(Self::OUTER_LEARNING_RATE_MICRO.0),
            wide(Self::OUTER_LEARNING_RATE_MICRO.1),
        )?;
        within(
            "Check share",
            wide(u32::from(self.check_share_percent)),
            0,
            wide(u32::from(Self::MAX_CHECK_SHARE_PERCENT)),
        )?;
        within(
            "Rounds between checks",
            wide(self.check_every),
            wide(Self::CHECK_EVERY.0),
            wide(Self::CHECK_EVERY.1),
        )?;
        within(
            "Sample variety",
            wide(self.sample_temperature_milli),
            0,
            wide(Self::MAX_SAMPLE_TEMPERATURE_MILLI),
        )?;
        within(
            "Sample length",
            wide(u32::from(self.sample_bytes)),
            wide(u32::from(Self::SAMPLE_BYTES.0)),
            wide(u32::from(Self::SAMPLE_BYTES.1)),
        )?;
        within(
            "Stop when stalled",
            wide(self.stop_when_stalled),
            0,
            wide(Self::MAX_STOP_WHEN_STALLED),
        )?;
        within(
            "Processor threads",
            wide(u32::from(self.threads)),
            0,
            wide(u32::from(Self::MAX_THREADS)),
        )?;
        within(
            "Rounds a helper may lag",
            wide(self.cutoff_rounds),
            wide(Self::CUTOFF_ROUNDS.0),
            wide(Self::CUTOFF_ROUNDS.1),
        )?;
        within(
            "Lag discount",
            wide(self.age_decay_milli),
            0,
            wide(Self::MAX_AGE_DECAY_MILLI),
        )?;
        within(
            "Seconds a helper may keep a packet",
            wide(self.lease_seconds),
            wide(Self::LEASE_SECONDS.0),
            wide(Self::LEASE_SECONDS.1),
        )
    }

    /// The recommended settings for a preset, this model shape, this much text and
    /// this computer. Always valid for `architecture`.
    pub fn recommended(
        preset: TrainingPreset,
        architecture: Architecture,
        text_len: usize,
        hardware: &HardwareInfo,
    ) -> Self {
        let table = architecture.is_table();
        let parameters = architecture.parameters();
        // How much a model must read before it is any good was measured, on a
        // 280 kB text with a round of about 512 bytes. A transformer first learns
        // which letters follow which and stays there (a check loss near 2.3) until
        // it has read enough: about a million bytes for Small, 1.6 million for
        // Medium and 3 to 4 million for Large. Then it improves quickly. Balanced
        // is sized to get past that point and Thorough to go well beyond it; a
        // Quick try only shows the run working. On an eight-core desktop
        // processor (about 45, 11 and 4 rounds a second for Small, Medium and
        // Large) a Quick try lands in roughly 10 to 30 seconds and Balanced in
        // about one, seven and thirty minutes.
        let numbers = match (preset, parameters) {
            (TrainingPreset::Quick, _) if table => (80, 10, 0),
            (TrainingPreset::Balanced, _) if table => (400, 20, 0),
            (TrainingPreset::Thorough, _) if table => (2_000, 50, 400),
            (TrainingPreset::Quick, ..=300_000) => (600, 20, 0),
            (TrainingPreset::Balanced, ..=300_000) => (2_000, 50, 0),
            (TrainingPreset::Thorough, ..=300_000) => (10_000, 100, 2_000),
            (TrainingPreset::Quick, ..=1_500_000) => (240, 20, 0),
            (TrainingPreset::Balanced, ..=1_500_000) => (3_200, 50, 0),
            (TrainingPreset::Thorough, ..=1_500_000) => (12_000, 100, 2_400),
            (TrainingPreset::Quick, _) => (120, 20, 0),
            (TrainingPreset::Balanced, _) => (6_000, 50, 0),
            (TrainingPreset::Thorough, _) => (20_000, 100, 4_000),
        };
        let mut numbers = PresetNumbers {
            rounds: numbers.0,
            check_every: numbers.1,
            stop_when_stalled: numbers.2,
        };
        // About 512 bytes of reading per step whatever the context, so a round
        // costs a similar share of the model's size on every shape.
        let context = usize::from(architecture.context);
        let mut windows = if table {
            1
        } else {
            (512 / context).clamp(1, 8)
        };
        if !table && hardware.engine == TrainingEngineKind::Gpu {
            // A graphics card reads many windows at once for little extra time
            // (measured: four times the reading for about a third more time). So
            // it reads four times as much each round and takes half the rounds:
            // twice the reading of a processor run, in a fraction of the time.
            windows *= 4;
            numbers.rounds /= 2;
            numbers.check_every /= 2;
            numbers.stop_when_stalled /= 2;
        }
        let local_steps: u32 = if table { 4 } else { 1 };
        // Room for every window of every step to be its own stretch of text (a
        // window, the byte after it, and up to three bytes to finish a character).
        let batch_bytes = if table {
            LEGACY_WORK_BATCH_BYTES
        } else {
            (windows * local_steps as usize * (context + 4))
                .clamp(context + 1, MAX_WORK_BATCH_BYTES)
        };
        // A check needs a held-back tail worth reading.
        let check_share_percent: u8 = if text_len / 10 >= 16 { 10 } else { 0 };
        let rounds = if table {
            // The Starter table sees each packet once, so long text needs enough
            // rounds to be read through.
            let passes = u32::try_from(text_len.div_ceil(LEGACY_WORK_BATCH_BYTES - 4))
                .unwrap_or(u32::MAX)
                .min(Self::ROUNDS.1);
            numbers.rounds.max(passes)
        } else {
            // A transformer that reads a short text over and over only memorises
            // it. Measured on the 2 kB sample, the check loss is at its best
            // after 60 to 150 readings and climbs after about 200, so no preset
            // reads the text more often than this.
            let learned = text_len - text_len * usize::from(check_share_percent) / 100;
            let per_round = windows * local_steps as usize * context.min(learned.max(1));
            let most = u32::try_from(MAX_READINGS * learned / per_round.max(1))
                .unwrap_or(u32::MAX)
                .clamp(Self::ROUNDS.0, Self::ROUNDS.1);
            numbers.rounds.min(most)
        };
        Self {
            rounds,
            batch_bytes: u32::try_from(batch_bytes).unwrap_or(MAX_BATCH),
            windows: u16::try_from(windows).unwrap_or(1),
            // One step a round for a transformer: measured, two steps folded
            // into one update taught no more than one step did, so the second
            // was wasted work.
            local_steps,
            learning_rate_micro: if table { 100_000 } else { 3_000 },
            schedule: if table {
                LrSchedule::Constant
            } else {
                LrSchedule::WarmupCosine
            },
            warmup_rounds: if table { 0 } else { (rounds / 20).max(5) },
            // Plain steps everywhere: the merge engine is the one Adam. A plain
            // step hands it the gradient itself, so one step a round is exactly
            // textbook Adam, and measured against the textbook trainer it lands
            // on the same check loss (Medium, 280 kB text: 1.41 both). Adaptive
            // local steps pass on only each value's direction, and measured
            // 0.06 to 0.07 worse on Small and Medium. Weight decay is off because
            // inside a plain step it is not the decoupled kind, and at these
            // sizes it measured no gain.
            optimizer: LocalOptimizer::Sgd,
            weight_decay_micro: 0,
            clip_milli: if table { 0 } else { 1_000 },
            // Measured: a larger model wants a smaller step each round.
            outer_learning_rate_micro: if table {
                250_000
            } else if parameters <= 300_000 {
                5_000
            } else if parameters <= 1_500_000 {
                3_000
            } else {
                1_500
            },
            check_share_percent,
            check_every: numbers.check_every,
            sample_temperature_milli: 800,
            sample_bytes: 200,
            seed: 42,
            stop_when_stalled: if check_share_percent == 0 {
                0
            } else {
                numbers.stop_when_stalled
            },
            threads: 0,
            cutoff_rounds: 128,
            age_decay_milli: 20,
            lease_seconds: 120,
        }
    }

    /// The schedule's share of the full step size for the round about to run
    /// (counted from zero), in thousandths: 1..=1000.
    pub fn schedule_milli(&self, round: u32) -> u32 {
        let cosine = |done: u32, of: u32| -> u32 {
            if of == 0 {
                return 1000;
            }
            let progress = f64::from(done.min(of)) / f64::from(of);
            let share = 0.1 + 0.9 * f64::midpoint(1.0, (std::f64::consts::PI * progress).cos());
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a share between 0.1 and 1.0 times a thousand fits u32"
            )]
            let milli = (share * 1000.0).round() as u32;
            milli.clamp(100, 1000)
        };
        match self.schedule {
            LrSchedule::Constant => 1000,
            LrSchedule::Cosine => cosine(round, self.rounds),
            LrSchedule::WarmupCosine if round < self.warmup_rounds => {
                let share = u64::from(round + 1) * 1000 / u64::from(self.warmup_rounds);
                u32::try_from(share).unwrap_or(1000).clamp(1, 1000)
            }
            LrSchedule::WarmupCosine => cosine(
                round - self.warmup_rounds,
                self.rounds.saturating_sub(self.warmup_rounds),
            ),
        }
    }

    fn scaled(value: u32, milli: u32) -> u32 {
        u32::try_from(u64::from(value) * u64::from(milli) / 1000)
            .unwrap_or(u32::MAX)
            .max(1)
    }

    /// The round step size for the round about to run, in millionths.
    pub fn outer_learning_rate_micro_at(&self, round: u32) -> u32 {
        Self::scaled(self.outer_learning_rate_micro, self.schedule_milli(round))
    }

    /// The same, as the fraction the merge engine takes
    /// (`GridSyncer::set_learning_rate`).
    pub fn outer_learning_rate_at(&self, round: u32) -> f64 {
        f64::from(self.outer_learning_rate_micro_at(round)) / 1e6
    }

    /// Next-byte guesses one packet of `text_len` bytes teaches: windows × bytes
    /// per window × steps. Both ends of a work packet compute this same number.
    pub fn work_tokens(&self, architecture: Architecture, text_len: usize) -> u64 {
        u64::from(self.windows)
            * architecture.window_tokens(text_len) as u64
            * u64::from(self.local_steps)
    }

    /// Guesses the whole run teaches when every packet is full.
    pub fn total_tokens(&self, architecture: Architecture) -> u64 {
        self.work_tokens(architecture, self.batch_bytes as usize)
            .saturating_mul(u64::from(self.rounds))
    }

    /// The merge engine's settings for this run. The step size is the first
    /// round's; set it again before each later merge.
    pub fn grid_settings(&self, architecture: Architecture) -> GridSettings {
        GridSettings {
            learning_rate: self.outer_learning_rate_at(0),
            age_decay: f64::from(self.age_decay_milli) / 1000.0,
            cutoff_rounds: self.cutoff_rounds,
            local_steps: self.local_steps,
            token_budget: u32::try_from(self.work_tokens(architecture, self.batch_bytes as usize))
                .unwrap_or(u32::MAX)
                .max(1),
            lease_ms: self.lease_seconds.saturating_mul(1000),
            ..GridSettings::default()
        }
    }

    /// Where the text divides into the part to learn from and the tail kept back
    /// for checking. Returns `text.len()` (no check) when the share is zero or
    /// either part would be too short to read.
    pub fn check_split(&self, text: &str) -> usize {
        let tail = text.len() * usize::from(self.check_share_percent) / 100;
        let mut split = text.len() - tail;
        while !text.is_char_boundary(split) {
            split -= 1;
        }
        if text.len() - split < 2 || split < MIN_TEXT_BYTES.min(text.len()) {
            text.len()
        } else {
            split
        }
    }

    /// Whether these settings are ones an old Starter helper already uses, so its
    /// packets can stay in the first wire form.
    pub fn is_legacy_starter(&self, architecture: Architecture) -> bool {
        architecture.is_table()
            && self.batch_bytes as usize <= LEGACY_WORK_BATCH_BYTES
            && self.optimizer == LocalOptimizer::Sgd
            && self.schedule == LrSchedule::Constant
            && self.learning_rate_micro == 100_000
            && self.weight_decay_micro == 0
            && self.clip_milli == 0
    }
}

/// Where a run's model starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingBase {
    /// A new, untrained model of this size.
    Fresh(BaseModel),
    /// A model already in the library, trained further. It keeps its shape.
    Continue(ModelId),
}

impl TrainingBase {
    /// The shape, when it is known without opening a library file.
    pub fn architecture(&self) -> Option<Architecture> {
        match self {
            Self::Fresh(model) => Some(model.architecture()),
            Self::Continue(_) => None,
        }
    }
}

/// The text a run learns from. Always an explicit choice: never chats, memories,
/// folders or shared notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainingSource {
    Sample,
    Text(String),
    /// A plain-text file the person picked. Core only checks the name; the runtime
    /// reads it, bounded, and turns it into text.
    File(PathBuf),
}

/// Checks that `text` is usable for training: 32 bytes to 4 MiB, not blank.
pub fn check_text(text: &str) -> Result<(), TrainingError> {
    if (MIN_TEXT_BYTES..=MAX_TEXT_BYTES).contains(&text.len()) && !text.trim().is_empty() {
        Ok(())
    } else {
        Err(TrainingError::InvalidInput)
    }
}

/// Turns a file's bytes into training text: plain UTF-8 only, a leading byte-order
/// mark removed, then the same bounds as pasted text. The runtime reads at most
/// one byte past the limit, so an over-long file fails here without being loaded.
pub fn text_from_file_bytes(bytes: Vec<u8>) -> Result<String, TrainingError> {
    const MARK: &[u8] = "\u{feff}".as_bytes();
    if bytes.len() > MAX_TEXT_BYTES + MARK.len() {
        return Err(TrainingError::InvalidInput);
    }
    let mut text = String::from_utf8(bytes).map_err(|_| {
        TrainingError::InvalidFile(
            "That file is not plain text. Choose a .txt or .md file saved as UTF-8.".into(),
        )
    })?;
    if text.as_bytes().starts_with(MARK) {
        text.drain(..MARK.len());
    }
    check_text(&text)?;
    Ok(text)
}

impl TrainingSource {
    /// The text itself when it is already in hand; `None` for a file, which the
    /// runtime has to read first.
    pub fn text(&self) -> Result<Option<&str>, TrainingError> {
        self.validate()?;
        Ok(match self {
            Self::Sample => Some(SAMPLE_TEXT),
            Self::Text(text) => Some(text),
            Self::File(_) => None,
        })
    }

    pub fn validate(&self) -> Result<(), TrainingError> {
        match self {
            Self::Sample => Ok(()),
            Self::Text(text) => check_text(text),
            Self::File(path) => {
                let named = path.file_name().is_some();
                let plain = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        ["txt", "md"]
                            .iter()
                            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                    });
                if named && plain {
                    Ok(())
                } else {
                    Err(TrainingError::InvalidFile(
                        "Choose a plain text file ending in .txt or .md.".into(),
                    ))
                }
            }
        }
    }

    /// The chosen file's name without its folders, cut to a displayable length.
    pub fn file_name(&self) -> Option<String> {
        match self {
            Self::Sample | Self::Text(_) => None,
            Self::File(path) => display_name(path),
        }
    }

    /// What the state shows about the source. It never carries the text.
    pub fn summary(&self, bytes: usize) -> SourceSummary {
        SourceSummary {
            kind: match self {
                Self::Sample => SourceKind::Sample,
                Self::Text(_) => SourceKind::Text,
                Self::File(_) => SourceKind::File,
            },
            bytes: bytes as u64,
            file_name: self.file_name(),
        }
    }
}

fn display_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    Some(
        name.chars()
            .filter(|c| !c.is_control())
            .take(MAX_FILE_NAME_CHARS)
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Sample,
    Text,
    File,
}

/// Which text a run uses, without the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSummary {
    pub kind: SourceKind,
    pub bytes: u64,
    pub file_name: Option<String>,
}

/// Everything a person chose for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingPlan {
    /// The finished model's name: 1 to 60 characters.
    pub name: String,
    pub base: TrainingBase,
    pub source: TrainingSource,
    /// `None` once any setting differs from a preset.
    pub preset: Option<TrainingPreset>,
    pub settings: TrainingSettings,
    /// Let the group's computers help with this text, and give everyone in the
    /// group the finished model. Only this explicit choice shares the text.
    pub with_group: bool,
}

/// Checks a model name: 1 to 60 characters, no line breaks or control characters.
pub fn check_name(name: &str) -> Result<(), TrainingError> {
    let length = name.chars().count();
    if name.trim().is_empty()
        || name != name.trim()
        || length > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(TrainingError::InvalidSetting(format!(
            "Give the model a name of 1 to {MAX_NAME_CHARS} characters."
        )));
    }
    Ok(())
}

impl TrainingPlan {
    /// What the page starts with: the sample, the model this computer is
    /// comfortable with, and Balanced. Pressing Start is all that is left to do.
    pub fn recommended(hardware: &HardwareInfo, with_group: bool) -> Self {
        let model = hardware.recommended_model();
        let preset = TrainingPreset::Balanced;
        Self {
            name: format!("My {} model", model.info().title),
            base: TrainingBase::Fresh(model),
            source: TrainingSource::Sample,
            preset: Some(preset),
            settings: TrainingSettings::recommended(
                preset,
                model.architecture(),
                SAMPLE_TEXT.len(),
                hardware,
            ),
            with_group: with_group && model.info().group_capable,
        }
    }

    /// Checks the whole plan for a model of this shape (the starting point's own
    /// shape, or the library model's when continuing).
    pub fn validate(&self, architecture: Architecture) -> Result<(), TrainingError> {
        check_name(&self.name)?;
        if self
            .base
            .architecture()
            .is_some_and(|expected| expected != architecture)
        {
            return Err(TrainingError::InvalidModel);
        }
        self.source.validate()?;
        self.settings.validate(architecture)?;
        if self.with_group && !architecture.group_capable() {
            return Err(TrainingError::InvalidSetting(
                "This model is too large for the group's computers to help train. Turn off Train with my group, or choose a smaller model. Your group can still be given the finished model.".into(),
            ));
        }
        Ok(())
    }

    /// The plan as the state shows it, without the text.
    pub fn summary(&self, architecture: Architecture, text_len: usize) -> TrainingPlanSummary {
        TrainingPlanSummary {
            name: self.name.clone(),
            base: self.base,
            architecture,
            source: self.source.summary(text_len),
            preset: self.preset,
            settings: self.settings.clone(),
            with_group: self.with_group,
        }
    }
}

/// The active or last run's plan, safe to copy into every state snapshot: it holds
/// the kind and size of the text, never the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingPlanSummary {
    pub name: String,
    pub base: TrainingBase,
    pub architecture: Architecture,
    pub source: SourceSummary,
    pub preset: Option<TrainingPreset>,
    pub settings: TrainingSettings,
    pub with_group: bool,
}

impl TrainingPlanSummary {
    /// Checks a summary read back from a saved run.
    pub fn validate(&self) -> Result<(), TrainingError> {
        check_name(&self.name)?;
        self.settings.validate(self.architecture)?;
        let file_name_ok = self.source.file_name.as_ref().is_none_or(|name| {
            name.chars().count() <= MAX_FILE_NAME_CHARS && !name.chars().any(char::is_control)
        });
        if !file_name_ok
            || self.source.bytes > MAX_TEXT_BYTES as u64
            || self
                .base
                .architecture()
                .is_some_and(|expected| expected != self.architecture)
            || (self.with_group && !self.architecture.group_capable())
        {
            return Err(TrainingError::InvalidCheckpoint);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
