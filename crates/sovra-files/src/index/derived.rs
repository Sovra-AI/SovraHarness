//! Shared bounds and encoding for derived index records.

use super::db_error;
use rusqlite::Connection;
use serde::{Serialize, de::DeserializeOwned};
use sovra_core::file_index::IndexError;

pub(super) fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

pub(super) fn bounded(db: &mut Connection) -> Result<(), IndexError> {
    let start = std::time::Instant::now();
    db.progress_handler(
        1000,
        Some(move || start.elapsed() > std::time::Duration::from_secs(5)),
    )
    .map_err(|e| db_error(&e))
}

pub(super) fn number(value: u64) -> Result<i64, IndexError> {
    i64::try_from(value)
        .map_err(|_| IndexError("Invalid knowledge identifier or timestamp.".into()))
}

pub(super) fn json<T: Serialize>(value: &T) -> Result<String, IndexError> {
    serde_json::to_string(value)
        .map_err(|_| IndexError("Knowledge data could not be encoded.".into()))
}

pub(super) fn decode<T: DeserializeOwned>(value: &str) -> Result<T, IndexError> {
    serde_json::from_str(value)
        .map_err(|_| IndexError("Stored knowledge data could not be read.".into()))
}
