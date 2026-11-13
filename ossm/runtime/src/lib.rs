//! Run OSSM firmware: register the modes it can switch between, then hand
//! them a platform to run on.
//!
//! ```ignore
//! Ossm::new(Esp32s3::new(spawner, config))
//!     .mode(OssmMode::Patterns, patterns)
//!     .start_in(OssmMode::Patterns)
//!     .run()
//!     .await
//! ```
//!
//! Writing modes needs only the items at the top level. Writing a
//! platform uses [`platform`] and the motion vocabulary in [`motion`].
#![no_std]

use core::convert::Infallible;
use core::fmt::Debug;

use modes::{Entry, ModeList, Modes};
use ossm_core::MotionChannels;
use ossm_core::fault;
use pattern_engine::{PatternEngine, PatternSender};
use static_cell::StaticCell;

pub use modes::{Mode, ModeKey, Motion, MotionStatus, OssmMode, Patterns, patterns};

pub mod platform;

/// Motion control: what modes drive the motor with, and what platforms
/// set a board up with.
pub mod motion {
    // Driving the motor, as modes do through `Motion`.
    pub use ossm_core::{
        Cancelled, MotionCommand, MotionPhase, MotionSender, MotionState, StateResponse,
    };
    // Setting up a board for the motion controller, as platforms do.
    pub use ossm_core::{Board, MechanicalConfig, MotionController, MotionLimits, MotionObserver};
}

use platform::Platform;

static CHANNELS: StaticCell<MotionChannels> = StaticCell::new();
static ENGINE: StaticCell<PatternEngine> = StaticCell::new();
static PATTERN_SENDER: StaticCell<PatternSender> = StaticCell::new();

/// OSSM firmware on platform `P`, with the modes registered so far.
pub struct Ossm<P, K, L> {
    platform: P,
    modes: Modes<K, L>,
}

impl<P: Platform> Ossm<P, Infallible, ()> {
    /// OSSM on `platform` with only the built-in modes, starting in
    /// [`Idle`](ModeKey::Idle).
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            modes: Modes::new(),
        }
    }

    /// Register the first mode under `key`, which picks the key type for
    /// the rest.
    pub fn mode<K, F, Args>(self, key: K, mode: F) -> Ossm<P, K, (Entry<K, F, Args>, ())>
    where
        K: Copy + PartialEq + Debug + 'static,
        F: Mode<Args>,
    {
        Ossm {
            platform: self.platform,
            modes: self.modes.mode(key, mode),
        }
    }
}

impl<P, K, E, Rest> Ossm<P, K, (E, Rest)>
where
    P: Platform,
    K: Copy + PartialEq + Debug + 'static,
    (E, Rest): ModeList<K>,
{
    /// Register `mode` under `key`. Registering a key again replaces the
    /// earlier mode.
    pub fn mode<F, Args>(self, key: K, mode: F) -> Ossm<P, K, (Entry<K, F, Args>, (E, Rest))>
    where
        F: Mode<Args>,
    {
        Ossm {
            platform: self.platform,
            modes: self.modes.mode(key, mode),
        }
    }
}

impl<P, K, L> Ossm<P, K, L>
where
    P: Platform,
    K: Copy + PartialEq + Debug + 'static,
    L: ModeList<K>,
{
    /// The mode to enter at boot. Defaults to [`Idle`](ModeKey::Idle).
    pub fn start_in(self, key: impl Into<ModeKey<K>>) -> Self {
        Self {
            modes: self.modes.start_in(key),
            ..self
        }
    }

    /// Set up the platform and run the modes for the rest of the program.
    ///
    /// Can only be called once: the motion channels and pattern engine
    /// it creates are unique to the program.
    pub async fn run(mut self) -> ! {
        let (receiver, motion_observer, motion) = CHANNELS.init(MotionChannels::new()).split();
        let (runner, pattern_observer, patterns) = ENGINE.init(PatternEngine::new()).split();

        let hardware = match self.platform.init().await {
            Ok(hardware) => hardware,
            Err(error) => {
                self.platform.start_indicator(motion_observer, pattern_observer);
                fault::halt(error).await
            }
        };

        let controller = receiver.into_controller(
            hardware.board,
            hardware.limits.clone(),
            hardware.update_interval_secs,
        );
        self.platform.start_motion(controller).await;

        self.platform.start_indicator(motion_observer, pattern_observer);
        let patterns = PATTERN_SENDER.init(patterns);
        self.platform.start_remotes(patterns, &hardware.limits);

        self.modes.run(motion, runner).await
    }
}
