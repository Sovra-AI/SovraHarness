//! Local automations: one-shot delays, repeating cadence, next-run, and ticks.
//!
//! Pure and clock-free. Callers supply [`Timestamp`] and a timezone offset in
//! minutes east of UTC. The runtime owns sleeping, persistence, and dispatch.

use serde::{Deserialize, Serialize};

use crate::chat_history::format_relative_time;
use crate::communications::{concrete_scheduled_recipient, validate_message};
use crate::connections::ChannelKind;
use crate::memory::Timestamp;

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const MAX_RUNS: usize = 200;

/// Operational state of a scheduled routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// Civil weekday, Monday first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub fn label(self) -> &'static str {
        match self {
            Self::Monday => "Monday",
            Self::Tuesday => "Tuesday",
            Self::Wednesday => "Wednesday",
            Self::Thursday => "Thursday",
            Self::Friday => "Friday",
            Self::Saturday => "Saturday",
            Self::Sunday => "Sunday",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Monday => "Mondays",
            Self::Tuesday => "Tuesdays",
            Self::Wednesday => "Wednesdays",
            Self::Thursday => "Thursdays",
            Self::Friday => "Fridays",
            Self::Saturday => "Saturdays",
            Self::Sunday => "Sundays",
        }
    }

    fn from_unix_days(days: i64) -> Self {
        match days.rem_euclid(7) {
            0 => Self::Thursday,
            1 => Self::Friday,
            2 => Self::Saturday,
            3 => Self::Sunday,
            4 => Self::Monday,
            5 => Self::Tuesday,
            _ => Self::Wednesday,
        }
    }
}

/// How often a schedule fires, in the person's local civil time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleCadence {
    Daily {
        hour: u8,
        minute: u8,
    },
    Weekdays {
        hour: u8,
        minute: u8,
    },
    Weekly {
        weekday: Weekday,
        hour: u8,
        minute: u8,
    },
    EveryHours {
        hours: u32,
    },
    /// A single fire `delay_ms` after next-run is computed. `0` means soon.
    Once {
        delay_ms: i64,
    },
}

impl ScheduleCadence {
    pub fn label(self) -> String {
        match self {
            Self::Daily { hour, minute } => format!("Every day at {}", clock(hour, minute)),
            Self::Weekdays { hour, minute } => format!("Weekdays at {}", clock(hour, minute)),
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => format!("{} at {}", weekday.plural(), clock(hour, minute)),
            Self::EveryHours { hours: 1 } => "Every hour".to_owned(),
            Self::EveryHours { hours } => format!("Every {hours} hours"),
            Self::Once { delay_ms } => once_label(delay_ms),
        }
    }

    fn is_one_shot(self) -> bool {
        matches!(self, Self::Once { .. })
    }
}

/// Confirmed `WhatsApp` or Telegram text that transmits when the job fires.
///
/// Consent is the create step (who, which app, exact wording). Fire does not
/// wait for another `Send` click.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledSend {
    pub channel: ChannelKind,
    pub recipient: String,
    pub text: String,
}

/// A recurring prompt the runtime fires on cadence, optionally a timed send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleItem {
    pub id: String,
    pub name: String,
    pub cadence: ScheduleCadence,
    pub prompt: String,
    /// When set, fire transmits this message instead of running a chat prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<ScheduledSend>,
    pub status: ScheduleStatus,
    pub created_at: Timestamp,
    pub next_run_at: Timestamp,
    pub tz_offset_minutes: i32,
}

impl ScheduleItem {
    pub fn cadence_label(&self) -> String {
        self.cadence.label()
    }

    pub fn next_run_label(&self, now: Timestamp) -> String {
        if self.status == ScheduleStatus::Paused {
            return "Paused".to_owned();
        }
        if self.next_run_at.0 <= now.0 {
            return "Due now".to_owned();
        }
        let next = civil_clock(self.next_run_at, self.tz_offset_minutes);
        let today = civil_clock(now, self.tz_offset_minutes);
        if next.days == today.days {
            format!("Today at {}", clock(next.hour, next.minute))
        } else if next.days == today.days + 1 {
            format!("Tomorrow at {}", clock(next.hour, next.minute))
        } else {
            format!(
                "{} at {}",
                Weekday::from_unix_days(next.days).label(),
                clock(next.hour, next.minute)
            )
        }
    }

    pub fn last_result_label(&self, runs: &[ScheduleRun], now: Timestamp) -> String {
        let Some(run) = runs.iter().rev().find(|run| run.schedule_id == self.id) else {
            return "Never run".to_owned();
        };
        if run.finished_at.is_none() {
            return "Running now".to_owned();
        }
        let when = format_relative_time(run.finished_at.unwrap_or(run.started_at), now);
        let stamp = if when == "now" {
            "just now".to_owned()
        } else {
            format!("{when} ago")
        };
        match run.outcome {
            Some(RunOutcome::Succeeded) => format!("Succeeded · {stamp}"),
            Some(RunOutcome::Failed) => match run
                .error
                .as_deref()
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
            {
                Some(reason) => format!("Didn't finish · {stamp} · {reason}"),
                None => format!("Didn't finish · {stamp}"),
            },
            Some(RunOutcome::Skipped) => format!("Skipped · {stamp}"),
            None => "Running now".to_owned(),
        }
    }
}

/// How a recorded run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Succeeded,
    Failed,
    Skipped,
}

/// One firing of a schedule, including in-flight work (`finished_at` is `None`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRun {
    pub id: String,
    pub schedule_id: String,
    pub session_id: String,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub outcome: Option<RunOutcome>,
    /// Why a failed run didn't finish. Absent on older saved books.
    #[serde(default)]
    pub error: Option<String>,
}

/// Persisted jobs plus bounded run history.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScheduleBook {
    #[serde(default)]
    pub schedules: Vec<ScheduleItem>,
    #[serde(default)]
    pub runs: Vec<ScheduleRun>,
    #[serde(default)]
    pub next_seq: u64,
}

