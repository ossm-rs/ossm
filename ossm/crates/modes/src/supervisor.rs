use core::cell::Cell;
use core::fmt::Debug;
use core::future::Future;
use core::pin::pin;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use ossm::{MotionPhase, MotionSender, fault};

use crate::ModeCtx;

/// The set of modes a firmware can switch between.
///
/// Typically a fieldless enum that dispatches each variant to a mode fn:
///
/// ```ignore
/// #[derive(Clone, Copy, PartialEq, Debug)]
/// enum AppMode { Idle, PatternEngine, FunScript, Fault }
///
/// impl ModeSet for AppMode {
///     const FAULT: Self = AppMode::Fault;
///
///     async fn run(self, ctx: &ModeCtx) {
///         match self {
///             AppMode::Idle => ctx.run(&idle).await,
///             AppMode::PatternEngine => ctx.run(&PatternEngine).await,
///             AppMode::FunScript => ctx.run(&FunScript).await,
///             AppMode::Fault => ctx.run(&fault).await,
///         }
///     }
/// }
/// ```
pub trait ModeSet: Copy + PartialEq + Debug + Send + 'static {
    /// Entered when a fault is raised and never left until restart.
    ///
    /// Should dispatch to a mode that takes no extractors. Requests to
    /// switch into it are ignored; only [`fault::raise`] enters it.
    const FAULT: Self;

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
    ///
    /// A raised fault interrupts any mode and switches to
    /// [`ModeSet::FAULT`], which is never left. Starts there directly if a
    /// fault was raised before the supervisor ran.
    pub async fn run(self, ctx: ModeCtx) -> ! {
        let sup = self.sup;
        let mut mode = match fault::raised() {
            Some(_) => M::FAULT,
            None => sup.current(),
        };

        loop {
            sup.current.lock(|cell| cell.set(mode));

            if mode == M::FAULT {
                log::error!("Entering mode {:?}; restart required", mode);
                mode.run(&ctx).await;
                loop {
                    core::future::pending::<()>().await;
                }
            }

            log::info!("Entering mode {:?}", mode);
            let mut next = {
                let mut run = pin!(mode.run(&ctx));
                loop {
                    match select(run.as_mut(), sup.next_event()).await {
                        Either::First(()) => break None,
                        Either::Second(next) if next == mode => continue,
                        Either::Second(next) => break Some(next),
                    }
                }
            };

            log::info!("Leaving mode {:?}", mode);
            make_safe(&ctx.motion).await;

            while next.is_none_or(|next| next == mode) {
                next = Some(sup.next_event().await);
            }

            mode = next.unwrap();
        }
    }
}

impl<M: ModeSet> Supervisor<M> {
    /// Wait for the next mode to switch to: [`ModeSet::FAULT`] once a
    /// fault is raised, otherwise the latest request.
    async fn next_event(&self) -> M {
        loop {
            match select(self.request.wait(), fault::wait()).await {
                Either::First(mode) if mode == M::FAULT => {
                    log::warn!("Ignoring request for {:?}; only a raised fault enters it", mode);
                }
                Either::First(mode) => return mode,
                Either::Second(_) => return M::FAULT,
            }
        }
    }
}

/// Handle for requesting mode switches.
///
/// Requests are latest-wins, requesting the current mode is a
/// no-op, and nothing leaves [`ModeSet::FAULT`].
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
