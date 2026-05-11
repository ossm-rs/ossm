//! Pattern command event for the events bus.
//!
//! Any input device that wants to control the pattern engine publishes
//! a [`PatternCmd`] via the [`events`] bus. The single consumer is the
//! pattern-engine bridge ([`PatternSender::apply`](crate::PatternSender::apply))
//! that routes each variant into the engine.

use crate::{AnyPattern, PatternMeta};

/// Unified pattern command. Flows through the [`events`] bus.
///
/// Combines one-shot playback transitions (`Play`/`Pause`/`Resume`/
/// `Stop`/`Home`) with continuous input values (`SetSpeed`/`SetDepth`/
/// `SetStroke`/`SetSensation`).
#[derive(Debug, Clone, Copy)]
pub enum PatternCmd {
    Play(usize),
    Pause,
    Resume,
    Stop,
    Home,
    /// 0.0-1.0 (fraction of max velocity).
    SetSpeed(f64),
    /// 0.0-1.0 (fraction of depth).
    SetStroke(f64),
    /// 0.0-1.0 (fraction of machine range).
    SetDepth(f64),
    /// -1.0-1.0 (pattern-specific).
    SetSensation(f64),
}

events::declare_event!(PatternCmd);

/// Static list of built-in pattern metadata.
pub fn pattern_list() -> &'static [PatternMeta] {
    &AnyPattern::BUILTIN_PATTERNS
}

/// Pattern description by index, if present.
pub fn pattern_description(idx: usize) -> Option<&'static str> {
    AnyPattern::BUILTIN_PATTERNS.get(idx).map(|m| m.description)
}