impl ScheduleBook {
    fn alloc_id(&mut self, prefix: &str) -> String {
        self.next_seq = self.next_seq.saturating_add(1);
        format!("{prefix}-{}", self.next_seq)
    }

    fn in_flight(&self, schedule_id: &str) -> bool {
        self.runs.iter().any(|run| {
            run.schedule_id == schedule_id && run.finished_at.is_none() && run.outcome.is_none()
        })
    }

    fn push_run(&mut self, run: ScheduleRun) {
        self.runs.push(run);
        let extra = self.runs.len().saturating_sub(MAX_RUNS);
        if extra > 0 {
            self.runs.drain(0..extra);
        }
    }
}

/// Aggregate header figures. Success rate uses finished runs in the last 30 days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleStats {
    pub total_schedules: usize,
    pub active_schedules: usize,
    pub runs_today: u32,
    pub success_rate_pct: u8,
}

/// Window drawn on the swimlane graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineRange {
    #[default]
    Hours24,
    Days7,
    Days30,
}

impl TimelineRange {
    pub const ALL: [Self; 3] = [Self::Hours24, Self::Days7, Self::Days30];

    pub fn label(self) -> &'static str {
        match self {
            Self::Hours24 => "24 hours",
            Self::Days7 => "7 days",
            Self::Days30 => "30 days",
        }
    }

    pub fn duration_ms(self) -> i64 {
        match self {
            Self::Hours24 => 24 * HOUR_MS,
            Self::Days7 => 7 * DAY_MS,
            Self::Days30 => 30 * DAY_MS,
        }
    }
}

/// One painted mark on the swimlane graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineMark {
    pub schedule_id: String,
    pub schedule_name: String,
    pub at: Timestamp,
    pub kind: TimelineMarkKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineMarkKind {
    Succeeded,
    Failed,
    Skipped,
    InFlight,
    Next,
}

/// A due job the runtime should send through the chat path, or transmit as a
/// confirmed timed text when [`Self::send`] is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueFire {
    pub schedule_id: String,
    pub name: String,
    pub prompt: String,
    pub session_id: String,
    pub send: Option<ScheduledSend>,
}

/// Result of [`tick`]: due Active jobs to fire, plus skipped overlapping ones.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TickEffect {
    pub fire: Vec<DueFire>,
    pub skipped: Vec<String>,
}

/// UI → runtime schedule requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleCommand {
    Load,
    Create {
        name: String,
        cadence: String,
        prompt: String,
        send: Option<ScheduledSend>,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Delete {
        id: String,
    },
    /// `session_id` is `Some` when the UI already dispatched [`crate::Command::RunPrompt`].
    RunNow {
        id: String,
        session_id: Option<String>,
    },
    Tick {
        now: Timestamp,
    },
    /// Runtime-only: close an in-flight run after a timed send finishes.
    FinishRun {
        session_id: String,
        succeeded: bool,
        error: Option<String>,
    },
}

/// Runtime → UI schedule notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleEvent {
    Snapshot {
        schedules: Vec<ScheduleItem>,
        runs: Vec<ScheduleRun>,
    },
    Fired {
        schedule_id: String,
        session_id: String,
        name: String,
    },
    RunRecorded {
        run: ScheduleRun,
    },
    Error(String),
}

/// Persistence handed in by the composition root. Calls run on a blocking worker.
pub trait ScheduleStore: Send + Sync + std::fmt::Debug {
    fn load(&self) -> Result<ScheduleBook, ScheduleError>;
    fn save(&self, book: &ScheduleBook) -> Result<(), ScheduleError>;
}

/// Store or parse failure shown in plain words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ScheduleError(pub String);

/// A new workspace starts with no schedules.
pub fn default_schedules() -> Vec<ScheduleItem> {
    Vec::new()
}

/// Parses plain-words cadence. Empty or `"daily"` means every day at 9:00.
/// Delay phrases (`"in 1 minute"`, `"just once"`) are a single fire, never Daily.
pub fn parse_cadence(input: &str) -> ScheduleCadence {
    let raw = input.trim();
    if raw.is_empty() {
        return ScheduleCadence::Daily { hour: 9, minute: 0 };
    }
    let s = raw.to_ascii_lowercase();
    let s = s
        .replace(['·', ','], " ")
        .replace("gst", " ")
        .replace("utc", " ");
    let compact = collapse_ws(&s);

    if let Some(once) = parse_once_cadence(&compact) {
        return once;
    }
    if let Some(hours) = parse_every_hours(&compact) {
        return ScheduleCadence::EveryHours {
            hours: hours.clamp(1, 168),
        };
    }
    let (hour, minute) = parse_clock(&compact).unwrap_or((9, 0));
    if let Some(weekday) = parse_weekday(&compact) {
        return ScheduleCadence::Weekly {
            weekday,
            hour,
            minute,
        };
    }
    if compact.contains("weekday") || compact.contains("workday") {
        return ScheduleCadence::Weekdays { hour, minute };
    }
    ScheduleCadence::Daily { hour, minute }
}

/// Next instant for `cadence` after `from` in the given offset.
///
/// Repeating civil cadences are strictly after `from`. A one-shot is `from`
/// plus its delay (`0` means due at `from`).
pub fn next_run_after(
    cadence: ScheduleCadence,
    tz_offset_minutes: i32,
    from: Timestamp,
) -> Timestamp {
    match cadence {
        ScheduleCadence::EveryHours { hours } => {
            let step = i64::from(hours.max(1)) * HOUR_MS;
            Timestamp(from.0.saturating_add(step))
        }
        ScheduleCadence::Daily { hour, minute } => {
            next_civil_after(from, tz_offset_minutes, hour, minute, None, false)
        }
        ScheduleCadence::Weekdays { hour, minute } => {
            next_civil_after(from, tz_offset_minutes, hour, minute, None, true)
        }
        ScheduleCadence::Weekly {
            weekday,
            hour,
            minute,
        } => next_civil_after(from, tz_offset_minutes, hour, minute, Some(weekday), false),
        ScheduleCadence::Once { delay_ms } => Timestamp(from.0.saturating_add(delay_ms.max(0))),
    }
}

