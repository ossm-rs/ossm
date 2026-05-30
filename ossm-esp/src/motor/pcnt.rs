use core::cell::RefCell;
use core::sync::atomic::{AtomicI32, Ordering};

use critical_section::Mutex;
use esp_hal::{
    gpio::interconnect::PeripheralInput,
    handler,
    interrupt::Priority,
    pcnt::{
        channel::{CtrlMode, EdgeMode},
        unit::{Counter, Unit},
        Pcnt,
    },
};
use ossm::PositionFeedback;

/// Watermarks are placed symmetrically around zero, well inside the
/// i16 counter range. When the hardware counter hits +HIGH_LIMIT or
/// -HIGH_LIMIT it resets itself to 0 and fires an interrupt; the ISR
/// folds that same amount into the extension counter so the
/// composite position is continuous.
const HIGH_LIMIT: i16 = 30_000;
const LOW_LIMIT: i16 = -30_000;

/// PCNT unit 0 is the only one we hand out today. If we ever need a
/// second motor we promote this module to be generic over the unit.
static UNIT: Mutex<RefCell<Option<Unit<'static, 0>>>> = Mutex::new(RefCell::new(None));

/// Extension counter managed by the ISR. Combined with the
/// hardware counter to form the full i32 position.
static EXTENSION: AtomicI32 = AtomicI32::new(0);

/// Hardware-counted position source.
///
/// Wires the STEP output signal into PCNT's edge input and the DIR
/// output signal into PCNT's control input. The hardware counts every
/// rising edge of STEP, with the count direction determined by DIR.
/// Watermark interrupts extend the 16-bit hardware counter into a
/// full i32 transparently.
///
/// The motion-task caller only ever sees the [`PositionFeedback`]
/// trait; the wrap-extension math lives entirely behind this type.
pub struct PcntPositionCounter {
    counter: Counter<'static, 0>,
}

impl PcntPositionCounter {
    /// Build a counter from the PCNT peripheral and the two signals to
    /// observe. `step_signal` should be the STEP line (rising edges
    /// trigger counting); `dir_signal` should be the DIR line (its
    /// level selects increment vs decrement).
    ///
    /// Consumes the whole `Pcnt`; only unit 0 is used. If we need to use
    /// others in the future, refactor to take a single Pcnt or expose them
    pub fn new(
        mut pcnt: Pcnt<'static>,
        step_signal: impl PeripheralInput<'static>,
        dir_signal: impl PeripheralInput<'static>,
    ) -> Self {
        pcnt.set_interrupt_handler(on_pcnt);

        let unit = pcnt.unit0;
        unit.set_low_limit(Some(LOW_LIMIT))
            .expect("LOW_LIMIT is negative");
        unit.set_high_limit(Some(HIGH_LIMIT))
            .expect("HIGH_LIMIT is positive");
        unit.clear();

        let channel = &unit.channel0;
        channel.set_edge_signal(step_signal);
        channel.set_ctrl_signal(dir_signal);
        // DIR low reverses the edge mode, DIR high keeps it; so with
        // base "increment on rising, hold on falling" the unit counts
        // up while DIR is high and down while DIR is low. This must
        // match the convention in StepDirMotor::set_absolute_position.
        channel.set_input_mode(EdgeMode::Hold, EdgeMode::Increment);
        channel.set_ctrl_mode(CtrlMode::Reverse, CtrlMode::Keep);

        unit.listen();
        unit.resume();

        let counter = unit.counter.clone();
        critical_section::with(|cs| {
            UNIT.borrow_ref_mut(cs).replace(unit);
        });

        Self { counter }
    }
}

impl PositionFeedback for PcntPositionCounter {
    fn position(&self) -> i32 {
        // Triple-read: extension, counter, extension. A mismatch means
        // the ISR fired between our two extension loads, so the
        // counter value we captured doesn't belong to either snapshot.
        // Retry until we see a consistent pair.
        loop {
            let before = EXTENSION.load(Ordering::Acquire);
            let raw = self.counter.get() as i32;
            let after = EXTENSION.load(Ordering::Acquire);
            if before == after {
                return before + raw;
            }
        }
    }

    fn reset(&mut self, value: i32) {
        // Homing is the only caller and the motor is stationary while
        // it runs, so the small window between clear() and the
        // extension store is safe. We still pause+resume to keep the
        // hardware from latching a stray edge mid-reset.
        critical_section::with(|cs| {
            if let Some(unit) = UNIT.borrow_ref(cs).as_ref() {
                unit.pause();
                unit.clear();
                EXTENSION.store(value, Ordering::Release);
                unit.resume();
            }
        });
    }

    fn applied(&mut self, _delta: i32) {
        // PCNT observes the pulses directly; software bookkeeping is
        // unused.
    }
}

/// Run the pcnt interrupt on very high priority, it executes very quickly
/// and is important for positional correctness
#[handler(priority = Priority::Priority3)]
fn on_pcnt() {
    critical_section::with(|cs| {
        if let Some(unit) = UNIT.borrow_ref(cs).as_ref() {
            if !unit.interrupt_is_set() {
                return;
            }
            let events = unit.events();
            if events.high_limit {
                EXTENSION.fetch_add(HIGH_LIMIT as i32, Ordering::Release);
            } else if events.low_limit {
                EXTENSION.fetch_add(LOW_LIMIT as i32, Ordering::Release);
            }
            unit.reset_interrupt();
        }
    });
}
