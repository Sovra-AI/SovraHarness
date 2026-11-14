//! Runs driven through the core ports, as the runtime drives them.

use sovra_core::group::Bytes;
use sovra_core::swarm::learn::scavenger::{GridCheckpoint, GridSettings};
use sovra_core::training::{
    Architecture, LocalOptimizer, LrSchedule, SAMPLE_TEXT, SourceKind, TrainingPreset,
    TrainingSettings, TrainingSource, WorkRecipe,
};

use std::fmt::Write as _;

use super::*;

/// A transformer small enough to train in milliseconds. Not a base model: runs
/// start from it the way a run continues any library model.
const MICRO: Architecture = Architecture {
    context: 8,
    width: 8,
    layers: 1,
    heads: 2,
};
const REPEATED: &str = "the cat sat on the mat. ";

fn engine() -> BurnTrainingEngine {
    BurnTrainingEngine::processor()
}

fn repeated() -> String {
    REPEATED.repeat(20)
}

fn micro_file(seed: u64) -> Vec<u8> {
    let header = ModelHeader {
        base: BaseModel::Small,
        architecture: MICRO,
        rounds: 0,
        tokens: 0,
        loss_milli: None,
        check_loss_milli: None,
    };
    encode_model(&header, &init::fresh(MICRO, seed)).unwrap()
}

fn settings(architecture: Architecture, text: &str) -> TrainingSettings {
    let mut settings = TrainingSettings::recommended(
        TrainingPreset::Quick,
        architecture,
        text.len(),
        &HardwareInfo::default(),
    );
    settings.rounds = 40;
    settings.warmup_rounds = 2;
    settings.check_every = 5;
    settings.sample_bytes = 24;
    if !architecture.is_table() {
        // A tiny model can take much larger steps than the real ones.
        settings.outer_learning_rate_micro = 30_000;
    }
    settings
}

fn plan(base: TrainingBase, settings: TrainingSettings) -> TrainingPlan {
    TrainingPlan {
        name: "Test model".into(),
        base,
        source: TrainingSource::Sample,
        preset: None,
        settings,
        with_group: false,
    }
}

fn micro_session(text: &str, run: &[u8]) -> Box<dyn TrainingSession> {
    let file = micro_file(1);
    engine()
        .create(
            &plan(
                TrainingBase::Continue(model_id(&file)),
                settings(MICRO, text),
            ),
            text,
            Some(&file),
            ContentHash::of(run),
        )
        .unwrap()
}

fn starter_session(text: &str, run: &[u8]) -> Box<dyn TrainingSession> {
    engine()
        .create(
            &plan(
                TrainingBase::Fresh(BaseModel::Starter),
                settings(Architecture::STARTER, text),
            ),
            text,
            None,
            ContentHash::of(run),
        )
        .unwrap()
}

/// Stands for a large byte blob in comparisons, so a failure prints two short
/// names instead of megabytes.
fn same(bytes: &[u8]) -> ContentHash {
    ContentHash::of(bytes)
}

fn run_rounds(session: &mut dyn TrainingSession, rounds: u32) {
    for now in 0..rounds {
        session.advance(u64::from(now)).unwrap();
    }
}

#[test]
fn the_small_transformer_learns_a_repetitive_text() {
    let text = repeated();
    let mut settings = settings(BaseModel::Small.architecture(), &text);
    settings.rounds = 20;
    settings.windows = 4;
    settings.check_every = 20;
    settings.outer_learning_rate_micro = 5_000;
    let mut session = engine()
        .create(
            &plan(TrainingBase::Fresh(BaseModel::Small), settings),
            &text,
            None,
            ContentHash::of(b"small"),
        )
        .unwrap();
    session.advance(0).unwrap();
    let first = session.metrics().unwrap();
    // A fresh model finds every byte about equally likely: ln 256 = 5.545.
    assert!((5_000..6_000).contains(&first.loss_milli), "{first:?}");
    assert!(
        first.check_loss_milli.is_some(),
        "checked on the first round"
    );
    assert!(first.tokens_per_second > 0);
    run_rounds(session.as_mut(), 19);
    let progress = session.progress();
    assert!(progress.is_complete());
    assert_eq!((progress.rounds, progress.total_rounds), (20, 20));
    assert_eq!(progress.tokens, 20 * 4 * 64);
    let last = session.metrics().unwrap();
    assert_eq!(last.round, 20);
    assert!(
        last.loss_milli * 10 < first.loss_milli * 7,
        "loss fell from {} to {}",
        first.loss_milli,
        last.loss_milli
    );
    let check = last.check_loss_milli.unwrap();
    assert!(check < first.check_loss_milli.unwrap());
    // Finished runs do nothing more and hand out no work.
    session.advance(21).unwrap();
    assert_eq!(session.progress().rounds, 20);
    assert!(session.assign(DeviceId([1; 32]), 22).is_err());
}

