extern crate alloc;
use alloc::string::String;

use embassy_time::{Delay, Duration, Ticker};
use ossm::{MechanicalConfig, MotionLimits, MotionObserver, MotionChannels};
use pattern_engine::{
    AnyPattern, PatternEngine, PatternEvent, PatternObserver, PatternSender, PatternUpdate,
    pattern_description, pattern_list,
};
use sim_board::SimBoard;
use sim_motor::SimMotor;
use static_cell::StaticCell;
use wasm_bindgen::prelude::*;

static OSSM_CELL: StaticCell<MotionChannels> = StaticCell::new();
static PATTERNS_CELL: StaticCell<PatternEngine> = StaticCell::new();

static MECHANICAL: MechanicalConfig = MechanicalConfig {
    pulley_teeth: 20,
    belt_pitch_mm: 2.0,
    reverse_direction: false,
};

#[wasm_bindgen]
pub struct Simulator {
    motion_observer: MotionObserver,
    pattern_observer: PatternObserver,
    patterns: PatternSender,
}

#[wasm_bindgen]
impl Simulator {
    #[wasm_bindgen(constructor)]
    pub fn new(update_interval_ms: f64) -> Self {
        ossm::logging::init(log::LevelFilter::Info, |line| {
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(line));
        });

        ossm::build_info!();

        let (receiver, motion_observer, motion) = OSSM_CELL.init(MotionChannels::new()).split();
        let (runner, pattern_observer, patterns) =
            PATTERNS_CELL.init(PatternEngine::new()).split();

        let update_interval_secs = update_interval_ms / 1000.0;
        let motor = SimMotor::new();
        let board = SimBoard::new(motor, &MECHANICAL);

        let limits = MotionLimits {
            min_position_mm: 10.0,
            max_position_mm: 250.0,
            ..MotionLimits::default()
        };

        let mut controller = receiver.into_controller(board, limits, update_interval_secs);

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

        wasm_bindgen_futures::spawn_local(async move {
            runner.run(&motion, AnyPattern::all_builtin(), Delay).await
        });

        Self {
            motion_observer,
            pattern_observer,
            patterns,
        }
    }

    pub fn get_engine_state(&self) -> u8 {
        self.pattern_observer.phase().as_u8()
    }

    pub fn get_position(&self) -> f32 {
        self.motion_observer.state().position
    }

    pub fn set_depth(&self, depth: f64) {
        self.patterns.update(PatternUpdate::Depth(depth));
    }

    pub fn set_stroke(&self, stroke: f64) {
        self.patterns.update(PatternUpdate::Stroke(stroke));
    }

    pub fn set_velocity(&self, velocity: f64) {
        self.patterns.update(PatternUpdate::Speed(velocity));
    }

    pub fn set_sensation(&self, sensation: f64) {
        self.patterns.update(PatternUpdate::Sensation(sensation));
    }

    pub fn play(&self, index: usize) {
        self.patterns.send(PatternEvent::Play(index));
    }

    pub fn pause(&self) {
        self.patterns.send(PatternEvent::Pause);
    }

    pub fn resume(&self) {
        self.patterns.send(PatternEvent::Resume);
    }

    pub fn stop(&self) {
        self.patterns.send(PatternEvent::Stop);
    }

    pub fn pattern_count(&self) -> usize {
        pattern_list().len()
    }

    pub fn pattern_name(&self, index: usize) -> String {
        pattern_list()
            .get(index)
            .map(|p| String::from(p.name))
            .unwrap_or_default()
    }

    pub fn pattern_description(&self, index: usize) -> String {
        pattern_description(index)
            .map(String::from)
            .unwrap_or_default()
    }
}
