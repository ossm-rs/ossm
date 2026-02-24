use crate::{MechanicalConfig, Motor};

/// A hardware factory trait. Implementors initialize board peripherals and
/// hand a ready-to-use motor to `Sossm`. The board struct itself is consumed
/// by `into_motor` - it exists only to configure hardware, not to persist.
///
/// Board-specific extras (buttons, displays, etc.) live on the concrete board
/// struct and are accessed before calling `into_motor`.
pub trait Board {
    type Motor: Motor;

    fn mechanical_config(&self) -> &MechanicalConfig;

    /// Consume the board and return the initialized motor.
    fn into_motor(self) -> Self::Motor;
}
