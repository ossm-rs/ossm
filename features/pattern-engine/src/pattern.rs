use embedded_hal_async::delay::DelayNs;
use ossm::{Command, MotionCommand, OssmChannels};

use crate::input::{PatternInput, SharedPatternInput};
use crate::util::scale;

pub const MIN_SENSATION: f64 = -1.0;
pub const MAX_SENSATION: f64 = 1.0;

/// An async pattern that drives repetitive motion.
///
/// `run()` loops forever (or until the future is dropped). Cancellation is
/// handled externally — the caller races the pattern against a cancel signal
/// using `embassy_futures::select`. Patterns need no explicit cancellation logic.
#[allow(async_fn_in_trait)]
pub trait Pattern {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>);
}

pub struct PatternCtx<D: DelayNs> {
    channels: &'static OssmChannels,
    input: &'static SharedPatternInput,
    delay: D,
}

impl<D: DelayNs> PatternCtx<D> {
    pub fn new(
        channels: &'static OssmChannels,
        input: &'static SharedPatternInput,
        delay: D,
    ) -> Self {
        Self {
            channels,
            input,
            delay,
        }
    }

    /// Read the current sensation value (-1.0 to 1.0).
    ///
    /// Re-read at each `.await` point to pick up live changes from BLE/UI.
    pub fn sensation(&self) -> f64 {
        self.input.lock(|cell| cell.get()).sensation
    }

    fn input(&self) -> PatternInput {
        self.input.lock(|cell| cell.get())
    }

    /// Start building a motion command.
    ///
    /// Chain `.position()` to set the target, optionally `.speed()` to override
    /// the velocity multiplier, then `.send().await` to execute.
    ///
    /// ```ignore
    /// ctx.motion().position(1.0).send().await;
    /// ctx.motion().position(0.5).speed(0.5).send().await;
    /// ```
    pub fn motion(&self) -> MotionBuilder<'_, D, NoPosition> {
        MotionBuilder {
            ctx: self,
            position: NoPosition,
            speed_factor: 1.0,
            torque: None,
        }
    }

    async fn send_command(&self, cmd: MotionCommand) {
        self.channels.move_complete.reset();
        let _ = self.channels.commands.try_send(Command::Motion(cmd));
        self.channels.move_complete.wait().await;
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
    ctx: &'a PatternCtx<D>,
    position: P,
    speed_factor: f64,
    torque: Option<f64>,
}

impl<'a, D: DelayNs, P> MotionBuilder<'a, D, P> {
    /// Set the velocity as a multiplier of the current input velocity.
    ///
    /// Default is 1.0 (full input velocity). 0.5 = half speed, 2.0 = double.
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

impl<'a, D: DelayNs> MotionBuilder<'a, D, HasPosition> {
    pub async fn send(self) {
        let input = self.ctx.input();
        let fraction = self.position.0.clamp(0.0, 1.0);
        let shallow = input.depth - input.stroke;
        let position = shallow + fraction * input.stroke;
        let speed = input.velocity * self.speed_factor;
        self.ctx
            .send_command(MotionCommand {
                position,
                speed,
                torque: self.torque,
            })
            .await;
    }
}
