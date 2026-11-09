use ossm::MotionSender;
use pattern_engine::{PatternRunner, PatternSender};

use crate::Mode;
use crate::remote::Inputs;

/// Owner of every capability a mode can ask for.
///
/// Built by the [`ModeRunner`](crate::ModeRunner) and never handed out;
/// modes reach it only through their extractors.
pub struct ModeCtx {
    pub(crate) motion: MotionSender,
    pub(crate) patterns: PatternRunner,
    pub(crate) pattern_sender: &'static PatternSender,
    pub(crate) inputs: &'static Inputs,
}

impl ModeCtx {
    pub(crate) fn new(
        motion: MotionSender,
        patterns: PatternRunner,
        pattern_sender: &'static PatternSender,
        inputs: &'static Inputs,
    ) -> Self {
        Self {
            motion,
            patterns,
            pattern_sender,
            inputs,
        }
    }

    /// Run `mode` until it completes, lending it the capabilities it
    /// declared.
    ///
    /// Remotes can only reach the mode with the input kinds its
    /// [`Input`](crate::Input) extractors declare, and only while it runs.
    /// Events queued for an earlier mode are discarded first.
    pub async fn run<Args, M: Mode<Args>>(&self, mode: &M) {
        self.inputs.open(M::ACCEPTS);
        let _close = CloseOnDrop(self.inputs);
        mode.run(self).await
    }
}

/// Stops `Inputs` accepting requests when a mode's run ends or is
/// dropped.
struct CloseOnDrop(&'static Inputs);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}
