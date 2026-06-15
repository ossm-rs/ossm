/// Monotonic time source for the [`MotionController`](crate::MotionController).
///
/// The controller *pulls* elapsed time from a `Clock` rather than trusting the
/// caller to report it: a caller cannot pass a stale, doubled, or negative
/// `dt`. Only differences between successive [`now_micros`](Clock::now_micros)
/// readings are meaningful, and implementations **must be monotonic** (never
/// return a smaller value than a previous call).
///
/// Injecting the clock (instead of calling `Instant::now()` directly) keeps the
/// controller free of a hard dependency on a running time driver, so tests can
/// drive it with controlled time via a fake clock.
pub trait Clock {
    /// A monotonically non-decreasing timestamp in microseconds.
    fn now_micros(&self) -> u64;
}

/// [`Clock`] backed by `embassy_time::Instant`.
///
/// Requires an embassy-time driver to be installed by the final binary (the
/// firmware crates and the wasm runtime both provide one). Zero-sized, so it
/// costs nothing to store on the controller.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmbassyClock;

impl Clock for EmbassyClock {
    fn now_micros(&self) -> u64 {
        embassy_time::Instant::now().as_micros()
    }
}
