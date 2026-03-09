#![no_std]

use core::fmt::Debug;
use embedded_hal_async::delay::DelayNs;
use ossm::{Board, MechanicalConfig, StepDir};

/// Abstracts ADC reading for current sensing.
///
/// The firmware layer provides a concrete implementation using the
/// platform's ADC peripheral (e.g. `esp-hal` ADC on GPIO36).
#[allow(async_fn_in_trait)]
pub trait CurrentSensor {
    type Error: Debug;

    /// Read the current sensor as a fraction (0.0–1.0), averaged over
    /// `samples` readings.
    ///
    /// The reference hardware uses a hall-effect sensor on the motor supply
    /// line. The value maps to the ADC's full range, not directly to amps.
    fn read_fraction(&mut self, samples: u32) -> Result<f32, Self::Error>;
}

pub struct HomingConfig {
    /// Current reading (fraction of full scale) above baseline that
    /// indicates a stall. Reference hardware uses 0.06.
    pub stall_threshold: f32,
    /// ADC samples for baseline calibration (at rest). Reference: 1000.
    pub baseline_samples: u32,
    /// ADC samples per current check during homing. Reference: 50.
    pub poll_samples: u32,
    /// Homing speed in steps/s (reference: 25mm worth of steps/s).
    pub speed_steps_s: u32,
    /// Distance to back away from stall point in mm.
    pub backoff_mm: f64,
    /// Maximum stroke length in mm (limits search distance).
    pub max_stroke_mm: f64,
    /// Minimum valid stroke length in mm.
    pub min_stroke_mm: f64,
    /// Homing timeout in milliseconds.
    pub timeout_ms: u64,
    /// Delay between current checks in milliseconds.
    pub poll_interval_ms: u32,
}

impl Default for HomingConfig {
    fn default() -> Self {
        Self {
            stall_threshold: 0.06,
            baseline_samples: 1000,
            poll_samples: 50,
            speed_steps_s: 500, // 25mm * 20 steps/mm
            backoff_mm: 10.0,
            max_stroke_mm: 500.0,
            min_stroke_mm: 50.0,
            timeout_ms: 40_000,
            poll_interval_ms: 10,
        }
    }
}

#[derive(Debug)]
pub enum BoardError<M: Debug, C: Debug> {
    Motor(M),
    CurrentSensor(C),
    StrokeTooShort,
    HomingTimeout,
}

impl<M: Debug, C: Debug> From<HomingError<M, C>> for BoardError<M, C> {
    fn from(e: HomingError<M, C>) -> Self {
        match e {
            HomingError::Motor(e) => BoardError::Motor(e),
            HomingError::CurrentSensor(e) => BoardError::CurrentSensor(e),
            HomingError::StrokeTooShort => BoardError::StrokeTooShort,
            HomingError::Timeout => BoardError::HomingTimeout,
        }
    }
}

#[derive(Debug)]
enum HomingError<M: Debug, C: Debug> {
    Motor(M),
    CurrentSensor(C),
    StrokeTooShort,
    Timeout,
}

/// Stock OSSM board: STEP/DIR motor with current-sense sensorless homing.
///
/// Unlike the OSSM Alt board which delegates homing to the motor firmware,
/// this board handles homing itself by crawling in one direction and
/// monitoring motor current via an ADC. A current spike indicates the
/// carriage has hit the end of the rail.
pub struct OssmStock<M, C, D>
where
    M: StepDir,
    C: CurrentSensor,
    D: DelayNs,
{
    motor: M,
    current: C,
    delay: D,
    mechanical: &'static MechanicalConfig,
    homing: HomingConfig,
    stroke_steps: i32,
}

impl<M, C, D> OssmStock<M, C, D>
where
    M: StepDir,
    C: CurrentSensor,
    D: DelayNs,
{
    pub fn new(
        motor: M,
        current: C,
        delay: D,
        mechanical: &'static MechanicalConfig,
        homing: HomingConfig,
    ) -> Self {
        Self {
            motor,
            current,
            delay,
            mechanical,
            homing,
            stroke_steps: 0,
        }
    }

    /// Measured stroke length in mm, available after homing.
    pub fn stroke_mm(&self) -> f64 {
        self.mechanical
            .steps_to_mm(self.stroke_steps, self.motor.steps_per_rev())
    }

    /// Run the current-sense homing sequence.
    ///
    /// Algorithm (from reference C++ firmware):
    /// 1. Calibrate current sensor baseline at rest
    /// 2. Crawl forward slowly, polling current every 10ms
    /// 3. When current exceeds threshold (stall), stop
    /// 4. Back away from the hard stop
    /// 5. Record stroke length
    /// 6. Return to home position (0)
    async fn run_homing(&mut self) -> Result<(), HomingError<M::Error, C::Error>> {
        // Calibrate: read current sensor at rest to get baseline
        let baseline = self
            .current
            .read_fraction(self.homing.baseline_samples)
            .map_err(HomingError::CurrentSensor)?;

        let steps_per_mm = self.mechanical.steps_per_mm(self.motor.steps_per_rev());
        let max_stroke_steps = (self.homing.max_stroke_mm * steps_per_mm as f64) as i32;
        let steps_per_poll =
            (self.homing.speed_steps_s as u64 * self.homing.poll_interval_ms as u64 / 1000) as i32;
        let steps_per_poll = steps_per_poll.max(1);

        // Crawl forward, checking current at each poll interval
        let mut total_steps: i32 = 0;
        let mut elapsed_ms: u64 = 0;

        while total_steps < max_stroke_steps {
            if elapsed_ms >= self.homing.timeout_ms {
                return Err(HomingError::Timeout);
            }

            // Check current
            let reading = self
                .current
                .read_fraction(self.homing.poll_samples)
                .map_err(HomingError::CurrentSensor)?;
            let current = reading - baseline;

            if current > self.homing.stall_threshold {
                // Stall detected — back away from the hard stop
                let backoff_steps =
                    (self.homing.backoff_mm * steps_per_mm as f64) as i32;
                let backoff_target = total_steps - backoff_steps;
                self.motor
                    .set_absolute_position(backoff_target)
                    .await
                    .map_err(HomingError::Motor)?;

                // Record measured stroke
                self.stroke_steps = total_steps.min(max_stroke_steps);

                let min_stroke_steps =
                    (self.homing.min_stroke_mm * steps_per_mm as f64) as i32;
                if self.stroke_steps < min_stroke_steps {
                    return Err(HomingError::StrokeTooShort);
                }

                // Return to home (position 0) and reset counter
                self.motor
                    .set_absolute_position(0)
                    .await
                    .map_err(HomingError::Motor)?;

                return Ok(());
            }

            self.delay
                .delay_ms(self.homing.poll_interval_ms)
                .await;
            elapsed_ms += self.homing.poll_interval_ms as u64;

            // Advance a small batch of steps
            let target = total_steps + steps_per_poll;
            self.motor
                .set_absolute_position(target)
                .await
                .map_err(HomingError::Motor)?;
            total_steps = target;
        }

        // Reached max stroke without detecting stall
        Err(HomingError::Timeout)
    }
}

impl<M, C, Dl> Board for OssmStock<M, C, Dl>
where
    M: StepDir,
    C: CurrentSensor,
    Dl: DelayNs,
{
    type Error = BoardError<M::Error, C::Error>;

    async fn enable(&mut self) -> Result<(), Self::Error> {
        self.motor.enable().await.map_err(BoardError::Motor)
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.motor.disable().await.map_err(BoardError::Motor)
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        self.run_homing().await?;
        self.motor.reset_position(0);
        Ok(())
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
