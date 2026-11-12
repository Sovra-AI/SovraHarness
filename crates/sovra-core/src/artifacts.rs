//! Artifacts: anything Sovra or the team produced that the user keeps — a
//! document, a piece of code, a table, a diagram, a web page, a checklist.
//!
//! The kinds follow what AI clients converge on (Claude artifacts: Markdown,
//! code, HTML, SVG, Mermaid; ChatGPT/Gemini canvas: documents and code;
//! `SovraSRX` `user_artifacts`: note, document, code, data, spreadsheet, link).
//! Each kind is one row in [`KINDS`]; adding a kind is one enum variant plus one
//! row, and the compiler lists every `match` to update. Content is plain text,
//! so every artifact can be previewed, copied and saved as a normal file.

mod project;
pub use project::WebsiteProject;

use serde::{Deserialize, Serialize};

use crate::memory::Timestamp;

/// What sort of thing an artifact is; decides its preview, icon and file type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Document,
    Checklist,
    Table,
    Code,
    Diagram,
    Webpage,
    Picture,
    Data,
    Note,
    Project,
}

/// Facts about one kind. `icon` is a semantic key the UI maps to its icon set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindInfo {
    pub kind: ArtifactKind,
    /// Singular label shown to people ("Document").
    pub label: &'static str,
    /// Plural label for filters ("Documents").
    pub plural: &'static str,
    /// One plain sentence explaining the kind to a non-technical person.
    pub explanation: &'static str,
    pub icon: &'static str,
    /// File extension used when the language does not pick a better one.
    pub extension: &'static str,
    pub mime: &'static str,
    /// Whether "Open" shows it in the web browser (it renders there, not here).
    pub opens_in_browser: bool,
}

/// One row per kind, in the order the library's filters show them.
pub const KINDS: [KindInfo; 10] = [
    KindInfo {
        kind: ArtifactKind::Document,
        label: "Document",
        plural: "Documents",
        explanation: "Writing you can read, print or share, like a letter or a guide.",
        icon: "document",
        extension: "md",
        mime: "text/markdown",
        opens_in_browser: false,
    },
    KindInfo {
        kind: ArtifactKind::Checklist,
        label: "Checklist",
        plural: "Checklists",
        explanation: "A list of things to do or buy, with boxes to tick.",
        icon: "checklist",
        extension: "md",
        mime: "text/markdown",
        opens_in_browser: false,
    },
    KindInfo {
        kind: ArtifactKind::Table,
        label: "Table",
        plural: "Tables",
        explanation: "Information in rows and columns; opens in a spreadsheet.",
        icon: "table",
        extension: "csv",
        mime: "text/csv",
        opens_in_browser: false,
    },
    KindInfo {
        kind: ArtifactKind::Code,
        label: "Code",
        plural: "Code",
        explanation: "Instructions for a computer, such as a script or a formula.",
        icon: "code",
        extension: "txt",
        mime: "text/plain",
        opens_in_browser: false,
    },
    KindInfo {
        kind: ArtifactKind::Diagram,
        label: "Diagram",
        plural: "Diagrams",
        explanation: "A chart of boxes and arrows, like a flowchart or a timeline.",
        icon: "diagram",
        extension: "mmd",
        mime: "text/vnd.mermaid",
        opens_in_browser: true,
    },
    KindInfo {
        kind: ArtifactKind::Webpage,
        label: "Web page",
        plural: "Web pages",
        explanation: "A page that opens in your web browser, like a small app or a flyer.",
        icon: "webpage",
        extension: "html",
        mime: "text/html",
        opens_in_browser: true,
    },
    KindInfo {
        kind: ArtifactKind::Picture,
        label: "Picture",
        plural: "Pictures",
        explanation: "A drawing, logo or icon that opens in your web browser.",
        icon: "picture",
        extension: "svg",
        mime: "image/svg+xml",
        opens_in_browser: true,
    },
    KindInfo {
        kind: ArtifactKind::Data,
        label: "Data",
        plural: "Data",
        explanation: "Structured information other apps can read.",
        icon: "data",
        extension: "json",
        mime: "application/json",
        opens_in_browser: false,
    },
    KindInfo {
        kind: ArtifactKind::Note,
        label: "Note",
        plural: "Notes",
        explanation: "A short note or snippet worth keeping.",
        icon: "note",
        extension: "txt",
        mime: "text/plain",
        opens_in_browser: false,
    },
    KindInfo {
        kind: ArtifactKind::Project,
        label: "Website project",
        plural: "Website projects",
        explanation: "A website made of several files, including React, Vue and other frontend frameworks.",
        icon: "project",
        extension: "json",
        mime: "application/json",
        opens_in_browser: true,
    },
];

