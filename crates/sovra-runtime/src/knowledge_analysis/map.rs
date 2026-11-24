use super::{Explorer, Sources, map_files};
use sovra_core::{
    Event, MemoryCursor, MemoryListQuery, MemorySourceCursor, MemorySourceQuery,
    knowledge_analysis::{Citation, Exploration, ExploreRequest},
    knowledge_map::{Branch, DetailLevel, Layer, MapPage, MapRequest},
};
use std::sync::Arc;

impl Explorer {
    pub(crate) fn map(&mut self, handle: &tokio::runtime::Handle, request: MapRequest) {
        if let Some(task) = self.map_task.take() {
            task.abort();
        }
        let sources = self.sources.clone();
        let events = self.events.clone();
        let cpu = self.cpu.clone();
        self.map_task = Some(handle.spawn(async move {
            let result = sources
                .map_page(request.branch, request.history, cpu)
                .await
                .map(Arc::new);
            let _ = events
                .send(Event::KnowledgeMapped {
                    request_id: request.id,
                    result,
                })
                .await;
        }));
    }
}
impl Sources {
    async fn map_page(
        &self,
        branch: Branch,
        history: bool,
        cpu: Arc<tokio::sync::Semaphore>,
    ) -> Result<MapPage, String> {
        match branch {
            Branch::Overview => {
                let mut page = MapPage::default();
                page.hubs();
                let (files, facts, sources, artifacts, groups) = tokio::join!(
                    self.map_files(Branch::Overview, history, cpu),
                    self.map_facts(None, history),
                    self.map_sources(None, history),
                    self.artifacts.map_page(None, history),
                    self.groups.map_page(history, None)
                );
                for (label, result) in [
                    ("Files", files),
                    ("Personal details", facts),
                    ("Original records", sources),
                ] {
                    match result {
                        Ok(part) => page.merge(part),
                        Err(e) => page.notices.push(format!("{label} · {e}")),
                    }
                }
                page.merge(artifacts);
                page.merge(groups);
                Ok(page)
            }
            Branch::Facts(cursor) => self.map_facts(cursor, history).await,
            Branch::Sources(cursor) => self.map_sources(cursor, history).await,
            Branch::Artifacts(cursor) => Ok(self.artifacts.map_page(cursor, history).await),
            Branch::GroupRecords(id, after) => {
                Ok(self.groups.map_page(history, Some((&id, after))).await)
            }
            Branch::Artifact(id, version) => {
                self.artifacts
                    .map_record(&id, version.filter(|_| history))
                    .await
            }
            Branch::Original(id) => {
                let s = self
                    .memory
                    .as_ref()
                    .ok_or("Personal memory is unavailable.")?
                    .review_memory_source(&id)
                    .await
                    .map_err(|e| e.to_string())?;
                if !history && s.metadata.revoked_at.is_some() {
                    return Err("This original record is stopped.".into());
                }
                let mut page = MapPage::default();
                let key = page.describe(
                    Layer::Personal,
                    format!("source:{}", id.0),
                    s.metadata.author_label(),
                    format!("{}\n{}", s.metadata.status_label(), s.episode.content),
                    DetailLevel::Full,
                );
                page.chat(&key, &s.episode.session_id, "");
                Ok(page)
            }
            Branch::Search(query) => {
                let report = self
                    .clone()
                    .read(
                        ExploreRequest {
                            id: 0,
                            query: query.clone(),
                            options: sovra_core::knowledge_analysis::AnalysisOptions::default(),
                        },
                        cpu,
                    )
                    .await?;
                Ok(map_search(&query, report))
            }
            branch => self.map_files(branch, history, cpu).await,
        }
    }
    async fn map_files(
        &self,
        branch: Branch,
        history: bool,
        cpu: Arc<tokio::sync::Semaphore>,
    ) -> Result<MapPage, String> {
        let store = self.files.clone().ok_or("Folder index is unavailable.")?;
        let permit = cpu.acquire_owned().await.map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if branch == Branch::Overview {
                map_files::overview(store.as_ref(), history)
            } else {
                map_files::expand(store.as_ref(), branch, history)
            }
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.0)
    }
    async fn map_facts(
        &self,
        before: Option<MemoryCursor>,
        history: bool,
    ) -> Result<MapPage, String> {
        let mut page = MapPage::default();
        let list = self
            .memory
            .as_ref()
            .ok_or("Personal memory is unavailable.")?
            .list_memories(MemoryListQuery {
                before,
                include_history: history,
                limit: 50,
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        let count = list.overview.map_or_else(
            || "Saved personal details".into(),
            |o| {
                format!(
                    "{} current · {} historical · {} stopped",
                    o.current, o.history, o.stopped
                )
            },
        );
        let hub = page.node(Layer::Personal, "facts".into(), "Saved details", count);
        page.member(Layer::Personal, &hub);
        for f in list.facts {
            let state = if f.stopped_at.is_some() {
                "Stopped"
            } else if f.is_current() {
                "Current"
            } else {
                "Earlier version"
            };
            let id = page.node(
                Layer::Personal,
                format!("fact:{}", f.id.0),
                &f.content,
                format!("{state} · {}\n{}", super::iso_time(f.created_at), f.content),
            );
            page.link(&hub, &id, "contains", true);
            for label in f.entities {
                page.subject(&id, &label, Layer::Personal);
            }
            if let Some(source) = f.source {
                let key = format!("source:{}", source.0);
                page.describe(
                    Layer::Personal,
                    key.clone(),
                    "Original record",
                    "Read the original statement",
                    DetailLevel::Reference,
                );
                page.link(&id, &key, "supported by", false);
                page.branch(&key, Some(Branch::Original(source)));
            }
            if let Some(old) = f.replaces {
                page.link(&id, &format!("fact:{}", old.0), "replaces", false);
            }
        }
        for chat in list.chats {
            page.chat(&format!("fact:{}", chat.fact.0), &chat.session, &chat.title);
        }
        page.branch(&hub, list.next.map(|c| Branch::Facts(Some(c))));
        Ok(page)
    }
    async fn map_sources(
        &self,
        before: Option<MemorySourceCursor>,
        history: bool,
    ) -> Result<MapPage, String> {
        let mut page = MapPage::default();
        let list = self
            .memory
            .as_ref()
            .ok_or("Personal memory is unavailable.")?
            .list_memory_sources(MemorySourceQuery {
                before,
                include_stopped: history,
                limit: 50,
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        let hub = page.node(
            Layer::Personal,
            "sources".into(),
            "Original records",
            "Saved evidence, including records without an extracted detail.",
        );
        page.member(Layer::Personal, &hub);
        for s in list.sources {
            let key = page.node(
                Layer::Personal,
                format!("source:{}", s.id.0),
                s.excerpt.clone(),
                format!(
                    "{} · {}\n{}",
                    s.metadata.author_label(),
                    s.metadata.status_label(),
                    s.excerpt
                ),
            );
            page.link(&hub, &key, "contains", true);
            page.branch(&key, Some(Branch::Original(s.id)));
        }
        page.branch(&hub, list.next.map(|c| Branch::Sources(Some(c))));
        Ok(page)
    }
}
fn map_search(query: &str, report: Exploration) -> MapPage {
    let mut page = MapPage::default();
    let mut ids = Vec::new();
    for (index, reading) in report.readings.into_iter().enumerate() {
        let e = reading.excerpt;
        let (layer, source) = match e.citation {
            Citation::File { reference, .. } => {
                let file = page.file(&reference.root_id, &reference.file_id, &e.title);
                let part = map_files::passage_key(&reference);
                page.describe(
                    Layer::Files,
                    part.clone(),
                    &e.title,
                    &e.provenance,
                    DetailLevel::Reference,
                );
                page.link(&file, &part, "contains passage", true);
                page.branch(&part, Some(Branch::Passage(reference)));
                (Layer::Files, part)
            }
            Citation::Artifact { id, .. } => (Layer::Artifacts, format!("artifact:{id}")),
            Citation::User { source_id, .. } => {
                (Layer::Personal, format!("source:{}", source_id.0))
            }
        };
        let key = page.node(
            layer,
            format!("search:{}:{query}:{index}", query.len()),
            e.title,
            format!("{}\n{}", e.provenance, e.wording),
        );
        page.member(layer, &key);
        page.link(&key, &source, "excerpt from", false);
        ids.push(key);
    }
    for (i, link) in report.shared.into_iter().enumerate() {
        let key = page.node(
            Layer::Findings,
            format!("agreement:{}:{query}:{i}", query.len()),
            &link.label,
            link.label.clone(),
        );
        for line in link.sources {
            if let Some(id) = ids.get(line.reading) {
                page.link(&key, id, &format!("source line {}", line.line), false);
            }
        }
    }
    page.notices = report.notices;
    page.notices.push(format!(
        "Search “{query}” added {} matching excerpts.",
        ids.len()
    ));
    page
}
