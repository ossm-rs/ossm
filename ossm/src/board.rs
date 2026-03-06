use core::fmt::Debug;

use crate::Motor;

/// Abstraction over a board + motor combination.
///
/// A `Board` exposes the same control surface as [`Motor`], but allows
/// board-level logic (e.g. current-sensing homing) to intercept or augment
/// individual operations.
///
/// Every [`Motor`] automatically implements `Board` via a blanket impl,
/// so existing code that passes a bare motor continues to work unchanged.
#[allow(async_fn_in_trait)]
pub trait Board {
    type Error: Debug;

    const STEPS_PER_REV: u32;

    async fn enable(&mut self) -> Result<(), Self::Error>;
    async fn disable(&mut self) -> Result<(), Self::Error>;

    /// Run the full homing sequence. Boards may override this to coordinate
    /// additional hardware (e.g. a current-sensing IC).
    async fn home(&mut self) -> Result<(), Self::Error>;

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error>;

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error>;
    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error>;
    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error>;
}

impl<M: Motor> Board for M {
    type Error = M::Error;

    const STEPS_PER_REV: u32 = M::STEPS_PER_REV;

    async fn enable(&mut self) -> Result<(), Self::Error> {
        Motor::enable(self).await
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        Motor::disable(self).await
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        Motor::home(self).await
    }

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        Motor::set_absolute_position(self, steps).await
    }

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error> {
        Motor::set_speed(self, rpm).await
    }

    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error> {
        Motor::set_acceleration(self, value).await
    }

    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error> {
        Motor::set_max_output(self, output).await
    }
}
