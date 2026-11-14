//! What this computer has to train with, as plain data, and what that is good for.
//!
//! Probing is the engine adapter's job. Nothing here reads the machine.

use serde::{Deserialize, Serialize};

use super::{Architecture, BASE_MODELS, BaseModel, TrainingSettings};

/// Longest text kept for any one hardware fact.
pub const MAX_HARDWARE_TEXT_CHARS: usize = 120;
/// Most graphics cards listed.
pub const MAX_GPUS: usize = 8;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingEngineKind {
    /// The built-in processor engine. Always available.
    #[default]
    Cpu,
    /// A graphics card, in builds that include the graphics engine.
    Gpu,
}

/// Facts about this computer, for the "This computer" card. A zero count or size
/// means that fact could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareInfo {
    pub os: String,
    pub arch: String,
    pub cpu: String,
    pub physical_cores: u16,
    pub logical_cores: u16,
    pub memory_total_bytes: u64,
    /// Memory free for use right now. Refreshed when the page loads.
    pub memory_available_bytes: u64,
    pub engine: TrainingEngineKind,
    pub engine_name: String,
    /// Graphics cards the engine can use. Empty in the standard build.
    pub gpus: Vec<String>,
}

impl HardwareInfo {
    /// The same facts with every text and list cut to its bound, so a snapshot of
    /// the state stays small whatever the operating system reports.
    #[must_use]
    pub fn bounded(mut self) -> Self {
        let cut = |text: &mut String| {
            let trimmed: String = text.trim().chars().take(MAX_HARDWARE_TEXT_CHARS).collect();
            *text = trimmed;
        };
        cut(&mut self.os);
        cut(&mut self.arch);
        cut(&mut self.cpu);
        cut(&mut self.engine_name);
        self.gpus.truncate(MAX_GPUS);
        self.gpus.iter_mut().for_each(cut);
        self
    }

    /// Whether this computer has the memory and processor threads the model's row
    /// asks for. With unknown memory or threads only Starter and Small count.
    pub fn comfortable(&self, model: BaseModel) -> bool {
        let row = model.info();
        if self.memory_total_bytes == 0 || self.logical_cores == 0 {
            return matches!(model, BaseModel::Starter | BaseModel::Small);
        }
        self.memory_total_bytes >= row.min_memory_bytes
            && self.logical_cores >= row.comfortable_cores
    }

    pub fn comfortable_models(&self) -> Vec<BaseModel> {
        BASE_MODELS
            .iter()
            .map(|row| row.model)
            .filter(|model| self.comfortable(*model))
            .collect()
    }

    /// The starting point preselected for a first run. It favours seeing results
    /// soon: Medium only on a well-equipped computer, Large only with a graphics
    /// engine, and never a model this computer is not comfortable with.
    pub fn recommended_model(&self) -> BaseModel {
        let roomy = self.logical_cores >= 8 && self.memory_total_bytes >= 8 * GIB;
        let wanted = match self.engine {
            TrainingEngineKind::Gpu => BaseModel::Large,
            TrainingEngineKind::Cpu if roomy => BaseModel::Medium,
            TrainingEngineKind::Cpu => BaseModel::Small,
        };
        self.comfortable_models()
            .into_iter()
            .filter(|model| *model <= wanted)
            .max()
            .unwrap_or(BaseModel::Starter)
    }

    /// One plain sentence for the hardware card.
    pub fn recommendation(&self) -> String {
        let titles: Vec<&str> = self
            .comfortable_models()
            .into_iter()
            .filter(|model| *model != BaseModel::Starter)
            .map(|model| model.info().title)
            .collect();
        match titles.as_slice() {
            [] => "This computer is best with the Starter model.".to_owned(),
            [one] => format!("This computer is comfortable with the {one} model."),
            [first, second] => {
                format!("This computer is comfortable with {first} and {second} models.")
            }
            [first, middle @ .., last] => format!(
                "This computer is comfortable with {first}, {} and {last} models.",
                middle.join(", ")
            ),
        }
    }

