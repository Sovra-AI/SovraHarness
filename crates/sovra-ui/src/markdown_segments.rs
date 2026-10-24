//! Splits a streamed Markdown reply into segments that render as separate,
//! virtualized list rows. A reply rendered as one row is laid out in full on every
//! streamed update, so frame cost grew with its length; per-segment rows keep the
//! cost proportional to what is on screen, and only the last segment changes.
//!
//! Splits happen only where Markdown is self-contained: at a non-indented line that
//! follows a blank line, outside fenced code. Scanning is incremental — each call
//! looks only at new bytes. A split is decided as soon as the byte that starts the
//! new segment arrives, so it never lands inside text that is already on screen.
//!
//! A single paragraph can be very long (a wall of prose with no blank lines). Such a
//! plain paragraph is also split softly between sentences once it passes
//! [`SOFT_SPLIT_BYTES`], never inside inline code or emphasis; the continuation is
//! drawn without a paragraph gap.

/// Segments grow to at least this many bytes before the next safe split.
pub(crate) const SEGMENT_BYTES: usize = 1536;

/// A plain paragraph longer than this is split between sentences.
pub(crate) const SOFT_SPLIT_BYTES: usize = 3072;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Segmenter {
    /// Byte offsets where segments after the first begin.
    boundaries: Vec<usize>,
    /// For each boundary: whether it splits a paragraph between sentences.
    soft: Vec<bool>,
    /// Inline markers on the current line when it is plain prose (eligible for
    /// sentence splits); `None` for headings, lists, quotes, tables and code.
    plain: Option<Markers>,
    /// How far into the current line sentence splits have been considered.
    line_scanned: usize,
    /// Start of the line being scanned (all earlier lines are complete).
    scanned: usize,
    /// Whether the split decision for the line at `scanned` has been made.
    decided: bool,
    /// The open code fence: its character and length.
    fence: Option<(char, usize)>,
    previous_blank: bool,
    segment_start: usize,
    /// A link reference definition (`[label]: url`) can resolve links anywhere in
    /// the reply, so such replies are kept whole.
    keep_whole: bool,
}

impl Segmenter {
    /// Scans the bytes of `text` added since the last call. `text` must extend the
    /// text previously scanned. New boundaries never precede the previous length.
    pub(crate) fn advance(&mut self, text: &str) {
        loop {
            let line_start = self.scanned;
            let Some(rest) = text.get(line_start..) else {
                return;
            };
            if !self.decided {
                // The rule needs only the line's first byte: a non-blank, non-indented
                // line after a blank one. Deciding now keeps splits ahead of the text
                // that has already been rendered.
                let Some(first) = rest.chars().next() else {
                    return;
                };
                if !self.keep_whole
                    && !matches!(first, '\n' | '\r' | ' ' | '\t')
                    && self.previous_blank
                    && self.fence.is_none()
                    && line_start - self.segment_start >= SEGMENT_BYTES
                {
                    self.boundaries.push(line_start);
                    self.soft.push(false);
                    self.segment_start = line_start;
                }
                self.plain = first.is_alphabetic().then(Markers::default);
                self.line_scanned = line_start;
                self.decided = true;
            }
            let line_end = rest.find('\n').map(|offset| line_start + offset);
            if self.plain.is_some() && self.fence.is_none() && !self.keep_whole {
                self.split_sentences(text, line_end.unwrap_or(text.len()));
            }
            let Some(offset) = line_end.map(|end| end - line_start) else {
                return;
            };
            let line = &rest[..offset];
            if self.fence.is_none() && is_reference_definition(line) {
                self.keep_whole = true;
                self.boundaries.clear();
                self.soft.clear();
            }
            self.track_fence(line.trim_start());
            self.previous_blank = line.trim().is_empty();
            self.scanned = line_start + offset + 1;
            self.decided = false;
        }
    }

    /// A segmenter that never splits.
    pub(crate) fn whole() -> Self {
        Self {
            keep_whole: true,
            ..Self::default()
        }
    }

