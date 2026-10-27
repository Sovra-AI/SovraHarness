//! Bounded container metadata reads; no codec execution or external probes.
//!
//! Duration hints do not certify media validity. Fragmented movies, compressed WAV
//! and unsupported containers return unknown rather than a guessed short duration.

const MAX_BOXES: usize = 4096;

pub(super) fn milliseconds(mime: &str, bytes: &[u8]) -> Option<u64> {
    if bytes.len() as u64 > sovra_core::files::MAX_FILE_BYTES {
        return None;
    }
    if mime.starts_with("audio/") && bytes.starts_with(b"RIFF") {
        wave(bytes)
    } else if matches!(
        mime,
        "video/mp4" | "video/quicktime" | "audio/mp4" | "audio/x-m4a"
    ) {
        movie(bytes)
    } else {
        None
    }
}

fn scaled(duration: u64, scale: u64) -> Option<u64> {
    if duration == 0 || scale == 0 || duration == u64::MAX || duration == u64::from(u32::MAX) {
        return None;
    }
    duration
        .checked_mul(1000)
        .map(|value| value.div_ceil(scale))
}

fn wave(bytes: &[u8]) -> Option<u64> {
    if bytes.get(8..12)? != b"WAVE"
        || usize::try_from(le32(bytes, 4)?).ok()?.checked_add(8)? != bytes.len()
    {
        return None;
    }
    let mut offset = 12usize;
    let mut rate = None;
    let mut alignment = None;
    let mut data_bytes = 0u64;
    let mut chunks = 0;
    while offset < bytes.len() {
        chunks += 1;
        if chunks > MAX_BOXES {
            return None;
        }
        let kind = bytes.get(offset..offset.checked_add(4)?)?;
        let length = usize::try_from(le32(bytes, offset.checked_add(4)?)?).ok()?;
        let start = offset.checked_add(8)?;
        let end = start.checked_add(length)?;
        let data = bytes.get(start..end)?;
        if kind == b"fmt " {
            if rate.is_some() || le16(data, 0)? != 1 {
                return None;
            }
            let channels = u32::from(le16(data, 2)?);
            let sample_rate = le32(data, 4)?;
            let byte_rate = le32(data, 8)?;
            let block = u32::from(le16(data, 12)?);
            let bits = u32::from(le16(data, 14)?);
            if channels == 0
                || sample_rate == 0
                || bits == 0
                || bits % 8 != 0
                || channels.checked_mul(bits / 8)? != block
                || block == 0
                || sample_rate.checked_mul(block)? != byte_rate
            {
                return None;
            }
            rate = Some(u64::from(byte_rate));
            alignment = Some(u64::from(block));
        } else if kind == b"data" {
            data_bytes = data_bytes.checked_add(length as u64)?;
        }
        offset = end.checked_add(length % 2)?;
        if offset > bytes.len() {
            return None;
        }
    }
    if !data_bytes.is_multiple_of(alignment?) {
        return None;
    }
    scaled(data_bytes, rate?)
}

fn le16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn le32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn be32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn be64(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        data.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// A small borrowed box list, capped across all nesting levels.
fn boxes<'a>(bytes: &'a [u8], remaining: &mut usize) -> Option<Vec<(&'a [u8], &'a [u8])>> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        *remaining = remaining.checked_sub(1)?;
        let rest = bytes.get(offset..)?;
        let kind = rest.get(4..8)?;
        let (size, header) = match be32(rest, 0)? {
            0 => (rest.len(), 8),
            1 => (usize::try_from(be64(rest, 8)?).ok()?, 16),
            size => (usize::try_from(size).ok()?, 8),
        };
        if size < header {
            return None;
        }
        result.push((kind, rest.get(header..size)?));
        offset = offset.checked_add(size)?;
    }
    Some(result)
}

fn timed_header(data: &[u8]) -> Option<(u64, u64)> {
    let (scale, duration) = match *data.first()? {
        0 => (u64::from(be32(data, 12)?), u64::from(be32(data, 16)?)),
        1 => (u64::from(be32(data, 20)?), be64(data, 24)?),
        _ => return None,
    };
    scaled(duration, scale)?;
    Some((duration, scale))
}

