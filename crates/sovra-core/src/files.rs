//! File attachments, model purposes and bounded document retrieval. No filesystem IO.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::models::{ModelInfo, ProviderKind};

pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 8;
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
pub const FILE_CHUNK_CHARS: usize = 4_000;

/// A job assigned to a model, distinct from the model's advertised output format.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum ModelPurpose {
    Documents,
    Images,
    Audio,
    Video,
}

impl ModelPurpose {
    pub const ALL: [Self; 4] = [Self::Documents, Self::Images, Self::Audio, Self::Video];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Documents => "Read documents",
            Self::Images => "Understand pictures",
            Self::Audio => "Listen to audio",
            Self::Video => "Watch videos",
        }
    }

    pub const fn modality(self) -> &'static str {
        match self {
            Self::Documents => "text",
            Self::Images => "image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }

    /// Unknown capability metadata never qualifies a media model. Transport support
    /// reflects the pinned ADK adapters, not merely what a provider advertises.
    pub fn accepts(self, model: &ModelInfo, provider: ProviderKind) -> bool {
        if !model.can_chat() {
            return false;
        }
        let transport = match self {
            Self::Documents | Self::Images => true,
            Self::Audio => matches!(
                provider,
                ProviderKind::OpenRouter | ProviderKind::OpenAiCompatible
            ),
            Self::Video => provider == ProviderKind::OpenRouter,
        };
        transport
            && (self == Self::Documents
                || model.input_modalities.iter().any(|m| m == self.modality()))
    }
}

/// Persisted alongside the chat message. Contains no original filesystem path.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub purpose: ModelPurpose,
    pub sections: usize,
    pub note: Option<String>,
    /// A small, normalized PNG thumbnail, never an arbitrary source image.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thumbnail: Vec<u8>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attachment")
            .field("id", &self.id)
            .field("mime_type", &self.mime_type)
            .field("size_bytes", &self.size_bytes)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentState {
    Preparing {
        id: String,
        name: String,
    },
    Ready(Attachment),
    Failed {
        id: String,
        name: String,
        message: String,
    },
}