impl ArtifactKind {
    pub fn info(self) -> &'static KindInfo {
        let index = match self {
            Self::Document => 0,
            Self::Checklist => 1,
            Self::Table => 2,
            Self::Code => 3,
            Self::Diagram => 4,
            Self::Webpage => 5,
            Self::Picture => 6,
            Self::Data => 7,
            Self::Note => 8,
            Self::Project => 9,
        };
        &KINDS[index]
    }

    pub fn label(self) -> &'static str {
        self.info().label
    }

    /// The kind a fenced block's language names (` ```mermaid ` is a diagram).
    pub fn for_language(language: &str) -> Self {
        match language.trim().to_ascii_lowercase().as_str() {
            "website-project" => Self::Project,
            "mermaid" | "mmd" => Self::Diagram,
            "html" | "htm" => Self::Webpage,
            "svg" => Self::Picture,
            "csv" | "tsv" => Self::Table,
            "json" | "yaml" | "yml" | "toml" | "xml" => Self::Data,
            "markdown" | "md" => Self::Document,
            "" | "text" | "txt" | "plain" | "plaintext" => Self::Note,
            _ => Self::Code,
        }
    }
}

/// File extension for saving: the language's own when it is code.
pub fn file_extension(kind: ArtifactKind, language: &str) -> &'static str {
    if kind != ArtifactKind::Code {
        return kind.info().extension;
    }
    match language.trim().to_ascii_lowercase().as_str() {
        "python" | "py" => "py",
        "rust" | "rs" => "rs",
        "javascript" | "js" => "js",
        "typescript" | "ts" => "ts",
        "bash" | "sh" | "shell" | "zsh" => "sh",
        "sql" => "sql",
        "css" => "css",
        "java" => "java",
        "c" => "c",
        "cpp" | "c++" => "cpp",
        "go" => "go",
        "swift" => "swift",
        _ => "txt",
    }
}

/// Where an artifact came from, so it can be traced back to its conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSource {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub message_id: String,
    /// Who made it: "Sovra" or a team member's name.
    #[serde(default)]
    pub author: String,
    /// The shared group artifact (its id in hex) this is a kept copy of, so sharing
    /// an edit makes it that artifact's newest version. Empty for everything else.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shared_from: String,
}

/// An artifact before the library assigned it an id and time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDraft {
    pub title: String,
    pub kind: ArtifactKind,
    /// The code language or format (`python`, `mermaid`); empty when not code.
    #[serde(default)]
    pub language: String,
    pub content: String,
    #[serde(default)]
    pub source: ArtifactSource,
}

/// An earlier version, kept when the artifact is edited (never lost).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub content: String,
    pub saved_at: Timestamp,
}

/// A saved artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub title: String,
    pub kind: ArtifactKind,
    #[serde(default)]
    pub language: String,
    pub content: String,
    #[serde(default)]
    pub source: ArtifactSource,
    #[serde(default)]
    pub collection: String,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Earlier contents, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<ArtifactVersion>,
}

impl Artifact {
    pub fn from_draft(id: String, draft: ArtifactDraft, at: Timestamp) -> Self {
        Self {
            id,
            title: clean_title(&draft.title, draft.kind),
            kind: draft.kind,
            language: draft.language,
            content: draft.content,
            source: draft.source,
            collection: String::new(),
            pinned: false,
            created_at: at,
            updated_at: at,
            versions: Vec::new(),
        }
    }

    /// Applies an edit, keeping the previous content as a version.
    pub fn edit(&mut self, title: &str, content: &str, at: Timestamp) {
        if content != self.content {
            self.versions.push(ArtifactVersion {
                content: std::mem::replace(&mut self.content, content.to_owned()),
                saved_at: self.updated_at,
            });
        }
        self.title = clean_title(title, self.kind);
        self.updated_at = at;
    }

    /// A safe file name for "Save a copy".
    pub fn file_name(&self) -> String {
        let stem: String = self
            .title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let stem = if stem.is_empty() {
            "Artifact".to_owned()
        } else {
            stem
        };
        format!("{stem}.{}", file_extension(self.kind, &self.language))
    }

    /// The file contents for saving: tables become CSV so spreadsheets open them.
    pub fn export_text(&self) -> String {
        if self.kind == ArtifactKind::Table
            && let Some((headers, rows)) = parse_markdown_table(&self.content)
        {
            let mut out = csv_line(&headers);
            for row in rows {
                out.push_str(&csv_line(&row));
            }
            return out;
        }
        self.content.clone()
    }

