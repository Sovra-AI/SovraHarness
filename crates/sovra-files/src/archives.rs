//! Bounded archive traversal in memory. Members never become filesystem paths.

use crate::{MAX_DECODED_BYTES, detected_mime, extract_single_document};
use sovra_core::files::{FileError, MAX_DOCUMENT_BYTES};
use std::io::{Cursor, Read};

pub(super) fn is_archive(mime: &str) -> bool {
    matches!(
        mime,
        "application/zip"
            | "application/hwp+zip"
            | "application/x-tar"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-xz"
            | "application/x-lzma"
            | "application/zstd"
            | "application/x-bzip2"
    )
}

pub(super) fn extract(name: &str, mime: &str, bytes: &[u8]) -> Result<Option<String>, FileError> {
    let mut budget = Budget {
        remaining: MAX_DECODED_BYTES,
        entries: 0,
        output: String::new(),
    };
    match visit(name, mime, bytes, 0, &mut budget) {
        Ok(()) => {
            if budget.output.is_empty() {
                budget
                    .output
                    .push_str("This archive is empty or contains no regular files.");
            }
            Ok(Some(budget.output))
        }
        Err(Failure::Unreadable) => Ok(None),
        Err(Failure::Limit(error)) => Err(error),
    }
}

#[derive(Debug)]
enum Failure {
    Unreadable,
    Limit(FileError),
}

struct Budget {
    remaining: u64,
    entries: usize,
    output: String,
}

impl Budget {
    fn read(&mut self, reader: impl Read) -> Result<Vec<u8>, Failure> {
        self.entries += 1;
        if self.entries > 100 {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        reader
            .take(self.remaining + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unreadable())?;
        if bytes.len() as u64 > self.remaining {
            return Err(too_large());
        }
        self.remaining -= bytes.len() as u64;
        Ok(bytes)
    }

    fn append(&mut self, name: &str, text: &str) -> Result<(), Failure> {
        if self
            .output
            .len()
            .saturating_add(text.len())
            .saturating_add(name.len())
            .saturating_add(20)
            > MAX_DOCUMENT_BYTES
        {
            return Err(too_large());
        }
        self.output.push_str("\n\nFile inside archive: ");
        self.output.push_str(name);
        self.output.push('\n');
        self.output.push_str(text);
        Ok(())
    }
}

fn visit(
    name: &str,
    mime: &str,
    bytes: &[u8],
    depth: usize,
    budget: &mut Budget,
) -> Result<(), Failure> {
    if depth > 2 {
        return Err(Failure::Limit(FileError("This archive contains too many layers of archives. Unpack it and add the documents directly.".into())));
    }
    match mime {
        "application/zip" | "application/hwp+zip" => {
            if mime == "application/hwp+zip" {
                budget.append(name, "HWPX package: XML source and readable members follow. This is not rendered page layout; embedded media is not decoded.")?;
            }
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| unreadable())?;
            if archive.len().saturating_add(budget.entries) > 100 {
                return Err(too_large());
            }
            for index in 0..archive.len() {
                let Ok(entry) = archive.by_index(index) else {
                    budget.append(&format!("Archive entry {}", index + 1), "[This entry is encrypted, damaged, or uses a compression method that could not be decoded. Other readable entries are included.]")?;
                    continue;
                };
                if entry.is_dir() {
                    continue;
                }
                if entry.size() > budget.remaining {
                    return Err(too_large());
                }
                let name = entry.name().map_err(|_| unreadable())?.into_owned();
                let content = budget.read(entry)?;
                member(&name, &content, depth, budget)?;
            }
            Ok(())
        }
        "application/x-tar" => {
            let mut archive = tar::Archive::new(Cursor::new(bytes));
            for entry in archive.entries().map_err(|_| unreadable())? {
                let entry = entry.map_err(|_| unreadable())?;
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                if entry.size() > budget.remaining {
                    return Err(too_large());
                }
                let name = entry
                    .path()
                    .map_err(|_| unreadable())?
                    .to_string_lossy()
                    .into_owned();
                let content = budget.read(entry)?;
                member(&name, &content, depth, budget)?;
            }
            Ok(())
        }
        "application/gzip" | "application/x-gzip" => {
            let content = budget.read(flate2::read::MultiGzDecoder::new(bytes))?;
            let inner = name.strip_suffix(".gz").unwrap_or("contents.tar");
            member(inner, &content, depth, budget)
        }
        "application/x-xz" => {
            let reader = lzma_rust2::XzReader::new_mem_limit(bytes, true, 64 * 1024);
            compressed_member(name, &[".xz", ".txz"], reader, depth, budget)
        }
        "application/x-lzma" => {
            let reader = lzma_rust2::LzmaReader::new_mem_limit(bytes, 64 * 1024, None)
                .map_err(|_| unreadable())?;
            compressed_member(name, &[".lzma"], reader, depth, budget)
        }
        "application/x-bzip2" => {
            budget.append(name, "[BZip2: only the first compressed stream is decoded. If this file contains concatenated streams, later streams are not included.]")?;
            compressed_member(
                name,
                &[".bz2", ".tbz2", ".tbz"],
                bzip2_rs::DecoderReader::new(bytes),
                depth,
                budget,
            )
        }
        "application/zstd" => {
            let content = zstandard(bytes, budget)?;
            let inner = [".zst", ".zstd", ".tzst"]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or("contents");
            member(inner, &content, depth, budget)
        }
        _ => Err(unreadable()),
    }
}

