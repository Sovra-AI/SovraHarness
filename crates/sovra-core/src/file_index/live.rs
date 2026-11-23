//! What a person sees while their folders are being read: plain sentences, an honest
//! progress fraction and a time estimate that only appears once it can be trusted.
//!
//! Rules (sources in `docs/file-ontology.md` → *Live progress*):
//! - Indeterminate "Finding your files…" while the walk is still counting; determinate
//!   "1,204 of 5,000 files processed" once the total is known (Apple HIG, NN/g).
//! - Time left comes from a smoothed rate, is hidden until it settles, and is rounded
//!   generously ("About 3 minutes left", "Almost done").
//! - Problems are one calm summary line, never a stream of alerts.
//! - "What I've found so far" turns waiting into seeing the system learn.

use super::IndexProgress;
use super::ontology::{FileKind, LifeArea, group_digits};

/// Exponentially smoothed settle rate in integer milli-files per minute, so the
/// estimate is deterministic and free of float casts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateEstimator {
    last: Option<(u64, u64)>,
    smoothed_milli_per_minute: Option<u64>,
}

/// Samples closer together than this are too noisy to update the rate.
const MIN_SAMPLE_MS: u64 = 500;
/// No estimate before this much time and work: early rates are dominated by caches.
const WARM_UP_MS: u64 = 3_000;
const WARM_UP_FILES: u64 = 16;

impl RateEstimator {
    /// Record that `handled` files had settled at `now_ms` since the scan began.
    pub fn observe(&mut self, now_ms: u64, handled: u64) {
        let Some((then_ms, then_handled)) = self.last else {
            self.last = Some((now_ms, handled));
            return;
        };
        let span = now_ms.saturating_sub(then_ms);
        if span < MIN_SAMPLE_MS {
            return;
        }
        let instant = handled
            .saturating_sub(then_handled)
            .saturating_mul(60_000_000)
            / span;
        // α = 0.3: responsive to a slow folder of videos, steady across bursts of tiny files.
        self.smoothed_milli_per_minute = Some(match self.smoothed_milli_per_minute {
            None => instant,
            Some(previous) => (instant * 3 + previous * 7) / 10,
        });
        self.last = Some((now_ms, handled));
    }

    /// Files per minute, or 0 while the estimate would be a guess.
    pub fn files_per_minute(&self, now_ms: u64, handled: u64) -> u64 {
        if now_ms < WARM_UP_MS || handled < WARM_UP_FILES {
            return 0;
        }
        self.smoothed_milli_per_minute
            .map_or(0, |milli| milli / 1000)
    }
}

/// In-flight work is kept separate from outcomes that have been committed to the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Discovering,
    Checking,
    Reading,
    Extracting,
    Saving,
    CheckingRemoved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanActivity {
    pub phase: ScanPhase,
    pub path: Option<String>,
    pub seconds_without_change: u64,
    pub passage: Option<u64>,
}

fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds / 60 % 60),
    }
}

/// Everything the live scan card shows, already worded.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanPicture {
    pub headline: &'static str,
    /// Percent (0–100) for a determinate bar; `None` while still counting.
    pub percent: Option<f32>,
    pub count_line: String,
    pub time_left: Option<String>,
    pub throughput: String,
    pub quiet: Option<String>,
    /// Full path relative to the connected folder.
    pub now_reading: Option<String>,
    /// Kinds found so far, most common first.
    pub found: Vec<(FileKind, u64)>,
    /// Life areas suggested so far, most common first.
    pub areas: Vec<(LifeArea, u64)>,
    pub problems: Option<String>,
}

