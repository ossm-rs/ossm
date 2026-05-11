use core::sync::atomic::Ordering;

use embassy_futures::select::{self, Either};
use embedded_hal_async::delay::DelayNs;
use log::info;
use ossm::{MotionSender, StateResponse};

use crate::AnyPattern;
use crate::engine::{EngineCommand, EngineState, PatternEngine};
use crate::pattern::{Pattern, PatternCtx};

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

/// Driver capability for the pattern engine.
///
/// Produced by [`PatternEngine::split`](crate::PatternEngine::split).
/// Consumed by [`run`](Self::run), which starts the engine's main
/// loop. The loop only returns if the host future is dropped (e.g. a
/// mode switch), at which point `PatternRunner` is gone for good.
/// Re-running requires constructing a fresh [`PatternEngine`] and
/// splitting again.
pub struct PatternRunner {
    engine: &'static PatternEngine,
}

impl PatternRunner {
    pub(crate) fn new(engine: &'static PatternEngine) -> Self {
        Self { engine }
    }

    /// Run the engine forever, processing commands and driving patterns.
    ///
    /// `motion` is borrowed for the lifetime of the run. `patterns` is
    /// moved in and lives on the runner's stack frame. `delay` must be
    /// `Clone` so a fresh [`PatternCtx`] can be created each time a
    /// pattern starts (all embassy `Delay` types are `Copy`).
    pub async fn run<const N: usize, D: DelayNs + Clone>(
        self,
        motion: &MotionSender,
        mut patterns: [AnyPattern; N],
        delay: D,
    ) -> ! {
        let engine = self.engine;
        let input = &engine.input;
        let mut state = RunnerState::Idle;

        loop {
            match state {
                RunnerState::Idle | RunnerState::Ready => {
                    let cmd = engine.commands.receive().await;
                    handle_command::<N>(engine, motion, cmd, &mut state).await;
                }
                RunnerState::Homing(maybe_idx) => {
                    if motion.enable().await != StateResponse::Completed {
                        log::error!("Enable failed, returning to idle");
                        set_state(engine, &mut state, RunnerState::Idle);
                        continue;
                    }

                    let home_fut = motion.home();
                    let mut home_fut = core::pin::pin!(home_fut);

                    loop {
                        let result =
                            select::select(home_fut.as_mut(), engine.commands.receive()).await;

                        match result {
                            Either::First(resp) => {
                                if resp != StateResponse::Completed {
                                    log::error!("Home failed, returning to idle");
                                    set_state(engine, &mut state, RunnerState::Idle);
                                } else {
                                    match maybe_idx {
                                        Some(idx) => {
                                            set_state(engine, &mut state, RunnerState::Playing(idx))
                                        }
                                        None => set_state(engine, &mut state, RunnerState::Ready),
                                    }
                                }
                                break;
                            }
                            Either::Second(EngineCommand::Stop | EngineCommand::Pause) => {
                                if motion.disable().await == StateResponse::Fault {
                                    log::error!("Board fault during disable");
                                }
                                set_state(engine, &mut state, RunnerState::Idle);
                                break;
                            }
                            Either::Second(_) => {}
                        }
                    }
                }
                RunnerState::Playing(idx) => {
                    let mut ctx = PatternCtx::new(motion, input, delay.clone());
                    let pattern_fut = core::pin::pin!(patterns[idx].run(&mut ctx));
                    let mut pattern_fut = pattern_fut;

                    loop {
                        let result =
                            select::select(pattern_fut.as_mut(), engine.commands.receive()).await;

                        match result {
                            Either::First(_result) => {
                                if matches!(state, RunnerState::Playing(_)) {
                                    state = RunnerState::Idle;
                                    store_and_publish(engine, EngineState::Idle);
                                }
                                break;
                            }
                            Either::Second(cmd) => match cmd {
                                EngineCommand::Pause => {
                                    if motion.pause().await != StateResponse::Completed {
                                        log::error!("Pause failed, stopping engine");
                                        state = RunnerState::Idle;
                                        store_and_publish(engine, EngineState::Idle);
                                        break;
                                    }
                                    store_and_publish(engine, EngineState::Paused(idx));
                                }
                                EngineCommand::Resume => {
                                    if motion.resume().await != StateResponse::Completed {
                                        log::error!("Resume failed, stopping engine");
                                        state = RunnerState::Idle;
                                        store_and_publish(engine, EngineState::Idle);
                                        break;
                                    }
                                    store_and_publish(engine, EngineState::Playing(idx));
                                }
                                EngineCommand::Play(i) if i == idx => {}
                                EngineCommand::Play(new_idx) if new_idx < N => {
                                    state = RunnerState::Playing(new_idx);
                                    store_and_publish(engine, EngineState::Playing(new_idx));
                                    break;
                                }
                                EngineCommand::Stop => {
                                    if motion.disable().await == StateResponse::Fault {
                                        log::error!("Board fault during disable");
                                    }
                                    state = RunnerState::Idle;
                                    store_and_publish(engine, EngineState::Idle);
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
}

fn set_state(engine: &PatternEngine, current: &mut RunnerState, new_state: RunnerState) {
    *current = new_state;
    let engine_state = new_state.as_engine_state();
    info!("Engine state: {:?}", engine_state);
    store_and_publish(engine, engine_state);
}

fn store_and_publish(engine: &PatternEngine, state: EngineState) {
    engine.state.store(state.encode(), Ordering::Relaxed);
    engine
        .state_channel
        .immediate_publisher()
        .publish_immediate(state);
}

async fn handle_command<const N: usize>(
    engine: &PatternEngine,
    motion: &MotionSender,
    cmd: EngineCommand,
    state: &mut RunnerState,
) {
    match cmd {
        EngineCommand::Play(idx) if idx < N => match *state {
            RunnerState::Idle => set_state(engine, state, RunnerState::Homing(Some(idx))),
            RunnerState::Homing(_) => log::warn!("Ignoring Play command while homing"),
            _ => set_state(engine, state, RunnerState::Playing(idx)),
        },
        EngineCommand::Play(_) => {}
        EngineCommand::Stop => {
            if motion.disable().await == StateResponse::Fault {
                log::error!("Board fault during disable");
            }
            set_state(engine, state, RunnerState::Idle);
        }
        EngineCommand::Home => {
            if let RunnerState::Idle = *state {
                set_state(engine, state, RunnerState::Homing(None));
            }
        }
        EngineCommand::Pause | EngineCommand::Resume => {
            // Only handled inside the Playing inner loop.
        }
    }
}
