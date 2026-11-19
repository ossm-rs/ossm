use core::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};

use crate::engine::{EnginePhase, PatternEngine};
use crate::input::{PatternEvent, PatternState, PatternUpdate};

/// Sender half of the pattern engine.
///
/// Holds the capability to send playback events and to change the live
/// pattern state, plus the read methods from
/// [`PatternObserver`](crate::PatternObserver) - senders almost always
/// also need to read state to plan, and reading is harmless. Code that
/// only needs to observe should still take `&PatternObserver` for the
/// tighter contract.
///
/// Produced by [`PatternEngine::split`](crate::PatternEngine::split).
/// Not [`Clone`] and not publicly constructible.
pub struct PatternSender {
    engine: &'static PatternEngine,
}

impl PatternSender {
    pub(crate) fn new(engine: &'static PatternEngine) -> Self {
        Self { engine }
    }

    /// Send a playback event. Dropped if the engine is four events behind.
    pub fn send(&self, event: PatternEvent) {
        let _ = self.engine.events.try_send(event);
    }

    /// Apply one update, clamped to its range.
    pub fn update(&self, update: PatternUpdate) {
        self.engine.state.sender().send_modify(|state| {
            if let Some(state) = state {
                state.apply(update);
            }
        });
    }

    /// Replace the whole state at once. Wakes the running pattern only if
    /// something changed.
    pub fn set(&self, state: PatternState) {
        self.engine.state.sender().send_if_modified(|current| {
            let changed = *current != Some(state);
            *current = Some(state);
            changed
        });
    }

    /// Current engine phase (idle, homing, playing, paused, ready).
    ///
    /// `PatternSender` carries [`PatternObserver`](crate::PatternObserver)'s
    /// read capability so command-issuing code can plan against current
    /// state without also being handed an observer.
    pub fn phase(&self) -> EnginePhase {
        EnginePhase::decode(self.engine.phase.load(Ordering::Relaxed))
    }

    /// Current pattern state (depth, stroke, speed, sensation).
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