/// Fires due **Active** jobs, skips overlapping in-flight work, ignores **Paused**.
pub fn tick(book: &mut ScheduleBook, now: Timestamp) -> TickEffect {
    let mut effect = TickEffect::default();
    let due: Vec<String> = book
        .schedules
        .iter()
        .filter(|item| item.status == ScheduleStatus::Active && item.next_run_at.0 <= now.0)
        .map(|item| item.id.clone())
        .collect();
    for id in due {
        if book.in_flight(&id) {
            let run_id = book.alloc_id("sched-run");
            book.push_run(ScheduleRun {
                id: run_id,
                schedule_id: id.clone(),
                session_id: String::new(),
                started_at: now,
                finished_at: Some(now),
                outcome: Some(RunOutcome::Skipped),
                error: None,
            });
            if let Some(item) = book.schedules.iter_mut().find(|item| item.id == id) {
                follow_on_after_run(item, now);
            }
            effect.skipped.push(id);
            continue;
        }
        let Some(item) = book.schedules.iter().find(|item| item.id == id).cloned() else {
            continue;
        };
        let session_id = format!("{}-at-{}", item.id, now.0);
        let fire = due_fire(&item, session_id.clone());
        let run_id = book.alloc_id("sched-run");
        if let Some(item) = book.schedules.iter_mut().find(|item| item.id == id) {
            follow_on_after_run(item, now);
        }
        book.push_run(ScheduleRun {
            id: run_id,
            schedule_id: item.id,
            session_id,
            started_at: now,
            finished_at: None,
            outcome: None,
            error: None,
        });
        effect.fire.push(fire);
    }
    effect
}

/// Adds a schedule. Empty name or prompt is rejected. A timed send must name
/// a recipient and have sendable text.
pub fn create_in_book(
    book: &mut ScheduleBook,
    name: &str,
    cadence: &str,
    prompt: &str,
    send: Option<ScheduledSend>,
    now: Timestamp,
    tz_offset_minutes: i32,
) -> Result<ScheduleItem, ScheduleError> {
    let name = name.trim();
    let prompt = prompt.trim();
    if name.is_empty() || prompt.is_empty() {
        return Err(ScheduleError(
            "Give the task a name and say what Sovra should do.".into(),
        ));
    }
    let send = match send {
        Some(send) => Some(validated_send(send)?),
        None => None,
    };
    let cadence = parse_cadence(cadence);
    let id = book.alloc_id("sched");
    let item = ScheduleItem {
        id,
        name: name.to_owned(),
        cadence,
        prompt: prompt.to_owned(),
        send,
        status: ScheduleStatus::Active,
        created_at: now,
        next_run_at: next_run_after(cadence, tz_offset_minutes, now),
        tz_offset_minutes,
    };
    book.schedules.push(item.clone());
    Ok(item)
}

pub fn pause_in_book(book: &mut ScheduleBook, id: &str) -> Result<(), ScheduleError> {
    let item = book
        .schedules
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| ScheduleError("That repeating task is no longer here.".into()))?;
    item.status = ScheduleStatus::Paused;
    Ok(())
}

pub fn resume_in_book(
    book: &mut ScheduleBook,
    id: &str,
    now: Timestamp,
) -> Result<(), ScheduleError> {
    let item = book
        .schedules
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| ScheduleError("That repeating task is no longer here.".into()))?;
    item.status = ScheduleStatus::Active;
    item.next_run_at = next_run_after(item.cadence, item.tz_offset_minutes, now);
    Ok(())
}

pub fn delete_in_book(book: &mut ScheduleBook, id: &str) -> bool {
    let before = book.schedules.len();
    book.schedules.retain(|item| item.id != id);
    book.schedules.len() < before
}

/// Records a user or helper "run now". Returns a fire when the runtime should dispatch.
pub fn run_now_in_book(
    book: &mut ScheduleBook,
    id: &str,
    session_id: Option<String>,
    now: Timestamp,
) -> Result<Option<DueFire>, ScheduleError> {
    let item = book
        .schedules
        .iter()
        .find(|item| item.id == id)
        .cloned()
        .ok_or_else(|| ScheduleError("That repeating task is no longer here.".into()))?;
    if book.in_flight(&item.id) {
        return Ok(None);
    }
    let dispatch = session_id.is_none();
    let session_id = session_id.unwrap_or_else(|| format!("{}-now-{}", item.id, now.0));
    let fire = due_fire(&item, session_id.clone());
    let run_id = book.alloc_id("sched-run");
    book.push_run(ScheduleRun {
        id: run_id,
        schedule_id: item.id.clone(),
        session_id,
        started_at: now,
        finished_at: None,
        outcome: None,
        error: None,
    });
    if item.cadence.is_one_shot()
        && let Some(stored) = book.schedules.iter_mut().find(|row| row.id == item.id)
    {
        stored.status = ScheduleStatus::Paused;
    }
    if dispatch { Ok(Some(fire)) } else { Ok(None) }
}

fn due_fire(item: &ScheduleItem, session_id: String) -> DueFire {
    DueFire {
        schedule_id: item.id.clone(),
        name: item.name.clone(),
        prompt: format!("[Schedule: {}] {}", item.name, item.prompt),
        session_id,
        send: item.send.clone(),
    }
}

