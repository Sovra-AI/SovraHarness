//! First-time sign-in decisions. Passwords stay in commands, never screen events.

use super::{Family, PiSetup};
use crate::models::ModelSecret;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

pub fn validate_pairing(setup: &PiSetup, password: &ModelSecret) -> Result<(), String> {
    let board = setup.validate()?;
    if board.family == Family::Pico {
        return Err("Pico boards connect by USB and don't need a network password.".into());
    }
    if setup.host.is_empty() {
        return Err("Choose your Pi's computer name and enter your account first.".into());
    }
    if password.0.is_empty() || password.0.len() > 256 || password.0.contains('\0') {
        return Err("Enter your Pi account password (up to 256 bytes).".into());
    }
    Ok(())
}

/// Stable, filename-safe, bounded identifier. A removed profile cannot erase a
/// different model/account's saved sign-in. The destination is checked again on read.
pub fn credential_id(setup: &PiSetup) -> String {
    let identity = format!(
        "{}\0{}\0{}",
        setup.board_id,
        setup.host.to_ascii_lowercase(),
        setup.user
    );
    Sha256::digest(identity.as_bytes())
        .iter()
        .fold("raspberry-pi-".to_owned(), |mut id, byte| {
            let _ = write!(id, "{byte:02x}");
            id
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingEvent {
    Started {
        request_id: u64,
        setup: PiSetup,
    },
    ConfirmIdentity {
        request_id: u64,
        fingerprint: String,
    },
    Finished {
        request_id: u64,
        result: Result<(), String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PairingPhase {
    #[default]
    Idle,
    Checking,
    ConfirmIdentity {
        fingerprint: String,
    },
    Connected,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct PairingState {
    pub request_id: Option<u64>,
    pub setup: Option<PiSetup>,
    pub phase: PairingPhase,
}

impl PairingState {
    pub fn begin(&mut self, setup: PiSetup) {
        self.request_id = None;
        self.setup = Some(setup);
        self.phase = PairingPhase::Checking;
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            PairingPhase::Checking | PairingPhase::ConfirmIdentity { .. }
        )
    }

    pub fn apply(&mut self, event: &PairingEvent) {
        match event {
            PairingEvent::Started { request_id, setup }
                if self.request_id.is_none_or(|current| *request_id > current) =>
            {
                self.request_id = Some(*request_id);
                self.setup = Some(setup.clone());
                self.phase = PairingPhase::Checking;
            }
            PairingEvent::ConfirmIdentity {
                request_id,
                fingerprint,
            } if self.request_id == Some(*request_id)
                && matches!(self.phase, PairingPhase::Checking) =>
            {
                self.phase = PairingPhase::ConfirmIdentity {
                    fingerprint: fingerprint.clone(),
                };
            }
            PairingEvent::Finished { request_id, result }
                if self.request_id == Some(*request_id) && self.busy() =>
            {
                self.phase = match result {
                    Ok(()) => PairingPhase::Connected,
                    Err(problem) => PairingPhase::Failed(problem.clone()),
                };
            }
            PairingEvent::Started { .. }
            | PairingEvent::ConfirmIdentity { .. }
            | PairingEvent::Finished { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> PiSetup {
        PiSetup {
            board_id: "pi-5".into(),
            host: "gardenpi.local".into(),
            user: "gardener".into(),
        }
    }

    #[test]
    fn pairing_requires_a_computer_account_and_keeps_password_spaces() {
        assert!(validate_pairing(&setup(), &ModelSecret(" secret with spaces ".into())).is_ok());
        for password in [String::new(), "x".repeat(257), "a\0b".into()] {
            assert!(validate_pairing(&setup(), &ModelSecret(password)).is_err());
        }
        let empty = PiSetup {
            host: String::new(),
            user: String::new(),
            ..setup()
        };
        assert!(validate_pairing(&empty, &ModelSecret("test".into())).is_err());
        let pico = PiSetup {
            board_id: "pico".into(),
            ..empty
        };
        assert!(validate_pairing(&pico, &ModelSecret("test".into())).is_err());
    }

    #[test]
    fn credential_ids_are_private_bounded_and_specific_to_the_saved_profile() {
        let original = credential_id(&setup());
        assert_eq!(original.len(), 77);
        assert!(!original.contains("gardener"));
        assert_eq!(
            original,
            credential_id(&PiSetup {
                host: "GARDENPI.LOCAL".into(),
                ..setup()
            })
        );
        assert_ne!(
            original,
            credential_id(&PiSetup {
                user: "other".into(),
                ..setup()
            })
        );
        assert_ne!(
            original,
            credential_id(&PiSetup {
                board_id: "pi-4-b".into(),
                ..setup()
            })
        );
    }

    #[test]
    fn a_late_confirmation_or_result_cannot_replace_a_new_pairing() {
        let mut state = PairingState::default();
        state.begin(setup());
        assert!(state.busy());
        state.apply(&PairingEvent::Started {
            request_id: 2,
            setup: setup(),
        });
        state.apply(&PairingEvent::ConfirmIdentity {
            request_id: 1,
            fingerprint: "old".into(),
        });
        state.apply(&PairingEvent::Finished {
            request_id: 1,
            result: Ok(()),
        });
        assert_eq!(state.phase, PairingPhase::Checking);
        state.apply(&PairingEvent::ConfirmIdentity {
            request_id: 2,
            fingerprint: "SHA256:example".into(),
        });
        assert!(state.busy());
        state.apply(&PairingEvent::Finished {
            request_id: 2,
            result: Err("Sign-in failed".into()),
        });
        assert!(!state.busy());
        state.apply(&PairingEvent::Finished {
            request_id: 2,
            result: Ok(()),
        });
        assert_eq!(state.phase, PairingPhase::Failed("Sign-in failed".into()));
        state.apply(&PairingEvent::Started {
            request_id: 3,
            setup: setup(),
        });
        state.apply(&PairingEvent::Finished {
            request_id: 3,
            result: Ok(()),
        });
        assert_eq!(state.phase, PairingPhase::Connected);
    }
}
