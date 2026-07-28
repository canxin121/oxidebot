use crate::{hooks::Endpoint, Context, Extract, HandlerError, HandlerResult, IntoOutcome, Outcome};
use futures_util::future::BoxFuture;
use std::{future::Future, marker::PhantomData, sync::Arc};

/// Marker used internally for handlers whose future returns an effect directly.
#[doc(hidden)]
pub enum InfallibleOutput {}

/// Marker used internally for handlers whose future returns `Result<T, E>`.
#[doc(hidden)]
pub enum FallibleOutput {}

/// Conversion used internally by [`crate::Module`] to accept ordinary async
/// functions with typed Bot arguments.
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
        impl<F, Fut, Output, S> IntoHandler<(InfallibleOutput,), S> for F
        where
            S: Send + Sync + 'static,
            F: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoOutcome + 'static,
        {
            fn into_endpoint(self) -> Arc<dyn Endpoint<S>> {
                Arc::new(FunctionEndpoint::<F, (InfallibleOutput,)> {
                    function: Arc::new(self),
                    _arguments: PhantomData,
                })
            }
        }

        impl<F, Fut, Output, S> Endpoint<S> for FunctionEndpoint<F, (InfallibleOutput,)>
        where
            S: Send + Sync + 'static,
            F: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoOutcome + 'static,
        {
            fn call(&self, _context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
                let function = Arc::clone(&self.function);
                Box::pin(async move { Ok(function().await.into_outcome()) })
            }
        }

        impl<F, Fut, Output, Error, S> IntoHandler<(FallibleOutput,), S> for F
        where
            S: Send + Sync + 'static,
            F: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Output, Error>> + Send + 'static,
            Output: IntoOutcome + 'static,
            Error: Into<HandlerError> + 'static,
        {
            fn into_endpoint(self) -> Arc<dyn Endpoint<S>> {
                Arc::new(FunctionEndpoint::<F, (FallibleOutput,)> {
                    function: Arc::new(self),
                    _arguments: PhantomData,
                })
            }
        }

        impl<F, Fut, Output, Error, S> Endpoint<S> for FunctionEndpoint<F, (FallibleOutput,)>
        where
            S: Send + Sync + 'static,
            F: Fn() -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Output, Error>> + Send + 'static,
            Output: IntoOutcome + 'static,
            Error: Into<HandlerError> + 'static,
        {
            fn call(&self, _context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
                let function = Arc::clone(&self.function);
                Box::pin(async move {
                    function()
                        .await
                        .map(IntoOutcome::into_outcome)
                        .map_err(Into::into)
                })
            }
        }
    };
    ($($argument:ident),+ $(,)?) => {
        impl<F, Fut, Output, S, $($argument,)+>
            IntoHandler<(InfallibleOutput, $($argument,)+), S> for F
        where
            S: Send + Sync + 'static,
            F: Fn($($argument),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoOutcome + 'static,
            $($argument: Extract<S> + Send + 'static,)+
        {
            fn into_endpoint(self) -> Arc<dyn Endpoint<S>> {
                Arc::new(FunctionEndpoint::<F, (InfallibleOutput, $($argument,)+)> {
                    function: Arc::new(self),
                    _arguments: PhantomData,
                })
            }
        }

        impl<F, Fut, Output, S, $($argument,)+> Endpoint<S>
            for FunctionEndpoint<F, (InfallibleOutput, $($argument,)+)>
        where
            S: Send + Sync + 'static,
            F: Fn($($argument),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Output> + Send + 'static,
            Output: IntoOutcome + 'static,
            $($argument: Extract<S> + Send + 'static,)+
        {
            #[allow(non_snake_case)]
            fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
                let function = Arc::clone(&self.function);
                Box::pin(async move {
                    $(
                        let $argument = match $argument::extract(&context) {
                            Ok(value) => value,
                            Err(rejection) => return Ok(rejection.into_outcome()),
                        };
                    )+
                    Ok(function($($argument),+).await.into_outcome())
                })
            }
        }

        impl<F, Fut, Output, Error, S, $($argument,)+>
            IntoHandler<(FallibleOutput, $($argument,)+), S> for F
        where
            S: Send + Sync + 'static,
            F: Fn($($argument),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Output, Error>> + Send + 'static,
            Output: IntoOutcome + 'static,
            Error: Into<HandlerError> + 'static,
            $($argument: Extract<S> + Send + 'static,)+
        {
            fn into_endpoint(self) -> Arc<dyn Endpoint<S>> {
                Arc::new(FunctionEndpoint::<F, (FallibleOutput, $($argument,)+)> {
                    function: Arc::new(self),
                    _arguments: PhantomData,
                })
            }
        }

        impl<F, Fut, Output, Error, S, $($argument,)+> Endpoint<S>
            for FunctionEndpoint<F, (FallibleOutput, $($argument,)+)>
        where
            S: Send + Sync + 'static,
            F: Fn($($argument),+) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Output, Error>> + Send + 'static,
            Output: IntoOutcome + 'static,
            Error: Into<HandlerError> + 'static,
            $($argument: Extract<S> + Send + 'static,)+
        {
            #[allow(non_snake_case)]
            fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
                let function = Arc::clone(&self.function);
                Box::pin(async move {
                    $(
                        let $argument = match $argument::extract(&context) {
                            Ok(value) => value,
                            Err(rejection) => return Ok(rejection.into_outcome()),
                        };
                    )+
                    function($($argument),+)
                        .await
                        .map(IntoOutcome::into_outcome)
                        .map_err(Into::into)
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
