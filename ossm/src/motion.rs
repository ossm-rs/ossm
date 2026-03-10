use rsruckig::prelude::*;

use crate::command::{Cancelled, MoveCommand, OssmChannels, StateCommand, StateResponse};
use crate::{Board, MotionLimits};

// Floor applied to velocity requests to prevent degenerate Ruckig inputs.
const MIN_VELOCITY: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MotionState {
    Disabled,
    Enabled,
    Ready,
    Moving,
    /// Ruckig is decelerating to a smooth stop for the given reason.
    Stopping(StopReason),
    /// Motor is stationary; the instructed target is preserved for resume.
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum StopReason {
    Pause,
    Disable,
    Home,
}

/// The last-commanded motion intent, independent of what ruckig is currently
/// planning. Pause/resume manipulates the ruckig input while leaving this
/// untouched.
#[derive(Debug, Clone, Copy)]
struct MotionTarget {
    /// Target position (mm).
    position: f64,
    /// Maximum velocity (mm/s).
    velocity: f64,
    /// Torque limit as a fraction (0.0–1.0). `None` uses the motor default.
    torque: Option<f64>,
}

/// Drives the motion state machine and enforces safe motion profiles.
///
/// The controller owns a ruckig instance and generates jerk-limited
/// trajectories. Each tick, it samples the trajectory and calls
/// `board.set_position(mm)` with the next point on the curve. The board
/// is a dumb position follower — it never plans its own trajectory.
///
/// # Safety
///
/// Ruckig enforces the acceleration and jerk limits from [`MotionLimits`].
/// No upstream code (patterns, UI, remote) can cause motion that exceeds
/// these limits. The motor's internal trajectory planner is bypassed by
/// configuring it for maximum tracking speed.
pub struct MotionController<'a, B: Board> {
    board: B,
    channels: &'a OssmChannels,
    state: MotionState,
    limits: MotionLimits,
    /// The last-instructed motion target. `Some` when a move has been commanded,
    /// `None` when there is no active motion intent (e.g. disabled, just homed).
    target: Option<MotionTarget>,
    /// Default velocity used by `MoveTo` commands (mm/s).
    default_speed: f64,
    /// Default torque used by `MoveTo` commands.
    default_torque: Option<f64>,
    ruckig: Ruckig<1, ThrowErrorHandler>,
    input: InputParameter<1>,
    output: OutputParameter<1>,
}

impl<'a, B: Board> MotionController<'a, B> {
    /// Create a new `MotionController` in the `Disabled` state.
    ///
    /// `update_interval_secs` must match the ticker period the caller uses.
    /// Ruckig uses this as its fixed time step, so timing accuracy matters.
    pub(crate) fn new(
        board: B,
        limits: MotionLimits,
        update_interval_secs: f64,
        channels: &'a OssmChannels,
    ) -> Self {
        let mut input = InputParameter::new(None);
        input.current_position[0] = limits.min_position_mm;
        input.target_position[0] = limits.min_position_mm;
        input.max_velocity[0] = MIN_VELOCITY;
        input.max_acceleration[0] = limits.max_acceleration_mm_s2;
        input.max_jerk[0] = limits.max_jerk_mm_s3;
        input.synchronization = Synchronization::None;
        input.duration_discretization = DurationDiscretization::Discrete;

        Self {
            board,
            channels,
            state: MotionState::Disabled,
            limits,
            target: None,
            default_speed: MIN_VELOCITY,
            default_torque: None,
            ruckig: Ruckig::<1, ThrowErrorHandler>::new(None, update_interval_secs),
            input,
            output: OutputParameter::new(None),
        }
    }

    /// Advance the motion control loop by one step.
    ///
    /// Ticks the board, then the ruckig trajectory, then processes commands.
    /// State commands are processed before move commands so that `SetSpeed`
    /// applies before a `MoveTo` arriving in the same tick.
    pub async fn update(&mut self) {
        // 1. Let the board do periodic housekeeping (fault polling etc.)
        let _ = self.board.tick().await;

        // 2. Advance the trajectory and send position to the board.
        self.tick().await;

        // 3. Process commands: state before move.
        if let Ok(cmd) = self.channels.state_cmd.try_receive() {
            self.process_state_command(cmd).await;
        }

        if let Ok(cmd) = self.channels.move_cmd.try_receive() {
            self.process_move_command(cmd).await;
        }
    }

