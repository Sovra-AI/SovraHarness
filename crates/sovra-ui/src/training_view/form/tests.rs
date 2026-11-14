use super::*;
use crate::training_view::knobs::Knob;
use sovra_core::group::{ContentHash, DeviceId, GroupId};
use sovra_core::training::{
    BaseModel, ModelCard, ModelFacts, ModelHeader, ModelOrigin, TrainingEngineKind,
};

const GIB: u64 = 1024 * 1024 * 1024;

fn computer(threads: u16, memory_gib: u64) -> HardwareInfo {
    HardwareInfo {
        os: "Test OS".into(),
        arch: "x86_64".into(),
        cpu: "Test processor".into(),
        physical_cores: threads / 2,
        logical_cores: threads,
        memory_total_bytes: memory_gib * GIB,
        memory_available_bytes: memory_gib * GIB / 2,
        engine: TrainingEngineKind::Cpu,
        engine_name: "Built-in processor engine".into(),
        gpus: Vec::new(),
    }
}

fn facts<'a>(hardware: &'a HardwareInfo, models: &'a [LibraryModel], in_group: bool) -> Facts<'a> {
    Facts {
        hardware,
        models,
        pasted_bytes: 0,
        pasted_blank: true,
        in_group,
    }
}

fn library_model(base: BaseModel, name: &str) -> LibraryModel {
    let header = ModelHeader::fresh(base);
    let architecture = header.architecture;
    let facts = ModelFacts {
        id: ContentHash([7; 32]),
        header,
        parameters: architecture.parameters() as u64,
        bytes: architecture.file_len() as u64,
    };
    LibraryModel {
        card: ModelCard::from_facts(
            &facts,
            name.into(),
            "Ada · Laptop".into(),
            ModelOrigin::Received {
                from: DeviceId([3; 32]),
            },
            Some(GroupId([1; 32])),
            1,
        ),
        saved_at: "/models/example.model".into(),
        delivery: Vec::new(),
    }
}

fn typed(settings: &TrainingSettings) -> Vec<String> {
    Knob::ALL
        .into_iter()
        .map(|knob| knob.shown(settings))
        .collect()
}

#[test]
fn the_page_opens_ready_to_start() {
    let hardware = computer(4, 8);
    let form = Form::new(&hardware);
    assert_eq!(form.source, SourceChoice::Sample);
    assert_eq!(form.base, TrainingBase::Fresh(BaseModel::Small));
    assert_eq!(form.preset, Some(TrainingPreset::Balanced));
    let facts = facts(&hardware, &[], false);
    assert_eq!(form.problem("My Small model", facts), None);
    let plan = form
        .plan("My Small model", String::new, facts)
        .expect("the default plan is valid");
    assert_eq!(plan, TrainingPlan::recommended(&hardware, false));
    // In a group the switch starts on.
    let grouped = Facts {
        in_group: true,
        ..facts
    };
    assert_eq!(
        form.plan("", String::new, grouped),
        Ok(TrainingPlan::recommended(&hardware, true))
    );
}

#[test]
fn choices_change_the_plan() {
    let hardware = computer(4, 8);
    let mut form = Form::new(&hardware);
    let facts = Facts {
        in_group: true,
        ..facts(&hardware, &[], true)
    };
    assert!(form.choose_preset(TrainingPreset::Quick, facts));
    assert!(form.choose_base(TrainingBase::Fresh(BaseModel::Starter), facts));
    form.group_choice = Some(false);
    let plan = form.plan(" ", String::new, facts).expect("valid");
    assert_eq!(plan.name, "My Starter model");
    assert_eq!(plan.base, TrainingBase::Fresh(BaseModel::Starter));
    assert_eq!(plan.preset, Some(TrainingPreset::Quick));
    assert!(!plan.with_group);
    assert_eq!(
        plan.settings,
        TrainingSettings::recommended(
            TrainingPreset::Quick,
            BaseModel::Starter.architecture(),
            SAMPLE_TEXT.len(),
            &hardware
        )
    );
    // Large cannot be trained by the group whatever the switch says.
    form.group_choice = Some(true);
    form.choose_base(TrainingBase::Fresh(BaseModel::Large), facts);
    assert!(!form.with_group(facts));
    assert!(
        !form
            .plan("Big", String::new, facts)
            .expect("valid")
            .with_group
    );
}

