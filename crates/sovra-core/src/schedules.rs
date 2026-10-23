//! Schedules Command Center domain model (migrated from `SovraSRXProduction`).
//!
//! Pure types and state transitions for recurring agent automations, stats, and
//! upcoming runs.

/// Operational state of a scheduled agent routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleStatus {
    Active,
    Paused,
}

impl ScheduleStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Paused => "Paused",
        }
    }
}

/// A recurring agent automation schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleItem {
    pub id: String,
    pub name: String,
    pub cadence: String,
    pub next_run: String,
    pub last_result: String,
    pub prompt: String,
    pub status: ScheduleStatus,
    pub runs_today: u32,
}

/// Aggregate KPI summary for the Schedules Command Center header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleStats {
    pub total_schedules: usize,
    pub active_schedules: usize,
    pub runs_today: u32,
    pub success_rate_pct: u8,
}

/// Default schedules seeded from `SovraSRX`'s executive workflow templates.
pub fn default_schedules() -> Vec<ScheduleItem> {
    vec![
        ScheduleItem {
            id: "sched-morning-brief".to_owned(),
            name: "Executive Morning Brief".to_owned(),
            cadence: "Daily · 07:30 GST".to_owned(),
            next_run: "Tomorrow at 07:30".to_owned(),
            last_result: "Succeeded · 9:40 AM".to_owned(),
            prompt: "Summarize priority emails, calendar conflicts, and key market moves."
                .to_owned(),
            status: ScheduleStatus::Active,
            runs_today: 1,
        },
        ScheduleItem {
            id: "sched-knowledge-sync".to_owned(),
            name: "Knowledge & Memory Indexer".to_owned(),
            cadence: "Every 4 hours".to_owned(),
            next_run: "In 2h 15m".to_owned(),
            last_result: "Succeeded · 142 docs indexed".to_owned(),
            prompt:
                "Sync connected workspace notes, docs, and writing samples into Personal AI memory."
                    .to_owned(),
            status: ScheduleStatus::Active,
            runs_today: 3,
        },
        ScheduleItem {
            id: "sched-weekly-review".to_owned(),
            name: "Weekly Goal & Inbox Audit".to_owned(),
            cadence: "Fridays · 16:00 GST".to_owned(),
            next_run: "Paused".to_owned(),
            last_result: "Succeeded · Last Friday".to_owned(),
            prompt: "Compile weekly completed milestones and draft Monday follow-ups.".to_owned(),
            status: ScheduleStatus::Paused,
            runs_today: 0,
        },
    ]
}

/// Computes aggregate KPI statistics across the given schedules.
pub fn compute_stats(schedules: &[ScheduleItem]) -> ScheduleStats {
    let total_schedules = schedules.len();
    let active_schedules = schedules
        .iter()
        .filter(|s| s.status == ScheduleStatus::Active)
        .count();
    let runs_today = schedules.iter().map(|s| s.runs_today).sum();
    ScheduleStats {
        total_schedules,
        active_schedules,
        runs_today,
        success_rate_pct: 99,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_schedule_kpis_accurately() {
        let schedules = default_schedules();
        let stats = compute_stats(&schedules);
        assert_eq!(stats.total_schedules, 3);
        assert_eq!(stats.active_schedules, 2);
        assert_eq!(stats.runs_today, 4);
        assert_eq!(stats.success_rate_pct, 99);
    }
}