fn validated_send(send: ScheduledSend) -> Result<ScheduledSend, ScheduleError> {
    let recipient = send.recipient.trim();
    if recipient.is_empty() {
        return Err(ScheduleError(
            "Say who the timed message should go to.".into(),
        ));
    }
    concrete_scheduled_recipient(send.channel, recipient)
        .map_err(|error| ScheduleError(error.to_string()))?;
    validate_message(&send.text).map_err(|error| ScheduleError(error.to_string()))?;
    Ok(ScheduledSend {
        channel: send.channel,
        recipient: recipient.to_owned(),
        text: send.text,
    })
}

/// One-shots pause after they fire so a later tick cannot run them again.
fn follow_on_after_run(item: &mut ScheduleItem, now: Timestamp) {
    if item.cadence.is_one_shot() {
        item.status = ScheduleStatus::Paused;
        return;
    }
    item.next_run_at = next_run_after(item.cadence, item.tz_offset_minutes, now);
}

/// Marks unfinished runs as failed so a later tick can fire. Returns whether any changed.
pub fn fail_stale_runs(book: &mut ScheduleBook, now: Timestamp) -> bool {
    let mut changed = false;
    for run in &mut book.runs {
        if run.finished_at.is_none() {
            run.finished_at = Some(now);
            run.outcome = Some(RunOutcome::Failed);
            changed = true;
        }
    }
    changed
}

/// Returns whether a matching in-flight run was closed.
pub fn complete_run(
    book: &mut ScheduleBook,
    session_id: &str,
    succeeded: bool,
    error: Option<String>,
    now: Timestamp,
) -> bool {
    if let Some(run) = book
        .runs
        .iter_mut()
        .rev()
        .find(|run| run.session_id == session_id && run.finished_at.is_none())
    {
        run.finished_at = Some(now);
        run.outcome = Some(if succeeded {
            RunOutcome::Succeeded
        } else {
            RunOutcome::Failed
        });
        run.error = if succeeded {
            None
        } else {
            error
                .map(|reason| reason.trim().to_owned())
                .filter(|reason| !reason.is_empty())
        };
        true
    } else {
        false
    }
}

