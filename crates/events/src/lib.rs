//! Typed event bus.
//!
//! Two primitives, side by side:
//!
//! - [`State`]: typed last-value cell. Latched, readable on demand,
//!   subscribable. For **facts** like the current engine state or
//!   knob input — values where last-write-wins and older ones are
//!   stale.
//!
//! - [`Event`]: fire-and-forget typed message with a per-subscriber
//!   queue. For **things that happened** like `MotionFaulted` or
//!   `SwitchMode(idx)` — missing one is meaningful, order matters.
//!
//! Each type gets its own static channel allocated by a `declare_*!`
//! macro at the call site, in whichever crate owns the type. The
//! events crate provides the storage primitives and lookup; it does
//! not know about any specific type.

#![no_std]

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber, WaitResult};
use embassy_sync::watch::{Receiver, Watch};

// ---------------------------------------------------------------------------
// State — typed last-value cell.
// ---------------------------------------------------------------------------

/// Per-state-type subscriber cap.
///
/// Maximum number of concurrent change-stream subscribers across the
/// whole program for a given state type.
pub const STATE_SUBSCRIBERS: usize = 8;

/// A typed last-value cell.
///
/// Use [`declare_state!`] to implement this trait and allocate the
/// backing static cell.
pub trait State: Clone + 'static {
    fn cell() -> &'static StateCell<Self>;
}

/// Backing storage for a single state type.
///
/// Constructed once as a `static` per type via [`declare_state!`], with
/// a declared initial value so [`StateHandle::read`] always succeeds.
pub struct StateCell<S: Clone + 'static> {
    watch: Watch<CriticalSectionRawMutex, S, STATE_SUBSCRIBERS>,
}

impl<S: Clone + 'static> StateCell<S> {
    pub const fn new_with(initial: S) -> Self {
        Self {
            watch: Watch::new_with(initial),
        }
    }
}

/// Look up the state handle for type `S`.
///
/// Cheap — returns a thin wrapper around the static cell.
pub fn state<S: State>() -> StateHandle<S> {
    StateHandle { cell: S::cell() }
}

/// Handle to a state cell.
///
/// Provides read/write/subscribe operations. Created via [`state`].
pub struct StateHandle<S: Clone + 'static> {
    cell: &'static StateCell<S>,
}

impl<S: Clone + 'static> StateHandle<S> {
    /// Read the current value.
    ///
    /// Always succeeds because cells are declared with an initial value.
    pub fn read(&self) -> S {
        self.cell
            .watch
            .try_get()
            .expect("events: state cell missing initial value")
    }

    /// Write a new value, waking every subscriber.
    pub fn write(&self, value: S) {
        self.cell.watch.sender().send(value);
    }

    /// Mutate the current value in place and wake every subscriber.
    ///
    /// The closure may be called more than once if the watch's internal
    /// retry path triggers; keep mutations idempotent.
    pub fn update<F: Fn(&mut S)>(&self, f: F) {
        self.cell.watch.sender().send_modify(|opt| {
            if let Some(value) = opt {
                f(value);
            }
        });
    }

    /// Subscribe to changes.
    ///
    /// The returned [`StateSubscription`] yields the current value the
    /// first time [`StateSubscription::changed`] is awaited, then yields
    /// each subsequent write.
    ///
    /// Panics if all subscriber slots ([`STATE_SUBSCRIBERS`]) are in use.
    pub fn subscribe(&self) -> StateSubscription<S> {
        let receiver = self
            .cell
            .watch
            .receiver()
            .expect("events: too many state subscribers - raise STATE_SUBSCRIBERS");
        StateSubscription { receiver }
    }
}

/// Active subscription to changes of a state cell.
///
/// Drop the subscription to release its subscriber slot.
pub struct StateSubscription<S: Clone + 'static> {
    receiver: Receiver<'static, CriticalSectionRawMutex, S, STATE_SUBSCRIBERS>,
}

