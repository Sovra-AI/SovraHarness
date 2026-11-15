//! Runtime-supplied typed calls shared by account integrations.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

type CallFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
pub(crate) type CallFn<R> = Arc<dyn Fn(R) -> CallFuture + Send + Sync>;

#[derive(Clone)]
pub struct ConnectedTools<S, R> {
    pub(crate) services: BTreeSet<S>,
    pub(crate) call: CallFn<R>,
}

impl<S: std::fmt::Debug, R> std::fmt::Debug for ConnectedTools<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectedTools")
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

impl<S, R> ConnectedTools<S, R> {
    pub fn from_fn<F, Fut>(services: BTreeSet<S>, call: F) -> Self
    where
        F: Fn(R) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
        Self {
            services,
            call: Arc::new(move |request| Box::pin(call(request))),
        }
    }
}
