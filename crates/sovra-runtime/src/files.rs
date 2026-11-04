//! Owned attachment jobs. Synchronous codecs/storage never occupy the command loop.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use sovra_core::Event;
use sovra_core::files::{
    Attachment, FileCommand, FileError, FilePrepared, FileStore, MAX_ATTACHMENTS, MAX_FILE_BYTES,
    MediaPart, ModelPurpose, PageViewer,
};
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

use crate::ActiveRun;
use crate::models::{FileModel, ModelExecution};

pub(crate) struct FileRuntime {
    commands: Sender<FileCommand>,
    task: tokio::task::JoinHandle<()>,
}

impl FileRuntime {
    pub(crate) fn start(
        store: Arc<dyn FileStore>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (commands, receiver) = async_channel::unbounded();
        let task = handle.spawn(async move {
            let mut jobs = JoinSet::new();
            let mut active = BTreeSet::new();
            let mut cancelled = BTreeSet::new();
            let slots = Arc::new(Semaphore::new(2));
            loop {
                tokio::select! {
                    command = receiver.recv() => match command {
                        Ok(FileCommand::Prepare { session_id, id, path }) => {
                            if active.contains(&id) { continue; }
                            if active.len() >= 16 {
                                let _ = events.send(Event::FilePrepared(FilePrepared { session_id, id, result: Err(FileError("Please wait for the current files to finish, then add this file again.".into())) })).await;
                                continue;
                            }
                            active.insert(id.clone());
                            let store = store.clone();
                            let slots = slots.clone();
                            jobs.spawn(async move {
                                let permit = slots.acquire_owned().await;
                                let file_id = id.clone();
                                let result = if permit.is_ok() {
                                    tokio::task::spawn_blocking(move || store.prepare(&file_id, &path)).await
                                        .unwrap_or_else(|_| Err(FileError("This file couldn't be prepared. Please try exporting another copy.".into())))
                                } else { Err(FileError("File reading has stopped. Please try again.".into())) };
                                (session_id, id, result)
                            });
                        }
                        Ok(FileCommand::Discard { id }) => {
                            if active.contains(&id) { cancelled.insert(id); }
                            else { discard(&store, id).await; }
                        }
                        Err(_) => break,
                    },
                    result = jobs.join_next(), if !jobs.is_empty() => {
                        if let Some(Ok((session_id, id, result))) = result {
                            active.remove(&id);
                            if cancelled.remove(&id) { discard(&store, id).await; }
                            else { let _ = events.send(Event::FilePrepared(FilePrepared { session_id, id, result })).await; }
                        }
                    }
                }
            }
            jobs.abort_all();
        });
        Self { commands, task }
    }

    pub(crate) fn send(&self, command: FileCommand) {
        let _ = self.commands.try_send(command);
    }
}

impl Drop for FileRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Page pictures for this turn's own attachments, rendered on blocking tasks
/// so PDF rasterising never occupies an async worker.
struct StorePages {
    store: Arc<dyn FileStore>,
    ids: BTreeSet<String>,
}

impl StorePages {
    fn allowed(&self, file_id: &str) -> Result<String, FileError> {
        if self.ids.contains(file_id) {
            Ok(file_id.to_owned())
        } else {
            Err(FileError(
                "Choose a file_id from this conversation's attachments.".into(),
            ))
        }
    }
}

#[async_trait::async_trait]
impl PageViewer for StorePages {
    async fn page_count(&self, file_id: &str) -> Result<usize, FileError> {
        let id = self.allowed(file_id)?;
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.page_count(&id))
            .await
            .unwrap_or_else(|_| Err(render_failed()))
    }

    async fn render_pages(
        &self,
        file_id: &str,
        pages: &[usize],
    ) -> Result<Vec<MediaPart>, FileError> {
        let id = self.allowed(file_id)?;
        let store = Arc::clone(&self.store);
        let pages = pages.to_vec();
        tokio::task::spawn_blocking(move || store.render_pages(&id, &pages))
            .await
            .unwrap_or_else(|_| Err(render_failed()))
    }
}

fn render_failed() -> FileError {
    FileError("These pages could not be shown as pictures.".into())
}

async fn discard(store: &Arc<dyn FileStore>, id: String) {
    let store = store.clone();
    let _ = tokio::task::spawn_blocking(move || store.discard(&id)).await;
}

