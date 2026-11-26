use core::future::Future;

use crate::sealed;
use crate::{Extract, ModeCtx, Subscriptions};

/// An async fn the arguments of which are all [`Extract`] types.
///
/// Sealed: implemented automatically for such functions, and nothing
/// else can be a mode.
pub trait Mode<Args>: sealed::Sealed<fn() -> Args> {
    /// What this mode subscribes to while it runs: its extractors'
    /// [`Extract::SUBSCRIBES`], combined.
    const SUBSCRIBES: Subscriptions;

    fn run<'a>(&'a self, ctx: &'a ModeCtx) -> impl Future<Output = ()> + 'a;
}

macro_rules! impl_mode {
    ($($arg:ident),*) => {
        // The first `AsyncFn` bound pins each `$arg` to the `'static`
        // form of the parameter type, which inference can't recover from
        // the projection alone. The second is the call we actually make,
        // valid for any borrow of the context.
        impl<F, $($arg: Extract + 'static),*> Mode<($($arg,)*)> for F
        where
            F: AsyncFn($($arg),*) + for<'a> AsyncFn($($arg::Out<'a>),*),
        {
            const SUBSCRIBES: Subscriptions = Subscriptions::NONE$(.union($arg::SUBSCRIBES))*;

            #[allow(unused_variables)]
            fn run<'a>(&'a self, ctx: &'a ModeCtx) -> impl Future<Output = ()> + 'a {
                self($($arg::extract(ctx)),*)
            }
        }

        impl<F, $($arg: Extract + 'static),*> sealed::Sealed<fn() -> ($($arg,)*)> for F
        where
            F: AsyncFn($($arg),*) + for<'a> AsyncFn($($arg::Out<'a>),*),
        {
        }
    };
}

impl_mode!();
impl_mode!(A);
impl_mode!(A, B);
impl_mode!(A, B, C);
impl_mode!(A, B, C, D);
impl_mode!(A, B, C, D, E);
