//! A chat turn is given up only when it goes quiet, not when it runs long.
//!
//! The connection timeout used to cap the whole turn. A turn that searches the web
//! several times, or shares the AI with other chats running at the same time, was
//! then cut off while it was still visibly working. Every event the turn sends
//! (text, reasoning, helper steps) now restarts the clock, so only a turn that
//! shows no progress for the whole timeout is stopped.

use std::future::Future;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use sovra_core::Event;
use tokio::time::Instant;

/// Enough room that a fast stream never waits on the forwarder.
const RELAY_CAPACITY: usize = 64;

/// The turn showed no progress for the whole timeout.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stalled;

/// A sender for the turn to write to, and the receiving end for [`forward`].
pub(crate) fn relay() -> (Sender<Event>, Receiver<Event>) {
    async_channel::bounded(RELAY_CAPACITY)
}

/// Runs `run`, passing everything it sends through `relayed` on to `events` in
/// order. Fails with [`Stalled`] once `quiet` passes without a single event,
/// unless `waiting_on_person` says the turn is waiting for the person (an approval
/// card): their time to decide is not the AI going quiet.
pub(crate) async fn forward<T>(
    run: impl Future<Output = T>,
    relayed: &Receiver<Event>,
    events: &Sender<Event>,
    quiet: Duration,
    waiting_on_person: impl Fn() -> bool,
) -> Result<T, Stalled> {
    let mut run = std::pin::pin!(run);
    let mut deadline = Instant::now() + quiet;
    loop {
        tokio::select! {
            // Events first, so nothing the turn sent is lost when it finishes.
            biased;
            event = relayed.recv() => match event {
                Ok(event) => {
                    deadline = Instant::now() + quiet;
                    // A closed window keeps the turn running, as a direct send would.
                    let _ = events.send(event).await;
                }
                // Every relay sender is gone, so the turn can no longer report progress.
                Err(_) => return tokio::time::timeout_at(deadline, run).await.map_err(|_| Stalled),
            },
            output = &mut run => {
                while let Ok(event) = relayed.try_recv() {
                    let _ = events.send(event).await;
                }
                return Ok(output);
            }
            () = tokio::time::sleep_until(deadline) => {
                if !waiting_on_person() {
                    return Err(Stalled);
                }
                deadline = Instant::now() + quiet;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: Duration = Duration::from_secs(60);

    fn chunk(text: &str) -> Event {
        Event::AgentChunk {
            session_id: "chat".into(),
            text: text.into(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_keeps_working_runs_past_the_timeout() {
        let (relay, relayed) = relay();
        let (events, seen) = async_channel::unbounded();
        let started = Instant::now();
        let run = async {
            // Five minutes of work, never silent for more than 45 seconds.
            for step in 0..7 {
                tokio::time::sleep(Duration::from_secs(45)).await;
                relay.send(chunk(&step.to_string())).await.unwrap();
            }
            "answer"
        };
        assert_eq!(
            forward(run, &relayed, &events, QUIET, || false).await,
            Ok("answer")
        );
        assert!(started.elapsed() > QUIET * 5);
        let texts: Vec<_> = std::iter::from_fn(|| seen.try_recv().ok()).collect();
        assert_eq!(
            texts,
            (0..7)
                .map(|step| chunk(&step.to_string()))
                .collect::<Vec<_>>()
        );
    }

    /// Regression: a pool check was cut off with "The AI stopped responding" while its
    /// approval card was still waiting for the person to choose Allow once.
    #[tokio::test(start_paused = true)]
    async fn waiting_for_the_person_to_approve_is_not_going_quiet() {
        let (relay, relayed) = relay();
        let (events, _seen) = async_channel::unbounded();
        let deciding = std::sync::atomic::AtomicBool::new(true);
        let started = Instant::now();
        let run = async {
            relay.send(chunk("asking to run a command")).await.unwrap();
            // The person takes three timeouts to decide, then the turn goes on.
            tokio::time::sleep(QUIET * 3).await;
            deciding.store(false, std::sync::atomic::Ordering::Relaxed);
            relay.send(chunk("ran it")).await.unwrap();
            "answer"
        };
        let waiting = || deciding.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            forward(run, &relayed, &events, QUIET, waiting).await,
            Ok("answer")
        );
        assert_eq!(started.elapsed(), QUIET * 3);

        // Once nobody is being asked, silence is still stopped as before.
        let (quiet_relay, quiet_relayed) = super::relay();
        let run = async {
            quiet_relay.send(chunk("thinking")).await.unwrap();
            std::future::pending::<()>().await;
        };
        assert_eq!(
            forward(run, &quiet_relayed, &events, QUIET, || false).await,
            Err(Stalled)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_goes_quiet_is_stopped_after_the_timeout() {
        let (relay, relayed) = relay();
        let (events, seen) = async_channel::unbounded();
        let started = Instant::now();
        let run = async {
            relay.send(chunk("thinking")).await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
            relay.send(chunk("still going")).await.unwrap();
            std::future::pending::<()>().await;
        };
        assert_eq!(
            forward(run, &relayed, &events, QUIET, || false).await,
            Err(Stalled)
        );
        // Measured from the last progress, not from the start of the turn.
        assert_eq!(started.elapsed(), Duration::from_secs(70));
        assert_eq!(seen.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn events_sent_just_before_finishing_are_all_delivered_in_order() {
        let (relay, relayed) = relay();
        let (events, seen) = async_channel::unbounded();
        let run = async {
            for index in 0..RELAY_CAPACITY {
                relay.send(chunk(&index.to_string())).await.unwrap();
            }
        };
        forward(run, &relayed, &events, QUIET, || false)
            .await
            .unwrap();
        let texts: Vec<_> = std::iter::from_fn(|| seen.try_recv().ok()).collect();
        assert_eq!(
            texts,
            (0..RELAY_CAPACITY)
                .map(|index| chunk(&index.to_string()))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_turn_that_answers_in_time_still_succeeds() {
        let (_relay, relayed) = relay();
        let (events, _seen) = async_channel::unbounded();
        let run = async {
            tokio::time::sleep(Duration::from_secs(59)).await;
            1
        };
        assert_eq!(
            forward(run, &relayed, &events, QUIET, || false).await,
            Ok(1)
        );
    }
}
