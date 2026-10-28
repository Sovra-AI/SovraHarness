//! Read-only SQLite/GeoPackage table extraction; file-supplied SQL is never executed.

use rusqlite::{Connection, MAIN_DB, config::DbConfig, limits::Limit, types::ValueRef};
use sovra_core::files::MAX_DOCUMENT_BYTES;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

pub(super) fn extract(bytes: &[u8]) -> rusqlite::Result<String> {
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
        (Limit::SQLITE_LIMIT_LENGTH, 1_000_000),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 100_000),
        (Limit::SQLITE_LIMIT_COLUMN, 256),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_VDBE_OP, 100_000),
    ] {
        db.set_limit(limit, maximum)?;
    }
    let started = Instant::now();
    db.progress_handler(
        1000,
        Some(move || started.elapsed() > Duration::from_secs(5)),
    )?;
    db.deserialize_read_exact(MAIN_DB, bytes, bytes.len(), true)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    let mut statement = db.prepare("SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 101")?;
    let tables: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut output = String::from(
        "SQLite tables (read-only). Views, virtual tables, triggers, and generated columns are not evaluated. Binary cells are identified by size.\n",
    );
    if tables.len() > 100 {
        output.push_str("[Table inventory limited to 100 tables.]\n");
    }
    for table in tables.iter().take(100) {
        let mut columns =
            db.prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE hidden=0 ORDER BY cid")?;
        let columns: Vec<String> = columns
            .query_map([table], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if columns.is_empty() {
            continue;
        }
        let _ = writeln!(output, "\nTable: {table}");
        let selected = columns
            .iter()
            .map(|name| quoted(name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut query = db.prepare(&format!(
            "SELECT {selected} FROM {} LIMIT 10001",
            quoted(table)
        ))?;
        let mut rows = query.query([])?;
        let mut count = 0;
        while let Some(row) = rows.next()? {
            if count == 10_000 {
                output.push_str("[Row inventory limited to 10,000 rows in this table.]\n");
                break;
            }
            let mut record = serde_json::Map::new();
            for (index, column) in columns.iter().enumerate() {
                let value = match row.get_ref(index)? {
                    ValueRef::Null => serde_json::Value::Null,
                    ValueRef::Integer(value) => value.into(),
                    ValueRef::Real(value) => serde_json::Value::from(value),
                    ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned().into(),
                    ValueRef::Blob(value) => {
                        format!("[Binary cell: {} bytes; not decoded]", value.len()).into()
                    }
                };
                record.insert(column.clone(), value);
            }
            let encoded = serde_json::Value::Object(record).to_string();
            if output.len().saturating_add(encoded.len()) > MAX_DOCUMENT_BYTES - 256 {
                output.push_str(
                    "[Extraction stopped at the text budget. Additional rows were not included.]\n",
                );
                return Ok(output);
            }
            output.push_str(&encoded);
            output.push('\n');
            count += 1;
        }
    }
    Ok(output)
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
        let result = extract(&data).unwrap();
        assert!(result.contains("Rose bushes") && result.contains("93"));
        assert!(!result.contains("\"calc\"") && !result.contains("Table: risky"));
        assert_eq!(&*data, before);
    }
}