impl AttachmentState {
    pub fn id(&self) -> &str {
        match self {
            Self::Preparing { id, .. } | Self::Failed { id, .. } => id,
            Self::Ready(file) => &file.id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Preparing { name, .. } | Self::Failed { name, .. } => name,
            Self::Ready(file) => &file.name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCommand {
    Prepare {
        session_id: String,
        id: String,
        path: PathBuf,
    },
    Discard {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePrepared {
    pub session_id: String,
    pub id: String,
    pub result: Result<Attachment, FileError>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct FileError(pub String);

/// Extracted material lives in the adapter, and crosses to the agent only for a turn.
#[derive(Clone)]
pub struct FileContent {
    pub attachment: Attachment,
    pub text: String,
    pub media: Vec<MediaPart>,
    /// Original bytes for explicitly labeled binary inspection; never executed.
    pub raw: Option<std::sync::Arc<[u8]>>,
}

#[derive(Clone)]
pub struct MediaPart {
    pub mime_type: String,
    pub data: Vec<u8>,
    /// Local container duration hint for admission, not a proof of codec validity.
    pub duration_millis: Option<u64>,
}

impl std::fmt::Debug for MediaPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPart")
            .field("mime_type", &self.mime_type)
            .field("bytes", &self.data.len())
            .field("duration_millis", &self.duration_millis)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSetup {
    pub purpose: ModelPurpose,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttachmentDraft {
    pub files: Vec<AttachmentState>,
    pub notice: Option<String>,
}

impl AttachmentDraft {
    pub fn begin(&mut self, id: String, name: String) -> bool {
        if self.files.len() >= MAX_ATTACHMENTS {
            self.notice = Some(
                "You can add up to eight files to one message. Send these first, then add more."
                    .into(),
            );
            return false;
        }
        self.notice = None;
        self.files.push(AttachmentState::Preparing { id, name });
        true
    }

    /// Late completions cannot resurrect a file the user removed.
    pub fn apply(&mut self, prepared: &FilePrepared) -> bool {
        let Some(file) = self.files.iter_mut().find(|file| file.id() == prepared.id) else {
            return false;
        };
        *file = match &prepared.result {
            Ok(attachment) => AttachmentState::Ready(attachment.clone()),
            Err(error) => AttachmentState::Failed {
                id: prepared.id.clone(),
                name: file.name().into(),
                message: error.0.clone(),
            },
        };
        true
    }

    pub fn ready(&self) -> Option<Vec<Attachment>> {
        self.files
            .iter()
            .map(|file| match file {
                AttachmentState::Ready(file) => Some(file.clone()),
                AttachmentState::Preparing { .. } | AttachmentState::Failed { .. } => None,
            })
            .collect()
    }

    pub fn remove(&mut self, id: &str) {
        self.files.retain(|file| file.id() != id);
        self.notice = None;
    }
}

// Content and media may be private documents; Debug must not print their bytes.
impl std::fmt::Debug for FileContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileContent")
            .field("id", &self.attachment.id)
            .finish_non_exhaustive()
    }
}

/// Synchronous IO port; runtime calls it only from owned blocking tasks.
pub trait FileStore: Send + Sync {
    fn prepare(&self, id: &str, path: &Path) -> Result<Attachment, FileError>;
    fn load(&self, id: &str) -> Result<FileContent, FileError>;
    fn discard(&self, id: &str) -> Result<(), FileError>;
}

/// Stable, Unicode-safe sections used both in prompt excerpts and the read-file tool.
pub fn file_sections(text: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let mut start = 0;
    let mut length = 0;
    for (offset, ch) in text.char_indices() {
        length += 1;
        if length >= FILE_CHUNK_CHARS && (ch.is_whitespace() || length >= FILE_CHUNK_CHARS + 200) {
            let end = offset + ch.len_utf8();
            sections.push(&text[start..end]);
            start = end;
            length = 0;
        }
    }
    if start < text.len() {
        sections.push(&text[start..]);
    }
    sections
}

/// Cached case folding for repeated searches. Original section text is borrowed
/// during construction and copied only for the bounded excerpts returned to a caller.
pub struct FileSearchIndex {
    ranges: Vec<std::ops::Range<usize>>,
    folded: Vec<String>,
    lines: Vec<(usize, usize)>,
}

impl std::fmt::Debug for FileSearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSearchIndex")
            .field("sections", &self.ranges.len())
            .finish_non_exhaustive()
    }
}

impl FileSearchIndex {
    pub fn new(text: &str) -> Self {
        let sections = file_sections(text);
        let mut offset = 0;
        let ranges = sections
            .iter()
            .map(|section| {
                let start = offset;
                offset += section.len();
                start..offset
            })
            .collect();
        let mut start_line = 1;
        let lines = sections
            .iter()
            .map(|section| {
                let next = start_line + section.bytes().filter(|byte| *byte == b'\n').count();
                let range = (
                    start_line,
                    next.saturating_sub(usize::from(section.ends_with('\n')))
                        .max(start_line),
                );
                start_line = next;
                range
            })
            .collect();
        Self {
            ranges,
            folded: sections.into_iter().map(str::to_lowercase).collect(),
            lines,
        }
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// `text` must be the unchanged source used to build this index. Invalid
    /// numbers or byte boundaries return None rather than panicking.
    pub fn section<'a>(&self, text: &'a str, number: usize) -> Option<&'a str> {
        text.get(self.ranges.get(number.checked_sub(1)?)?.clone())
    }

    /// Line numbers refer to the extracted text; source text retains original line endings.
    pub fn section_lines(&self, number: usize) -> Option<(usize, usize)> {
        self.lines.get(number.checked_sub(1)?).copied()
    }

    /// Matching sections in relevance order. Short identifiers and numbers are
    /// searchable; absent terms return no hits instead of unrelated opening text.
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        if limit == 0 {
            return Vec::new();
        }
        let words = query_terms(query);
        let mut scores = vec![0.0_f64; self.len()];
        let count = f64::from(u32::try_from(self.len()).unwrap_or(u32::MAX));
        for word in words {
            let matches: Vec<_> = self
                .folded
                .iter()
                .enumerate()
                .filter_map(|(index, section)| contains_term(section, &word).then_some(index))
                .collect();
            // Smoothed inverse document frequency gives a rare identifier more
            // weight than terms repeated throughout the document.
            let frequency = f64::from(u32::try_from(matches.len()).unwrap_or(u32::MAX));
            let weight = (1.0 + count / (1.0 + frequency)).ln();
            for index in matches {
                scores[index] += weight;
            }
        }
        let mut ranked: Vec<_> = scores
            .into_iter()
            .enumerate()
            .filter(|(_, score)| *score > 0.0)
            .collect();
        ranked
            .sort_unstable_by(|(left, a), (right, b)| b.total_cmp(a).then_with(|| left.cmp(right)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(index, _)| index + 1)
            .collect()
    }

    /// Initial prompt evidence: matches first, opening context when there is room,
    /// or the first sections for a broad question with no matching terms.
    pub fn initial_sections(&self, query: &str, limit: usize) -> Vec<usize> {
        let mut numbers = self.search(query, limit);
        if numbers.is_empty() {
            return (1..=self.len()).take(limit).collect();
        }
        if limit > 1 && !numbers.contains(&1) {
            if numbers.len() == limit {
                numbers.pop();
            }
            numbers.push(1);
        }
        numbers
    }

    /// A bounded, Unicode-safe window around a matching line, including exact
    /// extracted-text line citations and whether the full section was included.
    pub fn excerpt<'a>(
        &self,
        text: &'a str,
        number: usize,
        query: &str,
        char_budget: usize,
    ) -> Option<FileExcerpt<'a>> {
        if char_budget == 0 {
            return None;
        }
        let section = self.section(text, number)?;
        if section.len() <= char_budget || section.chars().count() <= char_budget {
            let (start_line, end_line) = self.section_lines(number)?;
            return Some(FileExcerpt {
                section: number,
                start_line,
                end_line,
                text: section,
                complete_section: true,
            });
        }
        let words = query_terms(query);
        let mut best = (0, 0);
        let mut offset = 0;
        for line in section.split_inclusive('\n') {
            let lower = line.to_lowercase();
            let score = words
                .iter()
                .filter(|word| contains_term(&lower, word))
                .count();
            if score > best.0 {
                // Translate through original characters: lowercasing can expand a
                // Unicode character, so folded offsets cannot index source bytes.
                let focus = line
                    .char_indices()
                    .find(|(at, ch)| {
                        if !ch.is_alphanumeric()
                            || (*at > 0
                                && line[..*at]
                                    .chars()
                                    .next_back()
                                    .is_some_and(char::is_alphanumeric))
                        {
                            return false;
                        }
                        let token: String = line[*at..]
                            .chars()
                            .take_while(|ch| ch.is_alphanumeric())
                            .flat_map(char::to_lowercase)
                            .collect();
                        words.contains(&token)
                    })
                    .map_or(0, |(at, _)| at);
                best = (score, offset + focus);
            }
            offset += line.len();
        }
        let boundaries: Vec<_> = section
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(section.len()))
            .collect();
        let focus = boundaries.partition_point(|offset| *offset < best.1);
        let start = focus.saturating_sub(char_budget / 3).min(
            boundaries
                .len()
                .saturating_sub(char_budget.saturating_add(1)),
        );
        let end = start.saturating_add(char_budget).min(boundaries.len() - 1);
        let excerpt = &section[boundaries[start]..boundaries[end]];
        let start_line = self.section_lines(number)?.0
            + section[..boundaries[start]]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
        let end_line = start_line + excerpt.bytes().filter(|byte| *byte == b'\n').count()
            - usize::from(excerpt.ends_with('\n'));
        Some(FileExcerpt {
            section: number,
            start_line,
            end_line,
            text: excerpt,
            complete_section: start == 0 && end == boundaries.len() - 1,
        })
    }
}

