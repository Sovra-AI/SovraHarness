//! Splits a streamed Markdown reply into segments that render as separate,
//! virtualized list rows. A reply rendered as one row is laid out in full on every
//! streamed update, so frame cost grew with its length; per-segment rows keep the
//! cost proportional to what is on screen, and only the last segment changes.
//!
//! Splits happen only where Markdown is self-contained: at a non-indented line that
//! follows a blank line, outside fenced code. Scanning is incremental — each call
//! looks only at newly completed lines.

/// Segments grow to at least this many bytes before the next safe split.
pub(crate) const SEGMENT_BYTES: usize = 1536;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Segmenter {
    /// Byte offsets where segments after the first begin.
    boundaries: Vec<usize>,
    /// Bytes of complete lines already scanned.
    scanned: usize,
    /// The open code fence: its character and length.
    fence: Option<(char, usize)>,
    previous_blank: bool,
    segment_start: usize,
}

impl Segmenter {
    /// Scans the lines of `text` completed since the last call. `text` must extend
    /// the text previously scanned.
    pub(crate) fn advance(&mut self, text: &str) {
        while let Some(offset) = text.get(self.scanned..).and_then(|rest| rest.find('\n')) {
            let line_start = self.scanned;
            let line = &text[line_start..line_start + offset];
            let blank = line.trim().is_empty();
            let indented = line.starts_with([' ', '\t']);
            if !blank
                && !indented
                && self.previous_blank
                && self.fence.is_none()
                && line_start - self.segment_start >= SEGMENT_BYTES
            {
                self.boundaries.push(line_start);
                self.segment_start = line_start;
            }
            self.track_fence(line.trim_start());
            self.previous_blank = blank;
            self.scanned = line_start + offset + 1;
        }
    }

    pub(crate) fn boundaries(&self) -> &[usize] {
        &self.boundaries
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
