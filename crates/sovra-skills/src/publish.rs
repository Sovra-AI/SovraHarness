//! Build static, bounded catalog pages from self-contained plugin packages.

use std::{fs, path::Path};

use sovra_core::{
    plugins::{MAX_PLUGIN_BYTES, PluginCatalog, PluginEntry, check_download_url},
    skills::SkillError,
};

use crate::{io_error, read_bytes, reject_symlink};

/// Writes a new output folder suitable for GitHub Pages. The output must not
/// exist, so a failed build cannot overwrite an already-published catalog.
pub fn publish_catalog(source: &Path, output: &Path, base_url: &str) -> Result<(), SkillError> {
    // Worst-case metadata still fits the catalog byte budget at this page size.
    const PAGE_SIZE: usize = 48;
    check_download_url(base_url)?;
    reject_symlink(source)?;
    let base_url = base_url.trim_end_matches('/');
    let mut entries = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for (index, entry) in fs::read_dir(source)
        .map_err(|error| io_error(&error))?
        .enumerate()
    {
        if index >= 10_000 {
            return Err(SkillError(
                "A published catalog build supports up to 10,000 source files.".into(),
            ));
        }
        let entry = entry.map_err(|error| io_error(&error))?;
        reject_symlink(&entry.path())?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let bytes = read_bytes(&entry.path(), MAX_PLUGIN_BYTES)?;
        let mut item = PluginEntry::for_package(format!("{base_url}/package.json"), &bytes)?;
        if !ids.insert(item.id.clone()) {
            return Err(SkillError(
                "Publish only one version of each plugin per catalog.".into(),
            ));
        }
        item.package_url = format!("{base_url}/packages/{}.json", item.sha256);
        item.validate()?;
        entries.push((item, entry.path()));
    }
    entries.sort_by(|(a, _), (b, _)| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
    let pages = entries.len().div_ceil(PAGE_SIZE).max(1);
    fs::create_dir(output).map_err(|error| io_error(&error))?;
    let result = (|| {
        fs::create_dir(output.join("packages")).map_err(|error| io_error(&error))?;
        for (entry, path) in &entries {
            let bytes = read_bytes(path, MAX_PLUGIN_BYTES)?;
            entry.verify_package(&bytes)?;
            fs::write(
                output
                    .join("packages")
                    .join(format!("{}.json", entry.sha256)),
                bytes,
            )
            .map_err(|error| io_error(&error))?;
        }
        for index in 0..pages {
            let page = PluginCatalog {
                schema_version: 1,
                plugins: entries
                    .iter()
                    .skip(index * PAGE_SIZE)
                    .take(PAGE_SIZE)
                    .map(|(entry, _)| entry.clone())
                    .collect(),
                next: (index + 1 < pages).then(|| format!("{base_url}/{}", page_name(index + 1))),
                previous: index
                    .checked_sub(1)
                    .map(|previous| format!("{base_url}/{}", page_name(previous))),
            };
            let bytes = serde_json::to_vec_pretty(&page)
                .map_err(|_| SkillError("Could not encode the plugin catalog.".into()))?;
            PluginCatalog::parse(&bytes)?;
            fs::write(output.join(page_name(index)), bytes).map_err(|error| io_error(&error))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

fn page_name(index: usize) -> String {
    if index == 0 {
        "catalog.json".into()
    } else {
        format!("catalog-{}.json", index + 1)
    }
}
