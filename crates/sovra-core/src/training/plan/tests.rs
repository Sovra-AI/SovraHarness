use super::*;
use crate::group::ContentHash;
use crate::swarm::learn::scavenger::GridSyncer;
use crate::training::{BASE_MODELS, TrainingEngineKind};

const MICRO: Architecture = Architecture {
    context: 8,
    width: 8,
    layers: 1,
    heads: 2,
};

type Change<T> = Box<dyn Fn(&mut T)>;

fn laptop() -> HardwareInfo {
    HardwareInfo {
        physical_cores: 4,
        logical_cores: 8,
        memory_total_bytes: 16 << 30,
        memory_available_bytes: 8 << 30,
        ..HardwareInfo::default()
    }
}

fn settings(architecture: Architecture) -> TrainingSettings {
    TrainingSettings::recommended(TrainingPreset::Balanced, architecture, 4000, &laptop())
}

/// A text long enough that no preset is cut short to keep from memorising it.
const LONG_TEXT: usize = 1 << 20;

fn long_settings(architecture: Architecture) -> TrainingSettings {
    TrainingSettings::recommended(TrainingPreset::Balanced, architecture, LONG_TEXT, &laptop())
}

fn shapes() -> Vec<Architecture> {
    let mut shapes: Vec<Architecture> = BASE_MODELS.iter().map(|row| row.architecture).collect();
    shapes.push(MICRO);
    shapes
}

fn plan(model: BaseModel) -> TrainingPlan {
    TrainingPlan {
        name: "Garden notes".into(),
        base: TrainingBase::Fresh(model),
        source: TrainingSource::Sample,
        preset: Some(TrainingPreset::Balanced),
        settings: settings(model.architecture()),
        with_group: false,
    }
}

#[test]
fn only_explicit_bounded_nonempty_text_is_accepted() {
    assert_eq!(TrainingSource::Sample.text(), Ok(Some(SAMPLE_TEXT)));
    for value in [
        String::new(),
        "x".repeat(MIN_TEXT_BYTES - 1),
        " ".repeat(MIN_TEXT_BYTES),
        "x".repeat(MAX_TEXT_BYTES + 1),
    ] {
        assert_eq!(check_text(&value), Err(TrainingError::InvalidInput));
        assert_eq!(
            TrainingSource::Text(value).text(),
            Err(TrainingError::InvalidInput)
        );
    }
    let least = "x".repeat(MIN_TEXT_BYTES);
    assert_eq!(
        TrainingSource::Text(least.clone()).text(),
        Ok(Some(least.as_str()))
    );
    assert_eq!(check_text(&"x".repeat(MAX_TEXT_BYTES)), Ok(()));
    assert_eq!(MAX_TEXT_BYTES, 4 * 1024 * 1024);
}

#[test]
fn the_sample_suits_every_starting_point() {
    assert_eq!(check_text(SAMPLE_TEXT), Ok(()));
    assert!(SAMPLE_TEXT.is_ascii());
    assert!(SAMPLE_TEXT.starts_with("The sun warms the garden."));
    for row in BASE_MODELS {
        let settings = TrainingSettings::recommended(
            TrainingPreset::Quick,
            row.architecture,
            SAMPLE_TEXT.len(),
            &laptop(),
        );
        let split = settings.check_split(SAMPLE_TEXT);
        // The part to learn from holds a full window, and a check tail is kept.
        assert!(split > usize::from(row.architecture.context) + 1, "{row:?}");
        assert!(SAMPLE_TEXT.len() - split >= 100, "{row:?}");
    }
}

