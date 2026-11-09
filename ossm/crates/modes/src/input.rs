use core::cell::Cell;
use core::fmt::Debug;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use pattern_engine::{PatternEvent, PatternState, PatternUpdate};

use crate::remote::{Inputs, RemoteId};
use crate::sealed;

/// Events of each kind `Inputs` holds before senders wait.
const EVENT_QUEUE_DEPTH: usize = 8;

/// Set of input kinds a mode accepts, derived from its
/// [`Input`](crate::Input) extractors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputKinds(pub(crate) u8);

impl InputKinds {
    pub const NONE: Self = Self(0);
    pub const PATTERN: Self = Self(1 << 0);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

/// A kind of input remotes send and a mode takes with
/// [`Input`](crate::Input).
///
/// Each kind has a state that updates are merged into, latest-wins, so
/// a remote sending them quickly can't fill anything up, and events that
/// are queued in order. To add a kind, implement this, add a bit in
/// [`InputKinds`], a slot in `Slots` and a [`Request`](crate::Request)
/// variant; the compiler points at the rest. Sealed, since those all
/// live in this crate.
pub trait InputKind: sealed::Sealed + Copy + Debug + 'static {
    /// The whole state, e.g. speed, depth and stroke.
    type State: Copy + Debug + PartialEq;
    /// A change to one part of the state.
    type Update: Copy + Debug;
    /// Something every one of which counts, in order.
    type Event: Copy + Debug;

    const KIND: InputKinds;
    const INITIAL: Self::State;

    fn apply(state: &mut Self::State, update: Self::Update);

    /// Adjust the state as a mode starts. Does nothing by default.
    fn on_open(_state: &mut Self::State) {}

    /// This kind's slot in `Inputs`.
    #[doc(hidden)]
    fn slot(inputs: &Inputs) -> &InputSlot<Self>;
}

/// The pattern engine's input: its state and playback events.
#[derive(Clone, Copy, Debug)]
pub struct PatternInput;

impl sealed::Sealed for PatternInput {}

impl InputKind for PatternInput {
    type State = PatternState;
    type Update = PatternUpdate;
    type Event = PatternEvent;

    const KIND: InputKinds = InputKinds::PATTERN;
    const INITIAL: PatternState = PatternState::DEFAULT;

    fn apply(state: &mut PatternState, update: PatternUpdate) {
        state.apply(update);
    }

    /// Every mode starts still, so the next pattern starts at speed 0.
    fn on_open(state: &mut PatternState) {
        state.speed = 0.0;
    }

    fn slot(inputs: &Inputs) -> &InputSlot<Self> {
        &inputs.slots.pattern
    }
}

/// Something a remote sends for input kind `K`.
#[derive(Clone, Copy, Debug)]
pub enum Message<K: InputKind> {
    Event(K::Event),
    Update(K::Update),
}

/// What [`Input::next`](crate::Input::next) waits for.
#[derive(Clone, Copy, Debug)]
pub enum Received<K: InputKind> {
    /// The state changed; this is all of it, after the change.
    State(K::State),
    /// An event, and the remote that sent it.
    Event { source: RemoteId, event: K::Event },
}

/// A queued event and the remote that sent it.
#[derive(Clone, Copy, Debug)]
pub struct Envelope<T> {
    source: RemoteId,
    event: T,
}

/// One input kind's part of `Inputs`: its state and its event queue.
pub struct InputSlot<K: InputKind> {
    state: Mutex<CriticalSectionRawMutex, Cell<K::State>>,
    changed: Signal<CriticalSectionRawMutex, ()>,
    events: Channel<CriticalSectionRawMutex, Envelope<K::Event>, EVENT_QUEUE_DEPTH>,
}

impl<K: InputKind> InputSlot<K> {
    const fn new() -> Self {
        Self {
            state: Mutex::new(Cell::new(K::INITIAL)),
            changed: Signal::new(),
            events: Channel::new(),
        }
    }

    /// Called as each mode starts. The state is kept, adjusted by
    /// [`InputKind::on_open`]; queued events are dropped.
    fn open(&self) {
        self.modify(K::on_open);
        self.changed.reset();
        self.events.clear();
    }

    pub(crate) fn state(&self) -> K::State {
        self.state.lock(Cell::get)
    }

    pub(crate) fn update(&self, update: K::Update) {
        if self.modify(|state| K::apply(state, update)) {
            self.changed.signal(());
        }
    }

    pub(crate) async fn push(&self, source: RemoteId, event: K::Event) {
        self.events.send(Envelope { source, event }).await;
    }

    /// State changes come first, so an event is never acted on with a
    /// state older than one already received.
    pub(crate) async fn next(&self) -> Received<K> {
        match select(self.changed.wait(), self.events.receive()).await {
            Either::First(()) => Received::State(self.state()),
            Either::Second(envelope) => Received::Event {
                source: envelope.source,
                event: envelope.event,
            },
        }
    }

    /// Returns whether the state changed.
    fn modify(&self, f: impl FnOnce(&mut K::State)) -> bool {
        self.state.lock(|cell| {
            let old = cell.get();
            let mut new = old;
            f(&mut new);
            cell.set(new);
            new != old
        })
    }
}

/// One slot per input kind, so a mode taking several kinds reads each
/// without seeing the others.
pub(crate) struct Slots {
    pub(crate) pattern: InputSlot<PatternInput>,
}

impl Slots {
    pub(crate) const fn new() -> Self {
        Self {
            pattern: InputSlot::new(),
        }
    }

    pub(crate) fn open(&self) {
        // Destructured so a new slot can't be left out.
        let Self { pattern } = self;
        pattern.open();
    }
}
