//! Append-only observation lineage; historical reads require a still-granted root.
//! Missing paths mean absence from a completed eligible walk, not proven deletion.

use super::{db_error, digest, now};
use rusqlite::{Connection, OptionalExtension as _, Row, params};
use sovra_core::file_index::{
    Confidence, EvidenceBasis, EvidenceTier, ExtractionState, FileHistory, FileHistoryQuery,
    FileObservation, HistoricalPassage, HistoryPassageRef, IndexError, IndexStatus,
    ObservedPresence, ScanActivity, SourceRepresentation,
};

pub(super) const EXTRACTOR_REVISION: &str = "extract-v2";

pub(super) fn migrate(db: &Connection) -> Result<(), IndexError> {
    db.execute_batch(include_str!("history_v4.sql"))
        .map_err(|e| db_error(&e))?;
    db.execute(
        "INSERT INTO index_observations(file_id,version_id,path,identity,present,byte_size,modified,created,extraction,note,recorded,observed,origin)
         SELECT id,current_version,path,identity,available,byte_size,modified,created,extraction,detail,?1,NULL,'legacy_import' FROM index_files",
        [time()],
    ).map_err(|e| db_error(&e))?;
    Ok(())
}

pub(super) fn begin(db: &Connection, root_id: &str) -> Result<i64, IndexError> {
    db.execute("INSERT INTO index_scans(root_id,started,status,extractor_revision) VALUES(?1,?2,'Scanning',?3)",
        params![root_id,time(),EXTRACTOR_REVISION]).map_err(|e| db_error(&e))?;
    Ok(db.last_insert_rowid())
}

pub(super) fn finish(db: &Connection, id: i64, status: IndexStatus) -> Result<(), IndexError> {
    let status =
        serde_json::to_value(status).map_err(|_| IndexError("Invalid scan status.".into()))?;
    db.execute(
        "UPDATE index_scans SET finished=?2,status=?3 WHERE id=?1 AND finished IS NULL",
        params![
            id,
            time(),
            status.as_str().expect("unit enum serializes to text")
        ],
    )
    .map_err(|e| db_error(&e))?;
    Ok(())
}

pub(super) fn recover(db: &Connection) -> Result<(), IndexError> {
    db.execute(
        "UPDATE index_scans SET status='Interrupted',finished=?1 WHERE finished IS NULL",
        [time()],
    )
    .map_err(|e| db_error(&e))?;
    Ok(())
}

pub(super) fn observe(db: &Connection, file_id: &str, activity: i64) -> Result<(), IndexError> {
    // Repeated scans update the catalogue's last-seen time without bloating history.
    // Each meaningful transition atomically points at its immutable predecessor.
    db.execute(
        "INSERT INTO index_observations(file_id,scan_id,version_id,path,identity,present,byte_size,modified,created,extraction,note,recorded,observed,origin,replaces)
         SELECT f.id,?2,f.current_version,f.path,f.identity,f.available,f.byte_size,f.modified,f.created,f.extraction,f.detail,?3,f.observed,'scan',o.sequence
         FROM index_files f LEFT JOIN index_observations o ON o.sequence=(SELECT max(sequence) FROM index_observations WHERE file_id=f.id)
         WHERE f.id=?1 AND (o.sequence IS NULL OR o.origin='legacy_import' OR o.version_id IS NOT f.current_version OR o.path IS NOT f.path
         OR o.identity IS NOT f.identity OR o.present IS NOT f.available OR o.byte_size IS NOT f.byte_size OR o.modified IS NOT f.modified
         OR o.created IS NOT f.created OR o.extraction IS NOT f.extraction OR o.note IS NOT f.detail)",
        params![file_id,activity,time()],
    ).map_err(|e| db_error(&e))?;
    Ok(())
}

fn time() -> i64 {
    i64::try_from(now()).expect("current Unix seconds fit i64")
}

fn bounded(db: &mut Connection) -> Result<(), IndexError> {
    let start = std::time::Instant::now();
    db.progress_handler(
        1000,
        Some(move || start.elapsed() > std::time::Duration::from_secs(5)),
    )
    .map_err(|e| db_error(&e))
}

