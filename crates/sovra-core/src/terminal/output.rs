//! Turning a child process's raw bytes into readable lines, and keeping a bounded
//! copy for the model.
//!
//! Output can be gigabytes (`yes`, a noisy build), so nothing here grows without a
//! bound: the decoder holds one line, [`OutputCapture`] keeps a head and a rolling
//! tail, and the UI keeps its own capped tail.

use std::collections::VecDeque;

/// A line longer than this is split so a single huge line cannot exhaust memory.
pub const MAX_LINE_BYTES: usize = 4 * 1024;
/// Bytes of the start of the output kept for the model.
pub const MODEL_HEAD_BYTES: usize = 12 * 1024;
/// Bytes of the end of the output kept for the model.
pub const MODEL_TAIL_BYTES: usize = 18 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Escape {
    #[default]
    None,
    /// After `ESC`.
    Start,
    /// Inside `ESC [ … final`.
    Csi,
    /// Inside `ESC ] … BEL` or `ESC ] … ESC \`.
    Osc,
    /// `ESC` seen inside an OSC string (maybe the `ESC \` terminator).
    OscEnd,
}

/// Incremental bytes → lines decoder: lossy UTF-8 (sequences split across reads are
/// rejoined), ANSI escape sequences removed, `\r\n` normalised and a lone `\r`
/// (progress bars) treated as "rewrite the current line".
#[derive(Debug, Default)]
pub struct LineDecoder {
    pending: Vec<u8>,
    line: String,
    escape: Escape,
    /// A `\r` was seen; the next character decides between `\r\n` and a rewrite.
    carriage: bool,
}

impl LineDecoder {
    /// Decodes `bytes`, appending each completed line to `out`.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<String>) {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut rest = input.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    self.feed(text, out);
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    // Valid UTF-8 by construction.
                    self.feed(std::str::from_utf8(valid).unwrap_or_default(), out);
                    if let Some(bad) = error.error_len() {
                        self.feed("\u{FFFD}", out);
                        rest = after.get(bad..).unwrap_or_default();
                    } else {
                        // An incomplete sequence at the end: wait for more bytes.
                        self.pending = after.to_vec();
                        break;
                    }
                }
            }
        }
    }

    /// Flushes the unfinished last line, if any.
    pub fn finish(&mut self, out: &mut Vec<String>) {
        if !self.pending.is_empty() {
            self.pending.clear();
            self.line.push('\u{FFFD}');
        }
        self.carriage = false;
        if !self.line.is_empty() {
            out.push(std::mem::take(&mut self.line));
        }
    }

    /// The unfinished line so far (e.g. a `Password:` prompt with no newline).
    pub fn partial(&self) -> &str {
        &self.line
    }

    fn feed(&mut self, text: &str, out: &mut Vec<String>) {
        for c in text.chars() {
            match self.escape {
                Escape::Start => {
                    self.escape = match c {
                        '[' => Escape::Csi,
                        ']' => Escape::Osc,
                        _ => Escape::None,
                    };
                    continue;
                }
                Escape::Csi => {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        self.escape = Escape::None;
                    }
                    continue;
                }
                Escape::Osc => {
                    match c {
                        '\u{7}' => self.escape = Escape::None,
                        '\u{1b}' => self.escape = Escape::OscEnd,
                        _ => {}
                    }
                    continue;
                }
                Escape::OscEnd => {
                    self.escape = if c == '\\' { Escape::None } else { Escape::Osc };
                    continue;
                }
                Escape::None => {}
            }
            if self.carriage {
                self.carriage = false;
                if c != '\n' {
                    self.line.clear();
                }
            }
            match c {
                '\u{1b}' => self.escape = Escape::Start,
                '\n' => out.push(std::mem::take(&mut self.line)),
                '\r' => self.carriage = true,
                '\t' => self.line.push_str("    "),
                '\u{8}' => {
                    self.line.pop();
                }
                c if c.is_control() => {}
                c => {
                    if self.line.len() + c.len_utf8() > MAX_LINE_BYTES {
                        out.push(std::mem::take(&mut self.line));
                    }
                    self.line.push(c);
                }
            }
        }
    }
}

/// A bounded copy of a command's output for the model: the first
/// [`MODEL_HEAD_BYTES`] and the last [`MODEL_TAIL_BYTES`], with a note of what was
/// left out in between.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OutputCapture {
    head: Vec<String>,
    head_bytes: usize,
    tail: VecDeque<String>,
    tail_bytes: usize,
    omitted_lines: u64,
    total_lines: u64,
}

impl OutputCapture {
    /// Records one line.
    pub fn push(&mut self, line: String) {
        self.total_lines += 1;
        let size = line.len() + 1;
        if self.tail.is_empty() && self.head_bytes + size <= MODEL_HEAD_BYTES {
            self.head_bytes += size;
            self.head.push(line);
            return;
        }
        self.tail_bytes += size;
        self.tail.push_back(line);
        while self.tail_bytes > MODEL_TAIL_BYTES {
            let Some(dropped) = self.tail.pop_front() else {
                break;
            };
            self.tail_bytes -= dropped.len() + 1;
            self.omitted_lines += 1;
        }
    }

    /// Lines seen in total.
    pub fn total_lines(&self) -> u64 {
        self.total_lines
    }

    /// Whether nothing was printed.
    pub fn is_empty(&self) -> bool {
        self.total_lines == 0
    }

    /// The text given to the model.
    pub fn render(&self) -> String {
        let mut text = String::with_capacity(self.head_bytes + self.tail_bytes + 64);
        for line in &self.head {
            text.push_str(line);
            text.push('\n');
        }
        if self.omitted_lines > 0 {
            text.push_str("… ");
            text.push_str(&self.omitted_lines.to_string());
            text.push_str(" lines omitted …\n");
        }
        for line in &self.tail {
            text.push_str(line);
            text.push('\n');
        }
        while text.ends_with('\n') {
            text.pop();
        }
        text
    }
}
