//! Measures training on this computer, through the same ports the app uses.
//!
//! ```text
//! cargo run --release -p sovra-train --features gpu --example training_benchmark -- --help
//! ```
//!
//! Every run is a real training run: a plan from the recommended settings, a
//! session from [`BurnTrainingEngine`], and one `advance` per round. Nothing is
//! mocked, so the numbers include the checks and sample text a real run writes.
//! Always build with `--release`: a debug build trains several times slower.
//!
//! `--textbook yes` follows each transformer run with the reference trainer
//! ([`sovra_train::reference`]) doing the same number of steps, so the two can
//! be compared like for like.

use std::io::Write as _;
use std::time::Instant;

use sovra_core::group::ContentHash;
use sovra_core::training::{
    Architecture, BaseModel, HardwareInfo, LocalOptimizer, LrSchedule, SAMPLE_TEXT, TrainingBase,
    TrainingEngine, TrainingPlan, TrainingPreset, TrainingSettings, TrainingSource,
};
use sovra_train::{BurnTrainingEngine, EngineChoice, reference};

type Failure = Box<dyn std::error::Error>;

const HELP: &str = "\
Measures Sovra training on this computer.

Options (each takes one value):
  --models  starter,small,medium,large   which starting points (default: all)
  --preset  quick|balanced|thorough      recommended settings to use (default: quick)
  --threads N                            processor threads; 0 lets Sovra decide (default: 0)
  --text    PATH                         learn from this UTF-8 file instead of the sample
  --seed    N                            starting number for random choices (default: 42)
  --engine  auto|processor               auto uses a working graphics card (default: auto)
  --set     NAME=VALUE                   change one recommended setting; may be repeated.
                                         Names: rounds, batch_bytes, windows, local_steps,
                                         learning_rate_micro, outer_learning_rate_micro,
                                         warmup_rounds, weight_decay_micro, clip_milli,
                                         check_every, check_share_percent,
                                         optimizer (sgd or adam) and
                                         schedule (constant, cosine or warmup)
  --textbook yes|no                      after each Small, Medium or Large run, train the same
                                         model the textbook way for as many steps (default: no)
";

struct Options {
    models: Vec<BaseModel>,
    preset: TrainingPreset,
    threads: u8,
    text: String,
    seed: u64,
    engine: EngineChoice,
    /// Recommended settings to replace, in the order given.
    changes: Vec<(String, String)>,
    textbook: bool,
}

