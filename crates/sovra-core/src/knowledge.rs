//! Plain-language picture of how Sovra remembers.
//!
//! The picture is a fixed example of the memory model in [`crate::memory`]:
//! a record is an episode, a belief is a fact, the names are entities, history
//! is supersession (never a delete), and finding a memory uses meaning plus
//! the words themselves. Nothing here reads a clock, a store, or the network.

/// Where a step sits in the life of one memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryBeat {
    /// Someone said or did something.
    Happened,
    /// That event was kept unchanged.
    RecordKept,
    /// A separate statement became what Sovra believes now.
    CurrentBelief,
    /// A later statement took the place of the earlier one.
    Replacement,
    /// The earlier statement remains, marked as history.
    HistoryKept,
}

/// One step of the worked example, in reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoryStep {
    pub beat: StoryBeat,
    pub heading: &'static str,
    pub body: &'static str,
}

/// Whether a belief is what Sovra holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeliefStanding {
    /// Sovra holds this statement now.
    Current,
    /// A newer statement took its place. The old one was kept.
    History,
}

impl BeliefStanding {
    /// Visible name for this standing. Everyday words only.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Current => "Current belief",
            Self::History => "Kept as history",
        }
    }
}

/// One belief in the example, including who or what it is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeliefPicture {
    pub statement: &'static str,
    /// People or things this belief is about.
    pub about: &'static [&'static str],
    pub standing: BeliefStanding,
    /// The earlier statement this one replaced, when it replaced one.
    pub replaces: Option<&'static str>,
}

/// How finding a memory works, in everyday words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchPicture {
    pub heading: &'static str,
    pub summary: &'static str,
}

/// One moment Sovra records for a belief. Shown only after the reader asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeNote {
    pub heading: &'static str,
    pub body: &'static str,
}

/// The whole teaching picture. Built from fixed data; safe to draw with no store open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryExplanation {
    pub title: &'static str,
    pub lead: &'static str,
    pub steps: &'static [StoryStep],
    pub beliefs: &'static [BeliefPicture],
    pub search: SearchPicture,
    pub times: &'static [TimeNote],
}

impl MemoryExplanation {
    /// Beliefs Sovra holds now. History is excluded.
    pub fn current_beliefs(&self) -> impl Iterator<Item = &BeliefPicture> {
        self.beliefs
            .iter()
            .filter(|belief| belief.standing == BeliefStanding::Current)
    }
}

