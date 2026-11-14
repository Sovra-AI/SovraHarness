//! What the person has chosen on the Training page, as plain data.
//!
//! The view keeps the typed text in input controls; everything decided from it
//! (which settings apply, whether Start may be pressed, the plan that is sent)
//! is worked out here so it can be tested without a window.

use std::path::PathBuf;

use sovra_core::training::{
    Architecture, HardwareInfo, LibraryModel, LocalOptimizer, LrSchedule, MAX_NAME_CHARS,
    MAX_TEXT_BYTES, MIN_TEXT_BYTES, SAMPLE_TEXT, TrainingBase, TrainingPlan, TrainingPreset,
    TrainingSettings, TrainingSource, check_name,
};

use super::knobs::read_settings;

/// The page cannot read a chosen file, so presets are sized for a text of this
/// length until the run starts and the real length is known.
const FILE_NOMINAL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceChoice {
    Sample,
    Text,
    File,
}

/// The facts a choice is weighed against: this computer, the library, and how
/// much text has been pasted.
#[derive(Clone, Copy)]
pub(super) struct Facts<'a> {
    pub hardware: &'a HardwareInfo,
    pub models: &'a [LibraryModel],
    pub pasted_bytes: usize,
    pub pasted_blank: bool,
    pub in_group: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Form {
    pub source: SourceChoice,
    pub file: Option<PathBuf>,
    pub base: TrainingBase,
    /// `None` once a setting differs from every preset: shown as Custom.
    pub preset: Option<TrainingPreset>,
    /// What Reset to recommended returns to.
    pub last_preset: TrainingPreset,
    pub settings: TrainingSettings,
    /// Why the typed expert fields cannot be used, when they cannot.
    pub settings_error: Option<String>,
    /// The person's own answer to Train with my group; `None` follows the default.
    pub group_choice: Option<bool>,
    /// Set by the first deliberate choice, after which a late hardware report no
    /// longer changes the starting point.
    pub customised: bool,
}

impl Form {
    /// The sample, the model this computer is comfortable with, and Balanced.
    pub(super) fn new(hardware: &HardwareInfo) -> Self {
        let plan = TrainingPlan::recommended(hardware, false);
        let preset = plan.preset.unwrap_or(TrainingPreset::Balanced);
        Self {
            source: SourceChoice::Sample,
            file: None,
            base: plan.base,
            preset: Some(preset),
            last_preset: preset,
            settings: plan.settings,
            settings_error: None,
            group_choice: None,
            customised: false,
        }
    }

    /// The model shape the plan would train: the starting point's own, or the
    /// library model's when continuing. `None` when that model is gone.
    pub(super) fn architecture(&self, models: &[LibraryModel]) -> Option<Architecture> {
        match self.base {
            TrainingBase::Fresh(model) => Some(model.architecture()),
            TrainingBase::Continue(id) => models
                .iter()
                .find(|model| model.card.id == id)
                .map(|model| model.card.architecture),
        }
    }

    fn text_bytes(&self, facts: Facts<'_>) -> usize {
        match self.source {
            SourceChoice::Sample => SAMPLE_TEXT.len(),
            SourceChoice::Text => facts.pasted_bytes.max(MIN_TEXT_BYTES),
            SourceChoice::File => FILE_NOMINAL_BYTES,
        }
    }

    /// The settings a preset gives for the current choices.
    pub(super) fn recommended(
        &self,
        preset: TrainingPreset,
        facts: Facts<'_>,
    ) -> Option<TrainingSettings> {
        let architecture = self.architecture(facts.models)?;
        Some(TrainingSettings::recommended(
            preset,
            architecture,
            self.text_bytes(facts),
            facts.hardware,
        ))
    }

    /// Re-derives the settings after the text, starting point or computer
    /// changed. Custom settings are kept as typed. Returns whether the shown
    /// numbers have to be rewritten.
    pub(super) fn refresh(&mut self, facts: Facts<'_>) -> bool {
        let Some(preset) = self.preset else {
            return false;
        };
        let Some(settings) = self.recommended(preset, facts) else {
            return false;
        };
        let changed = settings != self.settings || self.settings_error.is_some();
        self.settings = settings;
        self.settings_error = None;
        changed
    }

    /// Follows a hardware report: until the person has chosen something, the
    /// starting point is the one this computer is comfortable with.
    pub(super) fn follow_hardware(&mut self, facts: Facts<'_>) -> bool {
        let mut changed = false;
        if !self.customised {
            let base = TrainingBase::Fresh(facts.hardware.recommended_model());
            changed = base != self.base;
            self.base = base;
        }
        if matches!(self.base, TrainingBase::Continue(_))
            && self.architecture(facts.models).is_none()
        {
            // The model being continued is no longer in the library.
            self.base = TrainingBase::Fresh(facts.hardware.recommended_model());
            changed = true;
        }
        self.refresh(facts) || changed
    }

    pub(super) fn choose_source(&mut self, source: SourceChoice, facts: Facts<'_>) -> bool {
        self.source = source;
        self.refresh(facts)
    }

    pub(super) fn choose_base(&mut self, base: TrainingBase, facts: Facts<'_>) -> bool {
        self.base = base;
        self.customised = true;
        self.refresh(facts)
    }

    pub(super) fn choose_preset(&mut self, preset: TrainingPreset, facts: Facts<'_>) -> bool {
        self.preset = Some(preset);
        self.last_preset = preset;
        self.customised = true;
        self.refresh(facts);
        true
    }

