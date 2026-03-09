use embedded_hal_async::delay::DelayNs;

use crate::pattern::{Pattern, PatternCtx};

pub struct NonePattern;

impl Pattern for NonePattern {
    fn name(&self) -> &'static str {
        "None"
    }

    fn description(&self) -> &'static str {
        "No pattern. Holds position."
    }

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) -> Result<(), ossm::Cancelled> {
        loop {
            ctx.delay_ms(500).await;
        }
    }
}
