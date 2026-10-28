//! Persisted inclusion choices for a granted folder, separate from its access grant.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    pub include_hidden: bool,
    pub include_generated: bool,
    pub documents: DocumentBudget,
}

/// Per-document working budgets. Streaming source text does not use these limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentBudget {
    LowerMemory,
    #[default]
    Standard,
    Large,
}

impl DocumentBudget {
    pub const ALL: [Self; 3] = [Self::LowerMemory, Self::Standard, Self::Large];

    pub const fn source_bytes(self) -> u64 {
        match self {
            Self::LowerMemory => 64 * 1024 * 1024,
            Self::Standard => 256 * 1024 * 1024,
            Self::Large => 1024 * 1024 * 1024,
        }
    }

    pub const fn text_bytes(self) -> usize {
        match self {
            Self::LowerMemory => 16 * 1024 * 1024,
            Self::Standard => 64 * 1024 * 1024,
            Self::Large => 256 * 1024 * 1024,
        }
    }

    pub const fn expanded_bytes(self) -> u64 {
        self.source_bytes()
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::LowerMemory => "Lower memory · 64 MiB",
            Self::Standard => "Standard · 256 MiB",
            Self::Large => "Large · 1 GiB",
        }
    }

    pub const fn details(self) -> &'static str {
        match self {
            Self::LowerMemory => {
                "Per document: up to 64 MiB source, 16 MiB extracted text and 64 MiB unpacked content."
            }
            Self::Standard => {
                "Per document: up to 256 MiB source, 64 MiB extracted text and 256 MiB unpacked content."
            }
            Self::Large => {
                "Per document: up to 1 GiB source, 256 MiB extracted text and 1 GiB unpacked content."
            }
        }
    }
}

impl ScanOptions {
    /// A reason is also usable in scan notices; inclusion is decided in one place
    /// for scanning, notification registration and notification filtering.
    pub fn exclusion_reason(self, name: &str) -> Option<&'static str> {
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "dev-credentials"
                | "credentials"
                | "secrets"
                | "$recycle.bin"
                | "system volume information"
        ) {
            Some("protected credential or system folder")
        } else if !self.include_hidden && name.starts_with('.') {
            Some("hidden files and folders are switched off")
        } else if !self.include_generated
            && matches!(
                lower.as_str(),
                "target" | "node_modules" | "vendor" | "dist" | "build" | "__pycache__"
            )
        {
            Some("generated folders are switched off")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_budgets_are_explicit_and_old_folder_choices_get_the_standard_budget() {
        let old: ScanOptions =
            serde_json::from_str(r#"{"include_hidden":true,"include_generated":false}"#).unwrap();
        assert_eq!(old.documents, DocumentBudget::Standard);
        for (budget, source, text) in [
            (DocumentBudget::LowerMemory, 64, 16),
            (DocumentBudget::Standard, 256, 64),
            (DocumentBudget::Large, 1024, 256),
        ] {
            assert!(DocumentBudget::ALL.contains(&budget));
            assert_eq!(budget.source_bytes(), source * 1024 * 1024);
            assert_eq!(budget.text_bytes(), text * 1024 * 1024);
            assert_eq!(budget.expanded_bytes(), budget.source_bytes());
            assert!(
                budget
                    .label()
                    .contains(if source == 1024 { "GiB" } else { "MiB" })
            );
            assert!(
                budget
                    .details()
                    .contains(&format!("{text} MiB extracted text"))
            );
            let encoded = serde_json::to_string(&budget).unwrap();
            assert_eq!(
                serde_json::from_str::<DocumentBudget>(&encoded).unwrap(),
                budget
            );
        }
    }
    #[test]
    fn inclusion_choices_are_independent_and_preserve_protected_locations() {
        let default = ScanOptions::default();
        assert!(default.exclusion_reason(".notes").is_some());
        assert!(default.exclusion_reason("TARGET").is_some());
        let hidden = ScanOptions {
            include_hidden: true,
            ..default
        };
        assert_eq!(hidden.exclusion_reason(".notes"), None);
        assert!(hidden.exclusion_reason("target").is_some());
        let all = ScanOptions {
            include_generated: true,
            ..hidden
        };
        assert_eq!(all.exclusion_reason("target"), None);
        assert_eq!(all.exclusion_reason("notes"), None);
        for name in ["dev-credentials", "credentials", "secrets"] {
            assert!(all.exclusion_reason(name).is_some());
        }
        assert_eq!(serde_json::from_str::<ScanOptions>("{}").unwrap(), default);
    }
}
