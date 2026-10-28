//! Persisted inclusion choices for a granted folder, separate from its access grant.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    pub include_hidden: bool,
    pub include_generated: bool,
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
