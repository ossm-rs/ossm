use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::{self, Subscriber};
use embassy_time::{Duration, Instant, Timer};
use ossm::fault::{self, Fault};
use pattern_engine::commands::PatternCommand;
use pattern_engine::{EngineState, PatternInput};

use crate::{ModeSet, Supervisor};

/// How long [`RemoteHandle::switch`] waits for the requested mode before
/// giving up. Leaving a mode can include a full deceleration.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);
const SWITCH_POLL: Duration = Duration::from_millis(10);
/// Requests of each kind the mailbox holds before senders wait.
const QUEUE_DEPTH: usize = 8;

/// Which remote a request came from.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteId {
    Ble,
    M5,
}

/// Something a remote asks the active mode to do.
///
/// Each variant is an input kind with its own queue in the mailbox. To
/// add a kind, add a variant here, a bit in [`InputKinds`], a queue in
/// `Queues` and an [`Input`] impl; the compiler points at the rest.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum Request {
    Pattern(PatternCommand),
}

/// Why a remote's request was not delivered.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The active mode does not take this kind of request.
    NotAccepted,
    /// A fault is latched; nothing is accepted until restart.
    Faulted,
    /// Another switch was requested before this one took effect.
    Superseded,
    /// The supervisor did not reach the requested mode in time.
    TimedOut,
}

/// Set of request kinds a mode accepts, derived from its
/// [`Commands`](crate::Commands) extractors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputKinds(u8);

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

/// A request payload a mode can receive through
/// [`Commands`](crate::Commands).
pub trait Input: Sized {
    const KIND: InputKinds;

    /// This kind's queue in the mailbox.
    #[doc(hidden)]
    fn queue(mailbox: &Mailbox) -> &InputQueue<Self>;
}

impl Input for PatternCommand {
    const KIND: InputKinds = InputKinds::PATTERN;

    fn queue(mailbox: &Mailbox) -> &InputQueue<Self> {
        &mailbox.queues.pattern
    }
}

/// A request and the remote that sent it.
#[derive(Clone, Copy, Debug)]
pub struct Envelope<T> {
    pub(crate) source: RemoteId,
    pub(crate) input: T,
}

/// Requests of one kind waiting for the active mode.
pub type InputQueue<T> = Channel<CriticalSectionRawMutex, Envelope<T>, QUEUE_DEPTH>;

/// One queue per input kind, so a mode taking several kinds reads each
/// without seeing the others.
struct Queues {
    pattern: InputQueue<PatternCommand>,
}

impl Queues {
    const fn new() -> Self {
        Self {
            pattern: Channel::new(),
        }
    }

    fn clear(&self) {
        // Destructured so a new queue can't be left out.
        let Self { pattern } = self;
        pattern.clear();
    }
}

/// Requests from remotes on their way to the active mode.
///
/// Only accepts request kinds the active mode declared. Cleared each time
/// a mode starts so nothing sent to one mode reaches the next.
///
/// Named after the actor-model mailbox (Erlang, Akka, actix): messages
/// addressed to whichever mode is running, each in an [`Envelope`] that
/// records who sent it.
pub struct Mailbox {
    queues: Queues,
    accepts: AtomicU8,
}

impl Mailbox {
    pub(crate) const fn new() -> Self {
        Self {
            queues: Queues::new(),
            accepts: AtomicU8::new(InputKinds::NONE.0),
        }
    }

    pub(crate) fn open(&self, accepts: InputKinds) {
        self.queues.clear();
        self.accepts.store(accepts.0, Ordering::Relaxed);
    }

    pub(crate) fn close(&self) {
        self.accepts.store(InputKinds::NONE.0, Ordering::Relaxed);
    }

    fn accepts(&self) -> InputKinds {
        InputKinds(self.accepts.load(Ordering::Relaxed))
    }

    async fn deliver<T: Input>(&self, source: RemoteId, input: T) -> Result<(), Rejected> {
        if !self.accepts().contains(T::KIND) {
            return Err(Rejected::NotAccepted);
        }
        T::queue(self).send(Envelope { source, input }).await;
        Ok(())
    }
}

/// What remotes can show about the machine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status<M> {
    pub mode: M,
    pub fault: Option<Fault>,
    pub engine: EngineState,
    pub input: PatternInput,
}

/// The one handle remote adapters hold.
///
/// Requests go through [`send`](Self::send) and are accepted or rejected
/// immediately based on the active mode. Adapters decide what to report
/// back to their remote; a third-party remote may need a rejection
/// swallowed to stay compatible.
pub struct RemoteHandle<M: ModeSet> {
    pub(crate) sup: &'static Supervisor<M>,
}

impl<M: ModeSet> RemoteHandle<M> {
    /// Deliver a request to the active mode.
    ///
    /// Waits only if the active mode has fallen behind on this kind's
    /// queue.
    pub async fn send(&self, source: RemoteId, request: Request) -> Result<(), Rejected> {
        if fault::raised().is_some() {
            return Err(Rejected::Faulted);
        }
        let mailbox = &self.sup.mailbox;
        match request {
            Request::Pattern(command) => mailbox.deliver(source, command).await,
        }
    }

    /// Switch to `mode` and wait until it is running.
    ///
    /// Requests are latest-wins across remotes: if another switch is
    /// requested before this one takes effect, this returns
    /// [`Rejected::Superseded`]. Requesting the mode that is already
    /// running leaves it running. Requests for [`ModeSet::FAULT`] are
    /// rejected; only a raised fault enters it.
    pub async fn switch(&self, mode: M) -> Result<(), Rejected> {
        let sup = self.sup;
        if fault::raised().is_some() {
            return Err(Rejected::Faulted);
        }
        if mode == M::FAULT {
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
            if current == M::FAULT {
                return Err(Rejected::Faulted);
            }
            if sup.request_seq.load(Ordering::Relaxed) != ticket {
                return Err(Rejected::Superseded);
            }
            if Instant::now() >= deadline {
                return Err(Rejected::TimedOut);
            }
        }
    }

    pub fn status(&self) -> Status<M> {
        let patterns = self.sup.patterns;
        Status {
            mode: self.sup.current(),
            fault: fault::raised(),
            engine: patterns.state(),
            input: patterns.input(),
        }
    }

    /// Subscribe to pattern engine state transitions, as a cue to re-read
    /// [`status`](Self::status).
    ///
    /// Returns `Err` if all subscriber slots are in use.
    pub fn subscribe_engine(
        &self,
    ) -> Result<Subscriber<'static, CriticalSectionRawMutex, EngineState, 1, 8, 0>, pubsub::Error>
    {
        self.sup.patterns.subscribe()
    }
}
