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
enum EngineCommand {
    Play(usize),
    Pause,
    Resume,
    Stop,
    Home,
}

type EngineCommandChannel = Channel<CriticalSectionRawMutex, EngineCommand, 4>;

/// Observable engine state values written to [`PatternEngineChannels`].
pub mod engine_state {
    pub const IDLE: u8 = 0;
    pub const HOMING: u8 = 1;
    pub const PLAYING: u8 = 2;
    pub const PAUSED: u8 = 3;
    pub const READY: u8 = 4;
}

/// Shared channels and state for communication between the
/// [`PatternEngine`] handle and the [`PatternEngineRunner`] async task.
///
/// Create as a `static` and pass a reference to
/// [`PatternEngine::new()`].
pub struct PatternEngineChannels {
    commands: EngineCommandChannel,
    state: AtomicU8,
}

impl PatternEngineChannels {
    pub const fn new() -> Self {
        Self {
            commands: EngineCommandChannel::new(),
            state: AtomicU8::new(engine_state::IDLE),
        }
    }

    pub fn play(&self, index: usize) {
        let _ = self.commands.try_send(EngineCommand::Play(index));
    }

    pub fn pause(&self) {
        let _ = self.commands.try_send(EngineCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.try_send(EngineCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = self.commands.try_send(EngineCommand::Stop);
    }

    pub fn home(&self) {
        let _ = self.commands.try_send(EngineCommand::Home);
    }

    /// Current engine state as a `u8`. Compare with [`engine_state`] constants.
    pub fn state(&self) -> u8 {
        self.state.load(Ordering::Relaxed)
    }
}

/// Thin handle for sending commands to, and reading state from, the
/// pattern engine.
///
/// Create via [`PatternEngine::new()`], which returns this handle
/// alongside a [`PatternEngineRunner`] that should be spawned as an
/// async task.
pub struct PatternEngine<'a> {
    channels: &'a PatternEngineChannels,
}

impl<'a> PatternEngine<'a> {
    /// Create a new pattern engine handle and its runner.
    ///
    /// The handle is used to send commands and read state.
    /// The runner should be spawned as an async task via
    /// [`PatternEngineRunner::run()`].
    pub fn new<const N: usize>(
        patterns: [AnyPattern; N],
        channels: &'a PatternEngineChannels,
    ) -> (Self, PatternEngineRunner<'a, N>) {
        let handle = Self { channels };
        let runner = PatternEngineRunner {
            channels,
            patterns,
            state: EngineState::Idle,
        };
        (handle, runner)
    }

    pub fn play(&self, index: usize) {
        self.channels.play(index);
    }

    pub fn pause(&self) {
        self.channels.pause();
    }

    pub fn resume(&self) {
        self.channels.resume();
    }

    pub fn stop(&self) {
        self.channels.stop();
    }

    pub fn home(&self) {
        self.channels.home();
    }

    /// Current engine state as a `u8`. Compare with [`engine_state`] constants.
    pub fn state(&self) -> u8 {
        self.channels.state()
    }
}

#[derive(Debug, Clone, Copy)]
enum EngineState {
    Idle,
    Homing(Option<usize>),
    Ready,
    Playing(usize),
    Paused(usize),
}

impl EngineState {
    fn as_u8(self) -> u8 {
        match self {
            Self::Idle => engine_state::IDLE,
            Self::Homing(_) => engine_state::HOMING,
            Self::Ready => engine_state::READY,
            Self::Playing(_) => engine_state::PLAYING,
            Self::Paused(_) => engine_state::PAUSED,
        }
    }
}

pub struct PatternEngineRunner<'a, const N: usize> {
    channels: &'a PatternEngineChannels,
    patterns: [AnyPattern; N],
    state: EngineState,
}

impl<'a, const N: usize> PatternEngineRunner<'a, N> {
    /// Run the engine forever, processing commands and driving patterns.
    ///
    /// This method never returns. It should be the last `.await` in the
    /// pattern task, or spawned as a dedicated async task.
    ///
    /// `delay` must implement `Clone` so a fresh [`PatternCtx`] can be created
    /// each time a pattern starts. All embassy `Delay` types are `Copy`.
    pub async fn run<D: DelayNs + Clone>(
        &mut self,
        ossm_channels: &'static OssmChannels,
        input: &'static SharedPatternInput,
        delay: D,
    ) -> ! {
        loop {
            match self.state {
                EngineState::Idle | EngineState::Ready | EngineState::Paused(_) => {
                    let cmd = self.channels.commands.receive().await;
                    self.handle_command(cmd, ossm_channels);
                }
                EngineState::Homing(maybe_idx) => {
                    ossm_channels.homing_done.reset();
                    let _ = ossm_channels.commands.try_send(Command::Enable);
                    let _ = ossm_channels.commands.try_send(Command::Home);

                    let result = select::select(
                        ossm_channels.homing_done.wait(),
                        self.channels.commands.receive(),
                    )
                    .await;

                    match result {
                        Either::First(()) => match maybe_idx {
                            Some(idx) => self.set_state(EngineState::Playing(idx)),
                            None => self.set_state(EngineState::Ready),
                        },
                        Either::Second(cmd) => {
                            self.handle_command(cmd, ossm_channels);
                        }
                    }
                }
                EngineState::Playing(idx) => {
                    let mut ctx = PatternCtx::new(ossm_channels, input, delay.clone());

                    let result = select::select(
                        self.patterns[idx].run(&mut ctx),
                        self.channels.commands.receive(),
                    )
                    .await;

                    match result {
                        Either::First(()) => {
                            // Pattern returned (unusual — they normally loop forever).
                            self.set_state(EngineState::Idle);
                        }
                        Either::Second(cmd) => {
                            self.handle_command(cmd, ossm_channels);
                        }
                    }
                }
            }
        }
    }

    fn set_state(&mut self, state: EngineState) {
        self.state = state;
        self.channels.state.store(state.as_u8(), Ordering::Relaxed);
    }

    fn handle_command(&mut self, cmd: EngineCommand, ossm_channels: &OssmChannels) {
        match cmd {
            EngineCommand::Play(idx) => {
                if idx < N {
                    let next = match self.state {
                        EngineState::Idle => EngineState::Homing(Some(idx)),
                        EngineState::Paused(_) => {
                            let _ = ossm_channels.commands.try_send(Command::Resume);
                            EngineState::Playing(idx)
                        }
                        _ => EngineState::Playing(idx),
                    };
                    self.set_state(next);
                }
            }
            EngineCommand::Pause => {
                if let EngineState::Playing(idx) = self.state {
                    let _ = ossm_channels.commands.try_send(Command::Pause);
                    self.set_state(EngineState::Paused(idx));
                }
            }
            EngineCommand::Resume => {
                if let EngineState::Paused(idx) = self.state {
                    let _ = ossm_channels.commands.try_send(Command::Resume);
                    self.set_state(EngineState::Playing(idx));
                }
            }
            EngineCommand::Stop => {
                let _ = ossm_channels.commands.try_send(Command::Disable);
                self.set_state(EngineState::Idle);
            }
            EngineCommand::Home => {
                if let EngineState::Idle = self.state {
                    self.set_state(EngineState::Homing(None));
                }
            }
        }
    }
}
