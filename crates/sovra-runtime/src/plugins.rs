//! Static catalog downloads; package admission and persistence stay separate from
//! executable MCP connections. Called by the owned, serialized skills worker.

use std::{sync::Arc, time::Duration};

use sovra_core::plugins::{
    GitHubHosts, GitHubPin, GitHubRepository, InstalledPlugin, MAX_CATALOG_BYTES, MAX_PLUGIN_BYTES,
    PluginCatalog, PluginChanges, PluginCommand, PluginEntry, PluginEvent, PluginLibrary,
    PluginStore, check_download_url, check_replacement, require_official, turns_on_at_install,
};
use sovra_core::skills::{SkillCatalog, SkillError, SkillListing, SkillStore};

const ALREADY_ADDED: &str =
    "This repository is already added. Choose Check for update to get its newer changes.";

pub(crate) mod code;
mod github;

/// What the plugin library worker needs: storage, where GitHub is, and the sealed
/// box with the set of running code plugins it keeps current.
pub(crate) struct PluginServices {
    pub(crate) store: Option<Arc<dyn PluginStore>>,
    pub(crate) hosts: GitHubHosts,
    pub(crate) engine: Option<Arc<dyn sovra_core::plugins::PluginCodeEngine>>,
    pub(crate) code: Arc<code::CodeTools>,
}

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
    let response = client
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
    read_limited(response, limit).await
}

/// Reads a body without trusting its declared length.
pub(crate) async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, SkillError> {
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

/// Remote state the worker keeps between commands: the current catalog page and
/// an update the person has been shown but not yet accepted.
pub(crate) struct PluginSession {
    hosts: GitHubHosts,
    catalog: Option<RemoteCatalog>,
    pending: Option<(String, Download)>,
}

impl PluginSession {
    pub(crate) fn new(hosts: GitHubHosts) -> Self {
        Self {
            hosts,
            catalog: None,
            pending: None,
        }
    }
}

/// Verified package bytes and where they came from, ready to persist.
struct Download {
    entry: PluginEntry,
    bytes: Vec<u8>,
    catalog_url: String,
    github: Option<GitHubPin>,
    /// The update lets code run for the first time or reach somewhere new, so it
    /// is installed switched off and the person turns it on again themselves.
    requires_approval: bool,
}

/// Turns a one-click official install into the command that carries it out: adding
/// the repository like **Add from GitHub** when the plugin is not here yet, otherwise
/// only turning it on. The flag says a fresh add is saved switched on.
async fn resolve_official(
    command: PluginCommand,
    store: &Arc<dyn PluginStore>,
) -> Result<(PluginCommand, bool), SkillError> {
    let PluginCommand::InstallOfficial { id } = command else {
        return Ok((command, false));
    };
    let official = require_official(&id)?;
    let installed = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || {
            store
                .load()
                .map(|library| library.installed.iter().any(|plugin| plugin.entry.id == id))
        })
        .await
        .map_err(|_| SkillError("The plugin storage worker stopped. Try again.".into()))??
    };
    let command = if installed {
        PluginCommand::SetEnabled {
            id: official.id(),
            enabled: true,
        }
    } else {
        PluginCommand::AddFromGitHub {
            address: official.repository.to_owned(),
        }
    };
    Ok((command, true))
}

