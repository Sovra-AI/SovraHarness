//! Unicode matching for inventory and report filters; source paths remain unchanged.

use caseless::Caseless as _;
use rusqlite::{Connection, functions::FunctionFlags};
use unicode_normalization::UnicodeNormalization as _;

pub(super) fn fold(value: &str) -> String {
    // Canonical normalization also matches filenames supplied by filesystems
    // that represent accented letters with combining marks.
    value.nfd().default_case_fold().nfd().collect()
}

pub(super) fn register(db: &Connection) -> rusqlite::Result<()> {
    db.create_scalar_function(
        "sovra_fold",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_DIRECTONLY,
        |context| context.get::<String>(0).map(|value| fold(&value)),
    )
}