#[test]
fn a_file_source_is_a_named_plain_text_file_that_the_runtime_reads() {
    for good in [
        "notes.txt",
        "/home/ada/My Notes.MD",
        "C:\\docs\\story.Txt",
        "a.b.md",
    ] {
        let source = TrainingSource::File(good.into());
        assert_eq!(source.validate(), Ok(()), "{good}");
        assert_eq!(source.text(), Ok(None));
        assert!(source.file_name().is_some());
    }
    for bad in [
        "",
        "notes",
        "notes.pdf",
        "notes.txt.exe",
        ".txt",
        "folder/",
        "..",
        "notes.md5",
    ] {
        let source = TrainingSource::File(bad.into());
        assert!(
            matches!(source.validate(), Err(TrainingError::InvalidFile(_))),
            "{bad}"
        );
        assert!(source.text().is_err());
    }
    let source = TrainingSource::File("/home/ada/secret folder/notes.txt".into());
    assert_eq!(source.file_name().as_deref(), Some("notes.txt"));
    let summary = source.summary(1234);
    assert_eq!(
        summary,
        SourceSummary {
            kind: SourceKind::File,
            bytes: 1234,
            file_name: Some("notes.txt".into())
        }
    );
    let long = TrainingSource::File(format!("{}\u{7}.txt", "n".repeat(300)).into());
    let name = long.file_name().unwrap();
    assert_eq!(name.chars().count(), MAX_FILE_NAME_CHARS);
    assert!(!name.chars().any(char::is_control));
    assert_eq!(TrainingSource::Sample.file_name(), None);
    assert_eq!(
        TrainingSource::Text("x".repeat(40)).summary(40),
        SourceSummary {
            kind: SourceKind::Text,
            bytes: 40,
            file_name: None
        }
    );
    assert_eq!(TrainingSource::Sample.summary(9).kind, SourceKind::Sample);
}

#[test]
fn file_bytes_become_text_only_when_plain_bounded_utf8() {
    let body = "The garden grows. ".repeat(4);
    assert_eq!(
        text_from_file_bytes(body.clone().into_bytes()),
        Ok(body.clone())
    );
    let mut marked = vec![0xEF, 0xBB, 0xBF];
    marked.extend_from_slice(body.as_bytes());
    assert_eq!(text_from_file_bytes(marked), Ok(body));
    assert!(matches!(
        text_from_file_bytes(vec![0xff; 64]),
        Err(TrainingError::InvalidFile(_))
    ));
    assert!(matches!(
        text_from_file_bytes(vec![0; 64]).as_deref(),
        Ok(text) if text.len() == 64
    ));
    assert_eq!(
        text_from_file_bytes(b"too short".to_vec()),
        Err(TrainingError::InvalidInput)
    );
    // A mark alone does not make short text long enough.
    let mut short = vec![0xEF, 0xBB, 0xBF];
    short.extend_from_slice(&[b'x'; MIN_TEXT_BYTES - 1]);
    assert_eq!(
        text_from_file_bytes(short),
        Err(TrainingError::InvalidInput)
    );
    assert_eq!(
        text_from_file_bytes(vec![b'x'; MAX_TEXT_BYTES + 1]),
        Err(TrainingError::InvalidInput)
    );
    assert_eq!(
        text_from_file_bytes(vec![b'x'; MAX_TEXT_BYTES + 4]),
        Err(TrainingError::InvalidInput)
    );
    let mut full = vec![0xEF, 0xBB, 0xBF];
    full.extend(std::iter::repeat_n(b'x', MAX_TEXT_BYTES));
    assert_eq!(
        text_from_file_bytes(full).map(|t| t.len()),
        Ok(MAX_TEXT_BYTES)
    );
}

