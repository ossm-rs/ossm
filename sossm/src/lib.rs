#![no_std]
extern crate alloc;

mod command;
mod limits;
mod mechanical;
mod motion;
mod motor;

pub use command::{Command, CommandChannel};
pub use limits::MotionLimits;
pub use mechanical::MechanicalConfig;
pub use motion::MotionController;
pub use motor::{Motor, MotorTelemetry};

/// Lightweight command handle for application code.
///
/// `Sossm` sends commands to a [`MotionController`] via a shared channel.
/// All methods take `&self` and are safe to call from any context — no mutex
/// or critical section needed.
///
/// Create both halves with [`Sossm::new()`], then hand the
/// [`MotionController`] to an interrupt or timer task.
pub struct Sossm<'a> {
    commands: &'a CommandChannel,
    update_interval_secs: f64,
}

impl<'a> Sossm<'a> {
    /// Create a `Sossm` command handle and a [`MotionController`] engine,
    /// both connected to the given `commands` channel.
    ///
    /// The returned `MotionController` should be placed in a timer interrupt
    /// or periodic task that calls [`MotionController::update()`].
    pub fn new<M: Motor>(
        motor: M,
        config: &MechanicalConfig,
        limits: MotionLimits,
        update_interval_secs: f64,
        commands: &'a CommandChannel,
    ) -> (Self, MotionController<'a, M>) {
        let controller =
            MotionController::new(motor, config, limits, update_interval_secs, commands);
        let handle = Self {
            commands,
            update_interval_secs,
        };
        (handle, controller)
    }

    pub fn update_interval_secs(&self) -> f64 {
        self.update_interval_secs
    }

    pub fn enable(&self) {
        let _ = self.commands.try_send(Command::Enable);
    }

    pub fn disable(&self) {
        let _ = self.commands.try_send(Command::Disable);
    }

    pub fn move_to(&self, mm: f64) {
        let _ = self.commands.try_send(Command::MoveTo(mm));
    }

    pub fn set_speed(&self, mm_s: f64) {
        let _ = self.commands.try_send(Command::SetSpeed(mm_s));
    }
}
