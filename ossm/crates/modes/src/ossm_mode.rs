use embassy_futures::select::select3;
use embassy_time::Delay;
use pattern_engine::{AnyPattern, PatternEvent, PatternUpdate};

use crate::{Input, Motion, PatternInput, Patterns, Received, RemoteLost};

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
    lost: RemoteLost<'_>,
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
    // Losing a remote pauses, so the user can pick up where they left off
    // once it reconnects.
    let on_lost = async {
        loop {
            let event = lost.next().await;
            patterns.send(PatternEvent::Pause);
            event.handled();
        }
    };
    select3(
        patterns.run(&motion, AnyPattern::all_builtin(), Delay),
        forward,
        on_lost,
    )
    .await;
}
