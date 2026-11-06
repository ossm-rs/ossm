use core::ops::Deref;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};
use ossm::{MotionPhase, MotionSender, MotionState};
use pattern_engine::PatternRunner;

use crate::ModeCtx;

/// A capability a mode can declare as an argument.
///
/// Implemented on the `'static` form of each extractor type (e.g.
/// `Motion<'static>`), which only serves as a name for the type. The
/// value a mode actually receives is [`Out<'a>`](Self::Out), borrowed
/// from the [`ModeCtx`] for the duration of the run.
pub trait Extract {
    type Out<'a>;

    fn extract(ctx: &ModeCtx) -> Self::Out<'_>;
}

/// Permission to move the motor.
pub struct Motion<'a>(&'a MotionSender);

impl Deref for Motion<'_> {
    type Target = MotionSender;

    fn deref(&self) -> &MotionSender {
        self.0
    }
}

impl Extract for Motion<'static> {
    type Out<'a> = Motion<'a>;

    fn extract(ctx: &ModeCtx) -> Motion<'_> {
        Motion(&ctx.motion)
    }
}

/// Read only view of motion state and phase transitions.
///
/// Borrows the same [`MotionSender`] as [`Motion`] but only exposes its
/// read methods, so a mode holding just a `MotionStatus` cannot issue
/// commands.
pub struct MotionStatus<'a>(&'a MotionSender);

impl MotionStatus<'_> {
    /// Current motion state (position, velocity, torque, phase).
    pub fn state(&self) -> MotionState {
        self.0.state()
    }

    /// Subscribe to [`MotionPhase`] transitions.
    ///
    /// Returns `Err` if all subscriber slots are in use.
    pub fn subscribe(
        &self,
    ) -> Result<Subscriber<'_, CriticalSectionRawMutex, MotionPhase, 1, 8, 0>, pubsub::Error>
    {
        self.0.subscribe()
    }
}

impl Extract for MotionStatus<'static> {
    type Out<'a> = MotionStatus<'a>;

    fn extract(ctx: &ModeCtx) -> MotionStatus<'_> {
        MotionStatus(&ctx.motion)
    }
}

/// Permission to drive the pattern engine.
///
/// Derefs to [`PatternRunner`]. Running it also needs a [`Motion`].
pub struct Patterns<'a>(&'a PatternRunner);

impl Deref for Patterns<'_> {
    type Target = PatternRunner;

    fn deref(&self) -> &PatternRunner {
        self.0
    }
}

impl Extract for Patterns<'static> {
    type Out<'a> = Patterns<'a>;

    fn extract(ctx: &ModeCtx) -> Patterns<'_> {
        Patterns(&ctx.patterns)
    }
}
