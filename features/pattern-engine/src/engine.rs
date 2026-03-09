use core::cell::Cell;
use core::sync::atomic::{AtomicU16, Ordering};

use embassy_futures::select::{self, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_hal_async::delay::DelayNs;
use ossm::Ossm;

use crate::any_pattern::AnyPattern;
use crate::input::{PatternInput, SharedPatternInput};
use crate::pattern::{Pattern, PatternCtx};

#[derive(Debug, Clone, Copy)]
enum EngineCommand {
    Play(usize),
    Stop,
    Home,
    Pause,
    Resume,
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

struct PatternEngineChannels {
    commands: EngineCommandChannel,
    state: AtomicU16,
}

impl PatternEngineChannels {
    const fn new() -> Self {
        Self {
            commands: EngineCommandChannel::new(),
            state: AtomicU16::new(EngineState::Idle.encode()),
        }
    }

    fn state(&self) -> EngineState {
        EngineState::decode(self.state.load(Ordering::Relaxed))
    }

    fn store(&self, state: EngineState) {
        self.state.store(state.encode(), Ordering::Relaxed);
    }
}

/// Pattern engine that owns its command channels and delegates motion
/// to an [`Ossm`] instance.
///
/// Create as a `static` and use `&'static PatternEngine` as the handle
/// for sending commands and reading state. Create a
/// [`PatternEngineRunner`] via [`runner()`](Self::runner) and spawn it
/// as an async task.
pub struct PatternEngine {
    channels: PatternEngineChannels,
    input: SharedPatternInput,
    ossm: &'static Ossm,
}

impl PatternEngine {
    pub const fn new(ossm: &'static Ossm) -> Self {
        Self {
            channels: PatternEngineChannels::new(),
            input: SharedPatternInput::new(Cell::new(PatternInput::DEFAULT)),
            ossm,
        }
    }

    pub fn input(&self) -> &SharedPatternInput {
        &self.input
    }

    pub fn runner<const N: usize>(
        &'static self,
        patterns: [AnyPattern; N],
    ) -> PatternEngineRunner<N> {
        PatternEngineRunner {
            engine: self,
            patterns,
            state: RunnerState::Idle,
        }
    }

    pub fn ossm(&self) -> &Ossm {
        self.ossm
    }

    pub fn play(&self, index: usize) {
        self.channels.store(EngineState::Playing(index));
        let _ = self.channels.commands.try_send(EngineCommand::Play(index));
    }

    pub fn pause(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Stop);
    }

    pub fn home(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Home);
    }

    pub fn state(&self) -> EngineState {
        self.channels.state()
    }
}

/// Internal runner state.
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
    engine: &'static PatternEngine,
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
        delay: D,
    ) -> ! {
        let ossm = self.engine.ossm();
        let input = self.engine.input();

        loop {
            match self.state {
                RunnerState::Idle | RunnerState::Ready => {
                    let cmd = self.engine.channels.commands.receive().await;
                    self.handle_command(cmd).await;
                }
                RunnerState::Homing(maybe_idx) => {
                    ossm.enable().await;

                    let result = select::select(
                        ossm.home(),
                        self.engine.channels.commands.receive(),
                    )
                    .await;

                    match result {
                        Either::First(_) => match maybe_idx {
                            Some(idx) => self.set_state(RunnerState::Playing(idx)),
                            None => self.set_state(RunnerState::Ready),
                        },
                        Either::Second(cmd) => {
                            self.handle_command(cmd).await;
                        }
                    }
                }
                RunnerState::Playing(idx) => {
                    let mut ctx = PatternCtx::new(ossm, input, delay.clone());

                    // Split borrows: the pinned future holds `patterns[idx]`,
                    // so we access `engine` and `state` through separate refs.
                    let engine = self.engine;
                    let state = &mut self.state;
                    let pattern_fut = core::pin::pin!(
                        self.patterns[idx].run(&mut ctx)
                    );
                    let mut pattern_fut = pattern_fut;

                    loop {
                        let result = select::select(
                            pattern_fut.as_mut(),
                            engine.channels.commands.receive(),
                        )
                        .await;

                        match result {
                            Either::First(_result) => {
                                if matches!(*state, RunnerState::Playing(_)) {
                                    *state = RunnerState::Idle;
                                    engine.channels.store(EngineState::Idle);
                                }
                                break;
                            }
                            Either::Second(cmd) => match cmd {
                                EngineCommand::Pause => {
                                    let _ = ossm.pause().await;
                                    engine.channels.store(EngineState::Paused(idx));
                                }
                                EngineCommand::Resume => {
                                    let _ = ossm.resume().await;
                                    engine.channels.store(EngineState::Playing(idx));
                                }
                                EngineCommand::Play(i) if i == idx => {
                                    let _ = ossm.resume().await;
                                    engine.channels.store(EngineState::Playing(idx));
                                }
                                EngineCommand::Play(new_idx) if new_idx < N => {
                                    *state = RunnerState::Playing(new_idx);
                                    engine.channels.store(EngineState::Playing(new_idx));
                                    break;
                                }
                                EngineCommand::Stop => {
                                    let _ = ossm.disable().await;
                                    *state = RunnerState::Idle;
                                    engine.channels.store(EngineState::Idle);
                                    break;
                                }
                                _ => {}
                            },
                        }
                    }
                }
            }
        }
    }

    fn set_state(&mut self, state: RunnerState) {
        self.state = state;
        self.engine.channels.store(state.as_engine_state());
    }

    async fn handle_command(&mut self, cmd: EngineCommand) {
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
                let _ = self.engine.ossm().disable().await;
                self.set_state(RunnerState::Idle);
            }
            EngineCommand::Home => {
                if let RunnerState::Idle = self.state {
                    self.set_state(RunnerState::Homing(None));
                }
            }
            EngineCommand::Pause | EngineCommand::Resume => {
                // Only handled inside the Playing inner loop.
            }
        }
    }
}