pub fn compute_stats(book: &ScheduleBook, now: Timestamp) -> ScheduleStats {
    let total_schedules = book.schedules.len();
    let active_schedules = book
        .schedules
        .iter()
        .filter(|item| item.status == ScheduleStatus::Active)
        .count();
    let offset = book
        .schedules
        .first()
        .map_or(0, |item| item.tz_offset_minutes);
    let today = civil_clock(now, offset).days;
    let month_ago = Timestamp(now.0.saturating_sub(30 * DAY_MS));
    let runs_today = u32::try_from(
        book.runs
            .iter()
            .filter(|run| civil_clock(run.started_at, offset).days == today)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let window: Vec<_> = book
        .runs
        .iter()
        .filter(|run| run.started_at.0 >= month_ago.0)
        .filter_map(|run| run.outcome)
        .collect();
    let succeeded = window
        .iter()
        .filter(|outcome| **outcome == RunOutcome::Succeeded)
        .count();
    let decided = window
        .iter()
        .filter(|outcome| matches!(**outcome, RunOutcome::Succeeded | RunOutcome::Failed))
        .count();
    let success_rate_pct = u8::try_from(
        succeeded
            .saturating_mul(100)
            .checked_div(decided)
            .unwrap_or(0),
    )
    .unwrap_or(0);
    ScheduleStats {
        total_schedules,
        active_schedules,
        runs_today,
        success_rate_pct,
    }
}

pub fn timeline_marks(
    book: &ScheduleBook,
    now: Timestamp,
    range: TimelineRange,
) -> Vec<TimelineMark> {
    let span = range.duration_ms();
    let start = Timestamp(now.0.saturating_sub(span));
    let end = Timestamp(now.0.saturating_add(span));
    let mut marks = Vec::new();
    for run in &book.runs {
        if run.started_at.0 < start.0 || run.started_at.0 > end.0 {
            continue;
        }
        let Some(item) = book
            .schedules
            .iter()
            .find(|item| item.id == run.schedule_id)
        else {
            continue;
        };
        let kind = match run.outcome {
            Some(RunOutcome::Succeeded) => TimelineMarkKind::Succeeded,
            Some(RunOutcome::Failed) => TimelineMarkKind::Failed,
            Some(RunOutcome::Skipped) => TimelineMarkKind::Skipped,
            None => TimelineMarkKind::InFlight,
        };
        marks.push(TimelineMark {
            schedule_id: item.id.clone(),
            schedule_name: item.name.clone(),
            at: run.started_at,
            kind,
        });
    }
    for item in &book.schedules {
        if item.status == ScheduleStatus::Paused {
            continue;
        }
        if item.next_run_at.0 >= start.0 && item.next_run_at.0 <= end.0 {
            marks.push(TimelineMark {
                schedule_id: item.id.clone(),
                schedule_name: item.name.clone(),
                at: item.next_run_at,
                kind: TimelineMarkKind::Next,
            });
        }
    }
    marks
}

pub fn snapshot_event(book: &ScheduleBook) -> ScheduleEvent {
    ScheduleEvent::Snapshot {
        schedules: book.schedules.clone(),
        runs: book.runs.clone(),
    }
}

fn clock(hour: u8, minute: u8) -> String {
    format!("{hour}:{minute:02}")
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn once_label(delay_ms: i64) -> String {
    if delay_ms < MINUTE_MS {
        "Once, soon".to_owned()
    } else if delay_ms == MINUTE_MS {
        "Once, in 1 minute".to_owned()
    } else if delay_ms % HOUR_MS == 0 {
        let hours = delay_ms / HOUR_MS;
        if hours == 1 {
            "Once, in 1 hour".to_owned()
        } else {
            format!("Once, in {hours} hours")
        }
    } else if delay_ms % MINUTE_MS == 0 {
        let minutes = delay_ms / MINUTE_MS;
        format!("Once, in {minutes} minutes")
    } else {
        "Once, soon".to_owned()
    }
}

fn parse_once_cadence(s: &str) -> Option<ScheduleCadence> {
    if let Some(delay_ms) = parse_delay_ms(s) {
        return Some(ScheduleCadence::Once { delay_ms });
    }
    if is_once_phrase(s) {
        return Some(ScheduleCadence::Once { delay_ms: 0 });
    }
    None
}

fn is_once_phrase(s: &str) -> bool {
    matches!(
        s,
        "once"
            | "just once"
            | "only once"
            | "one time"
            | "one-shot"
            | "oneshot"
            | "one shot"
            | "once only"
            | "just the once"
    )
}

fn parse_delay_ms(s: &str) -> Option<i64> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    for i in 0..tokens.len() {
        if tokens[i] == "in" || tokens[i] == "for" {
            let Some(&qty) = tokens.get(i + 1) else {
                continue;
            };
            let unit = tokens.get(i + 2).copied().unwrap_or("");
            if let Some(ms) = qty_unit_ms(qty, unit) {
                return Some(ms);
            }
        }
        if tokens.get(i + 2) == Some(&"from")
            && tokens.get(i + 3) == Some(&"now")
            && let Some(ms) = qty_unit_ms(tokens[i], tokens.get(i + 1).copied().unwrap_or(""))
        {
            return Some(ms);
        }
    }
    None
}

fn qty_unit_ms(qty: &str, unit: &str) -> Option<i64> {
    let n: i64 = match qty {
        "a" | "an" | "one" => 1,
        other => other.parse().ok()?,
    };
    if n < 0 {
        return None;
    }
    let unit = unit.trim_end_matches('.').trim_end_matches('s');
    let step = match unit {
        "second" | "sec" => 1_000,
        "minute" | "min" => MINUTE_MS,
        "hour" | "hr" => HOUR_MS,
        "day" => DAY_MS,
        _ => return None,
    };
    Some(n.saturating_mul(step).min(365 * DAY_MS))
}

fn parse_every_hours(s: &str) -> Option<u32> {
    let rest = s.strip_prefix("every ")?;
    if rest == "hour" || rest == "hourly" {
        return Some(1);
    }
    let rest = rest
        .strip_suffix(" hours")
        .or_else(|| rest.strip_suffix(" hour"))?;
    let rest = rest.trim();
    if rest.is_empty() {
        return Some(1);
    }
    if rest == "four" {
        return Some(4);
    }
    rest.parse().ok()
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    const NAMES: &[(&str, Weekday)] = &[
        ("mondays", Weekday::Monday),
        ("monday", Weekday::Monday),
        ("mon", Weekday::Monday),
        ("tuesdays", Weekday::Tuesday),
        ("tuesday", Weekday::Tuesday),
        ("tue", Weekday::Tuesday),
        ("wednesdays", Weekday::Wednesday),
        ("wednesday", Weekday::Wednesday),
        ("wed", Weekday::Wednesday),
        ("thursdays", Weekday::Thursday),
        ("thursday", Weekday::Thursday),
        ("thu", Weekday::Thursday),
        ("fridays", Weekday::Friday),
        ("friday", Weekday::Friday),
        ("fri", Weekday::Friday),
        ("saturdays", Weekday::Saturday),
        ("saturday", Weekday::Saturday),
        ("sat", Weekday::Saturday),
        ("sundays", Weekday::Sunday),
        ("sunday", Weekday::Sunday),
        ("sun", Weekday::Sunday),
    ];
    for (name, day) in NAMES {
        if s.split_whitespace()
            .any(|word| word.trim_matches('.') == *name)
        {
            return Some(*day);
        }
    }
    None
}

fn parse_clock(s: &str) -> Option<(u8, u8)> {
    if s.contains("morning") && !s.contains(':') && !has_hour_number(s) {
        return Some((9, 0));
    }
    if s.contains("afternoon") && !s.contains(':') && !has_hour_number(s) {
        return Some((15, 0));
    }
    if s.contains("evening") && !s.contains(':') && !has_hour_number(s) {
        return Some((18, 0));
    }
    if s.contains("night") && !s.contains(':') && !has_hour_number(s) {
        return Some((21, 0));
    }
    let tokens: Vec<&str> = s.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if *token == "at"
            && let Some(next) = tokens.get(index + 1)
            && let Some(parsed) = parse_hhmm(next)
        {
            return Some(parsed);
        }
        if let Some(parsed) = parse_hhmm(token) {
            return Some(parsed);
        }
    }
    None
}

fn has_hour_number(s: &str) -> bool {
    s.split_whitespace()
        .any(|token| parse_hhmm(token).is_some())
}

fn parse_hhmm(token: &str) -> Option<(u8, u8)> {
    let compact = token.replace('.', "");
    let (digits, meridiem) = if let Some(rest) = compact.strip_suffix("pm") {
        (rest, Some(true))
    } else if let Some(rest) = compact.strip_suffix("am") {
        (rest, Some(false))
    } else {
        (compact.as_str(), None)
    };
    let (hour_s, minute_s) = digits
        .split_once(':')
        .map_or((digits, "0"), |(h, m)| (h, m));
    let hour: u8 = hour_s.parse().ok()?;
    let minute: u8 = minute_s.parse().ok()?;
    if minute > 59 {
        return None;
    }
    let hour = match meridiem {
        Some(true) if hour < 12 => hour + 12,
        Some(false) if hour == 12 => 0,
        _ => hour,
    };
    if hour > 23 {
        return None;
    }
    Some((hour, minute))
}

struct CivilClock {
    days: i64,
    hour: u8,
    minute: u8,
}

fn civil_clock(at: Timestamp, tz_offset_minutes: i32) -> CivilClock {
    let local = at.0 + i64::from(tz_offset_minutes) * MINUTE_MS;
    let days = local.div_euclid(DAY_MS);
    let tod = local.rem_euclid(DAY_MS);
    CivilClock {
        days,
        hour: u8::try_from(tod / HOUR_MS).unwrap_or(0),
        minute: u8::try_from((tod % HOUR_MS) / MINUTE_MS).unwrap_or(0),
    }
}

