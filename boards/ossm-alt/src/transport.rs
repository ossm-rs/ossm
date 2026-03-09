use embedded_hal_async::delay::DelayNs;
use embedded_io::{ErrorType, Read, Write};
use heapless::Vec;
use m57aim::ModbusTransport;

const MOTOR_TIMEOUT_RETRIES: usize = 500;
const RETRY_DELAY_US: u32 = 20;
const INTER_COMMAND_DELAY_US: u32 = 2_000;
const MIN_FRAME_BYTES: usize = 3;
const MAX_REGS_PER_READ: usize = 8;

#[derive(Debug)]
pub enum TransportError<E: core::fmt::Debug> {
    Uart(E),
    Timeout,
    Protocol(&'static str),
}

/// ModbusTransport over RS485 UART.
///
/// Handles RTU framing, CRC, and response parsing.
pub struct Rs485ModbusTransport<UART, DELAY> {
    uart: UART,
    delay: DELAY,
}

impl<UART, DELAY> Rs485ModbusTransport<UART, DELAY>
where
    UART: Read + Write,
    DELAY: DelayNs,
{
    pub fn new(uart: UART, delay: DELAY) -> Self {
        Self { uart, delay }
    }

    async fn read_exact(
        &mut self,
        buf: &mut [u8],
    ) -> Result<(), TransportError<<UART as ErrorType>::Error>> {
        let mut remaining = buf;
        let mut retries = 0;
        while !remaining.is_empty() {
            match self.uart.read(remaining) {
                Ok(0) => {
                    retries += 1;
                    if retries >= MOTOR_TIMEOUT_RETRIES {
                        return Err(TransportError::Timeout);
                    }
                    self.delay.delay_us(RETRY_DELAY_US).await;
                }
                Ok(n) => {
                    retries = 0;
                    remaining = &mut remaining[n..];
                }
                Err(e) => return Err(TransportError::Uart(e)),
            }
        }
        Ok(())
    }

    async fn read_response(
        &mut self,
        buf: &mut [u8],
    ) -> Result<usize, TransportError<<UART as ErrorType>::Error>> {
        self.read_exact(&mut buf[0..MIN_FRAME_BYTES]).await?;
        let len = rmodbus::guess_response_frame_len(
            &buf[0..MIN_FRAME_BYTES],
            rmodbus::ModbusProto::Rtu,
        )
        .map_err(|_| TransportError::Protocol("failed to guess frame length"))?
            as usize;
        if len > MIN_FRAME_BYTES {
            self.read_exact(&mut buf[MIN_FRAME_BYTES..len]).await?;
        }
        Ok(len)
    }

    async fn send_and_receive(
        &mut self,
        request: &[u8],
        response_buf: &mut [u8],
    ) -> Result<usize, TransportError<<UART as ErrorType>::Error>> {
        self.uart.write_all(request).map_err(TransportError::Uart)?;
        self.uart.flush().map_err(TransportError::Uart)?;
        let len = self.read_response(response_buf).await?;
        self.delay.delay_us(INTER_COMMAND_DELAY_US).await;
        Ok(len)
    }
}

impl<UART, DELAY> ModbusTransport for Rs485ModbusTransport<UART, DELAY>
where
    UART: Read + Write,
    <UART as ErrorType>::Error: core::fmt::Debug,
    DELAY: DelayNs,
{
    type Error = TransportError<<UART as ErrorType>::Error>;

    async fn write_holding(
        &mut self,
        device_addr: u8,
        register: u16,
        value: u16,
    ) -> Result<(), Self::Error> {
        let mut modbus_req =
            rmodbus::client::ModbusRequest::new(device_addr, rmodbus::ModbusProto::Rtu);
        let mut request: Vec<u8, 32> = Vec::new();
        modbus_req
            .generate_set_holding(register, value, &mut request)
            .map_err(|_| TransportError::Protocol("failed to generate write request"))?;
        let mut response = [0u8; 32];
        let len = self.send_and_receive(&request, &mut response).await?;
        modbus_req
            .parse_ok(&response[..len])
            .map_err(|_| TransportError::Protocol("write response parse error"))?;
        Ok(())
    }

    async fn read_holding(
        &mut self,
        device_addr: u8,
        register: u16,
        count: u16,
    ) -> Result<Vec<u16, MAX_REGS_PER_READ>, Self::Error> {
        let mut modbus_req =
            rmodbus::client::ModbusRequest::new(device_addr, rmodbus::ModbusProto::Rtu);
        let mut request: Vec<u8, 32> = Vec::new();
        modbus_req
            .generate_get_holdings(register, count, &mut request)
            .map_err(|_| TransportError::Protocol("failed to generate read request"))?;
        let mut response = [0u8; 32];
        let len = self.send_and_receive(&request, &mut response).await?;
        let mut result: Vec<u16, MAX_REGS_PER_READ> = Vec::new();
        modbus_req
            .parse_u16(&response[..len], &mut result)
            .map_err(|_| TransportError::Protocol("read response parse error"))?;
        Ok(result)
    }

    async fn raw_transaction(
        &mut self,
        request: &[u8],
        response: &mut [u8],
    ) -> Result<usize, Self::Error> {
        use crc::{Crc, CRC_16_MODBUS};
        const MODBUS_CRC: Crc<u16> = Crc::<u16>::new(&CRC_16_MODBUS);
        let mut frame: Vec<u8, 32> = Vec::new();
        frame.extend_from_slice(request).ok();
        let crc = MODBUS_CRC.checksum(request).to_le_bytes();
        frame.extend_from_slice(&crc).ok();
        self.uart.write_all(&frame).map_err(TransportError::Uart)?;
        self.uart.flush().map_err(TransportError::Uart)?;
        let expected_len = response.len();
        self.read_exact(&mut response[..expected_len]).await?;
        self.delay.delay_us(INTER_COMMAND_DELAY_US).await;
        Ok(expected_len)
    }
}
