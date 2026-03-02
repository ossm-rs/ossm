use core::cell::Cell;
use core::sync::atomic::{AtomicI32, Ordering};

use embassy_time::{Delay, Duration, Ticker};
use pattern_engine::patterns::Deeper;
use pattern_engine::{patterns::Simple, Pattern, PatternCtx, PatternInput, SharedPatternInput};
use sim_motor::SimMotor;
use sossm::{
    CommandChannel, HomingSignal, MechanicalConfig, Motor, MotionLimits, MoveCompleteSignal, Sossm,
};
use wasm_bindgen::prelude::*;

static COMMANDS: CommandChannel = CommandChannel::new();
static HOMING_DONE: HomingSignal = HomingSignal::new();
static MOVE_COMPLETE: MoveCompleteSignal = MoveCompleteSignal::new();
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

        let (sossm, mut controller) = Sossm::new(
            motor,
            &CONFIG,
            MotionLimits::default(),
            update_interval_secs,
            &COMMANDS,
            &HOMING_DONE,
            &MOVE_COMPLETE,
        );

        let interval_us = (update_interval_secs * 1_000_000.0) as u64;

        // Spawn the motion controller loop
        wasm_bindgen_futures::spawn_local(async move {
            let mut ticker = Ticker::every(Duration::from_micros(interval_us));
            loop {
                controller.update().await;
                ticker.next().await;
            }
        });

        // Spawn the lifecycle + pattern loop
        wasm_bindgen_futures::spawn_local(async move {
            sossm.enable();
            sossm.home().await;

            let mut ctx = PatternCtx::new(&COMMANDS, &MOVE_COMPLETE, &PATTERN_INPUT, Delay);
            let mut pattern = Deeper;
            pattern.run(&mut ctx).await;
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
        ((mm - self.min_position_mm) / range).clamp(0.0, 1.0)
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

    /// Set sensation value (-100.0 to 100.0). Meaning is pattern-specific.
    pub fn set_sensation(&self, sensation: f64) {
        PATTERN_INPUT.lock(|cell| {
            let mut input = cell.get();
            input.sensation = sensation;
            cell.set(input);
        });
    }
}
