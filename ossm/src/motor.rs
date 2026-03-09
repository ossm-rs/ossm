/// A motor that a board can drive as a position follower.
///
/// Covers the basics: position control, enable/disable, torque.
/// Communication interface traits ([`StepDir`], [`Rs485`]) extend this
/// with interface-specific capabilities.
#[allow(async_fn_in_trait)]
pub trait Motor {
    type Error: core::fmt::Debug;

    /// Steps per revolution, used by the board to convert mm to steps.
    fn steps_per_rev(&self) -> u32;

    /// Maximum output value for torque scaling.
    fn max_output(&self) -> u16;

    /// Enable the motor and prepare it for position following.
    async fn enable(&mut self) -> Result<(), Self::Error>;

    /// Disable the motor.
    async fn disable(&mut self) -> Result<(), Self::Error>;

    /// Command an absolute position in steps.
    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error>;

    /// Read the current absolute position in steps.
    async fn read_absolute_position(&mut self) -> Result<i32, Self::Error>;

    /// Set the maximum torque output (raw motor units).
    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error>;
}

/// A motor driven via step/direction GPIO pulses.
///
/// Position is tracked in software, so the board must call
/// [`reset_position`](StepDir::reset_position) after homing to
/// synchronise the counter.
#[allow(async_fn_in_trait)]
pub trait StepDir: Motor {
    /// Reset the internal position counter (e.g. after board-managed homing).
    fn reset_position(&mut self, position: i32);
}

/// A motor controlled over RS-485 (Modbus).
#[allow(async_fn_in_trait)]
pub trait Rs485: Motor {}

/// A motor that can home itself without board-level sensors.
///
/// Motors with built-in homing firmware (e.g. the 57AIM's Modbus homing
/// command) implement this. The board delegates homing entirely to the motor.
#[allow(async_fn_in_trait)]
pub trait SelfHoming: Motor {
    /// Run the full homing sequence. Returns when the motor is homed
    /// and ready for position following.
    async fn home(&mut self) -> Result<(), Self::Error>;
}