/// A cited slice of extracted text. Debug intentionally omits its private content.
pub struct FileExcerpt<'a> {
    pub section: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub text: &'a str,
    pub complete_section: bool,
}

impl std::fmt::Debug for FileExcerpt<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileExcerpt")
            .field("section", &self.section)
            .field("start_line", &self.start_line)
            .field("end_line", &self.end_line)
            .finish_non_exhaustive()
    }
}

fn query_terms(query: &str) -> BTreeSet<String> {
    let query: String = query.chars().take(4096).collect();
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(64)
        .map(str::to_lowercase)
        .collect()
}

fn contains_term(text: &str, term: &str) -> bool {
    text.match_indices(term).any(|(offset, _)| {
        !text[..offset]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
            && !text[offset + term.len()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
    })
}

/// Selects useful evidence before optional opening context. A single-section
/// budget always prefers a match; a broad/no-match question falls back to the opening.
pub fn relevant_sections(text: &str, query: &str, limit: usize) -> Vec<(usize, String)> {
    let index = FileSearchIndex::new(text);
    let numbers = index.initial_sections(query, limit);
    numbers
        .into_iter()
        .filter_map(|number| {
            index
                .section(text, number)
                .map(|section| (number, section.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_needs_known_inputs_and_supported_transport() {
        let mut model = ModelInfo {
            id: "example".into(),
            ..ModelInfo::default()
        };
        assert!(!ModelPurpose::Images.accepts(&model, ProviderKind::OpenRouter));
        model.input_modalities = vec!["image".into(), "video".into(), "audio".into()];
        assert!(ModelPurpose::Images.accepts(&model, ProviderKind::Anthropic));
        assert!(ModelPurpose::Video.accepts(&model, ProviderKind::OpenRouter));
        assert!(!ModelPurpose::Video.accepts(&model, ProviderKind::Ollama));
        assert!(!ModelPurpose::Audio.accepts(&model, ProviderKind::Anthropic));
        model.output_modalities = vec!["image".into()];
        assert!(!ModelPurpose::Images.accepts(&model, ProviderKind::OpenRouter));
        for purpose in ModelPurpose::ALL {
            assert_ne!(purpose.label(), "");
            assert_ne!(purpose.modality(), "");
        }
    }

    #[test]
    fn small_excerpt_budgets_find_relevant_sections_and_short_identifiers() {
        let text = format!(
            "{}invoice id = 93; fn go() {{}}",
            "introduction ".repeat(1000)
        );
        let index = FileSearchIndex::new(&text);
        for query in ["invoice", "93", "id", "go"] {
            let numbers = index.search(query, 1);
            assert_eq!(numbers.len(), 1);
            assert!(
                index
                    .section(&text, numbers[0])
                    .unwrap()
                    .contains("invoice id = 93"),
                "{query}"
            );
            let excerpts = relevant_sections(&text, query, 1);
            assert!(excerpts[0].1.contains("invoice id = 93"));
        }
        assert_eq!(index.search("nonexistent", 3), Vec::<usize>::new());
        assert_eq!(
            FileSearchIndex::new("193 idleness ongoing").search("93 id go", 3),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn excerpts_keep_unicode_and_original_line_numbers_within_the_character_budget() {
        let text = format!(
            "{}İstanbul invoice 93\r\nlast line\n",
            "你好 introductory line\r\n".repeat(900)
        );
        let index = FileSearchIndex::new(&text);
        let number = index.search("İstanbul", 1)[0];
        let excerpt = index
            .excerpt(&text, number, "İstanbul invoice", 60)
            .unwrap();
        assert!(excerpt.text.contains("İstanbul invoice 93"));
        assert!(excerpt.text.chars().count() <= 60);
        assert!(excerpt.start_line <= 901 && excerpt.end_line >= 901);
        assert!(!excerpt.complete_section);
        assert!(!format!("{excerpt:?}").contains("invoice"));
        let whole = index.excerpt(&text, number, "invoice", usize::MAX).unwrap();
        assert!(whole.complete_section);
        assert_eq!(
            (whole.start_line, whole.end_line),
            index.section_lines(number).unwrap()
        );
        assert!(index.excerpt(&text, number, "invoice", 0).is_none());
        assert!(index.excerpt(&text, usize::MAX, "invoice", 50).is_none());
        assert!(index.section_lines(0).is_none());
        assert_eq!(index.initial_sections("missing", 3), vec![1, 2, 3]);
        assert_eq!(index.initial_sections("invoice", 0), Vec::<usize>::new());
    }

    #[test]
    fn rarer_terms_outweigh_repeated_boilerplate_and_ties_are_stable() {
        let mut text = "common ".repeat(2400);
        text.push_str("special_identifier 93");
        let index = FileSearchIndex::new(&text);
        let hits = index.search("common special_identifier", index.len());
        assert!(
            index
                .section(&text, hits[0])
                .unwrap()
                .contains("special_identifier")
        );
        assert_eq!(
            index.search("common", index.len()),
            (1..=index.len()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_index_reuses_unicode_boundaries_and_handles_missing_sections() {
        let text = format!("{}invoice 93", "你好 ".repeat(6000));
        let index = FileSearchIndex::new(&text);
        assert!(!index.is_empty());
        assert_eq!(index.len(), file_sections(&text).len());
        let numbers = index.search("invoice", 2);
        assert_eq!(numbers.len(), 1);
        assert!(
            index
                .section(&text, numbers[0])
                .unwrap()
                .contains("invoice 93")
        );
        assert!(index.section(&text, 0).is_none());
        assert!(index.section(&text, usize::MAX).is_none());
        assert!(index.section("", 1).is_none());
        assert_eq!(index.search("invoice", 0), Vec::<usize>::new());
        assert!(FileSearchIndex::new("").is_empty());
    }

    #[test]
    fn retrieval_finds_material_beyond_the_opening_without_splitting_unicode() {
        let text = format!("{} unicorn invoice 93", "你好 ".repeat(6_000));
        let sections = file_sections(&text);
        assert_eq!(sections.concat(), text);
        let selected = relevant_sections(&text, "Find the unicorn invoice", 2);
        assert_eq!(selected.len(), 2);
        assert!(selected[0].1.contains("invoice 93"));
        assert_eq!(file_sections(""), Vec::<&str>::new());
        assert_eq!(
            relevant_sections(&text, "hello", 0),
            Vec::<(usize, String)>::new()
        );
    }

    #[test]
    fn pending_identity_survives_errors() {
        let state = AttachmentState::Failed {
            id: "one".into(),
            name: "file.txt".into(),
            message: "Retry".into(),
        };
        assert_eq!(state.id(), "one");
        assert_eq!(state.name(), "file.txt");
    }
    fn attachment() -> Attachment {
        Attachment {
            id: "one".into(),
            name: "picture.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 12,
            purpose: ModelPurpose::Images,
            sections: 1,
            note: None,
            thumbnail: Vec::new(),
        }
    }

    #[test]
    fn draft_retains_identity_limits_files_and_ignores_removed_completions() {
        let mut draft = AttachmentDraft::default();
        assert!(draft.begin("one".into(), "picture.png".into()));
        assert!(draft.ready().is_none());
        let prepared = FilePrepared {
            session_id: "chat".into(),
            id: "one".into(),
            result: Ok(attachment()),
        };
        assert!(draft.apply(&prepared));
        assert_eq!(draft.ready().unwrap(), vec![attachment()]);
        draft.remove("one");
        assert!(!draft.apply(&prepared));
        for index in 0..MAX_ATTACHMENTS {
            assert!(draft.begin(index.to_string(), "file.txt".into()));
        }
        assert!(!draft.begin("extra".into(), "file.txt".into()));
        assert!(draft.notice.is_some());
    }

    #[test]
    fn media_setup_requires_an_explicit_choice_and_preserves_the_chat_choice() {
        use crate::models::*;
        let mut connection = ModelConnection::new("test", ProviderKind::OpenRouter);
        connection.has_credential = true;
        let model = ModelInfo {
            id: "reader".into(),
            input_modalities: vec!["image".into(), "text".into(), "audio".into()],
            output_modalities: vec!["text".into()],
            ..ModelInfo::default()
        };
        let key = model.key("test");
        let mut state = ModelsState {
            loaded: true,
            connections: vec![connection],
            ..ModelsState::default()
        };
        state.catalogs.insert(
            "test".into(),
            CatalogState {
                snapshot: CatalogSnapshot {
                    models: vec![model],
                    ..CatalogSnapshot::default()
                },
                ..CatalogState::default()
            },
        );
        state.preferences.active = Some(key.clone());
        assert_eq!(
            state.model_for_files(&[attachment()]).unwrap_err().purpose,
            ModelPurpose::Images
        );
        state
            .preferences
            .purposes
            .insert(ModelPurpose::Images, key.clone());
        assert_eq!(state.model_for_files(&[attachment()]).unwrap(), key);
        let mut recording = attachment();
        recording.purpose = ModelPurpose::Audio;
        recording.mime_type = "audio/mpeg".into();
        state.preferences.purposes.remove(&ModelPurpose::Images);
        state
            .preferences
            .purposes
            .insert(ModelPurpose::Audio, key.clone());
        assert_eq!(
            state
                .model_for_files(&[attachment(), recording.clone()])
                .unwrap_err()
                .purpose,
            ModelPurpose::Images
        );
        state
            .preferences
            .purposes
            .insert(ModelPurpose::Images, key.clone());
        assert_eq!(
            state.model_for_files(&[attachment(), recording]).unwrap(),
            key
        );
        state.connections[0].has_credential = false;
        assert!(
            state
                .model_for_files(&[attachment()])
                .unwrap_err()
                .message
                .contains("key")
        );
        assert_eq!(state.preferences.active, Some(key));
        let legacy: ModelPreferences =
            serde_json::from_str(r#"{"favorites":[],"active":null}"#).unwrap();
        assert!(legacy.purposes.is_empty());
    }
}
