//! Public API for remote adapters
//!
//! Any new input device that wants to control the pattern engine should
//! use these commands.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};

use crate::{AnyPattern, EngineState, PatternEngine, PatternInput, PatternMeta};

/// Unified pattern command. Flows through the `input` event bus.
///
/// Combines what used to split as `PlaybackCommand` (one-shot transitions:
/// Play/Pause/Stop/Home) and `InputCommand` (continuous knob values:
/// SetSpeed/SetDepth/SetStroke/SetSensation). Transports publish a single
/// `PatternCmd` and the consumer side ([`dispatch_pattern_cmd`]) routes
/// each variant into the engine.
#[derive(Debug, Clone, Copy)]
pub enum PatternCmd {
    Play(usize),
    Pause,
    Resume,
    Stop,
    Home,
    /// 0.0-1.0 (fraction of max velocity).
    SetSpeed(f64),
    /// 0.0-1.0 (fraction of depth).
    SetStroke(f64),
    /// 0.0-1.0 (fraction of machine range).
    SetDepth(f64),
    /// -1.0-1.0 (pattern-specific).
    SetSensation(f64),
}

events::declare_event!(PatternCmd);

#[derive(Debug, Clone, Copy)]
pub enum PlaybackCommand {
    Play(usize),
    Pause,
    Resume,
    Stop,
    Home,
}

/// Input values are clamped to their valid range on dispatch.
#[derive(Debug, Clone, Copy)]
pub enum InputCommand {
    /// 0.0-1.0 (fraction of max velocity).
    SetSpeed(f64),
    /// 0.0-1.0 (fraction of depth).
    SetStroke(f64),
    /// 0.0-1.0 (fraction of machine range).
    SetDepth(f64),
    /// -1.0-1.0 (pattern-specific).
    SetSensation(f64),
}

impl From<PlaybackCommand> for PatternCmd {
    fn from(cmd: PlaybackCommand) -> Self {
        match cmd {
            PlaybackCommand::Play(i) => Self::Play(i),
            PlaybackCommand::Pause => Self::Pause,
            PlaybackCommand::Resume => Self::Resume,
            PlaybackCommand::Stop => Self::Stop,
            PlaybackCommand::Home => Self::Home,
        }
    }
}

impl From<InputCommand> for PatternCmd {
    fn from(cmd: InputCommand) -> Self {
        match cmd {
            InputCommand::SetSpeed(v) => Self::SetSpeed(v),
            InputCommand::SetStroke(v) => Self::SetStroke(v),
            InputCommand::SetDepth(v) => Self::SetDepth(v),
            InputCommand::SetSensation(v) => Self::SetSensation(v),
        }
    }
}

pub fn dispatch_playback(engine: &PatternEngine, cmd: PlaybackCommand) {
    match cmd {
        PlaybackCommand::Play(idx) => engine.play(idx),
        PlaybackCommand::Pause => engine.pause(),
        PlaybackCommand::Resume => engine.resume(),
        PlaybackCommand::Stop => engine.stop(),
        PlaybackCommand::Home => engine.home(),
    }
}

pub fn dispatch_input(engine: &PatternEngine, cmd: InputCommand) {
    engine.input().sender().send_modify(|opt| {
        if let Some(input) = opt {
            match cmd {
                InputCommand::SetSpeed(v) => input.velocity = v.clamp(0.0, 1.0),
                InputCommand::SetStroke(v) => input.stroke = v.clamp(0.0, 1.0),
                InputCommand::SetDepth(v) => input.depth = v.clamp(0.0, 1.0),
                InputCommand::SetSensation(v) => input.sensation = v.clamp(-1.0, 1.0),
            }
        }
    });
}

/// Apply a single [`PatternCmd`] to a pattern engine.
///
/// The consumer side of the events bus. A long-running task subscribes
/// to `PatternCmd` events and calls this for each one; semantically the
/// same as `dispatch_playback` + `dispatch_input` together.
pub fn dispatch_pattern_cmd(engine: &PatternEngine, cmd: PatternCmd) {
    match cmd {
        PatternCmd::Play(idx) => engine.play(idx),
        PatternCmd::Pause => engine.pause(),
        PatternCmd::Resume => engine.resume(),
        PatternCmd::Stop => engine.stop(),
        PatternCmd::Home => engine.home(),
        PatternCmd::SetSpeed(v) => engine.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.velocity = v.clamp(0.0, 1.0);
            }
        }),
        PatternCmd::SetStroke(v) => engine.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.stroke = v.clamp(0.0, 1.0);
            }
        }),
        PatternCmd::SetDepth(v) => engine.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.depth = v.clamp(0.0, 1.0);
            }
        }),
        PatternCmd::SetSensation(v) => engine.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.sensation = v.clamp(-1.0, 1.0);
            }
        }),
    }
}

pub fn current_state(engine: &PatternEngine) -> EngineState {
    engine.state()
}

pub fn current_input(engine: &PatternEngine) -> PatternInput {
    engine.input().try_get().unwrap_or(PatternInput::DEFAULT)
}

pub fn pattern_list() -> &'static [PatternMeta] {
    &AnyPattern::BUILTIN_PATTERNS
}

pub fn pattern_description(idx: usize) -> Option<&'static str> {
    AnyPattern::BUILTIN_PATTERNS.get(idx).map(|m| m.description)
}

pub type StateSubscriber<'a> = Subscriber<'a, CriticalSectionRawMutex, EngineState, 1, 8, 0>;

pub fn subscribe_state(engine: &PatternEngine) -> Result<StateSubscriber<'_>, pubsub::Error> {
    engine.state_subscriber()
}
