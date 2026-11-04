//! A conservative shell-command reader for permission checks.
//!
//! This is not a shell. It splits a command line into the simple commands the
//! shell would run, so each one can be checked on its own (`a && b` must have
//! both `a` and `b` approved). Anything it cannot read with certainty —
//! command substitution, subshells, here-docs, variables in the program
//! position — marks the whole command *opaque*: it may still run after the
//! person approves it once, but no saved rule or read-only shortcut applies.
//! Erring towards "ask" is the point; see `docs/terminal.md`.

use serde::{Deserialize, Serialize};

/// Commands longer than this are never analysed: they always ask.
pub const MAX_ANALYSED_COMMAND_BYTES: usize = 10_000;

/// Why a command could not be fully understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Opaque {
    /// `$(…)` or backticks: runs a hidden inner command.
    CommandSubstitution,
    /// `( … )`, `{ … }` groups or `<( … )` process substitution.
    Grouping,
    /// `<<` here-documents and here-strings.
    HereDocument,
    /// The program itself comes from a variable or wildcard.
    DynamicProgram,
    /// Unbalanced quotes, a dangling `&&`, a redirect with no target, …
    Malformed,
    /// Longer than [`MAX_ANALYSED_COMMAND_BYTES`].
    TooLong,
}

/// One shell word after quote removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The word with quotes removed (`'a b'` → `a b`).
    pub text: String,
    /// Contains a `$` expansion, so its runtime value is unknown.
    pub dynamic: bool,
    /// Contains an unquoted wildcard (`*`, `?`, `[`, `{`).
    pub glob: bool,
}

impl Word {
    fn literal(&self) -> bool {
        !self.dynamic && !self.glob
    }
}

/// Where a redirect points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// `> file`, `>> file`, `&> file`: writes a file.
    Write(String),
    /// `< file`: reads a file.
    Read(String),
    /// `2>&1`, `> /dev/null`: no file is touched.
    Harmless,
}

/// How a simple command is joined to the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joiner {
    /// `;`, a newline, or the end of the command.
    Sequence,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `|` or `|&`
    Pipe,
    /// `&`: the command keeps running in the background.
    Background,
}

/// One simple command: optional `NAME=value` prefixes, a program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    /// Leading `NAME=value` assignments, as written.
    pub assignments: Vec<String>,
    /// Wrappers removed before matching (`timeout 30`, `nice`, `time`, …).
    pub wrappers: Vec<String>,
    /// The program word, then its arguments (wrappers and assignments removed).
    pub words: Vec<Word>,
    /// Redirects attached to this command.
    pub redirects: Vec<Redirect>,
    /// How this command connects to the next.
    pub joiner: Joiner,
    /// The command's own text, for display.
    pub source: String,
}

impl SimpleCommand {
    /// The program as written (`npm`, `./run.sh`, `/usr/bin/ls`).
    pub fn program(&self) -> &str {
        self.words.first().map_or("", |word| word.text.as_str())
    }

    /// Lower-cased file name of the program without `.exe`/`.cmd`/`.bat`/`.ps1`, used to
    /// recognise it whatever path or casing was typed.
    pub fn program_name(&self) -> String {
        program_name(self.program())
    }

    /// The argument words (everything after the program).
    pub fn args(&self) -> &[Word] {
        self.words.get(1..).unwrap_or_default()
    }

    /// Whether an argument (not the program) equals `flag` exactly or starts with
    /// `flag=`.
    pub fn has_arg(&self, flag: &str) -> bool {
        self.args().iter().any(|word| {
            word.text == flag
                || word
                    .text
                    .strip_prefix(flag)
                    .is_some_and(|rest| rest.starts_with('='))
        })
    }

    /// The first argument that does not start with `-`, if any.
    pub fn first_operand(&self) -> Option<&Word> {
        self.args().iter().find(|word| !word.text.starts_with('-'))
    }

    /// Whether any redirect writes a file.
    pub fn writes_by_redirect(&self) -> bool {
        self.redirects
            .iter()
            .any(|redirect| matches!(redirect, Redirect::Write(_)))
    }
}

/// Lower-cased file name of a program path, without Windows executable extensions.
pub fn program_name(program: &str) -> String {
    let base = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
        if let Some(stem) = base.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    base
}

/// A command line split into simple commands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandAnalysis {
    /// The simple commands, in order.
    pub commands: Vec<SimpleCommand>,
    /// Everything that made the command unreadable; empty when fully understood.
    pub opaque: Vec<Opaque>,
}

impl CommandAnalysis {
    /// Whether every part of the command was understood.
    pub fn is_clear(&self) -> bool {
        self.opaque.is_empty()
    }