fn member(name: &str, bytes: &[u8], depth: usize, budget: &mut Budget) -> Result<(), Failure> {
    let mime = detected_mime(name, bytes);
    if is_archive(&mime) {
        return visit(name, &mime, bytes, depth + 1, budget);
    }
    if mime.starts_with("image/") || mime.starts_with("audio/") || mime.starts_with("video/") {
        if let Some(text) = crate::text::decode(bytes).map_err(Failure::Limit)? {
            return budget.append(name, &text.content);
        }
        return budget.append(name, "[Media was not read. Attach this file separately to use its picture, audio, or video model.]");
    }
    match extract_single_document(name, &mime, bytes) {
        Ok(Some(text)) if !text.trim().is_empty() => budget.append(name, &text),
        Ok(_) => budget.append(name, &crate::inspection::describe(bytes, &mime)),
        Err(error) => Err(Failure::Limit(error)),
    }
}

fn compressed_member(
    name: &str,
    suffixes: &[&str],
    reader: impl Read,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let bytes = budget.read(reader)?;
    let inner = suffixes
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or("contents");
    member(inner, &bytes, depth, budget)
}

fn zstandard(mut bytes: &[u8], budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    let mut output = Vec::new();
    while !bytes.is_empty() {
        // Skippable frames carry metadata, not decoded bytes. Validate their size
        // before slicing, and charge them against the member-count budget too.
        if bytes.len() >= 8
            && bytes[1..4] == [0x2a, 0x4d, 0x18]
            && (0x50..=0x5f).contains(&bytes[0])
        {
            budget.entries += 1;
            if budget.entries > 100 {
                return Err(too_large());
            }
            let length =
                u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| unreadable())?) as usize;
            bytes = bytes
                .get(8..)
                .and_then(|tail| tail.get(length..))
                .ok_or_else(unreadable)?;
            continue;
        }
        let mut reader =
            ruzstd::decoding::StreamingDecoder::new_with_max_window_size(bytes, MAX_DECODED_BYTES)
                .map_err(|_| unreadable())?;
        output.extend(budget.read(&mut reader)?);
        let remaining = reader.into_inner();
        if remaining.len() >= bytes.len() {
            return Err(unreadable());
        }
        bytes = remaining;
    }
    Ok(output)
}

fn unreadable() -> Failure {
    Failure::Unreadable
}
fn too_large() -> Failure {
    Failure::Limit(FileError("This archive expands into too much content. Try adding fewer than 100 files, under 64 MB unpacked.".into()))
}
