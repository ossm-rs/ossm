use core::ops::Deref;

use ossm::{MotionObserver, MotionSender};

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
pub struct MotionStatus<'a>(&'a MotionObserver);

impl Deref for MotionStatus<'_> {
    type Target = MotionObserver;

    fn deref(&self) -> &MotionObserver {
        self.0
    }
}

impl Extract for MotionStatus<'static> {
    type Out<'a> = MotionStatus<'a>;

    fn extract(ctx: &ModeCtx) -> MotionStatus<'_> {
        MotionStatus(&ctx.motion_status)
    }
}
