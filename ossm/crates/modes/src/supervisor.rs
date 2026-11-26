use alloc::boxed::Box;
use core::cell::Cell;
use core::fmt::{self, Debug};
use core::future::Future;
use core::marker::PhantomData;
use core::pin::pin;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use ossm::{MotionPhase, MotionSender};
use pattern_engine::{PatternRunner, PatternSender};

use crate::remote::{Inputs, Remotes};
use crate::sealed;
use crate::{Mode, ModeCtx};

/// Modes a firmware can register besides the built-in ones.
const MAX_MODES: usize = 8;

/// Names a mode the supervisor can run.
///
/// [`Idle`](Self::Idle) is built in and always available. A firmware
/// registers its own modes under keys of its choosing with
/// [`Modes::mode`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModeKey<K> {
    /// Nothing runs and the motor stays disabled.
    Idle,
    /// A mode the firmware registered.
    Custom(K),
}

impl<K> From<K> for ModeKey<K> {
    fn from(key: K) -> Self {
        ModeKey::Custom(key)
    }
}

impl<K: Debug> Debug for ModeKey<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModeKey::Idle => f.write_str("Idle"),
            ModeKey::Custom(key) => key.fmt(f),
        }
    }
}

/// The modes a firmware registered, built up by [`Modes::mode`].
pub trait ModeList<K>: sealed::Sealed {
    fn contains(&self, key: K) -> bool;

    /// Call `f` with every registered key.
    fn keys(&self, f: &mut impl FnMut(K));

    /// Run the mode registered under `key`. Returns `false` straight away
    /// if there is none.
    fn run<'a>(&'a self, key: K, ctx: &'a ModeCtx) -> impl Future<Output = bool> + 'a;
}

impl sealed::Sealed for () {}

impl<K> ModeList<K> for () {
    fn contains(&self, _key: K) -> bool {
        false
    }

    fn keys(&self, _f: &mut impl FnMut(K)) {}

    fn run<'a>(&'a self, _key: K, _ctx: &'a ModeCtx) -> impl Future<Output = bool> + 'a {
        async { false }
    }
}

/// One registered mode.
pub struct Entry<K, F, Args> {
    key: K,
    mode: F,
    _args: PhantomData<fn() -> Args>,
}

impl<K, F, Args> Entry<K, F, Args> {
    fn new(key: K, mode: F) -> Self {
        Self {
            key,
            mode,
            _args: PhantomData,
        }
    }
}

impl<K, F, Args, Rest> sealed::Sealed for (Entry<K, F, Args>, Rest) {}

impl<K, F, Args, Rest> ModeList<K> for (Entry<K, F, Args>, Rest)
where
    K: Copy + PartialEq + 'static,
    F: Mode<Args>,
    Rest: ModeList<K>,
{
    fn contains(&self, key: K) -> bool {
        self.0.key == key || self.1.contains(key)
    }

    fn keys(&self, f: &mut impl FnMut(K)) {
        f(self.0.key);
        self.1.keys(f);
    }

    fn run<'a>(&'a self, key: K, ctx: &'a ModeCtx) -> impl Future<Output = bool> + 'a {
        async move {
            if self.0.key == key {
                ctx.run(&self.0.mode).await;
                true
            } else {
                self.1.run(key, ctx).await
            }
        }
    }
}

/// The modes a firmware can switch between, and the one it starts in.
///
/// ```ignore
/// let (runner, remotes) = Modes::new()
///     .mode(AppMode::Patterns, patterns)
///     .start_in(AppMode::Patterns)
///     .split(pattern_sender);
/// ```
pub struct Modes<K, L> {
    list: L,
    start: ModeKey<K>,
}

impl<K> Modes<K, ()> {
    /// No modes besides the built-in ones, starting in
    /// [`Idle`](ModeKey::Idle).
    pub const fn new() -> Self {
        Self {
            list: (),
            start: ModeKey::Idle,
        }
    }
}

