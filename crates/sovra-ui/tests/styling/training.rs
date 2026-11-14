//! The Training page: one-click start, choices, expert settings, live metrics,
//! the model library and helping, readable in every layout and both themes.
use super::knowledge_profile::settle;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, ElementId, ScrollDelta, TestAppContext, WindowHandle, point, px, size,
};
use sovra_core::{
    Command, Event, NavTab,
    group::{ContentHash, DeviceId, GroupId},
    models::InterfaceScale,
    training::{
        BaseModel, Delivery, HardwareInfo, HelpingPhase, LibraryModel, LrSchedule, MetricPoint,
        ModelCard, ModelFacts, ModelHeader, ModelOrigin, RunSummary, SAMPLE_TEXT, TrainingBase,
        TrainingCommand, TrainingEngineKind, TrainingEvent, TrainingGroup, TrainingHelp,
        TrainingMetrics, TrainingPeer, TrainingPhase, TrainingPlan, TrainingPreset,
        TrainingProgress, TrainingSettings, TrainingSource, TrainingState, Trial,
    },
};
use sovra_runtime::Client;
use sovra_ui::{Shell, design};

type Commands = async_channel::Receiver<Command>;
type Events = async_channel::Sender<Event>;

const GIB: u64 = 1024 * 1024 * 1024;
const GROUP: GroupId = GroupId([1; 32]);
const HELPER: DeviceId = DeviceId([2; 32]);
const AWAY: DeviceId = DeviceId([3; 32]);

fn reveal(window: &mut gpui_kit::Window, id: impl Into<ElementId>, cx: &mut gpui_kit::App) {
    let id = id.into();
    for _ in 0..40 {
        window.render_frame(cx);
        let element = window.find(id.clone());
        let bounds = element.bounds();
        let pane = window.find("training-page").bounds();
        assert!(
            bounds.left() >= pane.left() - px(1.) && bounds.right() <= pane.right() + px(1.),
            "{id:?} overflows: {bounds:?} vs {pane:?}"
        );
        // Something taller than the pane only has to start inside it.
        let fits = bounds.bottom() <= pane.bottom() || bounds.size.height > pane.size.height;
        if element.visible() && bounds.top() >= pane.top() && fits {
            return;
        }
        // Bring it to the middle of the pane, or to the top when it is tall.
        let room = (pane.size.height - bounds.size.height).max(px(0.)) / 2.;
        let distance = pane.top() + room - bounds.top();
        window.scroll(
            "training-page",
            ScrollDelta::Pixels(point(px(0.), distance)),
            cx,
        );
    }
    panic!("{id:?} could not be reached");
}

fn click(window: &mut gpui_kit::Window, id: impl Into<ElementId>, cx: &mut gpui_kit::App) {
    let id = id.into();
    reveal(window, id.clone(), cx);
    window.click(id, cx);
    window.render_frame(cx);
}

/// Replaces what a field holds, the way a person selects it all and types.
/// The page hears about it once the surrounding window update has finished.
fn retype(window: &mut gpui_kit::Window, id: &'static str, text: &str, cx: &mut gpui_kit::App) {
    click(window, id, cx);
    window.press("end", cx);
    window.press("shift-home", cx);
    window.input(text, cx);
    window.render_frame(cx);
}

fn label(window: &gpui_kit::Window, id: impl Into<ElementId>) -> String {
    let id = id.into();
    window
        .find(id.clone())
        .label()
        .unwrap_or_else(|| panic!("{id:?} has an accessible name"))
        .to_owned()
}

fn take(commands: &Commands) -> TrainingCommand {
    std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            Command::Training(command) => Some(command),
            _ => None,
        })
        .expect("a training command")
}

fn started(commands: &Commands) -> TrainingPlan {
    match take(commands) {
        TrainingCommand::Start(plan) => *plan,
        other => panic!("expected a start, got {other:?}"),
    }
}