#[test]
fn a_run_shows_checks_samples_speed_and_the_scheduled_step_size() {
    let text = repeated();
    let mut session = micro_session(&text, b"metrics");
    assert_eq!(session.metrics(), None);
    assert_eq!(session.sample(), None);
    assert_eq!(session.progress().loss_milli, None);
    session.advance(0).unwrap();
    let first = session.metrics().unwrap();
    assert_eq!(first.round, 1);
    assert!(first.check_loss_milli.is_some());
    // Round one of a two-round warm-up runs at half the full step.
    assert_eq!(first.lr_micro, 15_000);
    let (round, sample) = session.sample().unwrap();
    assert_eq!(round, 1);
    assert!(sample.starts_with('t') && sample.len() <= 25 * 3);
    session.advance(1).unwrap();
    let second = session.metrics().unwrap();
    assert_eq!(second.round, 2);
    assert_eq!(second.check_loss_milli, None, "not a check round");
    assert_eq!(session.sample().unwrap().0, 1, "the sample is kept");
    run_rounds(session.as_mut(), 3);
    let fifth = session.metrics().unwrap();
    assert_eq!(fifth.round, 5);
    assert!(fifth.check_loss_milli.is_some());
    assert_eq!(session.sample().unwrap().0, 5);
    run_rounds(session.as_mut(), 35);
    let last = session.metrics().unwrap();
    assert!(last.loss_milli < first.loss_milli, "{first:?} -> {last:?}");
    assert!(last.check_loss_milli.unwrap() < first.check_loss_milli.unwrap());
    // The cosine winds the step down to about a tenth by the end.
    assert!((3_000..3_200).contains(&last.lr_micro), "{last:?}");
    let summary = session.plan();
    assert_eq!(summary.architecture, MICRO);
    assert_eq!(summary.name, "Test model");
    assert_eq!(summary.source.kind, SourceKind::Sample);
    assert_eq!(summary.source.bytes, text.len() as u64);
}

#[test]
fn a_saved_run_restores_text_position_model_optimizer_and_page_state() {
    let text = repeated();
    let mut original = micro_session(&text, b"save");
    run_rounds(original.as_mut(), 6);
    let bytes = original.checkpoint().unwrap();
    let mut restored = engine().restore(&bytes).unwrap();
    assert_eq!(same(&restored.checkpoint().unwrap()), same(&bytes));
    assert_eq!(restored.progress(), original.progress());
    assert_eq!(restored.plan(), original.plan());
    assert_eq!(restored.metrics(), original.metrics());
    assert_eq!(restored.sample(), original.sample());
    assert_eq!(
        same(&restored.export().unwrap()),
        same(&original.export().unwrap())
    );
    for now in 6..12 {
        original.advance(now).unwrap();
        restored.advance(now).unwrap();
    }
    assert_eq!(restored.progress(), original.progress());
    assert_eq!(
        same(&restored.export().unwrap()),
        same(&original.export().unwrap())
    );
    // Speed is measured again after a restart, so only it may differ.
    let strip = |point: Option<MetricPoint>| MetricPoint {
        tokens_per_second: 0,
        ..point.unwrap()
    };
    assert_eq!(strip(restored.metrics()), strip(original.metrics()));
}

#[test]
fn damaged_saved_runs_are_refused() {
    let text = repeated();
    let mut session = micro_session(&text, b"damage");
    run_rounds(session.as_mut(), 2);
    let bytes = session.checkpoint().unwrap();
    assert!(engine().restore(&bytes).is_ok());
    assert!(engine().restore(&[]).is_err());
    assert!(engine().restore(&bytes[..bytes.len() / 2]).is_err());
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(engine().restore(&longer).is_err());
    let mut version = bytes.clone();
    version[8] = 9;
    assert!(engine().restore(&version).is_err());
    // The text is tied to the header: changing a byte of it is noticed.
    let saved = saved::decode(&bytes).unwrap();
    let at = bytes
        .windows(text.len())
        .position(|window| window == text.as_bytes())
        .unwrap();
    let mut changed = bytes;
    changed[at] = b'T';
    assert!(engine().restore(&changed).is_err());
    // A header that disagrees with the rest is refused, whichever field it is.
    let refit = |edit: &dyn Fn(&mut Header)| {
        let mut header = saved.header.clone();
        edit(&mut header);
        saved::encode(&header, &saved.text, &saved.grid).unwrap()
    };
    assert!(engine().restore(&refit(&|_| {})).is_ok());
    assert!(engine().restore(&refit(&|h| h.cursor = 1 << 40)).is_err());
    assert!(
        engine()
            .restore(&refit(&|h| h.plan.settings.rounds = 1))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(&|h| h.plan.settings.local_steps = 3))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(&|h| h.plan.source.bytes += 1))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(&|h| h.family = BaseModel::Starter))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(&|h| h.best_check = Some((1, 99))))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(
                &|h| h.plan.architecture = BaseModel::Small.architecture()
            ))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(&|h| h.sample = Some((1, "x".repeat(600)))))
            .is_err()
    );
    assert!(
        engine()
            .restore(&refit(&|h| h.plan.settings.batch_bytes = 4))
            .is_err()
    );
}