    /// Reset to recommended: back to the last preset's numbers.
    pub(super) fn reset(&mut self, facts: Facts<'_>) -> bool {
        self.choose_preset(self.last_preset, facts)
    }

    /// Takes the expert fields as typed. A set of numbers that equals a preset
    /// is that preset again; anything else is Custom.
    pub(super) fn edited(
        &mut self,
        texts: &[String],
        schedule: LrSchedule,
        optimizer: LocalOptimizer,
        facts: Facts<'_>,
    ) {
        self.customised = true;
        let Some(architecture) = self.architecture(facts.models) else {
            return;
        };
        match read_settings(texts, schedule, optimizer, &self.settings, architecture) {
            Ok(settings) => {
                self.settings = settings;
                self.settings_error = None;
                self.preset = TrainingPreset::ALL.into_iter().find(|preset| {
                    self.recommended(*preset, facts).as_ref() == Some(&self.settings)
                });
                if let Some(preset) = self.preset {
                    self.last_preset = preset;
                }
            }
            Err(error) => {
                self.settings.schedule = schedule;
                self.settings.optimizer = optimizer;
                self.settings_error = Some(error);
                self.preset = None;
            }
        }
    }

    /// The name a model gets when the name field is left as it was.
    pub(super) fn default_name(&self, models: &[LibraryModel]) -> String {
        match self.base {
            TrainingBase::Fresh(model) => format!("My {} model", model.info().title),
            TrainingBase::Continue(id) => {
                const SUFFIX: &str = ", continued";
                let name = models
                    .iter()
                    .find(|model| model.card.id == id)
                    .map_or("My model", |model| model.card.name.as_str());
                let room = MAX_NAME_CHARS - SUFFIX.len();
                let kept: String = name.chars().take(room).collect();
                format!("{}{SUFFIX}", kept.trim_end())
            }
        }
    }

    /// Whether the chosen model can be trained by the group's computers at all.
    pub(super) fn group_capable(&self, models: &[LibraryModel]) -> bool {
        self.architecture(models)
            .is_some_and(|architecture| architecture.group_capable())
    }

    /// Train with my group: on by default for anyone in a group, and never for a
    /// model too large for the group's computers.
    pub(super) fn with_group(&self, facts: Facts<'_>) -> bool {
        facts.in_group && self.group_choice.unwrap_or(true) && self.group_capable(facts.models)
    }

    fn name(&self, typed: &str, models: &[LibraryModel]) -> String {
        let typed = typed.trim();
        if typed.is_empty() {
            self.default_name(models)
        } else {
            typed.to_owned()
        }
    }

    /// Why Start cannot be pressed yet, in plain words. Cheap enough to ask on
    /// every frame: it never looks at the pasted text itself.
    pub(super) fn problem(&self, typed_name: &str, facts: Facts<'_>) -> Option<String> {
        match (self.source, &self.file) {
            (SourceChoice::Text, _)
                if facts.pasted_blank || facts.pasted_bytes < MIN_TEXT_BYTES =>
            {
                return Some(
                    "Paste a few sentences to learn from (at least 32 letters), or try the sample."
                        .into(),
                );
            }
            (SourceChoice::Text, _) if facts.pasted_bytes > MAX_TEXT_BYTES => {
                return Some("That is more than 4 MB of text. Use a shorter piece.".into());
            }
            (SourceChoice::File, None) => {
                return Some("Choose a text file, or try the sample.".into());
            }
            (SourceChoice::File, Some(path)) => {
                if let Err(error) = TrainingSource::File(path.clone()).validate() {
                    return Some(error.to_string());
                }
            }
            (SourceChoice::Sample | SourceChoice::Text, _) => {}
        }
        let Some(architecture) = self.architecture(facts.models) else {
            return Some(
                "That model is no longer in Your models. Choose another starting point.".into(),
            );
        };
        if let Some(error) = &self.settings_error {
            return Some(error.clone());
        }
        if let Err(error) = check_name(&self.name(typed_name, facts.models)) {
            return Some(error.to_string());
        }
        self.settings
            .validate(architecture)
            .err()
            .map(|error| error.to_string())
    }

    /// The plan Start sends. `pasted` is only called when pasted text is the
    /// chosen source.
    pub(super) fn plan(
        &self,
        typed_name: &str,
        pasted: impl FnOnce() -> String,
        facts: Facts<'_>,
    ) -> Result<TrainingPlan, String> {
        if let Some(problem) = self.problem(typed_name, facts) {
            return Err(problem);
        }
        let architecture = self
            .architecture(facts.models)
            .ok_or_else(|| "Choose a starting point.".to_owned())?;
        let source = match self.source {
            SourceChoice::Sample => TrainingSource::Sample,
            SourceChoice::Text => TrainingSource::Text(pasted()),
            SourceChoice::File => TrainingSource::File(
                self.file
                    .clone()
                    .ok_or_else(|| "Choose a text file, or try the sample.".to_owned())?,
            ),
        };
        let plan = TrainingPlan {
            name: self.name(typed_name, facts.models),
            base: self.base,
            source,
            preset: self.preset,
            settings: self.settings.clone(),
            with_group: self.with_group(facts),
        };
        plan.validate(architecture)
            .map_err(|error| error.to_string())?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests;
