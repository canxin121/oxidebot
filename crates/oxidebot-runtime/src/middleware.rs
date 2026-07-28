use crate::{IntoResponse, Request, Response};
use futures_util::future::BoxFuture;
use std::{future::Future, sync::Arc};

#[doc(hidden)]
pub trait Endpoint<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn call(&self, request: Request<S>) -> BoxFuture<'static, Response>;
}

/// Middleware around one matched route.
pub trait Middleware<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn call(&self, request: Request<S>, next: Next<S>) -> BoxFuture<'static, Response>;
}

/// The remaining middleware chain and endpoint.
#[derive(Clone)]
pub struct Next<S>
where
    S: Send + Sync + 'static,
{
    endpoint: Arc<dyn Endpoint<S>>,
    middleware: Arc<[Arc<dyn Middleware<S>>]>,
    index: usize,
}

impl<S> Next<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn new(
        endpoint: Arc<dyn Endpoint<S>>,
        middleware: Arc<[Arc<dyn Middleware<S>>]>,
    ) -> Self {
        Self {
            endpoint,
            middleware,
            index: 0,
        }
    }

    /// Runs the rest of the chain exactly once.
    pub async fn run(mut self, request: Request<S>) -> Response {
        if let Some(layer) = self.middleware.get(self.index).cloned() {
            self.index = self.index.saturating_add(1);
            layer.call(request, self).await
        } else {
            self.endpoint.call(request).await
        }
    }
}

/// Function-backed middleware created by [`from_fn`].
pub struct FromFn<F> {
    function: Arc<F>,
}

/// Creates middleware from an ordinary async function accepting
/// `(Request<S>, Next<S>)`.
#[must_use]
pub fn from_fn<F>(function: F) -> FromFn<F> {
    FromFn {
        function: Arc::new(function),
    }
}

impl<S, F, Fut, Output> Middleware<S> for FromFn<F>
where
    S: Send + Sync + 'static,
    F: Fn(Request<S>, Next<S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Output> + Send + 'static,
    Output: IntoResponse + 'static,
{
    fn call(&self, request: Request<S>, next: Next<S>) -> BoxFuture<'static, Response> {
        let function = Arc::clone(&self.function);
        Box::pin(async move { function(request, next).await.into_response() })
    }
}
