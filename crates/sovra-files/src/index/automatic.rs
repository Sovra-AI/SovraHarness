//! Restartable, version-bound local discovery. Analyze saved data outside the write
//! transaction; publish only if the grant, path, version and cursor still match.
use super::{
    BatchWrites as _, LocalFileIndex, PASSAGES, db_error,
    derived::{decode, json},
    graph, passage,
};
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior, params};
use sovra_core::{
    file_index::{
        EvidenceTier, IndexError, PassageRef,
        automatic::{
            AutomaticIndex, DiscoveredFinding, DiscoveryOverview, FindingPage, source_claim,
        },
        graph::{GraphDraft, GraphOrigin, RelationKind},
        ontology, passage_wording,
    },
    knowledge_analysis::{self, Finding, FindingKind, mask::Mask},
};

type Result<T> = std::result::Result<T, IndexError>;

struct Job {
    file: String,
    path: String,
    version: Option<String>,
    mime: Option<String>,
    ordinal: u32,
    offset: u32,
}

const JOBS: u32 = 12;

impl AutomaticIndex for LocalFileIndex {
    fn advance(&self) -> Result<bool> {
        let mut db = self.connect()?;
        let backfilling = backfill(&mut db)?;
        let pending_sql =
            "SELECT f.id, f.path, f.current_version, v.mime, a.next_ordinal, a.byte_offset
            FROM index_auto_files a JOIN index_files f ON f.id=a.file_id
            LEFT JOIN index_versions v ON v.id=f.current_version
            WHERE a.next_ordinal IS NOT NULL ORDER BY a.next_ordinal, a.rowid LIMIT ?1";
        let jobs = db
            .prepare(pending_sql)
            .map_err(|e| db_error(&e))?
            .query_map([JOBS], |row| {
                Ok(Job {
                    file: row.get(0)?,
                    path: row.get(1)?,
                    version: row.get(2)?,
                    mime: row.get(3)?,
                    ordinal: row.get(4)?,
                    offset: row.get(5)?,
                })
            })
            .map_err(|e| db_error(&e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| db_error(&e))?;
        let worked = !jobs.is_empty();
        for job in jobs {
            process(&mut db, &job)?;
        }
        Ok(worked || backfilling)
    }

    fn findings(&self, kind: FindingKind, before: Option<i64>) -> Result<FindingPage> {
        findings(&self.connect()?, kind, before, 20)
    }

    fn overview(&self, since: Option<u64>) -> Result<Option<DiscoveryOverview>> {
        let mut db = self.connect()?;
        let tx = db.transaction().map_err(|e| db_error(&e))?;
        let (revision, backfilling): (u64, bool) = tx
            .query_row(
                "SELECT revision,through_row<last_old_row FROM index_auto_state",
                [],
                |r| Ok((super::derived::unsigned(r, 0)?, r.get(1)?)),
            )
            .map_err(|e| db_error(&e))?;
        if since == Some(revision) {
            return Ok(None);
        }
        let working = backfilling
            || tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM index_auto_files WHERE next_ordinal IS NOT NULL)",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| db_error(&e))?;
        let mut overview = DiscoveryOverview {
            revision,
            roots: tx
                .prepare("SELECT id FROM index_roots ORDER BY id")
                .map_err(|e| db_error(&e))?
                .query_map([], |r| r.get(0))
                .map_err(|e| db_error(&e))?
                .collect::<rusqlite::Result<_>>()
                .map_err(|e| db_error(&e))?,
            working,
            ..Default::default()
        };
        let mut counts = tx
            .prepare("SELECT facet,label,n FROM index_auto_counts WHERE n>0 ORDER BY n DESC,label")
            .map_err(|e| db_error(&e))?;
        let rows = counts
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    super::derived::unsigned(r, 2)?,
                ))
            })
            .map_err(|e| db_error(&e))?;
        for row in rows {
            let (facet, label, n) = row.map_err(|e| db_error(&e))?;
            match facet.as_str() {
                "kind" => overview.kinds.push((decode(&label)?, n)),
                "area" => overview.areas.push((decode(&label)?, n)),
                "subject" => overview.subjects.push((decode(&label)?, n)),
                "predicate" => overview.predicates.push((decode(&label)?, n)),
                "finding" => {
                    let kind = decode(&label)?;
                    overview.counts.push((kind, n));
                    overview
                        .examples
                        .extend(findings(&tx, kind, None, 2)?.items);
                }
                _ => return Err(IndexError("Unknown automatic knowledge category.".into())),
            }
        }
        Ok(Some(overview))
    }
}

