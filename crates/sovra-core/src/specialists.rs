//! Canonical `SovraSRX` 22-specialist agent roster and per-agent model overrides.
//!
//! Mirrors `SovraSRXProduction/backend/src/app/agents/orchestrator_react/agents.py` and
//! `questions.py::DISPATCHABLE_SPECIALISTS` so the dynamic router and per-agent model
//! selector operate over the exact `SovraSRX` specialist topology.

use serde::{Deserialize, Serialize};

use crate::models::ModelKey;
use crate::providers::ModelTier;

/// Specification for one specialist agent in the `SovraSRX` multi-agent roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecialistAgentSpec {
    /// Canonical `agent_map` identifier (e.g. `"WebSearchAgent"`, `"CodingAgent"`).
    pub id: String,
    /// Short routing choice key used by Jev / Laya (`"web_search"`, `"coding"`, etc.).
    pub route_key: String,
    /// Human-readable display name.
    pub name: String,
    /// Functional domain and responsibility description.
    pub description: String,
    /// Whether the fast complexity router may direct-dispatch to this agent without the full planner.
    pub direct_dispatchable: bool,
    /// Default complexity tier when no explicit model override is set.
    pub preferred_tier: ModelTier,
    /// Optional per-agent model override (`connection_id` + `model_id`).
    pub model_override: Option<ModelKey>,
}

impl SpecialistAgentSpec {
    /// Human-readable label of the model or tier currently assigned to this agent.
    pub fn assigned_model_label(&self, global_override: Option<&ModelKey>) -> String {
        if let Some(model) = &self.model_override {
            return format!("{} ({})", model.model_id, model.connection_id);
        }
        if let Some(global) = global_override {
            return format!("Global · {}", global.model_id);
        }
        format!("Auto ({})", self.preferred_tier.label())
    }
}

