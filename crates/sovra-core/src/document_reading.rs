//! Pure rules for reading attached documents cover to cover, in parallel.
//!
//! A whole-document read is split into small batches of sections. Each batch
//! is read independently by the model (map) and the document reader combines
//! the notes (reduce), the fan-out/gather shape ADK recommends for independent
//! work. Plans are fair across files and bounded per call, so a large library
//! advances every document and reports exactly where reading should resume
//! instead of silently skipping the rest.

/// Sections read together in one model call. Four 4,000-character sections
/// (about 4k tokens) fit comfortably in small local models' context windows.
pub const SECTIONS_PER_BATCH: usize = 4;
/// Batches one read may start, bounding model calls and cost per tool call.
/// Two parallel rounds keep a slow local model inside ADK's 300-second limit
/// for the specialist call that contains the read.
pub const MAX_BATCHES_PER_READ: usize = 8;
/// Batches read at the same time. Matches the turn's tool concurrency.
pub const MAX_PARALLEL_BATCHES: usize = 4;
/// The reply a batch reader gives when its sections say nothing relevant.
pub const NOTHING_RELEVANT: &str = "NONE";

/// Pictures or scanned pages read together in one model call. Gemini counts
/// 258 tokens per page image, so four stay small even for local vision models.
pub const PICTURES_PER_BATCH: usize = 4;
/// Recordings are read one per call: each can be long and large.
pub const RECORDINGS_PER_BATCH: usize = 1;

/// Where one file's reading starts. A unit is a text section, or a picture,
/// page image or recording for files read as media.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadRequest {
    /// Units in the file.
    pub total_sections: usize,
    /// 1-based unit to start from.
    pub first_section: usize,
    /// Units read together in one model call; `0` is treated as `1`.
    pub per_batch: usize,
}

/// A contiguous run of sections in one file, read by one model call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    /// Index into the requests the plan was made from.
    pub file: usize,
    /// First section, 1-based and inclusive.
    pub first: usize,
    /// Last section, inclusive.
    pub last: usize,
}

/// Batches to read now, in file then section order, and where each file
/// continues. `None` means the file has been read to its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingPlan {
    pub batches: Vec<Batch>,
    pub next_section: Vec<Option<usize>>,
}