fn nothing(commands: &Commands, why: &str) {
    let sent: Vec<Command> = std::iter::from_fn(|| commands.try_recv().ok())
        .filter(|command| matches!(command, Command::Training(_)))
        .collect();
    assert!(sent.is_empty(), "{why}: {sent:?}");
}

fn publish(events: &Events, state: &TrainingState, cx: &mut TestAppContext) {
    events
        .send_blocking(Event::Training(TrainingEvent::new(state.clone())))
        .expect("the training page is open");
    settle(cx);
}

fn open(
    cx: &mut TestAppContext,
    (width, height, scale): (f32, f32, u16),
    mode: ThemeMode,
) -> (WindowHandle<Root>, Commands, Events) {
    let (client, commands, events) = Client::loopback();
    cx.update(|cx| {
        sovra_ui::init(client, cx);
        design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
        Theme::change(mode, None, cx);
    });
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let shell = cx.new(|cx| {
            let mut shell = Shell::new(0, window, cx);
            shell.select_tab(NavTab::Training, cx);
            shell
        });
        Root::new(shell, window, cx)
    });
    assert_eq!(take(&commands), TrainingCommand::Load);
    (window, commands, events)
}

fn hardware() -> HardwareInfo {
    HardwareInfo {
        os: "Test OS 12 with a long edition name".into(),
        arch: "x86_64".into(),
        cpu: "Example Processor 7 3700X Eight-Core With A Long Name".into(),
        physical_cores: 2,
        logical_cores: 4,
        memory_total_bytes: 8 * GIB,
        memory_available_bytes: 3 * GIB,
        engine: TrainingEngineKind::Cpu,
        engine_name: "Built-in processor engine".into(),
        gpus: Vec::new(),
    }
}

fn group() -> TrainingGroup {
    TrainingGroup {
        id: GROUP,
        name: "Our family training group".into(),
        this_device: DeviceId([1; 32]),
        peers: vec![
            TrainingPeer {
                device: HELPER,
                name: "A family member · a computer with a very long friendly name".repeat(2),
                online: true,
                helping: true,
                rounds: 3,
            },
            TrainingPeer {
                device: AWAY,
                name: "Grandad · Old laptop".into(),
                online: false,
                helping: false,
                rounds: 0,
            },
        ],
    }
}

fn model(tag: u8, base: BaseModel, name: &str, origin: ModelOrigin) -> LibraryModel {
    let header = ModelHeader {
        rounds: 300,
        tokens: 76_800,
        loss_milli: Some(1_710),
        check_loss_milli: Some(2_250),
        ..ModelHeader::fresh(base)
    };
    let architecture = header.architecture;
    let facts = ModelFacts {
        id: ContentHash([tag; 32]),
        header,
        parameters: architecture.parameters() as u64,
        bytes: architecture.file_len() as u64,
    };
    let here = origin == ModelOrigin::TrainedHere;
    LibraryModel {
        card: ModelCard::from_facts(
            &facts,
            name.into(),
            "A family member · a computer with a very long friendly name".into(),
            origin,
            (!here).then_some(GROUP),
            u64::from(tag),
        ),
        saved_at: format!("/test/training/models/{}/{tag}.model", "a".repeat(120)),
        delivery: Vec::new(),
    }
}

fn library() -> Vec<LibraryModel> {
    let mut shared = model(
        9,
        BaseModel::Small,
        "A model with a name as long as a name may be, sixty letters.",
        ModelOrigin::TrainedHere,
    );
    shared.card.group = Some(GROUP);
    shared.delivery = vec![
        Delivery {
            device: HELPER,
            name: "A family member · Desktop".into(),
            delivered: true,
        },
        Delivery {
            device: AWAY,
            name: "Grandad · Old laptop".into(),
            delivered: false,
        },
    ];
    vec![
        shared,
        model(
            8,
            BaseModel::Starter,
            "Garden letters",
            ModelOrigin::TrainedHere,
        ),
        model(
            7,
            BaseModel::Medium,
            "Family stories",
            ModelOrigin::Received { from: HELPER },
        ),
    ]
}

