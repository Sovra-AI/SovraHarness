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
    use sovra_core::knowledge_analysis::Language;
    db.create_scalar_function(
        "sovra_stems",
        2,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| {
            let raw = ctx.get::<String>(0)?;
            let language: Language =
                serde_json::from_value(serde_json::Value::String(ctx.get::<String>(1)?))
                    .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
            Ok(language.stem_text(&raw))
        },
    )?;
    db.create_scalar_function(
        "sovra_fold",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_DIRECTONLY,
        |context| context.get::<String>(0).map(|value| fold(&value)),
    )
}

/// A selected language pays the one-time backfill cost. Triggers maintain it in
/// the chunk transaction. All interpolated identifiers come from a closed enum.
pub(super) fn language_index(
    db: &Connection,
    language: sovra_core::knowledge_analysis::Language,
) -> rusqlite::Result<String> {
    let lang = language.name();
    let table = format!("index_stems_{lang}");
    let exists = |db: &Connection| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1 AND type='table')",
            [&table],
            |row| row.get::<_, bool>(0),
        )
    };
    if exists(db)? {
        return Ok(table);
    }
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)?;
    if !exists(&tx)? {
        tx.execute_batch(&format!("CREATE VIRTUAL TABLE {table} USING fts5(body,content='',contentless_delete=1);
            INSERT INTO {table}(rowid,body) SELECT rowid,sovra_stems(body,'{lang}') FROM index_chunks;
            CREATE TRIGGER {table}_insert AFTER INSERT ON index_chunks BEGIN
                INSERT INTO {table}(rowid,body) VALUES(new.rowid,sovra_stems(new.body,'{lang}')); END;
            CREATE TRIGGER {table}_delete AFTER DELETE ON index_chunks BEGIN
                DELETE FROM {table} WHERE rowid=old.rowid; END;"))?;
    }
    tx.commit()?;
    Ok(table)
}
