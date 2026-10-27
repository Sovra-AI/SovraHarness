//! Bounded, non-executing evidence for formats without a complete decoder.

use file_format::FileFormat;
use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

const MAX_STRINGS: usize = 1024;
const MAX_STRING_BYTES: usize = 512;

pub(super) fn describe(bytes: &[u8], mime: &str) -> String {
    let format = FileFormat::from_bytes(bytes);
    let mut output = format!(
        "FILE INSPECTION — CONTENT NOT FULLY DECODED\nDetected format: {}\nMIME: {mime}\nSize: {} bytes\nSHA-256: {}\n\nThis is a binary inspection, not a decoded document. Strings below may be unrelated fragments or metadata. Do not infer missing content from them. The original bytes are available through read_attachment byte ranges and byte searches.\n\nHeader (first 128 bytes, hexadecimal):\n",
        format.name(),
        bytes.len(),
        fingerprint(bytes)
    );
    for byte in bytes.iter().take(128) {
        let _ = write!(output, "{byte:02x} ");
    }
    output.push_str("\n\nPrintable byte strings (offsets in the original file):\n");
    let mut count = 0;
    let mut start = 0;
    while start < bytes.len() && count < MAX_STRINGS {
        if !printable(bytes[start]) {
            start += 1;
            continue;
        }
        let end = bytes[start..]
            .iter()
            .position(|b| !printable(*b))
            .map_or(bytes.len(), |len| start + len);
        if end - start >= 6 {
            let shown = (end - start).min(MAX_STRING_BYTES);
            let string = String::from_utf8_lossy(&bytes[start..start + shown]);
            let _ = writeln!(
                output,
                "0x{start:08x}: {string}{}",
                if shown < end - start {
                    " [string shortened]"
                } else {
                    ""
                }
            );
            count += 1;
        }
        start = end;
    }
    if start < bytes.len() {
        output.push_str(
            "[String inventory limited; use byte search for material beyond this inventory.]\n",
        );
    }
    output.push_str("\nUTF-16 ASCII strings (byte offsets):\n");
    let mut wide_count = 0;
    for little in [true, false] {
        let mut offset = 0;
        while offset + 1 < bytes.len() && wide_count < MAX_STRINGS {
            let start = offset;
            let mut string = String::new();
            while offset + 1 < bytes.len() {
                let (character, zero) = if little {
                    (bytes[offset], bytes[offset + 1])
                } else {
                    (bytes[offset + 1], bytes[offset])
                };
                if zero != 0 || !printable(character) {
                    break;
                }
                if string.len() < MAX_STRING_BYTES {
                    string.push(char::from(character));
                }
                offset += 2;
            }
            if offset - start >= 12 {
                let _ = writeln!(
                    output,
                    "0x{start:08x} (UTF-16{}): {string}",
                    if little { "LE" } else { "BE" }
                );
                wide_count += 1;
            }
            offset += 1;
        }
    }
    if count == 0 && wide_count == 0 {
        output.push_str("No readable strings found. A format-specific decoder or decryption key is needed to recover semantic content.\n");
    }
    executable_metadata(bytes, &mut output);
    output
}

fn executable_metadata(bytes: &[u8], output: &mut String) {
    let Ok(object) = object::File::parse(bytes) else {
        return;
    };
    let _ = writeln!(
        output,
        "\nExecutable/object metadata (never executed):\nArchitecture: {:?}\nKind: {:?}\nEntry point: 0x{:x}",
        object.architecture(),
        object.kind(),
        object.entry()
    );
    for section in object.sections().take(256) {
        let _ = writeln!(
            output,
            "Section {:?}: {} bytes at 0x{:x}",
            section
                .name()
                .unwrap_or("[unnamed]")
                .chars()
                .take(256)
                .collect::<String>(),
            section.size(),
            section.address()
        );
    }
    let baseline = output.len();
    for symbol in object.symbols().chain(object.dynamic_symbols()).take(1024) {
        if output.len() - baseline > 64 * 1024 {
            output.push_str("[Symbol inventory limited]\n");
            break;
        }
        let name = symbol.name().unwrap_or("[unnamed]");
        let _ = writeln!(
            output,
            "Symbol 0x{:x}: {}",
            symbol.address(),
            name.chars().take(256).collect::<String>()
        );
    }
}

fn fingerprint(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn printable(byte: u8) -> bool {
    byte.is_ascii_graphic() || byte == b' ' || byte == b'\t'
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_labels_fragments_and_preserves_offsets() {
        let data = b"\0\xffImportant invoice total 93\0\x88";
        let report = describe(data, "application/octet-stream");
        assert!(report.contains("CONTENT NOT FULLY DECODED"));
        assert!(report.contains("0x00000002: Important invoice total 93"));
        assert!(report.contains(&fingerprint(data)));
    }
    #[test]
    fn limits_report_size_for_adversarial_long_strings() {
        let bytes = vec![b'A'; 20 * 1024 * 1024];
        assert!(describe(&bytes, "application/octet-stream").len() < 4096);
    }
}
