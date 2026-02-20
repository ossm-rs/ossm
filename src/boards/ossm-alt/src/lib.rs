#![no_std]

use esp_hal::{
    Async,
    peripherals::Peripherals,
    timer::timg::TimerGroup,
    uart::{Config, Uart},
};
use sossm::Board;

const MOTOR_BAUD_RATE: u32 = 115_200;

pub struct OssmAltBoard {
    pub uart: Uart<'static, Async>,
}

impl OssmAltBoard {
    pub fn new(p: Peripherals) -> Self {
        // Initialize the embassy time driver using TIMG0.
        let timg0 = TimerGroup::new(p.TIMG0);
        esp_rtos::start(timg0.timer0);

        let config = Config::default().with_baudrate(MOTOR_BAUD_RATE);
        let uart = Uart::new(p.UART1, config)
            .expect("Failed to initialize UART")
            .with_tx(p.GPIO10)
            .with_rx(p.GPIO12)
            .with_rts(p.GPIO11)
            .into_async();

        // Enable RS485 half-duplex mode — hardware drives DE high before TX
        // and low after the stop bit automatically (dl1_en).
        let regs = esp_hal::peripherals::UART1::regs();
        regs.rs485_conf()
            .modify(|_, w| w.rs485_en().set_bit().dl1_en().set_bit());

        Self { uart }
    }
}

impl Board for OssmAltBoard {}