#[expect(
    clippy::too_many_lines,
    reason = "one serialized match over every plugin command keeps download and persistence ordering visible"
)]
pub(crate) async fn handle(
    command: PluginCommand,
    store: Arc<dyn PluginStore>,
    skills: Option<Arc<dyn SkillStore>>,
    session: &mut PluginSession,
) -> Result<(PluginLibrary, Option<PluginEvent>), SkillError> {
    let (command, enable_on_add) = resolve_official(command, &store).await?;
    let mut discovered = None;
    let mut downloaded = None;
    let root = match &command {
        PluginCommand::Browse { url } => url.clone(),
        PluginCommand::Install { .. } | PluginCommand::NextPage | PluginCommand::PreviousPage => {
            session
                .catalog
                .as_ref()
                .ok_or_else(|| SkillError("Reload the plugin catalog first.".into()))?
                .root
                .clone()
        }
        PluginCommand::SetEnabled { .. }
        | PluginCommand::AllowCode { .. }
        | PluginCommand::Inspect { .. }
        | PluginCommand::AddFromGitHub { .. }
        | PluginCommand::CheckForUpdate { .. }
        | PluginCommand::ApplyUpdate { .. }
        | PluginCommand::Remove { .. }
        | PluginCommand::InstallOfficial { .. } => String::new(),
    };
    match &command {
        PluginCommand::Browse { .. } | PluginCommand::NextPage | PluginCommand::PreviousPage => {
            let page = session.catalog.as_ref().map(|page| &page.catalog);
            let url = match &command {
                PluginCommand::Browse { url } => url.clone(),
                PluginCommand::NextPage => page
                    .and_then(|page| page.next.clone())
                    .ok_or_else(|| SkillError("There is no next catalog page.".into()))?,
                PluginCommand::PreviousPage => page
                    .and_then(|page| page.previous.clone())
                    .ok_or_else(|| SkillError("There is no previous catalog page.".into()))?,
                PluginCommand::Install { .. }
                | PluginCommand::SetEnabled { .. }
                | PluginCommand::AllowCode { .. }
                | PluginCommand::Inspect { .. }
                | PluginCommand::AddFromGitHub { .. }
                | PluginCommand::CheckForUpdate { .. }
                | PluginCommand::ApplyUpdate { .. }
                | PluginCommand::Remove { .. }
                | PluginCommand::InstallOfficial { .. } => unreachable!("catalog commands only"),
            };
            if matches!(&command, PluginCommand::Browse { .. }) {
                session.catalog = None;
            }
            let next = PluginCatalog::parse(&download(&url, MAX_CATALOG_BYTES).await?)?;
            discovered = Some(PluginEvent::Catalog { url, catalog: next });
        }
        PluginCommand::Install { id } => {
            let entry = session
                .catalog
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
            downloaded = Some(Download {
                entry,
                bytes,
                catalog_url: root.clone(),
                github: None,
                // Catalog packages cannot carry code.
                requires_approval: false,
            });
        }
        PluginCommand::AddFromGitHub { address } => {
            let repository = GitHubRepository::parse(address)?;
            // Refused before reading GitHub: a repository can be megabytes of files,
            // and its unsigned requests count against GitHub's hourly limit. The save
            // step checks again in case it was added meanwhile.
            let id = repository.plugin_id();
            let library = {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.load())
                    .await
                    .map_err(|_| {
                        SkillError("The plugin storage worker stopped. Try again.".into())
                    })??
            };
            if library.installed.iter().any(|plugin| plugin.entry.id == id) {
                return Err(SkillError(ALREADY_ADDED.into()));
            }
            let reader = github::Reader::new(&session.hosts)?;
            let head = reader.head(&repository).await?;
            let github::Snapshot {
                entry,
                bytes,
                pin,
                left_out,
            } = reader.snapshot(&repository, head, store.as_ref()).await?;
            discovered = Some(PluginEvent::Added {
                id: entry.id.clone(),
                package: entry.verify_package(&bytes)?,
                left_out,
                enabled: enable_on_add && turns_on_at_install(&entry),
            });
            downloaded = Some(Download {
                entry,
                bytes,
                catalog_url: repository.web_url(),
                github: Some(pin),
                // Added switched off anyway.
                requires_approval: false,
            });
        }
        PluginCommand::CheckForUpdate { id } => {
            session.pending = None;
            let (library, installed, current) = {
                let store = store.clone();
                let id = id.clone();
                tokio::task::spawn_blocking(move || {
                    let library = store.load()?;
                    let installed = library
                        .installed
                        .iter()
                        .find(|plugin| plugin.entry.id == id)
                        .cloned()
                        .ok_or_else(|| SkillError("This plugin is no longer installed.".into()))?;
                    let current = store.package(&installed.entry)?;
                    Ok::<_, SkillError>((library, installed, current))
                })
                .await
                .map_err(|_| SkillError("The plugin storage worker stopped. Try again.".into()))??
            };
            let Some(pin) = installed.github else {
                return Err(SkillError(
                    "This plugin came from a catalog. Load its catalog to see newer versions."
                        .into(),
                ));
            };
            let reader = github::Reader::new(&session.hosts)?;
            let head = reader.head(&pin.repository).await?;
            if head.commit == pin.commit {
                return Ok((library, Some(PluginEvent::UpToDate { id: id.clone() })));
            }
            let github::Snapshot {
                entry,
                bytes,
                pin,
                left_out,
            } = reader
                .snapshot(&pin.repository, head, store.as_ref())
                .await?;
            let mut changes = PluginChanges::between(&current, &entry.verify_package(&bytes)?);
            changes.left_out = left_out;
            session.pending = Some((
                id.clone(),
                Download {
                    entry,
                    bytes,
                    catalog_url: installed.catalog_url,
                    github: Some(pin),
                    requires_approval: changes.code.needs_approval(),
                },
            ));
            return Ok((
                library,
                Some(PluginEvent::UpdateReady {
                    id: id.clone(),
                    changes,
                }),
            ));
        }
        PluginCommand::ApplyUpdate { id } => {
            // Only the exact bytes the person reviewed are applied, never a fresh download.
            let (_, pending) = session
                .pending
                .take()
                .filter(|(pending, _)| pending == id)
                .ok_or_else(|| {
                    SkillError("Choose Check for update again before updating.".into())
                })?;
            downloaded = Some(pending);
        }
        PluginCommand::SetEnabled { .. }
        | PluginCommand::AllowCode { .. }
        | PluginCommand::Inspect { .. }
        | PluginCommand::Remove { .. } => {}
        PluginCommand::InstallOfficial { .. } => {
            unreachable!("resolve_official replaced it with an add or a turn-on")
        }
    }
    let adding = matches!(&command, PluginCommand::AddFromGitHub { .. });
    let updating = matches!(&command, PluginCommand::ApplyUpdate { .. });
    let removing = match &command {
        PluginCommand::Remove { id } => Some(id.clone()),
        PluginCommand::Browse { .. }
        | PluginCommand::Install { .. }
        | PluginCommand::NextPage
        | PluginCommand::PreviousPage
        | PluginCommand::Inspect { .. }
        | PluginCommand::SetEnabled { .. }
        | PluginCommand::AllowCode { .. }
        | PluginCommand::AddFromGitHub { .. }
        | PluginCommand::CheckForUpdate { .. }
        | PluginCommand::ApplyUpdate { .. }
        | PluginCommand::InstallOfficial { .. } => None,
    };
    // Installs, updates and removals can leave a package file nothing refers to.
    let packages_change = removing.is_some() || downloaded.is_some();
    let (library, details) = tokio::task::spawn_blocking(move || {
        let mut library = store.load()?;
        let mut details = None;
        let mut required_plugin = None;
        match command {
            PluginCommand::Browse { url } => library.catalog_url = Some(url),
            PluginCommand::Install { .. }
            | PluginCommand::AddFromGitHub { .. }
            | PluginCommand::ApplyUpdate { .. } => {
                let Download {
                    entry,
                    bytes,
                    catalog_url,
                    github,
                    requires_approval,
                } = downloaded.expect("installs download and verify their package first");
                let switched_on = adding && enable_on_add && turns_on_at_install(&entry);
                let mut plugin = InstalledPlugin {
                    entry,
                    catalog_url,
                    enabled: switched_on,
                    github,
                };
                if let Some(existing) = library
                    .installed
                    .iter_mut()
                    .find(|existing| existing.entry.id == plugin.entry.id)
                {
                    if adding {
                        return Err(SkillError(ALREADY_ADDED.into()));
                    }
                    check_replacement(existing, &plugin)?;
                    plugin.enabled = existing.enabled && !requires_approval;
                    *existing = plugin.clone();
                } else if updating {
                    return Err(SkillError("This plugin is no longer installed.".into()));
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
                library.set_enabled(&id, enabled)?;
                if enabled {
                    required_plugin = Some(id);
                }
            }
            PluginCommand::AllowCode { id, package_sha256 } => {
                library.allow_code(&id, &package_sha256)?;
                required_plugin = Some(id);
            }
            PluginCommand::Remove { id } => {
                let index = library
                    .installed
                    .iter()
                    .position(|plugin| plugin.entry.id == id)
                    .ok_or_else(|| SkillError("This plugin is no longer installed.".into()))?;
                let title = library.installed.remove(index).entry.title;
                details = Some(PluginEvent::Removed { id, title });
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
            PluginCommand::NextPage
            | PluginCommand::PreviousPage
            | PluginCommand::CheckForUpdate { .. } => return Ok((library, details)),
            PluginCommand::InstallOfficial { .. } => {
                unreachable!("resolve_official replaced it with an add or a turn-on")
            }
        }
        library.validate()?;
        let admission = required_plugin
            .as_deref()
            .map_or(Admission::Recover, Admission::ValidatePlugin);
        listing(skills.as_deref(), &library, Some(store.as_ref()), admission)?;
        store.save(&library)?;
        if packages_change {
            // The saved index is already correct; a file left behind only costs space
            // and is retried after the next change, so this never fails the command.
            match store.remove_unused(&library) {
                Ok(0) => {}
                Ok(removed) => tracing::debug!(removed, "deleted unused plugin packages"),
                Err(error) => tracing::warn!(%error, "could not delete unused plugin packages"),
            }
        }
        Ok((library, details))
    })
    .await
    .map_err(|_| SkillError("The plugin storage worker stopped. Try again.".into()))??;
    if removing.is_some_and(|id| {
        session
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending == id)
    }) {
        session.pending = None;
    }
    if let Some(PluginEvent::Catalog { catalog: found, .. }) = &discovered {
        session.catalog = Some(RemoteCatalog {
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
