use crate::engine::{EngineState, PatternEngine};
use crate::input::PatternInput;

/// Read-only observer of the pattern engine.
///
/// Produced by [`PatternEngine::split`](crate::PatternEngine::split)
/// alongside [`PatternSender`](crate::PatternSender) and
/// [`PatternRunner`](crate::PatternRunner). Anyone with a borrow of
/// the observer can read engine state, subscribe to state transitions,
/// and read the current pattern input, but cannot issue commands.
///
/// Not [`Clone`] and not publicly constructible. State and input live
/// in the global event bus; this type is just a capability-gated
/// projection of `events::state::<EngineState>()` and
/// `events::state::<PatternInput>()`.
pub struct PatternObserver {
    _engine: &'static PatternEngine,
}

impl PatternObserver {
    pub(crate) fn new(engine: &'static PatternEngine) -> Self {
        Self { _engine: engine }
    }

    /// Current engine state (idle, homing, playing, paused, ready).
    pub fn state(&self) -> EngineState {
        events::state::<EngineState>().read()
    }

    /// Current pattern input (depth, stroke, velocity, sensation).
    pub fn input(&self) -> PatternInput {
        events::state::<PatternInput>().read()
    }

    /// Subscribe to [`EngineState`] transitions.
    ///
    /// The subscription yields the current state on first
    /// [`changed()`](events::StateSubscription::changed), then each
    /// subsequent transition.
    pub fn subscribe(&self) -> events::StateSubscription<EngineState> {
        events::state::<EngineState>().subscribe()
    }
}
