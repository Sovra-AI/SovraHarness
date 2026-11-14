//! What this computer is, for the Training page's "This computer" card.
//!
//! Read with `sysinfo` and the standard library only: nothing is run, nothing is
//! sent anywhere. The engine is always Burn's built-in processor backend, chosen
//! explicitly. A build with the `gpu` feature also lists the graphics cards Burn
//! can see, by name.

use sovra_core::training::{HardwareInfo, TrainingEngineKind};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Shown as the engine in use.
const ENGINE_NAME: &str = "Built-in processor engine";

fn count(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(feature = "gpu")]
fn graphics_cards() -> Vec<String> {
    burn::tensor::Device::enumerate_physical()
        .into_iter()
        .map(|card| card.name)
        .collect()
}

#[cfg(not(feature = "gpu"))]
fn graphics_cards() -> Vec<String> {
    Vec::new()
}

pub(crate) fn probe() -> HardwareInfo {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let logical = match system.cpus().len() {
        // The processor list can come back empty on an unusual system.
        0 => std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        listed => listed,
    };
    let cpu = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .filter(|brand| !brand.is_empty())
        .unwrap_or_else(|| "Unknown processor".into());
    let os = System::long_os_version()
        .or_else(System::name)
        .unwrap_or_else(|| std::env::consts::OS.into());
    HardwareInfo {
        os,
        arch: std::env::consts::ARCH.into(),
        cpu,
        physical_cores: count(System::physical_core_count().unwrap_or(logical)),
        logical_cores: count(logical),
        memory_total_bytes: system.total_memory(),
        memory_available_bytes: system.available_memory(),
        engine: TrainingEngineKind::Cpu,
        engine_name: ENGINE_NAME.into(),
        gpus: graphics_cards(),
    }
    .bounded()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_is_described_without_a_graphics_engine() {
        let info = probe();
        assert!(info.logical_cores >= 1);
        assert!(info.physical_cores >= 1);
        assert!(info.physical_cores <= info.logical_cores);
        assert!(info.memory_total_bytes > 0);
        assert!(info.memory_available_bytes <= info.memory_total_bytes);
        assert!(!info.cpu.is_empty() && !info.os.is_empty());
        assert_eq!(info.arch, std::env::consts::ARCH);
        assert_eq!(info.engine, TrainingEngineKind::Cpu);
        assert_eq!(info.engine_name, ENGINE_NAME);
        #[cfg(not(feature = "gpu"))]
        assert_eq!(info.gpus, Vec::<String>::new());
        // Asking again is safe and steady.
        assert_eq!(probe().cpu, info.cpu);
    }
}
