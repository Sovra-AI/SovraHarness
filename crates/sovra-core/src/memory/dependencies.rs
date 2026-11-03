//! Pure per-turn accounting of inputs exposed to helpers. Unknown stays unknown.

use super::{EpisodeId, FactId, MemoryContext, MemorySourceDependencies};

/// The adapter reports whether a provider request contained inputs whose grants
/// cannot yet be represented by conversation source/fact references.
pub trait ReplyInputObserver: Send + Sync {
    fn model_input(&self, untracked: bool);
}

#[derive(Debug, Clone)]
pub struct MemoryDependencyTracker {
    dependencies: Option<MemorySourceDependencies>,
    observed: bool,
}

impl MemoryDependencyTracker {
    pub fn new(context: Option<&crate::ConversationContext>) -> Self {
        let mut tracker = Self {
            dependencies: context.map(|_| MemorySourceDependencies::default()),
            observed: false,
        };
        if let Some(context) = context {
            for message in context.messages() {
                match &message.source {
                    Some(source) => tracker.source(source.clone()),
                    None => tracker.untracked(),
                }
            }
        }
        tracker
    }

    pub fn untracked(&mut self) {
        self.dependencies = None;
    }

    pub fn model_input(&mut self, untracked: bool) {
        self.observed = true;
        if untracked {
            self.untracked();
        }
    }

    pub fn source(&mut self, source: EpisodeId) {
        if let Some(dependencies) = &mut self.dependencies {
            if !dependencies.sources.contains(&source) {
                dependencies.sources.push(source);
            }
            if dependencies.validate().is_err() {
                self.untracked();
            }
        }
    }

    pub fn memory(&mut self, context: &MemoryContext) {
        for source in &context.sources {
            self.source(source.source_id.clone());
        }
        for evidence in context.memories.iter().chain(&context.background) {
            self.source(EpisodeId(evidence.source_id.clone()));
            if let Some(dependencies) = &mut self.dependencies {
                let fact = FactId(evidence.fact_id.clone());
                if !dependencies.facts.contains(&fact) {
                    dependencies.facts.push(fact);
                }
                if dependencies.validate().is_err() {
                    self.untracked();
                }
            }
        }
    }

    pub fn snapshot(&self) -> Option<MemorySourceDependencies> {
        self.observed.then(|| self.dependencies.clone()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_visible_sources_are_deduplicated_and_unknown_history_is_irreversible() {
        let mut message = crate::ChatMessage::user("known", "Visible wording", "");
        message.source = Some(EpisodeId("episode:known".into()));
        let context = crate::ConversationContext::from_messages(&[message.clone(), message]);
        let mut tracker = MemoryDependencyTracker::new(Some(&context));
        tracker.model_input(false);
        tracker.source(EpisodeId("episode:known".into()));
        assert_eq!(
            tracker.snapshot().unwrap().sources,
            vec![EpisodeId("episode:known".into())]
        );
        tracker.model_input(true);
        tracker.model_input(false);
        assert_eq!(tracker.snapshot(), None);
        let context = crate::ConversationContext::from_messages(&[crate::ChatMessage::user(
            "legacy",
            "No source link",
            "",
        )]);
        let mut tracker = MemoryDependencyTracker::new(Some(&context));
        tracker.model_input(false);
        tracker.source(EpisodeId("episode:later".into()));
        assert_eq!(tracker.snapshot(), None);
    }

    #[test]
    fn unknown_history_and_overflow_cannot_be_recertified() {
        let mut tracker = MemoryDependencyTracker::new(None);
        tracker.model_input(false);
        tracker.source(EpisodeId("known".into()));
        assert_eq!(tracker.snapshot(), None);
        let context = crate::ConversationContext::default();
        let mut tracker = MemoryDependencyTracker::new(Some(&context));
        assert_eq!(tracker.snapshot(), None);
        tracker.model_input(false);
        assert_eq!(
            tracker.snapshot(),
            Some(MemorySourceDependencies::default())
        );
        for index in 0..=super::super::MAX_MEMORY_SOURCE_DEPENDENCIES {
            tracker.source(EpisodeId(format!("source-{index}")));
        }
        tracker.model_input(false);
        assert_eq!(tracker.snapshot(), None);
    }
}
