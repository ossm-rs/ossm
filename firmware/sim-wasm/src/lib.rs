use core::sync::atomic::{AtomicI32, Ordering};

use embassy_time::{Delay, Duration, Ticker};
extern crate alloc;
use alloc::string::String;

use ossm::{MechanicalConfig, MotionLimits, Ossm};
use pattern_engine::{AnyPattern, Pattern, PatternEngine};
use sim_board::SimBoard;
use sim_motor::SimMotor;
use wasm_bindgen::prelude::*;

static OSSM: Ossm = Ossm::new();
static PATTERNS: PatternEngine = PatternEngine::new(&OSSM);
static MOTOR_POSITION: AtomicI32 = AtomicI32::new(0);

static MECHANICAL: MechanicalConfig = MechanicalConfig {
    pulley_teeth: 20,
    belt_pitch_mm: 2.0,
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
        ossm::logging::init(log::LevelFilter::Info, |line| {
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(line));
        });

        let update_interval_secs = update_interval_ms / 1000.0;
        let motor = SimMotor::new(&MOTOR_POSITION);
        let board = SimBoard::new(motor, &MECHANICAL);

        let limits = MotionLimits {
            min_position_mm: 10.0,
            max_position_mm: 250.0,
            ..MotionLimits::default()
        };

        let mut controller = OSSM.controller(board, limits.clone(), update_interval_secs);

        let interval_us = (update_interval_secs * 1_000_000.0) as u64;

        wasm_bindgen_futures::spawn_local(async move {
            let mut ticker = Ticker::every(Duration::from_micros(interval_us));
            loop {
                if let Err(e) = controller.update().await {
                    log::error!("Motion controller fault: {:?}", e);
                }
                ticker.next().await;
            }
        });

        let mut pattern_runner = PATTERNS.runner(AnyPattern::all_builtin());

        wasm_bindgen_futures::spawn_local(async move {
            pattern_runner.run(Delay).await;
        });

        let steps_per_mm = MECHANICAL.steps_per_mm(SimMotor::STEPS_PER_REV) as f64;

        Self {
            steps_per_mm,
            min_position_mm: limits.min_position_mm,
            max_position_mm: limits.max_position_mm,
        }
    }

    /// Engine state: 0 = idle, 1 = homing, 2 = playing, 3 = paused.
    pub fn get_engine_state(&self) -> u8 {
        PATTERNS.state().as_u8()
    }

    /// Current position as a fraction of the machine range (0.0-1.0).
    pub fn get_position(&self) -> f64 {
        let steps = MOTOR_POSITION.load(Ordering::Relaxed);
        let mm = steps as f64 / self.steps_per_mm;
        let range = self.max_position_mm - self.min_position_mm;
        (mm - self.min_position_mm) / range
    }

    /// Set the maximum depth as a fraction of the machine range (0.0-1.0).
    pub fn set_depth(&self, depth: f64) {
        PATTERNS.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.depth = depth;
            }
        });
    }

    /// Set the stroke length as a fraction of the machine range (0.0-1.0).
    pub fn set_stroke(&self, stroke: f64) {
        PATTERNS.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.stroke = stroke;
            }
        });
    }

    /// Set velocity as a fraction of max velocity (0.0-1.0).
    pub fn set_velocity(&self, velocity: f64) {
        PATTERNS.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.velocity = velocity;
            }
        });
    }

    /// Set sensation value (-1.0 to 1.0). Meaning is pattern-specific.
    pub fn set_sensation(&self, sensation: f64) {
        PATTERNS.input().sender().send_modify(|opt| {
            if let Some(input) = opt {
                input.sensation = sensation;
            }
        });
    }

    pub fn play(&self, index: usize) {
        PATTERNS.play(index);
    }

    pub fn pause(&self) {
        PATTERNS.pause();
    }

    pub fn resume(&self) {
        PATTERNS.resume();
    }

    pub fn stop(&self) {
        PATTERNS.stop();
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
