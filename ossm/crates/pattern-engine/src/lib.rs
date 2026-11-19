#![no_std]

mod engine;
mod input;
mod observer;
mod pattern;
pub mod patterns;
mod runner;
mod sender;
mod util;

pub use engine::{EnginePhase, PatternEngine};
pub use input::{PatternEvent, PatternState, PatternUpdate, SharedPatternState};
pub use observer::PatternObserver;
pub use pattern::{Pattern, PatternCtx};
pub use runner::PatternRunner;
pub use sender::PatternSender;
pub use util::scale;

/// Public-facing metadata for a pattern.
#[derive(Debug, Clone, Copy)]
pub struct PatternMeta {
    pub name: &'static str,
    pub description: &'static str,
}

/// Static list of built-in pattern metadata.
pub fn pattern_list() -> &'static [PatternMeta] {
    &AnyPattern::BUILTIN_PATTERNS
}

/// Pattern description by index, if present.
pub fn pattern_description(idx: usize) -> Option<&'static str> {
    AnyPattern::BUILTIN_PATTERNS
        .get(idx)
        .map(|m| m.description)
}

use embedded_hal_async::delay::DelayNs;

use crate::patterns::{
    Deeper,
    HalfHalf,
    Hammerjack,
    Humanize,
    Insist,
    Jackhammer,
    Knot,
    NonePattern,
    RoboStroke,
    Pull,
    Push,
    Simple,
    StopNGo,
    Stutter,
    TeasingPounding,
    Torque
};

// To add a new pattern:
// 1. Create a struct in `patterns/` that implements `Pattern`.
// 2. Re-export it from `patterns/mod.rs`.
// 3. Add a `Variant(Type)` entry below.
//
// The macro generates the `AnyPattern` enum, `Pattern` trait delegation,
// `From` impls, `BUILTIN_PATTERNS`, and `all_builtin()`.
// See `any_pattern_macro.rs` for the full definition.
include!("any_pattern_macro.rs");

define_patterns! {
    Simple(Simple),
    Deeper(Deeper),
    HalfHalf(HalfHalf),
    Hammerjack(Hammerjack),
    Humanize(Humanize),
    Insist(Insist),
    Jackhammer(Jackhammer),
    Knot(Knot),
    Pull(Pull),
    Push(Push),
    RoboStroke(RoboStroke),
    StopNGo(StopNGo),
    Stutter(Stutter),
    TeasingPounding(TeasingPounding),
    Torque(Torque),
    None(NonePattern),
}
