//! Typed event bus.
//!
//! `events` is the single demux point for messages flowing between
//! transports (BLE, ESP-NOW, UART, web) and the rest of the program.
//! Each transport decodes its native protocol into typed events and
//! publishes them through this crate. Whoever cares about those events
//! subscribes; whoever doesn't, ignores.
//!
//! Adding a new event like this:
//!
//! ```ignore
//! #[derive(Copy, Clone, Debug)]
//! pub enum PatternCmd {
//!     Play(usize),
//!     Pause,
//!     // ...
//! }
//! events::declare_event!(PatternCmd);
//!
//! // Publishing (transport side):
//! events::publish(PatternCmd::Play(3));
//!
//! // Subscribing (consumer side):
//! let mut cmds = events::subscribe::<PatternCmd>();
//! loop {
//!     let cmd = cmds.next().await;
//!     // ...
//! }
//! ```
#![no_std]

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber, WaitResult};

/// Per-event-type slot count.
///
/// How many in-flight events a single subscriber can fall behind by
/// before older events get dropped on its behalf. Sized for human-rate
/// input - knob updates, button presses, mode switches.
pub const SLOT_COUNT: usize = 8;

/// Per-event-type subscriber cap.
///
/// Maximum number of concurrent subscribers across the whole program for
/// a given event type.
pub const SUBSCRIBER_COUNT: usize = 4;

/// A typed event that flows through the input bus.
///
/// Use [`declare_event!`] to implement this trait and allocate the
/// backing static channel in one step.
pub trait Event: Copy + 'static {
    fn channel() -> &'static EventChannel<Self>;
}

/// Backing storage for a single event type's pubsub.
///
/// Constructed once as a `static` per event type via [`declare_event!`].
/// Not constructed by hand outside the macro.
pub struct EventChannel<E: Clone + 'static> {
    inner: PubSubChannel<CriticalSectionRawMutex, E, SLOT_COUNT, SUBSCRIBER_COUNT, 0>,
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
    sub: Subscriber<'static, CriticalSectionRawMutex, E, SLOT_COUNT, SUBSCRIBER_COUNT, 0>,
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
/// Drop-oldest is the right semantic for human-rate input: a stale depth
/// value is worse than a missed one.
pub fn publish<E: Event>(event: E) {
    E::channel()
        .inner
        .immediate_publisher()
        .publish_immediate(event);
}

/// Subscribe to events of type `E`.
///
/// Panics if the per-event-type subscriber cap ([`SUBSCRIBER_COUNT`]) is
/// exhausted. This is a programming error; raise the cap if a real use
/// case demands more concurrent subscribers.
pub fn subscribe<E: Event>() -> Subscription<E> {
    let sub = E::channel()
        .inner
        .subscriber()
        .expect("events: too many subscribers - raise SUBSCRIBER_COUNT");
    Subscription { sub }
}

/// Declare a type as an [`Event`] and allocate its backing channel.
///
/// Expands to a hidden `static EventChannel<T>` plus an `impl Event for T`.
/// Place at module scope alongside the type definition. Calling twice
/// for the same type fails to compile (duplicate `impl`).
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
