use crate::commands::PatternCommand;
use crate::engine::{EngineCommand, EngineState, PatternEngine};
use crate::input::PatternInput;

/// Sender half of the pattern engine.
///
/// Holds the capability to issue engine commands (`play`, `pause`,
/// `resume`, `stop`, `home`) and to mutate the live pattern input
/// (`set_speed`, `set_depth`, `set_stroke`, `set_sensation`), plus the
/// read methods from [`PatternObserver`](crate::PatternObserver) -
/// command issuers almost always also need to read state to plan,
/// and reading is harmless. Code that only needs to observe should
/// still take `&PatternObserver` for the tighter contract.
///
/// Produced by [`PatternEngine::split`](crate::PatternEngine::split).
/// Not [`Clone`] and not publicly constructible. The intended pattern
/// is to hand `&PatternSender` (or own a static one) to every
/// subsystem that needs to issue commands.
pub struct PatternSender {
    engine: &'static PatternEngine,
}

impl PatternSender {
    pub(crate) fn new(engine: &'static PatternEngine) -> Self {
        Self { engine }
    }

    pub fn play(&self, index: usize) {
        let _ = self.engine.commands.try_send(EngineCommand::Play(index));
    }

    pub fn pause(&self) {
        let _ = self.engine.commands.try_send(EngineCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.engine.commands.try_send(EngineCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = self.engine.commands.try_send(EngineCommand::Stop);
        events::state::<PatternInput>().update(|input| {
            input.velocity = 0.0;
        });
    }

    pub fn home(&self) {
        let _ = self.engine.commands.try_send(EngineCommand::Home);
    }

    /// Set velocity as a fraction of max velocity. Clamped to `0.0..=1.0`.
    pub fn set_speed(&self, value: f64) {
        events::state::<PatternInput>().update(|input| {
            input.velocity = value.clamp(0.0, 1.0);
        });
    }

    /// Set stroke as a fraction of depth. Clamped to `0.0..=1.0`.
    pub fn set_stroke(&self, value: f64) {
        events::state::<PatternInput>().update(|input| {
            input.stroke = value.clamp(0.0, 1.0);
        });
    }

    /// Set depth as a fraction of machine range. Clamped to `0.0..=1.0`.
    pub fn set_depth(&self, value: f64) {
        events::state::<PatternInput>().update(|input| {
            input.depth = value.clamp(0.0, 1.0);
        });
    }

    /// Set sensation (pattern-specific). Clamped to `-1.0..=1.0`.
    pub fn set_sensation(&self, value: f64) {
        events::state::<PatternInput>().update(|input| {
            input.sensation = value.clamp(-1.0, 1.0);
        });
    }

    /// Apply a single [`PatternCommand`].
    ///
    /// A long-running task subscribes to `PatternCommand` events and
    /// calls `apply` for each one.
    pub fn apply(&self, cmd: PatternCommand) {
        match cmd {
            PatternCommand::Play(idx) => self.play(idx),
            PatternCommand::Pause => self.pause(),
            PatternCommand::Resume => self.resume(),
            PatternCommand::Stop => self.stop(),
            PatternCommand::Home => self.home(),
            PatternCommand::SetSpeed(v) => self.set_speed(v),
            PatternCommand::SetStroke(v) => self.set_stroke(v),
            PatternCommand::SetDepth(v) => self.set_depth(v),
            PatternCommand::SetSensation(v) => self.set_sensation(v),
        }
    }

    /// Current engine state (idle, homing, playing, paused, ready).
    ///
    /// `PatternSender` carries [`PatternObserver`](crate::PatternObserver)'s
    /// read capability so command-issuing code can plan against current
    /// state without also being handed an observer.
    pub fn state(&self) -> EngineState {
        events::state::<EngineState>().read()
    }

    /// Current pattern input (depth, stroke, velocity, sensation).
    pub fn input(&self) -> PatternInput {
        events::state::<PatternInput>().read()
    }

    /// Subscribe to [`EngineState`] transitions.
    pub fn subscribe(&self) -> events::StateSubscription<EngineState> {
        events::state::<EngineState>().subscribe()
    }
}