impl ScanPicture {
    /// Picture of a scan in progress.
    pub fn of(progress: &IndexProgress) -> Self {
        let remaining = progress.discovered.saturating_sub(progress.handled);
        let activity = progress.activity.as_ref();
        let headline = match activity.map(|a| a.phase) {
            Some(ScanPhase::Checking) => "Checking for changes",
            Some(ScanPhase::Extracting) => "Extracting searchable text",
            Some(ScanPhase::Reading) => "Reading file contents",
            Some(ScanPhase::Saving) => "Saving files to the search index",
            Some(ScanPhase::CheckingRemoved) => "Checking for removed files",
            _ if !progress.walk_complete => "Finding your files…",
            _ if remaining == 0 => "Finishing up…",
            _ => "Processing files",
        };
        let percent = progress.walk_complete.then(|| {
            let permille = progress
                .handled
                .min(progress.discovered)
                .saturating_mul(1000)
                .checked_div(progress.discovered)
                .unwrap_or(1000);
            f32::from(u16::try_from(permille).unwrap_or(1000)) / 10.0
        });
        let count_line = if progress.walk_complete {
            format!(
                "{} of {} processed",
                group_digits(progress.handled.min(progress.discovered)),
                files(progress.discovered)
            )
        } else {
            format!(
                "{} found · {} processed",
                files(progress.discovered),
                group_digits(progress.handled)
            )
        };
        let quiet = activity
            .filter(|a| a.seconds_without_change >= 10)
            .map(|a| {
                format!(
                    "No new scan progress reported for {}.",
                    duration(a.seconds_without_change)
                )
            });
        let time_left = (quiet.is_none() && progress.walk_complete && remaining > 0)
            .then(|| time_left(remaining, progress.files_per_minute))
            .flatten();
        Self {
            headline,
            percent,
            count_line,
            time_left,
            now_reading: activity.and_then(|a| {
                a.path.as_ref().map(|path| match a.passage {
                    None => path.clone(),
                    Some(number) => format!("{path} · passage {}", group_digits(number)),
                })
            }),
            throughput: format!(
                "{} elapsed · {} MiB read · {} indexed · {} reused · {} skipped",
                duration(progress.elapsed_ms / 1000),
                group_digits(progress.bytes_read / (1024 * 1024)),
                group_digits(progress.indexed),
                group_digits(progress.reused),
                group_digits(progress.skipped)
            ),
            quiet,
            found: ranked(&progress.kinds),
            areas: ranked(&progress.areas),
            problems: problems(progress),
        }
    }
}

/// "Found 320 photos, 85 documents and 12 spreadsheets." — for finished scans and
/// screen readers. `None` when nothing was classified.
pub fn found_sentence(progress: &IndexProgress) -> Option<String> {
    let top: Vec<String> = ranked(&progress.kinds)
        .into_iter()
        .take(3)
        .map(|(kind, count)| kind.count_phrase(count))
        .collect();
    let others = progress.kinds.len().saturating_sub(top.len());
    let list = match (top.as_slice(), others) {
        ([], _) => return None,
        ([only], 0) => only.clone(),
        (items, 0) => {
            let (last, rest) = items.split_last().expect("non-empty list");
            format!("{} and {last}", rest.join(", "))
        }
        (items, _) => format!("{} and more", items.join(", ")),
    };
    Some(format!("Found {list}."))
}

fn files(count: u64) -> String {
    format!(
        "{} {}",
        group_digits(count),
        if count == 1 { "file" } else { "files" }
    )
}

fn problems(progress: &IndexProgress) -> Option<String> {
    match progress.failed {
        0 => None,
        1 => Some("1 file couldn't be read.".into()),
        n => Some(format!("{} files couldn't be read.", group_digits(n))),
    }
}

fn time_left(remaining: u64, files_per_minute: u64) -> Option<String> {
    if files_per_minute == 0 {
        return None;
    }
    let seconds = remaining.saturating_mul(60).div_ceil(files_per_minute);
    Some(match seconds {
        0..=15 => "Almost done".into(),
        16..=59 => "Less than a minute left".into(),
        60..=119 => "About a minute left".into(),
        120..=3_299 => format!("About {} minutes left", seconds.div_ceil(60)),
        3_300..=5_399 => "About an hour left".into(),
        _ => format!("About {} hours left", (seconds + 1_800) / 3_600),
    })
}

