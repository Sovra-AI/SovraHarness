//! The expert number fields: which setting each one edits, how its value is
//! shown and typed, and the plain-word message when a typed value is wrong.

use sovra_core::training::{
    Architecture, LocalOptimizer, LrSchedule, MAX_LOCAL_STEPS, MAX_WINDOWS, MAX_WORK_BATCH_BYTES,
    TrainingSettings,
};

use super::words;

/// One expert number field. Each maps to exactly one `TrainingSettings` value;
/// the two settings that are a choice of names are buttons instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Knob {
    Rounds,
    BatchBytes,
    Windows,
    LocalSteps,
    OuterLearningRate,
    WarmupRounds,
    LearningRate,
    WeightDecay,
    Clip,
    CheckShare,
    CheckEvery,
    StopWhenStalled,
    SampleTemperature,
    SampleBytes,
    Seed,
    Threads,
    CutoffRounds,
    AgeDecay,
    LeaseSeconds,
}

/// Headings the expert settings are grouped under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Amount,
    Steps,
    Checking,
    Computer,
    Helpers,
}

impl Section {
    pub(super) const ALL: [Self; 5] = [
        Self::Amount,
        Self::Steps,
        Self::Checking,
        Self::Computer,
        Self::Helpers,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Amount => "How much training",
            Self::Steps => "Step sizes",
            Self::Checking => "Checking and sample text",
            Self::Computer => "This computer",
            Self::Helpers => "Group helpers",
        }
    }
}

const THOUSANDTHS: u64 = 1_000;
const MILLIONTHS: u64 = 1_000_000;

impl Knob {
    pub(super) const ALL: [Self; 19] = [
        Self::Rounds,
        Self::BatchBytes,
        Self::Windows,
        Self::LocalSteps,
        Self::OuterLearningRate,
        Self::WarmupRounds,
        Self::LearningRate,
        Self::WeightDecay,
        Self::Clip,
        Self::CheckShare,
        Self::CheckEvery,
        Self::StopWhenStalled,
        Self::SampleTemperature,
        Self::SampleBytes,
        Self::Seed,
        Self::Threads,
        Self::CutoffRounds,
        Self::AgeDecay,
        Self::LeaseSeconds,
    ];

