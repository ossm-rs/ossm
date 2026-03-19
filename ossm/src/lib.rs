#![no_std]
extern crate alloc;

mod board;
mod command;
mod limits;
mod mechanical;
mod motion;
mod motor;
pub mod transport;

pub use board::Board;
pub use command::{Cancelled, MotionCommand, StateCommand, StateResponse};
use command::OssmChannels;
pub use limits::MotionLimits;
pub use mechanical::MechanicalConfig;
pub use motion::MotionController;
pub use motor::{Motor, Rs485Motor, SelfHoming, StepDir};
pub use transport::{
    Modbus, ModbusTransport, Rs485, Rs485ModbusTransport, StepDirConfig, StepDirError,
    StepDirMotor, StepOutput, TransportError,
};

pub struct Ossm {
    channels: OssmChannels,
}

impl Ossm {
    pub const fn new() -> Self {
        Self {
            channels: OssmChannels::new(),
        }
    }

    /// Create a motion controller bound to this Ossm instance.
    ///
    /// The board is a position follower — it doesn't need to know about
    /// mechanical config or motion limits. Those are the controller's concern.
    pub fn controller<B: Board>(
        &'static self,
        board: B,
        limits: MotionLimits,
        update_interval_secs: f64,
    ) -> MotionController<'static, B> {
        MotionController::new(board, limits, update_interval_secs, &self.channels)
    }

    /// Send a state command and wait for the motion controller to respond.
    async fn send_state(&self, cmd: StateCommand) -> StateResponse {
        self.channels.state_resp.reset();
        self.channels.state_cmd.send(cmd).await;
        self.channels.state_resp.wait().await
    }

    pub async fn enable(&self) -> StateResponse {
        self.send_state(StateCommand::Enable).await
    }

    pub async fn disable(&self) -> StateResponse {
        self.send_state(StateCommand::Disable).await
    }

    pub async fn home(&self) -> StateResponse {
        self.send_state(StateCommand::Home).await
    }

    pub async fn pause(&self) -> StateResponse {
        self.send_state(StateCommand::Pause).await
    }

    pub async fn resume(&self) -> StateResponse {
        self.send_state(StateCommand::Resume).await
    }

    /// Start a motion without waiting for completion.
    ///
    /// Resets the move response signal, so a subsequent [`await_motion`](Self::await_motion)
    /// will wait for this move to finish.
    pub fn begin_motion(&self, cmd: MotionCommand) {
        self.channels.move_resp.reset();
        let _ = self.channels.move_cmd.try_receive();
        let _ = self.channels.move_cmd.try_send(cmd);
    }

    /// Update the target of an in-flight motion without resetting the completion signal.
    pub fn update_motion(&self, cmd: MotionCommand) {
        let _ = self.channels.move_cmd.try_receive();
        let _ = self.channels.move_cmd.try_send(cmd);
    }

    /// Wait for the current in-flight motion to complete.
    pub async fn await_motion(&self) -> Result<(), Cancelled> {
        self.channels.move_resp.wait().await
    }
}
