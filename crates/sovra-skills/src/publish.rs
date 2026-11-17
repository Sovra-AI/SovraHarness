//! Build static, bounded catalog pages from self-contained plugin packages.

use std::{
    fs,
    path::{Path, PathBuf},
};

use sovra_core::{
    plugins::{MAX_PLUGIN_BYTES, PluginCatalog, PluginEntry, check_download_url},
    skills::SkillError,
};

use crate::{io_error, read_bytes, reject_symlink};

// Each page also passes the core byte limit after JSON encoding.
const PAGE_SIZE: usize = 48;

/// Checks a publisher's entire source folder and generated catalog pages without
/// creating output, using the same preparation as `publish_catalog`.
pub fn check_catalog(source: &Path, base_url: &str) -> Result<(), SkillError> {
    CatalogBuild::read(source, base_url).map(|_| ())
}

/// Writes a new output folder suitable for GitHub Pages. The output must not
/// exist, so a failed build cannot overwrite an already-published catalog.
pub fn publish_catalog(source: &Path, output: &Path, base_url: &str) -> Result<(), SkillError> {
    let build = CatalogBuild::read(source, base_url)?;
    fs::create_dir(output).map_err(|error| io_error(&error))?;
    let result = (|| {
        fs::create_dir(output.join("packages")).map_err(|error| io_error(&error))?;
        for (entry, path) in &build.entries {
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
        for index in 0..build.pages() {
            fs::write(output.join(page_name(index)), build.page(index)?)
                .map_err(|error| io_error(&error))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

struct CatalogBuild {
    base_url: String,
    entries: Vec<(PluginEntry, PathBuf)>,
}

impl CatalogBuild {
    fn read(source: &Path, base_url: &str) -> Result<Self, SkillError> {
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
            let path = entry.map_err(|error| io_error(&error))?.path();
            let item = (|| {
                reject_symlink(&path)?;
                if path.extension().is_none_or(|extension| extension != "json") {
                    return Ok(None);
                }
                let bytes = read_bytes(&path, MAX_PLUGIN_BYTES)?;
                let mut item =
                    PluginEntry::for_package(format!("{base_url}/package.json"), &bytes)?;
                if !ids.insert(item.id.clone()) {
                    return Err(SkillError(format!(
                        "Plugin {} is repeated. Publish one version of each plugin per catalog.",
                        item.id
                    )));
                }
                item.package_url = format!("{base_url}/packages/{}.json", item.sha256);
                item.validate()?;
                Ok(Some(item))
            })()
            .map_err(|error: SkillError| SkillError(format!("{}: {error}", path.display())))?;
            if let Some(item) = item {
                entries.push((item, path));
            }
        }
        entries.sort_by(|(a, _), (b, _)| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
        let build = Self {
            base_url: base_url.into(),
            entries,
        };
        // A read-only check covers the generated pages too, including encoded
        // JSON size and long page URLs, before publication creates any output.
        for index in 0..build.pages() {
            build.page(index)?;
        }
        Ok(build)
    }

    fn pages(&self) -> usize {
        self.entries.len().div_ceil(PAGE_SIZE).max(1)
    }

    fn page(&self, index: usize) -> Result<Vec<u8>, SkillError> {
        let page = PluginCatalog {
            schema_version: 1,
            plugins: self
                .entries
                .iter()
                .skip(index * PAGE_SIZE)
                .take(PAGE_SIZE)
                .map(|(entry, _)| entry.clone())
                .collect(),
            next: (index + 1 < self.pages())
                .then(|| format!("{}/{}", self.base_url, page_name(index + 1))),
            previous: index
                .checked_sub(1)
                .map(|previous| format!("{}/{}", self.base_url, page_name(previous))),
        };
        let bytes = serde_json::to_vec_pretty(&page)
            .map_err(|_| SkillError("Could not encode the plugin catalog.".into()))?;
        PluginCatalog::parse(&bytes)?;
        Ok(bytes)
    }
}

fn page_name(index: usize) -> String {
    if index == 0 {
        "catalog.json".into()
    } else {
        format!("catalog-{}.json", index + 1)
    }
}
