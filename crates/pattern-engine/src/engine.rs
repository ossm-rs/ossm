use core::sync::atomic::{AtomicU16, Ordering};

use embassy_futures::select::{self, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::{self, PubSubChannel};
use embedded_hal_async::delay::DelayNs;
use ossm::{MotionSender, StateResponse};

use log::info;

use crate::AnyPattern;
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

/// Broadcast channel for [`EngineState`].
///
/// Uses `PubSubChannel` so subscribers can be created and dropped
/// dynamically as services start and stop.
///
/// - `CAP = 1`: only the latest transition matters; older messages are dropped.
/// - `SUBS = 8`: up to 8 concurrent async subscribers.
/// - `PUBS = 0`: publishing uses [`PubSubChannel::immediate_publisher()`]
///   which does not consume a publisher slot.
type StateChannel = PubSubChannel<CriticalSectionRawMutex, EngineState, 1, 8, 0>;

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

/// Pattern engine that owns its command channels and shared input.
///
/// Create as a `static` and use `&'static PatternEngine` as the handle
/// for sending commands and reading state. Construct a
/// [`PatternEngineRunner`] via [`runner()`](Self::runner) and drive it
/// with `.run(delay).await` - the runner is the active driver and is
/// only alive for as long as the caller holds it.
///
/// The engine itself does **not** hold a [`MotionSender`]; it is just
/// the channel layer that remotes push to. Motion is granted to the
/// runner at construction time.
pub struct PatternEngine {
    channels: PatternEngineChannels,
    input: SharedPatternInput,
    state_channel: StateChannel,
}

impl PatternEngine {
    pub const fn new() -> Self {
        Self {
            channels: PatternEngineChannels::new(),
            input: SharedPatternInput::new_with(PatternInput::DEFAULT),
            state_channel: StateChannel::new(),
        }
    }

    pub(crate) fn input(&self) -> &SharedPatternInput {
        &self.input
    }

    pub(crate) fn state_subscriber(
        &self,
    ) -> Result<pubsub::Subscriber<'_, CriticalSectionRawMutex, EngineState, 1, 8, 0>, pubsub::Error>
    {
        self.state_channel.subscriber()
    }

    /// Build a runner bound to this engine and a [`MotionSender`].
    ///
    /// The runner is what actually drives motion. Constructing one
    /// turns this engine "live"; drop it to stop driving.
    pub fn runner<'m, const N: usize>(
        &'m self,
        motion: &'m MotionSender,
        patterns: [AnyPattern; N],
    ) -> PatternEngineRunner<'m, N> {
        PatternEngineRunner {
            engine: self,
            motion,
            patterns,
            state: RunnerState::Idle,
        }
    }

    pub(crate) fn play(&self, index: usize) {
        let _ = self.channels.commands.try_send(EngineCommand::Play(index));
    }

    pub(crate) fn pause(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Pause);
    }

    pub(crate) fn resume(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Resume);
    }

    pub(crate) fn stop(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Stop);
        self.input.sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.velocity = 0.0;
            }
        });
    }

    pub(crate) fn home(&self) {
        let _ = self.channels.commands.try_send(EngineCommand::Home);
    }

    pub(crate) fn state(&self) -> EngineState {
        self.channels.state()
    }

    fn publish_state(&self, state: EngineState) {
        self.state_channel
            .immediate_publisher()
            .publish_immediate(state);
    }
}

