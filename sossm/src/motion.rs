use rsruckig::prelude::*;

use crate::{MechanicalConfig, MotionLimits, Motor};

// Motor settings restored after homing. These configure the motor's internal
// closed-loop tracking - Ruckig controls actual machine speed by issuing
// position steps, so these are effectively "go as fast as commanded".
const OPERATING_SPEED_RPM: u16 = 3000;
const OPERATING_ACCELERATION: u16 = 50000;
const OPERATING_MAX_OUTPUT: u16 = 600;

// Floor applied to velocity requests to prevent degenerate Ruckig inputs.
const MIN_VELOCITY: f64 = 0.001;

pub struct MotionController<M: Motor> {
    motor: M,
    steps_per_mm: f64,
    min_position_mm: f64,
    max_position_mm: f64,
    limits: MotionLimits,
    ruckig: Ruckig<1, ThrowErrorHandler>,
    input: InputParameter<1>,
    output: OutputParameter<1>,
    move_in_progress: bool,
}

impl<M: Motor> MotionController<M> {
    /// Create a new `MotionController`.
    ///
    /// `update_interval_secs` must match how often `update()` will be called.
    /// Ruckig uses this as its fixed time step, so timing accuracy matters.
    pub fn new(
        motor: M,
        config: &MechanicalConfig,
        limits: MotionLimits,
        update_interval_secs: f64,
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
            steps_per_mm,
            min_position_mm: config.min_position_mm,
            max_position_mm: config.max_position_mm,
            limits,
            ruckig: Ruckig::<1, ThrowErrorHandler>::new(None, update_interval_secs),
            input,
            output: OutputParameter::new(None),
            move_in_progress: false,
        }
    }

    pub fn enable(&mut self) -> Result<(), M::Error> {
        self.motor.enable()
    }

    pub fn disable(&mut self) -> Result<(), M::Error> {
        self.motor.disable()
    }

    /// Home the motor, restore operating settings, and move to `min_position_mm`.
    /// Blocks until complete.
    pub fn home(&mut self) -> Result<(), M::Error> {
        self.motor.home()?;

        // motor.home() leaves speed at HOME_SPEED_RPM — restore for normal operation
        self.motor.set_speed(OPERATING_SPEED_RPM)?;
        self.motor.set_acceleration(OPERATING_ACCELERATION)?;
        self.motor.set_max_output(OPERATING_MAX_OUTPUT)?;

        let steps = (self.min_position_mm * self.steps_per_mm) as i32;
        self.motor.set_absolute_position(steps)?;

        // Sync Ruckig state to the new known position
        self.input.current_position[0] = self.min_position_mm;
        self.input.target_position[0] = self.min_position_mm;
        self.move_in_progress = false;

        Ok(())
    }

    /// Command a move to `mm` from the home position.
    /// The move is executed incrementally by subsequent `update()` calls.
    pub fn move_to(&mut self, mm: f64) {
        let mm = mm.clamp(self.min_position_mm, self.max_position_mm);
        if mm != self.input.target_position[0] {
            self.input.target_position[0] = mm;
            self.output.time = 0.0;
            self.move_in_progress = true;
        }
    }

    /// Set the maximum velocity for moves (mm/s), clamped to `MotionLimits`.
    pub fn set_speed(&mut self, mm_s: f64) {
        self.input.max_velocity[0] = mm_s.clamp(MIN_VELOCITY, self.limits.max_velocity_mm_s);
    }

    /// Advance the motion control loop by one step.
    ///
    /// Must be called at the same interval as `update_interval_secs` passed to `new()`.
    /// Computes the next Ruckig trajectory step and writes the target position to the motor.
    pub fn update(&mut self) -> Result<(), M::Error> {
        if !self.move_in_progress {
            return Ok(());
        }

        match self.ruckig.update(&self.input, &mut self.output) {
            Ok(RuckigResult::Working) => {
                let mm =
                    self.output.new_position[0].clamp(self.min_position_mm, self.max_position_mm);
                let steps = (mm * self.steps_per_mm) as i32;
                self.motor.set_absolute_position(steps)?;
                self.output.pass_to_input(&mut self.input);
            }
            Ok(RuckigResult::Finished) => {
                self.move_in_progress = false;
            }
            _ => {}
        }

        Ok(())
    }
}