    async fn process_state_command(&mut self, cmd: StateCommand) {
        match (&self.state, cmd) {
            (MotionState::Disabled, StateCommand::Enable) => {
                let _ = self.board.enable().await;
                self.state = MotionState::Enabled;
                self.respond(StateResponse::Completed);
            }

            (MotionState::Enabled | MotionState::Ready, StateCommand::Disable) => {
                self.disable().await;
                self.respond(StateResponse::Completed);
            }
            (MotionState::Paused, StateCommand::Disable) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.disable().await;
                self.respond(StateResponse::Completed);
            }
            (MotionState::Moving, StateCommand::Disable) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.stop(StopReason::Disable);
            }
            (MotionState::Stopping(_), StateCommand::Disable) => {
                self.state = MotionState::Stopping(StopReason::Disable);
            }

            (MotionState::Enabled | MotionState::Ready, StateCommand::Home) => {
                self.home().await;
                self.respond(StateResponse::Completed);
            }
            (MotionState::Moving, StateCommand::Home) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.stop(StopReason::Home);
            }
            (MotionState::Paused, StateCommand::Home) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.home().await;
                self.respond(StateResponse::Completed);
            }

            (MotionState::Moving, StateCommand::Pause) => {
                self.stop(StopReason::Pause);
                self.respond(StateResponse::Completed);
            }

            (MotionState::Paused, StateCommand::Resume) => {
                self.resume().await;
                self.respond(StateResponse::Completed);
            }

            (_, StateCommand::SetSpeed(fraction)) => {
                let vel = self.fraction_to_velocity(fraction);
                self.default_speed = vel;
                if matches!(self.state, MotionState::Moving) {
                    if let Some(target) = &mut self.target {
                        target.velocity = vel;
                        self.sync_ruckig();
                    }
                }
                self.respond(StateResponse::Completed);
            }

            (_, StateCommand::SetTorque(fraction)) => {
                self.default_torque = Some(fraction);
                if matches!(self.state, MotionState::Moving) {
                    if let Some(target) = &mut self.target {
                        target.torque = Some(fraction);
                    }
                    self.apply_torque().await;
                }
                self.respond(StateResponse::Completed);
            }

            _ => {
                self.respond(StateResponse::InvalidTransition);
            }
        }
    }

    async fn process_move_command(&mut self, cmd: MoveCommand) {
        match (&self.state, cmd) {
            (MotionState::Ready, MoveCommand::MoveTo(fraction)) => {
                let mm = self.fraction_to_mm(fraction);
                self.set_target(|t| t.position = mm);
                self.apply_torque().await;
                self.state = MotionState::Moving;
            }
            (MotionState::Ready, MoveCommand::Motion(cmd)) => {
                self.set_motion_target(cmd);
                self.apply_torque().await;
                self.state = MotionState::Moving;
            }

            (MotionState::Moving, MoveCommand::MoveTo(fraction)) => {
                let mm = self.fraction_to_mm(fraction);
                self.set_target(|t| t.position = mm);
                self.apply_torque().await;
            }
            (MotionState::Moving, MoveCommand::Motion(cmd)) => {
                self.set_motion_target(cmd);
                self.apply_torque().await;
            }

            _ => {}
        }
    }

    /// Sample the ruckig trajectory and send the position to the board.
    async fn tick(&mut self) {
        if !matches!(self.state, MotionState::Moving | MotionState::Stopping(_)) {
            return;
        }

        let Ok(result) = self.ruckig.update(&self.input, &mut self.output) else {
            return;
        };

        if !matches!(result, RuckigResult::Working | RuckigResult::Finished) {
            return;
        }

        let mm = self.output.new_position[0]
            .clamp(self.limits.min_position_mm, self.limits.max_position_mm);
        let _ = self.board.set_position(mm).await;
        self.output.pass_to_input(&mut self.input);

        if result == RuckigResult::Finished {
            match self.state {
                MotionState::Stopping(StopReason::Pause) => {
                    self.state = MotionState::Paused;
                }
                MotionState::Stopping(StopReason::Disable) => {
                    self.disable().await;
                    self.respond(StateResponse::Completed);
                }
                MotionState::Stopping(StopReason::Home) => {
                    self.home().await;
                    self.respond(StateResponse::Completed);
                }
                _ => {
                    self.target = None;
                    self.state = MotionState::Ready;
                    self.channels.move_resp.signal(Ok(()));
                }
            }
        }
    }

    async fn home(&mut self) {
        self.state = MotionState::Disabled;
        let _ = self.board.home().await;

        // Reset ruckig state to the home position.
        self.input.control_interface = ControlInterface::Position;
        self.input.current_position[0] = self.limits.min_position_mm;
        self.input.target_position[0] = self.limits.min_position_mm;
        self.input.current_velocity[0] = 0.0;
        self.input.current_acceleration[0] = 0.0;

        // Send the board to the min position (home).
        let _ = self.board.set_position(self.limits.min_position_mm).await;

        self.target = None;
        self.state = MotionState::Ready;
    }

    async fn disable(&mut self) {
        let _ = self.board.disable().await;
        self.input.control_interface = ControlInterface::Position;
        self.target = None;
        self.state = MotionState::Disabled;
    }

    fn stop(&mut self, reason: StopReason) {
        // Switch to velocity control and target zero velocity. Ruckig handles
        // the jerk-limited deceleration trajectory — no manual math needed.
        self.input.control_interface = ControlInterface::Velocity;
        self.input.target_velocity[0] = 0.0;
        self.output.time = 0.0;
        self.state = MotionState::Stopping(reason);
    }

    async fn resume(&mut self) {
        // Switch back to position control and restore the instructed target.
        self.input.control_interface = ControlInterface::Position;
        self.sync_ruckig();
        self.apply_torque().await;
        self.state = MotionState::Moving;
    }

    fn respond(&self, resp: StateResponse) {
        self.channels.state_resp.signal(resp);
    }

    fn fraction_to_mm(&self, fraction: f64) -> f64 {
        let mm = self.limits.min_position_mm
            + fraction * (self.limits.max_position_mm - self.limits.min_position_mm);
        mm.clamp(self.limits.min_position_mm, self.limits.max_position_mm)
    }

    fn fraction_to_velocity(&self, fraction: f64) -> f64 {
        let mm_s = fraction * self.limits.max_velocity_mm_s;
        mm_s.clamp(MIN_VELOCITY, self.limits.max_velocity_mm_s)
    }

    fn set_target(&mut self, f: impl FnOnce(&mut MotionTarget)) {
        let target = self.target.get_or_insert(MotionTarget {
            position: self.input.current_position[0],
            velocity: self.default_speed,
            torque: self.default_torque,
        });
        f(target);
        self.sync_ruckig();
    }

    fn set_motion_target(&mut self, cmd: crate::command::MotionCommand) {
        self.target = Some(MotionTarget {
            position: self.fraction_to_mm(cmd.position),
            velocity: self.fraction_to_velocity(cmd.speed),
            torque: cmd.torque,
        });
        self.sync_ruckig();
    }

    /// Write the instructed target into ruckig's input parameters and reset
    /// the trajectory timer so ruckig replans.
    fn sync_ruckig(&mut self) {
        if let Some(target) = &self.target {
            self.input.target_position[0] = target.position;
            self.input.max_velocity[0] = target.velocity;
            self.output.time = 0.0;
        }
    }

    /// Apply the current target's torque limit to the board.
    async fn apply_torque(&mut self) {
        let fraction = self.target.as_ref().and_then(|t| t.torque).unwrap_or(1.0);
        let _ = self.board.set_torque(fraction).await;
    }
}
