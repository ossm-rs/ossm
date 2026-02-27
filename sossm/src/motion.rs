use rsruckig::prelude::*;

use crate::command::{Command, CommandChannel, HomingSignal};
use crate::{MechanicalConfig, MotionLimits, Motor};

// Floor applied to velocity requests to prevent degenerate Ruckig inputs.
const MIN_VELOCITY: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MotionState {
    Disabled,
    Enabled,
    Ready,
    Moving,
}

pub struct MotionController<'a, M: Motor> {
    motor: M,
    commands: &'a CommandChannel,
    homing_done: &'a HomingSignal,
    state: MotionState,
    steps_per_mm: f64,
    min_position_mm: f64,
    max_position_mm: f64,
    limits: MotionLimits,
    ruckig: Ruckig<1, ThrowErrorHandler>,
    input: InputParameter<1>,
    output: OutputParameter<1>,
}

impl<'a, M: Motor> MotionController<'a, M> {
    /// Create a new `MotionController` in the `Disabled` state.
    ///
    /// `update_interval_secs` must match the ticker period the caller uses.
    /// Ruckig uses this as its fixed time step, so timing accuracy matters.
    pub fn new(
        motor: M,
        config: &MechanicalConfig,
        limits: MotionLimits,
        update_interval_secs: f64,
        commands: &'a CommandChannel,
        homing_done: &'a HomingSignal,
    ) -> Self {
        let steps_per_mm = config.steps_per_mm(M::STEPS_PER_REV) as f64;

        let mut input = InputParameter::new(None);
        input.current_position[0] = config.min_position_mm;
        input.target_position[0] = config.min_position_mm;
        input.max_velocity[0] = MIN_VELOCITY;
        input.max_acceleration[0] = limits.max_acceleration_mm_s2;
        input.max_jerk[0] = limits.max_jerk_mm_s3;
        input.synchronization = Synchronization::None;
        input.duration_discretization = DurationDiscretization::Discrete;

        Self {
            motor,
            commands,
            homing_done,
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

        if let Ok(cmd) = self.commands.try_receive() {
            self.process_command(cmd).await;
        }
    }

    async fn process_command(&mut self, cmd: Command) {
        match (&self.state, cmd) {
            // Disabled + Enable → Enabled
            (MotionState::Disabled, Command::Enable) => {
                let _ = self.motor.enable().await;
                self.state = MotionState::Enabled;
            }

            // Enabled + Home → home then Ready
            (MotionState::Enabled, Command::Home) => {
                self.do_home().await;
            }
            // Enabled + Disable → Disabled
            (MotionState::Enabled, Command::Disable) => {
                let _ = self.motor.disable().await;
                self.state = MotionState::Disabled;
            }

            // Ready + MoveTo → Moving
            (MotionState::Ready, Command::MoveTo(mm)) => {
                self.begin_move(mm);
                self.state = MotionState::Moving;
            }
            // Ready + SetSpeed
            (MotionState::Ready, Command::SetSpeed(mm_s)) => {
                self.set_speed(mm_s);
            }
            // Ready + Home → re-home
            (MotionState::Ready, Command::Home) => {
                self.do_home().await;
            }
            // Ready + Disable → Disabled
            (MotionState::Ready, Command::Disable) => {
                let _ = self.motor.disable().await;
                self.state = MotionState::Disabled;
            }

            // Moving + MoveTo → retarget (stay Moving)
            (MotionState::Moving, Command::MoveTo(mm)) => {
                self.begin_move(mm);
            }
            // Moving + SetSpeed
            (MotionState::Moving, Command::SetSpeed(mm_s)) => {
                self.set_speed(mm_s);
            }
            // Moving + Home → home
            (MotionState::Moving, Command::Home) => {
                self.do_home().await;
            }
            // Moving + Disable → Disabled
            (MotionState::Moving, Command::Disable) => {
                let _ = self.motor.disable().await;
                self.state = MotionState::Disabled;
            }

            // All other combinations are ignored
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
                    let _ = self.motor.set_absolute_position(steps).await;
                    self.output.pass_to_input(&mut self.input);

                    if result == RuckigResult::Finished {
                        self.state = MotionState::Ready;
                    }
                }
                _ => {}
            }
        }
    }

    async fn do_home(&mut self) {
        // Motor owns the entire homing sequence (trigger, poll, settle, restore)
        let _ = self.motor.home().await;

        // Move to min offset
        let steps = (self.min_position_mm * self.steps_per_mm) as i32;
        let _ = self.motor.set_absolute_position(steps).await;

        // Sync Ruckig state
        self.input.current_position[0] = self.min_position_mm;
        self.input.target_position[0] = self.min_position_mm;

        self.homing_done.signal(());
        self.state = MotionState::Ready;
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
