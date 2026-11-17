//! Static plugin catalog builder; validates the same contract as the app.

use sovra_core::skills::SkillError;
use std::path::Path;

fn main() -> Result<(), SkillError> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [base_url, source, output] = args.as_slice() else {
        return Err(SkillError(
            "Usage: publish_catalog HTTPS_BASE_URL PACKAGE_FOLDER NEW_OUTPUT_FOLDER".into(),
        ));
    };
    sovra_skills::publish_catalog(Path::new(source), Path::new(output), base_url)
}
