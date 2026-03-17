#![no_std]

mod engine;
mod input;
mod pattern;
pub mod patterns;
mod util;

pub use engine::{EngineState, PatternEngine, PatternEngineRunner};
pub use input::{PatternInput, SharedPatternInput};
pub use pattern::{Pattern, PatternCtx};
pub use util::scale;

use embedded_hal_async::delay::DelayNs;

use crate::patterns::{Deeper, HalfHalf, NonePattern, Simple, StopNGo, TeasingPounding, Torque};

// To add a new pattern:
// 1. Create a struct in `patterns/` that implements `Pattern`.
// 2. Re-export it from `patterns/mod.rs`.
// 3. Add a `Variant(Type)` entry below.
//
// The macro generates the `AnyPattern` enum, `Pattern` trait delegation,
// `From` impls, `BUILTIN_NAMES`, and `all_builtin()`.
// See `any_pattern_macro.rs` for the full definition.
include!("any_pattern_macro.rs");

define_patterns! {
    Simple(Simple),
    Deeper(Deeper),
    HalfHalf(HalfHalf),
    StopNGo(StopNGo),
    TeasingPounding(TeasingPounding),
    Torque(Torque),
    None(NonePattern),
}
