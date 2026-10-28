//! Strict incremental text decoding. Unicode units use a small input carry;
//! legacy decoders retain multibyte characters and escape state between blocks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Encoding {
    Utf8,
    Utf16 { little: bool },
    Utf32 { little: bool },
    Legacy(&'static encoding_rs::Encoding),
}

impl Encoding {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16 { little: true } => "UTF-16LE",
            Self::Utf16 { little: false } => "UTF-16BE",
            Self::Utf32 { little: true } => "UTF-32LE",
            Self::Utf32 { little: false } => "UTF-32BE",
            Self::Legacy(encoding) => encoding.name(),
        }
    }
}

pub(super) struct Decoder {
    pub(super) encoding: Encoding,
    pub(super) inferred: bool,
    high: Option<u16>,
    legacy: Option<encoding_rs::Decoder>,
}

impl Decoder {
    pub(super) fn start(prefix: &[u8]) -> (Self, usize) {
        let (encoding, bom) = if prefix.starts_with(&[0xff, 0xfe, 0, 0]) {
            (Encoding::Utf32 { little: true }, 4)
        } else if prefix.starts_with(&[0, 0, 0xfe, 0xff]) {
            (Encoding::Utf32 { little: false }, 4)
        } else if prefix.starts_with(&[0xff, 0xfe]) {
            (Encoding::Utf16 { little: true }, 2)
        } else if prefix.starts_with(&[0xfe, 0xff]) {
            (Encoding::Utf16 { little: false }, 2)
        } else if prefix.starts_with(&[0xef, 0xbb, 0xbf]) {
            (Encoding::Utf8, 3)
        } else if let Some(little) = crate::text::wide_little_endian(prefix) {
            (Encoding::Utf16 { little }, 0)
        } else {
            (Encoding::Utf8, 0)
        };
        (
            Self {
                encoding,
                inferred: bom == 0 && encoding != Encoding::Utf8,
                high: None,
                legacy: None,
            },
            bom,
        )
    }

    pub(super) fn legacy(encoding: &'static encoding_rs::Encoding) -> Self {
        Self {
            encoding: Encoding::Legacy(encoding),
            inferred: true,
            high: None,
            legacy: Some(encoding.new_decoder_without_bom_handling()),
        }
    }

    /// Returns the consumed prefix, leaving at most three bytes for the next read.
    /// No replacement characters are synthesized for invalid input.
    pub(super) fn decode(
        &mut self,
        bytes: &[u8],
        output: &mut String,
        last: bool,
    ) -> Option<usize> {
        output.clear();
        match self.encoding {
            Encoding::Utf8 => {
                let valid = match std::str::from_utf8(bytes) {
                    Ok(_) => bytes.len(),
                    Err(error) if error.error_len().is_none() => error.valid_up_to(),
                    Err(_) => return None,
                };
                output.push_str(std::str::from_utf8(&bytes[..valid]).expect("validated prefix"));
                Some(valid)
            }
            Encoding::Utf16 { little } => {
                let (units, remainder) = bytes.as_chunks::<2>();
                for bytes in units {
                    let unit = if little {
                        u16::from_le_bytes(*bytes)
                    } else {
                        u16::from_be_bytes(*bytes)
                    };
                    if let Some(high) = self.high.take() {
                        if !(0xdc00..=0xdfff).contains(&unit) {
                            return None;
                        }
                        let value =
                            0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00;
                        output.push(char::from_u32(value)?);
                    } else if (0xd800..=0xdbff).contains(&unit) {
                        self.high = Some(unit);
                    } else {
                        output.push(char::from_u32(u32::from(unit))?);
                    }
                }
                Some(bytes.len() - remainder.len())
            }
            Encoding::Utf32 { little } => {
                let (units, remainder) = bytes.as_chunks::<4>();
                for bytes in units {
                    let value = if little {
                        u32::from_le_bytes(*bytes)
                    } else {
                        u32::from_be_bytes(*bytes)
                    };
                    output.push(char::from_u32(value)?);
                }
                Some(bytes.len() - remainder.len())
            }
            Encoding::Legacy(_) => {
                let decoder = self.legacy.as_mut().expect("legacy encoding has a decoder");
                // The bound depends only on this fixed-size input block and
                // the decoder's small carry, never on the whole file size.
                output.reserve(decoder.max_utf8_buffer_length_without_replacement(bytes.len())?);
                let (result, read) =
                    decoder.decode_to_string_without_replacement(bytes, output, last);
                match result {
                    encoding_rs::DecoderResult::InputEmpty => Some(read),
                    encoding_rs::DecoderResult::OutputFull
                    | encoding_rs::DecoderResult::Malformed(..) => None,
                }
            }
        }
    }

    pub(super) fn finished(&self) -> bool {
        self.high.is_none()
    }
}