fn metrics() -> TrainingMetrics {
    let mut metrics = TrainingMetrics::default();
    for round in 1..=30u32 {
        metrics.push(MetricPoint {
            round,
            loss_milli: 5_500 - round * 100,
            check_loss_milli: (round == 1 || round % 5 == 0).then_some(5_600 - round * 90),
            lr_micro: 5_000,
            tokens_per_second: 27_500,
        });
    }
    metrics.observe(4_000, 2_000_000);
    metrics.set_sample(
        "The garden was qu et and the brds sang in the".as_bytes(),
        30,
    );
    metrics
}

fn ready() -> TrainingState {
    TrainingState {
        phase: TrainingPhase::Ready,
        hardware: Some(hardware()),
        ..TrainingState::default()
    }
}

fn running(plan: &TrainingPlan) -> TrainingState {
    let architecture = BaseModel::Small.architecture();
    TrainingState {
        phase: TrainingPhase::Running,
        progress: TrainingProgress {
            rounds: 30,
            total_rounds: plan.settings.rounds,
            tokens: 7_680,
            loss_milli: Some(2_500),
        },
        saved_at: Some("/test/training/run.train".into()),
        plan: Some(plan.summary(architecture, SAMPLE_TEXT.len())),
        metrics: metrics(),
        ..ready()
    }
}

const SIZES: [(f32, f32, u16); 3] = [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)];
const DESK: (f32, f32, u16) = (1280., 820., 100);

const SETTING_IDS: [&str; 19] = [
    "training-setting-rounds",
    "training-setting-packet",
    "training-setting-windows",
    "training-setting-steps",
    "training-setting-round-step",
    "training-setting-warmup",
    "training-setting-learning-rate",
    "training-setting-weight-decay",
    "training-setting-step-limit",
    "training-setting-check-share",
    "training-setting-check-every",
    "training-setting-stalled",
    "training-setting-variety",
    "training-setting-sample-length",
    "training-setting-seed",
    "training-setting-threads",
    "training-setting-lag",
    "training-setting-lag-discount",
    "training-setting-lease",
];

