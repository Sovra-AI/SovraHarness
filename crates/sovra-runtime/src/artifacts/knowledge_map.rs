use super::ArtifactRuntime;
use sovra_core::knowledge_map::{Branch, DetailLevel, Layer, MapPage};
use std::ops::Bound::{Excluded, Unbounded};

impl ArtifactRuntime {
    pub(crate) async fn map_record(
        &self,
        id: &str,
        version: Option<usize>,
    ) -> Result<MapPage, String> {
        let guard = self.library().await;
        let item = guard
            .as_ref()
            .and_then(|l| l.get(id))
            .ok_or("That saved artifact is unavailable.")?;
        let (key, content) = match version {
            Some(n) => (
                format!("artifact:{id}:version:{n}"),
                &item
                    .versions
                    .get(n)
                    .ok_or("That artifact version is unavailable.")?
                    .content,
            ),
            None => (format!("artifact:{id}"), &item.content),
        };
        let mut page = MapPage::default();
        page.describe(
            Layer::Artifacts,
            key,
            &item.title,
            content,
            DetailLevel::Full,
        );
        Ok(page)
    }
    pub(crate) async fn map_page(&self, after: Option<String>, history: bool) -> MapPage {
        let mut page = MapPage::default();
        let guard = self.library().await;
        let Some(library) = guard.as_ref() else {
            return page;
        };
        let hub = Layer::Artifacts.hub();
        page.node(
            Layer::Artifacts,
            hub.clone(),
            Layer::Artifacts.label(),
            format!("{} saved artifacts", library.len()),
        );
        let mut items = library
            .range((after.map_or(Unbounded, Excluded), Unbounded))
            .peekable();
        let mut last = None;
        for (_, a) in items.by_ref().take(64) {
            let key = page.node(
                Layer::Artifacts,
                format!("artifact:{}", a.id),
                &a.title,
                format!(
                    "{} · {}\n{}",
                    a.kind.label(),
                    a.source.author,
                    a.content.chars().take(1200).collect::<String>()
                ),
            );
            page.member(Layer::Artifacts, &key);
            if a.content.chars().nth(1200).is_some() {
                page.branch(&key, Some(Branch::Artifact(a.id.clone(), None)));
            }
            page.chat(&key, &a.source.session_id, "");
            if !a.source.shared_from.is_empty() {
                page.link(
                    &key,
                    &format!("shared:{}", a.source.shared_from),
                    "kept from group",
                    false,
                );
            }
            if !a.collection.is_empty() {
                let collection = page.node(
                    Layer::Artifacts,
                    format!("collection:{}", a.collection),
                    &a.collection,
                    "Saved-work collection",
                );
                page.link(&collection, &key, "contains", true);
            }
            if history {
                for (n, v) in a.versions.iter().enumerate() {
                    let version = page.node(
                        Layer::Artifacts,
                        format!("{key}:version:{n}"),
                        format!("{} · version {}", a.title, n + 1),
                        v.content.chars().take(1200).collect::<String>(),
                    );
                    page.link(&key, &version, "earlier version", false);
                    if v.content.chars().nth(1200).is_some() {
                        page.branch(&version, Some(Branch::Artifact(a.id.clone(), Some(n))));
                    }
                }
            }
            last = Some(a.id.clone());
        }
        page.branch(
            &hub,
            items.peek().and(last).map(|id| Branch::Artifacts(Some(id))),
        );
        page
    }
}