fn movie(bytes: &[u8]) -> Option<u64> {
    let mut remaining = MAX_BOXES;
    let top = boxes(bytes, &mut remaining)?;
    if top.iter().any(|(kind, _)| *kind == b"moof") {
        return None;
    }
    let mut movies = top.iter().filter(|(kind, _)| *kind == b"moov");
    let movie = movies.next()?.1;
    if movies.next().is_some() {
        return None;
    }
    let children = boxes(movie, &mut remaining)?;
    if children.iter().any(|(kind, _)| *kind == b"mvex") {
        return None;
    }
    let mut headers = children.iter().filter(|(kind, _)| *kind == b"mvhd");
    let (duration, scale) = timed_header(headers.next()?.1)?;
    if headers.next().is_some() {
        return None;
    }
    let mut longest = scaled(duration, scale)?;
    let mut tracks = 0;
    for (_, track) in children.iter().filter(|(kind, _)| *kind == b"trak") {
        tracks += 1;
        let mut track_header = false;
        let mut media_header = false;
        for (kind, data) in boxes(track, &mut remaining)? {
            if kind == b"tkhd" {
                if track_header {
                    return None;
                }
                track_header = true;
                let duration = match *data.first()? {
                    0 => u64::from(be32(data, 20)?),
                    1 => be64(data, 28)?,
                    _ => return None,
                };
                longest = longest.max(scaled(duration, scale)?);
            } else if kind == b"mdia" {
                for (kind, data) in boxes(data, &mut remaining)? {
                    if kind == b"mdhd" {
                        if media_header {
                            return None;
                        }
                        media_header = true;
                        let (duration, scale) = timed_header(data)?;
                        longest = longest.max(scaled(duration, scale)?);
                    }
                }
            }
        }
        if !track_header || !media_header {
            return None;
        }
    }
    (tracks > 0).then_some(longest)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MOVIE: &[u8] = include_bytes!("../../../tests/fixtures/media/ten-minute-blue.mp4");

    fn wave_fixture() -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(16_036u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(16_000u32.to_le_bytes());
        bytes.resize(16_044, 128);
        bytes
    }

    fn atom(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(data.len() + 8)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        bytes.extend(kind);
        bytes.extend(data);
        bytes
    }

    #[test]
    fn movie_version_one_extended_boxes_timescales_and_complexity_are_bounded() {
        let mut header = vec![0; 112];
        header[0] = 1;
        header[20..24].copy_from_slice(&3u32.to_be_bytes());
        header[24..32].copy_from_slice(&1u64.to_be_bytes());
        let mut track = vec![0; 96];
        track[0] = 1;
        track[28..36].copy_from_slice(&2u64.to_be_bytes());
        let mut media = vec![0; 36];
        media[0] = 1;
        media[20..24].copy_from_slice(&2u32.to_be_bytes());
        media[24..32].copy_from_slice(&3u64.to_be_bytes());
        let mut track = atom(*b"tkhd", &track);
        track.extend(atom(*b"mdia", &atom(*b"mdhd", &media)));
        let mut children = atom(*b"mvhd", &header);
        children.extend(atom(*b"trak", &track));
        let movie = atom(*b"moov", &children);
        assert_eq!(milliseconds("video/mp4", &movie), Some(1500));
        let mut extended = 1u32.to_be_bytes().to_vec();
        extended.extend(b"moov");
        extended.extend(u64::try_from(children.len() + 16).unwrap().to_be_bytes());
        extended.extend(&children);
        assert_eq!(milliseconds("video/mp4", &extended), Some(1500));
        extended[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(milliseconds("video/mp4", &extended), None);
        let mut fragmented = movie.clone();
        fragmented.extend(atom(*b"moof", &[]));
        assert_eq!(milliseconds("video/mp4", &fragmented), None);
        children.extend(atom(*b"mvex", &[]));
        assert_eq!(milliseconds("video/mp4", &atom(*b"moov", &children)), None);
        let mut excessive = atom(*b"free", &[]).repeat(MAX_BOXES);
        excessive.extend(movie);
        assert_eq!(milliseconds("video/mp4", &excessive), None);
    }

    #[test]
    fn compressed_movie_duration_survives_attachment_storage_and_reopen() {
        use sovra_core::files::FileStore as _;
        assert_eq!(milliseconds("video/mp4", MOVIE), Some(600_000));
        let file = crate::prepare_bytes("duration-test", "movie.mp4", MOVIE).unwrap();
        assert_eq!(file.media[0].duration_millis, Some(600_000));
        let directory =
            std::env::temp_dir().join(format!("sovra-media-duration-{}", std::process::id()));
        let store = crate::LocalFileStore::new(directory.clone());
        store.save(&file).unwrap();
        let reopened = crate::LocalFileStore::new(directory.clone())
            .load("duration-test")
            .unwrap();
        assert_eq!(reopened.media[0].duration_millis, Some(600_000));
        assert_eq!(reopened.media[0].data, MOVIE);
        let mut legacy = file;
        legacy.attachment.id = "unknown-duration".into();
        legacy.attachment.note = None;
        legacy.media[0].data = b"unsupported movie timing".to_vec();
        store.save(&legacy).unwrap();
        let unknown = store.load("unknown-duration").unwrap();
        assert_eq!(unknown.media[0].duration_millis, None);
        assert!(
            unknown
                .attachment
                .note
                .as_ref()
                .unwrap()
                .contains("length could not be checked")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn movie_uses_longest_header_and_unknown_or_truncated_data_is_not_guessed() {
        let mut bytes = MOVIE.to_vec();
        let movie = bytes.windows(4).position(|value| value == b"mvhd").unwrap() + 4;
        bytes[movie + 16..movie + 20].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(milliseconds("video/mp4", &bytes), Some(600_000));
        let media = bytes.windows(4).position(|value| value == b"mdhd").unwrap() + 4;
        bytes[media + 12..media + 16].fill(0);
        assert_eq!(milliseconds("video/mp4", &bytes), None);
        for length in 0..MOVIE.len() {
            // Complete early boxes can expose metadata even without later media payload;
            // this probe never promises a decoded or complete playable file.
            let _ = milliseconds("video/mp4", &MOVIE[..length]);
        }
        assert_eq!(milliseconds("video/webm", MOVIE), None);
        assert_eq!(milliseconds("video/mp4", &[0; 8]), None);
        assert_eq!(scaled(u64::MAX, 1), None);
        assert_eq!(scaled(u64::MAX - 1, 1), None);
        assert_eq!(scaled(1, 3), Some(334));
    }

    #[test]
    fn pcm_wave_checks_rates_alignment_lengths_and_unsupported_codecs() {
        let bytes = wave_fixture();
        assert_eq!(milliseconds("audio/wav", &bytes), Some(2000));
        for (offset, value) in [(20, 3), (28, 0), (32, 0), (34, 7)] {
            let mut invalid = bytes.clone();
            invalid[offset] = value;
            assert_eq!(milliseconds("audio/wav", &invalid), None);
        }
        assert_eq!(milliseconds("audio/wav", &bytes[..bytes.len() - 1]), None);
        assert_eq!(milliseconds("audio/mpeg", b"ID3"), None);
    }
}