    /// A short preview line for cards.
    pub fn preview(&self) -> String {
        let line = self
            .content
            .lines()
            .map(|l| {
                l.trim()
                    .trim_start_matches(['#', '-', '*', '>', '|', ' '])
                    .trim_end_matches('|')
                    .trim()
            })
            .find(|l| {
                !l.is_empty()
                    && !l.starts_with("```")
                    && !l.chars().all(|c| c == '-' || c == '|' || c == ':')
            })
            .unwrap_or_default();
        truncate(line, 120)
    }
}

fn csv_line(cells: &[String]) -> String {
    let quoted: Vec<String> = cells
        .iter()
        .map(|cell| {
            if cell.contains([',', '"', '\n']) {
                format!("\"{}\"", cell.replace('"', "\"\""))
            } else {
                cell.clone()
            }
        })
        .collect();
    format!("{}\n", quoted.join(","))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// A non-empty, single-line title of reasonable length.
pub fn clean_title(title: &str, kind: ArtifactKind) -> String {
    let line = title
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches('#')
                .trim()
                .trim_matches(['*', '_', '`'])
                .trim()
        })
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.is_empty() {
        format!("Untitled {}", kind.label().to_lowercase())
    } else {
        truncate(line, 60)
    }
}

/// Headers and rows of a Markdown pipe table, or of CSV text.
pub fn parse_markdown_table(text: &str) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let split = |line: &str| -> Vec<String> {
        line.trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_owned())
            .collect()
    };
    if lines.len() >= 2 && lines[0].contains('|') && is_separator(lines[1]) {
        let headers = split(lines[0]);
        let rows = lines[2..]
            .iter()
            .take_while(|l| l.contains('|'))
            .map(|l| split(l))
            .collect();
        return Some((headers, rows));
    }
    if lines.len() >= 2 && lines.iter().all(|l| l.contains(',')) {
        let split_csv = |l: &str| {
            l.split(',')
                .map(|c| c.trim().trim_matches('"').to_owned())
                .collect()
        };
        let headers: Vec<String> = split_csv(lines[0]);
        let rows = lines[1..].iter().map(|l| split_csv(l)).collect();
        return Some((headers, rows));
    }
    None
}

fn is_separator(line: &str) -> bool {
    line.contains('-') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// One part of a reply that could be kept on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub kind: ArtifactKind,
    pub language: String,
    pub content: String,
    /// Byte range in the reply, for "most of the reply is this block".
    pub span: std::ops::Range<usize>,
}

/// Finds fenced blocks, pipe tables and checklists in a Markdown reply.
pub fn find_pieces(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim();
        if let Some(info) = trimmed.strip_prefix("```") {
            let language = info
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let mut content = String::new();
            for inner in lines.by_ref() {
                offset += inner.len();
                if inner.trim_start().starts_with("```") {
                    break;
                }
                content.push_str(inner);
            }
            if !content.trim().is_empty() {
                pieces.push(Piece {
                    kind: ArtifactKind::for_language(&language),
                    language,
                    content: content.trim_end().to_owned(),
                    span: start..offset,
                });
            }
            continue;
        }
        let table_start =
            trimmed.starts_with('|') && lines.peek().is_some_and(|next| is_separator(next.trim()));
        let checklist_start = is_task_line(trimmed);
        if table_start || checklist_start {
            let mut content = line.to_owned();
            while let Some(next) = lines.peek() {
                let next_trimmed = next.trim();
                let continues = if table_start {
                    next_trimmed.starts_with('|') || is_separator(next_trimmed)
                } else {
                    is_task_line(next_trimmed)
                };
                if !continues {
                    break;
                }
                offset += next.len();
                content.push_str(next);
                lines.next();
            }
            pieces.push(Piece {
                kind: if table_start {
                    ArtifactKind::Table
                } else {
                    ArtifactKind::Checklist
                },
                language: String::new(),
                content: content.trim_end().to_owned(),
                span: start..offset,
            });
        }
    }
    pieces
}

fn is_task_line(line: &str) -> bool {
    ["- [ ]", "- [x]", "- [X]", "* [ ]", "* [x]"]
        .iter()
        .any(|p| line.starts_with(p))
}

