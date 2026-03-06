use rsruckig::prelude::*;

use crate::command::{Command, OssmChannels};
use crate::{Board, MechanicalConfig, MotionLimits};

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

pub struct MotionController<'a, B: Board> {
    board: B,
    channels: &'a OssmChannels,
    state: MotionState,
    steps_per_mm: f64,
    min_position_mm: f64,
    max_position_mm: f64,
    limits: MotionLimits,
    /// The last-instructed motion target. `Some` when a move has been commanded,
    /// `None` when there is no active motion intent (e.g. disabled, just homed).
    target: Option<MotionTarget>,
    ruckig: Ruckig<1, ThrowErrorHandler>,
    input: InputParameter<1>,
    output: OutputParameter<1>,
}

impl<'a, B: Board> MotionController<'a, B> {
    /// Create a new `MotionController` in the `Disabled` state.
    ///
    /// `update_interval_secs` must match the ticker period the caller uses.
    /// Ruckig uses this as its fixed time step, so timing accuracy matters.
    pub fn new(
        board: B,
        config: &MechanicalConfig,
        limits: MotionLimits,
        update_interval_secs: f64,
        channels: &'a OssmChannels,
    ) -> Self {
        let steps_per_mm = config.steps_per_mm(B::STEPS_PER_REV) as f64;

        let mut input = InputParameter::new(None);
        input.current_position[0] = config.min_position_mm;
        input.target_position[0] = config.min_position_mm;
        input.max_velocity[0] = MIN_VELOCITY;
        input.max_acceleration[0] = limits.max_acceleration_mm_s2;
        input.max_jerk[0] = limits.max_jerk_mm_s3;
        input.synchronization = Synchronization::None;
        input.duration_discretization = DurationDiscretization::Discrete;

        Self {
            board,
            channels,
            state: MotionState::Disabled,
            steps_per_mm,
            min_position_mm: config.min_position_mm,
            max_position_mm: config.max_position_mm,
            limits,
            target: None,
            ruckig: Ruckig::<1, ThrowErrorHandler>::new(None, update_interval_secs),
            input,
            output: OutputParameter::new(None),
        }
    }

    /// Advance the motion control loop by one step.
    ///
    /// Ticks the state machine then processes one pending command. May await
    /// internally (e.g. homing blocks until the motor completes), so the
    /// caller should reset its ticker afterward to avoid catch-up bursts.
    pub async fn update(&mut self) {
        self.tick().await;

        if let Ok(cmd) = self.channels.commands.try_receive() {
            self.process_command(cmd).await;
        }
    }

    async fn process_command(&mut self, cmd: Command) {
        match (&self.state, cmd) {
            (MotionState::Disabled, Command::Enable) => {
                let _ = self.board.enable().await;
                self.state = MotionState::Enabled;
            }

            (MotionState::Enabled, Command::Home) => {
                self.home().await;
            }
            (MotionState::Enabled, Command::Disable) => {
                self.disable().await;
            }

            (MotionState::Ready, Command::MoveTo(fraction)) => {
                let mm = self.fraction_to_mm(fraction);
                self.set_target(|t| t.position = mm);
                self.state = MotionState::Moving;
            }
            (MotionState::Ready, Command::Motion(cmd)) => {
                self.set_motion_target(cmd);
                self.state = MotionState::Moving;
            }
            (MotionState::Ready, Command::SetSpeed(fraction)) => {
                let vel = self.fraction_to_velocity(fraction);
                self.set_target(|t| t.velocity = vel);
            }
            (MotionState::Ready, Command::Home) => {
                self.home().await;
            }
            (MotionState::Ready, Command::Disable) => {
                self.disable().await;
            }

            (MotionState::Moving, Command::MoveTo(fraction)) => {
                let mm = self.fraction_to_mm(fraction);
                self.set_target(|t| t.position = mm);
            }
            (MotionState::Moving, Command::Motion(cmd)) => {
                self.set_motion_target(cmd);
            }
            (MotionState::Moving, Command::SetSpeed(fraction)) => {
                let vel = self.fraction_to_velocity(fraction);
                self.set_target(|t| t.velocity = vel);
            }
            (MotionState::Moving, Command::Home) => {
                self.stop(StopReason::Home);
            }
            (MotionState::Moving, Command::Pause) => {
                self.stop(StopReason::Pause);
            }
            (MotionState::Moving, Command::Disable) => {
                self.stop(StopReason::Disable);
            }

            (MotionState::Paused, Command::Resume) => {
                self.resume();
            }
            (MotionState::Paused, Command::Home) => {
                self.home().await;
            }
            (MotionState::Paused, Command::Disable) => {
                self.disable().await;
            }

            (MotionState::Stopping(_), Command::Disable) => {
                self.state = MotionState::Stopping(StopReason::Disable);
            }

            _ => {}
        }
    }

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
            .clamp(self.min_position_mm, self.max_position_mm);
        let steps = (mm * self.steps_per_mm) as i32;
        let _ = self.board.set_absolute_position(steps).await;
        self.output.pass_to_input(&mut self.input);

        if result == RuckigResult::Finished {
            match self.state {
                MotionState::Stopping(StopReason::Pause) => {
                    self.state = MotionState::Paused;
                }
                MotionState::Stopping(StopReason::Disable) => {
                    self.disable().await;
                }
                MotionState::Stopping(StopReason::Home) => {
                    self.home().await;
                }
                _ => {
                    self.target = None;
                    self.state = MotionState::Ready;
                    self.channels.move_complete.signal(());
                }
            }
        }
    }

    async fn home(&mut self) {
        self.state = MotionState::Disabled;
        let _ = self.board.home().await;

        let steps = (self.min_position_mm * self.steps_per_mm) as i32;
        let _ = self.board.set_absolute_position(steps).await;

        self.input.control_interface = ControlInterface::Position;
        self.input.current_position[0] = self.min_position_mm;
        self.input.target_position[0] = self.min_position_mm;
        self.input.current_velocity[0] = 0.0;
        self.input.current_acceleration[0] = 0.0;

        self.target = None;

        self.channels.homing_done.signal(());
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

    fn resume(&mut self) {
        // Switch back to position control and restore the instructed target.
        self.input.control_interface = ControlInterface::Position;
        self.sync_ruckig();
        self.state = MotionState::Moving;
    }

    fn fraction_to_mm(&self, fraction: f64) -> f64 {
        let mm = self.min_position_mm + fraction * (self.max_position_mm - self.min_position_mm);
        mm.clamp(self.min_position_mm, self.max_position_mm)
    }

    fn fraction_to_velocity(&self, fraction: f64) -> f64 {
        let mm_s = fraction * self.limits.max_velocity_mm_s;
        mm_s.clamp(MIN_VELOCITY, self.limits.max_velocity_mm_s)
    }

    fn set_target(&mut self, f: impl FnOnce(&mut MotionTarget)) {
        let target = self.target.get_or_insert(MotionTarget {
            position: self.input.current_position[0],
            velocity: MIN_VELOCITY,
            torque: None,
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
}