fn findings(
    db: &Connection,
    kind: FindingKind,
    before: Option<i64>,
    limit: u32,
) -> Result<FindingPage> {
    let sql = "SELECT f.root_id, f.id, f.current_version, f.path, d.ordinal, d.line, d.detail, d.id
        FROM index_auto_findings d JOIN index_files f ON f.id=d.file_id
        WHERE d.category=?1 AND d.id<?2 AND f.available=1 ORDER BY d.id DESC LIMIT ?3";
    let mut query = db.prepare_cached(sql).map_err(|e| db_error(&e))?;
    let rows = query
        .query_map(
            params![json(&kind)?, before.unwrap_or(i64::MAX), limit + 1],
            |r| {
                Ok((
                    r.get::<_, i64>(7)?,
                    DiscoveredFinding {
                        kind,
                        reference: PassageRef {
                            root_id: r.get(0)?,
                            file_id: r.get(1)?,
                            version_id: r.get(2)?,
                            ordinal: r.get(4)?,
                        },
                        path: r.get(3)?,
                        line: r.get(5)?,
                        detail: r.get(6)?,
                    },
                ))
            },
        )
        .map_err(|e| db_error(&e))?;
    let mut page = FindingPage::default();
    let mut last = None;
    for item in rows {
        let (id, finding) = item.map_err(|e| db_error(&e))?;
        if page.items.len() == limit as usize {
            page.next = last;
            break;
        }
        last = Some(id);
        page.items.push(finding);
    }
    Ok(page)
}

fn backfill(db: &mut Connection) -> Result<bool> {
    let pending = db
        .query_row(
            "SELECT through_row<last_old_row FROM index_auto_state",
            [],
            |r| r.get::<_, bool>(0),
        )
        .map_err(|e| db_error(&e))?;
    if !pending {
        return Ok(false);
    }
    let full = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM index_auto_files WHERE next_ordinal IS NOT NULL LIMIT 1 OFFSET 127)",
        [], |r|r.get::<_,bool>(0),
    ).map_err(|e| db_error(&e))?;
    if full {
        return Ok(true);
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| db_error(&e))?;
    let (start, end): (i64, i64) = tx
        .query_row(
            "SELECT through_row,last_old_row FROM index_auto_state",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| db_error(&e))?;
    let boundary = "SELECT rowid FROM index_files WHERE rowid>?1 AND rowid<=?2 ORDER BY rowid LIMIT 1 OFFSET 127";
    let stop = tx
        .query_row(boundary, params![start, end], |r| r.get::<_, i64>(0))
        .optional()
        .map_err(|e| db_error(&e))?
        .unwrap_or(end);
    let enqueue = "INSERT OR IGNORE INTO index_auto_files(file_id)
        SELECT id FROM index_files WHERE rowid>?1 AND rowid<=?2 AND available=1";
    tx.execute(enqueue, params![start, stop])
        .map_err(|e| db_error(&e))?;
    tx.execute(
        "UPDATE index_auto_state SET through_row=?1,revision=revision+1",
        [stop],
    )
    .map_err(|e| db_error(&e))?;
    tx.commit().map_err(|e| db_error(&e))?;
    Ok(stop < end)
}

struct Reading {
    findings: Vec<(u32, Finding)>,
    claims: Vec<(GraphDraft, EvidenceTier)>,
    next: Option<(u32, u32)>,
}