/// The single best artifact for "Save" on a reply: the reply's main block when it
/// is most of the reply, otherwise the whole reply as a document.
pub fn draft_from_reply(text: &str, source: ArtifactSource, fallback_title: &str) -> ArtifactDraft {
    let pieces = find_pieces(text);
    let prose = text.len().max(1);
    if let Some(main) = pieces.iter().max_by_key(|p| p.span.len())
        && main.span.len() * 10 >= prose * 6
    {
        return ArtifactDraft {
            title: title_for(text, main, fallback_title),
            kind: main.kind,
            language: main.language.clone(),
            content: main.content.clone(),
            source,
        };
    }
    let checklist = pieces.iter().any(|p| p.kind == ArtifactKind::Checklist);
    ArtifactDraft {
        title: heading(text).unwrap_or_else(|| fallback_title.to_owned()),
        kind: if checklist {
            ArtifactKind::Checklist
        } else {
            ArtifactKind::Document
        },
        language: String::new(),
        content: text.trim().to_owned(),
        source,
    }
}

/// A draft for one piece (the "Save" button on a code block).
pub fn draft_from_piece(
    language: &str,
    content: &str,
    source: ArtifactSource,
    fallback_title: &str,
) -> ArtifactDraft {
    let kind = ArtifactKind::for_language(language);
    let title = if kind == ArtifactKind::Code && !language.is_empty() {
        format!("{fallback_title} ({language})")
    } else {
        fallback_title.to_owned()
    };
    ArtifactDraft {
        title,
        kind,
        language: language.to_owned(),
        content: content.trim_end().to_owned(),
        source,
    }
}

fn heading(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches('#').trim().to_owned())
        .filter(|l| !l.is_empty())
}

fn title_for(text: &str, piece: &Piece, fallback: &str) -> String {
    if let Some(heading) = heading(text) {
        return heading;
    }
    let lead = text[..piece.span.start].trim();
    let first = lead
        .lines()
        .next_back()
        .unwrap_or_default()
        .trim()
        .trim_end_matches(':');
    if !first.is_empty() && first.chars().count() <= 60 {
        first.to_owned()
    } else {
        fallback.to_owned()
    }
}

/// Stable sorting choices for large libraries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArtifactSort {
    #[default]
    Recent,
    Name,
    Oldest,
}

/// Which artifacts the library page shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactFilter {
    pub collection: Option<String>,
    pub favorites_only: bool,
    pub sort: ArtifactSort,
    pub kind: Option<ArtifactKind>,
    pub query: String,
}

/// A request from the UI about artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactCommand {
    Load,
    Organize {
        id: String,
        collection: String,
        pinned: bool,
    },
    RunProject {
        id: String,
        session_id: String,
    },
    /// Serve a saved web page on this computer until stopped or Sovra closes.
    Deploy {
        id: String,
    },
    StopDeployment {
        id: String,
    },
    Save(ArtifactDraft),
    Edit {
        id: String,
        title: String,
        content: String,
    },
    Delete {
        id: String,
    },
    /// Write the artifact to `path` (chosen by the user in a save dialog).
    Export {
        id: String,
        path: String,
    },
    /// Serve a browser preview over loopback HTTP and open its URL.
    Open {
        id: String,
    },
}

/// A notification from the runtime about artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactEvent {
    Loaded(Vec<Artifact>),
    BuildStatus { id: String, message: Option<String> },
    Deployed { id: String, url: String },
    DeploymentStopped { id: String },
    Saved(Box<Artifact>),
    Deleted { id: String },
    Exported { id: String, path: String },
    Error(String),
}

/// The user's artifact library, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactLibrary {
    pub artifacts: Vec<Artifact>,
    /// Ephemeral browser addresses; never restored after application restart.
    pub deployments: std::collections::BTreeMap<String, String>,
    pub builds: std::collections::BTreeMap<String, String>,
    pub loaded: bool,
    /// The last thing that happened, in plain words, for a status line.
    pub notice: Option<String>,
}

