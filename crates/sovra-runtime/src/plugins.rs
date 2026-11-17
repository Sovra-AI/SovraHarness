//! Static catalog downloads; package admission and persistence stay separate from
//! executable MCP connections. Called by the owned, serialized skills worker.

use std::{sync::Arc, time::Duration};

use sovra_core::plugins::{
    InstalledPlugin, MAX_CATALOG_BYTES, MAX_PLUGIN_BYTES, PluginCatalog, PluginCommand,
    PluginLibrary, PluginStore, check_download_url,
};
use sovra_core::skills::{SkillCatalog, SkillError, SkillListing, SkillStore};

pub(crate) async fn download(url: &str, limit: usize) -> Result<Vec<u8>, SkillError> {
    check_download_url(url)?;
    // Pages URLs are direct static resources. Refuse redirects so a catalog
    // cannot bypass address validation or silently switch download publishers.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| SkillError("The plugin downloader could not start.".into()))?;
    let mut response = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| {
            SkillError(
                "The plugin address could not be reached. Check the address and your connection."
                    .into(),
            )
        })?;
    if !response.status().is_success() {
        return Err(SkillError(format!(
            "The plugin server returned HTTP {}. Use the direct address of the JSON file.",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(SkillError(
            "The plugin download exceeds its size limit.".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SkillError("The plugin download was interrupted.".into()))?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(SkillError(
                "The plugin download exceeds its size limit.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Clone, Copy)]
pub(crate) enum Admission<'a> {
    Recover,
    ValidateAll,
    ValidatePlugin(&'a str),
}

pub(crate) fn listing(
    skills: Option<&dyn SkillStore>,
    library: &PluginLibrary,
    plugins: Option<&dyn PluginStore>,
    admission: Admission<'_>,
) -> Result<SkillListing, SkillError> {
    let mut listing = skills
        .map(SkillStore::list)
        .transpose()?
        .unwrap_or_default();
    let mut all = sovra_core::skills::builtin_skills();
    all.extend(listing.skills.clone());
    SkillCatalog::new(all.clone())?;
    if let Some(store) = plugins {
        let requested = match admission {
            Admission::ValidatePlugin(id) => Some(id),
            Admission::Recover | Admission::ValidateAll => None,
        };
        // Existing valid contributions keep their places. A requested activation
        // is admitted last so exceeding the budget cannot evict another plugin.
        let existing = library
            .installed
            .iter()
            .filter(|plugin| plugin.enabled && Some(plugin.entry.id.as_str()) != requested);
        let candidate = library
            .installed
            .iter()
            .filter(|plugin| plugin.enabled && Some(plugin.entry.id.as_str()) == requested);
        for plugin in existing.chain(candidate) {
            let admitted = store.package(&plugin.entry).and_then(|package| {
                let skills = package.skills(true);
                let mut candidates = all.clone();
                candidates.extend(skills.clone());
                SkillCatalog::new(candidates)?;
                Ok(skills)
            });
            match admitted {
                Ok(skills) => {
                    all.extend(skills.clone());
                    listing.skills.extend(skills);
                }
                Err(error) => {
                    let required = match admission {
                        Admission::Recover => false,
                        Admission::ValidateAll => true,
                        Admission::ValidatePlugin(id) => plugin.entry.id == id,
                    };
                    if required {
                        return Err(error);
                    }
                    listing.issues.push(format!(
                        "{}: {error}. Turn this plugin off or restore its package.",
                        plugin.entry.title
                    ));
                }
            }
        }
    }
    Ok(listing)
}

pub(crate) struct RemoteCatalog {
    root: String,
    catalog: PluginCatalog,
}

pub(crate) async fn handle(
    command: PluginCommand,
    store: Arc<dyn PluginStore>,
    skills: Option<Arc<dyn SkillStore>>,
    catalog: &mut Option<RemoteCatalog>,
) -> Result<(PluginLibrary, Option<sovra_core::plugins::PluginEvent>), SkillError> {
    use sovra_core::plugins::{PluginEvent, check_update};
    let mut discovered = None;
    let mut downloaded = None;
    let root = match &command {
        PluginCommand::Browse { url } => url.clone(),
        PluginCommand::Install { .. } | PluginCommand::NextPage | PluginCommand::PreviousPage => {
            catalog
                .as_ref()
                .ok_or_else(|| SkillError("Reload the plugin catalog first.".into()))?
                .root
                .clone()
        }
        PluginCommand::SetEnabled { .. } | PluginCommand::Inspect { .. } => String::new(),
    };
    match &command {
        PluginCommand::Browse { .. } | PluginCommand::NextPage | PluginCommand::PreviousPage => {
            let url = match &command {
                PluginCommand::Browse { url } => url.clone(),
                PluginCommand::NextPage => catalog
                    .as_ref()
                    .and_then(|page| page.catalog.next.clone())
                    .ok_or_else(|| SkillError("There is no next catalog page.".into()))?,
                PluginCommand::PreviousPage => catalog
                    .as_ref()
                    .and_then(|page| page.catalog.previous.clone())
                    .ok_or_else(|| SkillError("There is no previous catalog page.".into()))?,
                PluginCommand::Install { .. }
                | PluginCommand::SetEnabled { .. }
                | PluginCommand::Inspect { .. } => unreachable!("catalog commands only"),
            };
            if matches!(&command, PluginCommand::Browse { .. }) {
                *catalog = None;
            }
            let next = PluginCatalog::parse(&download(&url, MAX_CATALOG_BYTES).await?)?;
            discovered = Some(PluginEvent::Catalog { url, catalog: next });
        }
        PluginCommand::Install { id } => {
            let entry = catalog
                .as_ref()
                .and_then(|page| page.catalog.plugins.iter().find(|entry| &entry.id == id))
                .ok_or_else(|| SkillError("Reload the plugin catalog before installing.".into()))?
                .clone();
            if entry.api_version != 1 {
                return Err(SkillError(
                    "This plugin requires a different Sovra plugin API version.".into(),
                ));
            }
            let bytes = download(&entry.package_url, MAX_PLUGIN_BYTES).await?;
            entry.verify_package(&bytes)?;
            downloaded = Some((entry, bytes));
        }
        PluginCommand::SetEnabled { .. } | PluginCommand::Inspect { .. } => {}
    }
    let index_root = root.clone();
    let (library, details) = tokio::task::spawn_blocking(move || {
        let mut library = store.load()?;
        let mut details = None;
        let mut required_plugin = None;
        match command {
            PluginCommand::Browse { url } => library.catalog_url = Some(url),
            PluginCommand::Install { .. } => {
                let (entry, bytes) = downloaded
                    .expect("install downloads and verifies its package before persistence");
                let mut plugin = InstalledPlugin {
                    entry,
                    catalog_url: index_root,
                    enabled: false,
                };
                if let Some(existing) = library
                    .installed
                    .iter_mut()
                    .find(|existing| existing.entry.id == plugin.entry.id)
                {
                    check_update(existing, &plugin.entry, &plugin.catalog_url)?;
                    plugin.enabled = existing.enabled;
                    *existing = plugin.clone();
                } else {
                    library.installed.push(plugin.clone());
                }
                if plugin.enabled {
                    required_plugin = Some(plugin.entry.id.clone());
                }
                library.validate()?;
                store.cache(&plugin.entry, &bytes)?;
            }
            PluginCommand::SetEnabled { id, enabled } => {
                let plugin = library
                    .installed
                    .iter_mut()
                    .find(|plugin| plugin.entry.id == id)
                    .ok_or_else(|| SkillError("This plugin is no longer installed.".into()))?;
                plugin.enabled = enabled;
                if enabled {
                    required_plugin = Some(id);
                }
            }
            PluginCommand::Inspect { id } => {
                let plugin = library
                    .installed
                    .iter()
                    .find(|plugin| plugin.entry.id == id)
                    .ok_or_else(|| SkillError("This plugin is no longer installed.".into()))?;
                details = Some(PluginEvent::Details {
                    id,
                    package: store.package(&plugin.entry)?,
                });
                return Ok::<_, SkillError>((library, details));
            }
            PluginCommand::NextPage | PluginCommand::PreviousPage => return Ok((library, details)),
        }
        library.validate()?;
        let admission = required_plugin
            .as_deref()
            .map_or(Admission::Recover, Admission::ValidatePlugin);
        listing(skills.as_deref(), &library, Some(store.as_ref()), admission)?;
        store.save(&library)?;
        Ok((library, details))
    })
    .await
    .map_err(|_| SkillError("The plugin storage worker stopped. Try again.".into()))??;
    if let Some(PluginEvent::Catalog { catalog: found, .. }) = &discovered {
        *catalog = Some(RemoteCatalog {
            root,
            catalog: found.clone(),
        });
    }
    Ok((library, discovered.or(details)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    async fn serve_once(response: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/catalog.json", listener.local_addr().unwrap());
        let task = tokio::runtime::Handle::current().spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            assert!(read > 0);
            socket.write_all(response).await.unwrap();
        });
        (url, task)
    }

    #[tokio::test]
    async fn download_enforces_status_redirect_and_streaming_byte_limits() {
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/escape\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n01234567890123456789".as_slice(),
        ] {
            let (url, server) = serve_once(response).await;
            assert!(download(&url, 10).await.is_err());
            server.await.unwrap();
        }
        let (url, server) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").await;
        assert_eq!(download(&url, 10).await.unwrap(), b"{}");
        server.await.unwrap();
    }
}