fn read(db: &Connection, job: &Job) -> Result<Reading> {
    let mut result = Reading {
        findings: vec![],
        claims: vec![],
        next: None,
    };
    let Some(version) = &job.version else {
        return Ok(result);
    };
    let sql = format!(
        "{PASSAGES} WHERE f.id=?1 AND v.id=?2 AND f.current_version=v.id
        AND vc.ordinal>=?3 AND f.available=1 ORDER BY vc.ordinal LIMIT 33"
    );
    let mut query = db.prepare(&sql).map_err(|e| db_error(&e))?;
    let rows = query
        .query_map(params![job.file, version, job.ordinal], passage)
        .map_err(|e| db_error(&e))?;
    let mut parts = Vec::new();
    let mut lines = 0;
    let mut text = String::new();
    for row in rows {
        let part = row.map_err(|e| db_error(&e))?;
        let body = passage_wording(&part.text);
        let offset = if part.reference.ordinal == job.ordinal {
            job.offset as usize
        } else {
            0
        };
        let Some(wording) = body.get(offset..) else {
            return Err(IndexError("Invalid saved analysis offset.".into()));
        };
        if wording.is_empty() {
            continue;
        }
        if parts.len() >= 32
            || text.len() + wording.len() > knowledge_analysis::MAX_ANALYSIS_BYTES - 2048
        {
            result.next = Some((part.reference.ordinal, 0));
            break;
        }
        parts.push((
            text.len(),
            part.start_line
                + u32::try_from(body[..offset].bytes().filter(|b| *b == b'\n').count())
                    .unwrap_or(0),
            part.reference.clone(),
            offset,
        ));
        // A short window bounds graph writes and keeps long relation tables resumable.
        let budget = 128usize.saturating_sub(lines);
        if let Some((last, _)) = wording.match_indices('\n').nth(budget.saturating_sub(1)) {
            text.push_str(&wording[..=last]);
            result.next = Some((
                part.reference.ordinal,
                u32::try_from(offset + last + 1).expect("bounded passage offset"),
            ));
            break;
        }
        lines += wording.bytes().filter(|b| *b == b'\n').count();
        text.push_str(wording);
    }
    let Some((_, first_line, first_ref, _)) = parts.first() else {
        return Ok(result);
    };
    // When a content-defined chunk ends mid-line, finish that line on the next pass.
    if result.next.is_some()
        && !text.ends_with('\n')
        && let Some(at) = text.rfind('\n')
    {
        let boundary = at + 1;
        if let Some((pos, _, reference, offset)) =
            parts.iter().rev().find(|(pos, _, _, _)| *pos <= boundary)
        {
            result.next = Some((
                reference.ordinal,
                u32::try_from(offset + boundary - pos).expect("bounded passage offset"),
            ));
            text.truncate(boundary);
        }
    }
    let source_bytes = text.len();
    let mut header_lines = 0;
    let mut header_bytes = 0;
    if *first_line > 1
        && matches!(
            job.mime.as_deref(),
            Some("text/csv" | "text/tab-separated-values" | "text/markdown")
        )
    {
        let header_sql =
            "SELECT c.body FROM index_version_chunks vc JOIN index_chunks c ON c.id=vc.chunk_id
            WHERE vc.version_id=?1 AND vc.ordinal=0";
        let first: String = db
            .query_row(header_sql, [version], |r| r.get(0))
            .map_err(|e| db_error(&e))?;
        let mut lines = passage_wording(&first).lines();
        if let Some(first) = lines.next().filter(|s| s.len() < 1024) {
            let prefix = if first.contains('|') {
                lines
                    .next()
                    .filter(|s| {
                        s.len() < 1024
                            && s.contains('-')
                            && s.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
                    })
                    .map(|rule| format!("{first}\n{rule}\n"))
            } else if first.contains([',', '\t']) {
                Some(format!("{first}\n"))
            } else {
                None
            };
            if let Some(prefix) = prefix {
                header_lines = u32::try_from(prefix.lines().count()).expect("two header lines");
                header_bytes = prefix.len();
                text.insert_str(0, &prefix);
            }
        }
    }
    let mut analysis =
        knowledge_analysis::analyze_indexed(&text).map_err(|e| IndexError(e.into()))?;
    if job.ordinal == 0 && job.offset == 0 {
        let mask: Option<(u16, u16, Vec<u8>)> = db
            .query_row(
                "SELECT width,height,bits FROM index_masks WHERE version_id=?1",
                [version],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| db_error(&e))?;
        if let Some((w, h, b)) = mask
            && let Some(mask) = Mask::unpack(w.into(), h.into(), &b)
        {
            let detail = mask.describe(1).map_err(|e| IndexError(e.into()))?;
            analysis.findings.push(Finding {
                kind: FindingKind::Morphology,
                from: 1,
                to: 1,
                detail,
            });
        }
    }
    let absolute = |local: u32| first_line.saturating_add(local.saturating_sub(1 + header_lines));
    let mut positions = vec![0];
    positions.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let source_line = |line: u32| {
        let i = line.saturating_sub(1) as usize;
        let start = positions.get(i).copied().unwrap_or(0);
        let end = positions.get(i + 1).copied().unwrap_or(text.len());
        let low = start.saturating_sub(header_bytes);
        let high = end.saturating_sub(header_bytes);
        let mut refs = Vec::new();
        for (i, (pos, _, reference, _)) in parts.iter().enumerate() {
            let stop = parts.get(i + 1).map_or(source_bytes, |p| p.0);
            if *pos < high && stop > low {
                refs.push(reference.clone());
            }
        }
        if refs.is_empty() {
            refs.push(first_ref.clone());
        }
        (text.get(start..end).unwrap_or_default().trim(), refs)
    };
    for claim in analysis.statements {
        let (wording, refs) = source_line(claim.line);
        if let Some(mut draft) = source_claim(&claim, &refs[0], wording) {
            draft.evidence = refs;
            let indices: Vec<_> =
                (0..u8::try_from(draft.evidence.len()).expect("bounded passage count")).collect();
            for entity in &mut draft.entities {
                entity.evidence.clone_from(&indices);
            }
            draft.relations[0].evidence = indices;
            if draft.validate().is_ok() {
                result.claims.push((
                    draft,
                    if claim.explicit {
                        EvidenceTier::DeterministicExtraction
                    } else {
                        EvidenceTier::HeuristicExtraction
                    },
                ));
            }
        }
    }
    for mut finding in analysis.findings {
        let ordinal = source_line(finding.from).1[0].ordinal;
        finding.from = absolute(finding.from);
        finding.to = absolute(finding.to);
        result.findings.push((ordinal, finding));
    }
    Ok(result)
}

