#![no_std]

use embedded_hal_async::delay::DelayNs;
use embedded_io::{ErrorType, Read, Write};
use heapless::Vec;
use rmodbus::{ModbusProto, client::ModbusRequest, guess_response_frame_len};
use crc::{Crc, CRC_16_MODBUS};
use ossm::{Motor, MotorTelemetry};

const MODBUS_CRC: Crc<u16> = Crc::<u16>::new(&CRC_16_MODBUS);

const DEVICE_ADDR: u8 = 0x01;
const SET_ABSOLUTE_POSITION_FUNC: u8 = 0x7b;
const PROTO: ModbusProto = ModbusProto::Rtu;
const MIN_REG_READ_REQUIRED: usize = 3;
const MAX_REG_READ_AT_ONCE: usize = 8;

// Retry limit for blocking reads before declaring a timeout.
// Each retry is a tight loop iteration; the motor typically responds within a few hundred iterations.
const MOTOR_TIMEOUT_RETRIES: usize = 500;
const RETRY_DELAY_US: u32 = 20; // 500 retries × 20µs ≈ 10ms total timeout
const INTER_COMMAND_DELAY_US: u32 = 2_000;
const HOME_STEP_THRESHOLD: i32 = 15;
const HOME_SPEED_RPM: u16 = 80;
const HOME_MAX_OUTPUT: u16 = 89;
const HOME_POLL_INTERVAL_MS: u32 = 50;

// Settle time after homing completes, before re-enabling Modbus.
const POST_HOME_SETTLE_MS: u32 = 20;

// Settle time after re-enabling Modbus. The M57AIM resets speed/output
// defaults when Modbus is toggled and needs time to stabilise.
const POST_MODBUS_ENABLE_SETTLE_MS: u32 = 800;

// Operating settings restored after homing. These configure the motor's internal
// closed-loop tracking - Ruckig controls actual machine speed by issuing
// position steps, so these are effectively "go as fast as commanded".
const OPERATING_SPEED_RPM: u16 = 3000;
const OPERATING_ACCELERATION: u16 = 50000;
const OPERATING_MAX_OUTPUT: u16 = 600;

pub const MAX_MOTOR_SPEED_RPM: u16 = 3000;

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u16)]
pub enum ReadWriteMotorRegisters {
    ModbusEnable = 0x00,
    DriverOutputEnable = 0x01,
    MotorTargetSpeed = 0x02,
    MotorAcceleration = 0x03,
    WeakMagneticAngle = 0x04,
    SpeedRingProportionalCoefficient = 0x05,
    SpeedLoopIntegrationTime = 0x06,
    PositionRingProportionalCoefficient = 0x07,
    SpeedFeedForward = 0x08,
    DirPolarity = 0x09,
    ElectronicGearNumerator = 0x0A,
    ElectronicGearDenominator = 0x0B,
    ParameterSaveFlag = 0x14,
    AbsolutePositionLowU16 = 0x16,
    AbsolutePositionHighU16 = 0x17,
    StandstillMaxOutput = 0x18,
    SpecificFunction = 0x19,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u16)]
pub enum ReadOnlyMotorRegisters {
    TargetPositionLowU16 = 0x0C,
    TargetPositionHighU16 = 0x0D,
    AlarmCode = 0x0E,
    SystemCurrent = 0x0F,
    MotorCurrentSpeed = 0x10,
    SystemVoltage = 0x11,
    SystemTemperature = 0x12,
    SystemOutputPwm = 0x13,
    DeviceAddress = 0x15,
}

pub trait ReadableMotorRegister {
    fn addr(&self) -> u16;
}

impl ReadableMotorRegister for ReadWriteMotorRegisters {
    fn addr(&self) -> u16 {
        *self as u16
    }
}

impl ReadableMotorRegister for ReadOnlyMotorRegisters {
    fn addr(&self) -> u16 {
        *self as u16
    }
}

#[derive(Debug)]
pub enum MotorError<E> {
    UartError(E),
    Timeout,
}

pub struct M57AIMMotor<UART, DELAY> {
    uart: UART,
    delay: DELAY,
}

impl<UART, DELAY> From<(UART, DELAY)> for M57AIMMotor<UART, DELAY>
where
    UART: Read + Write,
    DELAY: DelayNs,
{
    fn from((uart, delay): (UART, DELAY)) -> Self {
        Self::new(uart, delay)
    }
}

