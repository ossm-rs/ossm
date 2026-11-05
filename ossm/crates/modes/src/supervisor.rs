use core::cell::Cell;
use core::fmt::Debug;
use core::future::Future;
use core::pin::pin;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use ossm::{MotionPhase, MotionSender};

use crate::ModeCtx;

/// The set of modes a firmware can switch between.
///
/// Typically a fieldless enum that dispatches each variant to a mode fn:
///
/// ```ignore
/// #[derive(Clone, Copy, PartialEq, Debug)]
/// enum AppMode { Idle, PatternEngine, FunScript }
///
/// impl ModeSet for AppMode {
///     async fn run(self, ctx: &ModeCtx) {
///         match self {
///             AppMode::Idle => ctx.run(&idle).await,
///             AppMode::PatternEngine => ctx.run(&PatternEngine).await,
///             AppMode::FunScript => ctx.run(&FunScript).await,
///         }
///     }
/// }
/// ```
pub trait ModeSet: Copy + PartialEq + Debug + Send + 'static {
    fn run(self, ctx: &ModeCtx) -> impl Future<Output = ()>;
}

/// Root container for mode switching.
///
/// Instantiated once in static storage and consumed by
/// [`split`](Self::split), in the same shape as `Ossm` and
/// `PatternEngine`.
pub struct Supervisor<M: ModeSet> {
    request: Signal<CriticalSectionRawMutex, M>,
    current: Mutex<CriticalSectionRawMutex, Cell<M>>,
}

impl<M: ModeSet> Supervisor<M> {
    pub const fn new(initial: M) -> Self {
        Self {
            request: Signal::new(),
            current: Mutex::new(Cell::new(initial)),
        }
    }

    /// Split into the driver and the switch-request handle.
    pub fn split(&'static mut self) -> (ModeRunner<M>, ModeSwitcher<M>) {
        (ModeRunner { sup: self }, ModeSwitcher { sup: self })
    }

    fn current(&self) -> M {
        self.current.lock(|cell| cell.get())
    }
}

/// Driver capability for the [`Supervisor`].
///
/// Consumed by [`run`](Self::run), which takes ownership of the
/// [`ModeCtx`] for the duration.
pub struct ModeRunner<M: ModeSet> {
    sup: &'static Supervisor<M>,
}

impl<M: ModeSet> ModeRunner<M> {
    /// Run the active mode until a different mode is requested, bring
    /// the motor to a safe state, then start the requested mode.
    ///
    /// Requests for the mode that is already running are ignored and do
    /// not interrupt it. A mode that returns on its own stays current;
    /// the supervisor makes the motor safe and waits for the next request.
    pub async fn run(self, ctx: ModeCtx) -> ! {
        let sup = self.sup;
        let mut mode = sup.current();

        loop {
            log::info!("Entering mode {:?}", mode);
            let mut next = {
                let mut run = pin!(mode.run(&ctx));
                loop {
                    match select(run.as_mut(), sup.request.wait()).await {
                        Either::First(()) => break None,
                        Either::Second(next) if next == mode => continue,
                        Either::Second(next) => break Some(next),
                    }
                }
            };

            log::info!("Leaving mode {:?}", mode);
            make_safe(&ctx.motion).await;

            while next.is_none_or(|next| next == mode) {
                next = Some(sup.request.wait().await);
            }

            mode = next.unwrap();
            sup.current.lock(|cell| cell.set(mode));
        }
    }
}

/// Handle for requesting mode switches.
///
/// Requests are latest-wins, requesting the current mode is a
/// no-op.
pub struct ModeSwitcher<M: ModeSet> {
    sup: &'static Supervisor<M>,
}

impl<M: ModeSet> ModeSwitcher<M> {
    pub fn request(&self, mode: M) {
        self.sup.request.signal(mode);
    }

    pub fn current(&self) -> M {
        self.sup.current()
    }
}

/// Disable the motor and wait until the controller reports `Disabled`.
///
/// Loops on the observed phase rather than trusting a single response,
/// because the dropped mode may have had its own state command in flight
/// whose response could be mistaken for ours.
async fn make_safe(motion: &MotionSender) {
    while motion.state().phase != MotionPhase::Disabled {
        motion.disable().await;
    }
}