#[test]
fn a_finished_model_can_be_inspected_tried_and_continued() {
    let text = repeated();
    let mut session = micro_session(&text, b"export");
    run_rounds(session.as_mut(), 40);
    let progress = session.progress();
    let file = session.export().unwrap();
    assert_eq!(file.len(), MICRO.file_len());
    let facts = engine().inspect(&file).unwrap();
    assert_eq!(facts.id, model_id(&file));
    assert_eq!(facts.header.architecture, MICRO);
    assert_eq!(facts.header.base, BaseModel::Small);
    assert_eq!(facts.header.rounds, 40);
    assert_eq!(facts.header.tokens, progress.tokens);
    assert_eq!(facts.header.loss_milli, progress.loss_milli);
    assert!(facts.header.check_loss_milli.is_some());
    assert_eq!(facts.parameters, MICRO.parameters() as u64);
    assert!(engine().inspect(&file[..file.len() - 1]).is_err());

    // Trying it: the same request always writes the same text.
    let written = engine().generate(&file, "the c", 40, 800, 7).unwrap();
    assert_eq!(
        written,
        engine().generate(&file, "the c", 40, 800, 7).unwrap()
    );
    assert_ne!(
        written,
        engine().generate(&file, "the c", 40, 800, 8).unwrap()
    );
    assert!(!written.is_empty() && written.len() <= 40 * 3);
    assert_ne!(engine().generate(&file, "", 16, 0, 0).unwrap(), "");
    assert_eq!(engine().generate(&file, "the", 0, 800, 7).unwrap(), "");
    assert!(
        engine()
            .generate(&file, &"x".repeat(513), 8, 800, 7)
            .is_err()
    );
    assert!(engine().generate(&file[1..], "the", 8, 800, 7).is_err());
    // A trained model writes the text's own letters far more often than chance.
    let greedy = engine().generate(&file, "the cat ", 60, 0, 0).unwrap();
    assert!(
        greedy.chars().all(|c| REPEATED.contains(c)),
        "wrote {greedy:?}"
    );

    // Continuing keeps the shape and adds to what the model has already learned.
    let mut more = engine()
        .create(
            &plan(TrainingBase::Continue(facts.id), settings(MICRO, &text)),
            &text,
            Some(&file),
            ContentHash::of(b"continued"),
        )
        .unwrap();
    assert_eq!(more.plan().architecture, MICRO);
    more.advance(0).unwrap();
    let first = more.metrics().unwrap();
    assert!(
        first.loss_milli < 4_000,
        "it starts from what it already knows: {first:?}"
    );
    let continued = engine().inspect(&more.export().unwrap()).unwrap();
    assert_eq!(continued.header.rounds, 41);
    assert_eq!(
        continued.header.tokens,
        progress.tokens + more.progress().tokens
    );
    assert_ne!(continued.id, facts.id);
}

#[test]
fn a_run_only_starts_from_a_plan_and_model_that_agree() {
    let text = repeated();
    let file = micro_file(1);
    let id = model_id(&file);
    let run = ContentHash::of(b"start");
    let good = plan(TrainingBase::Continue(id), settings(MICRO, &text));
    assert!(engine().create(&good, &text, Some(&file), run).is_ok());
    // A model must be given exactly when the plan continues one, and be that one.
    assert!(engine().create(&good, &text, None, run).is_err());
    assert!(
        engine()
            .create(&good, &text, Some(&micro_file(2)), run)
            .is_err()
    );
    assert!(
        engine()
            .create(&good, &text, Some(&file[1..]), run)
            .is_err()
    );
    let fresh = plan(
        TrainingBase::Fresh(BaseModel::Starter),
        settings(Architecture::STARTER, &text),
    );
    assert!(engine().create(&fresh, &text, None, run).is_ok());
    assert!(engine().create(&fresh, &text, Some(&file), run).is_err());
    assert_eq!(
        engine().create(&fresh, "too short", None, run).err(),
        Some(TrainingError::InvalidInput)
    );
    // Settings are checked against the model's real shape.
    let mut bad = good.clone();
    bad.settings.batch_bytes = 8;
    assert!(matches!(
        engine().create(&bad, &text, Some(&file), run).err(),
        Some(TrainingError::InvalidSetting(_))
    ));
    let mut bad = good;
    bad.settings.rounds = 1;
    assert!(engine().create(&bad, &text, Some(&file), run).is_err());
    // The same seed gives the same fresh model and the same run.
    let small = || {
        let mut settings = settings(BaseModel::Small.architecture(), &text);
        settings.rounds = 20;
        engine()
            .create(
                &plan(TrainingBase::Fresh(BaseModel::Small), settings),
                &text,
                None,
                run,
            )
            .unwrap()
            .export()
            .unwrap()
    };
    assert_eq!(same(&small()), same(&small()));
}

