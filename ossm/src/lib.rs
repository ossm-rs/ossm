#![no_std]
extern crate alloc;

mod board;
mod command;
mod limits;
mod mechanical;
mod motion;
mod motor;
pub mod stepdir;

pub use board::Board;
pub use command::{Cancelled, MotionCommand, StateCommand, StateResponse};
use command::{MoveCommand, OssmChannels};
pub use limits::MotionLimits;
pub use mechanical::MechanicalConfig;
pub use motion::MotionController;
pub use motor::{Motor, Rs485, SelfHoming, StepDir};
pub use stepdir::{StepDirConfig, StepDirError, StepDirMotor, StepOutput};

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

    /// Drain any pending move command, then send the new one and await completion.
    async fn send_move(&self, cmd: MoveCommand) -> Result<(), Cancelled> {
        self.channels.move_resp.reset();
        let _ = self.channels.move_cmd.try_receive();
        let _ = self.channels.move_cmd.try_send(cmd);
        self.channels.move_resp.wait().await
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

    pub async fn set_speed(&self, speed: f64) -> StateResponse {
        self.send_state(StateCommand::SetSpeed(speed)).await
    }

    pub async fn set_torque(&self, torque: f64) -> StateResponse {
        self.send_state(StateCommand::SetTorque(torque)).await
    }

    pub async fn move_to(&self, position: f64) -> Result<(), Cancelled> {
        self.send_move(MoveCommand::MoveTo(position)).await
    }

    pub async fn push_motion(&self, cmd: MotionCommand) -> Result<(), Cancelled> {
        self.send_move(MoveCommand::Motion(cmd)).await
    }
}
