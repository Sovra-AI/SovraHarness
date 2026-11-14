//! Which engine does the tensor work: the built-in processor engine, always, or
//! a graphics card when this build includes the graphics engine and one works.
//!
//! A card is never assumed. It is opened once per process without panicking,
//! must be a real card (a software renderer is slower than the processor
//! engine), must agree with the processor engine on a small gradient, and must
//! clearly out-run the processor on a small real model before any run uses it:
//! a weak card built into a processor can be the slower of the two. A card that
//! later fails is retired for the rest of the process and the work is done
//! again on the processor.

use burn::tensor::Device;

/// Which engine a [`crate::BurnTrainingEngine`] may use.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EngineChoice {
    /// A working graphics card when there is one, otherwise the processor.
    #[default]
    Automatic,
    /// The built-in processor engine only. The same seed then gives the same
    /// run on every computer, which tests and comparisons rely on.
    Processor,
}

/// One engine's pair of devices: `plain` for reading a loss, `learning` for
/// gradients.
#[derive(Clone)]
pub(crate) struct Engine {
    pub(crate) plain: Device,
    pub(crate) learning: Device,
}

impl Engine {
    pub(crate) fn processor() -> Self {
        Self {
            plain: Device::flex(),
            learning: Device::flex().autodiff(),
        }
    }
}

/// A graphics card that passed its checks.
pub(crate) struct Graphics {
    pub(crate) engine: Engine,
    pub(crate) card: GraphicsCard,
}

/// The graphics card training runs on.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicsCard {
    /// The card as it names itself.
    pub name: String,
    /// How many times more text a second it read than the processor when it was
    /// checked, on the Small model.
    pub speed_up: f64,
}

/// The working graphics card, if this build has the graphics engine, the
/// computer has a card, and it has not failed since.
pub(crate) fn graphics() -> Option<&'static Graphics> {
    imp::graphics()
}

/// Stops using the graphics card for the rest of this process.
pub(crate) fn retire(why: &str) {
    imp::retire(why);
}

/// Hands back the card memory the engine keeps for reuse, if a card was ever
/// opened. Called when a run ends: other programs share the card, and a run
/// that is over should not go on holding its memory.
pub(crate) fn release() {
    imp::release();
}

/// Bytes the engine has reserved on the card, when a card is open and says.
#[cfg(test)]
pub(crate) fn reserved_bytes() -> Option<u64> {
    imp::reserved_bytes()
}

#[cfg(not(feature = "gpu"))]
mod imp {
    use super::Graphics;

    pub(super) fn graphics() -> Option<&'static Graphics> {
        None
    }

    pub(super) fn retire(_why: &str) {}

    pub(super) fn release() {}

    #[cfg(test)]
    pub(super) fn reserved_bytes() -> Option<u64> {
        None
    }
}

#[cfg(feature = "gpu")]
mod imp {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use burn::tensor::{Device, DeviceKind};
    use sovra_core::training::{Architecture, BaseModel};

    use super::{Engine, Graphics, GraphicsCard};
    use crate::init;
    use crate::net::{Batch, Net};

    static RETIRED: AtomicBool = AtomicBool::new(false);
    static CARD: OnceLock<Option<Graphics>> = OnceLock::new();

    /// A transformer small enough to check in milliseconds, but with every kind
    /// of layer the real ones have.
    const CHECK: Architecture = Architecture {
        context: 16,
        width: 16,
        layers: 2,
        heads: 2,
    };
    const CHECK_TEXT: &[u8] = b"the garden grows, the garden rests, and so do we.";
    /// Largest difference allowed between the two engines on the check, for the
    /// loss and for every gradient value. Real differences are near 1e-6.
    const CHECK_TOLERANCE: f32 = 1e-3;

    /// Whether the card computes the same loss and gradient as the processor.
    fn agrees(card: &Engine) -> bool {
        let Ok(net) = Net::new(CHECK) else {
            return false;
        };
        let params = init::fresh(CHECK, 1);
        let Ok(batch) = Batch::windows(CHECK_TEXT, &[0, 7, 19], usize::from(CHECK.context)) else {
            return false;
        };
        let processor = Engine::processor();
        let (Ok(expected), Ok(found)) = (
            net.gradient(&params, &batch, &processor.learning),
            net.gradient(&params, &batch, &card.learning),
        ) else {
            return false;
        };
        let Ok(plain) = net.loss(&params, &batch, &card.plain) else {
            return false;
        };
        (expected.0 - found.0).abs() < CHECK_TOLERANCE
            && (expected.0 - plain).abs() < CHECK_TOLERANCE
            && expected
                .1
                .iter()
                .zip(&found.1)
                .all(|(a, b)| (a - b).abs() < CHECK_TOLERANCE)
    }