#[test]
fn the_thread_setting_changes_speed_only_never_the_result() {
    let text = repeated();
    let file = micro_file(1);
    let run = |threads: u8| {
        let mut settings = settings(MICRO, &text);
        settings.threads = threads;
        let mut session = engine()
            .create(
                &plan(TrainingBase::Continue(model_id(&file)), settings),
                &text,
                Some(&file),
                ContentHash::of(b"threads"),
            )
            .unwrap();
        run_rounds(session.as_mut(), 5);
        (same(&session.export().unwrap()), session.sample())
    };
    let automatic = run(0);
    assert_eq!(automatic, run(1));
    assert_eq!(automatic, run(3));
}

#[test]
fn a_run_that_stops_improving_finishes_early_and_keeps_its_model() {
    let text = repeated();
    let file = micro_file(1);
    let mut settings = settings(MICRO, &text);
    // Steps too small to matter: the check cannot keep improving for long.
    settings.outer_learning_rate_micro = 1;
    settings.learning_rate_micro = 1;
    settings.schedule = LrSchedule::Constant;
    settings.check_every = 1;
    settings.stop_when_stalled = 3;
    settings.rounds = 1_000;
    let mut session = engine()
        .create(
            &plan(TrainingBase::Continue(model_id(&file)), settings),
            &text,
            Some(&file),
            ContentHash::of(b"stall"),
        )
        .unwrap();
    let mut now = 0;
    while !session.progress().is_complete() {
        session.advance(now).unwrap();
        now += 1;
        assert!(now < 200, "the run should have stalled long before");
    }
    let progress = session.progress();
    assert!(progress.rounds < 200);
    assert_eq!(progress.total_rounds, progress.rounds);
    assert!(engine().inspect(&session.export().unwrap()).is_ok());
    // Stopping early survives a restart.
    let restored = engine().restore(&session.checkpoint().unwrap()).unwrap();
    assert!(restored.progress().is_complete());
}

#[test]
fn a_short_text_without_a_held_back_tail_still_trains() {
    let text = "short text, thirty-two bytes ok!";
    assert_eq!(text.len(), 32);
    let file = micro_file(1);
    let mut settings = settings(MICRO, text);
    assert_eq!(settings.check_share_percent, 0);
    settings.rounds = 20;
    let mut session = engine()
        .create(
            &plan(TrainingBase::Continue(model_id(&file)), settings),
            text,
            Some(&file),
            ContentHash::of(b"short"),
        )
        .unwrap();
    run_rounds(session.as_mut(), 20);
    let point = session.metrics().unwrap();
    assert_eq!(point.check_loss_milli, None);
    assert!(session.sample().is_some());
    assert_eq!(
        engine()
            .inspect(&session.export().unwrap())
            .unwrap()
            .header
            .check_loss_milli,
        None
    );
}

#[test]
fn unicode_text_is_cut_into_packets_on_character_boundaries() {
    let text = format!("{}x", "🌱".repeat(128));
    for mut session in [
        micro_session(&text, b"unicode micro"),
        starter_session(&text, b"unicode starter"),
    ] {
        for now in 0..12 {
            let task = session.assign(DeviceId([4; 32]), now).unwrap();
            assert!(task.text.len() >= 2);
            assert!(task.text.len() <= 512);
            session
                .merge(
                    engine().work(task, DeviceId([4; 32])).unwrap(),
                    DeviceId([4; 32]),
                    now,
                )
                .unwrap();
        }
        assert_eq!(session.progress().rounds, 12);
    }
}

