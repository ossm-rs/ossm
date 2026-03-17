use embedded_hal_async::delay::DelayNs;

use crate::pattern::{MAX_SENSATION, Pattern, PatternCtx};
use crate::util::scale;

const MAX_SCALING_FACTOR: f64 = 5.0;
const BASE_SPEED: f64 = 1.0 / MAX_SCALING_FACTOR;

pub struct TeasingPounding;

impl Pattern for TeasingPounding {
    const NAME: &'static str = "Teasing Pounding";
    const DESCRIPTION: &'static str = "Alternating strokes. Sensation controls speed ratio of in and out strokes.";

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) -> Result<(), ossm::Cancelled> {
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

            ctx.motion().position(1.0).speed(out_speed).send().await?;
            ctx.motion().position(0.0).speed(in_speed).send().await?;
        }
    }
}
