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
    /// A fault was raised. Holds no capabilities; only a restart leaves.
    Fault,
}

impl ModeSet for OssmMode {
    const FAULT: Self = OssmMode::Fault;

    async fn run(self, ctx: &ModeCtx) {
        match self {
            OssmMode::Idle => ctx.run(&idle).await,
            OssmMode::Patterns => ctx.run(&patterns).await,
            OssmMode::Fault => ctx.run(&fault).await,
        }
    }
}

async fn idle() {
    core::future::pending().await
}

async fn fault() {
    core::future::pending().await
}

async fn patterns(motion: Motion<'_>, patterns: Patterns<'_>) {
    patterns
        .run(&motion, AnyPattern::all_builtin(), Delay)
        .await
}