#[gpui_kit::test]
fn every_part_of_the_training_page_fits_each_layout(cx: &mut TestAppContext) {
    for layout in SIZES {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (window, commands, events) = open(cx, layout, mode);
            // Before the runtime answers, the page already stands.
            cx.update_window(window.into(), |_, window, cx| {
                reveal(window, "training-hardware-advice", cx);
                reveal(window, "training-start", cx);
            })
            .expect("window open");

            let mut state = ready();
            publish(&events, &state, cx);
            cx.update_window(window.into(), |_, window, cx| {
                for id in [
                    "training-hardware-computer",
                    "training-hardware-processor",
                    "training-hardware-cores",
                    "training-hardware-memory",
                    "training-hardware-engine",
                    "training-hardware-advice",
                    "training-refresh",
                    "training-source-sample",
                    "training-source-text",
                    "training-source-file",
                    "training-sample-preview",
                    "training-base-starter",
                    "training-base-small",
                    "training-base-medium",
                    "training-base-large",
                    "training-preset-quick",
                    "training-preset-balanced",
                    "training-preset-thorough",
                    "training-name",
                    "training-models-empty",
                    "training-helper-status",
                ] {
                    reveal(window, id, cx);
                }
                click(window, "training-source-text", cx);
                reveal(window, "training-text", cx);
                reveal(window, "training-plan-problem", cx);
                click(window, "training-source-sample", cx);
                click(window, "training-advanced-toggle", cx);
                for id in SETTING_IDS {
                    reveal(window, id, cx);
                }
                for id in [
                    "training-reset",
                    "training-schedule-steady",
                    "training-schedule-ease",
                    "training-schedule-warmup",
                    "training-optimizer-sgd",
                    "training-optimizer-adam",
                ] {
                    reveal(window, id, cx);
                }
                // The simple path: one press, sent once.
                click(window, "training-start", cx);
                window.click("training-start", cx);
            })
            .expect("window open");
            let plan = started(&commands);
            assert_eq!(plan, TrainingPlan::recommended(&hardware(), false));
            nothing(&commands, "a pending start must not submit twice");

            state = TrainingState {
                group: Some(group()),
                sharing: Some(GROUP),
                helped_rounds: 3,
                models: library(),
                trial: Some(Trial {
                    model: ContentHash([9; 32]),
                    prompt: "The garden".into(),
                    output: Some(" grows quietly behind the house and the birds".repeat(4)),
                    running: false,
                }),
                ..running(&plan)
            };
            publish(&events, &state, cx);
            cx.update_window(window.into(), |_, window, cx| {
                let live = window.find("training-live").bounds();
                let choices = window.find("training-hardware").bounds();
                assert!(
                    live.top() < choices.top(),
                    "a run in progress is shown above the choices"
                );
                for id in [
                    "training-status",
                    "training-chart",
                    "training-tile-round",
                    "training-tile-speed",
                    "training-tile-time",
                    "training-tile-loss",
                    "training-tile-check",
                    "training-tile-helpers",
                    "training-sample",
                    "training-sharing-status",
                    "training-pause",
                    "training-resume",
                    "training-stop",
                    "training-stop-sharing",
                    "training-group-switch",
                    "training-group-members",
                ] {
                    reveal(window, id, cx);
                }
                for index in 0..3usize {
                    reveal(window, ("training-base-continue", index), cx);
                    reveal(window, ("training-model", index), cx);
                    reveal(window, ("training-model-try", index), cx);
                    reveal(window, ("training-model-copy", index), cx);
                }
                reveal(window, ("training-model-delivery", 0usize), cx);
                reveal(window, ("training-model-share", 1usize), cx);
                click(window, ("training-model-try", 0usize), cx);
                reveal(window, "training-try-prompt", cx);
                reveal(window, "training-try-run", cx);
                reveal(window, "training-try-output", cx);
                reveal(window, ("training-help-peer", 0usize), cx);
                click(window, "training-pause", cx);
            })
            .expect("window open");
            assert_eq!(take(&commands), TrainingCommand::Pause);

            state = TrainingState {
                phase: TrainingPhase::Complete,
                sharing: None,
                helping: Some(TrainingHelp {
                    group: GROUP,
                    peer: group().peers[0].clone(),
                    phase: HelpingPhase::Working,
                    accepted_rounds: 12,
                    progress: Some(TrainingProgress {
                        rounds: 40,
                        total_rounds: 800,
                        tokens: 20_480,
                        loss_milli: Some(3_100),
                    }),
                    run: Some(RunSummary {
                        name: "A model with a name as long as a name may be, sixty letters.".into(),
                        base: BaseModel::Medium,
                        architecture: BaseModel::Medium.architecture(),
                        rounds: 40,
                        total_rounds: 800,
                    }),
                    notice: Some("The other computer is offline. Waiting for it to return.".into()),
                }),
                notice: Some(
                    "Family stories from A family member · Desktop is now in Your models.".into(),
                ),
                error: Some(
                    "Training could not finish this step. The last saved step is kept.".into(),
                ),
                ..state
            };
            publish(&events, &state, cx);
            cx.update_window(window.into(), |_, window, cx| {
                let live = window.find("training-live").bounds();
                let choices = window.find("training-hardware").bounds();
                assert!(
                    live.top() > choices.top(),
                    "a finished run makes way for the next one's choices"
                );
                for id in [
                    "training-notice",
                    "training-error",
                    "training-helper-status",
                ] {
                    reveal(window, id, cx);
                }
                click(window, "training-stop-help", cx);
            })
            .expect("window open");
            assert_eq!(take(&commands), TrainingCommand::StopHelping);
        }
    }
}