    /// A rough guess at how long a run takes here, before any speed has been
    /// measured. It assumes a fixed amount of arithmetic per processor core,
    /// matched to measured runs on an eight-core desktop processor, and is only
    /// good for "seconds, minutes or hours"; once a run is going, the measured
    /// speed replaces it.
    pub fn rough_duration_ms(
        &self,
        architecture: Architecture,
        settings: &TrainingSettings,
    ) -> u64 {
        // Arithmetic one core gets through each millisecond, and how much more a
        // few cores or a graphics card manage (far less than proportional).
        const WORK_PER_CORE_MS: u64 = 10_000_000;
        let speedup_tenths: u64 = match (self.engine, self.physical_cores.max(1)) {
            (TrainingEngineKind::Gpu, _) => 160,
            (TrainingEngineKind::Cpu, 1) => 10,
            (TrainingEngineKind::Cpu, 2..=3) => 16,
            (TrainingEngineKind::Cpu, 4..=7) => 26,
            (TrainingEngineKind::Cpu, _) => 40,
        };
        let parameters = architecture.parameters() as u64;
        // Reading and correcting one byte touches every value a few times over,
        // plus a fixed cost that matters for the smallest models. The table is
        // small, but each of its steps still corrects the whole table.
        let per_token = if architecture.is_table() {
            150_000
        } else {
            6 * parameters + 500_000
        };
        let tokens = settings.total_tokens(architecture);
        // Merging a round copies and hashes the whole model whatever the text.
        let per_round = 40 * parameters;
        let work = tokens
            .saturating_mul(per_token)
            .saturating_add(u64::from(settings.rounds).saturating_mul(per_round));
        work.saturating_mul(10) / (WORK_PER_CORE_MS * speedup_tenths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::training::TrainingPreset;

    fn computer(cores: u16, memory_gib: u64) -> HardwareInfo {
        HardwareInfo {
            os: "Linux 6".into(),
            arch: "x86_64".into(),
            cpu: "Example Processor".into(),
            physical_cores: cores.div_ceil(2),
            logical_cores: cores,
            memory_total_bytes: memory_gib * GIB,
            memory_available_bytes: memory_gib * GIB / 2,
            engine: TrainingEngineKind::Cpu,
            engine_name: "Built-in processor engine".into(),
            gpus: Vec::new(),
        }
    }

    #[test]
    fn comfort_follows_memory_and_cores_and_is_cautious_when_unknown() {
        use BaseModel::{Large, Medium, Small, Starter};
        assert_eq!(computer(1, 1).comfortable_models(), [Starter]);
        assert_eq!(computer(2, 1).comfortable_models(), [Starter, Small]);
        assert_eq!(computer(4, 1).comfortable_models(), [Starter, Small]);
        assert_eq!(
            computer(4, 2).comfortable_models(),
            [Starter, Small, Medium]
        );
        assert_eq!(
            computer(16, 32).comfortable_models(),
            [Starter, Small, Medium, Large]
        );
        assert_eq!(
            HardwareInfo::default().comfortable_models(),
            [Starter, Small]
        );
        let no_memory = HardwareInfo {
            memory_total_bytes: 0,
            ..computer(16, 32)
        };
        assert_eq!(no_memory.comfortable_models(), [Starter, Small]);
    }

    #[test]
    fn the_recommended_model_is_comfortable_and_favours_quick_results() {
        assert_eq!(computer(1, 1).recommended_model(), BaseModel::Starter);
        assert_eq!(computer(2, 2).recommended_model(), BaseModel::Small);
        assert_eq!(computer(4, 4).recommended_model(), BaseModel::Small);
        assert_eq!(computer(8, 8).recommended_model(), BaseModel::Medium);
        assert_eq!(computer(32, 64).recommended_model(), BaseModel::Medium);
        assert_eq!(
            HardwareInfo::default().recommended_model(),
            BaseModel::Small
        );
        let gpu = HardwareInfo {
            engine: TrainingEngineKind::Gpu,
            gpus: vec!["Example Graphics".into()],
            ..computer(16, 32)
        };
        assert_eq!(gpu.recommended_model(), BaseModel::Large);
        let small_gpu = HardwareInfo {
            engine: TrainingEngineKind::Gpu,
            ..computer(4, 2)
        };
        assert_eq!(small_gpu.recommended_model(), BaseModel::Medium);
        for cores in [0, 1, 2, 4, 8, 64] {
            for memory in [0, 1, 2, 4, 8, 64] {
                let info = computer(cores, memory);
                assert!(info.comfortable(info.recommended_model()));
            }
        }
    }

    #[test]
    fn the_recommendation_is_one_plain_sentence() {
        assert_eq!(
            computer(1, 1).recommendation(),
            "This computer is best with the Starter model."
        );
        assert_eq!(
            computer(2, 1).recommendation(),
            "This computer is comfortable with the Small model."
        );
        assert_eq!(
            computer(4, 4).recommendation(),
            "This computer is comfortable with Small and Medium models."
        );
        assert_eq!(
            computer(16, 32).recommendation(),
            "This computer is comfortable with Small, Medium and Large models."
        );
    }

    #[test]
    fn facts_are_cut_to_their_bounds() {
        let info = HardwareInfo {
            cpu: format!("  {}  ", "é".repeat(500)),
            gpus: vec!["g".repeat(500); 20],
            ..computer(4, 4)
        }
        .bounded();
        assert_eq!(info.cpu.chars().count(), MAX_HARDWARE_TEXT_CHARS);
        assert_eq!(info.gpus.len(), MAX_GPUS);
        assert!(
            info.gpus
                .iter()
                .all(|gpu| gpu.len() == MAX_HARDWARE_TEXT_CHARS)
        );
        assert_eq!(info.os, "Linux 6");
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(serde_json::from_str::<HardwareInfo>(&json).unwrap(), info);
    }

    #[test]
    fn the_rough_duration_grows_with_the_model_and_shrinks_with_the_computer() {
        let laptop = computer(8, 16);
        let estimate = |info: &HardwareInfo, model: BaseModel, preset| {
            let architecture = model.architecture();
            let settings = TrainingSettings::recommended(preset, architecture, 3000, info);
            info.rough_duration_ms(architecture, &settings)
        };
        let quick_small = estimate(&laptop, BaseModel::Small, TrainingPreset::Quick);
        // A quick try of Small is meant to take seconds, not minutes.
        assert!((1_000..120_000).contains(&quick_small), "{quick_small}");
        assert!(estimate(&laptop, BaseModel::Starter, TrainingPreset::Quick) < quick_small);
        assert!(estimate(&laptop, BaseModel::Medium, TrainingPreset::Quick) > quick_small);
        assert!(estimate(&laptop, BaseModel::Small, TrainingPreset::Thorough) > quick_small);
        assert!(estimate(&computer(2, 16), BaseModel::Small, TrainingPreset::Quick) > quick_small);
        let gpu = HardwareInfo {
            engine: TrainingEngineKind::Gpu,
            ..laptop
        };
        assert!(estimate(&gpu, BaseModel::Small, TrainingPreset::Quick) < quick_small);
        // Unknown core counts still give a finite answer.
        assert!(
            estimate(
                &HardwareInfo::default(),
                BaseModel::Large,
                TrainingPreset::Thorough
            ) > 0
        );
    }
}
