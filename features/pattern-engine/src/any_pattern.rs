use embedded_hal_async::delay::DelayNs;

use crate::pattern::{Pattern, PatternCtx};
use crate::patterns::{Deeper, HalfHalf, NonePattern, Simple, StopNGo, TeasingPounding, Torque};

/// Enum dispatch wrapper for all built-in patterns.
///
/// Avoids trait objects (`dyn Pattern`) which are problematic in `no_std`
/// (object-safety constraints on async fns, no vtable allocation without alloc).
/// Each variant is a ZST or near-ZST, so `AnyPattern` itself is tiny.
pub enum AnyPattern {
    Simple(Simple),
    Deeper(Deeper),
    HalfHalf(HalfHalf),
    StopNGo(StopNGo),
    TeasingPounding(TeasingPounding),
    Torque(Torque),
    None(NonePattern),
}

impl AnyPattern {
    /// Names of the built-in patterns, matching the order of [`Self::all_builtin`].
    pub const BUILTIN_NAMES: [&'static str; 7] = [
        "Simple Stroke",
        "Deeper",
        "Half'n'Half",
        "Stop'n'Go",
        "Teasing Pounding",
        "Torque",
        "None",
    ];

    /// Returns an array of all built-in patterns in their default order.
    pub fn all_builtin() -> [AnyPattern; 7] {
        [
            AnyPattern::Simple(Simple),
            AnyPattern::Deeper(Deeper),
            AnyPattern::HalfHalf(HalfHalf),
            AnyPattern::StopNGo(StopNGo),
            AnyPattern::TeasingPounding(TeasingPounding),
            AnyPattern::Torque(Torque),
            AnyPattern::None(NonePattern),
        ]
    }
}

impl Pattern for AnyPattern {
    fn name(&self) -> &'static str {
        match self {
            Self::Simple(p) => p.name(),
            Self::Deeper(p) => p.name(),
            Self::HalfHalf(p) => p.name(),
            Self::StopNGo(p) => p.name(),
            Self::TeasingPounding(p) => p.name(),
            Self::Torque(p) => p.name(),
            Self::None(p) => p.name(),
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Simple(p) => p.description(),
            Self::Deeper(p) => p.description(),
            Self::HalfHalf(p) => p.description(),
            Self::StopNGo(p) => p.description(),
            Self::TeasingPounding(p) => p.description(),
            Self::Torque(p) => p.description(),
            Self::None(p) => p.description(),
        }
    }

    async fn run(&mut self, ctx: &mut PatternCtx<impl DelayNs>) {
        match self {
            Self::Simple(p) => p.run(ctx).await,
            Self::Deeper(p) => p.run(ctx).await,
            Self::HalfHalf(p) => p.run(ctx).await,
            Self::StopNGo(p) => p.run(ctx).await,
            Self::TeasingPounding(p) => p.run(ctx).await,
            Self::Torque(p) => p.run(ctx).await,
            Self::None(p) => p.run(ctx).await,
        }
    }
}

impl From<Simple> for AnyPattern {
    fn from(p: Simple) -> Self {
        Self::Simple(p)
    }
}

impl From<Deeper> for AnyPattern {
    fn from(p: Deeper) -> Self {
        Self::Deeper(p)
    }
}

impl From<HalfHalf> for AnyPattern {
    fn from(p: HalfHalf) -> Self {
        Self::HalfHalf(p)
    }
}

impl From<StopNGo> for AnyPattern {
    fn from(p: StopNGo) -> Self {
        Self::StopNGo(p)
    }
}

impl From<TeasingPounding> for AnyPattern {
    fn from(p: TeasingPounding) -> Self {
        Self::TeasingPounding(p)
    }
}

impl From<Torque> for AnyPattern {
    fn from(p: Torque) -> Self {
        Self::Torque(p)
    }
}

impl From<NonePattern> for AnyPattern {
    fn from(p: NonePattern) -> Self {
        Self::None(p)
    }
}
