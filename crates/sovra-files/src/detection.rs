//! Content signatures take precedence; names only refine formats with no useful magic.

use file_format::FileFormat;
use std::io::{Cursor, Read as _};

pub(super) fn mime(name: &str, bytes: &[u8]) -> String {
    let format = FileFormat::from_bytes(bytes);
    if format == FileFormat::Zip {
        return zip_mime(bytes).unwrap_or("application/zip").into();
    }
    let signature = format.media_type();
    if signature == "application/x-cfb" {
        let hint = mime_guess::from_path(name.to_ascii_lowercase()).first_raw();
        if let Some(
            hint @ ("application/msword"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.ms-outlook"
            | "application/x-hwp"),
        ) = hint
        {
            return hint.into();
        }
    }
    if !matches!(
        signature,
        "application/octet-stream" | "text/plain" | "application/x-empty"
    ) {
        // Signature-aware container readers distinguish Office/EPUB from generic ZIP,
        // even when the name is absent or deliberately misleading.
        return signature.into();
    }
    if let Some(kind) = infer::get(bytes) {
        return kind.mime_type().into();
    }
    if name.to_ascii_lowercase().ends_with(".ipynb") {
        return "application/x-ipynb+json".into();
    }
    mime_guess::from_path(name.to_ascii_lowercase())
        .first_raw()
        .unwrap_or("application/octet-stream")
        .into()
}

pub(super) fn formatted_text(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "message/rfc822" => bytes
            .windows(8)
            .any(|part| part.eq_ignore_ascii_case(b"subject:")),
        "text/rtf" | "application/rtf" => bytes.starts_with(b"{\\rtf"),
        "application/x-ipynb+json" | "application/x-jupyter-notebook+json" => true,
        _ => false,
    }
}

// file-format's optional ZIP reader allocates the declared size of a `mimetype`
// entry before reading it. Own this small probe so a hostile header cannot request
// gigabytes during detection, before extraction's expansion budget applies.
fn zip_mime(bytes: &[u8]) -> Option<&'static str> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    if archive.len() > 100 {
        return None;
    }
    for name in archive.file_names().filter_map(Result::ok) {
        if name.starts_with("word/") {
            return Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document");
        }
        if name.starts_with("xl/") {
            return Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
        }
        if name.starts_with("ppt/") {
            return Some(
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            );
        }
    }
    let mut entry = archive.by_name("mimetype").ok()?;
    if entry.size() > 256 {
        return None;
    }
    let mut mime = String::new();
    entry.by_ref().take(257).read_to_string(&mut mime).ok()?;
    match mime.trim() {
        "application/epub+zip" => Some("application/epub+zip"),
        "application/vnd.oasis.opendocument.text" => {
            Some("application/vnd.oasis.opendocument.text")
        }
        "application/vnd.oasis.opendocument.spreadsheet" => {
            Some("application/vnd.oasis.opendocument.spreadsheet")
        }
        "application/vnd.oasis.opendocument.presentation" => {
            Some("application/vnd.oasis.opendocument.presentation")
        }
        "application/hwp+zip" => Some("application/hwp+zip"),
        _ => None,
    }
}
