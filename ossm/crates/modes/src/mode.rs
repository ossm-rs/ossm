use core::future::Future;

use crate::remote::InputKinds;
use crate::{Extract, ModeCtx};

/// An async fn the arguments of which are all [`Extract`] types.
pub trait Mode<Args> {
    /// Request kinds remotes may send while this mode runs: the union of
    /// its extractors' [`Extract::ACCEPTS`], which must not overlap.
    const ACCEPTS: InputKinds;

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
            const ACCEPTS: InputKinds = {
                let accepts = InputKinds::NONE;
                $(
                    assert!(
                        !accepts.intersects($arg::ACCEPTS),
                        "a mode can take at most one `Commands` per input kind",
                    );
                    let accepts = accepts.union($arg::ACCEPTS);
                )*
                accepts
            };

            #[allow(unused_variables)]
            fn run<'a>(&'a self, ctx: &'a ModeCtx) -> impl Future<Output = ()> + 'a {
                self($($arg::extract(ctx)),*)
            }
        }
    };
}

impl_mode!();
impl_mode!(A);
impl_mode!(A, B);
impl_mode!(A, B, C);
impl_mode!(A, B, C, D);
impl_mode!(A, B, C, D, E);