impl ArtifactLibrary {
    pub fn apply(&mut self, event: &ArtifactEvent) {
        match event {
            ArtifactEvent::BuildStatus { id, message } => {
                if let Some(message) = message {
                    self.builds.insert(id.clone(), message.clone());
                } else {
                    self.builds.remove(id);
                }
            }
            ArtifactEvent::Deployed { id, url } => {
                self.deployments.insert(id.clone(), url.clone());
                self.notice = Some(format!("Website running on this computer: {url}"));
            }
            ArtifactEvent::DeploymentStopped { id } => {
                self.deployments.remove(id);
                self.notice =
                    Some("Website stopped. Your saved source is still in Artifacts.".into());
            }
            ArtifactEvent::Loaded(artifacts) => {
                self.artifacts.clone_from(artifacts);
                self.sort();
                self.loaded = true;
            }
            ArtifactEvent::Saved(artifact) => {
                self.notice = Some(format!("Saved “{}” to Artifacts", artifact.title));
                if let Some(existing) = self.artifacts.iter_mut().find(|a| a.id == artifact.id) {
                    *existing = artifact.as_ref().clone();
                } else {
                    self.artifacts.push(artifact.as_ref().clone());
                }
                self.sort();
            }
            ArtifactEvent::Deleted { id } => {
                self.artifacts.retain(|a| a.id != *id);
                self.deployments.remove(id);
                self.notice = Some("Deleted".to_owned());
            }
            ArtifactEvent::Exported { path, .. } => {
                self.notice = Some(format!("Saved a copy to {path}"));
            }
            ArtifactEvent::Error(message) => self.notice = Some(message.clone()),
        }
    }

    fn sort(&mut self) {
        self.artifacts
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
    }

    pub fn get(&self, id: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.id == id)
    }

    pub fn filtered(&self, filter: &ArtifactFilter) -> Vec<&Artifact> {
        let query = filter.query.trim().to_lowercase();
        let mut found: Vec<_> = self
            .artifacts
            .iter()
            .filter(|a| !filter.favorites_only || a.pinned)
            .filter(|a| {
                filter
                    .collection
                    .as_ref()
                    .is_none_or(|folder| &a.collection == folder)
            })
            .filter(|a| filter.kind.is_none_or(|kind| a.kind == kind))
            .filter(|a| {
                query.is_empty()
                    || a.collection.to_lowercase().contains(&query)
                    || a.title.to_lowercase().contains(&query)
                    || a.content.to_lowercase().contains(&query)
                    || a.source.author.to_lowercase().contains(&query)
            })
            .collect();
        found.sort_by(|a, b| {
            match filter.sort {
                ArtifactSort::Recent => b.updated_at.cmp(&a.updated_at),
                ArtifactSort::Name => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                ArtifactSort::Oldest => a.created_at.cmp(&b.created_at),
            }
            .then(a.id.cmp(&b.id))
        });
        found
    }

    /// Folder names and counts; the empty name represents unfiled work.
    pub fn collections(&self) -> std::collections::BTreeMap<String, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for artifact in &self.artifacts {
            *counts.entry(artifact.collection.clone()).or_default() += 1;
        }
        counts
    }

    /// Kinds present in the library, in filter order, with counts.
    pub fn kind_counts(&self) -> Vec<(ArtifactKind, usize)> {
        KINDS
            .iter()
            .map(|info| {
                (
                    info.kind,
                    self.artifacts
                        .iter()
                        .filter(|a| a.kind == info.kind)
                        .count(),
                )
            })
            .filter(|(_, count)| *count > 0)
            .collect()
    }
}

/// A unique id from the save time and a per-process counter.
pub fn artifact_id(at: Timestamp, sequence: u64) -> String {
    format!("art-{}-{sequence}", at.millis())
}

/// A self-contained page that shows the artifact in a browser: HTML as is, SVG
/// in a page, Mermaid rendered by mermaid.js (fetched from a CDN, so diagrams
/// need internet; the source is shown underneath either way).
pub fn browser_page(artifact: &Artifact) -> String {
    let title = html_escape(&artifact.title);
    match artifact.kind {
        ArtifactKind::Webpage => artifact.content.clone(),
        ArtifactKind::Picture => format!(
            "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
             <body style=\"margin:0;display:grid;place-items:center;min-height:100vh;background:#fff\">{}</body>",
            artifact.content
        ),
        ArtifactKind::Diagram => format!(
            "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
             <body style=\"font-family:system-ui;margin:2rem\"><h1>{title}</h1>\
             <pre class=\"mermaid\">{}</pre>\
             <script type=\"module\">import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs';mermaid.initialize({{startOnLoad:true}});</script></body>",
            html_escape(&artifact.content)
        ),
        ArtifactKind::Document
        | ArtifactKind::Checklist
        | ArtifactKind::Table
        | ArtifactKind::Code
        | ArtifactKind::Data
        | ArtifactKind::Note
        | ArtifactKind::Project => format!(
            "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
             <body style=\"font-family:system-ui;margin:2rem;max-width:48rem\"><h1>{title}</h1>\
             <pre style=\"white-space:pre-wrap\">{}</pre></body>",
            html_escape(&artifact.content)
        ),
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