pub(crate) struct FilePrompt {
    pub(crate) messaging: Option<Arc<dyn sovra_core::communications::MessagingAccess>>,
    pub(crate) google: Option<sovra_adk::GoogleTools>,
    /// Terminal access for private conversations; `None` when off or unavailable.
    pub(crate) terminal: Option<Arc<dyn sovra_core::terminal::TerminalAccess>>,
    pub(crate) context: Option<sovra_core::ConversationContext>,
    pub(crate) memory_reader: Option<Arc<dyn sovra_core::MemoryReader>>,
    pub(crate) skills: Arc<sovra_core::skills::SkillCatalog>,
    pub(crate) skill_resources: Option<Arc<dyn sovra_core::skills::SkillResources>>,
    pub(crate) indexed_files: Option<Arc<dyn sovra_core::file_index::IndexedFiles>>,
    pub(crate) session_id: String,
    pub(crate) prompt: String,
    pub(crate) ids: Vec<String>,
    pub(crate) store: Arc<dyn FileStore>,
    pub(crate) models: Arc<RwLock<BTreeMap<ModelPurpose, FileModel>>>,
    pub(crate) events: Sender<Event>,
    pub(crate) web_search_enabled: bool,
    pub(crate) search: sovra_adk::SearchClient,
}

pub(crate) fn spawn_prompt(handle: &tokio::runtime::Handle, job: FilePrompt) -> ActiveRun {
    let slot = Arc::new(std::sync::RwLock::new(None));
    let active = slot.clone();
    let interactive = crate::model_priority::interactive();
    let task = handle.spawn(async move {
        let _interactive = interactive;
        let session_id = job.session_id.clone();
        let events = job.events.clone();
        let result = run(job, active).await;
        if let Err(error) = result {
            let _ = events
                .send(Event::AgentError {
                    session_id,
                    message: error.0,
                })
                .await;
        }
    });
    ActiveRun {
        harness: slot,
        task,
    }
}