impl<S: Clone + 'static> StateSubscription<S> {
    /// Wait for the next change to the cell.
    ///
    /// The first call yields the current value of the cell; subsequent
    /// calls yield each write.
    pub async fn changed(&mut self) -> S {
        self.receiver.changed().await
    }
}

/// Declare a type as a [`State`] and allocate its backing cell.
///
/// Expands to a hidden `static StateCell<T>` plus an `impl State for T`.
/// Place at module scope alongside the type definition. The initial
/// value is what readers see before any [`write`](StateHandle::write).
/// Calling twice for the same type fails to compile (duplicate `impl`).
#[macro_export]
macro_rules! declare_state {
    ($ty:ty, $initial:expr) => {
        const _: () = {
            static CELL: $crate::StateCell<$ty> = $crate::StateCell::new_with($initial);
            impl $crate::State for $ty {
                fn cell() -> &'static $crate::StateCell<Self> {
                    &CELL
                }
            }
        };
    };
}

// ---------------------------------------------------------------------------
// Event — typed fire-and-forget pubsub.
// ---------------------------------------------------------------------------

/// Per-event-type slot count.
///
/// How many in-flight events a single subscriber can fall behind by
/// before older events get dropped. Sized for human-rate input — knob
/// updates, button presses, mode switches.
pub const EVENT_QUEUE: usize = 8;

/// Per-event-type subscriber cap.
pub const EVENT_SUBSCRIBERS: usize = 4;

/// A typed fire-and-forget event.
///
/// Use [`declare_event!`] to implement this trait and allocate the
/// backing static channel in one step.
pub trait Event: Copy + 'static {
    fn channel() -> &'static EventChannel<Self>;
}

/// Backing storage for a single event type.
///
/// Constructed once as a `static` per type via [`declare_event!`].
pub struct EventChannel<E: Clone + 'static> {
    inner: PubSubChannel<CriticalSectionRawMutex, E, EVENT_QUEUE, EVENT_SUBSCRIBERS, 0>,
}

impl<E: Clone + 'static> EventChannel<E> {
    pub const fn new() -> Self {
        Self {
            inner: PubSubChannel::new(),
        }
    }
}

impl<E: Clone + 'static> Default for EventChannel<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Active subscription to events of type `E`.
///
/// Drop the subscription to release its subscriber slot.
pub struct Subscription<E: Event> {
    sub: Subscriber<'static, CriticalSectionRawMutex, E, EVENT_QUEUE, EVENT_SUBSCRIBERS, 0>,
}

impl<E: Event> Subscription<E> {
    /// Wait for the next event of type `E`.
    pub async fn next(&mut self) -> E {
        loop {
            match self.sub.next_message().await {
                WaitResult::Message(e) => return e,
                WaitResult::Lagged(n) => {
                    log::warn!(
                        "events: subscription for {} lagged by {} events",
                        core::any::type_name::<E>(),
                        n,
                    );
                }
            }
        }
    }
}

/// Publish an event to all current subscribers of its type.
///
/// Drops the oldest queued event for any subscriber whose slot is full.
pub fn publish<E: Event>(event: E) {
    E::channel()
        .inner
        .immediate_publisher()
        .publish_immediate(event);
}

/// Subscribe to events of type `E`.
///
/// Panics if the per-event-type subscriber cap ([`EVENT_SUBSCRIBERS`])
/// is exhausted.
pub fn subscribe<E: Event>() -> Subscription<E> {
    let sub = E::channel()
        .inner
        .subscriber()
        .expect("events: too many subscribers - raise EVENT_SUBSCRIBERS");
    Subscription { sub }
}

/// Declare a type as an [`Event`] and allocate its backing channel.
///
/// Expands to a hidden `static EventChannel<T>` plus an `impl Event for T`.
#[macro_export]
macro_rules! declare_event {
    ($ty:ty) => {
        const _: () = {
            static CHANNEL: $crate::EventChannel<$ty> = $crate::EventChannel::new();
            impl $crate::Event for $ty {
                fn channel() -> &'static $crate::EventChannel<Self> {
                    &CHANNEL
                }
            }
        };
    };
}