impl Default for PatternEngine {
    fn default() -> Self {
        Self::new()
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

pub struct PatternEngineRunner<'m, const N: usize> {
    engine: &'m PatternEngine,
    motion: &'m MotionSender,
    patterns: [AnyPattern; N],
    state: RunnerState,
}

impl<'m, const N: usize> PatternEngineRunner<'m, N> {
    /// Run the engine forever, processing commands and driving patterns.
    ///
    /// `delay` must implement `Clone` so a fresh [`PatternCtx`] can be created
    /// each time a pattern starts. All embassy `Delay` types are `Copy`.
    pub async fn run<D: DelayNs + Clone>(&mut self, delay: D) -> ! {
        let motion = self.motion;
        let input = self.engine.input();

        loop {
            match self.state {
                RunnerState::Idle | RunnerState::Ready => {
                    let cmd = self.engine.channels.commands.receive().await;
                    self.handle_command(cmd).await;
                }
                RunnerState::Homing(maybe_idx) => {
                    if motion.enable().await != StateResponse::Completed {
                        log::error!("Enable failed, returning to idle");
                        self.set_state(RunnerState::Idle);
                        continue;
                    }

                    let home_fut = motion.home();
                    let mut home_fut = core::pin::pin!(home_fut);

                    loop {
                        let result = select::select(
                            home_fut.as_mut(),
                            self.engine.channels.commands.receive(),
                        )
                        .await;

                        match result {
                            Either::First(resp) => {
                                if resp != StateResponse::Completed {
                                    log::error!("Home failed, returning to idle");
                                    self.set_state(RunnerState::Idle);
                                } else {
                                    match maybe_idx {
                                        Some(idx) => self.set_state(RunnerState::Playing(idx)),
                                        None => self.set_state(RunnerState::Ready),
                                    }
                                }
                                break;
                            }
                            Either::Second(EngineCommand::Stop | EngineCommand::Pause) => {
                                if motion.disable().await == StateResponse::Fault {
                                    log::error!("Board fault during disable");
                                }
                                self.set_state(RunnerState::Idle);
                                break;
                            }
                            Either::Second(_) => {}
                        }
                    }
                }
                RunnerState::Playing(idx) => {
                    let mut ctx = PatternCtx::new(motion, input, delay.clone());

                    let engine = self.engine;
                    // Split borrows: the pinned future holds `patterns[idx]`,
                    // so we access `engine` and `state` through separate refs.
                    // This also means we cannot call `set_state()` here, so
                    // transitions publish state directly via
                    // `engine.publish_state()`.
                    let state = &mut self.state;
                    let pattern_fut = core::pin::pin!(self.patterns[idx].run(&mut ctx));
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
                                    engine.publish_state(EngineState::Idle);
                                }
                                break;
                            }
                            Either::Second(cmd) => match cmd {
                                EngineCommand::Pause => {
                                    if motion.pause().await != StateResponse::Completed {
                                        log::error!("Pause failed, stopping engine");
                                        *state = RunnerState::Idle;
                                        engine.channels.store(EngineState::Idle);
                                        engine.publish_state(EngineState::Idle);
                                        break;
                                    }
                                    engine.channels.store(EngineState::Paused(idx));
                                    engine.publish_state(EngineState::Paused(idx));
                                }
                                EngineCommand::Resume => {
                                    if motion.resume().await != StateResponse::Completed {
                                        log::error!("Resume failed, stopping engine");
                                        *state = RunnerState::Idle;
                                        engine.channels.store(EngineState::Idle);
                                        engine.publish_state(EngineState::Idle);
                                        break;
                                    }
                                    engine.channels.store(EngineState::Playing(idx));
                                    engine.publish_state(EngineState::Playing(idx));
                                }
                                EngineCommand::Play(i) if i == idx => {}
                                EngineCommand::Play(new_idx) if new_idx < N => {
                                    *state = RunnerState::Playing(new_idx);
                                    engine.channels.store(EngineState::Playing(new_idx));
                                    engine.publish_state(EngineState::Playing(new_idx));
                                    break;
                                }
                                EngineCommand::Stop => {
                                    if motion.disable().await == StateResponse::Fault {
                                        log::error!("Board fault during disable");
                                    }
                                    *state = RunnerState::Idle;
                                    engine.channels.store(EngineState::Idle);
                                    engine.publish_state(EngineState::Idle);
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
        let engine_state = state.as_engine_state();
        info!("Engine state: {:?}", engine_state);
        self.engine.channels.store(engine_state);
        self.engine.publish_state(engine_state);
    }

    async fn handle_command(&mut self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::Play(idx) => {
                if idx < N {
                    match self.state {
                        RunnerState::Idle => self.set_state(RunnerState::Homing(Some(idx))),
                        RunnerState::Homing(_) => {
                            log::warn!("Ignoring Play command while homing");
                        }
                        _ => self.set_state(RunnerState::Playing(idx)),
                    };
                }
            }
            EngineCommand::Stop => {
                if self.motion.disable().await == StateResponse::Fault {
                    log::error!("Board fault during disable");
                }
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