#[test]
fn recommended_settings_are_valid_for_every_preset_shape_text_and_computer() {
    let computers = [
        HardwareInfo::default(),
        laptop(),
        HardwareInfo {
            engine: TrainingEngineKind::Gpu,
            ..laptop()
        },
    ];
    for architecture in shapes() {
        for preset in TrainingPreset::ALL {
            for text_len in [MIN_TEXT_BYTES, 159, 160, SAMPLE_TEXT.len(), MAX_TEXT_BYTES] {
                for computer in &computers {
                    let settings =
                        TrainingSettings::recommended(preset, architecture, text_len, computer);
                    assert_eq!(
                        settings.validate(architecture),
                        Ok(()),
                        "{architecture:?} {preset:?} {text_len}"
                    );
                    assert_eq!(settings.check_share_percent == 0, text_len < 160);
                    assert!(settings.stop_when_stalled == 0 || settings.check_share_percent > 0);
                    assert!(settings.grid_settings(architecture).validate().is_ok());
                    assert!(!preset.title().is_empty() && !preset.summary().is_empty());
                }
            }
        }
    }
    // Presets get longer in order, and bigger shapes read about as much per step.
    for architecture in shapes() {
        // The Starter table's rounds grow with the text instead, so it is asked
        // about a short one.
        let text_len = if architecture.is_table() {
            4000
        } else {
            LONG_TEXT
        };
        let rounds: Vec<u32> = TrainingPreset::ALL
            .iter()
            .map(|preset| {
                TrainingSettings::recommended(*preset, architecture, text_len, &laptop()).rounds
            })
            .collect();
        assert!(rounds[0] < rounds[1] && rounds[1] < rounds[2], "{rounds:?}");
    }
    // A packet has room for every window to be its own stretch of text: a
    // window, the byte after it, and up to three bytes to finish a character.
    let small = long_settings(BaseModel::Small.architecture());
    assert_eq!((small.windows, small.batch_bytes), (8, 8 * 68));
    let medium = long_settings(BaseModel::Medium.architecture());
    assert_eq!((medium.windows, medium.batch_bytes), (4, 4 * 132));
    let large = long_settings(BaseModel::Large.architecture());
    assert_eq!((large.windows, large.batch_bytes), (2, 2 * 260));
    // Balanced reads enough to get a transformer past letter pairs: more rounds
    // the bigger the model, one step each.
    assert_eq!(
        (small.rounds, medium.rounds, large.rounds),
        (2_000, 3_200, 6_000)
    );
    assert_eq!(
        (small.local_steps, medium.local_steps, large.local_steps),
        (1, 1, 1)
    );
    // A graphics card reads four times as much each round in half the rounds.
    let card = HardwareInfo {
        engine: TrainingEngineKind::Gpu,
        ..laptop()
    };
    for row in BASE_MODELS
        .iter()
        .filter(|row| !row.architecture.is_table())
    {
        for preset in TrainingPreset::ALL {
            let on_processor =
                TrainingSettings::recommended(preset, row.architecture, LONG_TEXT, &laptop());
            let on_card = TrainingSettings::recommended(preset, row.architecture, LONG_TEXT, &card);
            assert_eq!(on_card.windows, on_processor.windows * 4, "{}", row.title);
            assert_eq!(on_card.rounds, on_processor.rounds / 2, "{}", row.title);
            assert_eq!(on_card.check_every, on_processor.check_every / 2);
            assert_eq!(
                on_card.total_tokens(row.architecture),
                on_processor.total_tokens(row.architecture) * 2
            );
            let reach = u32::from(row.architecture.context) + 4;
            assert_eq!(on_card.batch_bytes, u32::from(on_card.windows) * reach);
            assert!(on_card.batch_bytes as usize <= MAX_WORK_BATCH_BYTES);
        }
    }
    // The Starter table is the same on any engine.
    assert_eq!(
        TrainingSettings::recommended(TrainingPreset::Balanced, Architecture::STARTER, 4000, &card),
        settings(Architecture::STARTER)
    );
    // Long text gives the Starter table enough rounds to read it all once.
    let long = TrainingSettings::recommended(
        TrainingPreset::Quick,
        Architecture::STARTER,
        MAX_TEXT_BYTES,
        &laptop(),
    );
    assert_eq!(long.rounds, 8257);
    assert!(long.is_legacy_starter(Architecture::STARTER));
}

