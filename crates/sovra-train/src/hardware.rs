//! What this computer is, for the Training page's "This computer" card.
//!
//! Read with `sysinfo` and the standard library only: nothing is run, nothing is
//! sent anywhere. The engine is the graphics card the training engine opened and
//! checked, when there is one, and Burn's built-in processor backend otherwise.

use sovra_core::training::{HardwareInfo, TrainingEngineKind};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Shown as the engine in use.
const PROCESSOR_ENGINE: &str = "Built-in processor engine";
const GRAPHICS_ENGINE: &str = "Graphics card";

fn count(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// `card` is the graphics card training runs on, when it runs on one.
pub(crate) fn probe(card: Option<String>) -> HardwareInfo {
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
        engine: if card.is_some() {
            TrainingEngineKind::Gpu
        } else {
            TrainingEngineKind::Cpu
        },
        engine_name: if card.is_some() {
            GRAPHICS_ENGINE
        } else {
            PROCESSOR_ENGINE
        }
        .into(),
        gpus: card.into_iter().collect(),
    }
    .bounded()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_is_described_without_a_graphics_engine() {
        let info = probe(None);
        assert!(info.logical_cores >= 1);
        assert!(info.physical_cores >= 1);
        assert!(info.physical_cores <= info.logical_cores);
        assert!(info.memory_total_bytes > 0);
        assert!(info.memory_available_bytes <= info.memory_total_bytes);
        assert!(!info.cpu.is_empty() && !info.os.is_empty());
        assert_eq!(info.arch, std::env::consts::ARCH);
        assert_eq!(info.engine, TrainingEngineKind::Cpu);
        assert_eq!(info.engine_name, PROCESSOR_ENGINE);
        assert_eq!(info.gpus, Vec::<String>::new());
        // Asking again is safe and steady.
        assert_eq!(probe(None).cpu, info.cpu);
    }

    #[test]
    fn a_graphics_card_in_use_is_named_as_the_engine() {
        let info = probe(Some("Example Card 9000".into()));
        assert_eq!(info.engine, TrainingEngineKind::Gpu);
        assert_eq!(info.engine_name, GRAPHICS_ENGINE);
        assert_eq!(info.gpus, vec!["Example Card 9000".to_owned()]);
    }
}
