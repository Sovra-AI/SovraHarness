//! A grant owns each quoted snapshot. Removing the grant deletes the saved quotation.

use super::{LocalFileIndex, PASSAGES, db_error, derived, passage};
use rusqlite::{OptionalExtension as _, params};
use sovra_core::file_index::{DatedAnswer, IndexError, PassageRef, SavedAnswerPage};

impl LocalFileIndex {
    pub(super) fn store_answer(&self, reference: &PassageRef) -> Result<DatedAnswer, IndexError> {
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| db_error(&e))?;
        let key = params![
            reference.root_id,
            reference.file_id,
            reference.version_id,
            reference.ordinal
        ];
        let prior: Option<String> = tx.query_row(
            "SELECT snapshot FROM index_saved_answers WHERE root_id=?1 AND file_id=?2 AND version_id=?3 AND ordinal=?4",
            key, |row| row.get(0),
        ).optional().map_err(|e| db_error(&e))?;
        if let Some(prior) = prior {
            return derived::decode(&prior);
        }
        let source = tx.query_row(
            &format!("{PASSAGES} WHERE f.root_id=?1 AND f.id=?2 AND v.id=?3 AND vc.ordinal=?4 AND f.available=1 AND f.current_version=v.id"),
            key, passage,
        ).optional().map_err(|e| db_error(&e))?.ok_or_else(|| IndexError("This reading changed or its folder was removed. Search again to save it.".into()))?;
        let saved = DatedAnswer::save(&source)?;
        tx.execute("INSERT INTO index_saved_answers(root_id,file_id,version_id,ordinal,snapshot) VALUES(?1,?2,?3,?4,?5)",
            params![reference.root_id,reference.file_id,reference.version_id,reference.ordinal,derived::json(&saved)?]).map_err(|e| db_error(&e))?;
        tx.commit().map_err(|e| db_error(&e))?;
        Ok(saved)
    }

    pub(super) fn answer_page(&self, before: Option<u64>) -> Result<SavedAnswerPage, IndexError> {
        const PAGE_SIZE: usize = 20;
        let db = self.connect()?;
        let mut stmt = db.prepare("SELECT sequence,snapshot FROM index_saved_answers WHERE sequence < ?1 ORDER BY sequence DESC LIMIT 21").map_err(|e| db_error(&e))?;
        let mut rows = stmt
            .query([before.map(derived::number).transpose()?.unwrap_or(i64::MAX)])
            .map_err(|e| db_error(&e))?;
        let mut page = SavedAnswerPage::default();
        let mut last = None;
        while let Some(row) = rows.next().map_err(|e| db_error(&e))? {
            if page.answers.len() == PAGE_SIZE {
                page.next_before = last;
                break;
            }
            last = Some(derived::unsigned(row, 0).map_err(|e| db_error(&e))?);
            page.answers.push(derived::decode(
                &row.get::<_, String>(1).map_err(|e| db_error(&e))?,
            )?);
        }
        Ok(page)
    }
}
