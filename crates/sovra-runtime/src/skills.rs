//! Resolve explicit skills before routing, model calls or attachment extraction.

use async_channel::Sender;
use sovra_core::Event;
use sovra_core::skills::{
    Skill, SkillCatalog, SkillCommand, SkillError, SkillEvent, SkillKind, SkillStore,
};
use std::sync::Arc;
use tokio::sync::RwLock;

enum LibraryCommand {
    Skill(SkillCommand),
    Plugin(sovra_core::plugins::PluginCommand),
}

pub(crate) struct SkillsRuntime {
    pub(crate) resources: Option<Arc<dyn sovra_core::skills::SkillResources>>,
    pub(crate) catalog: Arc<RwLock<Arc<SkillCatalog>>>,
    commands: Sender<LibraryCommand>,
    events: Sender<Event>,
    task: tokio::task::JoinHandle<()>,
}

impl SkillsRuntime {
    pub(crate) fn start(
        store: Option<Arc<dyn SkillStore>>,
        plugins: Option<Arc<dyn sovra_core::plugins::PluginStore>>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let resources = store.as_ref().map(|store| {
            Arc::new(ResourceReader(store.clone())) as Arc<dyn sovra_core::skills::SkillResources>
        });
        let (commands, receiver) = async_channel::bounded(32);
        let catalog = Arc::new(RwLock::new(Arc::new(SkillCatalog::default())));
        let current = catalog.clone();
        let output = events.clone();
        let task = handle.spawn(async move {
            let mut remote_catalog = None;
            while let Ok(command) = receiver.recv().await {
                let plugin_command = matches!(&command, LibraryCommand::Plugin(_));
                let saved = match &command {
                    LibraryCommand::Skill(SkillCommand::Save(skill)) => Some(skill.clone()),
                    LibraryCommand::Skill(_) | LibraryCommand::Plugin(_) => None,
                };
                let mut discovered = None;
                let result = match command {
                    LibraryCommand::Plugin(command) => {
                        let _ = output
                            .send(Event::Plugins(sovra_core::plugins::PluginEvent::Busy))
                            .await;
                        if let Some(plugins) = plugins.clone() {
                            crate::plugins::handle(
                                command,
                                plugins,
                                store.clone(),
                                &mut remote_catalog,
                            )
                            .await
                            .map(|(_, found)| {
                                discovered = found;
                            })
                        } else {
                            Err(SkillError(
                                "Plugin storage is unavailable in this session.".into(),
                            ))
                        }
                    }
                    LibraryCommand::Skill(command) => {
                        let store = store.clone();
                        let plugins = plugins.clone();
                        tokio::task::spawn_blocking(move || {
                            update_skill(command, store.as_deref(), plugins.as_deref())
                        })
                        .await
                        .map_err(|_| {
                            SkillError("The skill storage worker stopped unexpectedly.".into())
                        })
                        .and_then(|result| result)
                    }
                };
                let result = match result {
                    Ok(()) => {
                        let store = store.clone();
                        let plugins = plugins.clone();
                        tokio::task::spawn_blocking(move || {
                            let library = plugins
                                .as_ref()
                                .map(|store| store.load())
                                .transpose()?
                                .unwrap_or_default();
                            let listing = crate::plugins::listing(
                                store.as_deref(),
                                &library,
                                plugins.as_deref(),
                                crate::plugins::Admission::Recover,
                            )?;
                            Ok::<_, SkillError>((listing, library))
                        })
                        .await
                        .map_err(|_| {
                            SkillError("The skill library worker stopped unexpectedly.".into())
                        })
                        .and_then(|result| result)
                    }
                    Err(error) => Err(error),
                };
                match result {
                    Ok((listing, library)) => {
                        let mut all = sovra_core::skills::builtin_skills();
                        all.extend(listing.skills);
                        match SkillCatalog::new(all.clone()) {
                            Ok(catalog) => {
                                *current.write().await = Arc::new(catalog);
                                let _ = output
                                    .send(Event::Skills(SkillEvent::Loaded {
                                        skills: all,
                                        issues: listing.issues,
                                    }))
                                    .await;
                                if plugins.is_some() {
                                    let _ = output
                                        .send(Event::Plugins(
                                            sovra_core::plugins::PluginEvent::Loaded(library),
                                        ))
                                        .await;
                                }
                                if let Some(event) = discovered {
                                    let _ = output.send(Event::Plugins(event)).await;
                                }
                                if let Some(skill) = saved {
                                    let _ =
                                        output.send(Event::Skills(SkillEvent::Saved(skill))).await;
                                }
                            }
                            Err(error) => {
                                let _ = output
                                    .send(Event::Skills(SkillEvent::Error(error.to_string())))
                                    .await;
                            }
                        }
                    }
                    Err(error) => {
                        let event = if plugin_command {
                            Event::Plugins(sovra_core::plugins::PluginEvent::Error(
                                error.to_string(),
                            ))
                        } else {
                            Event::Skills(SkillEvent::Error(error.to_string()))
                        };
                        let _ = output.send(event).await;
                    }
                }
            }
        });
        Self {
            resources,
            catalog,
            commands,
            events,
            task,
        }
    }

