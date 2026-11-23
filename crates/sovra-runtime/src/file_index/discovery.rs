//! One local, resumable worker per index. Regular scans never wait for analysis.
use super::{Event, FileIndex, IndexError, IndexEvent, Sender};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;

pub(super) fn start(
    store: Arc<dyn FileIndex>,
    events: Sender<Event>,
    handle: &tokio::runtime::Handle,
) -> JoinHandle<()> {
    handle.spawn(async move {
        let mut revision = None;
        let mut published = Instant::now();
        let mut last_error = None;
        loop {
            let reader = store.clone();
            let publish = revision.is_none() || published.elapsed() >= Duration::from_millis(700);
            let result = tokio::task::spawn_blocking(move || {
                let Some(index) = reader.automatic() else {
                    return Ok((false, None));
                };
                let work = index.advance()?;
                let overview = if publish || !work {
                    index.overview(revision)?
                } else {
                    None
                };
                Ok::<_, IndexError>((work, overview))
            })
            .await;
            let more = match result {
                Ok(Ok((more, overview))) => {
                    last_error = None;
                    if let Some(overview) = overview {
                        revision = Some(overview.revision);
                        published = Instant::now();
                        if events
                            .send(Event::FileIndex(IndexEvent::Discoveries(Ok(overview))))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    more
                }
                failure => {
                    let error = match failure {
                        Ok(Err(error)) => error,
                        _ => IndexError("Local knowledge analysis was interrupted.".into()),
                    };
                    if last_error.as_ref() != Some(&error) {
                        last_error = Some(error.clone());
                        if events
                            .send(Event::FileIndex(IndexEvent::Discoveries(Err(error))))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    revision = None;
                    false
                }
            };
            tokio::time::sleep(if more {
                Duration::from_millis(15)
            } else {
                Duration::from_secs(2)
            })
            .await;
        }
    })
}