pub(super) fn query(
    db: &mut Connection,
    query: &FileHistoryQuery,
) -> Result<FileHistory, IndexError> {
    query.validate()?;
    bounded(db)?;
    let tx = db.transaction().map_err(|e| db_error(&e))?;
    let root = super::get_root(&tx, &query.root_id)?;
    let file_id = resolve(&tx, query)?;
    let (path, present) = tx
        .query_row(
            "SELECT path,available FROM index_files WHERE id=?1 AND root_id=?2",
            params![file_id, query.root_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
        )
        .map_err(|e| db_error(&e))?;
    let sql = format!(
        "{OBSERVATIONS} WHERE o.file_id=?1 AND (?2 IS NULL OR o.sequence<?2) ORDER BY o.sequence DESC LIMIT ?3"
    );
    let mut statement = tx.prepare(&sql).map_err(|e| db_error(&e))?;
    let mut observations = statement
        .query_map(
            params![
                file_id,
                query
                    .before
                    .map(|v| i64::try_from(v).expect("validated cursor")),
                i64::try_from(query.limit + 1).expect("bounded history limit")
            ],
            observation,
        )
        .map_err(|e| db_error(&e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| db_error(&e))?;
    let has_more = observations.len() > query.limit;
    observations.truncate(query.limit);
    for item in &mut observations {
        if let Some(representation) = &mut item.representation
            && representation.extracted_passages > 0
        {
            representation.first_passage = Some(HistoryPassageRef {
                root_id: query.root_id.clone(),
                file_id: file_id.clone(),
                observation: item.sequence,
                ordinal: 0,
            });
        }
    }
    let next_before =
        has_more.then(|| observations.last().expect("nonempty bounded page").sequence);
    let versions_without_observation = tx.query_row(
        "SELECT count(*) FROM index_versions v WHERE v.file_id=?1 AND NOT EXISTS(SELECT 1 FROM index_observations o WHERE o.version_id=v.id)",
        [&file_id], |r| super::catalogue::counted(r,0),
    ).map_err(|e| db_error(&e))?;
    Ok(FileHistory {
        root_id: query.root_id.clone(),
        root_path: root.path.display().to_string(),
        file_id,
        current_path: path,
        currently_present: present,
        coverage: (&root).into(),
        observations,
        next_before,
        versions_without_observation,
    })
}

fn resolve(db: &Connection, query: &FileHistoryQuery) -> Result<String, IndexError> {
    if let Some(id) = &query.file_id {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM index_files WHERE id=?1 AND root_id=?2)",
                params![id, query.root_id],
                |r| r.get(0),
            )
            .map_err(|e| db_error(&e))?;
        return if exists {
            Ok(id.clone())
        } else {
            Err(IndexError(
                "This file is not in the selected indexed scope.".into(),
            ))
        };
    }
    let path = query.path.as_deref().expect("validated history selector");
    let mut statement = db.prepare(
        "SELECT id FROM index_files WHERE root_id=?1 AND path=?2 UNION SELECT f.id FROM index_observations o JOIN index_files f ON f.id=o.file_id WHERE f.root_id=?1 AND o.path=?2 LIMIT 2"
    ).map_err(|e| db_error(&e))?;
    let matches = statement
        .query_map(params![query.root_id, path], |r| r.get::<_, String>(0))
        .map_err(|e| db_error(&e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| db_error(&e))?;
    match matches.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(IndexError("No observed history matches that path in this scope.".into())),
        _ => Err(IndexError("This path refers to multiple tracked files. Use an exact file_id from the catalogue or a previous citation.".into())),
    }
}

const OBSERVATIONS: &str = "SELECT o.sequence,o.path,o.identity,o.present,o.byte_size,o.modified,o.created,o.extraction,o.note,o.recorded,o.observed,o.origin,o.replaces,
 s.id,s.started,s.finished,s.status,s.extractor_revision,v.id,v.digest,v.mime,v.observed,v.extractor_revision,v.generated_by,
 (SELECT count(*) FROM index_version_chunks WHERE version_id=v.id)
 FROM index_observations o LEFT JOIN index_scans s ON s.id=o.scan_id LEFT JOIN index_versions v ON v.id=o.version_id";