#[gpui_kit::test]
fn preset_starting_point_and_group_switch_change_the_plan(cx: &mut TestAppContext) {
    let (window, commands, events) = open(cx, DESK, ThemeMode::Light);
    let state = TrainingState {
        group: Some(group()),
        models: library(),
        ..ready()
    };
    publish(&events, &state, cx);
    // In a group the switch starts on, so one press trains with the group.
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            label(window, "training-group-switch"),
            "Train with my group: on"
        );
        assert_eq!(
            label(window, "training-preset-balanced"),
            "Balanced: chosen"
        );
        assert_eq!(label(window, "training-base-small"), "Small: chosen");
        assert!(label(window, "training-group-members").contains("1 helping now"));
        click(window, "training-start", cx);
    })
    .expect("window open");
    assert_eq!(
        started(&commands),
        TrainingPlan::recommended(&hardware(), true)
    );

    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-preset-quick", cx);
        click(window, "training-base-starter", cx);
        click(window, "training-group-switch", cx);
        assert_eq!(
            label(window, "training-group-switch"),
            "Train with my group: off"
        );
        click(window, "training-start", cx);
    })
    .expect("window open");
    let plan = started(&commands);
    assert_eq!(
        plan,
        TrainingPlan {
            name: "My Starter model".into(),
            base: TrainingBase::Fresh(BaseModel::Starter),
            source: TrainingSource::Sample,
            preset: Some(TrainingPreset::Quick),
            settings: TrainingSettings::recommended(
                TrainingPreset::Quick,
                BaseModel::Starter.architecture(),
                SAMPLE_TEXT.len(),
                &hardware(),
            ),
            with_group: false,
        }
    );

    // Continuing a library model keeps its shape; Large never trains with the group.
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-group-switch", cx);
        click(window, "training-preset-thorough", cx);
        click(window, ("training-base-continue", 2usize), cx);
        click(window, "training-start", cx);
    })
    .expect("window open");
    let plan = started(&commands);
    assert_eq!(plan.base, TrainingBase::Continue(ContentHash([7; 32])));
    assert_eq!(plan.name, "Family stories, continued");
    assert!(plan.with_group);
    assert_eq!(
        plan.settings,
        TrainingSettings::recommended(
            TrainingPreset::Thorough,
            BaseModel::Medium.architecture(),
            SAMPLE_TEXT.len(),
            &hardware(),
        )
    );

    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-base-large", cx);
        assert_eq!(window.find("training-group-switch").disabled(), Some(true));
        retype(window, "training-name", "Big one", cx);
    })
    .expect("window open");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-start", cx);
    })
    .expect("window open");
    let plan = started(&commands);
    assert_eq!(plan.base, TrainingBase::Fresh(BaseModel::Large));
    assert_eq!(plan.name, "Big one");
    assert!(!plan.with_group);
}

#[gpui_kit::test]
fn advanced_edits_reach_the_plan_and_a_wrong_value_blocks_start(cx: &mut TestAppContext) {
    let (window, commands, events) = open(cx, DESK, ThemeMode::Dark);
    let state = ready();
    publish(&events, &state, cx);
    let recommended = TrainingPlan::recommended(&hardware(), false);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(window.try_find("training-setting-rounds").is_none());
        click(window, "training-advanced-toggle", cx);
        assert!(window.try_find("training-preset-custom").is_none());
        retype(window, "training-setting-rounds", "1234", cx);
        retype(window, "training-setting-round-step", "0.002", cx);
        retype(window, "training-setting-seed", "7", cx);
    })
    .expect("window open");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-schedule-steady", cx);
        click(window, "training-optimizer-sgd", cx);
        // Editing makes the preset Custom.
        reveal(window, "training-preset-custom", cx);
        assert_eq!(label(window, "training-preset-balanced"), "Choose Balanced");
        click(window, "training-start", cx);
    })
    .expect("window open");
    let plan = started(&commands);
    assert_eq!(plan.preset, None);
    assert_eq!(
        plan.settings,
        TrainingSettings {
            rounds: 1234,
            outer_learning_rate_micro: 2_000,
            seed: 7,
            schedule: LrSchedule::Constant,
            optimizer: sovra_core::training::LocalOptimizer::Sgd,
            ..recommended.settings
        }
    );

    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        retype(window, "training-setting-rounds", "5", cx);
    })
    .expect("window open");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        let problem = label(window, "training-plan-problem");
        assert_eq!(problem, "Rounds must be between 20 and 100,000.");
        assert_eq!(label(window, "training-advanced-problem"), problem);
        assert_eq!(window.find("training-start").disabled(), Some(true));
        click(window, "training-start", cx);
    })
    .expect("window open");
    nothing(&commands, "a wrong setting must block Start");

    cx.update_window(window.into(), |_, window, cx| {
        // Letters are not accepted by a number field; an empty one says so.
        retype(window, "training-setting-rounds", "lots", cx);
        click(window, "training-setting-rounds", cx);
        window.press("end", cx);
        window.press("shift-home", cx);
        window.press("backspace", cx);
    })
    .expect("window open");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            label(window, "training-plan-problem"),
            "Rounds needs a number."
        );
        click(window, "training-reset", cx);
        assert!(window.try_find("training-plan-problem").is_none());
        assert!(window.try_find("training-preset-custom").is_none());
        assert_eq!(
            label(window, "training-preset-balanced"),
            "Balanced: chosen"
        );
        click(window, "training-start", cx);
    })
    .expect("window open");
    assert_eq!(started(&commands), recommended);
}