fn process(db: &mut Connection, job: &Job) -> Result<()> {
    let read_tx = db.transaction().map_err(|e| db_error(&e))?;
    let reading = read(&read_tx, job)?;
    read_tx.commit().map_err(|e| db_error(&e))?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| db_error(&e))?;
    let kind = ontology::classify(job.mime.as_deref(), &job.path);
    let area = ontology::life_area(&job.path);
    let claim_sql="UPDATE index_auto_files SET kind=?1,area=?2,next_ordinal=?3,byte_offset=?4
        WHERE file_id=?5 AND next_ordinal=?6 AND byte_offset=?7
        AND EXISTS(SELECT 1 FROM index_files f WHERE f.id=?5 AND f.path=?8 AND f.current_version IS ?9 AND f.available=1)";
    let changed = tx
        .execute(
            claim_sql,
            params![
                json(&kind)?,
                area.map(|a| json(&a)).transpose()?,
                reading.next.map(|v| v.0),
                reading.next.map_or(0, |v| v.1),
                job.file,
                job.ordinal,
                job.offset,
                job.path,
                job.version,
            ],
        )
        .map_err(|e| db_error(&e))?;
    if changed == 0 {
        return Ok(());
    }
    for (ordinal, item) in reading.findings {
        let insert = "INSERT INTO index_auto_findings(file_id,ordinal,line,category,detail) VALUES(?1,?2,?3,?4,?5)";
        tx.execute_cached(
            insert,
            params![job.file, ordinal, item.from, json(&item.kind)?, item.detail],
        )
        .map_err(|e| db_error(&e))?;
    }
    let origin = GraphOrigin {
        session_id: "local-file-index".into(),
        invocation_id: "source-analysis-v1".into(),
        agent: "Automatic file analysis".into(),
    };
    for (draft, tier) in reading.claims {
        if let RelationKind::Property(prop) = &draft.relations[0].relation
            && graph::write::property_conflicts(&tx, prop, &draft.relations[0].relation)?
        {
            continue;
        }
        if graph::write::automatic_cycle(&tx, &draft)? {
            continue;
        }
        let saved = graph::write::propose(&tx, &draft, &origin, Some(tier))?;
        for entity in &draft.entities {
            tx.execute_cached(
                "INSERT OR IGNORE INTO index_auto_terms VALUES(?1,'subject',?2,?3)",
                params![job.file, json(&entity.kind)?, entity.label],
            )
            .map_err(|e| db_error(&e))?;
        }
        tx.execute_cached(
            "INSERT OR IGNORE INTO index_auto_terms VALUES(?1,'predicate',?2,?3)",
            params![
                job.file,
                json(&draft.relations[0].relation)?,
                saved.id.to_string()
            ],
        )
        .map_err(|e| db_error(&e))?;
    }
    tx.execute("UPDATE index_auto_state SET revision=revision+1", [])
        .map_err(|e| db_error(&e))?;
    tx.commit().map_err(|e| db_error(&e))
}
