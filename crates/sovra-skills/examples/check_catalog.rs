//! Read-only plugin catalog validation for publisher CI.

use sovra_core::skills::SkillError;
use std::path::Path;

fn main() -> Result<(), SkillError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [base_url, source] = args.as_slice() else {
        return Err(SkillError(
            "Usage: check_catalog HTTPS_BASE_URL PACKAGE_FOLDER".into(),
        ));
    };
    sovra_skills::check_catalog(Path::new(source), base_url)
}
