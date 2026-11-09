use core::ops::Deref;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};
use ossm::{MotionPhase, MotionSender, MotionState};
use pattern_engine::commands::PatternCommand;
use pattern_engine::{PatternRunner, PatternSender};

use crate::ModeCtx;
use crate::remote::{Input, InputKinds, InputQueue, RemoteId};

/// A capability a mode can declare as an argument.
///
/// Implemented on the `'static` form of each extractor type (e.g.
/// `Motion<'static>`), which only serves as a name for the type. The
/// value a mode actually receives is [`Out<'a>`](Self::Out), borrowed
/// from the [`ModeCtx`] for the duration of the run.
pub trait Extract {
    type Out<'a>;

    /// Request kinds remotes may send to a mode holding this extractor.
    const ACCEPTS: InputKinds = InputKinds::NONE;

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
pub struct Patterns<'a> {
    runner: &'a PatternRunner,
    sender: &'a PatternSender,
}

impl Patterns<'_> {
    /// Apply a command to the running engine.
    pub fn apply(&self, command: PatternCommand) {
        self.sender.apply(command);
    }
}

impl Deref for Patterns<'_> {
    type Target = PatternRunner;

    fn deref(&self) -> &PatternRunner {
        self.runner
    }
}

impl Extract for Patterns<'static> {
    type Out<'a> = Patterns<'a>;

    fn extract(ctx: &ModeCtx) -> Patterns<'_> {
        Patterns {
            runner: &ctx.patterns,
            sender: ctx.pattern_sender,
        }
    }
}

/// Requests of kind `T` sent by remotes while the mode runs.
///
/// Holding one is what lets remotes reach the mode at all: requests of a
/// kind no extractor declares are rejected before they are queued. A
/// mode holding `Commands` must keep calling [`next`](Self::next), or
/// remotes sending that kind will wait.
///
/// Each kind has its own queue, so a mode can take one `Commands` per
/// kind and `select` over them. Taking the same kind twice fails to
/// build.
pub struct Commands<'a, T> {
    queue: &'a InputQueue<T>,
}

impl<T: Input> Commands<'_, T> {
    /// Wait for the next request and the remote it came from.
    pub async fn next(&self) -> (RemoteId, T) {
        let envelope = self.queue.receive().await;
        (envelope.source, envelope.input)
    }
}

impl<T: Input + 'static> Extract for Commands<'static, T> {
    type Out<'a> = Commands<'a, T>;

    const ACCEPTS: InputKinds = T::KIND;

    fn extract(ctx: &ModeCtx) -> Commands<'_, T> {
        Commands {
            queue: T::queue(ctx.mailbox),
        }
    }
}
