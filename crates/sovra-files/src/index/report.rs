//! Latest-scan diagnostics, persisted without an example cap and read in bounded pages.

use super::{db_error, digest, get_root};
use rusqlite::{Connection, OptionalExtension as _, params};
use sovra_core::file_index::{
    IndexError, IndexStatus, NoticeCursor, NoticeKind, NoticePage, NoticeQuery, ScanNotice,
};

pub(super) struct PendingNotice {
    pub(super) kind: NoticeKind,
    pub(super) path: String,
    pub(super) message: String,
}

pub(super) fn begin(db: &Connection, root: &str, scan: i64) -> Result<(), IndexError> {
    db.execute("DELETE FROM index_scan_reports WHERE root_id=?1", [root])
        .map_err(|e| db_error(&e))?;
    db.execute(
        "INSERT INTO index_scan_reports(root_id,scan_id) VALUES(?1,?2)",
        params![root, scan],
    )
    .map_err(|e| db_error(&e))?;
    Ok(())
}

pub(super) fn append(
    db: &mut Connection,
    root: &str,
    notices: &[PendingNotice],
) -> Result<(), IndexError> {
    if notices.is_empty() {
        return Ok(());
    }
    let tx = db.transaction().map_err(|e| db_error(&e))?;
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO index_scan_notices(root_id,kind,path,message) VALUES(?1,?2,?3,?4)",
            )
            .map_err(|e| db_error(&e))?;
        for notice in notices {
            insert
                .execute(params![
                    root,
                    kind_key(notice.kind),
                    notice.path,
                    notice.message
                ])
                .map_err(|e| db_error(&e))?;
        }
    }
    tx.commit().map_err(|e| db_error(&e))
}

pub(super) fn query(db: &mut Connection, query: &NoticeQuery) -> Result<NoticePage, IndexError> {
    query.validate()?;
    let started = std::time::Instant::now();
    db.progress_handler(
        1_000,
        Some(move || started.elapsed() > std::time::Duration::from_secs(5)),
    )
    .map_err(|e| db_error(&e))?;
    let tx = db.transaction().map_err(|e| db_error(&e))?;
    let root = get_root(&tx, &query.root_id)?;
    let header: Option<(u64, String)> = tx.query_row("SELECT s.id,s.status FROM index_scan_reports r JOIN index_scans s ON s.id=r.scan_id WHERE r.root_id=?1", [&query.root_id], |r| Ok((counted(r,0)?,r.get(1)?))).optional().map_err(|e| db_error(&e))?;
    let Some((scan_id, status)) = header else {
        return Ok(NoticePage {
            root_id: root.id,
            scan_id: None,
            status: root.status,
            total: 0,
            has_newer: false,
            counts: vec![],
            notices: vec![],
            next_cursor: None,
        });
    };
    let status: IndexStatus =
        serde_json::from_value(serde_json::Value::String(status)).map_err(|_| {
            IndexError("The saved scan status could not be read. Refresh the folder.".into())
        })?;
    let key = digest(
        &serde_json::to_vec(&(&query.root_id, query.kind, &query.contains))
            .map_err(|_| IndexError("Invalid report filter.".into()))?,
    );
    if query
        .cursor
        .as_ref()
        .is_some_and(|c| c.scan_id != scan_id || c.filter_key != key)
    {
        return Err(IndexError(
            "A new scan started or the filters changed. Reload the scan report.".into(),
        ));
    }
    let newest = tx
        .query_row(
            "SELECT coalesce(max(sequence),0) FROM index_scan_notices WHERE root_id=?1",
            [&query.root_id],
            |r| counted(r, 0),
        )
        .map_err(|e| db_error(&e))?;
    let through = query.cursor.as_ref().map_or(newest, |c| c.through);
    if through > newest {
        return Err(IndexError(
            "Reload the scan report to reset its pages.".into(),
        ));
    }
    let after = query.cursor.as_ref().map_or(0, |c| c.after);
    let through_sql = i64::try_from(through).expect("validated snapshot bound");
    let after_sql = i64::try_from(after).expect("validated cursor bound");
    let counts = {
        let mut statement = tx.prepare("SELECT kind,count(*) FROM index_scan_notices WHERE root_id=?1 AND sequence<=?2 GROUP BY kind ORDER BY kind").map_err(|e| db_error(&e))?;
        statement
            .query_map(params![query.root_id, through_sql], |r| {
                Ok((parse_kind(r.get(0)?)?, counted(r, 1)?))
            })
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?
    };
    let kind = query.kind.map(kind_key);
    let total = tx.query_row("SELECT count(*) FROM index_scan_notices WHERE root_id=?1 AND sequence<=?2 AND (?3 IS NULL OR kind=?3) AND (?4 IS NULL OR instr(path,?4)>0 OR instr(message,?4)>0)", params![query.root_id,through_sql,kind,query.contains], |r| counted(r,0)).map_err(|e| db_error(&e))?;
    let mut notices = {
        let mut statement = tx.prepare("SELECT sequence,kind,path,message FROM index_scan_notices WHERE root_id=?1 AND sequence>?2 AND sequence<=?3 AND (?4 IS NULL OR kind=?4) AND (?5 IS NULL OR instr(path,?5)>0 OR instr(message,?5)>0) ORDER BY sequence LIMIT ?6").map_err(|e| db_error(&e))?;
        statement
            .query_map(
                params![
                    query.root_id,
                    after_sql,
                    through_sql,
                    kind,
                    query.contains,
                    i64::try_from(query.limit + 1).expect("bounded page")
                ],
                |r| {
                    Ok(ScanNotice {
                        sequence: counted(r, 0)?,
                        kind: parse_kind(r.get(1)?)?,
                        path: r.get(2)?,
                        message: r.get(3)?,
                    })
                },
            )
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?
    };
    let more = notices.len() > query.limit;
    notices.truncate(query.limit);
    let next_cursor = more.then(|| NoticeCursor {
        scan_id,
        after: notices.last().expect("nonempty page").sequence,
        through,
        filter_key: key,
    });
    Ok(NoticePage {
        root_id: root.id,
        scan_id: Some(scan_id),
        status,
        total,
        has_newer: through < newest,
        counts,
        notices,
        next_cursor,
    })
}

fn kind_key(kind: NoticeKind) -> i64 {
    match kind {
        NoticeKind::Skipped => 0,
        NoticeKind::Unreadable => 1,
        NoticeKind::Failed => 2,
        NoticeKind::Partial => 3,
    }
}

fn parse_kind(kind: i64) -> rusqlite::Result<NoticeKind> {
    match kind {
        0 => Ok(NoticeKind::Skipped),
        1 => Ok(NoticeKind::Unreadable),
        2 => Ok(NoticeKind::Failed),
        3 => Ok(NoticeKind::Partial),
        _ => Err(rusqlite::Error::IntegralValueOutOfRange(1, kind)),
    }
}

fn counted(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
