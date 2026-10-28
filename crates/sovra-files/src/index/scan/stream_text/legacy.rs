//! Full-source encoding inference in bounded buffers, used after Unicode fails.

use super::*;
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

pub(super) fn detect(
    file: &mut (impl std::io::Read + std::io::Seek),
    length: u64,
    root: &mut IndexedRoot,
    observer: &dyn ScanObserver,
) -> Result<Option<&'static encoding_rs::Encoding>, IndexError> {
    if observer.cancelled() {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| io_error(&e))?;
    let mut buffer = vec![0; BUFFER_BYTES];
    let prefix = usize::try_from(length.min(PREFIX_BYTES as u64)).expect("bounded prefix");
    file.read_exact(&mut buffer[..prefix])
        .map_err(|e| io_error(&e))?;
    root.progress.bytes_read = root.progress.bytes_read.saturating_add(prefix as u64);
    let (unicode, bom) = Decoder::start(&buffer[..prefix]);
    // Explicit Unicode and conservative wide-text evidence must never fall back
    // to a single-byte interpretation after a malformed sequence.
    if bom != 0 || unicode.encoding != Encoding::Utf8 || buffer[..prefix].starts_with(b"%PDF-") {
        return Ok(None);
    }
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    let mut remaining = length - prefix as u64;
    let mut used = prefix;
    let mut blocks = 0_u64;
    loop {
        if observer.cancelled()
            || buffer[..used]
                .iter()
                .copied()
                .any(crate::text::forbidden_byte)
        {
            return Ok(None);
        }
        detector.feed(&buffer[..used], remaining == 0);
        if remaining == 0 {
            break;
        }
        let count = usize::try_from(remaining.min(BUFFER_BYTES as u64)).expect("bounded read");
        used = file.read(&mut buffer[..count]).map_err(|e| io_error(&e))?;
        if used == 0 {
            return Err(changed());
        }
        root.progress.bytes_read = root.progress.bytes_read.saturating_add(used as u64);
        remaining -= used as u64;
        blocks += 1;
        if blocks.is_multiple_of(16) {
            observer.progress(root);
        }
    }
    if file.read(&mut buffer[..1]).map_err(|e| io_error(&e))? != 0 {
        return Err(changed());
    }
    let encoding = detector.guess(None, Utf8Detection::Allow);
    Ok((encoding != encoding_rs::UTF_8).then_some(encoding))
}

/// ISO-2022-JP is ASCII at the byte level. Carry the escape prefix across reads
/// so an escape split at a block boundary still selects the Japanese decoder.
#[derive(Default)]
pub(super) struct EscapeHint(u8);

impl EscapeHint {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> bool {
        if self.0 == 0 && !bytes.contains(&0x1b) {
            return false;
        }
        for byte in bytes {
            match (self.0, byte) {
                (2, b'B' | b'@') | (3, b'J') => return true,
                (_, 0x1b) => self.0 = 1,
                (1, b'$') => self.0 = 2,
                (1, b'(') => self.0 = 3,
                _ => self.0 = 0,
            }
        }
        false
    }
}