#[test]
fn every_setting_outside_its_range_is_refused_by_name() {
    let architecture = BaseModel::Small.architecture();
    let good = settings(architecture);
    let cases: Vec<(&str, Change<TrainingSettings>)> = vec![
        ("Rounds", Box::new(|s| s.rounds = 19)),
        ("Rounds", Box::new(|s| s.rounds = 100_001)),
        ("Text per packet", Box::new(|s| s.batch_bytes = 64)),
        ("Text per packet", Box::new(|s| s.batch_bytes = 4097)),
        ("Windows per step", Box::new(|s| s.windows = 0)),
        ("Windows per step", Box::new(|s| s.windows = 65)),
        ("Steps per round", Box::new(|s| s.local_steps = 0)),
        ("Steps per round", Box::new(|s| s.local_steps = 17)),
        ("Learning rate", Box::new(|s| s.learning_rate_micro = 0)),
        (
            "Learning rate",
            Box::new(|s| s.learning_rate_micro = 10_000_001),
        ),
        (
            "Warm-up rounds",
            Box::new(|s| s.warmup_rounds = s.rounds + 1),
        ),
        (
            "Weight decay",
            Box::new(|s| s.weight_decay_micro = 1_000_001),
        ),
        ("Step limit", Box::new(|s| s.clip_milli = 1_000_001)),
        (
            "Round step size",
            Box::new(|s| s.outer_learning_rate_micro = 0),
        ),
        (
            "Round step size",
            Box::new(|s| s.outer_learning_rate_micro = 1_000_001),
        ),
        ("Check share", Box::new(|s| s.check_share_percent = 31)),
        ("Rounds between checks", Box::new(|s| s.check_every = 0)),
        (
            "Rounds between checks",
            Box::new(|s| s.check_every = 10_001),
        ),
        (
            "Sample variety",
            Box::new(|s| s.sample_temperature_milli = 5001),
        ),
        ("Sample length", Box::new(|s| s.sample_bytes = 15)),
        ("Sample length", Box::new(|s| s.sample_bytes = 513)),
        (
            "Stop when stalled",
            Box::new(|s| s.stop_when_stalled = 100_001),
        ),
        ("Processor threads", Box::new(|s| s.threads = 65)),
        ("Rounds a helper may lag", Box::new(|s| s.cutoff_rounds = 0)),
        (
            "Rounds a helper may lag",
            Box::new(|s| s.cutoff_rounds = 1025),
        ),
        ("Lag discount", Box::new(|s| s.age_decay_milli = 100_001)),
        (
            "Seconds a helper may keep a packet",
            Box::new(|s| s.lease_seconds = 4),
        ),
        (
            "Seconds a helper may keep a packet",
            Box::new(|s| s.lease_seconds = 3601),
        ),
    ];
    for (name, change) in &cases {
        let mut altered = good.clone();
        change(&mut altered);
        match altered.validate(architecture) {
            Err(TrainingError::InvalidSetting(message)) => {
                assert!(message.starts_with(name), "{name}: {message}");
                assert!(message.ends_with('.'));
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    let mut rounds = good.clone();
    rounds.rounds = 5;
    assert_eq!(
        rounds.validate(architecture).unwrap_err().to_string(),
        "Rounds must be between 20 and 100,000."
    );
    // The edges themselves are fine.
    let mut edges = good.clone();
    edges.rounds = 100_000;
    edges.batch_bytes = 65;
    edges.windows = 64;
    edges.local_steps = 16;
    edges.learning_rate_micro = 10_000_000;
    edges.warmup_rounds = 100_000;
    edges.clip_milli = 0;
    edges.check_share_percent = 30;
    edges.sample_temperature_milli = 0;
    edges.sample_bytes = 512;
    edges.threads = 64;
    edges.cutoff_rounds = 1024;
    edges.lease_seconds = 3600;
    assert_eq!(edges.validate(architecture), Ok(()));
    // A packet must hold one full window and the byte after it.
    let large = BaseModel::Large.architecture();
    let mut short = settings(large);
    short.batch_bytes = 256;
    assert!(short.validate(large).is_err());
    short.batch_bytes = 257;
    assert_eq!(short.validate(large), Ok(()));
    assert_eq!(TrainingSettings::min_batch_bytes(large), 257);
    assert_eq!(TrainingSettings::min_batch_bytes(MICRO), 64);
    assert_eq!(TrainingSettings::min_batch_bytes(Architecture::STARTER), 64);
    // The Starter table reads the whole packet: one window only.
    let mut table = settings(Architecture::STARTER);
    table.windows = 2;
    assert!(table.validate(Architecture::STARTER).is_err());
    // An unsupported shape is refused whatever the settings.
    let odd = Architecture { width: 9, ..MICRO };
    assert_eq!(good.validate(odd), Err(TrainingError::InvalidModel));
    // Unknown saved fields are refused rather than ignored.
    let mut json = serde_json::to_value(&good).unwrap();
    assert_eq!(
        serde_json::from_value::<TrainingSettings>(json.clone()).unwrap(),
        good
    );
    json["command"] = "run".into();
    assert!(serde_json::from_value::<TrainingSettings>(json).is_err());
}

#[test]
fn the_schedule_scales_the_round_step_size_and_never_reaches_zero() {
    let mut s = settings(BaseModel::Small.architecture());
    s.rounds = 1000;
    s.learning_rate_micro = 3000;
    s.outer_learning_rate_micro = 2000;

    s.schedule = LrSchedule::Constant;
    for round in [0, 1, 500, 999, 5000] {
        assert_eq!(s.schedule_milli(round), 1000);
    }
    assert_eq!(s.outer_learning_rate_micro_at(999), 2000);

    s.schedule = LrSchedule::Cosine;
    assert_eq!(s.schedule_milli(0), 1000);
    assert_eq!(s.schedule_milli(500), 550);
    assert_eq!(s.schedule_milli(1000), 100);
    assert_eq!(s.schedule_milli(u32::MAX), 100);
    assert!((0..1000).all(|r| s.schedule_milli(r) >= s.schedule_milli(r + 1)));

    s.schedule = LrSchedule::WarmupCosine;
    s.warmup_rounds = 100;
    assert_eq!(s.schedule_milli(0), 10);
    assert_eq!(s.schedule_milli(49), 500);
    assert_eq!(s.schedule_milli(99), 1000);
    assert_eq!(s.schedule_milli(100), 1000);
    assert_eq!(s.schedule_milli(550), 550);
    assert_eq!(s.schedule_milli(1000), 100);
    assert!((0..99).all(|r| s.schedule_milli(r) < s.schedule_milli(r + 1)));
    assert!((100..1000).all(|r| s.schedule_milli(r) >= s.schedule_milli(r + 1)));
    assert_eq!(s.outer_learning_rate_micro_at(0), 20);
    assert!((s.outer_learning_rate_at(0) - 0.00002).abs() < 1e-12);
    assert!((s.outer_learning_rate_at(100) - 0.002).abs() < 1e-12);

    // No warm-up, or warm-up for the whole run, are both safe.
    s.warmup_rounds = 0;
    assert_eq!(s.schedule_milli(0), 1000);
    s.warmup_rounds = 1000;
    assert_eq!(s.schedule_milli(999), 1000);
    assert_eq!(s.schedule_milli(1000), 1000);
    // The smallest step sizes never round down to nothing.
    s.learning_rate_micro = 1;
    s.outer_learning_rate_micro = 1;
    s.warmup_rounds = 100;
    for round in 0..1100 {
        assert_eq!(s.outer_learning_rate_micro_at(round), 1);
        assert!(s.outer_learning_rate_at(round) > 0.0);
    }
}

#[test]
fn a_short_text_is_not_read_so_often_that_it_is_only_memorised() {
    let small = BaseModel::Small.architecture();
    let card = HardwareInfo {
        engine: TrainingEngineKind::Gpu,
        ..laptop()
    };
    // The built-in sample keeps back a tenth and learns from 1,929 bytes. No
    // preset reads that more than 120 times, on either engine.
    let learned = 1929;
    for preset in TrainingPreset::ALL {
        for (computer, rounds) in [(laptop(), 452), (card.clone(), 113)] {
            let s = TrainingSettings::recommended(preset, small, SAMPLE_TEXT.len(), &computer);
            assert_eq!(s.check_split(SAMPLE_TEXT), learned);
            assert_eq!(s.rounds, rounds, "{preset:?}");
            assert!(s.total_tokens(small) <= 120 * learned as u64);
            assert!(s.warmup_rounds <= s.rounds);
            assert_eq!(s.validate(small), Ok(()));
        }
    }
    // A Quick try of a bigger model is shorter than that anyway.
    let medium = BaseModel::Medium.architecture();
    let quick =
        TrainingSettings::recommended(TrainingPreset::Quick, medium, SAMPLE_TEXT.len(), &laptop());
    assert_eq!(quick.rounds, 240);
    // The shortest text still gets the fewest rounds a run may have.
    let least =
        TrainingSettings::recommended(TrainingPreset::Thorough, small, MIN_TEXT_BYTES, &laptop());
    assert_eq!(least.rounds, TrainingSettings::ROUNDS.0);
    // A long text gets the preset's own rounds, and the Starter table, which
    // reads its text straight through, is never cut short.
    assert_eq!(long_settings(small).rounds, 2000);
    assert_eq!(settings(Architecture::STARTER).rounds, 400);
}

#[test]
fn token_counts_and_merge_settings_follow_from_the_plan() {
    let small = BaseModel::Small.architecture();
    let s = long_settings(small);
    assert_eq!((s.windows, s.local_steps, s.rounds), (8, 1, 2000));
    assert_eq!(s.work_tokens(small, 1024), 8 * 64);
    // A packet shorter than the model's reach teaches less.
    assert_eq!(s.work_tokens(small, 33), 8 * 32);
    assert_eq!(s.work_tokens(small, 1), 0);
    assert_eq!(s.total_tokens(small), 512 * 2000);
    let table = settings(Architecture::STARTER);
    assert_eq!(table.work_tokens(Architecture::STARTER, 512), 511 * 4);

    for architecture in shapes() {
        let s = settings(architecture);
        let grid = s.grid_settings(architecture);
        assert_eq!(grid.local_steps, s.local_steps);
        assert_eq!(grid.cutoff_rounds, 128);
        assert_eq!(grid.lease_ms, 120_000);
        assert!((grid.age_decay - 0.02).abs() < 1e-12);
        assert_eq!(
            u64::from(grid.token_budget),
            s.work_tokens(architecture, s.batch_bytes as usize)
        );
        assert!((grid.learning_rate - s.outer_learning_rate_at(0)).abs() < 1e-12);
    }
    // The extremes of every setting still make settings the merge engine accepts.
    let mut most = settings(MICRO);
    most.windows = 64;
    most.local_steps = 16;
    most.batch_bytes = 4096;
    most.outer_learning_rate_micro = 1_000_000;
    most.schedule = LrSchedule::Constant;
    most.age_decay_milli = 100_000;
    most.cutoff_rounds = 1024;
    most.lease_seconds = 3600;
    let widest = Architecture {
        context: 1024,
        width: 16,
        layers: 1,
        heads: 2,
    };
    assert_eq!(widest.validate(), Ok(()));
    assert_eq!(most.validate(widest), Ok(()));
    let grid = most.grid_settings(widest);
    assert_eq!(grid.token_budget, 1_048_576);
    assert!(GridSyncer::new(ContentHash::of(b"run"), vec![0.0; 4], grid).is_ok());
    let mut least = settings(MICRO);
    least.outer_learning_rate_micro = 1;
    least.age_decay_milli = 0;
    least.cutoff_rounds = 1;
    least.lease_seconds = 5;
    least.local_steps = 1;
    least.windows = 1;
    least.batch_bytes = 64;
    assert_eq!(least.validate(MICRO), Ok(()));
    let mut syncer = GridSyncer::new(
        ContentHash::of(b"run"),
        vec![0.0; 4],
        least.grid_settings(MICRO),
    )
    .unwrap();
    for round in [0, 10, 999, 100_000] {
        assert!(
            syncer
                .set_learning_rate(least.outer_learning_rate_at(round))
                .is_ok()
        );
    }
}

#[test]
fn the_check_tail_is_held_back_only_when_both_parts_are_readable() {
    let mut s = settings(MICRO);
    s.check_share_percent = 10;
    let text = "x".repeat(1000);
    assert_eq!(s.check_split(&text), 900);
    s.check_share_percent = 0;
    assert_eq!(s.check_split(&text), 1000);
    s.check_share_percent = 30;
    assert_eq!(s.check_split(&text), 700);
    // Too little text for a tail of two bytes, or for 32 bytes to learn from.
    s.check_share_percent = 10;
    assert_eq!(s.check_split(&"x".repeat(19)), 19);
    assert_eq!(s.check_split(&"x".repeat(32)), 32);
    assert_eq!(s.check_split(&"x".repeat(34)), 34);
    assert_eq!(s.check_split(&"x".repeat(35)), 32);
    assert_eq!(s.check_split(&"x".repeat(40)), 36);
    assert_eq!(s.check_split(""), 0);
    // The split never lands inside a character.
    let accents = "é".repeat(500);
    let split = s.check_split(&accents);
    assert!(accents.is_char_boundary(split));
    assert_eq!(split, 900);
    let mixed = format!("a{}", "é".repeat(500));
    let split = s.check_split(&mixed);
    assert!(mixed.is_char_boundary(split));
    assert!(mixed.len() - split >= 100);
}

#[test]
fn only_the_first_releases_own_starter_settings_keep_the_first_wire_form() {
    let table = Architecture::STARTER;
    let legacy = settings(table);
    assert!(legacy.is_legacy_starter(table));
    assert!(!settings(MICRO).is_legacy_starter(MICRO));
    assert!(!legacy.is_legacy_starter(MICRO));
    let changes: Vec<Change<TrainingSettings>> = vec![
        Box::new(|s| s.batch_bytes = 513),
        Box::new(|s| s.optimizer = LocalOptimizer::Adam),
        Box::new(|s| s.schedule = LrSchedule::Cosine),
        Box::new(|s| s.learning_rate_micro = 100_001),
        Box::new(|s| s.weight_decay_micro = 1),
        Box::new(|s| s.clip_milli = 1),
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut altered = legacy.clone();
        change(&mut altered);
        assert!(!altered.is_legacy_starter(table), "change {index}");
    }
    // Settings a helper never sees do not matter.
    let mut same = legacy;
    same.rounds = 5000;
    same.seed = 7;
    same.outer_learning_rate_micro = 9;
    same.batch_bytes = 64;
    assert!(same.is_legacy_starter(table));
}

#[test]
fn a_plan_is_checked_as_a_whole_for_its_model_shape() {
    for model in BaseModel::ALL {
        assert_eq!(
            plan(model).validate(model.architecture()),
            Ok(()),
            "{model:?}"
        );
    }
    let small = BaseModel::Small.architecture();
    for name in ["", "   ", " padded", "two\nlines", &"x".repeat(61)] {
        let mut named = plan(BaseModel::Small);
        named.name = name.into();
        assert!(
            matches!(named.validate(small), Err(TrainingError::InvalidSetting(_))),
            "{name:?}"
        );
    }
    let mut longest = plan(BaseModel::Small);
    longest.name = "é".repeat(MAX_NAME_CHARS);
    assert_eq!(longest.validate(small), Ok(()));
    // A fresh model has its row's shape and no other.
    assert_eq!(
        plan(BaseModel::Small).validate(BaseModel::Medium.architecture()),
        Err(TrainingError::InvalidModel)
    );
    let mut source = plan(BaseModel::Small);
    source.source = TrainingSource::Text("short".into());
    assert_eq!(source.validate(small), Err(TrainingError::InvalidInput));
    source.source = TrainingSource::File("notes.exe".into());
    assert!(matches!(
        source.validate(small),
        Err(TrainingError::InvalidFile(_))
    ));
    let mut setting = plan(BaseModel::Small);
    setting.settings.rounds = 1;
    assert!(setting.validate(small).is_err());
    // The group can help with every shape that fits its messages, not Large.
    for model in BaseModel::ALL {
        let mut shared = plan(model);
        shared.with_group = true;
        assert_eq!(
            shared.validate(model.architecture()).is_ok(),
            model.info().group_capable,
            "{model:?}"
        );
    }
    // Continuing a library model takes whatever shape that model has.
    let mut onward = plan(BaseModel::Small);
    onward.base = TrainingBase::Continue(ContentHash::of(b"model"));
    onward.settings = settings(MICRO);
    assert_eq!(onward.base.architecture(), None);
    assert_eq!(onward.validate(MICRO), Ok(()));
    assert_eq!(
        TrainingBase::Fresh(BaseModel::Medium).architecture(),
        Some(BaseModel::Medium.architecture())
    );
}

#[test]
fn the_page_opens_on_a_plan_that_only_needs_start_pressed() {
    for (computer, model) in [
        (HardwareInfo::default(), BaseModel::Small),
        (laptop(), BaseModel::Medium),
        (
            HardwareInfo {
                engine: TrainingEngineKind::Gpu,
                ..laptop()
            },
            // Also the largest model the group can help with.
            BaseModel::Medium,
        ),
    ] {
        for with_group in [false, true] {
            let plan = TrainingPlan::recommended(&computer, with_group);
            assert_eq!(plan.base, TrainingBase::Fresh(model));
            assert_eq!(plan.source, TrainingSource::Sample);
            assert_eq!(plan.preset, Some(TrainingPreset::Balanced));
            assert_eq!(plan.with_group, with_group && model.info().group_capable);
            assert_eq!(plan.validate(model.architecture()), Ok(()));
            assert_eq!(plan.name, format!("My {} model", model.info().title));
        }
    }
}

#[test]
fn the_summary_shows_the_plan_without_its_text() {
    let secret = "my private diary entry ".repeat(10);
    let mut plan = plan(BaseModel::Small);
    plan.source = TrainingSource::Text(secret.clone());
    plan.with_group = true;
    let small = BaseModel::Small.architecture();
    let summary = plan.summary(small, secret.len());
    assert_eq!(summary.name, "Garden notes");
    assert_eq!(summary.architecture, small);
    assert_eq!(summary.source.bytes, secret.len() as u64);
    assert_eq!(summary.source.kind, SourceKind::Text);
    assert_eq!(summary.settings, plan.settings);
    assert!(summary.with_group);
    assert_eq!(summary.validate(), Ok(()));
    let json = serde_json::to_string(&summary).unwrap();
    assert!(!json.contains("diary"));
    assert_eq!(
        serde_json::from_str::<TrainingPlanSummary>(&json).unwrap(),
        summary
    );
    let debug = format!("{summary:?}");
    assert!(!debug.contains("diary"));

    let changes: Vec<Change<TrainingPlanSummary>> = vec![
        Box::new(|s| s.name.clear()),
        Box::new(|s| s.settings.rounds = 0),
        Box::new(|s| s.source.bytes = MAX_TEXT_BYTES as u64 + 1),
        Box::new(|s| s.source.file_name = Some("n".repeat(MAX_FILE_NAME_CHARS + 1))),
        Box::new(|s| s.source.file_name = Some("a\nb".into())),
        Box::new(|s| s.architecture = BaseModel::Medium.architecture()),
        Box::new(|s| {
            s.base = TrainingBase::Fresh(BaseModel::Large);
            s.architecture = BaseModel::Large.architecture();
            s.settings.batch_bytes = 4096;
        }),
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut altered = summary.clone();
        change(&mut altered);
        assert!(altered.validate().is_err(), "change {index}");
    }
    let mut json = serde_json::to_value(&summary).unwrap();
    json["text"] = "x".into();
    assert!(serde_json::from_value::<TrainingPlanSummary>(json).is_err());
}
