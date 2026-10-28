//! Snapshot-consistent metadata queries. Only fixed column expressions enter SQL;
//! filenames, keyword phrases, ranges and scope IDs always use bound parameters.

use super::{db_error, digest};
use rusqlite::{Connection, Row, params_from_iter, types::Value};
use sovra_core::file_index::{
    CatalogCoverage, CatalogCursor, CatalogEntry, CatalogFilter, CatalogGroup, CatalogGroupBy,
    CatalogPage, CatalogQuery, ExtractionState, IndexError, PassageRef,
};
use std::fmt::Write as _;

pub(super) fn path_parts(path: &str) -> (String, String, String) {
    let (directory, name) = path.rsplit_once('/').unwrap_or(("", path));
    let extension = name
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension)
        .to_ascii_lowercase();
    (name.into(), directory.into(), extension)
}

pub(super) fn query(db: &mut Connection, query: &CatalogQuery) -> Result<CatalogPage, IndexError> {
    query.validate()?;
    let start = std::time::Instant::now();
    db.progress_handler(
        1_000,
        Some(move || start.elapsed() > std::time::Duration::from_secs(5)),
    )
    .map_err(|e| db_error(&e))?;
    let tx = db.transaction().map_err(|e| db_error(&e))?;
    let revision = tx
        .query_row(
            "SELECT revision FROM index_state WHERE singleton=1",
            [],
            |row| counted(row, 0),
        )
        .map_err(|e| db_error(&e))?;
    let key = digest(
        &serde_json::to_vec(&query.filter)
            .map_err(|_| IndexError("Invalid catalogue filters.".into()))?,
    );
    if let Some(cursor) = &query.cursor
        && (cursor.revision != revision || cursor.filter_key != key)
    {
        return Err(IndexError(
            "The file list or filters changed between pages. Reload the list to see the latest files."
                .into(),
        ));
    }
    let (filtered, mut values) = filter(&query.filter)?;
    let (total_files, total_known_bytes, unknown_sizes) = tx
        .query_row(
            &format!(
                "SELECT count(*),coalesce(sum(f.byte_size),0),coalesce(sum(f.byte_size IS NULL),0) {filtered}"
            ),
            params_from_iter(&values),
            |row| Ok((counted(row, 0)?, counted(row, 1)?, counted(row, 2)?)),
        )
        .map_err(|e| db_error(&e))?;
    let mut page = CatalogPage {
        revision,
        total_files,
        total_known_bytes,
        unknown_sizes,
        coverage: coverage(&tx, &query.filter.roots)?,
        files: vec![],
        groups: vec![],
        next_cursor: None,
        has_more: false,
    };
    if let Some(group_by) = query.group_by {
        let (scope, field) = match group_by {
            CatalogGroupBy::Root => ("f.root_id", "r.path"),
            CatalogGroupBy::Directory => ("f.root_id", "f.directory"),
            CatalogGroupBy::MimeType => ("NULL", "coalesce(v.mime,'(unknown)')"),
            CatalogGroupBy::Extension => ("NULL", "f.extension"),
        };
        let limit = bind(
            &mut values,
            Value::Integer(i64::try_from(query.limit + 1).expect("bounded query limit")),
        );
        let sql = format!(
            "SELECT {scope} AS scope,{field} AS group_key,count(*) AS n,coalesce(sum(f.byte_size),0),sum(f.byte_size IS NULL),sum(f.extraction IN ('text','partial_text')) {filtered} GROUP BY scope,group_key ORDER BY n DESC,scope,group_key LIMIT {limit}"
        );
        let mut statement = tx.prepare(&sql).map_err(|e| db_error(&e))?;
        page.groups = statement
            .query_map(params_from_iter(&values), |r| {
                Ok(CatalogGroup {
                    root_id: r.get(0)?,
                    key: r.get(1)?,
                    files: counted(r, 2)?,
                    total_known_bytes: counted(r, 3)?,
                    unknown_sizes: counted(r, 4)?,
                    files_with_text: counted(r, 5)?,
                })
            })
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| db_error(&e))?;
        page.has_more = page.groups.len() > query.limit;
        page.groups.truncate(query.limit);
    } else {
        let mut filtered = filtered;
        if let Some(cursor) = &query.cursor {
            let root = bind(&mut values, cursor.root_id.clone().into());
            let path = bind(&mut values, cursor.path.clone().into());
            let id = bind(&mut values, cursor.file_id.clone().into());
            write!(
                filtered,
                " AND (f.root_id,f.path,f.id)>({root},{path},{id})"
            )
            .expect("SQL fragment");
        }
        let limit = bind(
            &mut values,
            Value::Integer(i64::try_from(query.limit + 1).expect("bounded query limit")),
        );
        let sql = format!(
            "SELECT f.root_id,r.path,f.id,f.path,f.byte_size,v.mime,v.digest,f.observed,f.modified,f.created,f.extraction,f.detail,f.current_version {filtered} ORDER BY f.root_id,f.path,f.id LIMIT {limit}"
        );
        let mut statement = tx.prepare(&sql).map_err(|e| db_error(&e))?;
        page.files = statement
            .query_map(params_from_iter(&values), entry)
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| db_error(&e))?;
        page.has_more = page.files.len() > query.limit;
        page.files.truncate(query.limit);
        if page.has_more
            && let Some(last) = page.files.last()
        {
            page.next_cursor = Some(CatalogCursor {
                revision,
                filter_key: key,
                root_id: last.root_id.clone(),
                path: last.path.clone(),
                file_id: last.file_id.clone(),
            });
        }
    }
    Ok(page)
}

