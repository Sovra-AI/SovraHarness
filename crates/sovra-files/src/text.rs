//! Extension-independent text decoding. UTF-8 is borrowed; only transcoding allocates.

use std::borrow::Cow;

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, ISO_2022_JP, UTF_16BE, UTF_16LE};
use sovra_core::files::{FileError, MAX_DOCUMENT_BYTES};

pub(super) struct Text<'a> {
    pub(super) content: Cow<'a, str>,
    pub(super) encoding: &'static str,
    pub(super) guessed: bool,
}

pub(super) fn decode(bytes: &[u8]) -> Result<Option<Text<'_>>, FileError> {
    let decoded = if bytes.starts_with(&[0xff, 0xfe, 0, 0]) {
        utf32(&bytes[4..], true).map(|text| Text {
            content: Cow::Owned(text),
            encoding: "UTF-32LE",
            guessed: false,
        })
    } else if bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        utf32(&bytes[4..], false).map(|text| Text {
            content: Cow::Owned(text),
            encoding: "UTF-32BE",
            guessed: false,
        })
    } else if let Some((encoding, bom)) = Encoding::for_bom(bytes) {
        transcode(&bytes[bom..], encoding, false)
    } else if bytes
        .windows(3)
        .any(|part| matches!(part, b"\x1b$B" | b"\x1b$@" | b"\x1b(J"))
    {
        transcode(bytes, ISO_2022_JP, true)
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        if printable(text) {
            Some(Text {
                content: Cow::Borrowed(text),
                encoding: "UTF-8",
                guessed: false,
            })
        } else {
            wide_text(bytes)
        }
    } else if let Some(wide) = wide_text(bytes) {
        Some(wide)
    } else if bytes.iter().any(|byte| forbidden_byte(*byte)) {
        None
    } else {
        // A bounded sample chooses the encoding; the full input is then decoded
        // strictly. We never turn arbitrary undecodable bytes into replacement text.
        let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
        detector.feed(&bytes[..bytes.len().min(64 * 1024)], true);
        transcode(bytes, detector.guess(None, Utf8Detection::Allow), true)
    };
    if decoded
        .as_ref()
        .is_some_and(|text| text.content.len() > MAX_DOCUMENT_BYTES)
    {
        return Err(FileError("This file contains more than 4 MB of text. Split it into smaller parts so I can keep all of it.".into()));
    }
    Ok(decoded.filter(|text| printable(&text.content)))
}

fn transcode<'a>(bytes: &'a [u8], encoding: &'static Encoding, guessed: bool) -> Option<Text<'a>> {
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .filter(|text| printable(text))
        .map(|content| Text {
            content,
            encoding: encoding.name(),
            guessed,
        })
}

fn wide_text(bytes: &[u8]) -> Option<Text<'_>> {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let pairs = bytes.as_chunks::<2>().0.iter().take(2048);
    let count = pairs.len();
    let (even, odd) = pairs.fold((0, 0), |(a, b), pair| {
        (a + usize::from(pair[0] == 0), b + usize::from(pair[1] == 0))
    });
    // BOM-less UTF-16 is only inferred when one byte lane supplies strong evidence.
    // Arbitrary binary and ambiguous non-Latin text are not silently reinterpreted.
    if odd * 3 > count && even * 20 < count {
        transcode(bytes, UTF_16LE, true)
    } else if even * 3 > count && odd * 20 < count {
        transcode(bytes, UTF_16BE, true)
    } else {
        None
    }
}

fn utf32(bytes: &[u8], little: bool) -> Option<String> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|unit| {
            char::from_u32(if little {
                u32::from_le_bytes(*unit)
            } else {
                u32::from_be_bytes(*unit)
            })
        })
        .collect()
}

fn forbidden_byte(byte: u8) -> bool {
    byte < 0x20 && !matches!(byte, b'\n' | b'\r' | b'\t' | 0x0c | 0x1b | 0x08)
}

fn printable(text: &str) -> bool {
    !text.chars().any(|ch| {
        ch.is_control()
            && !matches!(
                ch,
                '\n' | '\r' | '\t' | '\u{000c}' | '\u{001b}' | '\u{0008}'
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_2022_jp_is_not_mistaken_for_ascii_source() {
        let source = "日本語の請求書 93";
        let (encoded, _, errors) = ISO_2022_JP.encode(source);
        assert!(!errors);
        assert_eq!(decode(&encoded).unwrap().unwrap().content, source);
    }

    #[test]
    fn decodes_unicode_without_replacing_or_normalizing_source() {
        let source = "fn main() { /* 你好 café */ }\r\n\tlet x = 1;\r\n";
        assert!(matches!(
            decode(source.as_bytes()).unwrap().unwrap().content,
            Cow::Borrowed(_)
        ));
        for little in [true, false] {
            let mut bytes = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in source.encode_utf16() {
                bytes.extend(if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            assert_eq!(decode(&bytes).unwrap().unwrap().content, source);
            let mut bytes = if little {
                vec![0xff, 0xfe, 0, 0]
            } else {
                vec![0, 0, 0xfe, 0xff]
            };
            for ch in source.chars() {
                bytes.extend(if little {
                    u32::from(ch).to_le_bytes()
                } else {
                    u32::from(ch).to_be_bytes()
                });
            }
            assert_eq!(decode(&bytes).unwrap().unwrap().content, source);
        }
    }

    #[test]
    fn decodes_legacy_text_and_rejects_binary_and_malformed_unicode() {
        let text = decode(b"Caf\xe9 invoice: \x80 93.\r\n").unwrap().unwrap();
        assert!(text.content.contains("Café"));
        assert!(text.guessed);
        for bytes in [
            &[0, 255, 128][..],
            &[0xff, 0xfe, 0][..],
            &[0, 0, 0xfe, 0xff, 0, 0x11, 0, 0][..],
        ] {
            assert!(decode(bytes).unwrap().is_none());
        }
        let ascii: Vec<_> = "A complete line of UTF16 text"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            decode(&ascii).unwrap().unwrap().content,
            "A complete line of UTF16 text"
        );
    }
}
