//! Tracing setup: daily-rolling file log, plus stderr in debug builds.
//!
//! Filter with `RUST_LOG` (e.g. `RUST_LOG=sovra_runtime=trace`).

use std::path::Path;

use anyhow::Context as _;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

const DEFAULT_FILTER: &str = "info,sovra_core=debug,sovra_runtime=debug,sovra_ui=debug,sovra=debug";

/// Installs the global subscriber. Keep the returned guard alive for the whole of
/// `main`; dropping it flushes buffered log lines.
pub(crate) fn init(log_dir: &Path) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create log directory {}", log_dir.display()))?;
    let (file_writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(log_dir, "sovra.log"));
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    let stderr = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr).boxed());
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_ansi(false).with_writer(file_writer))
        .with(stderr)
        .try_init()
        .context("failed to install the tracing subscriber")?;
    Ok(guard)
}