    /// Whether any part keeps running in the background (`&`).
    pub fn has_background(&self) -> bool {
        self.commands
            .iter()
            .any(|command| command.joiner == Joiner::Background)
    }
}

/// Splits `command` into simple commands. Never fails: problems are recorded in
/// [`CommandAnalysis::opaque`].
pub fn analyse(command: &str) -> CommandAnalysis {
    if command.len() > MAX_ANALYSED_COMMAND_BYTES {
        return CommandAnalysis {
            commands: Vec::new(),
            opaque: vec![Opaque::TooLong],
        };
    }
    let mut reader = Reader::new(command);
    reader.run();
    let mut analysis = reader.finish();
    for simple in &mut analysis.commands {
        strip_prefixes(simple);
        if simple
            .words
            .first()
            .is_some_and(|program| !program.literal())
        {
            mark(&mut analysis.opaque, Opaque::DynamicProgram);
        }
    }
    analysis
}

fn mark(opaque: &mut Vec<Opaque>, reason: Opaque) {
    if !opaque.contains(&reason) {
        opaque.push(reason);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRedirect {
    Write,
    Read,
    /// `>&` / `<&`: a file descriptor number or `-` duplicates; anything else is a file.
    Duplicate,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "a tokenizer's independent per-word flags, not a state machine in disguise"
)]
struct Reader<'a> {
    source: &'a str,
    chars: Vec<(usize, char)>,
    at: usize,
    quote: Quote,
    word: String,
    in_word: bool,
    dynamic: bool,
    glob: bool,
    words: Vec<Word>,
    redirects: Vec<Redirect>,
    pending: Option<PendingRedirect>,
    segment_start: usize,
    commands: Vec<SimpleCommand>,
    opaque: Vec<Opaque>,
    /// The last joiner needs something after it (`&&`, `||`, `|`).
    needs_more: bool,
}

