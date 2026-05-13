use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

use crate::observer::PatternObserver;
use crate::runner::PatternRunner;
use crate::sender::PatternSender;

#[derive(Debug, Clone, Copy)]
pub(crate) enum EngineCommand {
    Play(usize),
    Stop,
    Home,
    Pause,
    Resume,
}

pub(crate) type EngineCommandChannel = Channel<CriticalSectionRawMutex, EngineCommand, 4>;

/// Observable state of the pattern engine.
///
/// Published as the [`State`](events::State) for `EngineState`. Read via
/// `events::state::<EngineState>().read()` or subscribe for changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Idle,
    Homing,
    Ready,
    Playing(usize),
    Paused(usize),
}

impl EngineState {
    /// Numeric tag for the wasm/TypeScript boundary.
    ///
    /// 0 = idle, 1 = homing, 2 = playing, 3 = paused, 4 = ready.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Homing => 1,
            Self::Playing(_) => 2,
            Self::Paused(_) => 3,
            Self::Ready => 4,
        }
    }
}

events::declare_state!(EngineState, EngineState::Idle);

/// Root container for a pattern engine.
///
/// Carries the command channel that the three capability handles project
/// from. `PatternEngine` is instantiated once in static storage and
/// immediately consumed by [`split`](Self::split). After split returns,
/// nothing in the program holds an `&PatternEngine`: the three named
/// capabilities cover every role.
///
/// Engine state and pattern input live in the global event bus
/// (`events::state::<EngineState>()` / `events::state::<PatternInput>()`),
/// not as fields here — that way consumers can read or subscribe without
/// holding a handle to a particular engine instance.
pub struct PatternEngine {
    pub(crate) commands: EngineCommandChannel,
}

impl PatternEngine {
    pub const fn new() -> Self {
        Self {
            commands: EngineCommandChannel::new(),
        }
    }

    /// Split into the three pattern-engine capabilities.
    ///
    /// Consumes a unique `&'static mut Self`, so it can be called at
    /// most once: nothing in the program can produce a second
    /// `&'static mut PatternEngine`. The expected boot shape is
    /// `StaticCell::init(PatternEngine::new()).split()`.
    ///
    /// - [`PatternRunner`] is the driver capability, consumed by
    ///   [`run`](PatternRunner::run) to start the engine loop.
    /// - [`PatternObserver`] is the read-only handle for state, input,
    ///   and subscriptions.
    /// - [`PatternSender`] is the command + input issuer.
    pub fn split(&'static mut self) -> (PatternRunner, PatternObserver, PatternSender) {
        (
            PatternRunner::new(self),
            PatternObserver::new(self),
            PatternSender::new(self),
        )
    }
}