    pub(crate) fn send(&self, command: SkillCommand) {
        if self
            .commands
            .try_send(LibraryCommand::Skill(command))
            .is_err()
        {
            let _ = self.events.try_send(Event::Skills(SkillEvent::Error(
                "Too many skill updates are pending. Wait briefly and try again.".into(),
            )));
        }
    }

    pub(crate) fn send_plugin(&self, command: sovra_core::plugins::PluginCommand) {
        if self
            .commands
            .try_send(LibraryCommand::Plugin(command))
            .is_err()
        {
            let _ = self
                .events
                .try_send(Event::Plugins(sovra_core::plugins::PluginEvent::Error(
                    "Too many plugin updates are pending. Wait briefly and try again.".into(),
                )));
        }
    }
}

fn update_skill(
    command: SkillCommand,
    store: Option<&dyn SkillStore>,
    plugins: Option<&dyn sovra_core::plugins::PluginStore>,
) -> Result<(), SkillError> {
    if matches!(command, SkillCommand::Refresh) {
        return Ok(());
    }
    let store = store
        .ok_or_else(|| SkillError("User skill storage is unavailable in this session.".into()))?;
    let library = plugins
        .map(sovra_core::plugins::PluginStore::load)
        .transpose()?
        .unwrap_or_default();
    let owned: Vec<_> =
        crate::plugins::listing(None, &library, plugins, crate::plugins::Admission::Recover)?
            .skills;
    let name = match &command {
        SkillCommand::Save(skill) => Some(skill.name.as_str()),
        SkillCommand::Remove { name } | SkillCommand::SetEnabled { name, .. } => {
            Some(name.as_str())
        }
        SkillCommand::Refresh | SkillCommand::Import { .. } => None,
    };
    if name.is_some_and(|name| owned.iter().any(|skill| skill.name == name)) {
        return Err(SkillError(
            "Manage this skill through its plugin on the Skills page.".into(),
        ));
    }
    match command {
        SkillCommand::Refresh => {}
        SkillCommand::Save(skill) => {
            let mut candidates = sovra_core::skills::builtin_skills();
            candidates.extend(
                store
                    .list()?
                    .skills
                    .into_iter()
                    .filter(|old| old.name != skill.name),
            );
            candidates.extend(owned);
            candidates.push(*skill.clone());
            SkillCatalog::new(candidates)?;
            store.save(&skill)?;
        }
        SkillCommand::Import { directory } => {
            let skill = store.import(&directory)?;
            if let Err(error) = crate::plugins::listing(
                Some(store),
                &library,
                plugins,
                crate::plugins::Admission::ValidateAll,
            ) {
                store.remove(&skill.name)?;
                return Err(error);
            }
        }
        SkillCommand::Remove { name } => store.remove(&name)?,
        SkillCommand::SetEnabled { name, enabled } => {
            let mut skill = store
                .list()?
                .skills
                .into_iter()
                .find(|skill| skill.name == name)
                .ok_or_else(|| SkillError("This user skill is no longer installed.".into()))?;
            skill.enabled = enabled;
            store.save(&skill)?;
        }
    }
    Ok(())
}

