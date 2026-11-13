//! What a platform implements to run OSSM on its target.

use ossm_core::{Board, MotionController, MotionLimits, MotionObserver};

pub use ossm_core::fault::Fault;
pub use ossm_core::{build_info, logging};
pub use pattern_engine::{PatternObserver, PatternSender};

/// The hardware a platform set up.
pub struct Hardware<B> {
    pub board: B,
    pub limits: MotionLimits,
    /// How often the motion controller updates, which
    /// [`Platform::start_motion`] must keep to.
    pub update_interval_secs: f64,
}

/// Where OSSM runs: the board, how the motion controller is driven, and
/// what runs alongside the modes.
#[allow(async_fn_in_trait)]
pub trait Platform {
    type Board: Board;

    /// Set up the hardware, before anything else runs.
    async fn init(&mut self) -> Result<Hardware<Self::Board>, Fault>;

    /// Run the motion controller wherever it must run, such as its own
    /// core, at the hardware's update interval. Returns once it is
    /// running.
    async fn start_motion(&mut self, controller: MotionController<'static, Self::Board>);

    /// Show the machine's state, e.g. on an LED. Started even if
    /// [`init`](Self::init) fails, so the fault can be shown.
    fn start_indicator(&mut self, motion: MotionObserver, patterns: PatternObserver);

    /// Start the remotes. Until remotes go through the modes, they drive
    /// the pattern engine directly.
    fn start_remotes(&mut self, patterns: &'static PatternSender, limits: &MotionLimits);
}