impl<UART, DELAY> M57AIMMotor<UART, DELAY>
where
    UART: Read + Write,
    DELAY: DelayNs,
{
    pub fn new(uart: UART, delay: DELAY) -> Self {
        Self { uart, delay }
    }

    pub fn release(self) -> (UART, DELAY) {
        (self.uart, self.delay)
    }

    async fn read_exact(
        &mut self,
        buf: &mut [u8],
    ) -> Result<(), MotorError<<UART as ErrorType>::Error>> {
        let mut remaining = buf;
        let mut retries = 0;
        while !remaining.is_empty() {
            match self.uart.read(remaining) {
                Ok(0) => {
                    retries += 1;
                    if retries >= MOTOR_TIMEOUT_RETRIES {
                        return Err(MotorError::Timeout);
                    }
                    self.delay.delay_us(RETRY_DELAY_US).await;
                }
                Ok(n) => {
                    retries = 0;
                    remaining = &mut remaining[n..];
                }
                Err(e) => return Err(MotorError::UartError(e)),
            }
        }
        Ok(())
    }

    pub async fn write_register(
        &mut self,
        reg: &ReadWriteMotorRegisters,
        val: u16,
    ) -> Result<(), MotorError<<UART as ErrorType>::Error>> {
        let mut modbus_req = ModbusRequest::new(1, PROTO);
        let mut request: Vec<u8, 32> = Vec::new();

        modbus_req
            .generate_set_holding(reg.addr(), val, &mut request)
            .expect("Failed to generate reg write request");

        self.uart
            .write_all(&request)
            .map_err(MotorError::UartError)?;
        self.uart.flush().map_err(MotorError::UartError)?;

        let mut response = [0u8; 32];
        self.read_exact(&mut response[0..MIN_REG_READ_REQUIRED])
            .await?;

        let len = guess_response_frame_len(&response[0..MIN_REG_READ_REQUIRED], PROTO)
            .expect("Failed to guess frame len") as usize;
        if len > MIN_REG_READ_REQUIRED {
            self.read_exact(&mut response[MIN_REG_READ_REQUIRED..len])
                .await?;
        }
        let response = &response[0..len];

        modbus_req.parse_ok(response).expect("Modbus error");

        self.delay.delay_us(INTER_COMMAND_DELAY_US).await;

        Ok(())
    }

    pub async fn read_registers<T: ReadableMotorRegister>(
        &mut self,
        reg: &T,
        count: u16,
    ) -> Result<Vec<u16, MAX_REG_READ_AT_ONCE>, MotorError<<UART as ErrorType>::Error>> {
        let mut modbus_req = ModbusRequest::new(1, PROTO);
        let mut request: Vec<u8, 32> = Vec::new();

        modbus_req
            .generate_get_holdings(reg.addr(), count, &mut request)
            .expect("Failed to generate reg read request");

        self.uart
            .write_all(&request)
            .map_err(MotorError::UartError)?;
        self.uart.flush().map_err(MotorError::UartError)?;

        let mut response = [0u8; 32];
        self.read_exact(&mut response[0..MIN_REG_READ_REQUIRED])
            .await?;

        let len = guess_response_frame_len(&response[0..MIN_REG_READ_REQUIRED], PROTO)
            .expect("Failed to guess frame len") as usize;
        if len > MIN_REG_READ_REQUIRED {
            self.read_exact(&mut response[MIN_REG_READ_REQUIRED..len])
                .await?;
        }
        let response = &response[0..len];

        let mut res: Vec<u16, MAX_REG_READ_AT_ONCE> = Vec::new();
        modbus_req
            .parse_u16(response, &mut res)
            .expect("Failed to parse response reg");

        self.delay.delay_us(INTER_COMMAND_DELAY_US).await;

        Ok(res)
    }

    pub async fn read_register<T: ReadableMotorRegister>(
        &mut self,
        reg: &T,
    ) -> Result<u16, MotorError<<UART as ErrorType>::Error>> {
        Ok(self.read_registers(reg, 1).await?[0])
    }

    async fn get_remaining_steps_blocking(
        &mut self,
    ) -> Result<i32, MotorError<<UART as ErrorType>::Error>> {
        let regs = self
            .read_registers(&ReadOnlyMotorRegisters::TargetPositionLowU16, 2)
            .await?;
        let bytes = (((regs[1] as u32) << 16) | regs[0] as u32).to_le_bytes();
        Ok(i32::from_le_bytes(bytes))
    }
}

