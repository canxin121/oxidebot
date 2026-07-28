use crate::{middleware::Endpoint, FromRequest, IntoResponse, Request, Response};
use futures_util::future::BoxFuture;
use std::{future::Future, marker::PhantomData, sync::Arc};

/// Conversion used internally by [`crate::Router`] to accept ordinary async
/// functions with extractor arguments.
#[doc(hidden)]
pub trait IntoHandler<T, S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn into_endpoint(self) -> Arc<dyn Endpoint<S>>;
}

struct FunctionEndpoint<F, T> {
    function: Arc<F>,
    _arguments: PhantomData<fn() -> T>,
}

macro_rules! impl_into_handler {
    () => {
        impl<F, Fut, Output, S> IntoHandler<(), S> for F
        where
            S: Send + Sync + 'static,
            F: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoResponse + 'static,
        {
            fn into_endpoint(self) -> Arc<dyn Endpoint<S>> {
                Arc::new(FunctionEndpoint::<F, ()> {
                    function: Arc::new(self),
                    _arguments: PhantomData,
                })
            }
        }

        impl<F, Fut, Output, S> Endpoint<S> for FunctionEndpoint<F, ()>
        where
            S: Send + Sync + 'static,
            F: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoResponse + 'static,
        {
            fn call(&self, _request: Request<S>) -> BoxFuture<'static, Response> {
                let function = Arc::clone(&self.function);
                Box::pin(async move { function().await.into_response() })
            }
        }
    };
    ($($argument:ident),+ $(,)?) => {
        impl<F, Fut, Output, S, $($argument,)+> IntoHandler<($($argument,)+), S> for F
        where
            S: Send + Sync + 'static,
            F: Fn($($argument),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoResponse + 'static,
            $($argument: FromRequest<S> + Send + 'static,)+
        {
            fn into_endpoint(self) -> Arc<dyn Endpoint<S>> {
                Arc::new(FunctionEndpoint::<F, ($($argument,)+)> {
                    function: Arc::new(self),
                    _arguments: PhantomData,
                })
            }
        }

        impl<F, Fut, Output, S, $($argument,)+> Endpoint<S>
            for FunctionEndpoint<F, ($($argument,)+)>
        where
            S: Send + Sync + 'static,
            F: Fn($($argument),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoResponse + 'static,
            $($argument: FromRequest<S> + Send + 'static,)+
        {
            #[allow(non_snake_case)]
            fn call(&self, mut request: Request<S>) -> BoxFuture<'static, Response> {
                let function = Arc::clone(&self.function);
                Box::pin(async move {
                    $(
                        let $argument = match $argument::from_request(&mut request).await {
                            Ok(value) => value,
                            Err(rejection) => return rejection,
                        };
                    )+
                    function($($argument),+).await.into_response()
                })
            }
        }
    };
}

impl_into_handler!();
impl_into_handler!(A1);
impl_into_handler!(A1, A2);
impl_into_handler!(A1, A2, A3);
impl_into_handler!(A1, A2, A3, A4);
impl_into_handler!(A1, A2, A3, A4, A5);
impl_into_handler!(A1, A2, A3, A4, A5, A6);
impl_into_handler!(A1, A2, A3, A4, A5, A6, A7);
impl_into_handler!(A1, A2, A3, A4, A5, A6, A7, A8);
impl_into_handler!(A1, A2, A3, A4, A5, A6, A7, A8, A9);
impl_into_handler!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10);
impl_into_handler!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11);
impl_into_handler!(A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11, A12);