fn from_civil(days: i64, hour: u8, minute: u8, tz_offset_minutes: i32) -> Timestamp {
    let local = days * DAY_MS + i64::from(hour) * HOUR_MS + i64::from(minute) * MINUTE_MS;
    Timestamp(local - i64::from(tz_offset_minutes) * MINUTE_MS)
}

fn next_civil_after(
    from: Timestamp,
    tz_offset_minutes: i32,
    hour: u8,
    minute: u8,
    weekday: Option<Weekday>,
    weekdays_only: bool,
) -> Timestamp {
    let start = civil_clock(from, tz_offset_minutes);
    for add in 0..14 {
        let days = start.days + add;
        let candidate = from_civil(days, hour, minute, tz_offset_minutes);
        if candidate.0 <= from.0 {
            continue;
        }
        let day = Weekday::from_unix_days(days);
        if weekdays_only && matches!(day, Weekday::Saturday | Weekday::Sunday) {
            continue;
        }
        if weekday.is_some_and(|wanted| wanted != day) {
            continue;
        }
        return candidate;
    }
    from_civil(start.days + 1, hour, minute, tz_offset_minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NINE: i64 = 9 * HOUR_MS;

    fn book_with(item: ScheduleItem) -> ScheduleBook {
        ScheduleBook {
            next_seq: 1,
            schedules: vec![item],
            runs: Vec::new(),
        }
    }

    #[test]
    fn a_new_workspace_has_no_schedules() {
        assert_eq!(default_schedules(), Vec::<ScheduleItem>::new());
        let stats = compute_stats(&ScheduleBook::default(), Timestamp(0));
        assert_eq!(stats.total_schedules, 0);
        assert_eq!(stats.active_schedules, 0);
        assert_eq!(stats.runs_today, 0);
        assert_eq!(stats.success_rate_pct, 0);
    }

    #[test]
    fn parses_plain_words_cadence() {
        assert_eq!(
            parse_cadence(""),
            ScheduleCadence::Daily { hour: 9, minute: 0 }
        );
        assert_eq!(
            parse_cadence("daily"),
            ScheduleCadence::Daily { hour: 9, minute: 0 }
        );
        assert_eq!(
            parse_cadence("every morning"),
            ScheduleCadence::Daily { hour: 9, minute: 0 }
        );
        assert_eq!(
            parse_cadence("every morning at 7:30"),
            ScheduleCadence::Daily {
                hour: 7,
                minute: 30
            }
        );
        assert_eq!(
            parse_cadence("weekdays at 9"),
            ScheduleCadence::Weekdays { hour: 9, minute: 0 }
        );
        assert_eq!(
            parse_cadence("every 4 hours"),
            ScheduleCadence::EveryHours { hours: 4 }
        );
        assert_eq!(
            parse_cadence("every hour"),
            ScheduleCadence::EveryHours { hours: 1 }
        );
        assert_eq!(
            parse_cadence("fridays at 16:00"),
            ScheduleCadence::Weekly {
                weekday: Weekday::Friday,
                hour: 16,
                minute: 0
            }
        );
        assert_eq!(
            parse_cadence("fridays at 4pm"),
            ScheduleCadence::Weekly {
                weekday: Weekday::Friday,
                hour: 16,
                minute: 0
            }
        );
        assert_eq!(
            parse_cadence("every day at 12:00"),
            ScheduleCadence::Daily {
                hour: 12,
                minute: 0
            }
        );
        assert_eq!(
            parse_cadence("every day at 9pm"),
            ScheduleCadence::Daily {
                hour: 21,
                minute: 0
            }
        );
        assert_eq!(
            parse_cadence("mondays at 8"),
            ScheduleCadence::Weekly {
                weekday: Weekday::Monday,
                hour: 8,
                minute: 0
            }
        );
        assert_eq!(
            parse_cadence("Daily · 09:00 GST"),
            ScheduleCadence::Daily { hour: 9, minute: 0 }
        );
        assert_eq!(
            parse_cadence("in 1 minute"),
            ScheduleCadence::Once {
                delay_ms: MINUTE_MS
            }
        );
        assert_eq!(
            parse_cadence("in a minute"),
            ScheduleCadence::Once {
                delay_ms: MINUTE_MS
            }
        );
        assert_eq!(
            parse_cadence("just once"),
            ScheduleCadence::Once { delay_ms: 0 }
        );
        assert_eq!(
            parse_cadence("3:53"),
            ScheduleCadence::Daily {
                hour: 3,
                minute: 53
            }
        );
    }

    #[test]
    fn delay_and_once_phrases_are_one_shot_not_daily() {
        let now = Timestamp(1_000_000);
        for phrase in ["in 1 minute", "in a minute"] {
            let cadence = parse_cadence(phrase);
            assert!(
                matches!(
                    cadence,
                    ScheduleCadence::Once {
                        delay_ms: MINUTE_MS
                    }
                ),
                "{phrase} became {cadence:?}"
            );
            assert_eq!(
                next_run_after(cadence, 0, now),
                Timestamp(now.0 + MINUTE_MS)
            );
            let label = cadence.label();
            assert!(!label.contains("Every day"), "{phrase} labeled {label}");
            assert_eq!(label, "Once, in 1 minute");
        }
        let once = parse_cadence("just once");
        assert_eq!(once, ScheduleCadence::Once { delay_ms: 0 });
        let once_next = next_run_after(once, 0, now);
        assert_eq!(once_next, now);
        assert!(once_next.0 - now.0 < MINUTE_MS);
        assert_eq!(once.label(), "Once, soon");
        assert!(!once.label().contains("Every day"));
    }

    #[test]
    fn one_shot_fires_once_then_a_later_tick_does_not() {
        let mut book = ScheduleBook::default();
        let now = Timestamp(0);
        let item = create_in_book(
            &mut book,
            "Wake Ryan",
            "in 1 minute",
            "text Ryan: wake up from sovra",
            None,
            now,
            0,
        )
        .expect("create");
        assert_eq!(
            item.cadence,
            ScheduleCadence::Once {
                delay_ms: MINUTE_MS
            }
        );
        assert_eq!(item.cadence_label(), "Once, in 1 minute");
        assert!(!item.cadence_label().contains("Every day"));
        assert_eq!(item.next_run_at, Timestamp(MINUTE_MS));

        let too_soon = tick(&mut book, Timestamp(MINUTE_MS - 1));
        assert_eq!(too_soon.fire, Vec::new());

        let due = tick(&mut book, Timestamp(MINUTE_MS));
        assert_eq!(due.fire.len(), 1);
        assert!(due.fire[0].prompt.contains("wake up from sovra"));
        let session = due.fire[0].session_id.clone();
        assert_eq!(book.schedules[0].status, ScheduleStatus::Paused);
        assert!(complete_run(
            &mut book,
            &session,
            true,
            None,
            Timestamp(MINUTE_MS + 1)
        ));

        let later = tick(&mut book, Timestamp(MINUTE_MS * 10));
        assert_eq!(later.fire, Vec::new(), "one-shot must not fire again");
        assert_eq!(later.skipped, Vec::<String>::new());
        assert_eq!(book.schedules[0].status, ScheduleStatus::Paused);
    }

    #[test]
    fn create_persists_a_timed_send_and_tick_carries_it() {
        let mut book = ScheduleBook::default();
        let send = ScheduledSend {
            channel: ChannelKind::WhatsApp,
            recipient: "+44 7347 529758".into(),
            text: "wake up from sovra".into(),
        };
        let item = create_in_book(
            &mut book,
            "Wake Ryan",
            "in 1 minute",
            "Send Ryan a WhatsApp: wake up from sovra",
            Some(send.clone()),
            Timestamp(0),
            0,
        )
        .expect("create");
        assert_eq!(item.send.as_ref(), Some(&send));
        let json = serde_json::to_string(&item).expect("json");
        let loaded: ScheduleItem = serde_json::from_str(&json).expect("reload");
        assert_eq!(loaded.send, Some(send.clone()));
        let missing: ScheduleItem = serde_json::from_str(
            r#"{"id":"sched-1","name":"Brief","cadence":{"daily":{"hour":9,"minute":0}},"prompt":"Say hello","status":"active","created_at":0,"next_run_at":0,"tz_offset_minutes":0}"#,
        )
        .expect("old json");
        assert_eq!(missing.send, None);

        let due = tick(&mut book, Timestamp(MINUTE_MS));
        assert_eq!(due.fire.len(), 1);
        assert_eq!(due.fire[0].send.as_ref(), Some(&send));
        assert_eq!(
            due.fire[0].send.as_ref().expect("send").text,
            "wake up from sovra"
        );
        assert!(
            create_in_book(
                &mut ScheduleBook::default(),
                "Empty send",
                "in 1 minute",
                "do it",
                Some(ScheduledSend {
                    channel: ChannelKind::WhatsApp,
                    recipient: String::new(),
                    text: "hi".into(),
                }),
                Timestamp(0),
                0,
            )
            .is_err()
        );
        let name_only = create_in_book(
            &mut ScheduleBook::default(),
            "Wake Ryan",
            "in 1 minute",
            "Send Ryan a WhatsApp: wake up from sovra",
            Some(ScheduledSend {
                channel: ChannelKind::WhatsApp,
                recipient: "Ryan".into(),
                text: "wake up from sovra".into(),
            }),
            Timestamp(0),
            0,
        );
        let err = name_only.expect_err("first name is not a timed-send recipient");
        assert!(err.to_string().contains("first name"), "{err}");

        let mut failed_book = ScheduleBook::default();
        let failed = create_in_book(
            &mut failed_book,
            "Wake Ryan",
            "in 1 minute",
            "Send the text",
            Some(ScheduledSend {
                channel: ChannelKind::WhatsApp,
                recipient: "+44 7347 529758".into(),
                text: "wake up from sovra".into(),
            }),
            Timestamp(0),
            0,
        )
        .expect("phone is concrete");
        let due_failed = tick(&mut failed_book, Timestamp(MINUTE_MS));
        assert_eq!(due_failed.fire.len(), 1);
        assert!(complete_run(
            &mut failed_book,
            &due_failed.fire[0].session_id,
            false,
            Some(
                "For a timed WhatsApp, use the international phone number starting with +.".into()
            ),
            Timestamp(MINUTE_MS + 1),
        ));
        let label = failed.last_result_label(&failed_book.runs, Timestamp(MINUTE_MS + 1));
        assert!(label.contains("Didn't finish"), "{label}");
        assert!(label.contains("international phone number"), "{label}");

        let mut soon_book = ScheduleBook::default();
        let soon = create_in_book(
            &mut soon_book,
            "Soon ping",
            "just once",
            "Say hello",
            None,
            Timestamp(5_000),
            0,
        )
        .expect("create soon");
        assert_eq!(soon.cadence_label(), "Once, soon");
        let first = tick(&mut soon_book, Timestamp(5_000));
        assert_eq!(first.fire.len(), 1);
        assert!(complete_run(
            &mut soon_book,
            &first.fire[0].session_id,
            true,
            None,
            Timestamp(5_001)
        ));
        let again = tick(&mut soon_book, Timestamp(5_000 + DAY_MS));
        assert_eq!(again.fire, Vec::new());
    }

    #[test]
    fn next_run_advances_from_the_supplied_timestamp() {
        let daily = ScheduleCadence::Daily { hour: 9, minute: 0 };
        assert_eq!(next_run_after(daily, 0, Timestamp(0)), Timestamp(NINE));
        assert_eq!(
            next_run_after(daily, 0, Timestamp(NINE)),
            Timestamp(NINE + DAY_MS)
        );
        let weekdays = ScheduleCadence::Weekdays { hour: 9, minute: 0 };
        // Friday 15:00 UTC 1970-01-02 → Monday 09:00.
        let friday_afternoon = Timestamp(DAY_MS + 15 * HOUR_MS);
        assert_eq!(
            next_run_after(weekdays, 0, friday_afternoon),
            Timestamp(4 * DAY_MS + NINE)
        );
        let fridays = ScheduleCadence::Weekly {
            weekday: Weekday::Friday,
            hour: 16,
            minute: 0,
        };
        assert_eq!(
            next_run_after(fridays, 0, Timestamp(0)),
            Timestamp(DAY_MS + 16 * HOUR_MS)
        );
        assert_eq!(
            next_run_after(ScheduleCadence::EveryHours { hours: 4 }, 0, Timestamp(0)),
            Timestamp(4 * HOUR_MS)
        );
    }

    #[test]
    fn pause_prevents_a_due_tick_from_firing() {
        let mut book = book_with(ScheduleItem {
            id: "sched-1".into(),
            name: "Brief".into(),
            cadence: ScheduleCadence::Daily { hour: 9, minute: 0 },
            prompt: "Say hello".into(),
            send: None,
            status: ScheduleStatus::Paused,
            created_at: Timestamp(0),
            next_run_at: Timestamp(NINE),
            tz_offset_minutes: 0,
        });
        let effect = tick(&mut book, Timestamp(NINE));
        assert_eq!(effect.fire, Vec::new());
        assert_eq!(effect.skipped, Vec::<String>::new());
    }

    #[test]
    fn due_active_fires_and_overlap_skips() {
        let mut book = book_with(ScheduleItem {
            id: "sched-1".into(),
            name: "Brief".into(),
            cadence: ScheduleCadence::Daily { hour: 9, minute: 0 },
            prompt: "Say hello".into(),
            send: None,
            status: ScheduleStatus::Active,
            created_at: Timestamp(0),
            next_run_at: Timestamp(NINE),
            tz_offset_minutes: 0,
        });
        let first = tick(&mut book, Timestamp(NINE));
        assert_eq!(first.fire.len(), 1);
        assert_eq!(first.fire[0].prompt, "[Schedule: Brief] Say hello");
        assert!(book.in_flight("sched-1"));
        book.schedules[0].next_run_at = Timestamp(NINE);
        let second = tick(&mut book, Timestamp(NINE));
        assert_eq!(second.fire, Vec::new());
        assert_eq!(second.skipped, ["sched-1"]);
        let now = Timestamp(NINE + DAY_MS);
        assert!(fail_stale_runs(&mut book, now));
        assert!(!book.in_flight("sched-1"));
        let next = book.schedules[0].next_run_at;
        let third = tick(&mut book, next);
        assert_eq!(third.fire.len(), 1);
        assert_eq!(third.skipped, Vec::<String>::new());
    }

    #[test]
    fn stats_and_success_rate_come_from_recorded_runs() {
        let mut book = book_with(ScheduleItem {
            id: "sched-1".into(),
            name: "Brief".into(),
            cadence: ScheduleCadence::Daily { hour: 9, minute: 0 },
            prompt: "Say hello".into(),
            send: None,
            status: ScheduleStatus::Active,
            created_at: Timestamp(0),
            next_run_at: Timestamp(NINE + DAY_MS),
            tz_offset_minutes: 0,
        });
        book.runs = vec![
            ScheduleRun {
                id: "r1".into(),
                schedule_id: "sched-1".into(),
                session_id: "s1".into(),
                started_at: Timestamp(NINE),
                finished_at: Some(Timestamp(NINE + 1)),
                outcome: Some(RunOutcome::Succeeded),
                error: None,
            },
            ScheduleRun {
                id: "r2".into(),
                schedule_id: "sched-1".into(),
                session_id: "s2".into(),
                started_at: Timestamp(NINE + HOUR_MS),
                finished_at: Some(Timestamp(NINE + HOUR_MS + 1)),
                outcome: Some(RunOutcome::Succeeded),
                error: None,
            },
            ScheduleRun {
                id: "r3".into(),
                schedule_id: "sched-1".into(),
                session_id: "s3".into(),
                started_at: Timestamp(NINE + 2 * HOUR_MS),
                finished_at: Some(Timestamp(NINE + 2 * HOUR_MS + 1)),
                outcome: Some(RunOutcome::Failed),
                error: None,
            },
        ];
        let stats = compute_stats(&book, Timestamp(NINE + 3 * HOUR_MS));
        assert_eq!(stats.total_schedules, 1);
        assert_eq!(stats.active_schedules, 1);
        assert_eq!(stats.runs_today, 3);
        assert_eq!(stats.success_rate_pct, 66);
        let marks = timeline_marks(&book, Timestamp(NINE + 3 * HOUR_MS), TimelineRange::Hours24);
        assert!(
            marks
                .iter()
                .any(|mark| mark.kind == TimelineMarkKind::Succeeded)
        );
        assert!(marks.iter().any(|mark| mark.kind == TimelineMarkKind::Next));
    }

    #[test]
    fn labels_stay_in_plain_words() {
        let item = ScheduleItem {
            id: "sched-1".into(),
            name: "Brief".into(),
            cadence: parse_cadence("every morning at 7:30"),
            prompt: "Say hello".into(),
            send: None,
            status: ScheduleStatus::Active,
            created_at: Timestamp(0),
            next_run_at: Timestamp(7 * HOUR_MS + 30 * MINUTE_MS + DAY_MS),
            tz_offset_minutes: 0,
        };
        assert_eq!(item.cadence_label(), "Every day at 7:30");
        assert_eq!(item.next_run_label(Timestamp(0)), "Tomorrow at 7:30");
        assert_eq!(item.last_result_label(&[], Timestamp(0)), "Never run");
        assert!(!item.cadence_label().contains("GST"));
    }
}
