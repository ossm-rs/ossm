use core::cell::Cell;
use core::sync::atomic::{AtomicI32, Ordering};

use embassy_time::{Delay, Duration, Ticker};
extern crate alloc;
use alloc::string::String;

use pattern_engine::{
    AnyPattern, EngineCommand, EngineCommandChannel, Pattern, PatternEngine, PatternInput,
    SharedPatternInput,
};
use sim_motor::SimMotor;
use ossm::{MechanicalConfig, Motor, MotionLimits, Ossm, OssmChannels};
use wasm_bindgen::prelude::*;

static CHANNELS: OssmChannels = OssmChannels::new();
static ENGINE_COMMANDS: EngineCommandChannel = EngineCommandChannel::new();
static PATTERN_INPUT: SharedPatternInput =
    SharedPatternInput::new(Cell::new(PatternInput::DEFAULT));
static MOTOR_POSITION: AtomicI32 = AtomicI32::new(0);

const CONFIG: MechanicalConfig = MechanicalConfig {
    pulley_teeth: 20,
    belt_pitch_mm: 2.0,
    min_position_mm: 10.0,
    max_position_mm: 250.0,
};

#[wasm_bindgen]
pub struct Simulator {
    steps_per_mm: f64,
    min_position_mm: f64,
    max_position_mm: f64,
}

#[wasm_bindgen]
impl Simulator {
    /// Create a new simulator and start the motion + pattern tasks.
    ///
    /// `update_interval_ms` controls the motion controller tick rate (e.g. 10.0 for 10ms).
    #[wasm_bindgen(constructor)]
    pub fn new(update_interval_ms: f64) -> Self {
        let update_interval_secs = update_interval_ms / 1000.0;
        let motor = SimMotor::new(&MOTOR_POSITION);

        let (ossm, mut controller) = Ossm::new(
            motor,
            &CONFIG,
            MotionLimits::default(),
            update_interval_secs,
            &CHANNELS,
        );

        let interval_us = (update_interval_secs * 1_000_000.0) as u64;

        wasm_bindgen_futures::spawn_local(async move {
            let mut ticker = Ticker::every(Duration::from_micros(interval_us));
            loop {
                controller.update().await;
                ticker.next().await;
            }
        });

        wasm_bindgen_futures::spawn_local(async move {
            ossm.enable();
            ossm.home().await;

            let mut engine = PatternEngine::new(AnyPattern::all_builtin());
            engine
                .run(&ENGINE_COMMANDS, &CHANNELS, &PATTERN_INPUT, Delay)
                .await;
        });

        let steps_per_mm = CONFIG.steps_per_mm(SimMotor::STEPS_PER_REV) as f64;

        Self {
            steps_per_mm,
            min_position_mm: CONFIG.min_position_mm,
            max_position_mm: CONFIG.max_position_mm,
        }
    }

    /// Current position as a fraction of the machine range (0.0–1.0).
    pub fn get_position(&self) -> f64 {
        let steps = MOTOR_POSITION.load(Ordering::Relaxed);
        let mm = steps as f64 / self.steps_per_mm;
        let range = self.max_position_mm - self.min_position_mm;
        (mm - self.min_position_mm) / range
    }

    /// Set the maximum depth as a fraction of the machine range (0.0–1.0).
    pub fn set_depth(&self, depth: f64) {
        PATTERN_INPUT.lock(|cell| {
            let mut input = cell.get();
            input.depth = depth;
            cell.set(input);
        });
    }

    /// Set the stroke length as a fraction of the machine range (0.0–1.0).
    pub fn set_stroke(&self, stroke: f64) {
        PATTERN_INPUT.lock(|cell| {
            let mut input = cell.get();
            input.stroke = stroke;
            cell.set(input);
        });
    }

    /// Set velocity as a fraction of max velocity (0.0–1.0).
    pub fn set_velocity(&self, velocity: f64) {
        PATTERN_INPUT.lock(|cell| {
            let mut input = cell.get();
            input.velocity = velocity;
            cell.set(input);
        });
    }

    /// Set sensation value (-1.0 to 1.0). Meaning is pattern-specific.
    pub fn set_sensation(&self, sensation: f64) {
        PATTERN_INPUT.lock(|cell| {
            let mut input = cell.get();
            input.sensation = sensation;
            cell.set(input);
        });
    }

    pub fn play(&self, index: usize) {
        let _ = ENGINE_COMMANDS.try_send(EngineCommand::Play(index));
    }

    pub fn pause(&self) {
        let _ = ENGINE_COMMANDS.try_send(EngineCommand::Pause);
    }

    pub fn resume(&self) {
        let _ = ENGINE_COMMANDS.try_send(EngineCommand::Resume);
    }

    pub fn stop(&self) {
        let _ = ENGINE_COMMANDS.try_send(EngineCommand::Stop);
    }

    pub fn pattern_count(&self) -> usize {
        AnyPattern::all_builtin().len()
    }

    pub fn pattern_name(&self, index: usize) -> String {
        let patterns = AnyPattern::all_builtin();
        patterns
            .get(index)
            .map(|p| String::from(p.name()))
            .unwrap_or_default()
    }

    pub fn pattern_description(&self, index: usize) -> String {
        let patterns = AnyPattern::all_builtin();
        patterns
            .get(index)
            .map(|p| String::from(p.description()))
            .unwrap_or_default()
    }
}
