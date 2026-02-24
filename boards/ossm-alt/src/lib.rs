#![no_std]

mod rs485;

pub use rs485::Rs485;

use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    peripherals::Peripherals,
    timer::timg::TimerGroup,
    uart::{Config, Uart},
};
use sossm::{Board, MechanicalConfig, Motor};

const MOTOR_BAUD_RATE: u32 = 115_200;

pub struct OssmAltBoard<M: Motor> {
    motor: M,
    config: MechanicalConfig,
}

impl<M> OssmAltBoard<M>
where
    M: Motor + From<(Rs485<Uart<'static, Blocking>, Output<'static>>, Delay)>,
{
    pub fn new(p: Peripherals, config: MechanicalConfig) -> Self {
        let timg0 = TimerGroup::new(p.TIMG0);
        esp_rtos::start(timg0.timer0);

        let uart_config = Config::default().with_baudrate(MOTOR_BAUD_RATE);
        let uart = Uart::new(p.UART1, uart_config)
            .expect("Failed to initialize UART")
            .with_tx(p.GPIO10)
            .with_rx(p.GPIO12);

        // Manual DE control — hardware RS485 mode has inverted RTS polarity
        // on the OSSM Alt board, so we toggle a GPIO directly instead.
        let de = Output::new(p.GPIO11, Level::Low, OutputConfig::default());
        let rs485 = Rs485::new(uart, de);

        let delay = Delay::new();

        Self {
            motor: M::from((rs485, delay)),
            config,
        }
    }
}

impl<M: Motor> Board for OssmAltBoard<M> {
    type Motor = M;

    fn mechanical_config(&self) -> &MechanicalConfig {
        &self.config
    }

    fn into_motor(self) -> M {
        self.motor
    }
}