#[test]
fn an_edit_makes_the_plan_custom_and_matching_numbers_restore_the_preset() {
    let hardware = computer(4, 8);
    let facts = facts(&hardware, &[], false);
    let mut form = Form::new(&hardware);
    let recommended = form.settings.clone();
    let mut texts = typed(&recommended);
    texts[Knob::Rounds.index()] = "1234".into();
    form.edited(&texts, recommended.schedule, recommended.optimizer, facts);
    assert_eq!(form.preset, None);
    assert_eq!(form.settings.rounds, 1234);
    assert_eq!(form.problem("x", facts), None);
    let plan = form.plan("x", String::new, facts).expect("valid");
    assert_eq!((plan.preset, plan.settings.rounds), (None, 1234));

    // A named choice is an edit too.
    form.edited(&texts, LrSchedule::Constant, recommended.optimizer, facts);
    assert_eq!(form.settings.schedule, LrSchedule::Constant);

    // A bad field blocks the plan and says why; the last good settings stay.
    texts[Knob::Rounds.index()] = "5".into();
    form.edited(&texts, recommended.schedule, recommended.optimizer, facts);
    let problem = form.problem("x", facts).expect("blocked");
    assert!(
        problem.starts_with("Rounds must be between 20"),
        "{problem}"
    );
    assert_eq!(form.plan("x", String::new, facts), Err(problem));
    assert_eq!(form.settings.rounds, 1234);

    // Typing the preset's own numbers back is that preset again.
    form.edited(
        &typed(&recommended),
        recommended.schedule,
        recommended.optimizer,
        facts,
    );
    assert_eq!(form.preset, Some(TrainingPreset::Balanced));
    assert_eq!(form.settings_error, None);

    // Reset to recommended goes back to the last preset.
    texts[Knob::Rounds.index()] = "777".into();
    form.edited(&texts, recommended.schedule, recommended.optimizer, facts);
    assert!(form.reset(facts));
    assert_eq!(form.preset, Some(TrainingPreset::Balanced));
    assert_eq!(form.settings, recommended);
}

#[test]
fn text_and_file_sources_are_checked_before_start() {
    let hardware = computer(4, 8);
    let mut form = Form::new(&hardware);
    let base = facts(&hardware, &[], false);
    form.choose_source(SourceChoice::Text, base);
    assert!(form.problem("x", base).expect("blank").starts_with("Paste"));
    let pasted = "A sentence long enough to learn something from.".to_owned();
    let with_text = Facts {
        pasted_bytes: pasted.len(),
        pasted_blank: false,
        ..base
    };
    assert_eq!(form.problem("x", with_text), None);
    let plan = form.plan("x", || pasted.clone(), with_text).expect("valid");
    assert_eq!(plan.source, TrainingSource::Text(pasted));
    let too_long = Facts {
        pasted_bytes: MAX_TEXT_BYTES + 1,
        ..with_text
    };
    assert!(
        form.problem("x", too_long)
            .expect("too long")
            .contains("4 MB")
    );

    form.choose_source(SourceChoice::File, base);
    assert!(
        form.problem("x", base)
            .expect("no file")
            .starts_with("Choose a text file")
    );
    form.file = Some("/home/me/photo.png".into());
    assert!(
        form.problem("x", base)
            .expect("wrong kind")
            .contains(".txt")
    );
    form.file = Some("/home/me/notes.txt".into());
    assert_eq!(
        form.plan("x", String::new, base).expect("valid").source,
        TrainingSource::File("/home/me/notes.txt".into())
    );
    // A name that is too long is refused in words.
    assert!(
        form.problem(&"n".repeat(61), base)
            .expect("long name")
            .contains("name")
    );
}

#[test]
fn continuing_keeps_the_library_models_shape_and_follows_the_library() {
    let hardware = computer(4, 8);
    let models = vec![library_model(BaseModel::Medium, &"Long name ".repeat(6))];
    let id = models[0].card.id;
    let facts = facts(&hardware, &models, true);
    let mut form = Form::new(&hardware);
    form.choose_base(TrainingBase::Continue(id), facts);
    assert_eq!(
        form.architecture(&models),
        Some(BaseModel::Medium.architecture())
    );
    let name = form.default_name(&models);
    assert!(name.ends_with(", continued") && name.chars().count() <= MAX_NAME_CHARS);
    let plan = form.plan("", String::new, facts).expect("valid");
    assert_eq!(plan.base, TrainingBase::Continue(id));
    assert_eq!(
        plan.settings,
        TrainingSettings::recommended(
            TrainingPreset::Balanced,
            BaseModel::Medium.architecture(),
            SAMPLE_TEXT.len(),
            &hardware
        )
    );
    // The model leaves the library: Start is blocked, then the next snapshot
    // falls back to a fresh model.
    let gone = Facts {
        models: &[],
        ..facts
    };
    assert!(form.problem("x", gone).expect("gone").contains("no longer"));
    assert!(form.follow_hardware(gone));
    assert_eq!(form.base, TrainingBase::Fresh(BaseModel::Small));
}

#[test]
fn a_late_hardware_report_only_moves_an_untouched_form() {
    let unknown = HardwareInfo::default();
    let roomy = computer(16, 32);
    let mut form = Form::new(&unknown);
    assert_eq!(form.base, TrainingBase::Fresh(BaseModel::Small));
    assert!(form.follow_hardware(facts(&roomy, &[], false)));
    assert_eq!(form.base, TrainingBase::Fresh(BaseModel::Medium));
    assert!(!form.follow_hardware(facts(&roomy, &[], false)));

    let mut chosen = Form::new(&unknown);
    chosen.choose_base(
        TrainingBase::Fresh(BaseModel::Starter),
        facts(&unknown, &[], false),
    );
    chosen.follow_hardware(facts(&roomy, &[], false));
    assert_eq!(chosen.base, TrainingBase::Fresh(BaseModel::Starter));
}
