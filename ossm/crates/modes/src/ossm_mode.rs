use embassy_time::Delay;
use pattern_engine::AnyPattern;

use crate::{Motion, Patterns};

/// Keys for the modes OSSM firmware registers, besides the built-in
/// [`Idle`](crate::ModeKey::Idle).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OssmMode {
    /// The pattern engine drives the motor from remote commands.
    Patterns,
}

/// Run the built-in patterns from remote commands.
pub async fn patterns(motion: Motion<'_>, patterns: Patterns<'_>) {
    patterns
        .run(&motion, AnyPattern::all_builtin(), Delay)
        .await
}
