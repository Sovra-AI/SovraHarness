//! Reads the shipped license documents. The test is the check.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives in the workspace")
            .to_path_buf()
    }

    fn read(root: &Path, name: &str) -> String {
        std::fs::read_to_string(root.join(name)).unwrap_or_else(|_| panic!("read {name}"))
    }

    fn workspace_license(manifest: &str) -> String {
        let mut in_package = false;
        for line in manifest.lines() {
            if line == "[workspace.package]" {
                in_package = true;
                continue;
            }
            if in_package && line.starts_with('[') {
                break;
            }
            if in_package {
                let Some(rest) = line.strip_prefix("license = ") else {
                    continue;
                };
                return rest.trim().trim_matches('"').to_string();
            }
        }
        panic!("workspace license missing from Cargo.toml");
    }

    #[test]
    fn shipped_license_is_sustainable_use_with_a_sovra_enterprise_license() {
        let root = workspace_root();
        let license = read(&root, "LICENSE");
        let enterprise = read(&root, "LICENSE_EE.md");
        let readme = read(&root, "README.md");
        let contributing = read(&root, "CONTRIBUTING.md");
        let security = read(&root, "SECURITY.md");
        let manifest = read(&root, "Cargo.toml");
        let cargo_license = workspace_license(&manifest);

        assert!(license.contains("Sustainable Use License"));
        assert!(license.contains("Version 1.0"));
        assert!(license.contains("The licensor is Sovra"));
        assert!(!license.contains("n8n GmbH"));
        assert!(license.contains(
            "non-exclusive, royalty-free, worldwide, non-sublicensable, non-transferable"
        ));
        assert!(
            license.contains("use, copy, distribute, make available, and prepare derivative works")
        );
        assert!(license.contains("patent claims the licensor can license"));
        assert!(license.contains("modifications or additions to the software"));
        assert!(license.contains(
            "your patent license for the software granted under these terms ends immediately"
        ));
        assert!(license.contains("anyone who gets a copy of any part of the software from you also gets a copy of these terms"));
        assert!(license.contains("prominent notice stating that you have modified the software"));
        assert!(license.contains("do not imply any licenses other than those expressly granted"));
        assert!(license.contains("automatically terminate"));
        assert!(license.contains("30 days"));
        assert!(license.contains("terminate automatically and permanently"));
        assert!(license.contains("comes as is, without any warranty or condition"));
        assert!(license.contains("The \"software\" is the software the licensor makes available"));
        assert!(license.contains("\"You\" refers to the individual or entity"));
        assert!(license.contains("\"Your company\""));
        assert!(license.contains("\"Your license\""));
        assert!(license.contains("\"Use\" means anything you do with the software"));
        assert!(
            license.contains("\"Trademark\" means trademarks, service marks, and similar rights")
        );

        assert!(license.contains(
            "only for your own internal business purposes or for non-commercial or personal use"
        ));
        assert!(license.contains("only if you do so free of charge for non-commercial purposes"));
        assert!(license.contains(
            "may not alter, remove, or obscure any licensing, copyright, or other notices"
        ));
        assert!(license.contains("trademarks is subject to applicable law"));

        assert!(license.contains("default branch (i.e. \"main\") is not licensed"));
        assert!(license.contains("\".ee.\" in their filename"));
        assert!(license.contains("\".ee\" in their dirname"));
        assert!(license.contains("NOT licensed under the Sustainable Use License"));
        assert!(license.contains("LICENSE_EE.md"));
        assert!(license.contains("Sovra Enterprise License"));
        assert!(
            license.contains("original license provided by the owner of the applicable component")
        );

        assert!(enterprise.contains("Sovra Enterprise License"));
        assert!(enterprise.contains("Copyright (c) 2026-present Sovra"));
        assert!(!enterprise.contains("n8n GmbH"));
        assert!(enterprise.contains("may only be used in production"));
        assert!(enterprise.contains("valid Sovra Enterprise license corresponding to your usage"));
        assert!(enterprise.contains("free to modify this Software and publish patches"));
        assert!(enterprise.contains(
            "retain all right, title and interest in and to all such modifications and/or patches"
        ));
        assert!(
            enterprise
                .contains("development and testing purposes, without requiring a subscription")
        );
        assert!(
            enterprise.contains("not granted any other rights beyond what is expressly stated")
        );
        assert!(
            enterprise
                .contains("forbidden to copy, merge, publish, distribute, sublicense, and/or sell")
        );
        assert!(enterprise.contains("PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND"));
        assert!(
            enterprise
                .contains("original license provided by the owner of the applicable component")
        );

        for notice in [&readme, &contributing, &security] {
            let lower = notice.to_lowercase();
            assert!(!lower.contains("closed source") && !lower.contains("closed-source"));
            assert!(!lower.contains("proprietary"));
            assert!(!notice.contains("NOT licensed for public use"));
            assert!(!notice.contains("Do not copy, distribute, or disclose"));
        }
        assert!(readme.contains("fair-code") || readme.contains("Fair-code"));
        assert!(readme.contains("not an OSI open-source license"));
        assert!(readme.contains("n8n's Sustainable Use License"));
        assert!(contributing.contains("fair-code"));
        assert!(contributing.contains("not an OSI open-source license"));

        assert!(cargo_license.starts_with("LicenseRef-"));
        assert!(cargo_license.contains("SustainableUse"));
        assert_ne!(cargo_license, "LicenseRef-Proprietary");
        assert_ne!(cargo_license, "MIT");
        assert_ne!(cargo_license, "Apache-2.0");
    }
}
