//! Raspberry Pi plugin profiles and setup rules. Each board uses the existing MCP
//! service lifecycle; model differences are data, never a separate client stack.

mod catalogue;
mod discovery;
pub use discovery::{DiscoveryEvent, DiscoveryState, NetworkCandidate};
mod deploy;
mod gpio;
mod health;
mod pairing;
pub use deploy::{PiDeployment, PiProject, ProjectKind};
pub use pairing::{PairingEvent, PairingPhase, PairingState, credential_id, validate_pairing};
mod projects;
pub use gpio::{Pin, PinKind, PinMap};
pub use health::{connection_problem, device_model, health_report};
#[cfg(test)]
mod tests;

pub use catalogue::BOARDS;
use serde::{Deserialize, Serialize};

pub const SERVER_FLAG: &str = "--raspberry-pi-mcp";
pub const RESEARCHED_ON: &str = "2026-11-26";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Computer,
    Zero,
    Keyboard,
    ComputeModule,
    Pico,
}

impl Family {
    pub const ALL: [Self; 5] = [
        Self::Computer,
        Self::Zero,
        Self::Keyboard,
        Self::ComputeModule,
        Self::Pico,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Computer => "Pi computers",
            Self::Zero => "Pi Zero",
            Self::Keyboard => "Keyboard computers",
            Self::ComputeModule => "Compute Modules",
            Self::Pico => "Pico boards",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Board {
    pub id: &'static str,
    pub name: &'static str,
    pub family: Family,
    pub note: &'static str,
    pub source: &'static str,
}

pub fn board(id: &str) -> Option<&'static Board> {
    BOARDS.iter().find(|board| board.id == id)
}

pub fn search(query: &str, family: Option<Family>) -> Vec<&'static Board> {
    let query = query.trim().to_lowercase();
    BOARDS
        .iter()
        .filter(|board| {
            family.is_none_or(|family| board.family == family)
                && format!(
                    "{} {} {} {}",
                    board.id,
                    board.name,
                    board.family.label(),
                    board.note
                )
                .to_lowercase()
                .contains(&query)
        })
        .collect()
}

/// Only the setup screen supplies a destination. Model tools cannot choose another
/// host, account, executable, command or serial device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiSetup {
    pub board_id: String,
    pub host: String,
    pub user: String,
}

impl PiSetup {
    pub fn validate(&self) -> Result<&'static Board, String> {
        let board = board(&self.board_id).ok_or("Choose a Raspberry Pi model from the list.")?;
        if board.family == Family::Pico && (!self.host.is_empty() || !self.user.is_empty()) {
            return Err("Pico boards connect by USB. Leave the network details empty.".into());
        }
        if self.host.is_empty() && self.user.is_empty() {
            return Ok(board);
        }
        // Strict names also keep the destination safe when OpenSSH handles it.
        if self.host.len() > 253
            || self.host.is_empty()
            || !self.host.as_bytes()[0].is_ascii_alphanumeric()
            || !self
                .host
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'-'))
            || self
                .host
                .split('.')
                .any(|part| part.is_empty() || part.starts_with('-') || part.ends_with('-'))
        {
            return Err("Enter your Pi's name, such as gardenpi.local, or its network address, such as 192.168.1.20.".into());
        }
        if self.user.is_empty()
            || self.user.len() > 32
            || !self
                .user
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
            || self.user.starts_with('-')
        {
            return Err("Enter the account name you chose when setting up your Pi.".into());
        }
        Ok(board)
    }

    pub fn arguments(&self) -> Result<Vec<String>, String> {
        self.validate()?;
        Ok(vec![
            SERVER_FLAG.into(),
            serde_json::to_string(self).map_err(|_| "Couldn't prepare this Pi's setup.")?,
        ])
    }
}