    /// Position in [`Self::ALL`], which is also the order of the view's inputs.
    pub(super) fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|knob| *knob == self)
            .unwrap_or_default()
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Rounds => "training-setting-rounds",
            Self::BatchBytes => "training-setting-packet",
            Self::Windows => "training-setting-windows",
            Self::LocalSteps => "training-setting-steps",
            Self::OuterLearningRate => "training-setting-round-step",
            Self::WarmupRounds => "training-setting-warmup",
            Self::LearningRate => "training-setting-learning-rate",
            Self::WeightDecay => "training-setting-weight-decay",
            Self::Clip => "training-setting-step-limit",
            Self::CheckShare => "training-setting-check-share",
            Self::CheckEvery => "training-setting-check-every",
            Self::StopWhenStalled => "training-setting-stalled",
            Self::SampleTemperature => "training-setting-variety",
            Self::SampleBytes => "training-setting-sample-length",
            Self::Seed => "training-setting-seed",
            Self::Threads => "training-setting-threads",
            Self::CutoffRounds => "training-setting-lag",
            Self::AgeDecay => "training-setting-lag-discount",
            Self::LeaseSeconds => "training-setting-lease",
        }
    }

    /// The same names the domain uses in its own range messages.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Rounds => "Rounds",
            Self::BatchBytes => "Text per packet",
            Self::Windows => "Windows per step",
            Self::LocalSteps => "Steps per round",
            Self::OuterLearningRate => "Round step size",
            Self::WarmupRounds => "Warm-up rounds",
            Self::LearningRate => "Learning rate",
            Self::WeightDecay => "Weight decay",
            Self::Clip => "Step limit",
            Self::CheckShare => "Check share",
            Self::CheckEvery => "Rounds between checks",
            Self::StopWhenStalled => "Stop when stalled",
            Self::SampleTemperature => "Sample variety",
            Self::SampleBytes => "Sample length",
            Self::Seed => "Seed",
            Self::Threads => "Processor threads",
            Self::CutoffRounds => "Rounds a helper may lag",
            Self::AgeDecay => "Lag discount",
            Self::LeaseSeconds => "Seconds a helper may keep a packet",
        }
    }

    pub(super) fn hint(self) -> &'static str {
        match self {
            Self::Rounds => {
                "How many times the model is improved. More rounds learn more and take longer."
            }
            Self::BatchBytes => "Bytes of your text handed out in each round.",
            Self::Windows => {
                "Stretches of text read from the packet in each step. The Starter model always uses 1."
            }
            Self::LocalSteps => "Learning steps taken on a packet before its result is folded in.",
            Self::OuterLearningRate => {
                "How far the model moves each round. This is the step size that matters: lower is steadier, higher is quicker and riskier."
            }
            Self::WarmupRounds => {
                "Rounds spent easing the step size up at the start. Used by Warm up, then ease down."
            }
            Self::LearningRate => {
                "Step size inside one packet. It mostly sets the direction, so changing it does little."
            }
            Self::WeightDecay => {
                "Gently pulls values towards zero so the model does not just memorise. 0 turns it off."
            }
            Self::Clip => "Largest correction one step may make. 0 means no limit.",
            Self::CheckShare => {
                "Percent of the end of your text kept back to check the model on text it has not learned from. 0 means no check."
            }
            Self::CheckEvery => "Rounds between checks and fresh sample text.",
            Self::StopWhenStalled => {
                "Finish early after this many rounds without a better check result. 0 means never."
            }
            Self::SampleTemperature => {
                "How adventurous the sample text is. 0 always picks the likeliest letter."
            }
            Self::SampleBytes => "Bytes of sample text written each time.",
            Self::Seed => {
                "Starting number for the random choices. The same seed and text give the same run."
            }
            Self::Threads => {
                "Processor threads to use. 0 lets the computer decide. More rarely helps models this small."
            }
            Self::CutoffRounds => {
                "A helper's result is turned away once the run has moved this many rounds past where the helper started."
            }
            Self::AgeDecay => {
                "How quickly an older result counts for less, for each round it is behind."
            }
            Self::LeaseSeconds => {
                "How long a helper may hold a packet before it is given to another computer."
            }
        }
    }

    pub(super) fn section(self) -> Section {
        match self {
            Self::Rounds | Self::BatchBytes | Self::Windows | Self::LocalSteps => Section::Amount,
            Self::OuterLearningRate
            | Self::WarmupRounds
            | Self::LearningRate
            | Self::WeightDecay
            | Self::Clip => Section::Steps,
            Self::CheckShare
            | Self::CheckEvery
            | Self::StopWhenStalled
            | Self::SampleTemperature
            | Self::SampleBytes
            | Self::Seed => Section::Checking,
            Self::Threads => Section::Computer,
            Self::CutoffRounds | Self::AgeDecay | Self::LeaseSeconds => Section::Helpers,
        }
    }

    /// How many stored units make one shown unit: a stored 3,000 millionths is
    /// shown, and typed, as 0.003.
    fn scale(self) -> u64 {
        match self {
            Self::OuterLearningRate | Self::LearningRate | Self::WeightDecay => MILLIONTHS,
            Self::Clip | Self::SampleTemperature | Self::AgeDecay => THOUSANDTHS,
            Self::Rounds
            | Self::BatchBytes
            | Self::Windows
            | Self::LocalSteps
            | Self::WarmupRounds
            | Self::CheckShare
            | Self::CheckEvery
            | Self::StopWhenStalled
            | Self::SampleBytes
            | Self::Seed
            | Self::Threads
            | Self::CutoffRounds
            | Self::LeaseSeconds => 1,
        }
    }

    /// What one press of plus or minus changes, in shown units.
    pub(super) fn step(self) -> f64 {
        match self {
            Self::Rounds | Self::LeaseSeconds => 10.,
            Self::BatchBytes => 64.,
            Self::Windows | Self::LocalSteps | Self::Seed | Self::Threads => 1.,
            Self::OuterLearningRate | Self::LearningRate => 0.001,
            Self::WarmupRounds | Self::CheckShare | Self::CheckEvery => 5.,
            Self::WeightDecay | Self::AgeDecay => 0.01,
            Self::Clip | Self::SampleTemperature => 0.1,
            Self::StopWhenStalled => 50.,
            Self::SampleBytes => 16.,
            Self::CutoffRounds => 8.,
        }
    }

    fn get(self, settings: &TrainingSettings) -> u64 {
        match self {
            Self::Rounds => settings.rounds.into(),
            Self::BatchBytes => settings.batch_bytes.into(),
            Self::Windows => settings.windows.into(),
            Self::LocalSteps => settings.local_steps.into(),
            Self::OuterLearningRate => settings.outer_learning_rate_micro.into(),
            Self::WarmupRounds => settings.warmup_rounds.into(),
            Self::LearningRate => settings.learning_rate_micro.into(),
            Self::WeightDecay => settings.weight_decay_micro.into(),
            Self::Clip => settings.clip_milli.into(),
            Self::CheckShare => settings.check_share_percent.into(),
            Self::CheckEvery => settings.check_every.into(),
            Self::StopWhenStalled => settings.stop_when_stalled.into(),
            Self::SampleTemperature => settings.sample_temperature_milli.into(),
            Self::SampleBytes => settings.sample_bytes.into(),
            Self::Seed => settings.seed,
            Self::Threads => settings.threads.into(),
            Self::CutoffRounds => settings.cutoff_rounds.into(),
            Self::AgeDecay => settings.age_decay_milli.into(),
            Self::LeaseSeconds => settings.lease_seconds.into(),
        }
    }

    /// Stores a value already checked against [`Self::bounds`], so it fits.
    fn set(self, settings: &mut TrainingSettings, value: u64) {
        let narrow = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
        let small = |value: u64| u16::try_from(value).unwrap_or(u16::MAX);
        let tiny = |value: u64| u8::try_from(value).unwrap_or(u8::MAX);
        match self {
            Self::Rounds => settings.rounds = narrow(value),
            Self::BatchBytes => settings.batch_bytes = narrow(value),
            Self::Windows => settings.windows = small(value),
            Self::LocalSteps => settings.local_steps = narrow(value),
            Self::OuterLearningRate => settings.outer_learning_rate_micro = narrow(value),
            Self::WarmupRounds => settings.warmup_rounds = narrow(value),
            Self::LearningRate => settings.learning_rate_micro = narrow(value),
            Self::WeightDecay => settings.weight_decay_micro = narrow(value),
            Self::Clip => settings.clip_milli = narrow(value),
            Self::CheckShare => settings.check_share_percent = tiny(value),
            Self::CheckEvery => settings.check_every = narrow(value),
            Self::StopWhenStalled => settings.stop_when_stalled = narrow(value),
            Self::SampleTemperature => settings.sample_temperature_milli = narrow(value),
            Self::SampleBytes => settings.sample_bytes = small(value),
            Self::Seed => settings.seed = value,
            Self::Threads => settings.threads = tiny(value),
            Self::CutoffRounds => settings.cutoff_rounds = narrow(value),
            Self::AgeDecay => settings.age_decay_milli = narrow(value),
            Self::LeaseSeconds => settings.lease_seconds = narrow(value),
        }
    }

    /// The stored range for this model shape. The warm-up's upper end also
    /// depends on the rounds typed beside it; the domain's own check covers that.
    fn bounds(self, architecture: Architecture) -> (u64, u64) {
        let pair = |(low, high): (u32, u32)| (u64::from(low), u64::from(high));
        match self {
            Self::Rounds | Self::WarmupRounds => {
                let (low, high) = pair(TrainingSettings::ROUNDS);
                if self == Self::Rounds {
                    (low, high)
                } else {
                    (0, high)
                }
            }
            Self::BatchBytes => (
                TrainingSettings::min_batch_bytes(architecture).into(),
                MAX_WORK_BATCH_BYTES as u64,
            ),
            Self::Windows if architecture.is_table() => (1, 1),
            Self::Windows => (1, MAX_WINDOWS.into()),
            Self::LocalSteps => (1, MAX_LOCAL_STEPS.into()),
            Self::OuterLearningRate => pair(TrainingSettings::OUTER_LEARNING_RATE_MICRO),
            Self::LearningRate => pair(TrainingSettings::LEARNING_RATE_MICRO),
            Self::WeightDecay => (0, TrainingSettings::MAX_WEIGHT_DECAY_MICRO.into()),
            Self::Clip => (0, TrainingSettings::MAX_CLIP_MILLI.into()),
            Self::CheckShare => (0, TrainingSettings::MAX_CHECK_SHARE_PERCENT.into()),
            Self::CheckEvery => pair(TrainingSettings::CHECK_EVERY),
            Self::StopWhenStalled => (0, TrainingSettings::MAX_STOP_WHEN_STALLED.into()),
            Self::SampleTemperature => (0, TrainingSettings::MAX_SAMPLE_TEMPERATURE_MILLI.into()),
            Self::SampleBytes => (
                TrainingSettings::SAMPLE_BYTES.0.into(),
                TrainingSettings::SAMPLE_BYTES.1.into(),
            ),
            Self::Seed => (0, u64::MAX),
            Self::Threads => (0, TrainingSettings::MAX_THREADS.into()),
            Self::CutoffRounds => pair(TrainingSettings::CUTOFF_ROUNDS),
            Self::AgeDecay => (0, TrainingSettings::MAX_AGE_DECAY_MILLI.into()),
            Self::LeaseSeconds => pair(TrainingSettings::LEASE_SECONDS),
        }
    }

    /// The text an input shows for this setting.
    pub(super) fn shown(self, settings: &TrainingSettings) -> String {
        show(self.get(settings), self.scale(), false)
    }
}

