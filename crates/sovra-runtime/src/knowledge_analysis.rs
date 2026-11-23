//! Scoped, concurrent reads of the owner's connected sources.
use crate::{artifacts::ArtifactRuntime, recalling, remembering::MemoryControl};
use sovra_core::{
    Event, MemoryContext, MemorySearch, MemorySourceOrigin, MemoryStore,
    file_index::{FileIndex, IndexError, IndexPassage, IndexQuery},
    files::FileStore,
    knowledge_analysis::{Citation, Excerpt, Exploration, ExploreRequest, Language},
};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Sources {
    pub(crate) media: Option<Arc<dyn FileStore>>,
    pub(crate) files: Option<Arc<dyn FileIndex>>,
    pub(crate) memory: Option<Arc<dyn MemoryStore>>,
    pub(crate) control: MemoryControl,
    pub(crate) artifacts: ArtifactRuntime,
}
pub(crate) struct Explorer {
    sources: Sources,
    events: async_channel::Sender<Event>,
    task: Option<tokio::task::JoinHandle<()>>,
    cpu: Arc<tokio::sync::Semaphore>,
}
impl Explorer {
    pub(crate) fn new(sources: Sources, events: async_channel::Sender<Event>) -> Self {
        Self {
            sources,
            events,
            task: None,
            cpu: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
    pub(crate) fn search(&mut self, handle: &tokio::runtime::Handle, request: ExploreRequest) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let sources = self.sources.clone();
        let events = self.events.clone();
        let cpu = self.cpu.clone();
        self.task = Some(handle.spawn(async move {
            let request_id = request.id;
            let result = sources.read(request, cpu).await;
            let _ = events.send(Event::Explored { request_id, result }).await;
        }));
    }
}
impl Drop for Explorer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Sources {
    async fn read(
        self,
        request: ExploreRequest,
        cpu: Arc<tokio::sync::Semaphore>,
    ) -> Result<Exploration, String> {
        request.options.validate().map_err(str::to_owned)?;
        let query = IndexQuery {
            text: request.query.clone(),
            roots: vec![],
            path_prefix: None,
            limit: 6,
        };
        query.validate().map_err(|e| e.0)?;
        let revision = (self.control.pause_epoch(), self.control.recall_revision());
        let language = request.options.language;
        let (files, personal, artifacts) = tokio::join!(
            self.folders(query, language),
            self.personal(&request.query, language),
            self.artifacts.excerpts(request.query.clone(), language)
        );
        let mut excerpts = Vec::new();
        let mut notices = Vec::new();
        for (name, result) in [
            ("Folders", files),
            ("About you", personal),
            ("Artifacts", artifacts),
        ] {
            match result {
                Ok(hits) => excerpts.extend(hits),
                Err(message) => notices.push(format!("{name} · {message}")),
            }
        }
        let permit = cpu
            .acquire_owned()
            .await
            .map_err(|_| "Analysis stopped.".to_owned())?;
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Some(media) = self.media {
                for excerpt in &mut excerpts {
                    if matches!(excerpt.citation, Citation::Artifact { .. })
                        && excerpt.wording.trim_start().starts_with("<svg")
                    {
                        excerpt.mask =
                            media.binary_mask("image/svg+xml", excerpt.wording.as_bytes());
                    }
                }
            }
            Exploration::from_excerpts(excerpts, &request.options, notices)
        })
        .await
        .map_err(|_| "Analysis was interrupted.".to_owned())?
        .map_err(str::to_owned)?;
        if revision != (self.control.pause_epoch(), self.control.recall_revision()) {
            return Err(
                "Saved details changed during this search. Search again for current wording."
                    .into(),
            );
        }
        Ok(result)
    }
    async fn folders(&self, query: IndexQuery, language: Language) -> Result<Vec<Excerpt>, String> {
        let Some(store) = self.files.clone() else {
            return Ok(vec![]);
        };
        tokio::task::spawn_blocking(move || {
            let mut sources = Vec::new();
            let mut complete_tables = std::collections::BTreeSet::new();
            for hit in store.search_language(&query, language)? {
                let extension = hit.path.rsplit_once('.').map(|(_, ext)| ext);
                let tabular = extension
                    .is_some_and(|ext| ["csv", "tsv"].iter().any(|v| ext.eq_ignore_ascii_case(v)));
                if tabular && complete_tables.contains(&hit.reference.version_id) {
                    continue;
                }
                let mask = if hit.mime_type.starts_with("image/") {
                    store.binary_mask(&hit.reference)?
                } else {
                    None
                };
                let table = if tabular {
                    store.table_wording(&hit.reference)?
                } else {
                    None
                };
                let version = hit.reference.version_id.clone();
                let mut source = file_excerpt(hit);
                if let Some(whole) = table {
                    complete_tables.insert(version);
                    source.first_line = 1;
                    source.provenance = format!(
                        "Indexed table · extracted lines 1–{} · {}",
                        whole.lines().count(),
                        source.provenance
                    );
                    source.wording = whole;
                }
                source.mask = mask;
                sources.push(source);
            }
            Ok::<_, IndexError>(sources)
        })
        .await
        .map_err(|_| "Folder search was interrupted.".to_owned())?
        .map_err(|e| e.0)
    }
    async fn personal(&self, text: &str, language: Language) -> Result<Vec<Excerpt>, String> {
        let Some(reader) = recalling::reader(self.memory.as_ref(), &self.control, None) else {
            return Ok(vec![]);
        };
        let query = MemorySearch {
            text: text.into(),
            ..Default::default()
        };
        let recalled = if (language != Language::English
            || sovra_core::knowledge_analysis::is_identifier(text))
            && let Some(store) = &self.memory
        {
            let sources = store
                .search_source_language(query, crate::now(), language)
                .await
                .map_err(|e| e.to_string())?;
            MemoryContext {
                sources,
                ..Default::default()
            }
        } else {
            reader.search(query).await.map_err(|e| e.to_string())?
        };
        if recalled.memories.is_empty()
            && recalled.sources.is_empty()
            && let Some(notice) = recalled.notice
        {
            return Err(notice);
        }
        let mut excerpts = Vec::new();
        for item in recalled
            .sources
            .into_iter()
            .filter(|s| s.origin == MemorySourceOrigin::UserStatement)
        {
            let standing = if item.support_changed {
                " · supporting details changed"
            } else {
                ""
            };
            let provenance = format!(
                "Original statement · {} · version {} · bytes {}–{}{standing}",
                item.citation, item.version, item.start, item.end
            );
            excerpts.push(Excerpt {
                mask: None,
                citation: Citation::User {
                    source_id: item.source_id,
                    citation: item.citation,
                    recorded: item.source_time,
                },
                title: "Your statement".into(),
                provenance,
                wording: item.quote,
                first_line: 1,
            });
        }
        for item in recalled.memories {
            if excerpts.iter().any(|e| e.wording == item.quote && matches!(&e.citation, Citation::User { source_id, .. } if source_id.0 == item.source_id)) {
                continue;
            }
            let standing = if item.interpretation {
                "model interpretation"
            } else {
                "saved claim"
            };
            let provenance = format!(
                "Quoted source for {} · {standing} · learned {}",
                item.citation,
                iso_time(item.learned_at)
            );
            excerpts.push(Excerpt {
                mask: None,
                citation: Citation::User {
                    source_id: sovra_core::EpisodeId(item.source_id),
                    citation: item.citation,
                    recorded: item.source_time,
                },
                title: "Saved detail".into(),
                provenance,
                wording: item.quote,
                first_line: 1,
            });
        }
        Ok(excerpts)
    }
}
fn iso_time(at: sovra_core::Timestamp) -> String {
    time::OffsetDateTime::from_unix_timestamp(at.0.div_euclid(1000))
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| format!("{} ms since 1970 UTC", at.0))
}
fn file_excerpt(hit: IndexPassage) -> Excerpt {
    let wording = sovra_core::file_index::passage_wording(&hit.text);
    Excerpt {
        mask: None,
        citation: Citation::File {
            reference: hit.reference,
            observed: hit.observed_at,
        },
        title: hit.path,
        provenance: format!(
            "Indexed file · lines {}–{}{}",
            hit.start_line,
            hit.end_line,
            hit.note.map_or_else(String::new, |n| format!(" · {n}"))
        ),
        wording: wording.into(),
        first_line: hit.start_line,
    }
}
