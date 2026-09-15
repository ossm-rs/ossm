//! Steady status policy, independent of indicator hardware and scheduling.

use crate::{ColorIndicator, RGB8};
use ossm::MotionPhase;
use pattern_engine::EngineState;
use smart_leds::brightness;

pub const POLL_INTERVAL_MS: u64 = 50;
pub use crate::MAX_BRIGHTNESS;

pub const BLUE: RGB8 = RGB8::new(0, 0, 255);
pub const DIM_WHITE: RGB8 = RGB8::new(10, 10, 10);
pub const GREEN: RGB8 = RGB8::new(0, 255, 0);
pub const ORANGE: RGB8 = RGB8::new(255, 80, 0);
pub const PURPLE: RGB8 = RGB8::new(128, 0, 255);
pub const YELLOW: RGB8 = RGB8::new(255, 255, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Booting,
    Idle,
    Homing,
    Stopping,
    Playing,
    Paused,
    Ready,
}

/// Apply the shared brightness level without changing the palette's hues.
pub fn color(status: Status) -> RGB8 {
    let rgb = match status {
        Status::Booting => DIM_WHITE,
        Status::Idle => BLUE,
        Status::Homing => PURPLE,
        Status::Stopping => ORANGE,
        Status::Playing | Status::Ready => GREEN,
        Status::Paused => YELLOW,
    };
    brightness([rgb].into_iter(), MAX_BRIGHTNESS)
        .next()
        .unwrap()
}

/// Resolve independently sampled observers in priority order. Engine playing
/// covers pattern delays and zero-speed holds even when motion is stationary.
pub fn select(engine: EngineState, motion: MotionPhase) -> Status {
    match (engine, motion) {
        (EngineState::Homing, _) => Status::Homing,
        (_, MotionPhase::Disabled | MotionPhase::Enabled) => Status::Idle,
        (_, MotionPhase::Stopping) => Status::Stopping,
        (_, MotionPhase::Moving) | (EngineState::Playing(_), _) => Status::Playing,
        (EngineState::Paused(_), _) | (_, MotionPhase::Paused) => Status::Paused,
        (EngineState::Ready, _) => Status::Ready,
        _ => Status::Idle,
    }
}

/// Applies changed colors and explicitly turns on an initially-off indicator.
/// The caller schedules retries: errors never mark output as applied.
pub struct Output<I> {
    indicator: I,
    applied: Option<RGB8>,
    on: bool,
}

impl<I: ColorIndicator> Output<I> {
    pub fn new(indicator: I) -> Self {
        Self {
            indicator,
            applied: None,
            on: false,
        }
    }

    pub fn apply(&mut self, status: Status) -> Result<(), I::Error> {
        let desired = color(status);
        if self.applied == Some(desired) {
            return Ok(());
        }
        // A failed write may change physical output, even if the next request
        // returns to the previous color.
        self.applied = None;
        self.indicator.set_color(desired)?;
        if !self.on {
            self.indicator.set_on(true)?;
            self.on = true;
        }
        self.applied = Some(desired);
        Ok(())
    }
}
