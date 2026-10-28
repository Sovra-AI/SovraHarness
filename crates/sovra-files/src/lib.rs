//! Local attachment storage and document/media decoding behind the core `FileStore` port.
//! The composition root supplies a private directory; runtime owns every blocking operation.

mod archives;
mod clustering;
mod database;
mod detection;
mod index;
mod inspection;
mod media_duration;
mod property_list;
mod text;
mod vector;

use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read as _, Write as _};
use std::path::{Path, PathBuf};

use sovra_core::file_index::DocumentBudget;

use image::{ImageDecoder as _, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use sovra_core::files::{
    Attachment, FileContent, FileError, FileStore, MAX_DOCUMENT_BYTES, MAX_FILE_BYTES, MediaPart,
    ModelPurpose, file_sections,
};

const MAX_MEDIA_PARTS: usize = 12;
pub use clustering::RaptorClusterer;
pub use index::LocalFileIndex;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct LocalFileStore {
    directory: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    attachment: Attachment,
    media_types: Vec<String>,
    #[serde(default)]
    has_raw: bool,
}

impl LocalFileStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn location(&self, id: &str) -> Result<PathBuf, FileError> {
        if id.is_empty()
            || id.len() > 120
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(FileError(
                "This attachment reference is invalid. Please add the file again.".into(),
            ));
        }
        Ok(self.directory.join(id))
    }

    fn save(&self, content: &FileContent) -> Result<(), FileError> {
        private_directory(&self.directory)?;
        let destination = self.location(&content.attachment.id)?;
        let staging = self
            .directory
            .join(format!(".{}-preparing", content.attachment.id));
        fs::create_dir(&staging).map_err(storage_error)?;
        let result = (|| {
            set_directory_permissions(&staging)?;
            let manifest = Manifest {
                attachment: content.attachment.clone(),
                has_raw: content.raw.is_some(),
                media_types: content
                    .media
                    .iter()
                    .map(|part| part.mime_type.clone())
                    .collect(),
            };
            let metadata = serde_json::to_vec(&manifest)
                .map_err(|_| FileError("Could not save the file details.".into()))?;
            private_write(&staging.join("manifest.json"), &metadata)?;
            private_write(&staging.join("text.txt"), content.text.as_bytes())?;
            if let Some(raw) = &content.raw {
                private_write(&staging.join("source.bin"), raw)?;
            }
            for (index, part) in content.media.iter().enumerate() {
                private_write(&staging.join(format!("media-{index}")), &part.data)?;
            }
            // A completed directory appears atomically, so readers never observe a partial upload.
            if destination.exists() {
                return Err(FileError("This file has already been added.".into()));
            }
            fs::rename(&staging, destination).map_err(storage_error)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }
}

impl FileStore for LocalFileStore {
    fn prepare(&self, id: &str, path: &Path) -> Result<Attachment, FileError> {
        self.location(id)?;
        if !fs::metadata(path)
            .map_err(|_| FileError("I couldn't find this file. Please choose it again.".into()))?
            .is_file()
        {
            return Err(FileError(
                "Choose a file, or put a folder into a ZIP file first.".into(),
            ));
        }
        let file = File::open(path).map_err(|_| FileError("I couldn't open this file. Check that it is still on your computer and try adding it again.".into()))?;
        if !file.metadata().map_err(storage_error)?.is_file() {
            return Err(FileError(
                "Choose a file, or put a folder into a ZIP file first.".into(),
            ));
        }
        let bytes = bounded_read(file, MAX_FILE_BYTES)?;
        let name = path.file_name().map_or_else(
            || "Attachment".into(),
            |name| name.to_string_lossy().into_owned(),
        );
        let content = prepare_bytes(id, &name, &bytes)?;
        self.save(&content)?;
        Ok(content.attachment)
    }

    fn load(&self, id: &str) -> Result<FileContent, FileError> {
        let directory = self.location(id)?;
        let metadata = bounded_read(
            File::open(directory.join("manifest.json")).map_err(missing_file)?,
            512 * 1024,
        )?;
        let manifest: Manifest = serde_json::from_slice(&metadata).map_err(|_| {
            FileError("The saved file details could not be read. Please add the file again.".into())
        })?;
        if manifest.attachment.id != id || manifest.media_types.len() > MAX_MEDIA_PARTS {
            return Err(FileError(
                "The saved file details are invalid. Please add the file again.".into(),
            ));
        }
        let text = bounded_read(
            File::open(directory.join("text.txt")).map_err(missing_file)?,
            MAX_DOCUMENT_BYTES as u64,
        )?;
        let text = String::from_utf8(text).map_err(|_| {
            FileError("The saved text could not be read. Please add the file again.".into())
        })?;
        let mut media = Vec::new();
        let mut remaining = MAX_FILE_BYTES;
        for (index, mime_type) in manifest.media_types.into_iter().enumerate() {
            let data = bounded_read(
                File::open(directory.join(format!("media-{index}"))).map_err(missing_file)?,
                remaining,
            )?;
            remaining = remaining.saturating_sub(data.len() as u64);
            let duration_millis = media_duration::milliseconds(&mime_type, &data);
            media.push(MediaPart {
                mime_type,
                data,
                duration_millis,
            });
        }
        let raw = if manifest.has_raw {
            Some(
                bounded_read(
                    File::open(directory.join("source.bin")).map_err(missing_file)?,
                    MAX_FILE_BYTES,
                )?
                .into(),
            )
        } else {
            None
        };
        let mut attachment = manifest.attachment;
        note_recording_duration(&mut attachment, &media);
        Ok(FileContent {
            raw,
            attachment,
            text,
            media,
        })
    }

    fn discard(&self, id: &str) -> Result<(), FileError> {
        let directory = self.location(id)?;
        match fs::remove_dir_all(directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }
}

fn prepare_bytes(id: &str, name: &str, bytes: &[u8]) -> Result<FileContent, FileError> {
    prepare_for(id, name, bytes, ReadMode::Attachment, None).map(|prepared| prepared.content)
}

fn prepare_indexed_bytes(
    id: &str,
    name: &str,
    bytes: &[u8],
    budget: DocumentBudget,
    observer: Option<&dyn sovra_core::file_index::ScanObserver>,
) -> Result<PreparedFile, FileError> {
    prepare_for(id, name, bytes, ReadMode::Index(budget), observer)
}

struct PreparedFile {
    content: FileContent,
    partial: bool,
}

fn prepared(mut content: FileContent, mut warnings: Vec<String>) -> PreparedFile {
    warnings.sort();
    warnings.dedup();
    let partial = !warnings.is_empty();
    if partial {
        let note = content.attachment.note.get_or_insert_with(String::new);
        if !note.is_empty() {
            note.push('\n');
        }
        note.push_str("Reading may be incomplete. Check the original before relying on this file. Reader details:\n");
        note.push_str(&warnings.join("\n"));
    }
    PreparedFile { content, partial }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    Attachment,
    Index(DocumentBudget),
}

impl ReadMode {
    fn budget_error(self) -> FileError {
        FileError(match self {
            Self::Attachment => {
                "Reading this file exceeded the safety limits. Split it into smaller parts.".into()
            }
            Self::Index(budget) => format!(
                "Reading this document exceeded its decoding or expansion budget. {} Choose a larger Document reading budget in Folder settings, or split the document. Format nesting and decompression protections still apply.",
                budget.details()
            ),
        })
    }
    fn indexed(self) -> bool {
        matches!(self, Self::Index(_))
    }
    fn source_bytes(self) -> u64 {
        match self {
            Self::Attachment => MAX_FILE_BYTES,
            Self::Index(budget) => budget.source_bytes(),
        }
    }
    fn text_bytes(self) -> usize {
        match self {
            Self::Attachment => MAX_DOCUMENT_BYTES,
            Self::Index(budget) => budget.text_bytes(),
        }
    }
    fn expanded_bytes(self) -> u64 {
        match self {
            Self::Attachment => MAX_DECODED_BYTES,
            Self::Index(budget) => budget.expanded_bytes(),
        }
    }
}

fn prepare_for(
    id: &str,
    name: &str,
    bytes: &[u8],
    mode: ReadMode,
    observer: Option<&dyn sovra_core::file_index::ScanObserver>,
) -> Result<PreparedFile, FileError> {
    let mut warnings = Vec::new();
    let mime = detected_mime(name, bytes);
    let index_pdf = mode.indexed() && mime == "application/pdf";
    let index_media = mode.indexed()
        && (mime.starts_with("image/") || mime.starts_with("audio/") || mime.starts_with("video/"));
    let mut attachment = Attachment {
        id: id.into(),
        name: name.into(),
        mime_type: mime.clone(),
        size_bytes: bytes.len() as u64,
        purpose: ModelPurpose::Documents,
        sections: 0,
        note: None,
        thumbnail: Vec::new(),
    };
    let mut media = Vec::new();
    if bytes.is_empty() {
        return Ok(prepared(inspected(attachment, bytes, mode), warnings));
    }
    let plain = if mime == "application/pdf"
        || mime == "image/svg+xml"
        || detection::formatted_text(&mime, bytes)
    {
        None
    } else {
        text::decode_with_limit(bytes, mode.text_bytes())?
    };
    let text = if let Some(decoded) = plain {
        if decoded.encoding != "UTF-8" {
            attachment.note = Some(format!(
                "{} text decoded{}.",
                decoded.encoding,
                if decoded.guessed {
                    " (encoding detected)"
                } else {
                    ""
                }
            ));
        }
        if !mime.starts_with("text/") && !mime.contains("json") && !mime.contains("xml") {
            attachment.mime_type = "text/plain".into();
        }
        decoded.content.into_owned()
    } else if index_media {
        // The index stores searchable text, never model media payloads. Avoid
        // decoding pictures or copying recordings that would be discarded.
        attachment.note = Some("No searchable text was extracted from this media file. Its filename is searchable. Attach it in chat to use a suitable picture, audio or video model.".into());
        String::new()
    } else if mime.starts_with("image/") {
        let decoded = if mime == "image/svg+xml" {
            vector::render(bytes).and_then(|png| normalize_image(&png))
        } else {
            normalize_image(bytes)
        };
        let Ok((data, thumbnail)) = decoded else {
            return Ok(prepared(inspected(attachment, bytes, mode), warnings));
        };
        if mime == "image/svg+xml" {
            attachment.note = Some("Drawing rendered as a picture. External resources and nested SVG images are excluded.".into());
        }
        attachment.purpose = ModelPurpose::Images;
        attachment.thumbnail = thumbnail;
        media.push(MediaPart {
            mime_type: "image/png".into(),
            data,
            duration_millis: None,
        });
        if mime == "image/gif" || mime == "image/tiff" {
            attachment.note =
                Some("The first frame is used. Add other frames separately if needed.".into());
        }
        String::new()
    } else if mime.starts_with("audio/") || mime.starts_with("video/") {
        attachment.purpose = if mime.starts_with("audio/") {
            ModelPurpose::Audio
        } else {
            ModelPurpose::Video
        };
        let duration_millis = media_duration::milliseconds(&mime, bytes);
        media.push(MediaPart {
            duration_millis,
            mime_type: mime,
            data: bytes.to_vec(),
        });
        String::new()
    } else {
        let Some(document) = extract_document(name, &mime, bytes, mode, observer, &mut warnings)?
        else {
            return Ok(prepared(inspected(attachment, bytes, mode), warnings));
        };
        if index_pdf {
            attachment.note = Some(if document.trim().is_empty() {
                "No searchable text layer was found. This PDF may be scanned or empty. Use text recognition (OCR) to save a searchable copy, then refresh the folder. Picture content has not been read.".into()
            } else {
                "PDF text layer indexed. Pictures and scanned pages without text are not read; text recognition (OCR) is needed for their contents.".into()
            });
        } else if mime == "application/pdf" && document.trim().chars().count() < 40 {
            let Ok(pdf) = xberg::pdf::PdfRenderSession::open(bytes, None) else {
                return Ok(prepared(inspected(attachment, bytes, mode), warnings));
            };
            let pages = pdf.page_count();
            if pages == 0 || pages > MAX_MEDIA_PARTS {
                return Err(FileError("This PDF needs a picture-reading model. For a scanned document, add up to 12 pages at a time.".into()));
            }
            attachment.purpose = ModelPurpose::Images;
            attachment.note = Some(format!("Scanned PDF · {pages} pages to read as pictures"));
            for index in 0..pages {
                let png = pdf
                    .render_page_to_png(index, Some(144))
                    .map_err(extraction_error)?;
                let (data, thumbnail) = normalize_image(&png)?;
                if index == 0 {
                    attachment.thumbnail = thumbnail;
                }
                media.push(MediaPart {
                    mime_type: "image/png".into(),
                    data,
                    duration_millis: None,
                });
            }
        } else if mime == "application/pdf"
            || mime.contains("officedocument")
            || mime.contains("opendocument")
        {
            attachment.note = Some(
                "Readable text and tables are available. Embedded pictures are not included."
                    .into(),
            );
        }
        document
    };
    if text.len() > mode.text_bytes()
        || media.iter().map(|p| p.data.len() as u64).sum::<u64>() > MAX_FILE_BYTES
    {
        return Err(mode.budget_error());
    }
    if text.trim().is_empty() && media.is_empty() && !(index_pdf || index_media) {
        attachment.note =
            Some("This file contains only whitespace; there is no visible text to read.".into());
    }
    attachment.sections = file_sections(&text).len().max(media.len());
    note_recording_duration(&mut attachment, &media);
    Ok(prepared(
        FileContent {
            raw: None,
            attachment,
            text,
            media,
        },
        warnings,
    ))
}

fn note_recording_duration(attachment: &mut Attachment, media: &[MediaPart]) {
    const NOTE: &str = "The recording length could not be checked locally. The AI service may reject a long recording even when its file is small.";
    if media.iter().any(|part| {
        part.duration_millis.is_none()
            && (part.mime_type.starts_with("audio/") || part.mime_type.starts_with("video/"))
    }) {
        match &mut attachment.note {
            Some(note) if !note.contains(NOTE) => {
                note.push(' ');
                note.push_str(NOTE);
            }
            Some(_) => {}
            None => attachment.note = Some(NOTE.into()),
        }
    }
}

fn detected_mime(name: &str, bytes: &[u8]) -> String {
    detection::mime(name, bytes)
}

fn extract_document(
    name: &str,
    mime: &str,
    bytes: &[u8],
    mode: ReadMode,
    observer: Option<&dyn sovra_core::file_index::ScanObserver>,
    warnings: &mut Vec<String>,
) -> Result<Option<String>, FileError> {
    if archives::is_archive(mime) {
        return archives::extract(name, mime, bytes, mode, observer, warnings);
    }
    if bytes.starts_with(b"PK\x03\x04")
        && !mime.contains("officedocument")
        && !mime.contains("opendocument")
        && !mime.contains("epub")
        && !mime.contains("iwork")
        && !mime.contains("hwp")
    {
        return archives::extract(name, "application/zip", bytes, mode, observer, warnings);
    }
    extract_single_document(name, mime, bytes, mode, observer, warnings)
}

fn extract_single_document(
    name: &str,
    mime: &str,
    bytes: &[u8],
    mode: ReadMode,
    observer: Option<&dyn sovra_core::file_index::ScanObserver>,
    warnings: &mut Vec<String>,
) -> Result<Option<String>, FileError> {
    if bytes.starts_with(b"bplist00") {
        return property_list::extract(bytes, mode, observer);
    }
    if bytes.starts_with(b"SQLite format 3\0") {
        let mut details = Vec::new();
        let result = database::extract(bytes, mode, observer, &mut details);
        warnings.extend(
            details
                .into_iter()
                .map(|detail| format!("{name}: {detail}")),
        );
        if observer.is_some_and(sovra_core::file_index::ScanObserver::cancelled) {
            return Err(FileError("Reading was cancelled.".into()));
        }
        return Ok(match result {
            Ok(text) => Some(text),
            Err(error) => {
                warnings.push(format!("{name}: The database could not be read: {error}. Repair it or export the required tables as CSV or text."));
                None
            }
        });
    }
    if mime != "application/pdf"
        && !detection::formatted_text(mime, bytes)
        && let Some(text) = text::decode_with_limit(bytes, mode.text_bytes())?
    {
        return Ok(Some(text.content.into_owned()));
    }
    let cancel_token = observer.map(|_| xberg::cancellation::CancellationToken::new());
    // Runtime owns the watcher; its guard limits registration to this decode.
    let _cancellation = observer
        .zip(cancel_token.as_ref())
        .and_then(|(observer, token)| {
            let token = token.clone();
            observer.watch_cancellation(std::sync::Arc::new(move || token.cancel()))
        });
    let config = xberg::ExtractionConfig {
        cancel_token,
        use_cache: false,
        disable_ocr: true,
        enable_quality_processing: false,
        max_archive_depth: 2,
        max_embedded_file_bytes: Some(mode.source_bytes()),
        security_limits: Some(xberg::SecurityLimits {
            max_archive_size: usize::try_from(mode.expanded_bytes()).unwrap_or(usize::MAX),
            max_content_size: mode.text_bytes(),
            // Office/ODF/EPUB parts are a document's internal structure. Bound
            // their count by the already-bounded input (each ZIP central record
            // needs at least 46 bytes), not the generic archive's 100-file cap.
            // Expanded bytes, compression ratios, pages and XML depth still
            // have independent checks in the pinned decoder.
            max_files_in_archive: if mime.contains("officedocument")
                || mime.contains("opendocument")
                || mime == "application/epub+zip"
            {
                (bytes.len() / 46).max(1)
            } else {
                100
            },
            // Indexing extracts text without OCR, layout detection or page
            // rasterization. Source, expansion and extracted-text byte budgets
            // still apply; the attachment page count is a separate restriction.
            max_pages: match mode {
                ReadMode::Attachment => Some(500),
                ReadMode::Index(_) => None,
            },
            max_nesting_depth: 64,
            max_xml_depth: 64,
            ..xberg::SecurityLimits::default()
        }),
        ..xberg::ExtractionConfig::default()
    };
    let input = xberg::ExtractInput::from_bytes(bytes.to_vec(), mime, Some(name.into()));
    // Without xberg's tokio-runtime/network features this future is local CPU/IO
    // work. The caller already owns a runtime blocking task; no second runtime.
    let result = match futures::executor::block_on(xberg::extract(input, &config)) {
        Ok(result) => result,
        Err(xberg::XbergError::Security { .. } | xberg::XbergError::Timeout { .. }) => {
            return Err(mode.budget_error());
        }
        Err(xberg::XbergError::Cancelled) => {
            return Err(FileError("Reading was cancelled.".into()));
        }
        Err(error) => {
            warnings.push(format!("{name}: The document reader could not finish: {error}. Any retained text is the original source, not a rendered document."));
            if mode.indexed() {
                warnings.push("If the reader reports a size limit, choose a larger Document reading budget in Folder settings. Other reading problems may require repairing or converting the original file.".into());
            }
            if mime == "application/pdf" {
                return Ok(None);
            }
            return text::decode_with_limit(bytes, mode.text_bytes())
                .map(|text| text.map(|text| text.content.into_owned()));
        }
    };
    let mut parts = Vec::new();
    for document in result.results {
        // The plain-text extractor also accepts binary bytes and replaces invalid
        // UTF-8. It must not bypass our strict decoder just because of a filename.
        if matches!(
            document.metadata.format,
            Some(xberg::types::FormatMetadata::Text(_))
        ) && text::decode_with_limit(bytes, mode.text_bytes())?.is_none()
        {
            warnings.push(format!("{name}: The file could not be decoded as text. Readable byte fragments are not treated as document content."));
            continue;
        }
        warnings.extend(
            document
                .processing_warnings
                .into_iter()
                .map(|warning| format!("{name}: {}", warning.message)),
        );
        parts.push(document.content);
    }
    let text = parts.join("\n\n");
    warnings.extend(
        result
            .errors
            .into_iter()
            .map(|error| format!("{name}: {}", error.message)),
    );
    Ok((mime == "application/pdf" || !text.trim().is_empty()).then_some(text))
}

fn inspected(mut attachment: Attachment, bytes: &[u8], mode: ReadMode) -> FileContent {
    attachment.note = Some(if bytes.is_empty() {
        "This file is empty. There is no content to read.".into()
    } else if mode.indexed() {
        "No readable document text was decoded. Its filename and size remain in the file list. Attach it in chat for format and byte inspection, or export a readable copy from its original app and refresh the folder.".into()
    } else {
        "File inspection available. Its contents have not been fully decoded; I can examine its format, readable fragments, and bytes.".into()
    });
    attachment.purpose = ModelPurpose::Documents;
    if mode.indexed() {
        // The index keeps metadata only; byte inspection is an attachment action.
        return FileContent {
            attachment,
            text: String::new(),
            media: Vec::new(),
            raw: None,
        };
    }
    let text = inspection::describe(bytes, &attachment.mime_type);
    attachment.sections = file_sections(&text).len();
    FileContent {
        attachment,
        text,
        media: Vec::new(),
        raw: Some(bytes.into()),
    }
}

fn normalize_image(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), FileError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| image_error())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| image_error())?;
    let orientation = decoder.orientation().map_err(|_| image_error())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| image_error())?;
    image.apply_orientation(orientation);
    let image = if image.width() > 2048 || image.height() > 2048 {
        image.thumbnail(2048, 2048)
    } else {
        image
    };
    let mut data = Cursor::new(Vec::new());
    image
        .write_to(&mut data, ImageFormat::Png)
        .map_err(|_| image_error())?;
    if image.width() <= 160 && image.height() <= 160 {
        let data = data.into_inner();
        return Ok((data.clone(), data));
    }
    let mut thumbnail = Cursor::new(Vec::new());
    image
        .thumbnail(160, 160)
        .write_to(&mut thumbnail, ImageFormat::Png)
        .map_err(|_| image_error())?;
    Ok((data.into_inner(), thumbnail.into_inner()))
}

