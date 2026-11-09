use alloc::boxed::Box;
use core::fmt::Debug;
use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{self, Subscriber};
use embassy_time::{Duration, Instant, Timer};
use pattern_engine::{EnginePhase, PatternEvent, PatternState, PatternUpdate};

use crate::input::{InputKind, InputKinds, Message, PatternInput, Slots};
use crate::supervisor::Supervisor;
use crate::ModeKey;

/// How long [`RemoteHandle::switch`] waits for the requested mode before
/// giving up. Leaving a mode can include a full deceleration.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);
const SWITCH_POLL: Duration = Duration::from_millis(10);

/// Which remote a request came from, named by what the remote is rather
/// than how it connects.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteId {
    /// The RADR wireless remote app, over BLE.
    Radr,
    /// The M5 remote, over ESP-NOW.
    M5,
}

/// What a remote may do, fixed when its handle is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    sends: InputKinds,
    switches: bool,
}

impl Scope {
    /// Nothing: no input and no mode switches.
    pub const fn new() -> Self {
        Self {
            sends: InputKinds::NONE,
            switches: false,
        }
    }

    /// Also allow input of kind `K`.
    pub const fn sends<K: InputKind>(self) -> Self {
        Self {
            sends: self.sends.union(K::KIND),
            ..self
        }
    }

    /// Also allow switching modes.
    pub const fn switches(self) -> Self {
        Self {
            switches: true,
            ..self
        }
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

/// Something a remote asks the active mode to do, by input kind.
///
/// Remotes rarely build one directly: [`RemoteHandle::send`] takes
/// anything that converts, such as a [`PatternEvent`] or
/// [`PatternUpdate`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum Request {
    Pattern(Message<PatternInput>),
}

impl Request {
    fn kind(&self) -> InputKinds {
        match self {
            Request::Pattern(_) => PatternInput::KIND,
        }
    }
}

impl From<PatternEvent> for Request {
    fn from(event: PatternEvent) -> Self {
        Request::Pattern(Message::Event(event))
    }
}

impl From<PatternUpdate> for Request {
    fn from(update: PatternUpdate) -> Self {
        Request::Pattern(Message::Update(update))
    }
}

/// Why a remote's request was not delivered.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The remote's scope does not include this.
    NotAllowed,
    /// The active mode does not take this kind of request, or the
    /// requested mode is not registered.
    NotAccepted,
    /// Another switch was requested before this one took effect.
    Superseded,
    /// The supervisor did not reach the requested mode in time.
    TimedOut,
}

/// The active mode's inputs from remotes.
///
/// Only accepts input kinds the active mode declared. For each kind it
/// keeps the latest state, updated in place so a flood of updates never
/// queues, and a queue of events, each in an envelope that records which
/// remote sent it. Queued events are dropped each time a mode starts so
/// nothing sent to one mode reaches the next.
pub struct Inputs {
    pub(crate) slots: Slots,
    accepts: AtomicU8,
}

impl Inputs {
    pub(crate) const fn new() -> Self {
        Self {
            slots: Slots::new(),
            accepts: AtomicU8::new(InputKinds::NONE.0),
        }
    }

    pub(crate) fn open(&self, accepts: InputKinds) {
        self.slots.open();
        self.accepts.store(accepts.0, Ordering::Relaxed);
    }

    pub(crate) fn close(&self) {
        self.accepts.store(InputKinds::NONE.0, Ordering::Relaxed);
    }

    fn accepts(&self) -> InputKinds {
        InputKinds(self.accepts.load(Ordering::Relaxed))
    }

    async fn deliver<K: InputKind>(
        &self,
        source: RemoteId,
        message: Message<K>,
    ) -> Result<(), Rejected> {
        if !self.accepts().contains(K::KIND) {
            return Err(Rejected::NotAccepted);
        }
        let slot = K::slot(self);
        match message {
            Message::Event(event) => slot.push(source, event).await,
            Message::Update(update) => slot.update(update),
        }
        Ok(())
    }
}

