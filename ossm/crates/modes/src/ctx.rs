use ossm::{MotionObserver, MotionSender};

use crate::Mode;

/// Owner of every capability a mode can ask for.
pub struct ModeCtx {
    pub(crate) motion: MotionSender,
    pub(crate) motion_status: MotionObserver,
}

impl ModeCtx {
    pub fn new(motion: MotionSender, motion_status: MotionObserver) -> Self {
        Self {
            motion,
            motion_status,
        }
    }

    /// Run `mode` until it completes, lending it the capabilities it
    /// declared.
    pub async fn run<Args, M: Mode<Args>>(&self, mode: &M) {
        mode.run(self).await
    }
}