async fn run(
    job: FilePrompt,
    slot: Arc<std::sync::RwLock<Option<sovra_adk::AdkHarness>>>,
) -> Result<(), FileError> {
    let (skill, prompt) = crate::skills::resolve(&job.skills, &job.prompt)
        .map_err(|error| FileError(error.to_string()))?;
    if job.ids.is_empty() || job.ids.len() > MAX_ATTACHMENTS {
        return Err(FileError(
            "Add between one and eight files to this message.".into(),
        ));
    }
    let store = job.store;
    let ids = job.ids;
    let page_viewer = StorePages {
        store: Arc::clone(&store),
        ids: ids.iter().cloned().collect(),
    };
    let loaded = tokio::task::spawn_blocking(move || {
        ids.iter()
            .map(|id| store.load(id))
            .collect::<Result<Vec<_>, _>>()
    });
    let files = tokio::time::timeout(Duration::from_secs(30), loaded)
        .await
        .map_err(|_| {
            FileError(
                "Opening these files took too long. Please try again with a smaller file.".into(),
            )
        })?
        .map_err(|_| {
            FileError("The saved attachments could not be opened. Please add them again.".into())
        })??;
    if files
        .iter()
        .flat_map(|file| &file.media)
        .map(|part| part.data.len() as u64)
        .sum::<u64>()
        > MAX_FILE_BYTES
    {
        return Err(FileError("These pictures or recordings are too large to send together. Please send them in separate messages.".into()));
    }
    let attachments: Vec<Attachment> = files.iter().map(|file| file.attachment.clone()).collect();
    let purpose = attachments
        .iter()
        .map(|file| file.purpose)
        .max()
        .unwrap_or(ModelPurpose::Documents);
    let models = job.models.read().await;
    for required in attachments.iter().map(|file| file.purpose) {
        if required != ModelPurpose::Documents && !models.contains_key(&required) {
            return Err(FileError(format!(
                "Choose a model for '{}' in Models, then send your message again.",
                required.label()
            )));
        }
    }
    let selected = models.get(&purpose).cloned().ok_or_else(|| {
        FileError(format!(
            "Choose a model for '{}' in Models, then send your message again.",
            purpose.label()
        ))
    })?;
    drop(models);
    if !attachments
        .iter()
        .all(|file| file.purpose.accepts(&selected.model, selected.provider))
    {
        return Err(FileError("The chosen model cannot read all these file types together. Send them separately, or choose a model that reads each type.".into()));
    }
    if selected.provider == sovra_core::models::ProviderKind::OpenAiCompatible
        && attachments.iter().any(|file| {
            file.purpose == ModelPurpose::Audio
                && !matches!(
                    file.mime_type.as_str(),
                    "audio/wav" | "audio/x-wav" | "audio/mpeg" | "audio/mp3"
                )
        })
    {
        return Err(FileError("This service reads WAV and MP3 audio. Convert the recording, or choose an audio model through OpenRouter.".into()));
    }
    let ModelExecution::Selected { harness, timeout } = selected.execution else {
        return Err(FileError(
            "The selected model is unavailable. Check its connection in Models.".into(),
        ));
    };
    let tools_enabled = selected.model.supports("tools");
    // Page pictures only help a model that can see them.
    let pages: Option<Arc<dyn PageViewer>> = (tools_enabled
        && ModelPurpose::Images.accepts(&selected.model, selected.provider))
    .then(|| Arc::new(page_viewer) as Arc<dyn PageViewer>);
    let harness = harness
        .with_working_context(job.context.as_ref())
        .map_err(|error| FileError(error.to_string()))?
        .with_conversation_memory(job.memory_reader, &prompt, crate::now())
        .map_err(|error| FileError(error.to_string()))?;
    // Build the shared prompt/tool index off the executor; large files must not
    // stall other chats or the command loop while their text is case-folded.
    let harness = tokio::task::spawn_blocking(move || {
        let harness = harness
            .with_files(Arc::new(files), tools_enabled)?
            .with_page_viewer(pages)?;
        let harness = if job.web_search_enabled {
            harness.with_web_search(job.search)?
        } else {
            harness.without_web_search()?
        };
        let harness = harness
            .with_indexed_files(job.indexed_files)?
            .with_messaging(job.messaging)?
            .with_google(job.google.as_ref())?;
        let harness = match job.terminal {
            Some(access) => harness.with_terminal(access)?,
            None => harness,
        };
        let harness = harness.with_skill_library(job.skills, job.skill_resources)?;
        if let Some(skill) = skill {
            harness.with_skill(&skill)
        } else {
            Ok(harness)
        }
    })
    .await
    .map_err(|_| FileError("The file search index could not be prepared. Try fewer files.".into()))?
    .map_err(|error| FileError(error.to_string()))?;
    *slot
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(harness.clone());
    let char_budget = usize::try_from(selected.model.context_length.unwrap_or(32_768))
        .unwrap_or(24_000)
        .min(24_000)
        / 2;
    tokio::time::timeout(
        timeout,
        harness.run_with_files(&job.session_id, &prompt, char_budget, &job.events),
    )
    .await
    .map_err(|_| {
        FileError(
            "The model took too long to read these files. Try again, or send fewer files at once."
                .into(),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records which ids reach the store; every id has three pages.
    #[derive(Default)]
    struct PagedStore {
        asked: std::sync::Mutex<Vec<String>>,
    }

    impl FileStore for PagedStore {
        fn prepare(&self, _: &str, _: &std::path::Path) -> Result<Attachment, FileError> {
            Err(FileError("unused".into()))
        }
        fn load(&self, _: &str) -> Result<sovra_core::files::FileContent, FileError> {
            Err(FileError("unused".into()))
        }
        fn discard(&self, _: &str) -> Result<(), FileError> {
            Ok(())
        }
        fn page_count(&self, id: &str) -> Result<usize, FileError> {
            self.asked.lock().expect("asked").push(id.to_owned());
            Ok(3)
        }
        fn render_pages(&self, id: &str, pages: &[usize]) -> Result<Vec<MediaPart>, FileError> {
            self.asked.lock().expect("asked").push(id.to_owned());
            Ok(pages
                .iter()
                .map(|_| MediaPart {
                    mime_type: "image/png".into(),
                    data: vec![1],
                    duration_millis: None,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn page_pictures_are_limited_to_the_turns_own_attachments() {
        let store = Arc::new(PagedStore::default());
        let pages = StorePages {
            store: Arc::clone(&store) as Arc<dyn FileStore>,
            ids: BTreeSet::from(["report".to_owned()]),
        };
        assert_eq!(pages.page_count("report").await.expect("count"), 3);
        assert_eq!(
            pages
                .render_pages("report", &[1, 2])
                .await
                .expect("pages")
                .len(),
            2
        );
        assert!(pages.page_count("other-turn-file").await.is_err());
        assert!(pages.render_pages("../escape", &[1]).await.is_err());
        assert_eq!(*store.asked.lock().expect("asked"), ["report", "report"]);
    }
}
