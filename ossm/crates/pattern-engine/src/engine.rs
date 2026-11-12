use core::sync::atomic::AtomicU16;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::PubSubChannel;

use crate::input::{PatternEvent, PatternState, SharedPatternState};
use crate::observer::PatternObserver;
use crate::runner::PatternRunner;
use crate::sender::PatternSender;

pub(crate) type EventChannel = Channel<CriticalSectionRawMutex, PatternEvent, 4>;

/// Lifecycle phase of the pattern engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePhase {
    Idle,
    Homing,
    Ready,
    Playing(usize),
    Paused(usize),
}

impl EnginePhase {
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

/// Broadcast channel for [`EnginePhase`].
///
/// - `CAP = 1`: only the latest transition matters.
/// - `SUBS = 8`: up to 8 concurrent async subscribers.
/// - `PUBS = 0`: publishing uses `immediate_publisher`.
pub(crate) type PhaseChannel = PubSubChannel<CriticalSectionRawMutex, EnginePhase, 1, 8, 0>;

/// Root container for a pattern engine.
///
/// Carries the event channel, shared pattern state, and phase pubsub
/// that the three capability handles project from. `PatternEngine` is
/// instantiated once in static storage and immediately consumed by
/// [`split`](Self::split). After split returns, nothing in the program
/// holds an `&PatternEngine`: the three named capabilities cover every
/// role.
pub struct PatternEngine {
    pub(crate) events: EventChannel,
    pub(crate) phase: AtomicU16,
    pub(crate) state: SharedPatternState,
    pub(crate) phase_channel: PhaseChannel,
}

impl PatternEngine {
    pub const fn new() -> Self {
        Self {
            events: EventChannel::new(),
            phase: AtomicU16::new(EnginePhase::Idle.encode()),
            state: SharedPatternState::new_with(PatternState::DEFAULT),
            phase_channel: PhaseChannel::new(),
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
    /// - [`PatternObserver`] is the read-only handle for phase, state,
    ///   and subscriptions.
    /// - [`PatternSender`] sends events and updates the state.
    pub fn split(&'static mut self) -> (PatternRunner, PatternObserver, PatternSender) {
        (
            PatternRunner::new(self),
            PatternObserver::new(self),
            PatternSender::new(self),
        )
    }
}
