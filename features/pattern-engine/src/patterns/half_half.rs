use embedded_hal_async::delay::DelayNs;

use crate::pattern::{Pattern, PatternCtx, MAX_SENSATION};
use crate::util::scale;

const MAX_SCALING_FACTOR: f64 = 5.0;
const BASE_SPEED: f64 = 1.0 / MAX_SCALING_FACTOR;

pub struct HalfHalf;

impl Pattern for HalfHalf {
    fn name(&self) -> &'static str {
        "Half'n'Half"
    }

    fn description(&self) -> &'static str {
        "Alternate between full and half strokes. Sensation controls speed ratio."
    }

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) -> Result<(), ossm::Cancelled> {
        let mut half = false;

        loop {
            let sensation = ctx.sensation();
            let factor = scale(sensation.abs(), 0.0, MAX_SENSATION, 1.0, MAX_SCALING_FACTOR);

            let (out_speed, in_speed) = if sensation > 0.0 {
                (BASE_SPEED, BASE_SPEED * factor)
            } else if sensation < 0.0 {
                (BASE_SPEED * factor, BASE_SPEED)
            } else {
                (BASE_SPEED, BASE_SPEED)
            };

            let depth = if half { 0.5 } else { 1.0 };
            half = !half;

            ctx.motion().position(depth).speed(out_speed).send().await?;
            ctx.motion().position(0.0).speed(in_speed).send().await?;
        }
    }
}
