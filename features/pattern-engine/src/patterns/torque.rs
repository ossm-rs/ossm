use embedded_hal_async::delay::DelayNs;

use crate::pattern::{Pattern, PatternCtx};

pub struct Torque;

impl Pattern for Torque {
    fn name(&self) -> &'static str {
        "Torque"
    }

    fn description(&self) -> &'static str {
        "Same as simple. Sensation controls the torque applied."
    }

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) -> Result<(), ossm::Cancelled> {
        loop {
            let torque = ctx.scale_sensation(0.0, 1.0);
            ctx.motion().position(1.0).torque(torque).send().await?;
            ctx.motion().position(0.0).torque(torque).send().await?;
        }
    }
}