impl Drop for SkillsRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
struct ResourceReader(Arc<dyn SkillStore>);

impl sovra_core::skills::SkillResources for ResourceReader {
    fn read(&self, name: String, path: String) -> sovra_core::skills::SkillResourceFuture {
        let store = self.0.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || store.resource(&name, &path))
                .await
                .map_err(|_| SkillError("The skill resource reader stopped unexpectedly.".into()))?
        })
    }
}

pub(crate) fn resolve(
    catalog: &SkillCatalog,
    prompt: &str,
) -> Result<(Option<Skill>, String), SkillError> {
    let Some(invocation) = catalog.resolve(prompt)? else {
        return Ok((None, prompt.into()));
    };
    match invocation.skill.kind {
        SkillKind::Action(_) => Err(SkillError(format!(
            "/{} is a workspace action. Choose it from the composer / menu.",
            invocation.skill.name
        ))),
        SkillKind::Procedure => Ok((
            Some(invocation.skill),
            if invocation.arguments.is_empty() {
                "Run the selected skill.".into()
            } else {
                invocation.arguments.into()
            },
        )),
    }
}

pub(crate) fn web_requested(catalog: &SkillCatalog, prompt: &str) -> bool {
    match catalog.resolve(prompt) {
        Ok(Some(invocation)) => invocation
            .skill
            .tools
            .iter()
            .any(|tool| tool == "web_search" || tool == sovra_core::WEB_SEARCH_AGENT_ID),
        Ok(None) => sovra_core::is_explicit_web_search(prompt),
        Err(_) => false,
    }
}

/// Keep attachments when an explicit procedure needs both sources. Ordinary web
/// turns retain their existing route without loading unrelated session files.
pub(crate) fn web_without_files(catalog: &SkillCatalog, prompt: &str) -> bool {
    web_requested(catalog, prompt)
        && !catalog
            .resolve(prompt)
            .ok()
            .flatten()
            .is_some_and(|invocation| {
                invocation
                    .skill
                    .tools
                    .iter()
                    .any(|tool| tool == "read_attachment")
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_skills_override_keyword_routing_and_keep_arguments_separate() {
        assert!(web_requested(
            &SkillCatalog::default(),
            "/search Rust release"
        ));
        assert!(!web_requested(
            &SkillCatalog::default(),
            "/read-files search the web for this phrase"
        ));
        assert!(!web_requested(
            &SkillCatalog::default(),
            "/bogus search the web"
        ));
        let (skill, prompt) =
            resolve(&SkillCatalog::default(), "/table a comparison of A and B").unwrap();
        assert_eq!(skill.unwrap().tools, ["render_table"]);
        assert_eq!(prompt, "a comparison of A and B");
        assert!(resolve(&SkillCatalog::default(), "/models").is_err());
        assert!(resolve(&SkillCatalog::default(), "/web").is_err());
        assert_eq!(
            resolve(&SkillCatalog::default(), "/Users/me/document.txt").unwrap(),
            (None, "/Users/me/document.txt".into())
        );
    }

    #[test]
    fn mixed_file_and_web_skills_keep_attachments_on_the_turn() {
        let mut skill = SkillCatalog::default().find("read-files").unwrap().clone();
        skill.name = "compare-sources".into();
        skill.aliases.clear();
        skill.builtin = false;
        skill.tools.push("WebSearchAgent".into());
        let catalog = SkillCatalog::new(vec![skill]).unwrap();
        assert!(web_requested(&catalog, "/compare-sources findings"));
        assert!(!web_without_files(&catalog, "/compare-sources findings"));
        assert!(web_without_files(&SkillCatalog::default(), "/web news"));
        assert!(web_without_files(
            &SkillCatalog::default(),
            "search the web for news"
        ));
    }
}
