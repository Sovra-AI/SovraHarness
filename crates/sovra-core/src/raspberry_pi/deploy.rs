//! Bounded source bundles and model identity checks for Pi deployment.
use super::{Board, Family};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Python,
    Node,
    Website,
    MicroPython,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiProject {
    pub kind: ProjectKind,
    pub files: BTreeMap<String, String>,
}

impl PiProject {
    pub fn validate(&self, board: &Board) -> Result<(), String> {
        if self.files.is_empty()
            || self.files.len() > 64
            || self.files.values().map(String::len).sum::<usize>() > 2 * 1024 * 1024
        {
            return Err("Keep the project to 1–64 source files and 2 MB.".into());
        }
        for (path, content) in &self.files {
            if path.len() > 160
                || path.split('/').any(|part| {
                    part.is_empty()
                        || part.starts_with('.')
                        || !part
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                })
                || content.len() > 256 * 1024
            {
                return Err("Use relative source file names, with each file under 256 KB.".into());
            }
        }
        if (board.family == Family::Pico) != (self.kind == ProjectKind::MicroPython) {
            return Err(
                "Pico projects need MicroPython; Pi computers need Python, Node or a website."
                    .into(),
            );
        }
        let entry = match self.kind {
            ProjectKind::Python | ProjectKind::MicroPython => "main.py",
            ProjectKind::Node => "main.js",
            ProjectKind::Website => "index.html",
        };
        if !self.files.contains_key(entry) {
            return Err(format!("The project needs {entry}."));
        }
        Ok(())
    }

    pub fn from_workspace(workspace: &crate::CodeWorkspace, board: &Board) -> Result<Self, String> {
        let files: BTreeMap<String, String> = workspace
            .list()
            .into_iter()
            .map(|path| workspace.read(&path).map(|content| (path, content)))
            .collect::<Result<_, _>>()?;
        let kind = if board.family == Family::Pico {
            ProjectKind::MicroPython
        } else if files.contains_key("main.py") {
            ProjectKind::Python
        } else if files.contains_key("main.js") {
            ProjectKind::Node
        } else {
            ProjectKind::Website
        };
        let project = Self { kind, files };
        project.validate(board)?;
        Ok(project)
    }
}

impl Board {
    /// Firmware cannot distinguish soldered headers or Lite storage variants.
    pub fn matches_device_model(&self, model: &str) -> bool {
        let model = model
            .trim()
            .trim_end_matches('\0')
            .split(" Rev ")
            .next()
            .unwrap_or("")
            .split(" with ")
            .next()
            .unwrap_or("");
        let expected = match self.id {
            "pi-1-a" => "Raspberry Pi Model A",
            "pi-1-b" => "Raspberry Pi Model B",
            "pi-1-a-plus" => "Raspberry Pi Model A Plus",
            "pi-1-b-plus" => "Raspberry Pi Model B Plus",
            "pi-5" => "Raspberry Pi 5 Model B",
            "zero-wh" => "Raspberry Pi Zero W",
            "zero-2-wh" => "Raspberry Pi Zero 2 W",
            "pico-h" => "Raspberry Pi Pico",
            "pico-wh" => "Raspberry Pi Pico W",
            "pico-2-headers" => "Raspberry Pi Pico 2",
            "pico-2-w-headers" => "Raspberry Pi Pico 2 W",
            _ => self.name.strip_suffix(" Lite").unwrap_or(self.name),
        };
        model.replace('+', " Plus") == expected.replace('+', " Plus")
            || (self.id == "cm-1" && model == "Raspberry Pi Compute Module")
    }
}

/// Adapter port used after the coding run has durably saved its source.
#[async_trait::async_trait]
pub trait PiDeployment: Send + Sync + std::fmt::Debug {
    fn board_id(&self) -> &str;
    async fn check(&self) -> Result<(), String>;
    async fn deploy(&self, project: PiProject) -> Result<String, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raspberry_pi_projects_reject_paths_wrong_devices_and_partial_builds() {
        let board = super::super::board("pi-5").unwrap();
        let mut project = PiProject {
            kind: ProjectKind::Python,
            files: BTreeMap::from([("main.py".into(), "print('Hi')".into())]),
        };
        project.validate(board).unwrap();
        for path in ["../secret", "/tmp/x", ".ssh/key", "a\\b", "a//b"] {
            project.files.insert(path.into(), "x".into());
            assert!(project.validate(board).is_err());
            project.files.remove(path);
        }
        assert!(
            project
                .validate(super::super::board("pico").unwrap())
                .is_err()
        );
        assert!(board.matches_device_model("Raspberry Pi 5 Model B Rev 1.0"));
        assert!(!board.matches_device_model("Raspberry Pi 4 Model B Rev 1.2"));
        let mut workspace = crate::CodeWorkspace::default();
        workspace.write("main.py", "print('Hi')").unwrap();
        assert_eq!(
            PiProject::from_workspace(&workspace, board).unwrap(),
            project
        );
        assert!(
            super::super::board("pico-2-w-headers")
                .unwrap()
                .matches_device_model("Raspberry Pi Pico 2 W with RP2350")
        );
    }
}
