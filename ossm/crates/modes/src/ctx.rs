use ossm::MotionSender;
use pattern_engine::PatternRunner;

use crate::Mode;

/// Owner of every capability a mode can ask for.
pub struct ModeCtx {
    pub(crate) motion: MotionSender,
    pub(crate) patterns: PatternRunner,
}

impl ModeCtx {
    pub(crate) fn new(motion: MotionSender, patterns: PatternRunner) -> Self {
        Self { motion, patterns }
    }

    /// Run `mode` until it completes, lending it the capabilities it
    /// declared.
    pub async fn run<Args, M: Mode<Args>>(&self, mode: &M) {
        mode.run(self).await
    }
}