impl<UART, DELAY> Motor for M57AIMMotor<UART, DELAY>
where
    UART: Read + Write,
    <UART as ErrorType>::Error: core::fmt::Debug,
    DELAY: DelayNs,
{
    type Error = MotorError<<UART as ErrorType>::Error>;

    const STEPS_PER_REV: u32 = 32768;
    const MAX_OUTPUT: u16 = OPERATING_MAX_OUTPUT;

    async fn enable(&mut self) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::ModbusEnable, 1)
            .await
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::ModbusEnable, 0)
            .await
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        // Configure homing speed and current limit
        self.write_register(&ReadWriteMotorRegisters::MotorTargetSpeed, HOME_SPEED_RPM)
            .await?;
        self.write_register(
            &ReadWriteMotorRegisters::StandstillMaxOutput,
            HOME_MAX_OUTPUT,
        )
        .await?;

        // Trigger hardware homing
        self.write_register(&ReadWriteMotorRegisters::SpecificFunction, 1)
            .await?;

        // Poll until remaining steps drops below threshold
        loop {
            self.delay.delay_ms(HOME_POLL_INTERVAL_MS).await;
            let remaining = self.get_remaining_steps_blocking().await?;
            if remaining.abs() < HOME_STEP_THRESHOLD {
                break;
            }
        }

        // Let the motor settle after homing completes
        self.delay.delay_ms(POST_HOME_SETTLE_MS).await;

        // Re-enable modbus — M57AIM homing resets it
        self.enable().await?;

        // Modbus re-enable resets speed/output defaults — let it settle
        self.delay.delay_ms(POST_MODBUS_ENABLE_SETTLE_MS).await;

        // Restore operating settings
        self.set_speed(OPERATING_SPEED_RPM).await?;
        self.set_acceleration(OPERATING_ACCELERATION).await?;
        self.set_max_output(OPERATING_MAX_OUTPUT).await?;

        Ok(())
    }

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        let mut request = [0u8; 8];
        request[0] = DEVICE_ADDR;
        request[1] = SET_ABSOLUTE_POSITION_FUNC;
        request[2..6].copy_from_slice(&steps.to_be_bytes());
        let crc = MODBUS_CRC.checksum(&request[..6]).to_le_bytes();
        request[6..8].copy_from_slice(&crc);

        self.uart
            .write_all(&request)
            .map_err(MotorError::UartError)?;
        self.uart.flush().map_err(MotorError::UartError)?;

        let mut response = [0u8; 8];
        self.read_exact(&mut response).await?;

        self.delay.delay_us(INTER_COMMAND_DELAY_US).await;

        Ok(())
    }

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::MotorTargetSpeed, rpm)
            .await
    }

    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::MotorAcceleration, value)
            .await
    }

    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::StandstillMaxOutput, output)
            .await
    }
}

impl<UART, DELAY> MotorTelemetry for M57AIMMotor<UART, DELAY>
where
    UART: Read + Write,
    <UART as ErrorType>::Error: core::fmt::Debug,
    DELAY: DelayNs,
{
    type Error = MotorError<<UART as ErrorType>::Error>;

    async fn get_absolute_position(&mut self) -> Result<i32, Self::Error> {
        let regs = self
            .read_registers(&ReadWriteMotorRegisters::AbsolutePositionLowU16, 2)
            .await?;
        let bytes = (((regs[1] as u32) << 16) | regs[0] as u32).to_le_bytes();
        Ok(i32::from_le_bytes(bytes))
    }

    async fn get_remaining_steps(&mut self) -> Result<i32, Self::Error> {
        self.get_remaining_steps_blocking().await
    }

    async fn get_speed(&mut self) -> Result<u16, Self::Error> {
        self.read_register(&ReadWriteMotorRegisters::MotorTargetSpeed)
            .await
    }

    async fn get_acceleration(&mut self) -> Result<u16, Self::Error> {
        self.read_register(&ReadWriteMotorRegisters::MotorAcceleration)
            .await
    }

    async fn get_max_output(&mut self) -> Result<u16, Self::Error> {
        self.read_register(&ReadWriteMotorRegisters::StandstillMaxOutput)
            .await
    }

    async fn get_current_amps(&mut self) -> Result<f32, Self::Error> {
        let reg = self
            .read_register(&ReadOnlyMotorRegisters::SystemCurrent)
            .await?;
        Ok(reg as f32 / 2000.0)
    }

    async fn get_voltage_volts(&mut self) -> Result<f32, Self::Error> {
        let reg = self
            .read_register(&ReadOnlyMotorRegisters::SystemVoltage)
            .await?;
        Ok(reg as f32 / 327.0)
    }
}
