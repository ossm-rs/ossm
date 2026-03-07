use core::sync::atomic::{AtomicU16, Ordering};

use embassy_futures::select::{self, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_hal_async::delay::DelayNs;
use ossm::{Command, Ossm, OssmChannels};

use crate::any_pattern::AnyPattern;
use crate::input::SharedPatternInput;
use crate::pattern::{Pattern, PatternCtx};

#[derive(Debug, Clone, Copy)]
enum EngineCommand {
    Play(usize),
    Stop,
    Home,
}

type EngineCommandChannel = Channel<CriticalSectionRawMutex, EngineCommand, 4>;

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

    const fn encode(self) -> u16 {
        match self {
            Self::Idle => (Self::TAG_IDLE as u16) << 8,
            Self::Homing => (Self::TAG_HOMING as u16) << 8,
            Self::Ready => (Self::TAG_READY as u16) << 8,
            Self::Playing(idx) => ((Self::TAG_PLAYING as u16) << 8) | idx as u16,
            Self::Paused(idx) => ((Self::TAG_PAUSED as u16) << 8) | idx as u16,
        }
    }

    fn decode(v: u16) -> Self {
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

/// Shared channels and state for communication between the
/// [`PatternEngine`] handle and the [`PatternEngineRunner`] async task.
///
/// Create as a `static` and pass a reference to
/// [`PatternEngine::new()`].
pub struct PatternEngineChannels {
    commands: EngineCommandChannel,
    state: AtomicU16,
}

impl PatternEngineChannels {
    pub const fn new() -> Self {
        Self {
            commands: EngineCommandChannel::new(),
            state: AtomicU16::new(EngineState::Idle.encode()),
        }
    }

    fn play(&self, index: usize) {
        let _ = self.commands.try_send(EngineCommand::Play(index));
    }

    fn stop(&self) {
        let _ = self.commands.try_send(EngineCommand::Stop);
    }

    fn home(&self) {
        let _ = self.commands.try_send(EngineCommand::Home);
    }

    /// Current engine state.
    pub fn state(&self) -> EngineState {
        EngineState::decode(self.state.load(Ordering::Relaxed))
    }

    fn store(&self, state: EngineState) {
        self.state.store(state.encode(), Ordering::Relaxed);
    }
}

/// Thin handle for sending commands to, and reading state from, the
/// pattern engine.
///
/// Create via [`PatternEngine::new()`], which returns this handle
/// alongside a [`PatternEngineRunner`] that should be spawned as an
/// async task.
#[derive(Clone, Copy)]
pub struct PatternEngine {
    channels: &'static PatternEngineChannels,
    ossm: Ossm,
}

impl PatternEngine {
    /// Create a new pattern engine handle and its runner.
    ///
    /// The handle is used to send commands and read state.
    /// The runner should be spawned as an async task via
    /// [`PatternEngineRunner::run()`].
    pub fn new<const N: usize>(
        patterns: [AnyPattern; N],
        channels: &'static PatternEngineChannels,
        ossm: Ossm,
    ) -> (Self, PatternEngineRunner<N>) {
        let handle = Self { channels, ossm };
        let runner = PatternEngineRunner {
            channels,
            patterns,
            state: RunnerState::Idle,
        };
        (handle, runner)
    }

    pub fn play(&self, index: usize) {
        if let EngineState::Paused(current) = self.channels.state() {
            self.ossm.resume();
            if current == index {
                self.channels.store(EngineState::Playing(index));
                return;
            }
        }
        self.channels.store(EngineState::Playing(index));
        self.channels.play(index);
    }

    pub fn pause(&self) {
        if let EngineState::Playing(idx) = self.channels.state() {
            self.ossm.pause();
            self.channels.store(EngineState::Paused(idx));
        }
    }

    pub fn resume(&self) {
        if let EngineState::Paused(idx) = self.channels.state() {
            self.ossm.resume();
            self.channels.store(EngineState::Playing(idx));
        }
    }

    pub fn stop(&self) {
        self.channels.stop();
    }

    pub fn home(&self) {
        self.channels.home();
    }

    pub fn state(&self) -> EngineState {
        self.channels.state()
    }
}

/// Internal runner state. Carries extra detail (e.g. which pattern to play
/// after homing) that the public [`EngineState`] does not expose.
#[derive(Debug, Clone, Copy)]
enum RunnerState {
    Idle,
    Homing(Option<usize>),
    Ready,
    Playing(usize),
}

impl RunnerState {
    fn as_engine_state(self) -> EngineState {
        match self {
            Self::Idle => EngineState::Idle,
            Self::Homing(_) => EngineState::Homing,
            Self::Ready => EngineState::Ready,
            Self::Playing(idx) => EngineState::Playing(idx),
        }
    }
}

pub struct PatternEngineRunner<const N: usize> {
    channels: &'static PatternEngineChannels,
    patterns: [AnyPattern; N],
    state: RunnerState,
}

impl<const N: usize> PatternEngineRunner<N> {
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
                RunnerState::Idle | RunnerState::Ready => {
                    let cmd = self.channels.commands.receive().await;
                    self.handle_command(cmd, ossm_channels);
                }
                RunnerState::Homing(maybe_idx) => {
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
                            Some(idx) => self.set_state(RunnerState::Playing(idx)),
                            None => self.set_state(RunnerState::Ready),
                        },
                        Either::Second(cmd) => {
                            self.handle_command(cmd, ossm_channels);
                        }
                    }
                }
                RunnerState::Playing(idx) => {
                    let mut ctx = PatternCtx::new(ossm_channels, input, delay.clone());

                    let result = select::select(
                        self.patterns[idx].run(&mut ctx),
                        self.channels.commands.receive(),
                    )
                    .await;

                    match result {
                        Either::First(()) => {
                            // Pattern returned (unusual — they normally loop forever).
                            self.set_state(RunnerState::Idle);
                        }
                        Either::Second(cmd) => {
                            self.handle_command(cmd, ossm_channels);
                        }
                    }
                }
            }
        }
    }

    fn set_state(&mut self, state: RunnerState) {
        self.state = state;
        self.channels.store(state.as_engine_state());
    }

    fn handle_command(&mut self, cmd: EngineCommand, ossm_channels: &OssmChannels) {
        match cmd {
            EngineCommand::Play(idx) => {
                if idx < N {
                    let next = match self.state {
                        RunnerState::Idle => RunnerState::Homing(Some(idx)),
                        _ => RunnerState::Playing(idx),
                    };
                    self.set_state(next);
                }
            }
            EngineCommand::Stop => {
                let _ = ossm_channels.commands.try_send(Command::Disable);
                self.set_state(RunnerState::Idle);
            }
            EngineCommand::Home => {
                if let RunnerState::Idle = self.state {
                    self.set_state(RunnerState::Homing(None));
                }
            }
        }
    }
}
