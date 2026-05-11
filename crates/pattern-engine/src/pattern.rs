use embassy_futures::select::{self, Either3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::watch::Receiver;
use embassy_time::{Duration, Ticker};
use embedded_hal_async::delay::DelayNs;
use ossm::{Cancelled, MotionSender, MotionCommand};

use crate::input::{PatternInput, SharedPatternInput};
use crate::util::scale;

/// Cap on how often input-driven motion updates are forwarded to the controller.
/// Each forwarded command costs a Ruckig replan;  replans can take 5-15 ms,
/// so an unthrottled BLE input flood starves the 100 Hz motion loop.
/// 250 ms keeps replans to ~4 Hz, well under the 10 ms tick budget while
/// still feeling responsive to slider movement.
const INPUT_UPDATE_THROTTLE: Duration = Duration::from_millis(250);

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

    async fn run(&mut self, ctx: &mut PatternCtx<'_, impl DelayNs>) -> Result<(), Cancelled>;
}

pub struct PatternCtx<'m, D: DelayNs> {
    motion: &'m MotionSender,
    input: &'m SharedPatternInput,
    input_receiver: Receiver<'m, CriticalSectionRawMutex, PatternInput, 1>,
    delay: D,
}

impl<'m, D: DelayNs> PatternCtx<'m, D> {
    pub fn new(
        motion: &'m MotionSender,
        input: &'m SharedPatternInput,
        delay: D,
    ) -> Self {
        let input_receiver = input.receiver().expect("Watch receiver slot already taken");
        Self {
            motion,
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
    pub fn motion(&mut self) -> MotionBuilder<'_, 'm, D, NoPosition> {
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
/// Created via [`PatternCtx::motion()`]. Call `.position()` before `.send()` -
/// the type system enforces this at compile time.
pub struct MotionBuilder<'a, 'm, D: DelayNs, P> {
    ctx: &'a mut PatternCtx<'m, D>,
    position: P,
    speed_factor: f64,
    torque: Option<f64>,
}

impl<'a, 'm, D: DelayNs, P> MotionBuilder<'a, 'm, D, P> {
    /// Set the velocity as a multiplier of the current input velocity.
    ///
    /// Default is 1.0 (full input velocity). 0.5 = half speed, max is 1.0.
    pub fn speed(mut self, factor: f64) -> Self {
        self.speed_factor = factor;
        self
    }

    /// Set the torque limit as a factor between 0.0 and 1.0.
    ///
    /// `None` (the default) uses the motor's default torque.
    pub fn torque(mut self, factor: f64) -> Self {
        self.torque = Some(factor);
        self
    }
}

impl<'a, 'm, D: DelayNs> MotionBuilder<'a, 'm, D, NoPosition> {
    /// Set the target position as a fraction of the stroke range.
    ///
    /// 0.0 = shallowest (`depth - stroke`), 1.0 = deepest (`depth`).
    pub fn position(self, fraction: f64) -> MotionBuilder<'a, 'm, D, HasPosition> {
        MotionBuilder {
            ctx: self.ctx,
            position: HasPosition(fraction),
            speed_factor: self.speed_factor,
            torque: self.torque,
        }
    }
}

fn compute_command(
    input: &PatternInput,
    fraction: f64,
    speed_factor: f64,
    torque: Option<f64>,
) -> MotionCommand {
    let stroke = input.depth * input.stroke.clamp(0.0, 1.0);
    let shallow = input.depth - stroke;
    let position = shallow + fraction * stroke;
    let speed = input.velocity * speed_factor.clamp(0.0, 1.0);
    MotionCommand {
        position,
        speed,
        torque,
    }
}

impl<'a, 'm, D: DelayNs> MotionBuilder<'a, 'm, D, HasPosition> {
    pub async fn send(self) -> Result<(), Cancelled> {
        let fraction = self.position.0.clamp(0.0, 1.0);
        let speed_factor = self.speed_factor;
        let torque = self.torque;

        let input = self.ctx.input();
        let cmd = compute_command(&input, fraction, speed_factor, torque);
        self.ctx.motion.begin_motion(cmd);

        let mut move_done = core::pin::pin!(self.ctx.motion.await_motion());
        let mut throttle = Ticker::every(INPUT_UPDATE_THROTTLE);
        let mut pending: Option<PatternInput> = None;

        loop {
            match select::select3(
                move_done.as_mut(),
                self.ctx.input_receiver.changed(),
                throttle.next(),
            )
            .await
            {
                Either3::First(result) => return result,
                Either3::Second(new_input) => {
                    pending = Some(new_input);
                }
                Either3::Third(()) => {
                    if let Some(input) = pending.take() {
                        let cmd = compute_command(&input, fraction, speed_factor, torque);
                        self.ctx.motion.update_motion(cmd);
                    }
                }
            }
        }
    }
}