fn coverage(db: &Connection, roots: &[String]) -> Result<Vec<CatalogCoverage>, IndexError> {
    let filter =
        serde_json::to_string(roots).map_err(|_| IndexError("Invalid folder filter.".into()))?;
    let mut query = db.prepare("SELECT details FROM index_roots WHERE ?1='[]' OR id IN (SELECT value FROM json_each(?1)) ORDER BY id").map_err(|e| db_error(&e))?;
    let mut result = vec![];
    for row in query
        .query_map([filter], |row| row.get::<_, String>(0))
        .map_err(|e| db_error(&e))?
    {
        let root: sovra_core::file_index::IndexedRoot =
            super::decode(&row.map_err(|e| db_error(&e))?)?;
        result.push(CatalogCoverage::from(&root));
    }
    if !roots.is_empty()
        && result.len()
            != roots
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
    {
        return Err(IndexError(
            "An indexed folder was removed or its ID is unknown. Call list_indexed_folders again."
                .into(),
        ));
    }
    Ok(result)
}

fn entry(row: &Row<'_>) -> rusqlite::Result<CatalogEntry> {
    let state: String = row.get(10)?;
    let extraction = match state.as_str() {
        "text" => ExtractionState::Text,
        "partial_text" => ExtractionState::PartialText,
        "metadata_only" => ExtractionState::MetadataOnly,
        "failed" => ExtractionState::Failed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let root_id: String = row.get(0)?;
    let file_id: String = row.get(2)?;
    let version: Option<String> = row.get(12)?;
    let first_passage = version
        .filter(|_| {
            matches!(
                extraction,
                ExtractionState::Text | ExtractionState::PartialText
            )
        })
        .map(|version_id| PassageRef {
            root_id: root_id.clone(),
            file_id: file_id.clone(),
            version_id,
            ordinal: 0,
        });
    Ok(CatalogEntry {
        root_id,
        root_path: row.get(1)?,
        file_id,
        path: row.get(3)?,
        byte_size: optional_counted(row, 4)?,
        mime_type: row.get(5)?,
        content_digest: row.get(6)?,
        observed_at: optional_counted(row, 7)?,
        modified_at: optional_counted(row, 8)?,
        created_at: optional_counted(row, 9)?,
        extraction,
        note: row.get(11)?,
        first_passage,
    })
}

/// SQLite has no `u64`. A negative catalogue count is corrupt, not zero.
pub(super) fn counted(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

pub(super) fn optional_counted(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    let Some(value) = row.get::<_, Option<i64>>(index)? else {
        return Ok(None);
    };
    u64::try_from(value)
        .map(Some)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn bind(values: &mut Vec<Value>, value: Value) -> String {
    values.push(value);
    format!("?{}", values.len())
}

fn filter(filter: &CatalogFilter) -> Result<(String, Vec<Value>), IndexError> {
    let mut sql = "FROM index_files f JOIN index_roots r ON r.id=f.root_id LEFT JOIN index_versions v ON v.id=f.current_version WHERE f.available=1".to_owned();
    let mut values = vec![];
    if !filter.roots.is_empty() {
        let roots = serde_json::to_string(&filter.roots)
            .map_err(|_| IndexError("Invalid folder filter.".into()))?;
        let parameter = bind(&mut values, roots.into());
        write!(
            sql,
            " AND f.root_id IN (SELECT value FROM json_each({parameter}))"
        )
        .expect("SQL fragment");
    }
    for (value, column, substring) in [
        (&filter.path_prefix, "f.path", false),
        (&filter.name_contains, "f.filename", true),
        (&filter.mime_prefix, "v.mime", false),
    ] {
        if let Some(value) = value {
            let parameter = bind(&mut values, value.clone().into());
            if substring {
                write!(sql, " AND instr({column},{parameter})>0").expect("SQL fragment");
            } else {
                write!(
                    sql,
                    " AND substr({column},1,length({parameter}))={parameter}"
                )
                .expect("SQL fragment");
            }
        }
    }
    for (value, column, op) in [
        (filter.modified_after, "f.modified", ">="),
        (filter.modified_before, "f.modified", "<="),
        (filter.min_bytes, "f.byte_size", ">="),
        (filter.max_bytes, "f.byte_size", "<="),
    ] {
        if let Some(value) = value {
            let parameter = bind(
                &mut values,
                Value::Integer(i64::try_from(value).map_err(|_| {
                    IndexError("Numeric filter exceeds the supported range.".into())
                })?),
            );
            write!(sql, " AND {column}{op}{parameter}").expect("SQL fragment");
        }
    }
    if let Some(has_text) = filter.has_text {
        sql.push_str(if has_text {
            " AND f.extraction IN ('text','partial_text')"
        } else {
            " AND f.extraction NOT IN ('text','partial_text')"
        });
    }
    if let Some(directory) = &filter.directory {
        let parameter = bind(&mut values, directory.clone().into());
        write!(sql, " AND f.directory={parameter}").expect("SQL fragment");
    }
    if let Some(extraction) = filter.extraction {
        let state = match extraction {
            ExtractionState::Text => "text",
            ExtractionState::PartialText => "partial_text",
            ExtractionState::MetadataOnly => "metadata_only",
            ExtractionState::Failed => "failed",
        };
        let parameter = bind(&mut values, state.to_owned().into());
        write!(sql, " AND f.extraction={parameter}").expect("SQL fragment");
    }
    if let Some(keywords) = &filter.keywords {
        let terms = keywords
            .split_whitespace()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let parameter = bind(&mut values, terms.into());
        write!(sql, " AND f.current_version IN (SELECT vc.version_id FROM index_fts JOIN index_chunks c ON c.rowid=index_fts.rowid JOIN index_version_chunks vc ON vc.chunk_id=c.id WHERE index_fts MATCH {parameter})").expect("SQL fragment");
    }
    Ok((sql, values))
}