/// A stored value in shown units: 3,000 millionths is `0.003`. `grouped` adds
/// thousands separators for messages; inputs show plain digits.
fn show(value: u64, scale: u64, grouped: bool) -> String {
    let whole = value / scale;
    let whole = if grouped {
        words::count(whole)
    } else {
        whole.to_string()
    };
    let fraction = value % scale;
    if fraction == 0 {
        return whole;
    }
    let width = scale.ilog10() as usize;
    let digits = format!("{fraction:0width$}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

/// Reads typed digits, with an optional decimal part no finer than the scale,
/// into stored units. Anything else, including a minus sign, is `None`.
fn read(text: &str, scale: u64) -> Option<u64> {
    let text = text.trim();
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let places = scale.ilog10() as usize;
    if (whole.is_empty() && fraction.is_empty())
        || !digits(whole)
        || !digits(fraction)
        || fraction.len() > places
    {
        return None;
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<places$}").parse().ok()?
    };
    whole.checked_mul(scale)?.checked_add(fraction)
}

/// Turns the typed expert fields (in [`Knob::ALL`] order) into settings, or says
/// in plain words which field is wrong.
pub(super) fn read_settings(
    texts: &[String],
    schedule: LrSchedule,
    optimizer: LocalOptimizer,
    seed: &TrainingSettings,
    architecture: Architecture,
) -> Result<TrainingSettings, String> {
    let mut settings = seed.clone();
    settings.schedule = schedule;
    settings.optimizer = optimizer;
    for knob in Knob::ALL {
        let text = texts.get(knob.index()).map_or("", String::as_str);
        let scale = knob.scale();
        let (low, high) = knob.bounds(architecture);
        let range = || {
            format!(
                "{} must be between {} and {}.",
                knob.label(),
                show(low, scale, true),
                show(high, scale, true)
            )
        };
        let Some(value) = read(text, scale) else {
            return Err(if text.trim().is_empty() {
                format!("{} needs a number.", knob.label())
            } else if text.trim().starts_with('-') {
                range()
            } else {
                format!(
                    "{} needs a number such as {}.",
                    knob.label(),
                    knob.shown(seed)
                )
            });
        };
        if !(low..=high).contains(&value) {
            return Err(range());
        }
        knob.set(&mut settings, value);
    }
    settings
        .validate(architecture)
        .map_err(|error| error.to_string())?;
    Ok(settings)
}

#[cfg(test)]
mod tests;
