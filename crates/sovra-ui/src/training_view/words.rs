//! Numbers said the way a person would say them. Pure, so every phrase is tested.

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// A whole number with thousands separators: `137,216`.
pub(super) fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A rough length of time: seconds, minutes or hours, never false precision.
pub(super) fn duration(ms: u64) -> String {
    let seconds = ms.div_ceil(1000);
    match seconds {
        0..=2 => "a second or two".to_owned(),
        3..=50 => format!("about {} seconds", seconds.div_ceil(5) * 5),
        51..=89 => "about a minute".to_owned(),
        90..=3_299 => format!("about {} minutes", (seconds + 30) / 60),
        3_300..=5_399 => "about an hour".to_owned(),
        _ => format!("about {} hours", (seconds + 1_800) / 3_600),
    }
}

/// Memory in gigabytes with one decimal: `15.6 GB`. Zero means it is not known.
pub(super) fn gigabytes(bytes: u64) -> String {
    if bytes == 0 {
        return "Not known".to_owned();
    }
    let tenths = (u128::from(bytes) * 10 + u128::from(GIB / 2)) / u128::from(GIB);
    format!("{}.{} GB", tenths / 10, tenths % 10)
}

/// A file size: `0.5 MB`, or kilobytes when smaller than a tenth of that.
pub(super) fn file_size(bytes: u64) -> String {
    if bytes < MIB / 10 {
        return format!("{} KB", bytes.div_ceil(1024));
    }
    let tenths = (bytes * 10 + MIB / 2) / MIB;
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

/// A loss in thousandths as a decimal: `2.143`.
pub(super) fn loss(milli: u32) -> String {
    format!("{}.{:03}", milli / 1000, milli % 1000)
}

/// Processor cores in words. Zero means the count is not known.
pub(super) fn cores(physical: u16, logical: u16) -> String {
    match (physical, logical) {
        (0, 0) => "Not known".to_owned(),
        (0, threads) => format!("{threads} threads"),
        (cores, 0) => format!("{cores} cores"),
        (cores, threads) if cores == threads => format!("{cores} cores"),
        (cores, threads) => format!("{cores} cores, {threads} threads"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_like_a_person_wrote_them() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(137_216), "137,216");
        assert_eq!(count(2_817_280), "2,817,280");
        assert_eq!(loss(2143), "2.143");
        assert_eq!(loss(7), "0.007");
        assert_eq!(cores(8, 16), "8 cores, 16 threads");
        assert_eq!(cores(4, 4), "4 cores");
        assert_eq!(cores(0, 0), "Not known");
    }

    #[test]
    fn durations_stay_rough() {
        assert_eq!(duration(0), "a second or two");
        assert_eq!(duration(11_200), "about 15 seconds");
        assert_eq!(duration(50_000), "about 50 seconds");
        assert_eq!(duration(70_000), "about a minute");
        assert_eq!(duration(190_000), "about 3 minutes");
        assert_eq!(duration(3_600_000), "about an hour");
        assert_eq!(duration(3 * 3_600_000), "about 3 hours");
    }

    #[test]
    fn sizes_keep_one_decimal() {
        assert_eq!(gigabytes(0), "Not known");
        assert_eq!(gigabytes(16 * GIB), "16.0 GB");
        assert_eq!(gigabytes(GIB / 2), "0.5 GB");
        assert_eq!(file_size(548_928), "0.5 MB");
        assert_eq!(file_size(2_048), "2 KB");
        assert_eq!(file_size(11 * MIB + MIB / 4), "11.3 MB");
    }
}
