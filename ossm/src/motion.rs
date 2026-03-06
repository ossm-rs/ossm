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
}

pub struct MotionController<'a, B: Board> {
    board: B,
    channels: &'a OssmChannels,
    state: MotionState,
    steps_per_mm: f64,
    min_position_mm: f64,
    max_position_mm: f64,
    limits: MotionLimits,
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
                let _ = self.board.disable().await;
                self.state = MotionState::Disabled;
            }

            (MotionState::Ready, Command::MoveTo(fraction)) => {
                self.begin_move(self.fraction_to_mm(fraction));
                self.state = MotionState::Moving;
            }
            (MotionState::Ready, Command::Motion(cmd)) => {
                self.apply_motion(cmd);
                self.state = MotionState::Moving;
            }
            (MotionState::Ready, Command::SetSpeed(fraction)) => {
                self.set_speed(fraction * self.limits.max_velocity_mm_s);
            }
            (MotionState::Ready, Command::Home) => {
                self.home().await;
            }
            (MotionState::Ready, Command::Disable) => {
                let _ = self.board.disable().await;
                self.state = MotionState::Disabled;
            }

            (MotionState::Moving, Command::MoveTo(fraction)) => {
                self.begin_move(self.fraction_to_mm(fraction));
            }
            (MotionState::Moving, Command::Motion(cmd)) => {
                self.apply_motion(cmd);
            }
            (MotionState::Moving, Command::SetSpeed(fraction)) => {
                self.set_speed(fraction * self.limits.max_velocity_mm_s);
            }
            (MotionState::Moving, Command::Home) => {
                self.home().await;
            }
            (MotionState::Moving, Command::Disable) => {
                let _ = self.board.disable().await;
                self.state = MotionState::Disabled;
            }

            _ => {}
        }
    }

    async fn tick(&mut self) {
        if self.state == MotionState::Moving {
            match self.ruckig.update(&self.input, &mut self.output) {
                Ok(result @ RuckigResult::Working) | Ok(result @ RuckigResult::Finished) => {
                    let mm = self.output.new_position[0]
                        .clamp(self.min_position_mm, self.max_position_mm);
                    let steps = (mm * self.steps_per_mm) as i32;
                    let _ = self.board.set_absolute_position(steps).await;
                    self.output.pass_to_input(&mut self.input);

                    if result == RuckigResult::Finished {
                        self.state = MotionState::Ready;
                        self.channels.move_complete.signal(());
                    }
                }
                _ => {}
            }
        }
    }

    async fn home(&mut self) {
        self.state = MotionState::Disabled;
        let _ = self.board.home().await;

        let steps = (self.min_position_mm * self.steps_per_mm) as i32;
        let _ = self.board.set_absolute_position(steps).await;

        self.input.current_position[0] = self.min_position_mm;
        self.input.target_position[0] = self.min_position_mm;
        self.input.current_velocity[0] = 0.0;
        self.input.current_acceleration[0] = 0.0;

        self.channels.homing_done.signal(());
        self.state = MotionState::Ready;
    }

    fn apply_motion(&mut self, cmd: crate::command::MotionCommand) {
        self.set_speed(cmd.speed * self.limits.max_velocity_mm_s);
        self.begin_move(self.fraction_to_mm(cmd.position));
    }

    fn fraction_to_mm(&self, fraction: f64) -> f64 {
        self.min_position_mm + fraction * (self.max_position_mm - self.min_position_mm)
    }

    fn begin_move(&mut self, mm: f64) {
        let mm = mm.clamp(self.min_position_mm, self.max_position_mm);
        if mm != self.input.target_position[0] {
            self.input.target_position[0] = mm;
            self.output.time = 0.0;
        }
    }

    fn set_speed(&mut self, mm_s: f64) {
        self.input.max_velocity[0] = mm_s.clamp(MIN_VELOCITY, self.limits.max_velocity_mm_s);
    }
}