#[test]
fn a_transformer_packet_is_short_stretches_from_all_over_the_text() {
    // Numbered lines, so where a stretch came from can be read off it.
    let mut text = String::new();
    for line in 0..400 {
        writeln!(text, "line {line:03} of the garden book.").unwrap();
    }
    let small = BaseModel::Small.architecture();
    let helper = DeviceId([4; 32]);
    let create = || {
        engine()
            .create(
                &plan(
                    TrainingBase::Fresh(BaseModel::Small),
                    settings(small, &text),
                ),
                &text,
                None,
                ContentHash::of(b"scattered"),
            )
            .unwrap()
    };
    let mut session = create();
    let task = session.assign(helper, 0).unwrap();
    let limits = session.plan().settings;
    let wanted = usize::from(limits.windows) * limits.local_steps as usize;
    assert_eq!(task.starts.len(), wanted);
    assert!(task.text.len() <= limits.batch_bytes as usize);
    assert!(task.parameters(helper).is_ok());
    // Every window is a whole stretch of the text exactly as it stands, with the
    // byte after it, and the stretches come from different places.
    let reach = usize::from(small.context) + 1;
    let learned = &text[..limits.check_split(&text)];
    let places: std::collections::BTreeSet<usize> = task
        .window_starts()
        .unwrap()
        .into_iter()
        .map(|start| {
            let stretch = &task.text[start..start + reach];
            let mut found = learned.match_indices(stretch);
            let place = found.next().expect("the stretch is in the learned text").0;
            assert!(found.next().is_none(), "numbered lines make it unique");
            place
        })
        .collect();
    assert_eq!(places.len(), wanted, "no two windows read the same place");
    let spread = places.last().unwrap() - places.first().unwrap();
    assert!(spread > learned.len() / 3, "spread over {spread} bytes");
    // The seed and the packet's number decide it: the same run gives the same
    // packet again, and the next packet is a different one.
    let mut twin = create();
    assert_eq!(twin.assign(helper, 0).unwrap().text, task.text);
    session
        .merge(engine().work(task.clone(), helper).unwrap(), helper, 1)
        .unwrap();
    let next = session.assign(helper, 2).unwrap();
    assert_ne!(next.text, task.text);
    // The owner's own rounds read scattered packets too, and still learn.
    let before = session.progress().tokens;
    session.advance(3).unwrap();
    assert_eq!(
        session.progress().tokens - before,
        (wanted * usize::from(small.context)) as u64
    );
}

#[test]
fn stretches_keep_whole_characters_and_share_a_packet_that_is_too_small() {
    let text = "🌱é".repeat(200);
    let mut rng = Rng::new(3);
    let (packet, starts) = scattered(&text, 8, 12, 4096, &mut rng);
    assert_eq!(starts.len(), 12);
    let distinct: std::collections::BTreeSet<u32> = starts.iter().copied().collect();
    assert_eq!(distinct.len(), 12);
    for start in &starts {
        let start = *start as usize;
        assert!(packet.is_char_boundary(start));
        // A window of eight bytes and the byte after it lie inside the packet.
        assert!(start + 8 < packet.len());
    }
    // Room for only two stretches: the windows take turns on them.
    let (packet, starts) = scattered(&text, 8, 12, 24, &mut rng);
    assert!(packet.len() <= 24);
    assert_eq!(starts.len(), 12);
    assert_eq!(starts[0], starts[2]);
    assert_ne!(starts[0], starts[1]);
    // A text no longer than one window and its next byte is the whole packet.
    let (packet, starts) = scattered("short one", 8, 3, 4096, &mut rng);
    assert_eq!(packet, "short one");
    assert_eq!(starts, vec![0, 0, 0]);
    // The Starter table still reads straight through, overlapping one character.
    assert_eq!(next_stretch("abcdefghij", 0, 4), (0, 4, 3));
    assert_eq!(next_stretch("abcdefghij", 6, 4), (6, 10, 0));
}

#[test]
fn starter_runs_on_recommended_settings_keep_the_first_wire_form() {
    let mut session = starter_session(SAMPLE_TEXT, b"starter");
    let helper = DeviceId([5; 32]);
    let task = session.assign(helper, 0).unwrap();
    assert_eq!(task.recipe, TrainingRecipe::ByteBigramV1);
    assert!(task.text.len() <= 512);
    assert_eq!(task.lease.steps, 4);
    assert_eq!(task.model.0.len(), 65_536 * 4);
    let result = engine().work(task, helper).unwrap();
    session.merge(result, helper, 1).unwrap();
    let first = session.progress().loss_milli.unwrap();
    run_rounds(session.as_mut(), 39);
    assert!(session.progress().is_complete());
    assert!(session.progress().loss_milli.unwrap() < first);
    // Any other Starter settings use the second form, which names the rule.
    let mut custom = settings(Architecture::STARTER, SAMPLE_TEXT);
    custom.optimizer = LocalOptimizer::Adam;
    custom.learning_rate_micro = 20_000;
    let mut session = engine()
        .create(
            &plan(TrainingBase::Fresh(BaseModel::Starter), custom),
            SAMPLE_TEXT,
            None,
            ContentHash::of(b"custom starter"),
        )
        .unwrap();
    let task = session.assign(helper, 0).unwrap();
    assert!(matches!(
        task.recipe,
        TrainingRecipe::ByteModelV2(WorkRecipe {
            optimizer: LocalOptimizer::Adam,
            windows: 1,
            ..
        })
    ));
    session
        .merge(engine().work(task, helper).unwrap(), helper, 1)
        .unwrap();
}