    pub(crate) fn boundaries(&self) -> &[usize] {
        &self.boundaries
    }

    /// Whether boundary `index` splits a paragraph between sentences.
    pub(crate) fn is_soft(&self, index: usize) -> bool {
        self.soft.get(index).copied().unwrap_or(false)
    }

    /// Considers new bytes of the current plain line, up to `end`, for a split at
    /// the start of a sentence (`. `, `! ` or `? ` followed by a capital letter).
    fn split_sentences(&mut self, text: &str, end: usize) {
        let from = self.line_scanned;
        let Some(mut markers) = self.plain else {
            return;
        };
        for (offset, c) in text[from..end].char_indices() {
            let at = from + offset;
            markers.count(c);
            if c.is_uppercase()
                && at - self.segment_start >= SOFT_SPLIT_BYTES
                && markers.balanced()
                && [". ", "! ", "? "]
                    .iter()
                    .any(|end| text[..at].ends_with(end))
            {
                self.boundaries.push(at);
                self.soft.push(true);
                self.segment_start = at;
            }
        }
        self.plain = Some(markers);
        self.line_scanned = end;
    }

    /// Whether the reply must render as a single segment.
    pub(crate) fn keep_whole(&self) -> bool {
        self.keep_whole
    }

    fn track_fence(&mut self, line: &str) {
        let Some(marker) = line.chars().next().filter(|c| matches!(c, '`' | '~')) else {
            return;
        };
        let length = line.chars().take_while(|c| *c == marker).count();
        if length < 3 {
            return;
        }
        match self.fence {
            None => self.fence = Some((marker, length)),
            Some((open, open_length))
                if open == marker
                    && length >= open_length
                    && line[length * marker.len_utf8()..].trim().is_empty() =>
            {
                self.fence = None;
            }
            Some(_) => {}
        }
    }
}

/// Inline code and emphasis markers seen on a line; a sentence split needs each
/// of them balanced so it never lands inside `code`, *emphasis* or **strong** text.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Markers {
    backticks: usize,
    stars: usize,
    underscores: usize,
}

impl Markers {
    fn count(&mut self, c: char) {
        match c {
            '`' => self.backticks += 1,
            '*' => self.stars += 1,
            '_' => self.underscores += 1,
            _ => {}
        }
    }

    fn balanced(self) -> bool {
        self.backticks.is_multiple_of(2)
            && self.stars.is_multiple_of(2)
            && self.underscores.is_multiple_of(2)
    }
}

