use ossm::MotionSender;
use pattern_engine::{PatternRunner, PatternSender};

use crate::Mode;
use crate::remote::Mailbox;

/// Owner of every capability a mode can ask for.
///
/// Built by the [`ModeRunner`](crate::ModeRunner) and never handed out;
/// modes reach it only through their extractors.
pub struct ModeCtx {
    pub(crate) motion: MotionSender,
    pub(crate) patterns: PatternRunner,
    pub(crate) pattern_sender: &'static PatternSender,
    pub(crate) mailbox: &'static Mailbox,
}

impl ModeCtx {
    pub(crate) fn new(
        motion: MotionSender,
        patterns: PatternRunner,
        pattern_sender: &'static PatternSender,
        mailbox: &'static Mailbox,
    ) -> Self {
        Self {
            motion,
            patterns,
            pattern_sender,
            mailbox,
        }
    }

    /// Run `mode` until it completes, lending it the capabilities it
    /// declared.
    ///
    /// Remotes can only reach the mode with the request kinds its
    /// [`Commands`](crate::Commands) extractors declare, and only while it
    /// runs. Anything queued for an earlier mode is discarded first.
    pub async fn run<Args, M: Mode<Args>>(&self, mode: &M) {
        self.mailbox.open(M::ACCEPTS);
        let _close = CloseOnDrop(self.mailbox);
        mode.run(self).await
    }
}

/// Stops the mailbox accepting requests when a mode's run ends or is
/// dropped.
struct CloseOnDrop(&'static Mailbox);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}
