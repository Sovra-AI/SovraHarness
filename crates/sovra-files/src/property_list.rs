//! Stream property lists with explicit nesting and output budgets.

use crate::ReadMode;
use plist::stream::{Event, Reader};
use sovra_core::{file_index::ScanObserver, files::FileError};
use std::fmt::Write as _;
use std::io::Cursor;

pub(super) fn extract(
    bytes: &[u8],
    mode: ReadMode,
    observer: Option<&dyn ScanObserver>,
) -> Result<Option<String>, FileError> {
    let mut output =
        String::from("Property list. Dictionaries contain alternating keys and values.\n");
    let mut depth = 0usize;
    for (count, event) in Reader::new(Cursor::new(bytes)).enumerate() {
        if observer.is_some_and(ScanObserver::cancelled) {
            return Err(FileError("Reading was cancelled.".into()));
        }
        if !mode.indexed() && count >= 100_000 {
            return Err(limit(mode));
        }
        let Ok(event) = event else {
            return Ok(None);
        };
        let (line, close) = match event {
            Event::StartArray(_) => ("Array [".into(), false),
            Event::StartDictionary(_) => ("Dictionary {".into(), false),
            Event::EndCollection => ("End collection".into(), true),
            Event::Boolean(value) => (value.to_string(), false),
            Event::Data(value) => (
                format!("[Binary value: {} bytes; not decoded]", value.len()),
                false,
            ),
            Event::Date(value) => (format!("{value:?}"), false),
            Event::Integer(value) => (value.to_string(), false),
            Event::Real(value) => (value.to_string(), false),
            Event::String(value) => (
                serde_json::Value::String(value.into_owned()).to_string(),
                false,
            ),
            Event::Uid(value) => (format!("UID {}", value.get()), false),
            _ => ("[Unrecognized property-list event]".into(), false),
        };
        if close {
            depth = depth.saturating_sub(1);
        }
        if output
            .len()
            .saturating_add(line.len())
            .saturating_add(depth * 2 + 1)
            > mode.text_bytes()
        {
            return Err(limit(mode));
        }
        let _ = writeln!(output, "{}{}", "  ".repeat(depth), line);
        if line.ends_with('[') || line.ends_with('{') {
            depth += 1;
        }
        if depth > 64 {
            return Err(FileError("This property list has more than 64 nested levels. Export a flatter representation as text, then refresh the folder.".into()));
        }
    }
    Ok(Some(output))
}

fn limit(mode: ReadMode) -> FileError {
    if mode.indexed() {
        return mode.budget_error();
    }
    FileError(
        "This property list exceeds the nesting or text limits. Split it into smaller files."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_excessive_nesting_and_malformed_binary_lengths() {
        let mut value = plist::Value::String("leaf".into());
        for _ in 0..70 {
            value = plist::Value::Array(vec![value]);
        }
        let mut bytes = Vec::new();
        value.to_writer_binary(&mut bytes).unwrap();
        assert!(extract(&bytes, ReadMode::Attachment, None).is_err());
        assert!(
            extract(b"bplist00\xff\xff\xff", ReadMode::Attachment, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reads_binary_plist_values() {
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("Total".into(), plist::Value::Integer(93.into()));
        let mut bytes = Vec::new();
        plist::Value::Dictionary(dictionary)
            .to_writer_binary(&mut bytes)
            .unwrap();
        let text = extract(&bytes, ReadMode::Attachment, None)
            .unwrap()
            .unwrap();
        assert!(text.contains("Total") && text.contains("93"));
    }
}