#[gpui_kit::test]
fn pasted_text_and_a_chosen_file_become_the_plans_source(cx: &mut TestAppContext) {
    let (window, commands, events) = open(cx, DESK, ThemeMode::Light);
    let state = ready();
    publish(&events, &state, cx);
    let pasted = "Once upon a time there was a small model that learned to write.";
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-source-text", cx);
        assert!(
            window
                .elements()
                .iter()
                .any(|element| element.label() == Some("Text for training"))
        );
        assert!(label(window, "training-plan-problem").starts_with("Paste a few sentences"));
        click(window, "training-start", cx);
    })
    .expect("window open");
    nothing(&commands, "no text, no start");
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-text", cx);
        window.input(pasted, cx);
        window.render_frame(cx);
    })
    .expect("window open");
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            window
                .try_find("training-plan-problem")
                .and_then(|problem| problem.label().map(str::to_owned)),
            None
        );
        click(window, "training-start", cx);
    })
    .expect("window open");
    assert_eq!(
        started(&commands).source,
        TrainingSource::Text(pasted.into())
    );

    // The file is picked with the system dialog; only its path is passed on.
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-source-file", cx);
    })
    .expect("window open");
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec!["/notes/diary.txt".into()])
    });
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            label(window, "training-file-name"),
            "Chosen file: diary.txt"
        );
        click(window, "training-choose-file", cx);
    })
    .expect("window open");
    cx.simulate_path_prompt_response(|_| Some(vec!["/notes/holiday.png".into()]));
    settle(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(label(window, "training-file-notice").contains(".txt or .md"));
        assert_eq!(
            label(window, "training-file-name"),
            "Chosen file: diary.txt"
        );
        click(window, "training-start", cx);
    })
    .expect("window open");
    assert_eq!(
        started(&commands).source,
        TrainingSource::File("/notes/diary.txt".into())
    );
}

