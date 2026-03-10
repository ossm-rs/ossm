#![no_std]

mod rs485;
mod transport;

pub use rs485::Rs485;
pub use transport::Rs485ModbusTransport;

use ossm::{Board, MechanicalConfig, Rs485 as Rs485Motor, SelfHoming};

#[derive(Debug)]
pub enum BoardError<E: core::fmt::Debug> {
    Motor(E),
}

/// OSSM Alt board, generic over any [`Rs485`](ossm::Rs485) + [`SelfHoming`] motor.
///
/// This board is a **position follower**. The motion controller calls
/// `set_position(mm)` every tick with the next point on the ruckig
/// trajectory. The board converts mm to steps and sends the command
/// to the motor.
pub struct OssmAlt<M: Rs485Motor + SelfHoming> {
    motor: M,
    mechanical: &'static MechanicalConfig,
}

impl<M: Rs485Motor + SelfHoming> OssmAlt<M> {
    pub fn new(motor: M, mechanical: &'static MechanicalConfig) -> Self {
        Self { motor, mechanical }
    }
}

impl<M: Rs485Motor + SelfHoming> Board for OssmAlt<M> {
    type Error = BoardError<M::Error>;

    async fn enable(&mut self) -> Result<(), Self::Error> {
        self.motor.enable().await.map_err(BoardError::Motor)
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.motor.disable().await.map_err(BoardError::Motor)
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        self.motor.home().await.map_err(BoardError::Motor)
    }

    async fn set_position(&mut self, position_mm: f64) -> Result<(), Self::Error> {
        let steps = self
            .mechanical
            .mm_to_steps(position_mm, self.motor.steps_per_rev());
        self.motor
            .set_absolute_position(steps)
            .await
            .map_err(BoardError::Motor)
    }

    async fn set_torque(&mut self, fraction: f64) -> Result<(), Self::Error> {
        let output = (fraction.clamp(0.0, 1.0) * self.motor.max_output() as f64) as u16;
        self.motor
            .set_max_output(output)
            .await
            .map_err(BoardError::Motor)
    }

    async fn position_mm(&mut self) -> Result<f64, Self::Error> {
        let steps = self
            .motor
            .read_absolute_position()
            .await
            .map_err(BoardError::Motor)?;
        Ok(self
            .mechanical
            .steps_to_mm(steps, self.motor.steps_per_rev()))
    }

    async fn tick(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
