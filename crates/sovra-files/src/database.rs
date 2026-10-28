//! Read-only SQLite/GeoPackage table extraction; file-supplied SQL is never executed.

use crate::ReadMode;
use rusqlite::{Connection, MAIN_DB, config::DbConfig, limits::Limit, types::ValueRef};
use sovra_core::file_index::ScanObserver;
use std::fmt::Write as _;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(super) fn extract(
    bytes: &[u8],
    mode: ReadMode,
    observer: Option<&dyn ScanObserver>,
    warnings: &mut Vec<String>,
) -> rusqlite::Result<String> {
    let mut db = Connection::open_in_memory()?;
    for (config, enabled) in [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
        (DbConfig::SQLITE_DBCONFIG_DQS_DML, false),
        (DbConfig::SQLITE_DBCONFIG_DQS_DDL, false),
    ] {
        db.set_db_config(config, enabled)?;
    }
    for (limit, maximum) in [
        (
            Limit::SQLITE_LIMIT_LENGTH,
            if mode.indexed() {
                i32::try_from(mode.text_bytes()).expect("document budgets fit i32")
            } else {
                1_000_000
            },
        ),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 100_000),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_VDBE_OP, 100_000),
    ] {
        db.set_limit(limit, maximum)?;
    }
    if !mode.indexed() {
        db.set_limit(Limit::SQLITE_LIMIT_COLUMN, 256)?;
    }
    let started = Instant::now();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    let _cancellation = observer.and_then(|observer| {
        observer.watch_cancellation(Arc::new(move || signal.store(true, Ordering::Relaxed)))
    });
    db.progress_handler(
        1000,
        Some(move || {
            cancelled.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(5)
        }),
    )?;
    db.deserialize_read_exact(MAIN_DB, bytes, bytes.len(), true)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    let mut output = String::from(
        "SQLite tables (read-only). Views, virtual tables, triggers, and generated columns are not evaluated. Binary cells are identified by size.\n",
    );
    if let Err(error) = read_tables(&db, mode, observer, &mut output, warnings) {
        warnings.push(if started.elapsed() > Duration::from_secs(5) {
            "Database reading reached its five-second work budget. Rows read so far are retained; additional tables or rows may be missing. Export the required tables as CSV or text and refresh the folder.".into()
        } else {
            format!("Database reading could not finish: {error}. Rows read so far are retained; additional tables or rows may be missing. Repair the database or export the required tables as CSV or text.")
        });
    }
    Ok(output)
}

fn read_tables(
    db: &Connection,
    mode: ReadMode,
    observer: Option<&dyn ScanObserver>,
    output: &mut String,
    warnings: &mut Vec<String>,
) -> rusqlite::Result<()> {
    let mut statement = db.prepare("SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT ?1")?;
    let tables = statement.query_map([if mode.indexed() { -1 } else { 101 }], |row| {
        row.get::<_, String>(0)
    })?;
    let mut binary_cells = false;
    let mut invalid_text = false;
    for (index, table) in tables.enumerate() {
        if observer.is_some_and(ScanObserver::cancelled) {
            return Ok(());
        }
        if !mode.indexed() && index == 100 {
            warnings.push("Table inventory limited to 100 tables. Connect the folder for indexing to read more tables, or export the required tables separately.".into());
            break;
        }
        let table = table?;
        let mut columns =
            db.prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE hidden=0 ORDER BY cid")?;
        let columns: Vec<String> = columns
            .query_map([&table], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if columns.is_empty() {
            continue;
        }
        let heading = format!("\nTable: {table}\n");
        if !append(output, &heading, mode, warnings) {
            return Ok(());
        }
        let selected = columns
            .iter()
            .map(|name| quoted(name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut query = db.prepare(&format!(
            "SELECT {selected} FROM {} LIMIT ?1",
            quoted(&table)
        ))?;
        let mut rows = query.query([if mode.indexed() { -1 } else { 10001 }])?;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            if observer.is_some_and(ScanObserver::cancelled) {
                return Ok(());
            }
            if !mode.indexed() && count == 10_000 {
                warnings.push(format!("{table}: Row inventory limited to 10,000 rows. Connect the folder for indexing to read more rows, or export this table separately."));
                break;
            }
            let mut record = serde_json::Map::new();
            for (index, column) in columns.iter().enumerate() {
                let value = match row.get_ref(index)? {
                    ValueRef::Null => serde_json::Value::Null,
                    ValueRef::Integer(value) => value.into(),
                    ValueRef::Real(value) => serde_json::Value::from(value),
                    ValueRef::Text(value) => {
                        if !invalid_text && std::str::from_utf8(value).is_err() {
                            warnings.push("Some database text cells contain invalid UTF-8. Undecodable bytes are shown as replacement characters; check the original values.".into());
                            invalid_text = true;
                        }
                        String::from_utf8_lossy(value).into_owned().into()
                    }
                    ValueRef::Blob(value) => {
                        if !binary_cells {
                            warnings.push("Binary database cells were identified by size but their contents were not decoded. Export those values in a readable format if needed.".into());
                            binary_cells = true;
                        }
                        format!("[Binary cell: {} bytes; not decoded]", value.len()).into()
                    }
                };
                record.insert(column.clone(), value);
            }
            let encoded = serde_json::Value::Object(record).to_string();
            if !append(output, &encoded, mode, warnings) {
                return Ok(());
            }
            count += 1;
        }
    }
    Ok(())
}

fn append(output: &mut String, text: &str, mode: ReadMode, warnings: &mut Vec<String>) -> bool {
    if output.len().saturating_add(text.len()).saturating_add(1) > mode.text_bytes() {
        warnings.push(if mode.indexed() {
            "Database reading reached the folder's text budget. Additional rows or tables were not included. Choose a larger Document reading budget in Folder settings, or export the required tables as CSV or text.".into()
        } else {
            "Database reading reached the attachment text budget. Additional rows or tables were not included. Connect the folder for indexing or export the required tables separately.".into()
        });
        return false;
    }
    let _ = writeln!(output, "{text}");
    true
}

fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_values_and_quoted_identifiers_without_evaluating_views_or_generated_columns() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE \"invoice\"\"lines\" (item TEXT, total INTEGER, calc INTEGER GENERATED ALWAYS AS (total * 2)); INSERT INTO \"invoice\"\"lines\" (item,total) VALUES ('Rose bushes',93); CREATE VIEW risky AS SELECT load_extension('/not/a/library');").unwrap();
        let data = db.serialize(MAIN_DB).unwrap();
        let before = data.to_vec();
        let result = extract(&data, ReadMode::Attachment, None, &mut Vec::new()).unwrap();
        assert!(result.contains("Rose bushes") && result.contains("93"));
        assert!(!result.contains("\"calc\"") && !result.contains("Table: risky"));
        assert_eq!(&*data, before);
    }
}
