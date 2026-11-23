//! A saved answer keeps the wording that was read.
//!
//! File history is an observation log, not a byte-for-byte copy of the file, so
//! the answer stores the extracted quote. The date is when that version was
//! read. A later search is free to return newer text.

use super::{IndexError, IndexPassage, PassageRef};

/// The wording, file, line range, and read time captured from one passage.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DatedAnswer {
    reference: PassageRef,
    file: String,
    wording: String,
    start_line: u32,
    end_line: u32,
    /// Unix seconds when this version was read. Never a filesystem modification time.
    read_on: u64,
}

/// What reopening a saved answer shows. It is copied from the save, not from the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatedReading {
    pub file: String,
    pub wording: String,
    pub start_line: u32,
    pub end_line: u32,
    pub read_on: u64,
}

impl DatedAnswer {
    /// Copy the passage that was read. `modified_at` is ignored on purpose.
    pub fn save(passage: &IndexPassage) -> Result<Self, IndexError> {
        super::validate_relative_path(&passage.path)?;
        if passage.text.trim().is_empty() || passage.text.len() > 32_768 {
            return Err(IndexError(
                "Save a passage that has wording, up to 32,768 bytes.".into(),
            ));
        }
        if passage.end_line < passage.start_line {
            return Err(IndexError("This passage has no line range to save.".into()));
        }
        if passage.observed_at == 0 || i64::try_from(passage.observed_at).is_err() {
            return Err(IndexError(
                "This passage has no time for when it was read.".into(),
            ));
        }
        Ok(Self {
            reference: passage.reference.clone(),
            file: passage.path.clone(),
            wording: passage.text.clone(),
            start_line: passage.start_line,
            end_line: passage.end_line,
            read_on: passage.observed_at,
        })
    }

    pub fn reference(&self) -> &PassageRef {
        &self.reference
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    pub fn wording(&self) -> &str {
        &self.wording
    }

    pub fn start_line(&self) -> u32 {
        self.start_line
    }

    pub fn end_line(&self) -> u32 {
        self.end_line
    }

    pub fn read_on(&self) -> u64 {
        self.read_on
    }

    /// The saved file, passage, and read date. The current file is not an input.
    pub fn reread(&self) -> DatedReading {
        DatedReading {
            file: self.file.clone(),
            wording: self.wording.clone(),
            start_line: self.start_line,
            end_line: self.end_line,
            read_on: self.read_on,
        }
    }

    /// Plain words for the screen. The date is the UTC day that version was read.
    pub fn screen_words(&self) -> String {
        format!(
            "File: {file}. Passage: lines {start}–{end}. {wording} Read on {date}. This date is when this version was read. A later edit of the file does not replace this wording.",
            file = self.file,
            start = self.start_line,
            end = self.end_line,
            wording = self.wording,
            date = read_on_words(self.read_on),
        )
    }

    /// The read date in plain words, including what that date means.
    pub fn read_date_sentence(&self) -> String {
        format!(
            "Read on {}. This date is when this version was read.",
            read_on_words(self.read_on)
        )
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// UTC calendar day for a Unix timestamp, in plain English (`14 November 2023`).
fn read_on_words(unix_seconds: u64) -> String {
    let days = i64::try_from(unix_seconds / 86_400).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(days);
    let name = month
        .checked_sub(1)
        .and_then(|index| MONTHS.get(index as usize))
        .copied()
        .unwrap_or("January");
    format!("{day} {name} {year}")
}

/// Howard Hinnant's `civil_from_days`. `z` is days since 1970-01-01.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097).cast_unsigned();
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::try_from(yoe).unwrap_or(i64::MAX) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(text: &str, observed_at: u64, modified_at: Option<u64>) -> IndexPassage {
        IndexPassage {
            reference: PassageRef {
                root_id: "folder".into(),
                file_id: "file".into(),
                version_id: "version-read".into(),
                ordinal: 0,
            },
            path: "keeper-note.txt".into(),
            text: text.into(),
            start_line: 3,
            end_line: 4,
            mime_type: "text/plain".into(),
            note: None,
            observed_at,
            modified_at,
        }
    }

    #[test]
    fn saved_wording_stays_on_the_version_that_was_read() {
        let read = passage(
            "The allotment levy is forty-two dinars.",
            1_700_000_000,
            Some(1_600_000_000),
        );
        let saved = DatedAnswer::save(&read).expect("passage can be saved");
        let mut changed = read.clone();
        changed.text = "The allotment levy is ninety-nine dinars.".into();
        changed.observed_at = 1_800_000_000;
        changed.modified_at = Some(1_800_000_000);
        changed.reference.version_id = "version-later".into();

        let reading = saved.reread();
        assert_eq!(reading.file, "keeper-note.txt");
        assert_eq!(reading.wording, "The allotment levy is forty-two dinars.");
        assert_eq!(reading.start_line, 3);
        assert_eq!(reading.end_line, 4);
        assert_eq!(reading.read_on, 1_700_000_000);
        assert_ne!(reading.read_on, changed.modified_at.unwrap());
        assert_ne!(reading.read_on, read.modified_at.unwrap());
        assert_eq!(read_on_words(reading.read_on), "14 November 2023");
        assert_eq!(read_on_words(1_600_000_000), "13 September 2020");
        assert_eq!(read_on_words(0), "1 January 1970");
        let words = saved.screen_words();
        assert!(words.contains("keeper-note.txt"));
        assert!(words.contains("lines 3–4"));
        assert!(words.contains("forty-two dinars"));
        assert!(words.contains("14 November 2023"));
        assert!(words.contains("This date is when this version was read."));
        assert!(words.contains("A later edit of the file does not replace this wording."));
        assert!(!words.to_ascii_lowercase().contains("market"));
        assert!(!words.to_ascii_lowercase().contains("licen"));
        assert!(!words.to_ascii_lowercase().contains("industry"));
        assert_eq!(saved.reference().version_id, "version-read");
        assert_ne!(changed.text, reading.wording);
    }

    #[test]
    fn a_passage_without_a_read_time_is_not_saved_from_the_file_clock() {
        let missing = passage("A sentence.", 0, Some(1_700_000_000));
        assert!(DatedAnswer::save(&missing).is_err());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedAnswerPage {
    pub answers: Vec<DatedAnswer>,
    pub next_before: Option<u64>,
}
