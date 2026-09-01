use crate::motor::{Motor, StepDir as StepDirTrait};
use core::fmt::Debug;
use embedded_hal::digital::OutputPin;

/// Abstracts step-pulse generation.
///
/// The firmware layer provides a concrete implementation (e.g. MCPWM on ESP32).
/// The driver only cares that `count` pulses are produced - timing, duty cycle,
/// and hardware details live behind this trait.
#[allow(async_fn_in_trait)]
pub trait StepOutput {
    type Error: Debug;

    /// Generate `count` step pulses. Returns when all pulses have been emitted.
    async fn step(&mut self, count: u32) -> Result<(), Self::Error>;
}

/// Reports the current step position.
///
/// Software implementations rely on [`applied`](Self::applied) being called
/// after every step batch. Hardware implementations (e.g. ESP32 PCNT) observe
/// pulses directly and treat `applied` as a no-op.
pub trait PositionFeedback {
    /// Current absolute position in steps.
    fn position(&self) -> i32;

    /// Establish a new absolute position. Called after homing.
    fn reset(&mut self, value: i32);

    /// Record that `delta` steps were just emitted. Signed so that direction
    /// reversals decrement the counter. Hardware-counted implementations
    /// leave this as a no-op.
    fn applied(&mut self, delta: i32);
}

/// In-memory position counter. The default for boards that do not have a
/// hardware pulse counter; trusts every commanded step to land.
#[derive(Debug, Default)]
pub struct SoftwarePositionCounter {
    position: i32,
}

impl SoftwarePositionCounter {
    pub const fn new() -> Self {
        Self { position: 0 }
    }
}

impl PositionFeedback for SoftwarePositionCounter {
    fn position(&self) -> i32 {
        self.position
    }

    fn reset(&mut self, value: i32) {
        self.position = value;
    }

    fn applied(&mut self, delta: i32) {
        self.position = self.position.wrapping_add(delta);
    }
}

pub struct StepDirConfig {
    pub steps_per_rev: u32,
    /// Maximum output value for the Motor trait. Step/dir drivers handle
    /// current limiting in hardware, so this is largely informational.
    pub max_output: u16,
}

impl Default for StepDirConfig {
    fn default() -> Self {
        Self {
            steps_per_rev: 800,
            max_output: 1000,
        }
    }
}

#[derive(Debug)]
pub enum StepDirError<S: Debug, P: Debug> {
    Step(S),
    Pin(P),
}

pub struct StepDirMotor<S: StepOutput, D: OutputPin, E: OutputPin, F: PositionFeedback> {
    step: S,
    dir: D,
    enable: E,
    feedback: F,
    config: StepDirConfig,
}

impl<S: StepOutput, D: OutputPin, E: OutputPin, F: PositionFeedback> StepDirMotor<S, D, E, F> {
    pub fn new(step: S, dir: D, enable: E, feedback: F, config: StepDirConfig) -> Self {
        Self {
            step,
            dir,
            enable,
            feedback,
            config,
        }
    }
}

impl<S, D, E, F> Motor for StepDirMotor<S, D, E, F>
where
    S: StepOutput,
    D: OutputPin,
    E: OutputPin<Error = D::Error>,
    F: PositionFeedback,
{
    type Error = StepDirError<S::Error, D::Error>;

    fn steps_per_rev(&self) -> u32 {
        self.config.steps_per_rev
    }

    fn max_output(&self) -> u16 {
        self.config.max_output
    }

    async fn enable(&mut self) -> Result<(), Self::Error> {
        // ENA is active-low on stock OSSM hardware
        self.enable.set_low().map_err(StepDirError::Pin)?;
        Ok(())
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.enable.set_high().map_err(StepDirError::Pin)?;
        Ok(())
    }

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        let delta = steps - self.feedback.position();
        if delta == 0 {
            return Ok(());
        }

        if delta > 0 {
            self.dir.set_high().map_err(StepDirError::Pin)?;
        } else {
            self.dir.set_low().map_err(StepDirError::Pin)?;
        }

        self.step
            .step(delta.unsigned_abs())
            .await
            .map_err(StepDirError::Step)?;

        self.feedback.applied(delta);
        Ok(())
    }

    async fn read_absolute_position(&mut self) -> Result<i32, Self::Error> {
        Ok(self.feedback.position())
    }

    async fn set_max_output(&mut self, _output: u16) -> Result<(), Self::Error> {
        // Step/dir drivers handle current limiting in hardware.
        Ok(())
    }
}

impl<S, D, E, F> StepDirTrait for StepDirMotor<S, D, E, F>
where
    S: StepOutput,
    D: OutputPin,
    E: OutputPin<Error = D::Error>,
    F: PositionFeedback,
{
    fn reset_position(&mut self, position: i32) {
        self.feedback.reset(position);
    }
}
