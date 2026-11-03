//! Deep research board state and plan parsing.
//!
//! Pure helpers for the in-chat research team: parse a plan into goals, track
//! each goal's lane (waiting / working / done), and shape AG-UI snapshots so
//! the chat can show a kanban without a separate window.

use serde::{Deserialize, Serialize};

/// Skill name and specialist id that always take the parallel research path.
pub const DEEP_RESEARCH_SKILL: &str = "deep-research";
pub const DEEP_RESEARCH_AGENT_ID: &str = "DeepResearchAgent";

/// Whether this skill invocation must run as the parallel research team.
pub fn is_deep_research_skill(skill_name: &str) -> bool {
    skill_name == DEEP_RESEARCH_SKILL
}

/// Whether the tools list names the deep-research specialist.
pub fn skill_requests_deep_research(tools: &[String]) -> bool {
    tools.iter().any(|tool| tool == DEEP_RESEARCH_AGENT_ID)
}

/// Kind of goal from the research plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchGoalKind {
    Research,
    Deliverable,
}

/// Where a research goal sits on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchGoalState {
    Waiting,
    Working,
    Done,
    Failed,
}

impl ResearchGoalState {
    /// Plain-words lane label shown above the column.
    pub fn lane_label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting",
            Self::Working => "Working",
            Self::Done | Self::Failed => "Done",
        }
    }
}

/// One research or deliverable goal on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchGoal {
    pub id: String,
    pub kind: ResearchGoalKind,
    pub text: String,
    pub state: ResearchGoalState,
    /// Short note once finished (kept off the transcript; board-only).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Phase of the overall research run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhase {
    Planning,
    Researching,
    Writing,
    Done,
}

/// Live AG-UI board for a deep-research run (`activityType: "sovra.research"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchBoard {
    pub title: String,
    /// Plain-words status ("Planning your research…", "3 helpers working…").
    pub status: String,
    pub phase: ResearchPhase,
    pub goals: Vec<ResearchGoal>,
    pub finished: bool,
}

impl ResearchBoard {
    pub fn planning(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            status: "Planning your research…".to_owned(),
            phase: ResearchPhase::Planning,
            goals: Vec::new(),
            finished: false,
        }
    }

    /// Replaces goals from a parsed plan; every goal starts Waiting.
    pub fn with_plan(&mut self, goals: Vec<ResearchGoal>) {
        self.goals = goals;
        self.phase = ResearchPhase::Researching;
        let research = self
            .goals
            .iter()
            .filter(|g| g.kind == ResearchGoalKind::Research)
            .count();
        self.status = if research == 0 {
            "No research goals yet".to_owned()
        } else if research == 1 {
            "1 research goal ready…".to_owned()
        } else {
            format!("{research} research goals ready…")
        };
    }

    pub fn set(&mut self, goal_id: &str, state: ResearchGoalState) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            let label = goal.text.clone();
            goal.state = state;
            self.status = match state {
                ResearchGoalState::Working => {
                    let short = truncate_words(&label, 8);
                    format!("Looking into: {short}")
                }
                ResearchGoalState::Done => {
                    let done = self
                        .goals
                        .iter()
                        .filter(|g| {
                            matches!(g.state, ResearchGoalState::Done | ResearchGoalState::Failed)
                        })
                        .count();
                    let total = self.goals.len();
                    format!("{done} of {total} goals finished")
                }
                ResearchGoalState::Failed => {
                    format!("Could not finish: {}", truncate_words(&label, 8))
                }
                ResearchGoalState::Waiting => self.status.clone(),
            };
        }
    }

    pub fn set_note(&mut self, goal_id: &str, note: impl Into<String>) {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            goal.note = note.into();
        }
    }

    pub fn begin_writing(&mut self) {
        self.phase = ResearchPhase::Writing;
        "Writing your research document…".clone_into(&mut self.status);
    }

    pub fn finish(&mut self, status: impl Into<String>) {
        self.finished = true;
        self.phase = ResearchPhase::Done;
        self.status = status.into();
        for goal in &mut self.goals {
            if matches!(
                goal.state,
                ResearchGoalState::Waiting | ResearchGoalState::Working
            ) {
                goal.state = ResearchGoalState::Failed;
            }
        }
    }

    /// Goals grouped into waiting / working / done lanes for the kanban.
    pub fn lanes(&self) -> ResearchLanes<'_> {
        let mut waiting = Vec::new();
        let mut working = Vec::new();
        let mut done = Vec::new();
        for goal in &self.goals {
            match goal.state {
                ResearchGoalState::Waiting => waiting.push(goal),
                ResearchGoalState::Working => working.push(goal),
                ResearchGoalState::Done | ResearchGoalState::Failed => done.push(goal),
            }
        }
        ResearchLanes {
            waiting,
            working,
            done,
        }
    }

    pub fn working_count(&self) -> usize {
        self.goals
            .iter()
            .filter(|g| g.state == ResearchGoalState::Working)
            .count()
    }
}

/// Borrowed lane lists for rendering.
#[derive(Debug)]
pub struct ResearchLanes<'a> {
    pub waiting: Vec<&'a ResearchGoal>,
    pub working: Vec<&'a ResearchGoal>,
    pub done: Vec<&'a ResearchGoal>,
}

/// A goal line parsed from the plan generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedResearchGoal {
    pub index: usize,
    pub kind: ResearchGoalKind,
    pub text: String,
}

impl ParsedResearchGoal {
    pub fn into_goal(self) -> ResearchGoal {
        ResearchGoal {
            id: format!("goal-{}", self.index),
            kind: self.kind,
            text: self.text,
            state: ResearchGoalState::Waiting,
            note: String::new(),
        }
    }
}

