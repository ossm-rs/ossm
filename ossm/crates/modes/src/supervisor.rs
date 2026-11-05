use core::cell::Cell;
use core::convert::Infallible;
use core::fmt::{self, Debug};
use core::future::Future;
use core::marker::PhantomData;
use core::pin::pin;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use ossm::{MotionPhase, MotionSender};
use pattern_engine::PatternRunner;

use crate::sealed;
use crate::{Mode, ModeCtx};

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

    /// Run the mode registered under `key`. Returns `false` straight away
    /// if there is none.
    fn run<'a>(&'a self, key: K, ctx: &'a ModeCtx) -> impl Future<Output = bool> + 'a;
}

impl sealed::Sealed for () {}

impl<K> ModeList<K> for () {
    fn contains(&self, _key: K) -> bool {
        false
    }

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
/// Modes::new()
///     .mode(AppMode::Patterns, patterns)
///     .start_in(AppMode::Patterns)
///     .run(motion, runner)
///     .await
/// ```
pub struct Modes<K, L> {
    list: L,
    start: ModeKey<K>,
}

impl Modes<Infallible, ()> {
    /// No modes besides the built-in ones, starting in
    /// [`Idle`](ModeKey::Idle).
    ///
    /// The first [`mode`](Self::mode) picks the key type, so a firmware
    /// that only needs the built-in modes never names one.
    pub const fn new() -> Self {
        Self {
            list: (),
            start: ModeKey::Idle,
        }
    }

    /// Register the first mode under `key`, which picks the key type for
    /// the rest.
    pub fn mode<K, F, Args>(self, key: K, mode: F) -> Modes<K, (Entry<K, F, Args>, ())>
    where
        K: Copy + PartialEq + Debug + 'static,
        F: Mode<Args>,
    {
        // Before any mode is registered, only Idle can be the start mode.
        Modes {
            list: (Entry::new(key, mode), ()),
            start: ModeKey::Idle,
        }
    }
}

impl Default for Modes<Infallible, ()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, E, Rest> Modes<K, (E, Rest)>
where
    K: Copy + PartialEq + Debug + 'static,
    (E, Rest): ModeList<K>,
{
    /// Register `mode` under `key`. Registering a key again replaces the
    /// earlier mode.
    pub fn mode<F, Args>(self, key: K, mode: F) -> Modes<K, (Entry<K, F, Args>, (E, Rest))>
    where
        F: Mode<Args>,
    {
        Modes {
            list: (Entry::new(key, mode), self.list),
            start: self.start,
        }
    }
}

impl<K, L> Modes<K, L>
where
    K: Copy + PartialEq + Debug + 'static,
    L: ModeList<K>,
{
    /// The mode to enter at boot. Defaults to [`Idle`](ModeKey::Idle).
    pub fn start_in(self, key: impl Into<ModeKey<K>>) -> Self {
        Self {
            start: key.into(),
            ..self
        }
    }

    /// Run the modes for the rest of the program, lending each the
    /// capabilities it declares.
    pub async fn run(self, motion: MotionSender, patterns: PatternRunner) -> ! {
        let ctx = ModeCtx::new(motion, patterns);
        let start = match self.start {
            ModeKey::Custom(key) if !self.list.contains(key) => {
                log::error!("Start mode {:?} is not registered; starting in Idle", key);
                ModeKey::Idle
            }
            start => start,
        };
        let supervisor = Supervisor::new(start);
        supervisor.run(&self.list, &ctx).await
    }
}

/// Switches between modes, one at a time.
struct Supervisor<K> {
    request: Signal<CriticalSectionRawMutex, ModeKey<K>>,
    current: Mutex<CriticalSectionRawMutex, Cell<ModeKey<K>>>,
}

impl<K> Supervisor<K>
where
    K: Copy + PartialEq + Debug + 'static,
{
    const fn new(initial: ModeKey<K>) -> Self {
        Self {
            request: Signal::new(),
            current: Mutex::new(Cell::new(initial)),
        }
    }

    fn current(&self) -> ModeKey<K> {
        self.current.lock(|cell| cell.get())
    }

    /// Run the active mode until a different mode is requested, bring
    /// the motor to a safe state, then start the requested mode.
    ///
    /// Requests for the mode that is already running are ignored and do
    /// not interrupt it. A mode that returns on its own stays current;
    /// the supervisor makes the motor safe and waits for the next request.
    async fn run(&self, list: &impl ModeList<K>, ctx: &ModeCtx) -> ! {
        let mut mode = self.current();

        loop {
            log::info!("Entering mode {:?}", mode);
            let mut next = {
                let mut run = pin!(run_mode(list, mode, ctx));
                loop {
                    match select(run.as_mut(), self.request.wait()).await {
                        Either::First(()) => break None,
                        Either::Second(next) if next == mode => continue,
                        Either::Second(next) => break Some(next),
                    }
                }
            };

            log::info!("Leaving mode {:?}", mode);
            make_safe(&ctx.motion).await;

            while next.is_none_or(|next| next == mode) {
                next = Some(self.request.wait().await);
            }

            mode = next.unwrap();
            self.current.lock(|cell| cell.set(mode));
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