/// Splits the requested reads into at most `max_batches` batches. Files take
/// turns, so one long file cannot starve the others. A start past a file's
/// end (or `0`) reads nothing for that file.
pub fn plan_reading(files: &[ReadRequest], max_batches: usize) -> ReadingPlan {
    let mut cursors: Vec<usize> = files
        .iter()
        .map(|file| {
            if file.first_section == 0 {
                file.total_sections + 1
            } else {
                file.first_section
            }
        })
        .collect();
    let mut batches = Vec::new();
    'rounds: loop {
        let mut progressed = false;
        for (index, file) in files.iter().enumerate() {
            if batches.len() == max_batches {
                break 'rounds;
            }
            let first = cursors[index];
            if first > file.total_sections {
                continue;
            }
            let size = file.per_batch.max(1);
            let last = first.saturating_add(size - 1).min(file.total_sections);
            batches.push(Batch {
                file: index,
                first,
                last,
            });
            cursors[index] = last + 1;
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    batches.sort_unstable_by_key(|batch| (batch.file, batch.first));
    let next_section = files
        .iter()
        .zip(cursors)
        .map(|(file, cursor)| (cursor <= file.total_sections).then_some(cursor))
        .collect();
    ReadingPlan {
        batches,
        next_section,
    }
}

/// Combines per-file ranked hits into one list, taking each file's next best
/// hit in turn. Rank within a file is kept; no file dominates the first page.
/// Items are `(file index, section)`.
pub fn interleave_ranked(per_file: &[Vec<usize>], limit: usize) -> Vec<(usize, usize)> {
    let mut merged = Vec::new();
    let mut rank = 0;
    while merged.len() < limit {
        let mut found = false;
        for (file, hits) in per_file.iter().enumerate() {
            if let Some(section) = hits.get(rank) {
                found = true;
                if merged.len() < limit {
                    merged.push((file, *section));
                }
            }
        }
        if !found {
            break;
        }
        rank += 1;
    }
    merged
}

/// The useful part of a batch reader's reply, or `None` when it found
/// nothing relevant. Tolerates models that decorate the sentinel.
pub fn batch_finding(reply: &str) -> Option<&str> {
    let reply = reply.trim();
    let bare = reply.trim_matches(|ch: char| !ch.is_alphanumeric());
    (!reply.is_empty() && !bare.eq_ignore_ascii_case(NOTHING_RELEVANT)).then_some(reply)
}

/// "3" or "3–6", for citations people read.
pub fn span_label(first: usize, last: usize) -> String {
    if first == last {
        first.to_string()
    } else {
        format!("{first}–{last}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(total_sections: usize, first_section: usize) -> ReadRequest {
        ReadRequest {
            total_sections,
            first_section,
            per_batch: 4,
        }
    }

    #[test]
    fn a_short_file_is_read_whole_in_one_plan() {
        let plan = plan_reading(&[request(9, 1)], 12);
        assert_eq!(
            plan.batches,
            [
                Batch {
                    file: 0,
                    first: 1,
                    last: 4
                },
                Batch {
                    file: 0,
                    first: 5,
                    last: 8
                },
                Batch {
                    file: 0,
                    first: 9,
                    last: 9
                },
            ]
        );
        assert_eq!(plan.next_section, [None]);
    }

    #[test]
    fn files_take_turns_so_a_long_file_cannot_starve_the_others() {
        let plan = plan_reading(&[request(400, 1), request(3, 1), request(10, 1)], 5);
        assert_eq!(plan.batches.len(), 5);
        let per_file = |file| plan.batches.iter().filter(|b| b.file == file).count();
        assert_eq!((per_file(0), per_file(1), per_file(2)), (2, 1, 2));
        assert_eq!(plan.next_section, [Some(9), None, Some(9)]);
        // Results come back in reading order for each file.
        assert!(
            plan.batches
                .windows(2)
                .all(|w| (w[0].file, w[0].first) < (w[1].file, w[1].first))
        );
    }

    #[test]
    fn reading_resumes_where_the_previous_plan_stopped() {
        let first = plan_reading(&[request(30, 1)], 3);
        let resume = first.next_section[0].expect("more to read");
        assert_eq!(resume, 13);
        let second = plan_reading(&[request(30, resume)], 12);
        assert_eq!(second.batches[0].first, 13);
        assert_eq!(second.batches.last().map(|b| b.last), Some(30));
        assert_eq!(second.next_section, [None]);
    }

    #[test]
    fn empty_files_and_out_of_range_starts_read_nothing() {
        let plan = plan_reading(&[request(0, 1), request(5, 6), request(5, 0)], 12);
        assert_eq!(plan.batches.len(), 0);
        assert_eq!(plan.next_section, [None, None, None]);
        assert_eq!(plan_reading(&[], 12).batches.len(), 0);
        assert_eq!(plan_reading(&[request(5, 1)], 0).batches.len(), 0);
        // A zero batch size still makes progress.
        let zero = ReadRequest {
            per_batch: 0,
            ..request(2, 1)
        };
        assert_eq!(plan_reading(&[zero], 12).batches.len(), 2);
    }

    #[test]
    fn each_file_uses_its_own_batch_size() {
        let recording = ReadRequest {
            per_batch: RECORDINGS_PER_BATCH,
            ..request(2, 1)
        };
        let pictures = ReadRequest {
            per_batch: PICTURES_PER_BATCH,
            ..request(6, 1)
        };
        let plan = plan_reading(&[recording, pictures], 12);
        let spans: Vec<_> = plan
            .batches
            .iter()
            .map(|b| (b.file, b.first, b.last))
            .collect();
        assert_eq!(spans, [(0, 1, 1), (0, 2, 2), (1, 1, 4), (1, 5, 6)]);
    }

    #[test]
    fn ranked_hits_interleave_by_rank_and_respect_the_limit() {
        let merged = interleave_ranked(&[vec![7, 2, 9], vec![], vec![4, 1]], 4);
        assert_eq!(merged, [(0, 7), (2, 4), (0, 2), (2, 1)]);
        assert_eq!(interleave_ranked(&[vec![1]], 10), [(0, 1)]);
        assert_eq!(interleave_ranked(&[vec![1]], 0).len(), 0);
        assert_eq!(interleave_ranked(&[], 3).len(), 0);
    }

    #[test]
    fn the_nothing_relevant_sentinel_is_recognised_in_common_shapes() {
        for reply in ["NONE", " none ", "**NONE**", "None.", "", "  "] {
            assert_eq!(batch_finding(reply), None, "{reply:?}");
        }
        assert_eq!(
            batch_finding("  Total due: 93 (lines 4–5)\n"),
            Some("Total due: 93 (lines 4–5)")
        );
        assert!(batch_finding("None of the clauses mention refunds.").is_some());
    }

    #[test]
    fn spans_read_naturally() {
        assert_eq!(span_label(3, 3), "3");
        assert_eq!(span_label(3, 6), "3–6");
    }
}
