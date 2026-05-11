use core::sync::atomic::AtomicU16;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::PubSubChannel;

use crate::input::{PatternInput, SharedPatternInput};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Idle,
    Homing,
    Ready,
    Playing(usize),
    Paused(usize),
}

impl EngineState {
    const TAG_IDLE: u8 = 0;
    const TAG_HOMING: u8 = 1;
    const TAG_PLAYING: u8 = 2;
    const TAG_PAUSED: u8 = 3;
    const TAG_READY: u8 = 4;

    pub(crate) const fn encode(self) -> u16 {
        match self {
            Self::Idle => (Self::TAG_IDLE as u16) << 8,
            Self::Homing => (Self::TAG_HOMING as u16) << 8,
            Self::Ready => (Self::TAG_READY as u16) << 8,
            Self::Playing(idx) => ((Self::TAG_PLAYING as u16) << 8) | idx as u16,
            Self::Paused(idx) => ((Self::TAG_PAUSED as u16) << 8) | idx as u16,
        }
    }

    pub(crate) fn decode(v: u16) -> Self {
        let tag = (v >> 8) as u8;
        let idx = (v & 0xFF) as usize;
        match tag {
            Self::TAG_HOMING => Self::Homing,
            Self::TAG_PLAYING => Self::Playing(idx),
            Self::TAG_PAUSED => Self::Paused(idx),
            Self::TAG_READY => Self::Ready,
            _ => Self::Idle,
        }
    }

    /// Numeric tag for the wasm/TypeScript boundary.
    ///
    /// 0 = idle, 1 = homing, 2 = playing, 3 = paused, 4 = ready.
    pub fn as_u8(self) -> u8 {
        (self.encode() >> 8) as u8
    }
}

/// Broadcast channel for [`EngineState`].
///
/// - `CAP = 1`: only the latest transition matters.
/// - `SUBS = 8`: up to 8 concurrent async subscribers.
/// - `PUBS = 0`: publishing uses `immediate_publisher`.
pub(crate) type StateChannel = PubSubChannel<CriticalSectionRawMutex, EngineState, 1, 8, 0>;

/// Root container for a pattern engine.
///
/// Carries the command channel, shared pattern input, and state pubsub
/// that the three capability handles project from. `PatternEngine` is
/// instantiated once in static storage and immediately consumed by
/// [`split`](Self::split). After split returns, nothing in the program
/// holds an `&PatternEngine`: the three named capabilities cover every
/// role.
pub struct PatternEngine {
    pub(crate) commands: EngineCommandChannel,
    pub(crate) state: AtomicU16,
    pub(crate) input: SharedPatternInput,
    pub(crate) state_channel: StateChannel,
}

impl PatternEngine {
    pub const fn new() -> Self {
        Self {
            commands: EngineCommandChannel::new(),
            state: AtomicU16::new(EngineState::Idle.encode()),
            input: SharedPatternInput::new_with(PatternInput::DEFAULT),
            state_channel: StateChannel::new(),
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
