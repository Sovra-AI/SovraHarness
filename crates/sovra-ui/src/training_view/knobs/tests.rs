use super::*;
use sovra_core::training::{
    BaseModel, HardwareInfo, SAMPLE_TEXT, TrainingEngineKind, TrainingPreset,
};

fn computer() -> HardwareInfo {
    HardwareInfo {
        physical_cores: 4,
        logical_cores: 8,
        memory_total_bytes: 16 * 1024 * 1024 * 1024,
        engine: TrainingEngineKind::Cpu,
        ..HardwareInfo::default()
    }
}

fn typed(settings: &TrainingSettings) -> Vec<String> {
    Knob::ALL
        .into_iter()
        .map(|knob| knob.shown(settings))
        .collect()
}

#[test]
fn shown_numbers_read_back_exactly() {
    assert_eq!(show(3_000, MILLIONTHS, false), "0.003");
    assert_eq!(show(250_000, MILLIONTHS, false), "0.25");
    assert_eq!(show(10_000_000, MILLIONTHS, true), "10");
    assert_eq!(show(800, THOUSANDTHS, false), "0.8");
    assert_eq!(show(100_000, 1, true), "100,000");
    assert_eq!(show(100_000, 1, false), "100000");
    assert_eq!(read("0.003", MILLIONTHS), Some(3_000));
    assert_eq!(read(" .5 ", THOUSANDTHS), Some(500));
    assert_eq!(read("2.", THOUSANDTHS), Some(2_000));
    assert_eq!(read("42", 1), Some(42));
    assert_eq!(read("18446744073709551615", 1), Some(u64::MAX));
    for bad in [
        "",
        ".",
        "-1",
        "1.5",
        "1e3",
        "ten",
        "1,000",
        "18446744073709551616",
    ] {
        assert_eq!(read(bad, 1), None, "{bad:?}");
    }
    assert_eq!(read("0.0000001", MILLIONTHS), None);
    assert_eq!(read("18446744073709551615", MILLIONTHS), None);
}

#[test]
fn every_setting_has_one_field_or_named_choice_and_round_trips() {
    let hardware = computer();
    for row in sovra_core::training::BASE_MODELS {
        for preset in TrainingPreset::ALL {
            let settings = TrainingSettings::recommended(
                preset,
                row.architecture,
                SAMPLE_TEXT.len(),
                &hardware,
            );
            let read = read_settings(
                &typed(&settings),
                settings.schedule,
                settings.optimizer,
                &settings,
                row.architecture,
            );
            assert_eq!(read, Ok(settings), "{} {preset:?}", row.title);
        }
    }
    // Changing any one field changes the settings, so no field is decorative.
    let architecture = BaseModel::Small.architecture();
    let settings = TrainingSettings::recommended(
        TrainingPreset::Balanced,
        architecture,
        SAMPLE_TEXT.len(),
        &hardware,
    );
    for knob in Knob::ALL {
        let mut texts = typed(&settings);
        let (low, high) = knob.bounds(architecture);
        let other = if knob.get(&settings) == low {
            high.min(low + 1)
        } else {
            low
        };
        texts[knob.index()] = show(other, knob.scale(), false);
        // Fewer rounds than warm-up rounds is refused by the domain's own check.
        if knob == Knob::Rounds {
            texts[Knob::WarmupRounds.index()] = "0".into();
        }
        let changed = read_settings(
            &texts,
            settings.schedule,
            settings.optimizer,
            &settings,
            architecture,
        )
        .unwrap_or_else(|error| panic!("{knob:?}: {error}"));
        assert_eq!(knob.get(&changed), other, "{knob:?}");
        assert_ne!(changed, settings, "{knob:?}");
    }
    let mut ids: Vec<&str> = Knob::ALL.into_iter().map(Knob::id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), Knob::ALL.len());
}

#[test]
fn a_wrong_field_is_named_in_plain_words() {
    let hardware = computer();
    let architecture = BaseModel::Small.architecture();
    let settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        architecture,
        SAMPLE_TEXT.len(),
        &hardware,
    );
    let with = |knob: Knob, text: &str| {
        let mut texts = typed(&settings);
        texts[knob.index()] = text.into();
        read_settings(
            &texts,
            settings.schedule,
            settings.optimizer,
            &settings,
            architecture,
        )
        .expect_err("the field is wrong")
    };
    assert_eq!(
        with(Knob::Rounds, "5"),
        "Rounds must be between 20 and 100,000."
    );
    assert_eq!(with(Knob::Rounds, ""), "Rounds needs a number.");
    assert_eq!(
        with(Knob::Rounds, "lots"),
        "Rounds needs a number such as 300."
    );
    assert_eq!(
        with(Knob::LearningRate, "11"),
        "Learning rate must be between 0.000001 and 10."
    );
    assert_eq!(
        with(Knob::Threads, "-1"),
        "Processor threads must be between 0 and 64."
    );
    assert_eq!(
        with(Knob::BatchBytes, "64"),
        "Text per packet must be between 65 and 4,096."
    );
    assert_eq!(
        with(Knob::WarmupRounds, "301"),
        "Warm-up rounds must be between 0 and 300."
    );
}

/// Fails to compile when a setting is added to the domain, so Advanced cannot
/// silently fall behind: every field below is a number field or a named choice.
#[test]
fn advanced_covers_every_training_setting() {
    let settings = TrainingSettings::recommended(
        TrainingPreset::Balanced,
        BaseModel::Small.architecture(),
        SAMPLE_TEXT.len(),
        &computer(),
    );
    let TrainingSettings {
        rounds,
        batch_bytes,
        windows,
        local_steps,
        learning_rate_micro,
        schedule: _,
        warmup_rounds,
        optimizer: _,
        weight_decay_micro,
        clip_milli,
        outer_learning_rate_micro,
        check_share_percent,
        check_every,
        sample_temperature_milli,
        sample_bytes,
        seed,
        stop_when_stalled,
        threads,
        cutoff_rounds,
        age_decay_milli,
        lease_seconds,
    } = settings;
    let numbers: [(Knob, u64); 19] = [
        (Knob::Rounds, rounds.into()),
        (Knob::BatchBytes, batch_bytes.into()),
        (Knob::Windows, windows.into()),
        (Knob::LocalSteps, local_steps.into()),
        (Knob::OuterLearningRate, outer_learning_rate_micro.into()),
        (Knob::WarmupRounds, warmup_rounds.into()),
        (Knob::LearningRate, learning_rate_micro.into()),
        (Knob::WeightDecay, weight_decay_micro.into()),
        (Knob::Clip, clip_milli.into()),
        (Knob::CheckShare, check_share_percent.into()),
        (Knob::CheckEvery, check_every.into()),
        (Knob::StopWhenStalled, stop_when_stalled.into()),
        (Knob::SampleTemperature, sample_temperature_milli.into()),
        (Knob::SampleBytes, sample_bytes.into()),
        (Knob::Seed, seed),
        (Knob::Threads, threads.into()),
        (Knob::CutoffRounds, cutoff_rounds.into()),
        (Knob::AgeDecay, age_decay_milli.into()),
        (Knob::LeaseSeconds, lease_seconds.into()),
    ];
    assert_eq!(numbers.map(|(knob, _)| knob), Knob::ALL);
    for (knob, value) in numbers {
        assert_eq!(knob.get(&settings), value, "{knob:?}");
        assert!(!knob.hint().is_empty() && !knob.label().is_empty());
    }
}
