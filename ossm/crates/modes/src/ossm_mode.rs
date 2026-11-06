use embassy_time::Delay;
use pattern_engine::AnyPattern;

use crate::{ModeCtx, ModeSet, Motion, Patterns};

/// The modes OSSM firmware can switch between.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OssmMode {
    /// Nothing runs and the motor stays disabled.
    Idle,
    /// The pattern engine drives the motor from remote commands.
    Patterns,
}

impl ModeSet for OssmMode {
    async fn run(self, ctx: &ModeCtx) {
        match self {
            OssmMode::Idle => ctx.run(&idle).await,
            OssmMode::Patterns => ctx.run(&patterns).await,
        }
    }
}

async fn idle() {
    core::future::pending().await
}

async fn patterns(motion: Motion<'_>, patterns: Patterns<'_>) {
    patterns
        .run(&motion, AnyPattern::all_builtin(), Delay)
        .await
}