/// The shipped explanation. This is the start state for the Knowledge page.
pub fn memory_explanation() -> MemoryExplanation {
    MemoryExplanation {
        title: "Knowledge",
        lead: "This example is not your chat. Follow one memory from the moment it happens until a newer belief replaces it. The old belief stays as history. Sovra never erases it.",
        steps: &[
            StoryStep {
                beat: StoryBeat::Happened,
                heading: "Something happens",
                body: "Maya tells Sovra, \"I live in Lisbon.\"",
            },
            StoryStep {
                beat: StoryBeat::RecordKept,
                heading: "A record is kept",
                body: "Sovra keeps Maya's words just as she said them. That record does not change.",
            },
            StoryStep {
                beat: StoryBeat::CurrentBelief,
                heading: "A current belief",
                body: "From that record, Sovra writes a separate belief: \"Maya lives in Lisbon.\" This belief is about Maya.",
            },
            StoryStep {
                beat: StoryBeat::Replacement,
                heading: "A replacement",
                body: "Later, Maya says she moved to Porto. This replacement becomes the current belief: \"Maya lives in Porto.\"",
            },
            StoryStep {
                beat: StoryBeat::HistoryKept,
                heading: "History kept",
                body: "\"Maya lives in Lisbon.\" stays visible as history. Sovra does not erase it.",
            },
        ],
        beliefs: &[
            BeliefPicture {
                statement: "Maya lives in Porto.",
                about: &["Maya", "Porto"],
                standing: BeliefStanding::Current,
                replaces: Some("Maya lives in Lisbon."),
            },
            BeliefPicture {
                statement: "Maya lives in Lisbon.",
                about: &["Maya", "Lisbon"],
                standing: BeliefStanding::History,
                replaces: None,
            },
        ],
        search: SearchPicture {
            heading: "How Sovra finds a memory",
            summary: "Sovra uses the meaning and the words themselves. The meaning of \"where does Maya live?\" can find this memory. The word Lisbon can find it too.",
        },
        times: &[
            TimeNote {
                heading: "When Sovra wrote it down",
                body: "The moment this belief was saved.",
            },
            TimeNote {
                heading: "When it became true",
                body: "When this became the case in the world.",
            },
            TimeNote {
                heading: "When it stopped being true",
                body: "When the world changed and this stopped being the case.",
            },
            TimeNote {
                heading: "When a newer belief took its place",
                body: "From this moment the older belief is history, and it is still kept.",
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worked_example_teaches_the_memory_lifecycle() {
        let story = memory_explanation();
        let beats: Vec<StoryBeat> = story.steps.iter().map(|step| step.beat).collect();
        assert_eq!(
            beats,
            vec![
                StoryBeat::Happened,
                StoryBeat::RecordKept,
                StoryBeat::CurrentBelief,
                StoryBeat::Replacement,
                StoryBeat::HistoryKept,
            ]
        );

        let history: Vec<_> = story
            .beliefs
            .iter()
            .filter(|belief| belief.standing == BeliefStanding::History)
            .collect();
        let current: Vec<_> = story.current_beliefs().collect();
        assert_eq!(
            history.len(),
            1,
            "the example must keep the replaced belief as history"
        );
        assert_eq!(current.len(), 1, "the example must have one current belief");
        for old in &history {
            assert!(
                current
                    .iter()
                    .all(|belief| belief.statement != old.statement),
                "replaced belief is still a current belief: {}",
                old.statement
            );
            assert!(
                current
                    .iter()
                    .any(|belief| belief.replaces == Some(old.statement)),
                "no current belief replaces {}",
                old.statement
            );
            assert!(
                story.steps.iter().any(|step| {
                    step.beat == StoryBeat::HistoryKept && step.body.contains(old.statement)
                }),
                "the history step does not show the replaced belief"
            );
        }

        for belief in story.beliefs {
            assert!(
                belief.about.iter().any(|name| !name.is_empty()),
                "{} must name who or what it is about",
                belief.statement
            );
            for name in belief.about {
                assert!(
                    story.steps.iter().any(|step| step.body.contains(name)),
                    "{name} is named on a belief but missing from the story"
                );
            }
        }

        let search = story.search.summary.to_ascii_lowercase();
        assert!(
            search.contains("meaning"),
            "search must describe meaning: {}",
            story.search.summary
        );
        assert!(
            search.contains("word"),
            "search must describe the words themselves: {}",
            story.search.summary
        );

        let first_layer = first_layer_text(&story);
        for jargon in [
            "graph",
            "database",
            "ranking",
            "embedding",
            "vector",
            "bm25",
            "hnsw",
            "episode",
            "entity",
            "schema",
            "bi-temporal",
            "surreal",
        ] {
            assert!(
                !first_layer.to_ascii_lowercase().contains(jargon),
                "first layer uses jargon {jargon}: {first_layer}"
            );
        }
    }

    fn first_layer_text(story: &MemoryExplanation) -> String {
        let mut text = format!("{}\n{}\n", story.title, story.lead);
        for step in story.steps {
            text.push_str(step.heading);
            text.push('\n');
            text.push_str(step.body);
            text.push('\n');
        }
        text.push_str(story.search.heading);
        text.push('\n');
        text.push_str(story.search.summary);
        text.push('\n');
        for belief in story.beliefs {
            text.push_str(belief.standing.label());
            text.push(' ');
            text.push_str(belief.statement);
            for name in belief.about {
                text.push(' ');
                text.push_str(name);
            }
            text.push('\n');
        }
        text
    }
}