#[test]
fn independent_helpers_handle_dropout_replay_restore_and_distinct_runs() {
    let text = repeated();
    let fast = DeviceId([1; 32]);
    let slow = DeviceId([2; 32]);
    let gone = DeviceId([3; 32]);
    let mut session = micro_session(&text, b"first");
    let slow_task = session.assign(slow, 0).unwrap();
    let abandoned = session.assign(gone, 0).unwrap();
    let first = session.assign(fast, 0).unwrap();
    assert!(matches!(first.recipe, TrainingRecipe::ByteModelV2(_)));
    assert_eq!(first.lease.worker, fast);
    let first = engine().work(first, fast).unwrap();
    assert_eq!(first.tokens, 8 * 8);

    // A changed count, a result from someone else, and a replay are all refused
    // and leave the saved run exactly as it was.
    let before = session.checkpoint().unwrap();
    let mut forged = first.clone();
    forged.tokens -= 1;
    assert!(session.merge(forged, fast, 1).is_err());
    assert!(session.merge(first.clone(), slow, 1).is_err());
    let mut relabelled = first.clone();
    relabelled.lease.worker = slow;
    assert!(session.merge(relabelled, slow, 1).is_err());
    let mut short = first.clone();
    short.pseudo_gradient.0.truncate(16);
    assert!(session.merge(short, fast, 1).is_err());
    let mut poisoned = first.clone();
    poisoned.pseudo_gradient.0[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(session.merge(poisoned, fast, 1).is_err());
    let mut other_run = first.clone();
    other_run.lease.job = ContentHash::of(b"another run");
    assert!(session.merge(other_run, fast, 1).is_err());
    assert_eq!(same(&before), same(&session.checkpoint().unwrap()));

    session.merge(first.clone(), fast, 1).unwrap();
    let saved = session.checkpoint().unwrap();
    assert!(session.merge(first, fast, 1).is_err(), "replay");
    assert_eq!(same(&saved), same(&session.checkpoint().unwrap()));
    let initial = session.progress().loss_milli.unwrap();

    // The slow helper's older packet still counts; nobody waited for it.
    session
        .merge(engine().work(slow_task, slow).unwrap(), slow, 2)
        .unwrap();
    for now in 3..30 {
        let task = session.assign(fast, now).unwrap();
        session
            .merge(engine().work(task, fast).unwrap(), fast, now)
            .unwrap();
    }
    assert!(session.progress().loss_milli.unwrap() < initial);
    assert_eq!(session.progress().rounds, 29);
    // The owner's own rounds and helpers' rounds mix freely.
    session.advance(30).unwrap();
    assert_eq!(session.progress().rounds, 30);

    // In-flight work does not survive a restart: the helper is given a new packet.
    let outstanding = session.assign(fast, 31).unwrap();
    let checkpoint = session.checkpoint().unwrap();
    let mut restored = engine().restore(&checkpoint).unwrap();
    let result = engine().work(outstanding.clone(), fast).unwrap();
    assert!(restored.merge(result.clone(), fast, 32).is_err());
    let replacement = restored.assign(fast, 32).unwrap();
    assert!(replacement.lease.id > outstanding.lease.id);
    restored
        .merge(engine().work(replacement, fast).unwrap(), fast, 33)
        .unwrap();

    // Work from one run means nothing to another, even on the same text.
    let mut another = micro_session(&text, b"second");
    let fresh = another.assign(fast, 31).unwrap();
    assert_ne!(fresh.lease.job, outstanding.lease.job);
    assert!(another.merge(result, fast, 32).is_err());
    // The helper that vanished long ago is too far behind to count.
    let late = engine().work(abandoned, gone).unwrap();
    for now in 34..180 {
        if session.progress().is_complete() {
            break;
        }
        session.advance(now).unwrap();
    }
    assert!(session.merge(late, gone, 200).is_err());
}

#[test]
fn a_dropped_helper_keeps_the_saved_model_and_can_take_a_packet_again() {
    let text = repeated();
    let helper = DeviceId([9; 32]);
    let mut session = micro_session(&text, b"dropout");
    session.advance(0).unwrap();
    let first = session.assign(helper, 1).unwrap();
    let rounds = session.progress().rounds;
    let saved = session.checkpoint().unwrap();
    let model_before = session.export().unwrap();
    let again = session.assign(helper, 2).unwrap();
    // A transformer's packets are drawn afresh from all over the text, so the
    // helper gets as much to do as before, of the same model.
    assert_eq!(again.starts.len(), first.starts.len());
    assert_eq!(again.expected_tokens(), first.expected_tokens());
    assert!(again.parameters(helper).is_ok());
    assert_eq!(same(&again.model.0), same(&first.model.0));
    assert!(again.lease.id > first.lease.id);
    assert_eq!(session.progress().rounds, rounds);
    // Only the lease counter moved: model, optimizer and progress are untouched.
    let reassigned = session.checkpoint().unwrap();
    assert_eq!(saved.len(), reassigned.len());
    let differing = saved
        .iter()
        .zip(&reassigned)
        .filter(|(a, b)| a != b)
        .count();
    assert!((1..=8).contains(&differing), "{differing} bytes differ");
    assert!(
        session
            .merge(engine().work(first, helper).unwrap(), helper, 3)
            .is_err()
    );
    assert_eq!(
        same(&session.checkpoint().unwrap()),
        same(&reassigned),
        "rejected work cannot change the saved run"
    );
    // Abandoning hands the packet back without touching the model.
    session.abandon(helper);
    assert!(
        session
            .merge(engine().work(again.clone(), helper).unwrap(), helper, 3)
            .is_err()
    );
    assert_eq!(same(&session.export().unwrap()), same(&model_before));
    assert_eq!(session.progress().rounds, rounds);
    let third = session.assign(helper, 4).unwrap();
    assert_eq!(third.expected_tokens(), again.expected_tokens());
    session
        .merge(engine().work(third, helper).unwrap(), helper, 5)
        .unwrap();
    assert_eq!(session.progress().rounds, rounds + 1);
    // One helper's packet at a time, and never this computer's own slot.
    assert!(session.assign(LOCAL, 6).is_err());
}

#[test]
fn a_helper_refuses_forged_or_mismatched_packets_before_doing_any_work() {
    let text = repeated();
    let helper = DeviceId([7; 32]);
    let mut session = micro_session(&text, b"forged");
    let task = session.assign(helper, 0).unwrap();
    let work = |task: TrainingTask| engine().work(task, helper);
    assert!(work(task.clone()).is_ok());
    // Meant for someone else.
    assert!(engine().work(task.clone(), DeviceId([8; 32])).is_err());
    let mut changed = task.clone();
    changed.lease.worker = DeviceId([8; 32]);
    assert!(work(changed).is_err());
    // A model that is not the one the assignment names.
    let mut changed = task.clone();
    changed.model.0[0] ^= 1;
    assert!(work(changed).is_err());
    // A model of another size, or a recipe for another shape.
    let mut changed = task.clone();
    changed.model = Bytes(vec![0; 16]);
    assert!(work(changed).is_err());
    let mut changed = task.clone();
    changed.recipe = TrainingRecipe::ByteBigramV1;
    assert!(work(changed).is_err());
    let mut changed = task.clone();
    if let TrainingRecipe::ByteModelV2(recipe) = &mut changed.recipe {
        recipe.architecture.width = 16;
    }
    assert!(work(changed).is_err());
    // More windows or steps than the assignment paid for, and unbounded values.
    for edit in [
        (|recipe: &mut WorkRecipe| recipe.windows = 64) as fn(&mut WorkRecipe),
        |recipe| recipe.windows = 0,
        |recipe| recipe.learning_rate_micro = 0,
        |recipe| recipe.weight_decay_micro = u32::MAX,
        |recipe| recipe.clip_milli = u32::MAX,
        |recipe| recipe.architecture = BaseModel::Large.architecture(),
    ] {
        let mut changed = task.clone();
        if let TrainingRecipe::ByteModelV2(recipe) = &mut changed.recipe {
            edit(recipe);
        }
        assert!(work(changed).is_err());
    }
    let mut changed = task.clone();
    changed.lease.steps = 17;
    assert!(work(changed).is_err());
    let mut changed = task.clone();
    changed.lease.token_budget = 1;
    assert!(work(changed).is_err());
    let mut changed = task.clone();
    changed.text = "x".into();
    assert!(work(changed).is_err());
    let mut changed = task.clone();
    changed.text = "x".repeat(4097);
    assert!(work(changed).is_err());
    // A different seed is honest work on different windows: the owner's count
    // still matches, so it is the recipe bounds, not the seed, that are policed.
    let honest = work(task.clone()).unwrap();
    assert_eq!(honest.lease, task.lease);
    assert_eq!(honest.tokens, task.expected_tokens().unwrap());
    assert_eq!(honest.pseudo_gradient.0.len(), MICRO.parameters() * 4);
    // Doing the same packet twice gives the same answer.
    assert_eq!(
        same(&honest.pseudo_gradient.0),
        same(&work(task).unwrap().pseudo_gradient.0)
    );
}

#[test]
fn a_model_too_large_for_one_message_trains_here_only() {
    let large = BaseModel::Large.architecture();
    assert!(!large.group_capable());
    let text = repeated();
    let mut settings = settings(large, &text);
    settings.batch_bytes = TrainingSettings::min_batch_bytes(large);
    let mut session = engine()
        .create(
            &plan(TrainingBase::Fresh(BaseModel::Large), settings.clone()),
            &text,
            None,
            ContentHash::of(b"large"),
        )
        .unwrap();
    assert!(session.assign(DeviceId([1; 32]), 0).is_err());
    assert_eq!(session.progress().rounds, 0);
    // Asking to train it with the group is refused when the run is created.
    let mut shared = plan(TrainingBase::Fresh(BaseModel::Large), settings);
    shared.with_group = true;
    assert!(
        engine()
            .create(&shared, &text, None, ContentHash::of(b"large shared"))
            .is_err()
    );
}

/// The saved form the first release wrote.
fn first_release_file(version: u32, rounds: u32) -> (Vec<u8>, Box<dyn TrainingSession>) {
    // The run under the name the first release gave it, on that release's fixed
    // settings: the whole text learned from, eighty rounds.
    let run = saved::legacy_recipe(SAMPLE_TEXT);
    let mut plan = plan(
        TrainingBase::Fresh(BaseModel::Starter),
        settings(Architecture::STARTER, SAMPLE_TEXT),
    );
    plan.settings.check_share_percent = 0;
    plan.settings.rounds = 80;
    let mut session = engine().create(&plan, SAMPLE_TEXT, None, run).unwrap();
    run_rounds(session.as_mut(), rounds);
    let now = saved::decode(&session.checkpoint().unwrap()).unwrap();
    let legacy = saved::Legacy {
        version,
        recipe: (version == 2).then_some(run),
        text: SAMPLE_TEXT.into(),
        cursor: usize::try_from(now.header.cursor).unwrap(),
        grid: now.grid,
        loss_milli: now.header.loss_milli,
    };
    (serde_json::to_vec(&legacy).unwrap(), session)
}

#[test]
fn runs_saved_by_the_first_release_carry_on() {
    for version in [1, 2] {
        let (file, mut session) = first_release_file(version, 3);
        assert_eq!(file[0], b'{');
        let mut restored = engine().restore(&file).unwrap();
        assert_eq!(restored.progress(), session.progress());
        let summary = restored.plan();
        assert_eq!(summary.architecture, Architecture::STARTER);
        assert_eq!(summary.settings.rounds, 80);
        assert!(!summary.with_group);
        restored.advance(3).unwrap();
        session.advance(3).unwrap();
        assert_eq!(
            same(&restored.export().unwrap()),
            same(&session.export().unwrap())
        );
        // From here on it is saved in today's form.
        let saved = restored.checkpoint().unwrap();
        assert!(saved.starts_with(b"SOVRARUN"));
        assert_eq!(
            engine().restore(&saved).unwrap().progress(),
            restored.progress()
        );
    }
    let (file, _) = first_release_file(2, 1);
    let mut changed: serde_json::Value = serde_json::from_slice(&file).unwrap();
    changed["text"] = SAMPLE_TEXT.replace("garden", "castle").into();
    assert!(
        engine()
            .restore(&serde_json::to_vec(&changed).unwrap())
            .is_err()
    );
    let mut changed: serde_json::Value = serde_json::from_slice(&file).unwrap();
    changed["version"] = 7.into();
    assert!(
        engine()
            .restore(&serde_json::to_vec(&changed).unwrap())
            .is_err()
    );
    // A version-one file is tied to its text by the run's name.
    let (file, _) = first_release_file(1, 1);
    let mut changed: serde_json::Value = serde_json::from_slice(&file).unwrap();
    changed["text"] = SAMPLE_TEXT.replace("garden", "castle").into();
    assert!(
        engine()
            .restore(&serde_json::to_vec(&changed).unwrap())
            .is_err()
    );
}

#[test]
fn the_merge_state_is_saved_compactly() {
    let text = repeated();
    let session = micro_session(&text, b"compact");
    let bytes = session.checkpoint().unwrap();
    // Twenty bytes a value, the text once, and a small header: no JSON numbers.
    let fixed = GridCheckpoint::byte_len(MICRO.parameters()) + text.len();
    assert!(bytes.len() > fixed);
    assert!(bytes.len() < fixed + 4096, "{} bytes", bytes.len());
    let settings = saved::decode(&bytes).unwrap().header.plan.settings;
    let grid: GridSettings = settings.grid_settings(MICRO);
    assert_eq!(grid.local_steps, 1);
}
