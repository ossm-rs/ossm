#![no_std]
extern crate alloc;

mod board;
mod limits;
mod mechanical;
mod motion;
mod motor;

pub use board::Board;
pub use limits::MotionLimits;
pub use mechanical::MechanicalConfig;
pub use motion::MotionController;
pub use motor::{Motor, MotorTelemetry};

pub struct Sossm<M: Motor> {
    motion: MotionController<M>,
}

impl<M: Motor> Sossm<M> {
    pub fn new(
        motor: M,
        config: &MechanicalConfig,
        limits: MotionLimits,
        update_interval_secs: f64,
    ) -> Self {
        Self {
            motion: MotionController::new(motor, config, limits, update_interval_secs),
        }
    }

    pub fn enable(&mut self) -> Result<(), M::Error> {
        self.motion.enable()
    }

    pub fn disable(&mut self) -> Result<(), M::Error> {
        self.motion.disable()
    }

    pub fn home(&mut self) -> Result<(), M::Error> {
        self.motion.home()
    }

    pub fn move_to(&mut self, mm: f64) {
        self.motion.move_to(mm)
    }

    pub fn set_speed(&mut self, mm_s: f64) {
        self.motion.set_speed(mm_s)
    }

    pub fn update(&mut self) -> Result<(), M::Error> {
        self.motion.update()
    }
}