impl<K> Default for Modes<K, ()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, L> Modes<K, L>
where
    K: Copy + PartialEq + Debug + 'static,
    L: ModeList<K>,
{
    /// Register `mode` under `key`. Registering a key again replaces the
    /// earlier mode.
    pub fn mode<F, Args>(self, key: K, mode: F) -> Modes<K, (Entry<K, F, Args>, L)>
    where
        F: Mode<Args>,
    {
        Modes {
            list: (Entry::new(key, mode), self.list),
            start: self.start,
        }
    }

    /// The mode to enter at boot. Defaults to [`Idle`](ModeKey::Idle).
    pub fn start_in(self, key: impl Into<ModeKey<K>>) -> Self {
        Self {
            start: key.into(),
            ..self
        }
    }

    /// Split into the runner that drives the modes and the source of
    /// remote adapters' handles.
    ///
    /// Call once: the supervisor they share lives for the rest of the
    /// program.
    pub fn split(self, patterns: &'static PatternSender) -> (ModeRunner<K, L>, Remotes<K>) {
        let mut registered = heapless::Vec::new();
        self.list.keys(&mut |key| {
            if registered.push(key).is_err() {
                log::error!("More than {} modes registered; {:?} cannot be switched to", MAX_MODES, key);
            }
        });
        let start = match self.start {
            ModeKey::Custom(key) if !self.list.contains(key) => {
                log::error!("Start mode {:?} is not registered; starting in Idle", key);
                ModeKey::Idle
            }
            start => start,
        };
        let sup: &'static Supervisor<K> = Box::leak(Box::new(Supervisor {
            request: Signal::new(),
            request_seq: AtomicU32::new(0),
            current: Mutex::new(Cell::new(start)),
            leaving: AtomicBool::new(false),
            inputs: Inputs::new(),
            patterns,
            registered,
        }));
        (ModeRunner { sup, list: self.list }, Remotes { sup })
    }
}

/// State the supervisor shares with remote adapters.
pub(crate) struct Supervisor<K> {
    pub(crate) request: Signal<CriticalSectionRawMutex, ModeKey<K>>,
    /// Bumped on every switch request so a waiting caller can tell it was
    /// superseded.
    pub(crate) request_seq: AtomicU32,
    current: Mutex<CriticalSectionRawMutex, Cell<ModeKey<K>>>,
    /// Set from deciding to leave the current mode until the next one is
    /// entered, so `current` is not mistaken for where the machine stays.
    pub(crate) leaving: AtomicBool,
    pub(crate) inputs: Inputs,
    /// Shared with remotes that still drive the engine directly, until
    /// every remote goes through a [`RemoteHandle`](crate::RemoteHandle).
    pub(crate) patterns: &'static PatternSender,
    registered: heapless::Vec<K, MAX_MODES>,
}

impl<K> Supervisor<K>
where
    K: Copy + PartialEq + Debug + 'static,
{
    pub(crate) fn current(&self) -> ModeKey<K> {
        self.current.lock(|cell| cell.get())
    }

    pub(crate) fn is_registered(&self, mode: ModeKey<K>) -> bool {
        match mode {
            ModeKey::Idle => true,
            ModeKey::Custom(key) => self.registered.contains(&key),
        }
    }
}

/// Drives the modes, consumed by [`run`](Self::run).
pub struct ModeRunner<K: 'static, L> {
    sup: &'static Supervisor<K>,
    list: L,
}

impl<K, L> ModeRunner<K, L>
where
    K: Copy + PartialEq + Debug + 'static,
    L: ModeList<K>,
{
    /// Run the active mode until a different mode is requested, bring
    /// the motor to a safe state, then start the requested mode, for the
    /// rest of the program.
    ///
    /// Requests for the mode that is already running are ignored and do
    /// not interrupt it. A mode that returns on its own stays current;
    /// the supervisor makes the motor safe and waits for the next request.
    pub async fn run(self, motion: MotionSender, patterns: PatternRunner) -> ! {
        let sup = self.sup;
        let ctx = ModeCtx::new(motion, patterns, sup.patterns, &sup.inputs);
        let mut mode = sup.current();

        loop {
            sup.current.lock(|cell| cell.set(mode));
            sup.leaving.store(false, Ordering::Relaxed);

            log::info!("Entering mode {:?}", mode);
            let mut next = {
                let mut run = pin!(run_mode(&self.list, mode, &ctx));
                loop {
                    match select(run.as_mut(), sup.request.wait()).await {
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
                next = Some(sup.request.wait().await);
            }

            sup.leaving.store(true, Ordering::Relaxed);
            mode = next.unwrap();
        }
    }
}

async fn run_mode<K>(list: &impl ModeList<K>, mode: ModeKey<K>, ctx: &ModeCtx) {
    match mode {
        ModeKey::Idle => ctx.run(&idle).await,
        ModeKey::Custom(key) => {
            // Only registered keys get this far.
            list.run(key, ctx).await;
        }
    }
}

async fn idle() {
    core::future::pending().await
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