fn observation(row: &Row<'_>) -> rusqlite::Result<FileObservation> {
    use super::catalogue::{counted, optional_counted};
    let sequence = counted(row, 0)?;
    let origin: String = row.get(11)?;
    let tier = match origin.as_str() {
        "scan" => EvidenceTier::FilesystemObservation,
        "legacy_import" => EvidenceTier::LegacyImport,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let activity = optional_counted(row, 13)?
        .map(|id| -> rusqlite::Result<_> {
            let status: String = row.get(16)?;
            Ok(ScanActivity {
                id,
                started_at: counted(row, 14)?,
                finished_at: optional_counted(row, 15)?,
                status: serde_json::from_value(serde_json::Value::String(status))
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                extractor_revision: row.get(17)?,
            })
        })
        .transpose()?;
    let version: Option<String> = row.get(18)?;
    let representation = version
        .map(|version_id| -> rusqlite::Result<_> {
            Ok(SourceRepresentation {
                version_id,
                content_digest: row.get(19)?,
                mime_type: row.get(20)?,
                first_indexed_at: counted(row, 21)?,
                extractor_revision: row.get(22)?,
                generated_by: optional_counted(row, 23)?,
                extracted_passages: counted(row, 24)?,
                first_passage: None,
            })
        })
        .transpose()?;
    let extraction: String = row.get(7)?;
    let extraction = match extraction.as_str() {
        "text" => ExtractionState::Text,
        "metadata_only" => ExtractionState::MetadataOnly,
        "failed" => ExtractionState::Failed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let identity: String = row.get(2)?;
    Ok(FileObservation {
        sequence,
        path: row.get(1)?,
        physical_id: (!identity.is_empty()).then(|| digest(identity.as_bytes())),
        presence: if row.get::<_, bool>(3)? {
            ObservedPresence::Present
        } else {
            ObservedPresence::NotObserved
        },
        byte_size: optional_counted(row, 4)?,
        modified_at: optional_counted(row, 5)?,
        created_at: optional_counted(row, 6)?,
        extraction,
        note: row.get(8)?,
        basis: EvidenceBasis {
            tier,
            confidence: (tier == EvidenceTier::FilesystemObservation)
                .then(|| Confidence::new(1.0).expect("unit confidence")),
            activity_id: activity.as_ref().map(|a| a.id),
            recorded_at: counted(row, 9)?,
            // Legacy versions retain their known first-indexed time, but no
            // timestamp establishes when all imported path/metadata became true.
            observed_at: if tier == EvidenceTier::LegacyImport {
                None
            } else {
                optional_counted(row, 10)?
            },
            valid_time: None,
        },
        activity,
        representation,
        replaces: optional_counted(row, 12)?,
    })
}

pub(super) fn read(
    db: &mut Connection,
    reference: &HistoryPassageRef,
) -> Result<HistoricalPassage, IndexError> {
    reference.validate()?;
    bounded(db)?;
    let tx = db.transaction().map_err(|e| db_error(&e))?;
    super::get_root(&tx, &reference.root_id)?;
    let sql = format!(
        "{OBSERVATIONS} JOIN index_files f ON f.id=o.file_id WHERE o.sequence=?1 AND o.file_id=?2 AND f.root_id=?3"
    );
    let observed = tx
        .query_row(
            &sql,
            params![
                i64::try_from(reference.observation).expect("validated observation"),
                reference.file_id,
                reference.root_id
            ],
            observation,
        )
        .optional()
        .map_err(|e| db_error(&e))?
        .ok_or_else(|| {
            IndexError("This history reference is unknown or its scope was revoked.".into())
        })?;
    let representation = observed
        .representation
        .ok_or_else(|| IndexError("This observation has no extracted text snapshot.".into()))?;
    let sql = format!(
        "{} WHERE f.root_id=?1 AND f.id=?2 AND v.id=?3 AND vc.ordinal=?4",
        super::PASSAGES
    );
    let mut passage = tx
        .query_row(
            &sql,
            params![
                reference.root_id,
                reference.file_id,
                representation.version_id,
                reference.ordinal
            ],
            super::passage,
        )
        .optional()
        .map_err(|e| db_error(&e))?
        .ok_or_else(|| IndexError("This historical passage does not exist.".into()))?;
    passage.path = observed.path;
    let next = (u64::from(reference.ordinal) + 1 < representation.extracted_passages).then(|| {
        HistoryPassageRef {
            ordinal: reference.ordinal + 1,
            ..reference.clone()
        }
    });
    Ok(HistoricalPassage {
        reference: reference.clone(),
        passage,
        basis: observed.basis,
        next,
    })
}