#[gpui_kit::test]
fn live_metrics_and_run_controls_follow_the_runtime(cx: &mut TestAppContext) {
    let (window, commands, events) = open(cx, DESK, ThemeMode::Dark);
    let plan = TrainingPlan::recommended(&hardware(), true);
    let mut state = TrainingState {
        group: Some(group()),
        sharing: Some(GROUP),
        helped_rounds: 3,
        ..running(&plan)
    };
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            label(window, "training-status"),
            "Learning with your group · round 30 of 452."
        );
        assert_eq!(label(window, "training-tile-round"), "Round: 30 of 452");
        assert_eq!(
            label(window, "training-tile-speed"),
            "Speed: 27,500 letters a second"
        );
        assert_eq!(
            label(window, "training-tile-time"),
            "Time left: about a minute"
        );
        assert_eq!(label(window, "training-tile-loss"), "Loss: 2.500");
        assert_eq!(label(window, "training-tile-check"), "Check loss: 2.900");
        assert_eq!(
            label(window, "training-tile-helpers"),
            "Helpers: 1 helping now, 3 rounds given"
        );
        assert_eq!(
            label(window, "training-chart"),
            "Loss chart. At round 30 the learning loss is 2.500 and the check loss is 2.900."
        );
        assert!(window.find("training-chart").visible());
        assert_eq!(
            label(window, "training-sample"),
            "The garden was qu et and the brds sang in the"
        );
        assert!(label(window, "training-sharing-status").contains("3 rounds"));
        // Choices are locked while it runs, and a finished run cannot resume.
        assert_eq!(window.find("training-start").disabled(), Some(true));
        click(window, "training-resume", cx);
        click(window, "training-stop-sharing", cx);
    })
    .expect("window open");
    assert_eq!(take(&commands), TrainingCommand::StopSharing);
    nothing(&commands, "a running run has nothing to resume");

    state.sharing = None;
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(window.try_find("training-stop-sharing").is_none());
        click(window, "training-pause", cx);
    })
    .expect("window open");
    assert_eq!(take(&commands), TrainingCommand::Pause);

    state.phase = TrainingPhase::Paused;
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(label(window, "training-status").starts_with("Paused"));
        click(window, "training-pause", cx);
        click(window, "training-resume", cx);
    })
    .expect("window open");
    assert_eq!(take(&commands), TrainingCommand::Resume);
    nothing(&commands, "a paused run cannot be paused again");

    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, "training-stop", cx);
    })
    .expect("window open");
    assert_eq!(take(&commands), TrainingCommand::Stop);

    state.phase = TrainingPhase::Complete;
    state.progress.rounds = state.progress.total_rounds;
    state.metrics.eta_ms = None;
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(label(window, "training-status").starts_with("Finished"));
        assert_eq!(label(window, "training-tile-time"), "Time left: Done");
        click(window, "training-resume", cx);
        click(window, "training-pause", cx);
        click(window, "training-stop", cx);
        // The next run can start straight away.
        assert_ne!(window.find("training-start").disabled(), Some(true));
    })
    .expect("window open");
    nothing(&commands, "a finished run takes no more run commands");

    // A run with no held-back text draws one line and says the check is off.
    let mut unchecked = plan;
    unchecked.settings.check_share_percent = 0;
    let mut state = running(&unchecked);
    state.metrics = TrainingMetrics::default();
    for round in 1..=3u32 {
        state.metrics.push(MetricPoint {
            round,
            loss_milli: 5_000 - round * 100,
            check_loss_milli: None,
            lr_micro: 5_000,
            tokens_per_second: 0,
        });
    }
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "training-tile-check"), "Check loss: Off");
        assert_eq!(label(window, "training-tile-speed"), "Speed: Measuring…");
        assert_eq!(
            label(window, "training-chart"),
            "Loss chart. At round 3 the learning loss is 4.700."
        );
        assert!(label(window, "training-sample").starts_with("The first sample"));
    })
    .expect("window open");
}