/// What remotes can show about the machine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status<K> {
    pub mode: ModeKey<K>,
    pub engine: EnginePhase,
    pub pattern: PatternState,
}

/// Where the platform gets each remote's handle from.
///
/// Given only to the platform, which hands each remote adapter a
/// [`RemoteHandle`] scoped to what that remote may do.
pub struct Remotes<K: 'static> {
    pub(crate) sup: &'static Supervisor<K>,
}

impl<K> Remotes<K>
where
    K: Copy + PartialEq + Debug + 'static,
{
    /// A handle for the remote `id`, limited to `scope`.
    ///
    /// Call once per remote at boot: each handle lives for the rest of
    /// the program.
    pub fn handle(&self, id: RemoteId, scope: Scope) -> &'static RemoteHandle<K> {
        Box::leak(Box::new(RemoteHandle {
            sup: self.sup,
            id,
            scope,
        }))
    }
}

/// The handle a remote adapter holds.
///
/// Requests go through [`send`](Self::send) and are accepted or rejected
/// immediately, based on the remote's [`Scope`] and the active mode.
/// Adapters decide what to report back to their remote; a third-party
/// remote may need a rejection swallowed to stay compatible.
pub struct RemoteHandle<K: 'static> {
    sup: &'static Supervisor<K>,
    id: RemoteId,
    scope: Scope,
}

impl<K> RemoteHandle<K>
where
    K: Copy + PartialEq + Debug + 'static,
{
    /// The remote this handle belongs to.
    pub fn id(&self) -> RemoteId {
        self.id
    }

    /// Deliver a request to the active mode.
    ///
    /// Updates are merged straight into the kind's state and never wait.
    /// Events wait only if the active mode has fallen behind on them.
    pub async fn send(&self, request: impl Into<Request>) -> Result<(), Rejected> {
        let request = request.into();
        if !self.scope.sends.contains(request.kind()) {
            return Err(Rejected::NotAllowed);
        }
        let inputs = &self.sup.inputs;
        match request {
            Request::Pattern(message) => inputs.deliver(self.id, message).await,
        }
    }

    /// Switch to `mode` and wait until it is running.
    ///
    /// Requests are latest-wins across remotes: if another switch is
    /// requested before this one takes effect, this returns
    /// [`Rejected::Superseded`]. Requesting the mode that is already
    /// running leaves it running. Modes the firmware did not register
    /// are rejected.
    pub async fn switch(&self, mode: impl Into<ModeKey<K>>) -> Result<(), Rejected> {
        if !self.scope.switches {
            return Err(Rejected::NotAllowed);
        }
        let sup = self.sup;
        let mode = mode.into();
        if !sup.is_registered(mode) {
            return Err(Rejected::NotAccepted);
        }

        // Always signal, even if `mode` looks current: the supervisor may
        // already be leaving it for an earlier request.
        let ticket = sup.request_seq.fetch_add(1, Ordering::Relaxed) + 1;
        sup.request.signal(mode);

        let deadline = Instant::now() + SWITCH_TIMEOUT;
        loop {
            Timer::after(SWITCH_POLL).await;
            let current = sup.current();
            let settled = !sup.leaving.load(Ordering::Relaxed) && !sup.request.signaled();
            if current == mode && settled {
                return Ok(());
            }
            if sup.request_seq.load(Ordering::Relaxed) != ticket {
                return Err(Rejected::Superseded);
            }
            if Instant::now() >= deadline {
                return Err(Rejected::TimedOut);
            }
        }
    }

    pub fn status(&self) -> Status<K> {
        let patterns = self.sup.patterns;
        Status {
            mode: self.sup.current(),
            engine: patterns.phase(),
            pattern: patterns.state(),
        }
    }

    /// Subscribe to pattern engine phase transitions, as a cue to re-read
    /// [`status`](Self::status).
    ///
    /// Returns `Err` if all subscriber slots are in use.
    pub fn subscribe_engine(
        &self,
    ) -> Result<Subscriber<'static, CriticalSectionRawMutex, EnginePhase, 1, 8, 0>, pubsub::Error>
    {
        self.sup.patterns.subscribe()
    }
}
