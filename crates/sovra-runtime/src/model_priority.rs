//! Defer new memory model calls while interactive turns own the model.

use std::sync::OnceLock;
use tokio::sync::watch;

fn activity() -> &'static watch::Sender<usize> {
    static ACTIVE: OnceLock<watch::Sender<usize>> = OnceLock::new();
    ACTIVE.get_or_init(|| watch::channel(0).0)
}

pub(crate) struct Interactive(watch::Sender<usize>);

impl Interactive {
    fn acquire(sender: watch::Sender<usize>) -> Self {
        sender.send_modify(|count| *count += 1);
        Self(sender)
    }
}

impl Drop for Interactive {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count -= 1);
    }
}

pub(crate) fn interactive() -> Interactive {
    Interactive::acquire(activity().clone())
}

pub(crate) fn busy() -> bool {
    *activity().borrow() > 0
}

pub(crate) fn subscribe() -> watch::Receiver<usize> {
    activity().subscribe()
}

pub(crate) async fn background_ready() {
    wait_idle(subscribe()).await;
}

async fn wait_idle(mut activity: watch::Receiver<usize>) {
    while *activity.borrow_and_update() > 0 {
        if activity.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn background_waits_for_all_interactive_turns_including_cancellation() {
        let (sender, receiver) = watch::channel(0);
        let first = Interactive::acquire(sender.clone());
        let second = Interactive::acquire(sender);
        let background = tokio::runtime::Handle::current().spawn(wait_idle(receiver));
        tokio::task::yield_now().await;
        assert!(!background.is_finished());
        drop(first);
        tokio::task::yield_now().await;
        assert!(!background.is_finished());
        let owner = tokio::runtime::Handle::current().spawn(async move {
            let _guard = second;
            std::future::pending::<()>().await;
        });
        owner.abort();
        let _ = owner.await;
        tokio::time::timeout(std::time::Duration::from_secs(1), background)
            .await
            .unwrap()
            .unwrap();
    }
}