/// Returns the full 22-agent `SovraSRX` specialist roster (`create_specialist_agents`).
pub fn default_sovra_srx_agents() -> Vec<SpecialistAgentSpec> {
    let specs: [(&str, &str, &str, &str, bool, ModelTier); 22] = [
        (
            "OrchestratorAgent",
            "general",
            "ReAct Orchestrator & Planner",
            "Top-level multi-step planner and response synthesizer across all specialists.",
            false,
            ModelTier::Deep,
        ),
        (
            "WebSearchAgent",
            "web_search",
            "Web Search Specialist",
            "Looks things up on the internet: news, prices, opening hours, recent events, or any fact worth checking. Answers with source links.",
            true,
            ModelTier::Fast,
        ),
        (
            "CodingAgent",
            "coding",
            "Coding & Architecture Specialist",
            "Writing, debugging, explaining, refactoring, or reviewing code and software.",
            true,
            ModelTier::Deep,
        ),
        (
            "DeepResearchAgent",
            "deep_research",
            "Deep Research Pipeline",
            "Multi-source investigation, verification, and long-form research synthesis.",
            false,
            ModelTier::Deep,
        ),
        (
            "StockMarketAgent",
            "stock_market",
            "Stock & Markets Analyst",
            "Share prices, tickers, market data, and company financials.",
            true,
            ModelTier::Standard,
        ),
        (
            "HousingMarketAgent",
            "housing_market",
            "Housing Market Analyst",
            "Renting, buying, property listings, neighbourhoods, and housing prices.",
            true,
            ModelTier::Standard,
        ),
        (
            "HousingAdvisorAgent",
            "housing_advisor",
            "Housing & Mortgage Advisor",
            "Affordability scenarios, mortgage calculations, and relocation guidance.",
            false,
            ModelTier::Standard,
        ),
        (
            "ScholarAgent",
            "scholar",
            "Academic & Scholar Specialist",
            "Scientific literature, papers, citations, and methodology review.",
            false,
            ModelTier::Deep,
        ),
        (
            "ImageSearchAgent",
            "image_search",
            "Image Search Specialist",
            "Finding existing images, visual references, or photos on the web.",
            true,
            ModelTier::Fast,
        ),
        (
            "ImageAgent",
            "image_generation",
            "Creative Image Generator",
            "Creating or editing images, logos, and visual assets.",
            true,
            ModelTier::Standard,
        ),
        (
            "ExecutionAgent",
            "execution",
            "Code & Sandbox Executor",
            "Running isolated scripts, calculations, and data transformations.",
            false,
            ModelTier::Standard,
        ),
        (
            "DocumentationAgent",
            "documentation",
            "Technical Documentation Specialist",
            "API references, architecture decision records, and developer guides.",
            false,
            ModelTier::Standard,
        ),
        (
            "ProjectAgent",
            "project",
            "Project & Milestone Manager",
            "Tracking deliverables, tasks, timelines, and project workspaces.",
            false,
            ModelTier::Standard,
        ),
        (
            "DocumentReaderAgent",
            "document_reader",
            "Document & PDF Reader",
            "Reads attached files of any kind (PDFs, Word, spreadsheets, slides, emails, ebooks, web pages, archives and more) cover to cover, several at once, and answers with file, section and line citations.",
            false,
            ModelTier::Standard,
        ),
        (
            "PresentationAgent",
            "presentation",
            "Presentation Deck Author",
            "Designing structured slide decks and executive briefings.",
            false,
            ModelTier::Deep,
        ),
        (
            "DocumentAuthorAgent",
            "document_author",
            "Document & Report Author",
            "Drafting polished reports, memos, and long-form documents.",
            false,
            ModelTier::Deep,
        ),
        (
            "SpreadsheetAgent",
            "spreadsheet",
            "Spreadsheet & Financial Modeler",
            "Building structured tables, formulas, and quantitative models.",
            false,
            ModelTier::Standard,
        ),
        (
            "GoogleWorkspace",
            "google_workspace",
            "Google Workspace Specialist",
            "Gmail, Google Calendar, Google Drive, and Google Docs integration.",
            false,
            ModelTier::Standard,
        ),
        (
            "MicrosoftWorkspace",
            "microsoft_workspace",
            "Microsoft 365 Specialist",
            "Outlook, Teams, OneDrive, SharePoint, Word, and Excel integration.",
            false,
            ModelTier::Standard,
        ),
        (
            "SchedulerAgent",
            "scheduler",
            "Automation & Schedule Manager",
            "Creating, pausing, resuming, and monitoring recurring agent routines.",
            false,
            ModelTier::Standard,
        ),
        (
            "TodosAgent",
            "todos",
            "Personal To-Do Specialist",
            "Adding, listing, prioritizing, and completing personal action items.",
            true,
            ModelTier::Fast,
        ),
        (
            "GitHubAgent",
            "github",
            "GitHub & Pull Request Specialist",
            "Repositories, issues, pull requests, code reviews, and CI checks.",
            false,
            ModelTier::Deep,
        ),
    ];

    specs
        .into_iter()
        .map(
            |(id, route_key, name, description, direct_dispatchable, preferred_tier)| {
                SpecialistAgentSpec {
                    id: id.to_owned(),
                    route_key: route_key.to_owned(),
                    name: name.to_owned(),
                    description: description.to_owned(),
                    direct_dispatchable,
                    preferred_tier,
                    model_override: None,
                }
            },
        )
        .collect()
}

/// Resolves the canonical `agent_map` ID from a Jev/Laya specialist route key.
pub fn specialist_id_for_route_key<'a>(
    agents: &'a [SpecialistAgentSpec],
    route_key: &str,
) -> Option<&'a str> {
    agents
        .iter()
        .find(|a| a.route_key == route_key && a.direct_dispatchable)
        .map(|a| a.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_all_22_sovra_srx_specialists_and_resolves_dispatchable_keys() {
        let agents = default_sovra_srx_agents();
        assert_eq!(agents.len(), 22);
        assert_eq!(
            specialist_id_for_route_key(&agents, "coding"),
            Some("CodingAgent")
        );
        assert_eq!(
            specialist_id_for_route_key(&agents, "web_search"),
            Some("WebSearchAgent")
        );
        assert_eq!(specialist_id_for_route_key(&agents, "scheduler"), None);
    }
}
