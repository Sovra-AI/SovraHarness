//! Indexed keyset walk for the whole-knowledge map. Only count passages for the
//! page being opened. The inventory's whole-folder statistics would rescan it.
use super::{db_error, derived::decode, get_root};
use rusqlite::{Connection, params};
use sovra_core::{
    file_index::{
        IndexError, PassageRef,
        ontology::{FileKind, LifeArea},
    },
    knowledge_map::{Branch, DetailLevel, Layer, MapPage},
};

pub(super) fn folder_page(
    db: &Connection,
    root: &str,
    after: &str,
    history: bool,
) -> Result<MapPage, IndexError> {
    get_root(db, root)?;
    let mut query = db
        .prepare_cached(
            "SELECT f.id,f.path,CASE WHEN f.available THEN f.current_version END,a.kind,a.area,
        (SELECT count(*) FROM index_version_chunks vc WHERE vc.version_id=f.current_version)
        FROM index_files f LEFT JOIN index_auto_files a ON a.file_id=f.id
        WHERE f.root_id=?1 AND f.path>?2 AND (?3 OR f.available=1) ORDER BY f.path LIMIT 129",
        )
        .map_err(|e| db_error(&e))?;
    let mut rows = query
        .query(params![root, after, history])
        .map_err(|e| db_error(&e))?;
    let mut page = MapPage::default();
    let mut last = String::new();
    let mut next = None;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| db_error(&e))? {
        if count == 128 {
            next = Some(Branch::Files {
                root: root.into(),
                after: last,
            });
            break;
        }
        let (file, path, version, kind, area, parts): (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            u32,
        ) = (|| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })()
        .map_err(|e| db_error(&e))?;
        let id = page.file(root, &file, &path);
        if history {
            let key = page.node(
                Layer::Files,
                format!("history:{file}"),
                "File history",
                &path,
            );
            page.link(&id, &key, "has observations", true);
            page.branch(
                &key,
                Some(Branch::Observations(
                    sovra_core::file_index::FileHistoryQuery {
                        root_id: root.into(),
                        file_id: Some(file.clone()),
                        ..Default::default()
                    },
                )),
            );
        }
        if let Some(version_id) = version
            && parts > 0
        {
            page.branch(
                &id,
                Some(Branch::Parts(
                    PassageRef {
                        root_id: root.into(),
                        file_id: file,
                        version_id,
                        ordinal: 0,
                    },
                    parts,
                )),
            );
        }
        let kind = kind
            .map(|s| decode::<FileKind>(&s))
            .transpose()?
            .map(|k| k.concept().pref_label);
        let area = area
            .map(|s| decode::<LifeArea>(&s))
            .transpose()?
            .map(|a| a.concept().pref_label);
        for (facet, label) in [("File kind", kind), ("Life area", area)] {
            if let Some(label) = label {
                let key = page.describe(
                    Layer::Files,
                    format!("{facet}:{label}"),
                    label,
                    facet,
                    DetailLevel::Reference,
                );
                page.link(&id, &key, facet, false);
            }
        }
        last = path;
        count += 1;
    }
    page.branch(&format!("root:{root}"), next);
    Ok(page)
}