fn image_error() -> FileError {
    FileError(
        "I couldn't open this picture. Try saving it as PNG or JPEG, or use a smaller copy.".into(),
    )
}
fn missing_file(_: std::io::Error) -> FileError {
    FileError("This saved attachment is no longer available. Please add the file again.".into())
}
fn storage_error(_: std::io::Error) -> FileError {
    FileError("I couldn't save this attachment on your computer. Check available disk space and try again.".into())
}
fn extraction_error(_: xberg::XbergError) -> FileError {
    FileError("I couldn't read this file. It may be password-protected, damaged, or an unsupported format. Try exporting an unlocked copy as PDF or text.".into())
}

fn bounded_read(file: File, limit: u64) -> Result<Vec<u8>, FileError> {
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(storage_error)?;
    if bytes.len() as u64 > limit {
        return Err(FileError("This file is too large. Please choose a file under 20 MB or split it into smaller parts.".into()));
    }
    Ok(bytes)
}

fn private_directory(path: &Path) -> Result<(), FileError> {
    fs::create_dir_all(path).map_err(storage_error)?;
    set_directory_permissions(path)
}

fn set_directory_permissions(path: &Path) -> Result<(), FileError> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
    }
    Ok(())
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(storage_error)?;
    file.write_all(bytes).map_err(storage_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_detection_beats_a_misleading_picture_extension() {
        assert_eq!(detected_mime("wrong.jpg", b"%PDF-1.4\n"), "application/pdf");
        assert_eq!(detected_mime("notes.rs", b"fn main() {}"), "text/x-rust");
    }

    #[test]
    fn reads_text_and_inspects_binary_without_trusting_extensions() {
        let file = prepare_bytes("file-1", "notes.txt", b"The launch date is November 3.").unwrap();
        assert!(file.text.contains("November 3"));
        assert_eq!(file.attachment.purpose, ModelPurpose::Documents);
        assert!(
            prepare_bytes("file-2", "blob.unknown", &[0, 255, 128])
                .unwrap()
                .raw
                .is_some()
        );
        assert_eq!(
            prepare_bytes("file-3", "photo.png", b"broken")
                .unwrap()
                .text,
            "broken"
        );
    }

    #[test]
    fn images_are_decoded_and_normalized_before_the_model_or_preview_sees_them() {
        let image = image::DynamicImage::new_rgb8(16, 12);
        let mut encoded = Cursor::new(Vec::new());
        image.write_to(&mut encoded, ImageFormat::Bmp).unwrap();
        let result = prepare_bytes("picture", "photo.bmp", encoded.get_ref()).unwrap();
        assert_eq!(result.attachment.purpose, ModelPurpose::Images);
        assert_eq!(result.media[0].mime_type, "image/png");
        assert_ne!(result.attachment.thumbnail, Vec::<u8>::new());
    }

    #[test]
    fn references_cannot_escape_managed_storage() {
        let store = LocalFileStore::new(PathBuf::from("/unused"));
        for id in ["", "../secret", "a/b", "a\\b", ".hidden"] {
            assert!(store.location(id).is_err());
        }
        assert_eq!(
            store.location("file-1").unwrap(),
            PathBuf::from("/unused/file-1")
        );
    }

    #[test]
    fn managed_content_survives_the_original_file_being_removed() {
        let root = std::env::temp_dir().join(format!("sovra-files-test-{}", std::process::id()));
        let source = root.join("source.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&source, "Keep this evidence.").unwrap();
        let store = LocalFileStore::new(root.join("managed"));
        let attachment = store.prepare("persisted-file", &source).unwrap();
        fs::remove_file(&source).unwrap();
        let restored = store.load(&attachment.id).unwrap();
        assert!(restored.text.contains("Keep this evidence."));
        store.discard(&attachment.id).unwrap();
        assert!(store.load(&attachment.id).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod format_tests;
