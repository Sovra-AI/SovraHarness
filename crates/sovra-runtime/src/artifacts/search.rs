//! Index current artifact contents on first search. Updates replace one document's
//! postings under the same library lock as the saved artifact.
use sovra_core::{
    artifacts::Artifact,
    knowledge_analysis::{Excerpt, Language},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Library {
    items: BTreeMap<String, Artifact>,
    search: Option<Postings>,
}
impl std::ops::Deref for Library {
    type Target = BTreeMap<String, Artifact>;
    fn deref(&self) -> &Self::Target {
        &self.items
    }
}
struct Postings {
    language: Language,
    by_word: BTreeMap<String, BTreeMap<String, usize>>,
    by_id: BTreeMap<String, Vec<String>>,
}
impl Postings {
    fn remove(&mut self, id: &str) {
        for word in self.by_id.remove(id).unwrap_or_default() {
            if let Some(hits) = self.by_word.get_mut(&word) {
                hits.remove(id);
                if hits.is_empty() {
                    self.by_word.remove(&word);
                }
            }
        }
    }
    fn index(&mut self, artifact: &Artifact) {
        self.remove(&artifact.id);
        let stemmer = self.language.stemmer();
        let mut terms = BTreeMap::new();
        for (text, title) in [(&artifact.title, true), (&artifact.content, false)] {
            for word in text
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
            {
                let offset = word.as_ptr().addr() - text.as_ptr().addr();
                if word.len() > 128 {
                    continue;
                }
                let word = word.to_lowercase();
                let position = if title { 0 } else { offset };
                terms.entry(format!("={word}")).or_insert(position);
                terms
                    .entry(stemmer.stem(&word).into_owned())
                    .or_insert(position);
            }
        }
        for (word, at) in &terms {
            self.by_word
                .entry(word.clone())
                .or_default()
                .insert(artifact.id.clone(), *at);
        }
        self.by_id
            .insert(artifact.id.clone(), terms.into_keys().collect());
    }
}
impl Library {
    pub(super) fn loaded(items: Vec<Artifact>) -> Self {
        Self {
            items: items.into_iter().map(|a| (a.id.clone(), a)).collect(),
            search: None,
        }
    }
    pub(super) fn put(&mut self, item: Artifact) {
        if let Some(index) = &mut self.search {
            index.index(&item);
        }
        self.items.insert(item.id.clone(), item);
    }
    pub(super) fn remove(&mut self, id: &str) {
        self.items.remove(id);
        if let Some(index) = &mut self.search {
            index.remove(id);
        }
    }
    pub(super) fn excerpts(&mut self, query: &str, language: Language) -> Vec<Excerpt> {
        if self
            .search
            .as_ref()
            .is_none_or(|index| index.language != language)
        {
            let mut index = Postings {
                language,
                by_word: BTreeMap::new(),
                by_id: BTreeMap::new(),
            };
            for item in self.items.values() {
                index.index(item);
            }
            self.search = Some(index);
        }
        let index = self.search.as_ref().expect("postings initialized");
        let stemmer = language.stemmer();
        let literal = query
            .chars()
            .any(|c| c.is_ascii_digit() || (!c.is_alphanumeric() && !c.is_whitespace()));
        let terms: BTreeSet<_> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|w| {
                let word = w.to_lowercase();
                if literal {
                    format!("={word}")
                } else {
                    stemmer.stem(&word).into_owned()
                }
            })
            .collect();
        let Some(mut lists) = terms
            .iter()
            .map(|term| index.by_word.get(term))
            .collect::<Option<Vec<_>>>()
        else {
            return vec![];
        };
        lists.sort_by_key(|list| list.len());
        let Some((first, rest)) = lists.split_first() else {
            return vec![];
        };
        first
            .iter()
            .filter(|(id, _)| rest.iter().all(|list| list.contains_key(*id)))
            .filter_map(|(id, &offset)| {
                let item = self.items.get(id)?;
                if sovra_core::knowledge_analysis::is_identifier(query)
                    && ![item.title.as_str(), item.content.as_str()]
                        .iter()
                        .any(|body| {
                            sovra_core::knowledge_analysis::contains_identifier(body, query)
                        })
                {
                    return None;
                }
                Some(Excerpt::artifact(
                    id.clone(),
                    item.updated_at,
                    item.title.clone(),
                    &item.content,
                    offset,
                ))
            })
            .take(6)
            .collect()
    }
}
