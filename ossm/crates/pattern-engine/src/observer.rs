use core::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};

use crate::engine::{EnginePhase, PatternEngine};
use crate::input::PatternState;

/// Read-only observer of the pattern engine.
///
/// Produced by [`PatternEngine::split`](crate::PatternEngine::split)
/// alongside [`PatternSender`](crate::PatternSender) and
/// [`PatternRunner`](crate::PatternRunner). Anyone with a borrow of
/// the observer can read the engine phase, subscribe to phase transitions,
/// and read the current pattern state, but cannot issue commands.
///
/// Not [`Clone`] and not publicly constructible.
pub struct PatternObserver {
    engine: &'static PatternEngine,
}

impl PatternObserver {
    pub(crate) fn new(engine: &'static PatternEngine) -> Self {
        Self { engine }
    }

    /// Current engine phase (idle, homing, playing, paused, ready).
    pub fn phase(&self) -> EnginePhase {
        EnginePhase::decode(self.engine.phase.load(Ordering::Relaxed))
    }

    /// Current pattern state (depth, stroke, speed, sensation).
    ///
    /// Returns the defaults if the watch has not been initialised (i.e.
    /// nothing has set any value yet).
    pub fn state(&self) -> PatternState {
        self.engine.state.try_get().unwrap_or(PatternState::DEFAULT)
    }

    /// Subscribe to [`EnginePhase`] transitions.
    ///
    /// Returns `Err` if all subscriber slots are in use.
    pub fn subscribe(
        &self,
    ) -> Result<Subscriber<'static, CriticalSectionRawMutex, EnginePhase, 1, 8, 0>, pubsub::Error>
    {
        self.engine.phase_channel.subscriber()
    }
}
