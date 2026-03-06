use core::sync::atomic::{AtomicU8, Ordering};

use embassy_futures::select::{self, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_hal_async::delay::DelayNs;
use ossm::{Command, OssmChannels};

use crate::any_pattern::AnyPattern;
use crate::input::SharedPatternInput;
use crate::pattern::{Pattern, PatternCtx};

#[derive(Debug, Clone, Copy)]
pub enum EngineCommand {
    Play(usize),
    Pause,
    Resume,
    Stop,
}

/// Channel for sending commands to the engine.
///
/// Capacity of 4 is sufficient: commands are processed one at a time
/// and senders are typically a single UI/BLE task.
pub type EngineCommandChannel = Channel<CriticalSectionRawMutex, EngineCommand, 4>;

/// Shared atomic for observing the engine's current state from outside.
pub type SharedEngineState = AtomicU8;

/// Observable engine state values written to [`SharedEngineState`].
pub mod engine_state {
    pub const IDLE: u8 = 0;
    pub const HOMING: u8 = 1;
    pub const PLAYING: u8 = 2;
    pub const PAUSED: u8 = 3;
}

#[derive(Debug, Clone, Copy)]
enum EngineState {
    Idle,
    Homing(usize),
    Playing(usize),
    Paused(usize),
}

impl EngineState {
    fn as_u8(self) -> u8 {
        match self {
            Self::Idle => engine_state::IDLE,
            Self::Homing(_) => engine_state::HOMING,
            Self::Playing(_) => engine_state::PLAYING,
            Self::Paused(_) => engine_state::PAUSED,
        }
    }
}

pub struct PatternEngine<const N: usize> {
    patterns: [AnyPattern; N],
    state: EngineState,
}

impl<const N: usize> PatternEngine<N> {
    pub fn new(patterns: [AnyPattern; N]) -> Self {
        Self {
            patterns,
            state: EngineState::Idle,
        }
    }

    pub fn pattern_count(&self) -> usize {
        N
    }

    pub fn pattern_name(&self, index: usize) -> Option<&'static str> {
        self.patterns.get(index).map(|p| p.name())
    }

    pub fn pattern_description(&self, index: usize) -> Option<&'static str> {
        self.patterns.get(index).map(|p| p.description())
    }

    pub fn pattern_list(&self) -> impl Iterator<Item = (usize, &'static str, &'static str)> + '_ {
        self.patterns
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.name(), p.description()))
    }

    /// Run the engine forever, processing commands and driving patterns.
    ///
    /// This method never returns. It should be the last `.await` in the
    /// pattern task, or spawned as a dedicated async task.
    ///
    /// `delay` must implement `Clone` so a fresh [`PatternCtx`] can be created
    /// each time a pattern starts. All embassy `Delay` types are `Copy`.
    pub async fn run<D: DelayNs + Clone>(
        &mut self,
        engine_commands: &EngineCommandChannel,
        channels: &'static OssmChannels,
        input: &'static SharedPatternInput,
        shared_state: &SharedEngineState,
        delay: D,
    ) -> ! {
        loop {
            match self.state {
                EngineState::Idle | EngineState::Paused(_) => {
                    let cmd = engine_commands.receive().await;
                    self.handle_command(cmd, channels, shared_state);
                }
                EngineState::Homing(idx) => {
                    channels.homing_done.reset();
                    let _ = channels.commands.try_send(Command::Enable);
                    let _ = channels.commands.try_send(Command::Home);

                    let result = select::select(
                        channels.homing_done.wait(),
                        engine_commands.receive(),
                    )
                    .await;

                    match result {
                        Either::First(()) => {
                            self.set_state(EngineState::Playing(idx), shared_state);
                        }
                        Either::Second(cmd) => {
                            self.handle_command(cmd, channels, shared_state);
                        }
                    }
                }
                EngineState::Playing(idx) => {
                    let mut ctx = PatternCtx::new(channels, input, delay.clone());

                    let result = select::select(
                        self.patterns[idx].run(&mut ctx),
                        engine_commands.receive(),
                    )
                    .await;

                    match result {
                        Either::First(()) => {
                            // Pattern returned (unusual — they normally loop forever).
                            self.set_state(EngineState::Idle, shared_state);
                        }
                        Either::Second(cmd) => {
                            self.handle_command(cmd, channels, shared_state);
                        }
                    }
                }
            }
        }
    }

    fn set_state(&mut self, state: EngineState, shared: &SharedEngineState) {
        self.state = state;
        shared.store(state.as_u8(), Ordering::Relaxed);
    }

    fn handle_command(
        &mut self,
        cmd: EngineCommand,
        channels: &OssmChannels,
        shared_state: &SharedEngineState,
    ) {
        match cmd {
            EngineCommand::Play(idx) => {
                if idx < N {
                    let next = match self.state {
                        EngineState::Idle => EngineState::Homing(idx),
                        _ => EngineState::Playing(idx),
                    };
                    self.set_state(next, shared_state);
                }
            }
            EngineCommand::Pause => {
                if let EngineState::Playing(idx) = self.state {
                    let _ = channels.commands.try_send(Command::Pause);
                    self.set_state(EngineState::Paused(idx), shared_state);
                }
            }
            EngineCommand::Resume => {
                if let EngineState::Paused(idx) = self.state {
                    let _ = channels.commands.try_send(Command::Resume);
                    self.set_state(EngineState::Playing(idx), shared_state);
                }
            }
            EngineCommand::Stop => {
                self.set_state(EngineState::Idle, shared_state);
            }
        }
    }
}
