//! Bounded read-only device checks. No arbitrary commands or destinations enter
//! tool arguments. SSH host authentication is never weakened for convenience.

use super::{PiPrograms, access, process};
use sovra_core::raspberry_pi::{PiSetup, device_model, health_report};

const OUTPUT_LIMIT: u16 = 16 * 1024;
const PROBE: &str = include_str!("health.py");

async fn probe(setup: &PiSetup, programs: &PiPrograms) -> Result<String, String> {
    setup.validate()?;
    if setup.host.is_empty() {
        return Err("Your Pi hasn't been checked yet. Add your Pi's computer name and account in Connection details first. Enter its password to pair, or use an existing trusted sign-in key.".into());
    }
    match access::run_saved(
        programs.credentials.as_ref(),
        setup,
        PROBE.as_bytes(),
        15,
        usize::from(OUTPUT_LIMIT),
    )
    .await?
    {
        Some(output) => Ok(output),
        None => {
            process::run(
                process::ssh(setup),
                PROBE.as_bytes(),
                15,
                u64::from(OUTPUT_LIMIT),
            )
            .await
        }
    }
}

pub(super) async fn check(setup: &PiSetup, programs: &PiPrograms) -> Result<String, String> {
    let output = probe(setup, programs).await?;
    device_model(setup, output.as_bytes())?;
    health_report(output.as_bytes())
}

pub(super) async fn usb() -> Result<String, String> {
    let ports = tokio::task::spawn_blocking(serialport::available_ports).await.map_err(|_| "Couldn't check USB connections. Try again.")?
        .map_err(|_| "Couldn't list USB connections. Check this computer's device permissions and try again.")?;
    let count = ports.iter().filter(|port| matches!(&port.port_type, serialport::SerialPortType::UsbPort(info) if info.vid == 0x2e8a)).count();
    Ok(if count == 0 {
        "No Raspberry Pi USB serial device was found. Use a data USB cable and install MicroPython for your exact Pico model if the board is new. A board in BOOTSEL mode appears as a drive, not a serial device. Other firmware may use a different USB identity. Nothing was changed.".into()
    } else {
        format!(
            "Found {count} Raspberry Pi USB serial connection(s). This is USB detection only: it cannot verify the exact Pico model, installed firmware or a running project. Open Thonny to select your board. No port was opened and no project was interrupted."
        )
    })
}

pub(super) async fn model(setup: &PiSetup, programs: &PiPrograms) -> Result<String, String> {
    device_model(setup, probe(setup, programs).await?.as_bytes())
}