fn ranked<K: Copy + Ord>(counts: &std::collections::BTreeMap<K, u64>) -> Vec<(K, u64)> {
    let mut ranked: Vec<(K, u64)> = counts
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(k, n)| (*k, *n))
        .collect();
    // Stable sort keeps the concept-scheme order for equal counts.
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress() -> IndexProgress {
        IndexProgress::default()
    }

    #[test]
    fn counting_phase_is_indeterminate_and_says_so() {
        let mut p = progress();
        p.discovered = 3_214;
        let picture = ScanPicture::of(&p);
        assert_eq!(picture.headline, "Finding your files…");
        assert_eq!(picture.percent, None);
        assert_eq!(picture.count_line, "3,214 files found · 0 processed");
        assert_eq!(picture.time_left, None, "no estimate without a total");
    }

    #[test]
    fn reading_phase_is_determinate_with_rounded_time_left() {
        let mut p = progress();
        p.walk_complete = true;
        p.discovered = 5_000;
        p.handled = 1_204;
        p.files_per_minute = 1_000;
        p.current = Some("Photos/Spain/IMG_0001.jpg".into());
        let picture = ScanPicture::of(&p);
        assert_eq!(picture.headline, "Processing files");
        assert_eq!(picture.percent, Some(24.0));
        assert_eq!(picture.count_line, "1,204 of 5,000 files processed");
        assert_eq!(picture.time_left.as_deref(), Some("About 4 minutes left"));
        assert_eq!(
            picture.now_reading, None,
            "a settled file is no longer active"
        );
    }

    #[test]
    fn slow_save_exposes_active_path_and_suppresses_stale_estimate() {
        let mut p = progress();
        p.discovered = 200;
        p.walk_complete = true;
        p.handled = 10;
        p.files_per_minute = 600;
        p.elapsed_ms = 65000;
        p.current = Some("finished.txt".into());
        p.activity = Some(ScanActivity {
            phase: ScanPhase::Saving,
            path: Some("photos/large.svg".into()),
            seconds_without_change: 32,
            passage: Some(71),
        });
        let picture = ScanPicture::of(&p);
        assert_eq!(picture.headline, "Saving files to the search index");
        assert_eq!(
            picture.now_reading.as_deref(),
            Some("photos/large.svg · passage 71")
        );
        assert!(picture.quiet.as_deref().unwrap().contains("32s"));
        assert!(picture.throughput.contains("1m 5s elapsed"));
        assert_eq!(picture.time_left, None);
    }

    #[test]
    fn scan_headline_names_the_work_in_progress() {
        let mut p = progress();
        for (phase, label) in [
            (ScanPhase::Checking, "Checking for changes"),
            (ScanPhase::Reading, "Reading file contents"),
            (ScanPhase::Extracting, "Extracting searchable text"),
        ] {
            p.activity = Some(ScanActivity {
                phase,
                path: Some("notes/agenda.pdf".into()),
                seconds_without_change: 0,
                passage: None,
            });
            let picture = ScanPicture::of(&p);
            assert_eq!(picture.headline, label);
            assert_eq!(picture.now_reading.as_deref(), Some("notes/agenda.pdf"));
        }
    }

    #[test]
    fn time_left_is_hidden_until_rate_is_known_and_softens_near_the_end() {
        assert_eq!(time_left(100, 0), None);
        assert_eq!(time_left(10, 600).as_deref(), Some("Almost done"));
        assert_eq!(
            time_left(50, 100).as_deref(),
            Some("Less than a minute left")
        );
        assert_eq!(time_left(100, 100).as_deref(), Some("About a minute left"));
        assert_eq!(time_left(6_000, 100).as_deref(), Some("About an hour left"));
        assert_eq!(
            time_left(30_000, 100).as_deref(),
            Some("About 5 hours left")
        );
    }

    #[test]
    fn empty_and_finished_folders_never_divide_by_zero() {
        let mut p = progress();
        p.walk_complete = true;
        let picture = ScanPicture::of(&p);
        assert_eq!(picture.percent, Some(100.0));
        assert_eq!(picture.headline, "Finishing up…");
        assert_eq!(picture.count_line, "0 of 0 files processed");
    }

    #[test]
    fn problems_are_one_calm_line() {
        let mut p = progress();
        assert_eq!(ScanPicture::of(&p).problems, None);
        p.failed = 1;
        assert_eq!(
            ScanPicture::of(&p).problems.as_deref(),
            Some("1 file couldn't be read.")
        );
        p.failed = 1_200;
        assert_eq!(
            ScanPicture::of(&p).problems.as_deref(),
            Some("1,200 files couldn't be read.")
        );
    }

    #[test]
    fn found_lists_are_ranked_and_read_as_a_sentence() {
        let mut p = progress();
        assert_eq!(found_sentence(&p), None);
        p.kinds.insert(FileKind::Photo, 320);
        assert_eq!(found_sentence(&p).as_deref(), Some("Found 320 photos."));
        p.kinds.insert(FileKind::Document, 85);
        p.kinds.insert(FileKind::Spreadsheet, 12);
        assert_eq!(
            found_sentence(&p).as_deref(),
            Some("Found 320 photos, 85 documents and 12 spreadsheets.")
        );
        p.kinds.insert(FileKind::Video, 1);
        assert_eq!(
            found_sentence(&p).as_deref(),
            Some("Found 320 photos, 85 documents, 12 spreadsheets and more.")
        );
        let picture = ScanPicture::of(&p);
        assert_eq!(picture.found[0], (FileKind::Photo, 320));
        assert_eq!(picture.found.last(), Some(&(FileKind::Video, 1)));
    }

    #[test]
    fn rate_estimate_warms_up_then_smooths_bursts() {
        let mut rate = RateEstimator::default();
        rate.observe(0, 0);
        rate.observe(1_000, 10);
        assert_eq!(rate.files_per_minute(1_000, 10), 0, "still warming up");
        rate.observe(4_000, 40);
        // 600/min then 600/min → 600.
        assert_eq!(rate.files_per_minute(4_000, 40), 600);
        rate.observe(4_100, 400);
        assert_eq!(
            rate.files_per_minute(4_100, 400),
            600,
            "samples under 500ms are ignored"
        );
        rate.observe(5_000, 400);
        // Burst of 360 files in 1s (21,600/min) is damped: 0.3*21600 + 0.7*600 = 6,900.
        assert_eq!(rate.files_per_minute(5_000, 400), 6_900);
    }
}