impl Board {
    pub fn setup_steps(&self) -> Vec<&'static str> {
        match self.family {
            Family::Pico => vec![
                "Use a USB cable that carries data. A charging-only cable will power the board but cannot connect it.",
                "For a new board, open the official setup guide below. It introduces MicroPython, the small program that lets your Pico run projects. Choose the download for the exact model printed on your board.",
                "Hold BOOTSEL while connecting the USB cable. Release it when a new drive appears, then copy the matching MicroPython file onto that drive. This replaces the board's built-in software; save any existing project first.",
                "Install the free Thonny app from the official guide. Choose MicroPython (Raspberry Pi Pico) and your board in Thonny, then try the built-in light project. Wireless Pico boards also work over USB.",
                "Use Check USB connection in Sovra to look for a Raspberry Pi USB device. Detection alone cannot confirm which model or program is installed.",
            ],
            Family::ComputeModule => vec![
                "Use a compatible carrier or development board with your module correctly fitted. Switch the power off before attaching anything, and follow that board's connection instructions.",
                "Use the official Compute Module guide to prepare its built-in storage. Lite modules use storage supplied by the carrier board.",
                "In Raspberry Pi Imager, choose your device and a compatible Raspberry Pi OS. Set a memorable computer name and create your own account.",
                "Connect the carrier board to your home network and finish the on-screen setup. Keep a copy of anything already on the storage before replacing its contents.",
                "Setup and project help work immediately. To connect, enable remote access on your Pi, then enter its computer name, account and password in Connection details. Sovra remembers your sign-in; an existing trusted sign-in key also works.",
            ],
            Family::Keyboard => vec![
                "Check whether Raspberry Pi OS is already installed. The Pi 500+ comes with it on the internal drive; 400 and 500 kits include a prepared memory card. Keep that installation to get started.",
                "Only if you have no prepared storage, use Raspberry Pi Imager to write a memory card for your model. Save anything on the card first because writing replaces its contents.",
                "Connect a screen, mouse and the recommended power supply. The keyboard is already part of your computer.",
                "Follow the first-start screens to create your own account and join your home network. Then try the shopping-list project.",
                "Setup and project help work immediately. To connect, enable remote access on your Pi, then enter its computer name, account and password in Connection details. Sovra remembers your sign-in; an existing trusted sign-in key also works.",
            ],
            Family::Computer | Family::Zero => vec![
                "Read the model name printed on your Pi or its case. Choose that model in Raspberry Pi Imager so it offers a compatible operating system.",
                "Prepare a memory card with Raspberry Pi Imager. Choose a computer name, your own account and, where available, your home Wi-Fi. Writing the card replaces its contents, so save anything you need first.",
                "Insert the card, connect a screen and keyboard if you have them, and use the recommended power supply. Models without built-in Wi-Fi need a network cable or a compatible USB network adapter.",
                "Complete the on-screen setup. Ask Sovra for a first project whenever you are ready.",
                "Setup and project help work immediately. To connect, enable remote access on your Pi, then enter its computer name, account and password in Connection details. Sovra remembers your sign-in; an existing trusted sign-in key also works.",
            ],
        }
    }

    pub const fn setup_url(&self) -> &'static str {
        match self.family {
            Family::Pico => {
                "https://www.raspberrypi.com/documentation/microcontrollers/micropython.html"
            }
            Family::ComputeModule => {
                "https://www.raspberrypi.com/documentation/computers/compute-module.html"
            }
            Family::Keyboard => {
                "https://www.raspberrypi.com/documentation/computers/keyboard-computers.html#getting-started"
            }
            Family::Computer | Family::Zero => "https://www.raspberrypi.com/software/",
        }
    }

    pub fn starter_project(&self) -> String {
        if self.family == Family::Pico {
            "Make the built-in light blink. You need only your Pico, a data USB cable and Thonny. With MicroPython installed for your exact model, choose MicroPython (Raspberry Pi Pico) in Thonny, paste this into a new file and press Run. Stop ends the project. This uses only the board's own light, with no external wiring.\n\n```python\nfrom machine import Pin\nfrom time import sleep\n\nlight = Pin('LED', Pin.OUT)\ntry:\n    for _ in range(10):\n        light.on()\n        sleep(0.5)\n        light.off()\n        sleep(0.5)\nfinally:\n    light.off()\n```\n\nSave the code to your library if you want to keep it. Nothing is installed or run by this plugin.".into()
        } else {
            "Make a tiny shopping-list helper. On your Pi, open Thonny (or Python 3), paste this into a new file and press Run. It needs no extra parts, internet account or software packages.\n\n```python\nitems = []\nprint('Type an item, or press Enter to finish.')\nwhile True:\n    item = input('Add to my list: ').strip()\n    if not item:\n        break\n    items.append(item)\nprint('\\nMy shopping list')\nfor number, item in enumerate(items, 1):\n    print(str(number) + '. ' + item)\n```\n\nSave the code and your list to your library to keep them. Nothing is installed or run by this plugin.".into()
        }
    }
}

/// Recognize a bundled profile for presentation. This grants no execution approval;
/// imported programs still go through the existing MCP program review.
pub fn service_setup(service: &crate::mcp::McpService) -> Option<PiSetup> {
    let crate::mcp::McpTransport::Program { args, .. } = &service.transport else {
        return None;
    };
    if args.len() != 2 || args[0] != SERVER_FLAG || args[1].len() > 2048 {
        return None;
    }
    let setup: PiSetup = serde_json::from_str(&args[1]).ok()?;
    setup.validate().ok()?;
    Some(setup)
}

/// A bounded local result panel, separate from chat and from connection status.
#[derive(Debug, Clone, Default)]
pub struct PiResults {
    pub text: std::collections::BTreeMap<String, String>,
    pub pending: std::collections::BTreeSet<String>,
}

impl PiResults {
    pub fn begin(&mut self, service_id: &str) {
        self.text.remove(service_id);
        self.pending.insert(service_id.to_owned());
    }

    pub fn apply(&mut self, event: &crate::mcp::McpEvent) {
        use crate::mcp::McpEvent;
        match event {
            McpEvent::ToolResult {
                service_id,
                text,
                session_id: None,
                ..
            } if self.pending.remove(service_id) => {
                // Bound even a manually imported server pretending to be a profile.
                self.text
                    .insert(service_id.clone(), text.chars().take(16 * 1024).collect());
            }
            McpEvent::Removed { id } => {
                self.pending.remove(id);
                self.text.remove(id);
            }
            McpEvent::Upsert(service)
                if !service.enabled || service.phase == crate::mcp::McpPhase::Failed =>
            {
                if self.pending.remove(&service.id) {
                    self.text.insert(
                        service.id.clone(),
                        "The check stopped. Resume or reconnect the plugin, then try again.".into(),
                    );
                }
            }
            McpEvent::Loaded(state) => {
                self.pending.clear();
                self.text.retain(|id, _| state.service(id).is_some());
            }
            McpEvent::Upsert(_)
            | McpEvent::ToolResult { .. }
            | McpEvent::Notice(_)
            | McpEvent::RaspberryPiPairing(_)
            | McpEvent::RaspberryPiDiscovery(_)
            | McpEvent::OpenSignIn { .. } => {}
        }
    }
}

/// Opens a Pi conversation without claiming the physical device answered.
pub fn begin_chat(state: &mut crate::WorkspaceState, service_id: &str, board_id: &str) -> bool {
    let Some(board) = board(board_id) else {
        return false;
    };
    state.push_service_reply(
        board.name,
        "What would you like to make? Your Pi's pins and setup help are here.",
        Some(crate::AgUiWidget::RaspberryPi {
            service_id: service_id.into(),
            board_id: board_id.into(),
        }),
    );
    true
}