fn options() -> Result<Option<Options>, Failure> {
    let mut parsed = Options {
        models: BaseModel::ALL.to_vec(),
        preset: TrainingPreset::Quick,
        threads: 0,
        text: SAMPLE_TEXT.to_owned(),
        seed: 42,
        engine: EngineChoice::Automatic,
        changes: Vec::new(),
        textbook: false,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(name) = arguments.next() {
        if name == "--help" || name == "-h" {
            return Ok(None);
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{name} needs a value"))?;
        match name.as_str() {
            "--models" => {
                parsed.models = value
                    .split(',')
                    .map(|word| match word.trim() {
                        "starter" => Ok(BaseModel::Starter),
                        "small" => Ok(BaseModel::Small),
                        "medium" => Ok(BaseModel::Medium),
                        "large" => Ok(BaseModel::Large),
                        other => Err(format!("unknown model {other}")),
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--preset" => {
                parsed.preset = match value.as_str() {
                    "quick" => TrainingPreset::Quick,
                    "balanced" => TrainingPreset::Balanced,
                    "thorough" => TrainingPreset::Thorough,
                    other => return Err(format!("unknown preset {other}").into()),
                };
            }
            "--threads" => parsed.threads = value.parse()?,
            "--text" => parsed.text = std::fs::read_to_string(&value)?,
            "--seed" => parsed.seed = value.parse()?,
            "--set" => {
                let (name, number) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--set needs NAME=VALUE, not {value}"))?;
                parsed.changes.push((name.to_owned(), number.to_owned()));
            }
            "--textbook" => {
                parsed.textbook = match value.as_str() {
                    "yes" => true,
                    "no" => false,
                    other => return Err(format!("--textbook takes yes or no, not {other}").into()),
                };
            }
            "--engine" => {
                parsed.engine = match value.as_str() {
                    "auto" => EngineChoice::Automatic,
                    "processor" => EngineChoice::Processor,
                    other => return Err(format!("unknown engine {other}").into()),
                };
            }
            other => return Err(format!("unknown option {other}").into()),
        }
    }
    Ok(Some(parsed))
}

/// What one run measured.
struct Measured {
    model: &'static str,
    rounds: u32,
    /// Creating the session and finishing the first round, which pays for any
    /// one-time setup.
    first_round_ms: f64,
    /// The quickest round: the closest to what an idle computer manages.
    fastest_round_ms: f64,
    median_round_ms: f64,
    total_seconds: f64,
    /// Next-byte guesses a second over every round after the first.
    steady_speed: f64,
    loss: Option<f64>,
    check_loss: Option<f64>,
}

fn thousandths(value: Option<u32>) -> Option<f64> {
    value.map(|milli| f64::from(milli) / 1000.0)
}

fn seconds(from: Instant) -> f64 {
    from.elapsed().as_secs_f64()
}

/// Replaces one recommended setting. The plan is still checked as a whole when
/// the run is created, so a value out of range is refused there.
fn change(
    settings: &mut TrainingSettings,
    architecture: Architecture,
    name: &str,
    value: &str,
) -> Result<(), Failure> {
    match name {
        "rounds" => {
            settings.rounds = value.parse()?;
            settings.warmup_rounds = settings.warmup_rounds.min(settings.rounds);
        }
        "batch_bytes" => settings.batch_bytes = value.parse()?,
        // The Starter table always reads its packet as one window.
        "windows" if architecture.is_table() => {}
        "windows" => settings.windows = value.parse()?,
        "local_steps" => settings.local_steps = value.parse()?,
        "learning_rate_micro" => settings.learning_rate_micro = value.parse()?,
        "outer_learning_rate_micro" => settings.outer_learning_rate_micro = value.parse()?,
        "warmup_rounds" => settings.warmup_rounds = value.parse()?,
        "weight_decay_micro" => settings.weight_decay_micro = value.parse()?,
        "clip_milli" => settings.clip_milli = value.parse()?,
        "check_every" => settings.check_every = value.parse()?,
        "check_share_percent" => settings.check_share_percent = value.parse()?,
        "optimizer" => {
            settings.optimizer = match value {
                "sgd" => LocalOptimizer::Sgd,
                "adam" => LocalOptimizer::Adam,
                other => return Err(format!("unknown optimizer {other}").into()),
            };
        }
        "schedule" => {
            settings.schedule = match value {
                "constant" => LrSchedule::Constant,
                "cosine" => LrSchedule::Cosine,
                "warmup" => LrSchedule::WarmupCosine,
                other => return Err(format!("unknown schedule {other}").into()),
            };
        }
        other => return Err(format!("unknown setting {other}").into()),
    }
    Ok(())
}

/// The settings a run of `model` uses: the preset's for this computer, with the
/// asked-for changes.
fn settings_for(
    hardware: &HardwareInfo,
    model: BaseModel,
    options: &Options,
) -> Result<TrainingSettings, Failure> {
    let architecture = model.architecture();
    let mut settings =
        TrainingSettings::recommended(options.preset, architecture, options.text.len(), hardware);
    for (name, value) in &options.changes {
        change(&mut settings, architecture, name, value)?;
    }
    settings.threads = options.threads;
    settings.seed = options.seed;
    Ok(settings)
}

fn run(
    engine: BurnTrainingEngine,
    settings: TrainingSettings,
    model: BaseModel,
    options: &Options,
) -> Result<Measured, Failure> {
    let source = if options.text == SAMPLE_TEXT {
        TrainingSource::Sample
    } else {
        TrainingSource::Text(options.text.clone())
    };
    let plan = TrainingPlan {
        name: format!("Benchmark {}", model.info().title),
        base: TrainingBase::Fresh(model),
        source,
        preset: None,
        settings,
        with_group: false,
    };
    let started = Instant::now();
    let run = ContentHash::of(format!("benchmark {model:?} {}", options.seed).as_bytes());
    let mut session = engine.create(&plan, &options.text, None, run)?;
    let mut round_ms = Vec::new();
    let mut first_round_ms = 0.0;
    let mut tokens_after_first = 0_u64;
    let mut seconds_after_first = 0.0;
    let mut now_ms = 0_u64;
    while !session.progress().is_complete() {
        let before = session.progress().tokens;
        let round_started = Instant::now();
        now_ms += 1;
        session.advance(now_ms)?;
        let took = seconds(round_started);
        if round_ms.is_empty() {
            first_round_ms = seconds(started) * 1000.0;
        } else {
            tokens_after_first += session.progress().tokens - before;
            seconds_after_first += took;
        }
        round_ms.push(took * 1000.0);
    }
    let total_seconds = seconds(started);
    let progress = session.progress();
    let check_loss = session
        .metrics()
        .and_then(|point| thousandths(point.check_loss_milli));
    let mut sorted = round_ms.clone();
    sorted.sort_by(f64::total_cmp);
    #[expect(
        clippy::cast_precision_loss,
        reason = "token counts in a benchmark are far below 2^52"
    )]
    let steady_speed = if seconds_after_first > 0.0 {
        tokens_after_first as f64 / seconds_after_first
    } else {
        0.0
    };
    Ok(Measured {
        model: model.info().title,
        rounds: progress.rounds,
        first_round_ms,
        fastest_round_ms: sorted.first().copied().unwrap_or(0.0),
        median_round_ms: sorted.get(sorted.len() / 2).copied().unwrap_or(0.0),
        total_seconds,
        steady_speed,
        loss: thousandths(progress.loss_milli),
        check_loss,
    })
}

fn loss(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_owned(), |loss| format!("{loss:.3}"))
}

fn main() -> Result<(), Failure> {
    let mut out = std::io::stdout().lock();
    let Some(options) = options()? else {
        out.write_all(HELP.as_bytes())?;
        return Ok(());
    };
    let engine = BurnTrainingEngine::new(options.engine);
    let hardware = engine.hardware();
    let graphics = engine.graphics_card().map_or_else(
        || "none in use".to_owned(),
        |card| {
            format!(
                "{} ({:.1} times the processor's speed when checked)",
                card.name, card.speed_up
            )
        },
    );
    writeln!(
        out,
        "Computer: {}, {} cores / {} threads\nEngine:   {}\nGraphics: {}\nText:     {} bytes, preset {}\n",
        hardware.cpu,
        hardware.physical_cores,
        hardware.logical_cores,
        hardware.engine_name,
        graphics,
        options.text.len(),
        options.preset.title(),
    )?;
    writeln!(
        out,
        "| Model | Rounds | First round | Fastest round | Median round | Whole run | Guesses a second | Loss | Check loss |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    )?;
    for model in &options.models {
        let settings = settings_for(&hardware, *model, &options)?;
        let measured = run(engine, settings.clone(), *model, &options)?;
        writeln!(
            out,
            "| {} | {} | {:.0} ms | {:.1} ms | {:.1} ms | {:.1} s | {:.0} | {} | {} |",
            measured.model,
            measured.rounds,
            measured.first_round_ms,
            measured.fastest_round_ms,
            measured.median_round_ms,
            measured.total_seconds,
            measured.steady_speed,
            loss(measured.loss),
            loss(measured.check_loss),
        )?;
        out.flush()?;
        if options.textbook && !model.architecture().is_table() {
            let steps = settings.rounds.saturating_mul(settings.local_steps);
            let textbook =
                reference::train(options.engine, *model, &settings, &options.text, steps)?;
            let guesses = f64::from(steps)
                * f64::from(settings.windows)
                * f64::from(model.architecture().context);
            writeln!(
                out,
                "| {}, textbook | {} | - | - | - | {:.1} s | {:.0} | {:.3} | {} |",
                measured.model,
                textbook.steps,
                textbook.seconds,
                guesses / textbook.seconds.max(f64::EPSILON),
                textbook.loss,
                loss(textbook.check_loss.map(f64::from)),
            )?;
            out.flush()?;
        }
    }
    Ok(())
}
