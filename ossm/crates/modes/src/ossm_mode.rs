use embassy_futures::select::select;
use embassy_time::Delay;
use pattern_engine::{AnyPattern, PatternEvent, PatternUpdate};

use crate::{Input, Motion, PatternInput, Patterns, Received};

/// Keys for the modes OSSM firmware registers, besides the built-in
/// [`Idle`](crate::ModeKey::Idle).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OssmMode {
    /// The pattern engine drives the motor from remote commands.
    Patterns,
}

/// Run the built-in patterns from remote commands.
pub async fn patterns(
    motion: Motion<'_>,
    patterns: Patterns<'_>,
    input: Input<'_, PatternInput>,
) {
    let forward = async {
        loop {
            match input.next().await {
                Received::State(state) => patterns.set(state),
                Received::Event { event, .. } => {
                    // The engine zeroes its own speed on stop, but the input
                    // holds the state it is fed from, so zero that too or the
                    // next update would bring the old speed back.
                    if event == PatternEvent::Stop {
                        input.update(PatternUpdate::Speed(0.0));
                    }
                    patterns.send(event);
                }
            }
        }
    };
    select(
        patterns.run(&motion, AnyPattern::all_builtin(), Delay),
        forward,
    )
    .await;
}
