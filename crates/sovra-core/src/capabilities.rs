//! What each specialist needs to do its job, and which specialists the
//! orchestrator may offer this turn.
//!
//! The orchestrator routes by reading specialist descriptions, so offering one
//! whose integration is missing invites a confident guess ("checked your
//! calendar…"). A specialist is offered only when every capability it requires
//! is actually present. Adapters provide capabilities; adding one (say Gmail)
//! makes the specialists that need it appear without touching the orchestrator.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::specialists::SpecialistAgentSpec;

/// Specialist id of the parent itself; never offered as its own delegate.
pub const ORCHESTRATOR_AGENT_ID: &str = "OrchestratorAgent";

/// A real tool set an adapter can attach to specialists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Searching the public web and reading the pages it finds.
    WebSearch,
    /// Finding existing images on the web.
    ImageSearch,
    /// Creating or editing images.
    ImageGeneration,
    /// Running scripts and calculations in a sandbox.
    CodeExecution,
    /// Reading files the user attached, inside the specialist's own call.
    AttachedFiles,
    Projects,
    Schedules,
    Todos,
    GoogleWorkspace,
    MicrosoftWorkspace,
    GitHub,
}

impl Capability {
    /// Plain words for people, e.g. in "needs web search".
    pub fn label(self) -> &'static str {
        match self {
            Self::WebSearch => "web search",
            Self::ImageSearch => "image search",
            Self::ImageGeneration => "image making",
            Self::CodeExecution => "running code",
            Self::AttachedFiles => "reading attached files",
            Self::Projects => "projects",
            Self::Schedules => "schedules",
            Self::Todos => "to-do lists",
            Self::GoogleWorkspace => "Google account access",
            Self::MicrosoftWorkspace => "Microsoft account access",
            Self::GitHub => "GitHub access",
        }
    }
}

/// Capabilities a roster specialist needs. Model-only specialists (writing,
/// coding advice, maths) need none; unknown ids need none.
pub fn required_capabilities(specialist_id: &str) -> &'static [Capability] {
    match specialist_id {
        "WebSearchAgent" | "DeepResearchAgent" | "StockMarketAgent" | "HousingMarketAgent"
        | "ScholarAgent" => &[Capability::WebSearch],
        "ImageSearchAgent" => &[Capability::ImageSearch],
        "ImageAgent" => &[Capability::ImageGeneration],
        "ExecutionAgent" => &[Capability::CodeExecution],
        // Delegates run on isolated sessions; attachments stay with the parent.
        "DocumentReaderAgent" => &[Capability::AttachedFiles],
        "ProjectAgent" => &[Capability::Projects],
        "SchedulerAgent" => &[Capability::Schedules],
        "TodosAgent" => &[Capability::Todos],
        "GoogleWorkspace" => &[Capability::GoogleWorkspace],
        "MicrosoftWorkspace" => &[Capability::MicrosoftWorkspace],
        "GitHubAgent" => &[Capability::GitHub],
        _ => &[],
    }
}

/// Specialists the orchestrator may call: every requirement met, never itself.
/// Keeps roster order so the catalogue the model reads is stable.
pub fn available_specialists<'a>(
    roster: &'a [SpecialistAgentSpec],
    available: &BTreeSet<Capability>,
) -> Vec<&'a SpecialistAgentSpec> {
    roster
        .iter()
        .filter(|spec| spec.id != ORCHESTRATOR_AGENT_ID)
        .filter(|spec| {
            required_capabilities(&spec.id)
                .iter()
                .all(|needed| available.contains(needed))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_sovra_srx_agents;

    fn ids<'a>(specs: &[&'a SpecialistAgentSpec]) -> Vec<&'a str> {
        specs.iter().map(|spec| spec.id.as_str()).collect()
    }

    #[test]
    fn without_capabilities_only_model_only_specialists_are_offered() {
        let roster = default_sovra_srx_agents();
        let offered = available_specialists(&roster, &BTreeSet::new());
        let offered = ids(&offered);
        assert!(offered.contains(&"CodingAgent"));
        assert!(offered.contains(&"DocumentAuthorAgent"));
        for missing in [
            ORCHESTRATOR_AGENT_ID,
            "WebSearchAgent",
            "StockMarketAgent",
            "GoogleWorkspace",
            "TodosAgent",
            "ExecutionAgent",
            "DocumentReaderAgent",
        ] {
            assert!(
                !offered.contains(&missing),
                "{missing} offered without its tools"
            );
        }
    }

    #[test]
    fn web_search_lights_up_every_web_backed_specialist_and_nothing_else() {
        let roster = default_sovra_srx_agents();
        let none = available_specialists(&roster, &BTreeSet::new()).len();
        let web = available_specialists(&roster, &BTreeSet::from([Capability::WebSearch]));
        let web_ids = ids(&web);
        for specialist in [
            "WebSearchAgent",
            "DeepResearchAgent",
            "StockMarketAgent",
            "HousingMarketAgent",
            "ScholarAgent",
        ] {
            assert!(web_ids.contains(&specialist), "{specialist} missing");
        }
        assert_eq!(web.len(), none + 5);
        assert!(!web_ids.contains(&"GitHubAgent"));
    }

    #[test]
    fn the_orchestrator_is_never_its_own_delegate() {
        let roster = default_sovra_srx_agents();
        let every: BTreeSet<Capability> = [
            Capability::WebSearch,
            Capability::ImageSearch,
            Capability::ImageGeneration,
            Capability::CodeExecution,
            Capability::AttachedFiles,
            Capability::Projects,
            Capability::Schedules,
            Capability::Todos,
            Capability::GoogleWorkspace,
            Capability::MicrosoftWorkspace,
            Capability::GitHub,
        ]
        .into();
        let offered = available_specialists(&roster, &every);
        assert_eq!(offered.len(), roster.len() - 1);
        assert!(offered.iter().all(|spec| spec.id != ORCHESTRATOR_AGENT_ID));
    }

    #[test]
    fn every_roster_id_is_known_to_the_requirements_table() {
        // A renamed specialist would silently fall back to "needs nothing".
        let model_only = [
            "OrchestratorAgent",
            "CodingAgent",
            "HousingAdvisorAgent",
            "DocumentationAgent",
            "PresentationAgent",
            "DocumentAuthorAgent",
            "SpreadsheetAgent",
        ];
        for spec in default_sovra_srx_agents() {
            assert!(
                !required_capabilities(&spec.id).is_empty()
                    || model_only.contains(&spec.id.as_str()),
                "{} has no requirement entry",
                spec.id
            );
        }
    }
}
