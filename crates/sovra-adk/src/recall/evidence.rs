//! A shared byte budget for separately labeled, ephemeral lookup results.

use super::{MAX_MEMORY_CONTEXT_BYTES, MemoryContext, MemorySearch};

#[derive(serde::Serialize)]
struct LookupEvidence {
    query: MemorySearch,
    evidence: MemoryContext,
}

#[derive(serde::Serialize)]
struct Batch {
    lookups: Vec<LookupEvidence>,
}

#[derive(serde::Serialize)]
struct Single<'a> {
    query: &'a MemorySearch,
    #[serde(flatten)]
    evidence: &'a MemoryContext,
}

fn serialize(batch: &Batch) -> Result<String, serde_json::Error> {
    if let [entry] = batch.lookups.as_slice() {
        serde_json::to_string(&Single {
            query: &entry.query,
            evidence: &entry.evidence,
        })
    } else {
        serde_json::to_string(batch)
    }
}

pub(super) fn unavailable() -> String {
    serde_json::to_string(&MemoryContext::unavailable())
        .unwrap_or_else(|_| "{\"notice\":\"Saved evidence is unavailable.\"}".into())
}

pub(super) fn encode(queries: Vec<MemorySearch>, contexts: Vec<MemoryContext>) -> String {
    if queries.len() != contexts.len() || sovra_core::validate_memory_searches(&queries).is_err() {
        return unavailable();
    }
    let mut batch = Batch {
        lookups: queries
            .into_iter()
            .zip(contexts)
            .map(|(query, evidence)| LookupEvidence { query, evidence })
            .collect(),
    };
    loop {
        if let Ok(data) = serialize(&batch)
            && data.len() <= MAX_MEMORY_CONTEXT_BYTES
        {
            return data;
        }
        let largest = batch
            .lookups
            .iter_mut()
            .filter(|entry| {
                !entry.evidence.background.is_empty()
                    || !entry.evidence.sources.is_empty()
                    || !entry.evidence.memories.is_empty()
            })
            .max_by_key(|entry| {
                serde_json::to_vec(&entry.evidence).map_or(usize::MAX, |data| data.len())
            });
        let Some(entry) = largest else {
            return unavailable();
        };
        // Drop whole low-priority records, never cut a quote or alter its meaning.
        if entry.evidence.background.pop().is_none() && entry.evidence.sources.pop().is_none() {
            entry.evidence.memories.pop();
        }
        entry.evidence.notice = Some(
            "Only a bounded selection fits in this lookup. Additional saved evidence may exist."
                .into(),
        );
    }
}
