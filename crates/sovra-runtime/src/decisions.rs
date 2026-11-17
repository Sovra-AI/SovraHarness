//! Decision IO runs inside the auth actor, ordered with account changes.
//! A successful event always follows a committed store write; no transient saves.

use sovra_core::decisions::{
    DECISION_PAGE_SIZE, DecisionCommand, DecisionEvent, DecisionPerson, DecisionRecord,
};
use sovra_core::{AuthSession, MemoryStore};

pub(crate) async fn run(
    command: DecisionCommand,
    session: Option<&AuthSession>,
    store: Option<&dyn MemoryStore>,
) -> DecisionEvent {
    match execute(command, session, store).await {
        Ok(event) => event,
        Err(message) => DecisionEvent::Error(message),
    }
}

async fn execute(
    command: DecisionCommand,
    session: Option<&AuthSession>,
    store: Option<&dyn MemoryStore>,
) -> Result<DecisionEvent, String> {
    let by = DecisionPerson::from(
        session.ok_or("Sign in before opening or changing company decisions.")?,
    );
    let store = store.ok_or("Decision storage is unavailable. Your draft has not been saved.")?;
    let now = crate::now();
    let (next, expected) = match command {
        DecisionCommand::Load { offset } => {
            let mut records = store
                .list_decisions(offset)
                .await
                .map_err(|error| error.to_string())?;
            let more = records.len() > DECISION_PAGE_SIZE;
            records.truncate(DECISION_PAGE_SIZE);
            return Ok(DecisionEvent::Listed {
                records,
                offset,
                more,
            });
        }
        DecisionCommand::Open { id } => {
            return Ok(DecisionEvent::Opened(Box::new(get(store, &id).await?)));
        }
        DecisionCommand::Save {
            id,
            expected_revision,
            draft,
        } => {
            let record = if let Some(id) = id {
                get(store, &id)
                    .await?
                    .edit(expected_revision, *draft, by, now)
            } else {
                if expected_revision != 0 {
                    return Err("A new decision must start without a saved revision.".into());
                }
                // Random IDs avoid collisions across restarts or clocks moving backwards.
                let id = format!("decision-{}", crate::private_storage::random_secret()?);
                DecisionRecord::create(id, *draft, by, now)
            }
            .map_err(|error| error.to_string())?;
            (record, expected_revision)
        }
        DecisionCommand::Accept {
            id,
            expected_revision,
        } => (
            get(store, &id)
                .await?
                .accept(expected_revision, by, now)
                .map_err(|error| error.to_string())?,
            expected_revision,
        ),
        DecisionCommand::End {
            id,
            expected_revision,
            reason,
        } => (
            get(store, &id)
                .await?
                .end(expected_revision, reason, by, now)
                .map_err(|error| error.to_string())?,
            expected_revision,
        ),
    };
    store
        .save_decision(&next, expected)
        .await
        .map_err(|error| format!("The decision was not saved: {error}"))?;
    Ok(DecisionEvent::Saved(Box::new(next)))
}

async fn get(store: &dyn MemoryStore, id: &str) -> Result<DecisionRecord, String> {
    store
        .decision(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This decision could not be found. Reopen the list and try again.".into())
}