impl<'a> Reader<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.char_indices().collect(),
            at: 0,
            quote: Quote::None,
            word: String::new(),
            in_word: false,
            dynamic: false,
            glob: false,
            words: Vec::new(),
            redirects: Vec::new(),
            pending: None,
            segment_start: 0,
            commands: Vec::new(),
            opaque: Vec::new(),
            needs_more: false,
        }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).map(|&(_, c)| c)
    }

    fn offset(&self) -> usize {
        self.chars
            .get(self.at)
            .map_or(self.source.len(), |&(offset, _)| offset)
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            match self.quote {
                Quote::Single => self.single(c),
                Quote::Double => self.double(c),
                Quote::None => self.unquoted(c),
            }
        }
    }

    fn push(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }

    fn single(&mut self, c: char) {
        self.at += 1;
        if c == '\'' {
            self.quote = Quote::None;
        } else {
            self.word.push(c);
        }
    }

    fn double(&mut self, c: char) {
        self.at += 1;
        match c {
            '"' => self.quote = Quote::None,
            '\\' => match self.peek(0) {
                Some(next @ ('$' | '`' | '"' | '\\')) => {
                    self.word.push(next);
                    self.at += 1;
                }
                Some('\n') => self.at += 1,
                _ => self.word.push('\\'),
            },
            '`' => mark(&mut self.opaque, Opaque::CommandSubstitution),
            '$' => self.dollar(),
            other => self.word.push(other),
        }
    }

    /// Called after consuming a `$` (inside or outside double quotes).
    fn dollar(&mut self) {
        match self.peek(0) {
            Some('(') => mark(&mut self.opaque, Opaque::CommandSubstitution),
            Some('\'') if self.quote == Quote::None => mark(&mut self.opaque, Opaque::Malformed),
            Some(c) if c.is_ascii_alphanumeric() || "_{@*#?$!-".contains(c) => {
                self.dynamic = true;
            }
            _ => {}
        }
        self.word.push('$');
        self.in_word = true;
    }

    fn unquoted(&mut self, c: char) {
        self.at += 1;
        match c {
            ' ' | '\t' | '\r' => self.end_word(),
            '\n' => self.end_command(Joiner::Sequence, false),
            '\\' => match self.peek(0) {
                Some('\n') => self.at += 1,
                Some(next) => {
                    self.push(next);
                    self.at += 1;
                }
                None => self.push('\\'),
            },
            '\'' => {
                self.in_word = true;
                self.quote = Quote::Single;
            }
            '"' => {
                self.in_word = true;
                self.quote = Quote::Double;
            }
            '#' if !self.in_word => {
                while self.peek(0).is_some_and(|next| next != '\n') {
                    self.at += 1;
                }
            }
            '`' | '(' | ')' => mark(
                &mut self.opaque,
                if c == '`' {
                    Opaque::CommandSubstitution
                } else {
                    Opaque::Grouping
                },
            ),
            '$' => self.dollar(),
            // A lone `{` opens a command group; elsewhere it is brace expansion.
            '{' if !self.in_word && matches!(self.peek(0), Some(' ' | '\t' | '\n') | None) => {
                mark(&mut self.opaque, Opaque::Grouping);
            }
            '*' | '?' | '[' | '{' => {
                self.glob = true;
                self.push(c);
            }
            '<' => self.less(),
            '>' => self.greater(),
            '&' => self.ampersand(),
            '|' => {
                if self.peek(0) == Some('|') {
                    self.at += 1;
                    self.end_command(Joiner::Or, true);
                } else {
                    if self.peek(0) == Some('&') {
                        self.at += 1;
                    }
                    self.end_command(Joiner::Pipe, true);
                }
            }
            ';' => {
                if self.peek(0) == Some(';') {
                    mark(&mut self.opaque, Opaque::Grouping);
                    self.at += 1;
                }
                self.end_command(Joiner::Sequence, false);
            }
            other => self.push(other),
        }
    }

    /// A word made only of digits right before `<`/`>` is a file-descriptor number.
    fn take_fd_prefix(&mut self) {
        if self.in_word
            && !self.word.is_empty()
            && self.word.bytes().all(|b| b.is_ascii_digit())
            && !self.dynamic
            && !self.glob
        {
            self.word.clear();
            self.in_word = false;
        } else {
            self.end_word();
        }
    }

    fn less(&mut self) {
        match self.peek(0) {
            Some('<') => {
                mark(&mut self.opaque, Opaque::HereDocument);
                self.at += 1;
                self.end_word();
            }
            Some('(') => mark(&mut self.opaque, Opaque::Grouping),
            Some('&') => {
                self.at += 1;
                self.take_fd_prefix();
                self.set_pending(PendingRedirect::Duplicate);
            }
            Some('>') => {
                // `<>` opens read-write.
                self.at += 1;
                self.take_fd_prefix();
                self.set_pending(PendingRedirect::Write);
            }
            _ => {
                self.take_fd_prefix();
                self.set_pending(PendingRedirect::Read);
            }
        }
    }

    fn greater(&mut self) {
        if self.peek(0) == Some('(') {
            mark(&mut self.opaque, Opaque::Grouping);
            return;
        }
        self.take_fd_prefix();
        match self.peek(0) {
            Some('>' | '|') => {
                self.at += 1;
                self.set_pending(PendingRedirect::Write);
            }
            Some('&') => {
                self.at += 1;
                self.set_pending(PendingRedirect::Duplicate);
            }
            _ => self.set_pending(PendingRedirect::Write),
        }
    }

    fn ampersand(&mut self) {
        match self.peek(0) {
            Some('&') => {
                self.at += 1;
                self.end_command(Joiner::And, true);
            }
            Some('>') => {
                // `&>` / `&>>`: stdout and stderr to a file.
                self.at += 1;
                if self.peek(0) == Some('>') {
                    self.at += 1;
                }
                self.end_word();
                self.set_pending(PendingRedirect::Write);
            }
            _ => self.end_command(Joiner::Background, false),
        }
    }

    fn set_pending(&mut self, pending: PendingRedirect) {
        if self.pending.is_some() {
            mark(&mut self.opaque, Opaque::Malformed);
        }
        self.pending = Some(pending);
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = Word {
            text: std::mem::take(&mut self.word),
            dynamic: std::mem::take(&mut self.dynamic),
            glob: std::mem::take(&mut self.glob),
        };
        self.in_word = false;
        match self.pending.take() {
            None => self.words.push(word),
            Some(kind) => self.redirects.push(redirect_for(kind, &word)),
        }
    }

    fn end_command(&mut self, joiner: Joiner, needs_more: bool) {
        self.end_word();
        if self.pending.take().is_some() {
            mark(&mut self.opaque, Opaque::Malformed);
        }
        let end = self.offset();
        // Joiner characters were consumed already; trim them off the display text.
        let source = self
            .source
            .get(self.segment_start..end)
            .unwrap_or_default()
            .trim_end_matches(['&', '|', ';', '\n'])
            .trim()
            .to_owned();
        self.segment_start = end;
        if self.words.is_empty() {
            if !self.redirects.is_empty() || self.needs_more || joiner != Joiner::Sequence {
                mark(&mut self.opaque, Opaque::Malformed);
            }
            self.redirects.clear();
            self.needs_more = needs_more;
            return;
        }
        self.commands.push(SimpleCommand {
            assignments: Vec::new(),
            wrappers: Vec::new(),
            words: std::mem::take(&mut self.words),
            redirects: std::mem::take(&mut self.redirects),
            joiner,
            source,
        });
        self.needs_more = needs_more;
    }

    fn finish(mut self) -> CommandAnalysis {
        if self.quote != Quote::None {
            mark(&mut self.opaque, Opaque::Malformed);
            self.quote = Quote::None;
        }
        // A dangling `&&`/`||`/`|` leaves an empty final command, which
        // `end_command` records as malformed.
        self.end_command(Joiner::Sequence, false);
        CommandAnalysis {
            commands: self.commands,
            opaque: self.opaque,
        }
    }
}