#[gpui_kit::test]
fn your_models_can_be_tried_shared_and_found(cx: &mut TestAppContext) {
    let (window, commands, events) = open(cx, DESK, ThemeMode::Light);
    let mut state = TrainingState {
        group: Some(group()),
        models: library(),
        ..ready()
    };
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            label(window, ("training-model-delivery", 0usize)),
            "On 1 of 2 computers in your group"
        );
        assert_eq!(
            label(window, ("training-model-delivery", 2usize)),
            "Given to you by your group"
        );
        // Only a model trained here and not yet shared offers Share.
        assert!(window.try_find(("training-model-share", 0usize)).is_none());
        assert!(window.try_find(("training-model-share", 2usize)).is_none());
        click(window, ("training-model-share", 1usize), cx);
    })
    .expect("window open");
    assert_eq!(
        take(&commands),
        TrainingCommand::Share(ContentHash([8; 32]))
    );

    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        click(window, ("training-model-copy", 2usize), cx);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some(state.models[2].saved_at.as_str())
        );
        assert!(window.try_find("training-try-prompt").is_none());
        click(window, ("training-model-try", 2usize), cx);
        click(window, "training-try-prompt", cx);
        window.input("The garden", cx);
        click(window, "training-try-run", cx);
        window.click("training-try-run", cx);
    })
    .expect("window open");
    assert_eq!(
        take(&commands),
        TrainingCommand::Try {
            model: ContentHash([7; 32]),
            prompt: "The garden".into(),
        }
    );
    nothing(&commands, "one trial at a time");

    state.trial = Some(Trial {
        model: ContentHash([7; 32]),
        prompt: "The garden".into(),
        output: None,
        running: true,
    });
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "training-try-output"), "Writing…");
        click(window, "training-try-run", cx);
    })
    .expect("window open");
    nothing(&commands, "a trial in progress is not started again");

    state.trial = Some(Trial {
        model: ContentHash([7; 32]),
        prompt: "The garden".into(),
        output: Some(" grows quietly behind the house.".into()),
        running: false,
    });
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            label(window, "training-try-output"),
            "The garden grows quietly behind the house."
        );
        // Another model's panel does not show this model's answer.
        click(window, ("training-model-try", 1usize), cx);
        assert!(window.try_find("training-try-output").is_none());
    })
    .expect("window open");
}

#[gpui_kit::test]
fn helping_your_group_shows_who_and_stops_on_request(cx: &mut TestAppContext) {
    let (window, commands, events) = open(cx, DESK, ThemeMode::Dark);
    let mut state = TrainingState {
        group: Some(group()),
        ..ready()
    };
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(label(window, "training-helper-status").contains("not helping"));
        assert!(window.try_find("training-stop-help").is_none());
        // Helping one named computer by hand lives in Advanced.
        click(window, "training-advanced-toggle", cx);
        click(window, ("training-help-peer", 1usize), cx);
    })
    .expect("window open");
    assert_eq!(take(&commands), TrainingCommand::Help(AWAY));

    // Help that was asked for by hand holds this computer until it is stopped.
    let peer = group().peers[1].clone();
    state.helping = Some(TrainingHelp {
        group: GROUP,
        peer: peer.clone(),
        phase: HelpingPhase::Waiting,
        accepted_rounds: 3,
        progress: None,
        run: None,
        notice: Some("The other computer is offline. Waiting for it to return.".into()),
    });
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            label(window, "training-helper-status"),
            "Waiting to help Grandad · Old laptop · 3 rounds given"
        );
        assert_eq!(window.find("training-start").disabled(), Some(true));
        click(window, "training-start", cx);
        click(window, "training-stop-help", cx);
    })
    .expect("window open");
    assert_eq!(take(&commands), TrainingCommand::StopHelping);
    nothing(
        &commands,
        "this computer's own run waits while it helps by hand",
    );

    // Help that started by itself gives way the moment the person starts a run.
    state.helping = Some(TrainingHelp {
        group: GROUP,
        peer,
        phase: HelpingPhase::Working,
        accepted_rounds: 12,
        progress: Some(TrainingProgress {
            rounds: 40,
            total_rounds: 800,
            tokens: 20_480,
            loss_milli: Some(3_100),
        }),
        run: Some(RunSummary {
            name: "Family stories".into(),
            base: BaseModel::Medium,
            architecture: BaseModel::Medium.architecture(),
            rounds: 40,
            total_rounds: 800,
        }),
        notice: None,
    });
    publish(&events, &state, cx);
    cx.update_window(window.into(), |_, window, cx| {
        assert_eq!(
            label(window, "training-helper-status"),
            "Helping Grandad · Old laptop · 12 rounds given"
        );
        click(window, "training-start", cx);
    })
    .expect("window open");
    assert_eq!(
        started(&commands),
        TrainingPlan::recommended(&hardware(), true)
    );
}