    /// Windows in one timed step on each engine: what a run's recommended
    /// settings give the Small model on a card and on a processor.
    const CARD_WINDOWS: usize = 32;
    const PROCESSOR_WINDOWS: usize = 8;
    /// A card is used only when it reads at least this many times more text a
    /// second than the processor. Close to even is not worth it: the processor
    /// is the engine every run is tested on.
    const MIN_SPEED_UP: f64 = 1.5;

    /// The quickest of three steps on `batch`, in seconds, after `warm_ups`
    /// untimed ones (a card compiles its programs on first use).
    fn quickest(
        net: &Net,
        params: &[f32],
        batch: &Batch,
        engine: &Engine,
        warm_ups: usize,
    ) -> Option<f64> {
        for _ in 0..warm_ups {
            net.gradient(params, batch, &engine.learning).ok()?;
        }
        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let started = Instant::now();
            net.gradient(params, batch, &engine.learning).ok()?;
            best = best.min(started.elapsed().as_secs_f64());
        }
        (best > 0.0 && best.is_finite()).then_some(best)
    }

    /// How many times more text a second the card reads than the processor,
    /// measured on the Small model, the one a card helps least.
    fn speed_up(card: &Engine) -> Option<f64> {
        let shape = BaseModel::Small.architecture();
        let net = Net::new(shape).ok()?;
        let params = init::fresh(shape, 1);
        let context = usize::from(shape.context);
        let text: Vec<u8> = CHECK_TEXT.iter().copied().cycle().take(4096).collect();
        let batch = |windows: usize| {
            let starts: Vec<usize> = (0..windows)
                .map(|index| index * 61 % (text.len() - context - 1))
                .collect();
            Batch::windows(&text, &starts, context).ok()
        };
        let card_seconds = quickest(&net, &params, &batch(CARD_WINDOWS)?, card, 2)?;
        let processor_seconds = quickest(
            &net,
            &params,
            &batch(PROCESSOR_WINDOWS)?,
            &Engine::processor(),
            1,
        )?;
        #[expect(clippy::cast_precision_loss, reason = "two small window counts")]
        let ratio =
            (CARD_WINDOWS as f64 / card_seconds) / (PROCESSOR_WINDOWS as f64 / processor_seconds);
        Some(ratio)
    }

    fn open(kind: DeviceKind) -> Option<Graphics> {
        let plain = Device::wgpu_options().device_kind(kind).init().ok()?;
        // A software renderer reports no card behind it.
        let identity = plain.identity()?;
        identity.physical.as_ref()?;
        let name = identity.name.trim().to_owned();
        let engine = Engine {
            learning: plain.clone().autodiff(),
            plain,
        };
        if !agrees(&engine) {
            tracing::warn!(card = %name, "the graphics card's results differ from the processor's, so it is not used");
            return None;
        }
        let speed_up = speed_up(&engine)?;
        if speed_up < MIN_SPEED_UP {
            tracing::info!(card = %name, speed_up, "the graphics card is not clearly faster than the processor, so it is not used");
            return None;
        }
        tracing::info!(card = %name, speed_up, "training uses the graphics card");
        Some(Graphics {
            engine,
            card: GraphicsCard { name, speed_up },
        })
    }

    pub(super) fn graphics() -> Option<&'static Graphics> {
        if RETIRED.load(Ordering::Relaxed) {
            return None;
        }
        CARD.get_or_init(|| {
            // A separate card first; the one built into the processor otherwise.
            [DeviceKind::DiscreteGpu(0), DeviceKind::IntegratedGpu(0)]
                .into_iter()
                .find_map(|kind| {
                    // A driver fault while opening or checking must not take the
                    // app down: it only means the processor does the work.
                    let opened = catch_unwind(AssertUnwindSafe(|| open(kind.clone())));
                    opened.unwrap_or_else(|_| {
                        tracing::warn!(?kind, "the graphics card failed its check");
                        None
                    })
                })
        })
        .as_ref()
    }

    /// The card, only if it has already been opened: never opens one.
    fn opened() -> Option<&'static Graphics> {
        if RETIRED.load(Ordering::Relaxed) {
            return None;
        }
        CARD.get().and_then(Option::as_ref)
    }

    pub(super) fn release() {
        if let Some(card) = opened() {
            // A failing card must not take the app down on the way out either.
            let released = catch_unwind(AssertUnwindSafe(|| {
                card.engine.plain.memory_cleanup();
            }));
            if released.is_err() {
                retire("the graphics card failed while giving memory back");
            }
        }
    }

    #[cfg(test)]
    pub(super) fn reserved_bytes() -> Option<u64> {
        opened()?
            .engine
            .plain
            .memory_pool_usage()
            .map(|usage| usage.bytes_reserved)
    }

    pub(super) fn retire(why: &str) {
        if !RETIRED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                why,
                "training stopped using the graphics card and carries on with the processor"
            );
        }
    }
}