fn redirect_for(kind: PendingRedirect, target: &Word) -> Redirect {
    let text = target.text.as_str();
    let null_device = matches!(text, "/dev/null" | "NUL" | "nul" | "$null");
    match kind {
        PendingRedirect::Duplicate
            if text == "-" || (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())) =>
        {
            Redirect::Harmless
        }
        _ if null_device => Redirect::Harmless,
        PendingRedirect::Read => Redirect::Read(text.to_owned()),
        PendingRedirect::Write | PendingRedirect::Duplicate => Redirect::Write(text.to_owned()),
    }
}

/// Environment variables that only change formatting or verbosity, so an allow rule
/// may still match past `NAME=value cmd`. Any other assignment (e.g. `PATH=`,
/// `LD_PRELOAD=`) can change what runs, so rules stop matching.
const SAFE_ASSIGNMENTS: &[&str] = &[
    "CI",
    "CLICOLOR",
    "COLUMNS",
    "FORCE_COLOR",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "NODE_ENV",
    "NO_COLOR",
    "PYTHONUNBUFFERED",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "TERM",
    "TZ",
];

/// Whether every leading assignment only sets a formatting variable.
pub(crate) fn assignments_are_safe(command: &SimpleCommand) -> bool {
    command.assignments.iter().all(|assignment| {
        assignment
            .split_once('=')
            .is_some_and(|(name, _)| SAFE_ASSIGNMENTS.contains(&name) || name.starts_with("LC_"))
    })
}

fn is_assignment(word: &Word) -> bool {
    let Some((name, _)) = word.text.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Removes leading assignments and pass-through wrappers so rules match the real
/// program: `NO_COLOR=1 timeout 30 npm test` is checked as `npm test`.
fn strip_prefixes(command: &mut SimpleCommand) {
    let mut start = 0;
    while command.words.get(start).is_some_and(is_assignment) {
        start += 1;
    }
    command.assignments = command.words[..start]
        .iter()
        .map(|word| word.text.clone())
        .collect();
    while let Some(program) = command.words.get(start) {
        let name = program_name(&program.text);
        let Some(skip) = wrapper_length(&name, &command.words[start + 1..]) else {
            break;
        };
        let taken: Vec<&str> = command.words[start..=start + skip]
            .iter()
            .map(|word| word.text.as_str())
            .collect();
        command.wrappers.push(taken.join(" "));
        start += 1 + skip;
    }
    command.words.drain(..start);
}

/// For a pass-through wrapper, how many of its own arguments precede the wrapped
/// program; `None` when `name` is not a wrapper or its arguments cannot be read.
fn wrapper_length(name: &str, rest: &[Word]) -> Option<usize> {
    let wrapped_exists = |skip: usize| rest.len() > skip;
    match name {
        "time" | "nohup" | "builtin" | "noglob" => wrapped_exists(0).then_some(0),
        // `command -v x` looks a program up instead of running it.
        "command" => (wrapped_exists(0) && !rest[0].text.starts_with('-')).then_some(0),
        "nice" => {
            let skip = match rest.first().map(|word| word.text.as_str()) {
                Some("-n") => 2,
                Some(flag) if flag.starts_with('-') => 1,
                _ => 0,
            };
            wrapped_exists(skip).then_some(skip)
        }
        "stdbuf" => {
            let skip = rest
                .iter()
                .take_while(|word| {
                    let text = word.text.as_str();
                    ["-i", "-o", "-e", "--input", "--output", "--error"]
                        .iter()
                        .any(|flag| text.starts_with(flag))
                })
                .count();
            (skip > 0 && wrapped_exists(skip)).then_some(skip)
        }
        "timeout" => {
            let mut skip = 0;
            while let Some(word) = rest.get(skip) {
                match word.text.as_str() {
                    "-s" | "-k" | "--signal" | "--kill-after" => skip += 2,
                    flag if flag.starts_with('-') => skip += 1,
                    _ => break,
                }
            }
            // The duration itself.
            let duration = rest.get(skip)?;
            if !duration.literal()
                || !duration
                    .text
                    .trim_end_matches(['s', 'm', 'h', 'd'])
                    .parse::<f64>()
                    .is_ok_and(f64::is_finite)
            {
                return None;
            }
            skip += 1;
            wrapped_exists(skip).then_some(skip)
        }
        _ => None,
    }
}
