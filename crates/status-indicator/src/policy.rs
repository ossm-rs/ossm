//! Steady status policy, independent of indicator hardware and scheduling.

use crate::{ColorIndicator, Rgb};
use ossm::MotionPhase;
use pattern_engine::EngineState;

pub const POLL_INTERVAL_MS: u64 = 50;
pub use crate::MAX_BRIGHTNESS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Homing,
    Stopping,
    Playing,
    Paused,
    Ready,
}

/// Cap brightness while preserving each palette color's channel proportions.
pub fn color(status: Status) -> Rgb {
    let rgb = match status {
        Status::Idle => Rgb::DIM_WHITE,
        Status::Homing => Rgb::YELLOW,
        Status::Stopping => Rgb::ORANGE,
        Status::Playing => Rgb::GREEN,
        Status::Paused => Rgb::BLUE,
        Status::Ready => Rgb::GREEN,
    };
    let peak = rgb.red.max(rgb.green).max(rgb.blue);
    // Keep the cap configurable, including its current full-intensity setting.
    #[allow(clippy::absurd_extreme_comparisons)]
    if peak <= MAX_BRIGHTNESS {
        return rgb;
    }
    let scale = |channel: u8| (channel as u16 * MAX_BRIGHTNESS as u16 / peak as u16) as u8;
    Rgb::new(scale(rgb.red), scale(rgb.green), scale(rgb.blue))
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
    applied: Option<Rgb>,
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

    pub async fn apply(&mut self, status: Status) -> Result<(), I::Error> {
        let desired = color(status);
        if self.applied == Some(desired) {
            return Ok(());
        }
        // Invalidate before awaiting: a failed or cancelled write may have
        // changed physical output, even if the next request is the old color.
        self.applied = None;
        self.indicator.set_color(desired).await?;
        if !self.on {
            self.indicator.set_on(true).await?;
            self.on = true;
        }
        self.applied = Some(desired);
        Ok(())
    }
}
