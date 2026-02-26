use rsruckig::prelude::*;

use crate::command::{Command, CommandChannel, HomingSignal};
use crate::{MechanicalConfig, MotionLimits, Motor, Sleep};

// Motor settings restored after homing. These configure the motor's internal
// closed-loop tracking - Ruckig controls actual machine speed by issuing
// position steps, so these are effectively "go as fast as commanded".
const OPERATING_SPEED_RPM: u16 = 3000;
const OPERATING_ACCELERATION: u16 = 50000;
const OPERATING_MAX_OUTPUT: u16 = 600;

// Floor applied to velocity requests to prevent degenerate Ruckig inputs.
const MIN_VELOCITY: f64 = 0.001;

// Settle time after re-enabling Modbus post-homing. The M57AIM resets
// speed/output defaults when Modbus is toggled and needs time to stabilise.
const POST_HOMING_SETTLE_MS: u32 = 800;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MotionState {
    Disabled,
    Enabled,
    Homing,
    Ready,
    Moving,
}

pub struct MotionController<'a, M: Motor, S: Sleep> {
    motor: M,
    sleep: S,
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

impl<'a, M: Motor, S: Sleep> MotionController<'a, M, S> {
    /// Create a new `MotionController` in the `Disabled` state.
    ///
    /// `update_interval_secs` must match the ticker period the caller uses.
    /// Ruckig uses this as its fixed time step, so timing accuracy matters.
    pub fn new(
        motor: M,
        sleep: S,
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
            sleep,
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
    /// internally (e.g. post-homing settle), so the caller should reset its
    /// ticker afterward to avoid catch-up bursts.
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
                let _ = self.motor.enable();
                self.state = MotionState::Enabled;
            }

            // Enabled + Home → Homing
            (MotionState::Enabled, Command::Home) => {
                let _ = self.motor.start_home();
                self.state = MotionState::Homing;
            }
            // Enabled + Disable → Disabled
            (MotionState::Enabled, Command::Disable) => {
                let _ = self.motor.disable();
                self.state = MotionState::Disabled;
            }

            // Homing + Disable → Disabled
            (MotionState::Homing, Command::Disable) => {
                let _ = self.motor.disable();
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
            // Ready + Home → Homing (re-home)
            (MotionState::Ready, Command::Home) => {
                let _ = self.motor.start_home();
                self.state = MotionState::Homing;
            }
            // Ready + Disable → Disabled
            (MotionState::Ready, Command::Disable) => {
                let _ = self.motor.disable();
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
            // Moving + Home → Homing
            (MotionState::Moving, Command::Home) => {
                let _ = self.motor.start_home();
                self.state = MotionState::Homing;
            }
            // Moving + Disable → Disabled
            (MotionState::Moving, Command::Disable) => {
                let _ = self.motor.disable();
                self.state = MotionState::Disabled;
            }

            // All other combinations are ignored
            _ => {}
        }
    }

    async fn tick(&mut self) {
        match self.state {
            MotionState::Homing => {
                if self.motor.is_home_complete().unwrap_or(false) {
                    self.finish_homing().await;
                }
            }
            MotionState::Moving => match self.ruckig.update(&self.input, &mut self.output) {
                Ok(RuckigResult::Working) => {
                    let mm = self.output.new_position[0]
                        .clamp(self.min_position_mm, self.max_position_mm);
                    let steps = (mm * self.steps_per_mm) as i32;
                    let _ = self.motor.set_absolute_position(steps);
                    self.output.pass_to_input(&mut self.input);
                }
                Ok(RuckigResult::Finished) => {
                    self.state = MotionState::Ready;
                }
                _ => {}
            },
            _ => {}
        }
    }

    async fn finish_homing(&mut self) {
        // Re-enable modbus — M57AIM homing resets it
        let _ = self.motor.enable();

        // Modbus re-enable resets speed/output defaults — let it settle
        self.sleep.sleep_ms(POST_HOMING_SETTLE_MS).await;

        // Restore operating settings
        let _ = self.motor.set_speed(OPERATING_SPEED_RPM);
        let _ = self.motor.set_acceleration(OPERATING_ACCELERATION);
        let _ = self.motor.set_max_output(OPERATING_MAX_OUTPUT);

        // Move to min offset
        let steps = (self.min_position_mm * self.steps_per_mm) as i32;
        let _ = self.motor.set_absolute_position(steps);

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