/// Extract typed goals from a plan (bullets with `[RESEARCH]` / `[DELIVERABLE]`).
///
/// Falls back to plain bullet/numbered lines as research goals when tags are absent.
pub fn parse_research_plan(plan_text: &str) -> Vec<ParsedResearchGoal> {
    let mut goals = Vec::new();
    for line in plan_text.lines() {
        let stripped = line.trim();
        if stripped.is_empty() {
            continue;
        }
        if let Some(goal) = parse_tagged_line(stripped) {
            goals.push(ParsedResearchGoal {
                index: goals.len(),
                kind: goal.0,
                text: goal.1,
            });
        }
    }
    if goals.is_empty() {
        for line in plan_text.lines() {
            let cleaned = strip_bullet(line.trim());
            if cleaned.chars().count() > 10 {
                goals.push(ParsedResearchGoal {
                    index: goals.len(),
                    kind: ResearchGoalKind::Research,
                    text: cleaned,
                });
            }
        }
    }
    goals
}

fn parse_tagged_line(line: &str) -> Option<(ResearchGoalKind, String)> {
    let cleaned = strip_bullet(line);
    let upper = cleaned.to_ascii_uppercase();
    let (prefix, kind) = [
        ("[RESEARCH]", ResearchGoalKind::Research),
        ("[DELIVERABLE]", ResearchGoalKind::Deliverable),
        ("RESEARCH:", ResearchGoalKind::Research),
        ("DELIVERABLE:", ResearchGoalKind::Deliverable),
    ]
    .into_iter()
    .find(|(prefix, _)| upper.starts_with(prefix))?;
    let rest = cleaned.get(prefix.len()..)?.trim();
    (rest.chars().count() > 5).then(|| (kind, rest.to_owned()))
}

fn strip_bullet(line: &str) -> String {
    let mut chars = line.chars().peekable();
    match chars.peek() {
        Some('-' | '*' | '•') => {
            chars.next();
            chars.next_if(|c| c.is_whitespace());
        }
        Some(c) if c.is_ascii_digit() => {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            chars.next_if(|c| *c == '.' || *c == ')');
            chars.next_if(|c| c.is_whitespace());
        }
        _ => {}
    }
    chars.collect::<String>().trim().to_owned()
}

fn truncate_words(text: &str, max_words: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        return text.to_owned();
    }
    format!("{}…", words[..max_words].join(" "))
}

/// Short title for the board from the user's research question.
pub fn research_board_title(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return "Deep research".to_owned();
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() <= 10 {
        trimmed.to_owned()
    } else {
        format!("{}…", words[..10].join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_plan_splits_research_and_deliverable() {
        let plan = "\
- [RESEARCH] Investigate heat pump efficiency in cold climates
- [RESEARCH] Compare running costs vs gas boilers
* [DELIVERABLE] Write a side-by-side summary for homeowners
";
        let goals = parse_research_plan(plan);
        assert_eq!(goals.len(), 3);
        assert_eq!(goals[0].kind, ResearchGoalKind::Research);
        assert!(goals[0].text.contains("heat pump"));
        assert_eq!(goals[2].kind, ResearchGoalKind::Deliverable);
        assert_eq!(goals[0].index, 0);
        assert_eq!(goals[2].index, 2);
    }

    #[test]
    fn untagged_bullets_become_research_goals() {
        let plan = "\
1. Look up recent heat pump subsidies
2. Check typical install costs for a small home
";
        let goals = parse_research_plan(plan);
        assert_eq!(goals.len(), 2);
        assert!(goals.iter().all(|g| g.kind == ResearchGoalKind::Research));
        assert!(goals[0].text.contains("subsidies"));
    }

    #[test]
    fn board_lanes_and_transitions() {
        let mut board = ResearchBoard::planning("Heat pumps");
        assert_eq!(board.phase, ResearchPhase::Planning);
        let goals: Vec<_> = parse_research_plan(
            "- [RESEARCH] Goal one about heat pumps today\n- [RESEARCH] Goal two about gas boilers now\n- [DELIVERABLE] Write the homeowner summary document",
        )
        .into_iter()
        .map(ParsedResearchGoal::into_goal)
        .collect();
        board.with_plan(goals);
        assert_eq!(board.phase, ResearchPhase::Researching);
        assert_eq!(board.lanes().waiting.len(), 3);
        board.set("goal-0", ResearchGoalState::Working);
        board.set("goal-1", ResearchGoalState::Working);
        assert_eq!(board.working_count(), 2);
        assert_eq!(board.lanes().working.len(), 2);
        board.set("goal-0", ResearchGoalState::Done);
        board.set_note("goal-0", "Found three solid sources");
        board.set("goal-1", ResearchGoalState::Done);
        board.begin_writing();
        assert_eq!(board.phase, ResearchPhase::Writing);
        board.set("goal-2", ResearchGoalState::Working);
        board.set("goal-2", ResearchGoalState::Done);
        board.finish("Your research document is ready");
        assert!(board.finished);
        assert_eq!(board.lanes().done.len(), 3);
        assert_eq!(board.lanes().waiting.len(), 0);
        assert_eq!(board.lanes().working.len(), 0);
    }

    #[test]
    fn skill_detection() {
        assert!(is_deep_research_skill("deep-research"));
        assert!(!is_deep_research_skill("web-search"));
        assert!(skill_requests_deep_research(&["DeepResearchAgent".into()]));
        assert!(!skill_requests_deep_research(&["WebSearchAgent".into()]));
    }
}