/// `[label]: destination`, indented at most three spaces (`CommonMark` 4.7).
fn is_reference_definition(line: &str) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let Some(rest) = line[indent..].strip_prefix('[').filter(|_| indent <= 3) else {
        return false;
    };
    rest.find("]:").is_some_and(|end| {
        end > 0 && !rest[..end].contains(']') && !rest[end + 2..].trim().is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments(text: &str) -> Vec<&str> {
        let mut segmenter = Segmenter::default();
        segmenter.advance(text);
        let mut starts = vec![0];
        starts.extend_from_slice(segmenter.boundaries());
        starts.push(text.len());
        starts.windows(2).map(|w| &text[w[0]..w[1]]).collect()
    }

    #[test]
    fn short_replies_stay_whole() {
        assert_eq!(segments("Hello\n\nWorld\n").len(), 1);
    }

    #[test]
    fn long_replies_split_at_paragraphs_and_rejoin_exactly() {
        let paragraph = "word ".repeat(100);
        let text = format!("{paragraph}\n\n").repeat(12);
        let parts = segments(&text);
        assert!(parts.len() > 2);
        assert_eq!(parts.concat(), text);
        for part in &parts[1..] {
            assert!(part.starts_with("word"), "splits land on block starts");
        }
    }

    #[test]
    fn never_splits_inside_fenced_code_or_indented_continuations() {
        let filler = "x".repeat(SEGMENT_BYTES);
        let text = format!(
            "{filler}\n\n```rust\nfn a() {{}}\n\nfn b() {{}}\n```\n\n- item\n\n  continued\n\n~~~~\n~~~\n\nstill code\n~~~~\n"
        );
        for part in segments(&text) {
            let fences = part.lines().filter(|l| l.starts_with("```")).count();
            assert!(fences % 2 == 0, "a fence was split: {part:?}");
            assert!(!part.starts_with("  continued"));
            assert!(
                !part.starts_with("still code"),
                "inner ~~~ did not close ~~~~"
            );
        }
    }

    #[test]
    fn replies_with_link_reference_definitions_stay_whole() {
        let paragraph = format!("{} [docs][d]\n\n", "word ".repeat(100));
        let text = format!("{}[d]: https://example.com\n", paragraph.repeat(12));
        let mut segmenter = Segmenter::default();
        segmenter.advance(&text);
        assert!(segmenter.keep_whole());
        assert_eq!(segmenter.boundaries(), &[] as &[usize]);
        assert!(!is_reference_definition("[not a definition] here"));
        assert!(!is_reference_definition("    [code]: indented four"));
    }

    #[test]
    fn streamed_splits_never_land_inside_text_already_scanned() {
        // Regression: splits were decided only once a line was complete, after its
        // partial text had already been rendered, and the UI sliced backwards.
        let code = "```rust\nfn step(input: &str) -> usize {\n    input.len()\n}\n```\n\nExplanation words. ";
        let text = code.repeat(60);
        let mut segmenter = Segmenter::default();
        let mut seen = 0;
        for end in (1..=text.len()).step_by(5).chain([text.len()]) {
            let before = segmenter.boundaries().len();
            segmenter.advance(&text[..end]);
            for boundary in &segmenter.boundaries()[before..] {
                assert!(
                    *boundary >= seen,
                    "split at {boundary} behind rendered {seen}"
                );
            }
            seen = end;
        }
        assert!(segmenter.boundaries().len() > 2);
    }

    #[test]
    fn walls_of_prose_split_softly_between_sentences() {
        let text = "This sentence has `inline code` and **bold** words. ".repeat(200);
        let mut segmenter = Segmenter::default();
        let mut seen = 0;
        for end in (1..=text.len()).step_by(11).chain([text.len()]) {
            let before = segmenter.boundaries().len();
            segmenter.advance(&text[..end]);
            for boundary in &segmenter.boundaries()[before..] {
                assert!(*boundary >= seen, "split behind rendered text");
            }
            seen = end;
        }
        assert!(segmenter.boundaries().len() >= 2);
        for (index, boundary) in segmenter.boundaries().iter().enumerate() {
            assert!(segmenter.is_soft(index));
            assert!(text[*boundary..].starts_with("This sentence"));
            let piece = &text[..*boundary];
            assert_eq!(
                piece.matches('`').count() % 2,
                0,
                "split inside inline code"
            );
            assert_eq!(piece.matches("**").count() % 2, 0, "split inside bold");
        }
    }

    #[test]
    fn block_lines_are_never_split_mid_line() {
        for prefix in ["- ", "> ", "| ", "# ", "1. "] {
            let text = format!(
                "{prefix}{}\n",
                "Words end here. Next one starts. ".repeat(200)
            );
            let mut segmenter = Segmenter::default();
            segmenter.advance(&text);
            assert_eq!(segmenter.boundaries(), &[] as &[usize], "{prefix:?}");
        }
    }

    #[test]
    fn incremental_scanning_matches_a_single_scan() {
        let text = format!("{}\n\n", "token ".repeat(90)).repeat(20);
        let mut streamed = Segmenter::default();
        for end in (0..=text.len()).step_by(7).chain([text.len()]) {
            streamed.advance(&text[..end]);
        }
        let mut whole = Segmenter::default();
        whole.advance(&text);
        assert_eq!(streamed.boundaries(), whole.boundaries());
    }
}
