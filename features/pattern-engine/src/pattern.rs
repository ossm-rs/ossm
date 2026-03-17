use embassy_futures::select::{self, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::watch::Receiver;
use embedded_hal_async::delay::DelayNs;
use ossm::{Cancelled, MotionCommand, Ossm};

use crate::input::{PatternInput, SharedPatternInput};
use crate::util::scale;

pub const MIN_SENSATION: f64 = -1.0;
pub const MAX_SENSATION: f64 = 1.0;

/// An async pattern that drives repetitive motion.
///
/// `run()` loops forever, using `?` on each move to propagate cancellation.
/// When a state command (disable, home) cancels the in-flight move,
/// `send().await?` returns `Err(Cancelled)`, which exits the pattern cleanly.
#[allow(async_fn_in_trait)]
pub trait Pattern {
    const NAME: &'static str;
    const DESCRIPTION: &'static str;

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn description(&self) -> &'static str {
        Self::DESCRIPTION
    }

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) -> Result<(), Cancelled>;
}

pub struct PatternCtx<D: DelayNs> {
    ossm: &'static Ossm,
    input: &'static SharedPatternInput,
    input_receiver: Receiver<'static, CriticalSectionRawMutex, PatternInput, 1>,
    delay: D,
}

impl<D: DelayNs> PatternCtx<D> {
    pub fn new(ossm: &'static Ossm, input: &'static SharedPatternInput, delay: D) -> Self {
        let input_receiver = input.receiver().expect("Watch receiver slot already taken");
        Self {
            ossm,
            input,
            input_receiver,
            delay,
        }
    }

    /// Read the current sensation value (-1.0 to 1.0).
    ///
    /// Re-read at each `.await` point to pick up live changes from BLE/UI.
    pub fn sensation(&self) -> f64 {
        self.input
            .try_get()
            .unwrap_or(PatternInput::DEFAULT)
            .sensation
    }

    fn input(&self) -> PatternInput {
        self.input.try_get().unwrap_or(PatternInput::DEFAULT)
    }

    /// Start building a motion command.
    ///
    /// Chain `.position()` to set the target, optionally `.speed()` to override
    /// the velocity multiplier, then `.send().await?` to execute.
    ///
    /// ```ignore
    /// ctx.motion().position(1.0).send().await?;
    /// ctx.motion().position(0.5).speed(0.5).send().await?;
    /// ```
    pub fn motion(&mut self) -> MotionBuilder<'_, D, NoPosition> {
        MotionBuilder {
            ctx: self,
            position: NoPosition,
            speed_factor: 1.0,
            torque: None,
        }
    }

    pub async fn delay_ms(&mut self, ms: u64) {
        self.delay.delay_ms(ms as u32).await;
    }

    /// Map the current sensation (-1.0..1.0) to an output range.
    pub fn scale_sensation(&self, out_min: f64, out_max: f64) -> f64 {
        scale(
            self.sensation(),
            MIN_SENSATION,
            MAX_SENSATION,
            out_min,
            out_max,
        )
    }
}

pub struct NoPosition;

pub struct HasPosition(f64);

/// Builder for a single motion command.
///
/// Created via [`PatternCtx::motion()`]. Call `.position()` before `.send()` —
/// the type system enforces this at compile time.
pub struct MotionBuilder<'a, D: DelayNs, P> {
    ctx: &'a mut PatternCtx<D>,
    position: P,
    speed_factor: f64,
    torque: Option<f64>,
}

impl<'a, D: DelayNs, P> MotionBuilder<'a, D, P> {
    /// Set the velocity as a multiplier of the current input velocity.
    ///
    /// Default is 1.0 (full input velocity). 0.5 = half speed. Clamped to 0.0–1.0.
    pub fn speed(mut self, factor: f64) -> Self {
        self.speed_factor = factor;
        self
    }

    /// Set the torque limit as a factor (0.0–1.0).
    ///
    /// `None` (the default) uses the motor's default torque.
    pub fn torque(mut self, factor: f64) -> Self {
        self.torque = Some(factor);
        self
    }
}

impl<'a, D: DelayNs> MotionBuilder<'a, D, NoPosition> {
    /// Set the target position as a fraction of the stroke range.
    ///
    /// 0.0 = shallowest (`depth - stroke`), 1.0 = deepest (`depth`).
    pub fn position(self, fraction: f64) -> MotionBuilder<'a, D, HasPosition> {
        MotionBuilder {
            ctx: self.ctx,
            position: HasPosition(fraction),
            speed_factor: self.speed_factor,
            torque: self.torque,
        }
    }
}

fn compute_command(input: &PatternInput, fraction: f64, speed_factor: f64, torque: Option<f64>) -> MotionCommand {
    let shallow = (input.depth - input.stroke).max(0.0);
    let stroke = input.depth - shallow;
    let position = shallow + fraction * stroke;
    let speed = input.velocity * speed_factor.clamp(0.0, 1.0);
    MotionCommand {
        position,
        speed,
        torque,
    }
}

impl<'a, D: DelayNs> MotionBuilder<'a, D, HasPosition> {
    pub async fn send(self) -> Result<(), Cancelled> {
        let fraction = self.position.0.clamp(0.0, 1.0);
        let speed_factor = self.speed_factor;
        let torque = self.torque;

        let input = self.ctx.input();
        let cmd = compute_command(&input, fraction, speed_factor, torque);
        self.ctx.ossm.begin_motion(cmd);

        let mut move_done = core::pin::pin!(self.ctx.ossm.await_motion());

        loop {
            match select::select(move_done.as_mut(), self.ctx.input_receiver.changed()).await {
                Either::First(result) => return result,
                Either::Second(new_input) => {
                    let cmd = compute_command(&new_input, fraction, speed_factor, torque);
                    self.ctx.ossm.update_motion(cmd);
                }
            }
        }
    }
}
