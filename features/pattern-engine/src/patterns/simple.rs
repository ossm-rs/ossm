use embedded_hal_async::delay::DelayNs;

use crate::pattern::{Pattern, PatternCtx};

pub struct Simple;

impl Pattern for Simple {
    fn name(&self) -> &'static str {
        "Simple Stroke"
    }

    fn description(&self) -> &'static str {
        "Simple in and out. Sensation does nothing."
    }

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) -> Result<(), ossm::Cancelled> {
        loop {
            ctx.motion().position(1.0).send().await?;
            ctx.motion().position(0.0).send().await?;
        }
    }
}
