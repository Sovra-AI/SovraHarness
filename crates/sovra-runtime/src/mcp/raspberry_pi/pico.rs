//! `MicroPython` identity and deployment over one explicitly identified USB device.
use super::process;
use sovra_core::raspberry_pi::{Board, PiProject};
use std::path::Path;

#[derive(Debug, Clone)]
pub(super) struct Identity {
    pub(super) port: String,
    pub(super) serial: String,
    pub(super) model: String,
}

async fn usb_identity() -> Result<(String, String), String> {
    let ports = tokio::task::spawn_blocking(serialport::available_ports)
        .await
        .map_err(|_| "Couldn't inspect USB devices.")?
        .map_err(|_| "Couldn't list USB devices.")?;
    let mut found = ports.into_iter().filter_map(|port| match port.port_type {
        serialport::SerialPortType::UsbPort(info) if info.vid == 0x2e8a => {
            info.serial_number.map(|serial| (port.port_name, serial))
        }
        _ => None,
    });
    let identity = found.next().ok_or("Connect your Pico with a data USB cable and install its matching MicroPython firmware first.")?;
    if found.next().is_some() {
        return Err("More than one Pico is connected. Leave only the Pico for this chat connected, then try again.".into());
    }
    Ok(identity)
}

pub(super) async fn connect(board: &Board, program: &Path) -> Result<Identity, String> {
    let (port, serial) = usb_identity().await?;
    let mut command = tokio::process::Command::new(program);
    command.args([
        "connect",
        &port,
        "exec",
        "import os; print(os.uname().machine)",
    ]);
    let output = process::run(command, &[], 20, 4096).await?;
    let model = output.trim().to_owned();
    if !board.matches_device_model(&model) {
        return Err("The Pico firmware reports a different model. Select that model before deploying; check the official firmware if it is wrong.".into());
    }
    Ok(Identity {
        port,
        serial,
        model,
    })
}

pub(super) async fn deploy(
    program: &Path,
    identity: &Identity,
    project: &PiProject,
    folder: &Path,
) -> Result<String, String> {
    let (port, serial) = usb_identity().await?;
    if port != identity.port || serial != identity.serial {
        return Err(
            "The USB device changed while building. Reconnect your Pico before deploying.".into(),
        );
    }
    let mut nonce = [0_u8; 8];
    getrandom::fill(&mut nonce).map_err(|_| "Couldn't create a project identifier.")?;
    let nonce_hex = nonce.iter().fold(String::new(), |mut hex, byte| {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    let release = format!("sovra_{nonce_hex}");
    let mut command = tokio::process::Command::new(program);
    command.args([
        "connect",
        &port,
        "exec",
        &format!("import os; os.mkdir('{release}')"),
    ]);
    process::run(command, &[], 20, 4096).await?;
    let mut directories = std::collections::BTreeSet::new();
    for (path, text) in &project.files {
        let local = folder.join(path);
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| "Couldn't prepare source files.")?;
        }
        tokio::fs::write(&local, text)
            .await
            .map_err(|_| "Couldn't prepare source files.")?;
        let parts: Vec<_> = path.split('/').collect();
        for end in 1..parts.len() {
            let directory = format!("{release}/{}", parts[..end].join("/"));
            if directories.insert(directory.clone()) {
                let mut command = tokio::process::Command::new(program);
                command.args([
                    "connect",
                    &port,
                    "resume",
                    "exec",
                    &format!("import os; os.mkdir('{directory}')"),
                ]);
                process::run(command, &[], 20, 4096).await?;
            }
        }
        let mut command = tokio::process::Command::new(program);
        command
            .args(["connect", &port, "resume", "fs", "cp"])
            .arg(&local)
            .arg(format!(":{release}/{path}"));
        process::run(command, &[], 20, 4096).await?;
    }
    let loader = format!(
        "import os,sys\nos.chdir('/{release}')\nsys.path.insert(0, '/{release}')\nexec(open('main.py').read(), globals())\n"
    );
    let loader_json =
        serde_json::to_string(&loader).map_err(|_| "Couldn't prepare the Pico launcher.")?;
    let activate = format!(
        "import os\nwith open('{release}/launcher.py', 'w') as f: f.write({loader_json})\nif 'main.py' in os.listdir(): os.rename('main.py', '{release}/previous_main.py')\ntry:\n os.rename('{release}/launcher.py', 'main.py')\nexcept OSError:\n if 'previous_main.py' in os.listdir('{release}'): os.rename('{release}/previous_main.py', 'main.py')\n raise\nprint('SOVRA_INSTALLED_{release}')"
    );
    let mut command = tokio::process::Command::new(program);
    command.args(["connect", &port, "resume", "exec", &activate]);
    let receipt = process::run(command, &[], 20, 4096).await?;
    if !receipt.contains(&format!("SOVRA_INSTALLED_{release}")) {
        return Err("The Pico did not confirm installation. Check it before retrying.".into());
    }
    let mut command = tokio::process::Command::new(program);
    command.args(["connect", &port, "resume", "reset"]);
    process::run(command, &[], 20, 4096).await?;
    Ok(format!(
        "Saved the project in /{release} and sent a restart to your Pico. The previous main.py, if present, is kept in that folder. A restart acknowledgement does not confirm the project's physical effects; check the expected light or output."
    ))
}
