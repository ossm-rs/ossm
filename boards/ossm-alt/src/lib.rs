#![no_std]

mod rs485;

pub use rs485::Rs485;

use embassy_time::Delay;
use esp_hal::{
    Blocking,
    gpio::{Level, Output, OutputConfig},
    peripherals::{GPIO10, GPIO11, GPIO12, UART1},
    uart::{Config, Uart},
};
use ossm::{Board, MechanicalConfig, Motor};

const MOTOR_BAUD_RATE: u32 = 115_200;

pub struct OssmAltBoard<M: Motor> {
    motor: M,
    config: MechanicalConfig,
}

impl<M> OssmAltBoard<M>
where
    M: Motor + From<(Rs485<Uart<'static, Blocking>, Output<'static>>, Delay)>,
{
    pub fn new(
        uart1: UART1<'static>,
        tx_pin: GPIO10<'static>,
        rx_pin: GPIO12<'static>,
        de_pin: GPIO11<'static>,
        config: MechanicalConfig,
    ) -> Self {
        let uart_config = Config::default().with_baudrate(MOTOR_BAUD_RATE);
        let uart = Uart::new(uart1, uart_config)
            .expect("Failed to initialize UART")
            .with_tx(tx_pin)
            .with_rx(rx_pin);

        // Manual DE control — hardware RS485 mode has inverted RTS polarity
        // on the OSSM Alt board, so we toggle a GPIO directly instead.
        let de = Output::new(de_pin, Level::Low, OutputConfig::default());
        let rs485 = Rs485::new(uart, de);

        let delay = Delay;

        Self {
            motor: M::from((rs485, delay)),
            config,
        }
    }
}

impl<M: Motor> OssmAltBoard<M> {
    pub fn mechanical_config(&self) -> &MechanicalConfig {
        &self.config
    }
}

impl<M: Motor> Board for OssmAltBoard<M> {
    type Error = M::Error;

    const STEPS_PER_REV: u32 = M::STEPS_PER_REV;

    async fn enable(&mut self) -> Result<(), Self::Error> {
        self.motor.enable().await
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.motor.disable().await
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        self.motor.home().await
    }

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        self.motor.set_absolute_position(steps).await
    }

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error> {
        self.motor.set_speed(rpm).await
    }

    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error> {
        self.motor.set_acceleration(value).await
    }

    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error> {
        self.motor.set_max_output(output).await
    }
}
