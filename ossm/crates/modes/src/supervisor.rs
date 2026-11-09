use core::cell::Cell;
use core::fmt::Debug;
use core::future::Future;
use core::pin::pin;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use ossm::{MotionPhase, MotionSender, fault};
use pattern_engine::{PatternRunner, PatternSender};

use crate::remote::{Mailbox, RemoteHandle};
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
    pub(crate) request: Signal<CriticalSectionRawMutex, M>,
    /// Bumped on every switch request so a waiting caller can tell it was
    /// superseded.
    pub(crate) request_seq: AtomicU32,
    current: Mutex<CriticalSectionRawMutex, Cell<M>>,
    /// Set from deciding to leave the current mode until the next one is
    /// entered, so `current` is not mistaken for where the machine stays.
    pub(crate) leaving: AtomicBool,
    pub(crate) mailbox: Mailbox,
    /// Shared with remotes that still drive the engine directly, until
    /// every remote goes through [`RemoteHandle`].
    pub(crate) patterns: &'static PatternSender,
}

impl<M: ModeSet> Supervisor<M> {
    pub const fn new(initial: M, patterns: &'static PatternSender) -> Self {
        Self {
            request: Signal::new(),
            request_seq: AtomicU32::new(0),
            current: Mutex::new(Cell::new(initial)),
            leaving: AtomicBool::new(false),
            mailbox: Mailbox::new(),
            patterns,
        }
    }

    /// Split into the driver and the handle remote adapters share.
    pub fn split(&'static mut self) -> (ModeRunner<M>, RemoteHandle<M>) {
        (ModeRunner { sup: self }, RemoteHandle { sup: self })
    }

    pub(crate) fn current(&self) -> M {
        self.current.lock(|cell| cell.get())
    }
}

/// Driver capability for the [`Supervisor`].
///
/// Consumed by [`run`](Self::run), which takes ownership of the
/// capabilities for the duration.
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
    pub async fn run(self, motion: MotionSender, patterns: PatternRunner) -> ! {
        let sup = self.sup;
        let ctx = ModeCtx::new(motion, patterns, sup.patterns, &sup.mailbox);
        let mut mode = match fault::raised() {
            Some(_) => M::FAULT,
            None => sup.current(),
        };

        loop {
            sup.current.lock(|cell| cell.set(mode));
            sup.leaving.store(false, Ordering::Relaxed);

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
            sup.leaving.store(next.is_some(), Ordering::Relaxed);
            make_safe(&ctx.motion).await;

            while next.is_none_or(|next| next == mode) {
                next = Some(sup.next_event().await);
            }

            sup.leaving.store(true, Ordering::Relaxed);
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
