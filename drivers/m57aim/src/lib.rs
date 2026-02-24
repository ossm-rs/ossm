#![no_std]

use embedded_hal::delay::DelayNs;
use embedded_io::{ErrorType, Read, Write};
use heapless::Vec;
use rmodbus::{ModbusProto, client::ModbusRequest, guess_response_frame_len};
use sossm::{Motor, MotorTelemetry};

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

// Taken from the rmodbus crate
fn calc_crc16(frame: &[u8], data_length: u8) -> u16 {
    let mut crc: u16 = 0xffff;
    for i in frame.iter().take(data_length as usize) {
        crc ^= u16::from(*i);
        for _ in (0..8).rev() {
            if (crc & 0x0001) == 0 {
                crc >>= 1;
            } else {
                crc >>= 1;
                crc ^= 0xA001;
            }
        }
    }
    crc
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

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), MotorError<<UART as ErrorType>::Error>> {
        let mut remaining = buf;
        let mut retries = 0;
        while !remaining.is_empty() {
            match self.uart.read(remaining) {
                Ok(0) => {
                    retries += 1;
                    if retries >= MOTOR_TIMEOUT_RETRIES {
                        return Err(MotorError::Timeout);
                    }
                    self.delay.delay_us(RETRY_DELAY_US);
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

    pub fn write_register(
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
        self.read_exact(&mut response[0..MIN_REG_READ_REQUIRED])?;

        let len = guess_response_frame_len(&response[0..MIN_REG_READ_REQUIRED], PROTO)
            .expect("Failed to guess frame len") as usize;
        if len > MIN_REG_READ_REQUIRED {
            self.read_exact(&mut response[MIN_REG_READ_REQUIRED..len])?;
        }
        let response = &response[0..len];

        modbus_req.parse_ok(response).expect("Modbus error");

        self.delay.delay_us(INTER_COMMAND_DELAY_US);

        Ok(())
    }

    pub fn read_registers<T: ReadableMotorRegister>(
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
        self.read_exact(&mut response[0..MIN_REG_READ_REQUIRED])?;

        let len = guess_response_frame_len(&response[0..MIN_REG_READ_REQUIRED], PROTO)
            .expect("Failed to guess frame len") as usize;
        if len > MIN_REG_READ_REQUIRED {
            self.read_exact(&mut response[MIN_REG_READ_REQUIRED..len])?;
        }
        let response = &response[0..len];

        let mut res: Vec<u16, MAX_REG_READ_AT_ONCE> = Vec::new();
        modbus_req
            .parse_u16(response, &mut res)
            .expect("Failed to parse response reg");

        self.delay.delay_us(INTER_COMMAND_DELAY_US);

        Ok(res)
    }

    pub fn read_register<T: ReadableMotorRegister>(
        &mut self,
        reg: &T,
    ) -> Result<u16, MotorError<<UART as ErrorType>::Error>> {
        Ok(self.read_registers(reg, 1)?[0])
    }

    fn get_remaining_steps_blocking(
        &mut self,
    ) -> Result<i32, MotorError<<UART as ErrorType>::Error>> {
        let regs = self.read_registers(&ReadOnlyMotorRegisters::TargetPositionLowU16, 2)?;
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

    fn enable(&mut self) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::ModbusEnable, 1)?;
        self.write_register(&ReadWriteMotorRegisters::DriverOutputEnable, 1)
    }

    fn disable(&mut self) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::DriverOutputEnable, 0)?;
        self.write_register(&ReadWriteMotorRegisters::ModbusEnable, 0)
    }

    fn home(&mut self) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::MotorTargetSpeed, HOME_SPEED_RPM)?;
        self.write_register(
            &ReadWriteMotorRegisters::StandstillMaxOutput,
            HOME_MAX_OUTPUT,
        )?;
        self.write_register(&ReadWriteMotorRegisters::SpecificFunction, 1)?;

        loop {
            let remaining = self.get_remaining_steps_blocking()?;
            if remaining.abs() < HOME_STEP_THRESHOLD {
                break;
            }
            self.delay.delay_us(INTER_COMMAND_DELAY_US * 2);
        }

        // Re-enable modbus - homing resets it to defaults
        self.write_register(&ReadWriteMotorRegisters::ModbusEnable, 1)?;

        Ok(())
    }

    fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        let mut request = [0u8; 8];
        let bytes = steps.to_be_bytes();

        request[0] = 0x1;
        request[1] = 0x7b;
        request[2..6].copy_from_slice(&bytes);
        let crc = calc_crc16(&request[0..6], 6).to_le_bytes();
        request[6..8].copy_from_slice(&crc);

        self.uart
            .write_all(&request)
            .map_err(MotorError::UartError)?;
        self.uart.flush().map_err(MotorError::UartError)?;

        let mut response = [0u8; 8];
        self.read_exact(&mut response)?;

        Ok(())
    }

    fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::MotorTargetSpeed, rpm)
    }

    fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::MotorAcceleration, value)
    }

    fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error> {
        self.write_register(&ReadWriteMotorRegisters::StandstillMaxOutput, output)
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
        let regs = self.read_registers(&ReadWriteMotorRegisters::AbsolutePositionLowU16, 2)?;
        let bytes = (((regs[1] as u32) << 16) | regs[0] as u32).to_le_bytes();
        Ok(i32::from_le_bytes(bytes))
    }

    async fn get_remaining_steps(&mut self) -> Result<i32, Self::Error> {
        self.get_remaining_steps_blocking()
    }

    async fn get_speed(&mut self) -> Result<u16, Self::Error> {
        self.read_register(&ReadWriteMotorRegisters::MotorTargetSpeed)
    }

    async fn get_acceleration(&mut self) -> Result<u16, Self::Error> {
        self.read_register(&ReadWriteMotorRegisters::MotorAcceleration)
    }

    async fn get_max_output(&mut self) -> Result<u16, Self::Error> {
        self.read_register(&ReadWriteMotorRegisters::StandstillMaxOutput)
    }

    async fn get_current_amps(&mut self) -> Result<f32, Self::Error> {
        let reg = self.read_register(&ReadOnlyMotorRegisters::SystemCurrent)?;
        Ok(reg as f32 / 2000.0)
    }

    async fn get_voltage_volts(&mut self) -> Result<f32, Self::Error> {
        let reg = self.read_register(&ReadOnlyMotorRegisters::SystemVoltage)?;
        Ok(reg as f32 / 327.0)
    }
}
