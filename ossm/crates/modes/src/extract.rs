use core::ops::Deref;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};
use ossm::{MotionPhase, MotionSender, MotionState};
use pattern_engine::{PatternEvent, PatternRunner, PatternSender, PatternState};

use crate::ModeCtx;
use crate::input::{InputKind, InputKinds, InputSlot, Received};
use crate::remote::{Inputs, RemoteId};
use crate::sealed;

/// A capability a mode can declare as an argument.
///
/// Implemented on the `'static` form of each extractor type (e.g.
/// `Motion<'static>`), which only serves as a name for the type. The
/// value a mode actually receives is [`Out<'a>`](Self::Out), borrowed
/// from the supervisor for the duration of the run.
///
/// Sealed: the extractors in this crate are the only capabilities a mode
/// can ask for, until custom extractors are supported.
pub trait Extract: sealed::Sealed {
    type Out<'a>;

    /// What a mode holding this extractor subscribes to. Most extractors
    /// are plain capabilities and subscribe to nothing.
    const SUBSCRIBES: Subscriptions = Subscriptions::NONE;

    fn extract(ctx: &ModeCtx) -> Self::Out<'_>;
}

/// Traffic from remotes that the supervisor routes to a mode while it
/// runs, rather than turning it away or handling it itself.
///
/// A mode's subscriptions are those of its extractors, combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subscriptions {
    /// Input kinds remotes may send.
    inputs: InputKinds,
    /// Whether the mode handles a remote losing its link itself, rather
    /// than the supervisor falling back to Idle.
    lost: bool,
}

impl Subscriptions {
    pub(crate) const NONE: Self = Self {
        inputs: InputKinds::NONE,
        lost: false,
    };

    /// Remotes losing their link, handled by the mode.
    pub(crate) const LOST: Self = Self {
        inputs: InputKinds::NONE,
        lost: true,
    };

    /// Input of these kinds from remotes.
    pub(crate) const fn inputs(inputs: InputKinds) -> Self {
        Self {
            inputs,
            lost: false,
        }
    }

    pub(crate) const fn accepts(self) -> InputKinds {
        self.inputs
    }

    pub(crate) const fn handles_lost(self) -> bool {
        self.lost
    }

    /// Both sets of subscriptions. Fails to build if both take the same
    /// input kind, since each kind has one slot.
    pub(crate) const fn union(self, other: Self) -> Self {
        assert!(
            !self.inputs.intersects(other.inputs),
            "a mode can take at most one `Input` per input kind",
        );
        Self {
            inputs: self.inputs.union(other.inputs),
            lost: self.lost || other.lost,
        }
    }
}

/// Permission to move the motor.
pub struct Motion<'a>(&'a MotionSender);

impl Deref for Motion<'_> {
    type Target = MotionSender;

    fn deref(&self) -> &MotionSender {
        self.0
    }
}

impl sealed::Sealed for Motion<'static> {}

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

impl sealed::Sealed for MotionStatus<'static> {}

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
    /// Send a playback event to the running engine.
    pub fn send(&self, event: PatternEvent) {
        self.sender.send(event);
    }

    /// Replace the engine's state.
    pub fn set(&self, state: PatternState) {
        self.sender.set(state);
    }
}

impl Deref for Patterns<'_> {
    type Target = PatternRunner;

    fn deref(&self) -> &PatternRunner {
        self.runner
    }
}

impl sealed::Sealed for Patterns<'static> {}

impl Extract for Patterns<'static> {
    type Out<'a> = Patterns<'a>;

    fn extract(ctx: &ModeCtx) -> Patterns<'_> {
        Patterns {
            runner: &ctx.patterns,
            sender: ctx.pattern_sender,
        }
    }
}

/// Input of kind `K` sent by remotes while the mode runs.
///
/// Holding one is what lets remotes reach the mode at all: input of a
/// kind no extractor declares is rejected. Updates are merged into the
/// kind's state, latest-wins. Events are queued; a mode must keep calling
/// [`next`](Self::next), or remotes sending events will wait.
///
/// Each kind has its own slot, so a mode can take one `Input` per kind
/// and `select` over them. Taking the same kind twice fails to build.
pub struct Input<'a, K: InputKind> {
    slot: &'a InputSlot<K>,
}

impl<K: InputKind> Input<'_, K> {
    /// The current state, with every update received so far.
    pub fn state(&self) -> K::State {
        self.slot.state()
    }

    /// Update the state as a remote would, e.g. to bring speed back to 0.
    pub fn update(&self, update: K::Update) {
        self.slot.update(update);
    }

    /// Wait for the state to change or an event to arrive.
    pub async fn next(&self) -> Received<K> {
        self.slot.next().await
    }
}

impl<K: InputKind> sealed::Sealed for Input<'static, K> {}

impl<K: InputKind> Extract for Input<'static, K> {
    type Out<'a> = Input<'a, K>;

    const SUBSCRIBES: Subscriptions = Subscriptions::inputs(K::KIND);

    fn extract(ctx: &ModeCtx) -> Input<'_, K> {
        Input {
            slot: K::slot(ctx.inputs),
        }
    }
}

/// Remotes losing their link while the mode runs.
///
/// A mode holding one decides what losing a remote means, but must call
/// [`LostEvent::handled`] within 100ms of receiving the event. If it
/// doesn't, or a mode doesn't hold one, the supervisor switches to
/// [`Idle`](crate::ModeKey::Idle).
pub struct RemoteLost<'a> {
    inputs: &'a Inputs,
}

impl RemoteLost<'_> {
    pub async fn next(&self) -> LostEvent<'_> {
        let source = self.inputs.lost.wait().await;
        LostEvent {
            source,
            inputs: self.inputs,
        }
    }
}

impl sealed::Sealed for RemoteLost<'static> {}

impl Extract for RemoteLost<'static> {
    type Out<'a> = RemoteLost<'a>;

    const SUBSCRIBES: Subscriptions = Subscriptions::LOST;

    fn extract(ctx: &ModeCtx) -> RemoteLost<'_> {
        RemoteLost { inputs: ctx.inputs }
    }
}

/// A remote lost its link. Dropping it without calling
/// [`handled`](Self::handled) leaves the supervisor's fallback in place.
#[must_use = "call `handled` once the loss is dealt with"]
pub struct LostEvent<'a> {
    pub source: RemoteId,
    inputs: &'a Inputs,
}

impl LostEvent<'_> {
    pub fn handled(self) {
        self.inputs.lost_ack.signal(());
    }
}
