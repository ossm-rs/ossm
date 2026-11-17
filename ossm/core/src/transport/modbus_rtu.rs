use embassy_time::Instant;
use embedded_hal_async::delay::DelayNs;
use embedded_io::{ErrorType, Write};
use heapless::Vec;

use super::ModbusTransport;

const RESPONSE_TIMEOUT_MS: u64 = 100;
const MAX_RETRIES: usize = 3;
const POLL_DELAY_US: u32 = 20;
const INTER_COMMAND_DELAY_US: u32 = 2_000;
const MIN_FRAME_BYTES: usize = 3;
const MAX_REGS_PER_READ: usize = 8;

#[derive(Debug)]
pub enum TransportError<E: core::fmt::Debug> {
    Uart(E),
    Timeout,
    /// Wire corruption: garbled header, CRC mismatch, etc. Retryable.
    Corrupt(&'static str),
    /// Logic/programming error: failed to build request, buffer too small. Fatal.
    Protocol(&'static str),
}

/// Non-blocking read: returns 0 immediately when no data is available.
/// Required for timeout support - `embedded_io::Read` on blocking UARTs
/// hangs forever waiting for data, making timeouts impossible.
pub trait ReadNonBlocking: ErrorType {
    fn read_nb(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
}

/// Change the UART's baud rate in place.
///
/// Used for one-shot device provisioning - e.g. switching to a motor's
/// factory baud to issue a baud-change register sequence. MCU-specific
/// implementations live in the firmware layer; this trait lets the
/// motor driver request a reconfigure without knowing how to do it.
#[allow(async_fn_in_trait)]
pub trait UartReconfigure {
    type Error: core::fmt::Debug;

    /// Set the UART baud rate. Returns once the hardware is configured
    /// and ready to send/receive at the new rate.
    async fn reconfigure_baud(&mut self, baud: u32) -> Result<(), Self::Error>;
}

/// ModbusTransport over RS485 UART.
///
/// Handles RTU framing, CRC, response parsing, and automatic retries
/// on timeout.
pub struct Rs485ModbusTransport<UART, DELAY> {
    uart: UART,
    delay: DELAY,
}

impl<UART, DELAY> Rs485ModbusTransport<UART, DELAY>
where
    UART: ReadNonBlocking + Write,
    DELAY: DelayNs,
{
    pub fn new(uart: UART, delay: DELAY) -> Self {
        Self { uart, delay }
    }

    /// Fill `buf[*filled..end]`, with a deadline. `filled` advances as
    /// bytes arrive so callers can report partial frames on failure.
    async fn read_exact(
        &mut self,
        buf: &mut [u8],
        filled: &mut usize,
        end: usize,
        deadline: Instant,
    ) -> Result<(), TransportError<<UART as ErrorType>::Error>> {
        while *filled < end {
            if Instant::now() > deadline {
                return Err(TransportError::Timeout);
            }
            match self.uart.read_nb(&mut buf[*filled..end]) {
                Ok(0) => self.delay.delay_us(POLL_DELAY_US).await,
                Ok(n) => *filled += n,
                Err(e) => return Err(TransportError::Uart(e)),
            }
        }
        Ok(())
    }

    /// Drain any stale bytes from the RX FIFO.
    fn drain_rx(&mut self) {
        let mut junk = [0u8; 32];
        while let Ok(n) = self.uart.read_nb(&mut junk) {
            if n == 0 {
                break;
            }
        }
    }

    async fn read_response(
        &mut self,
        buf: &mut [u8],
        filled: &mut usize,
        deadline: Instant,
    ) -> Result<usize, TransportError<<UART as ErrorType>::Error>> {
        self.read_exact(buf, filled, MIN_FRAME_BYTES, deadline)
            .await?;
        let len =
            rmodbus::guess_response_frame_len(&buf[0..MIN_FRAME_BYTES], rmodbus::ModbusProto::Rtu)
                .map_err(|_| TransportError::Corrupt("failed to guess frame length"))?
                as usize;
        if len > buf.len() {
            return Err(TransportError::Corrupt("guessed frame length exceeds buffer"));
        }
        if len > MIN_FRAME_BYTES {
            self.read_exact(buf, filled, len, deadline).await?;
        }
        Ok(len)
    }

    /// Send a request and read the raw response. Single attempt, no retry.
    /// Retryable failures are logged with the bytes on the wire.
    async fn exchange(
        &mut self,
        op: &str,
        attempt: usize,
        request: &[u8],
        response_buf: &mut [u8],
    ) -> Result<usize, TransportError<<UART as ErrorType>::Error>> {
        self.drain_rx();
        self.uart.write_all(request).map_err(TransportError::Uart)?;
        self.uart.flush().map_err(TransportError::Uart)?;

        let sent_at = Instant::now();
        let deadline = sent_at + embassy_time::Duration::from_millis(RESPONSE_TIMEOUT_MS);
        let mut filled = 0;
        let result = self.read_response(response_buf, &mut filled, deadline).await;
        if let Err(e @ (TransportError::Timeout | TransportError::Corrupt(_))) = &result {
            warn_failure(op, attempt, e, request, &response_buf[..filled], sent_at);
        }
        result
    }
}

/// Log a failed attempt with what we sent, what (if anything) came back,
/// and how long we waited.
fn warn_failure<E: core::fmt::Debug>(
    op: &str,
    attempt: usize,
    error: &E,
    request: &[u8],
    received: &[u8],
    sent_at: Instant,
) {
    log::warn!(
        "Modbus {} {:?} after {}us, retry {}/{} tx={:02X?} rx={:02X?}",
        op,
        error,
        sent_at.elapsed().as_micros(),
        attempt + 1,
        MAX_RETRIES,
        request,
        received,
    );
}

impl<UART, DELAY> ModbusTransport for Rs485ModbusTransport<UART, DELAY>
where
    UART: ReadNonBlocking + Write,
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
        for attempt in 0..MAX_RETRIES {
            match self.exchange("write", attempt, &request, &mut response).await {
                Ok(len) => match modbus_req.parse_ok(&response[..len]) {
                    Ok(()) => {
                        self.delay.delay_us(INTER_COMMAND_DELAY_US).await;
                        return Ok(());
                    }
                    Err(e) => log::warn!(
                        "Modbus write rejected response ({:?}), retry {}/{} tx={:02X?} rx={:02X?}",
                        e,
                        attempt + 1,
                        MAX_RETRIES,
                        request.as_slice(),
                        &response[..len],
                    ),
                },
                Err(TransportError::Timeout | TransportError::Corrupt(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Err(TransportError::Timeout)
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
        for attempt in 0..MAX_RETRIES {
            match self.exchange("read", attempt, &request, &mut response).await {
                Ok(len) => {
                    let mut result: Vec<u16, MAX_REGS_PER_READ> = Vec::new();
                    match modbus_req.parse_u16(&response[..len], &mut result) {
                        Ok(()) => {
                            self.delay.delay_us(INTER_COMMAND_DELAY_US).await;
                            return Ok(result);
                        }
                        Err(e) => log::warn!(
                            "Modbus read rejected response ({:?}), retry {}/{} tx={:02X?} rx={:02X?}",
                            e,
                            attempt + 1,
                            MAX_RETRIES,
                            request.as_slice(),
                            &response[..len],
                        ),
                    }
                }
                Err(TransportError::Timeout | TransportError::Corrupt(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Err(TransportError::Timeout)
    }

    async fn raw_transaction(
        &mut self,
        request: &[u8],
        response: &mut [u8],
    ) -> Result<usize, Self::Error> {
        use crc::{CRC_16_MODBUS, Crc};
        const MODBUS_CRC: Crc<u16> = Crc::<u16>::new(&CRC_16_MODBUS);
        let mut frame: Vec<u8, 32> = Vec::new();
        frame
            .extend_from_slice(request)
            .map_err(|_| TransportError::Protocol("request exceeds frame buffer"))?;
        let crc = MODBUS_CRC.checksum(request).to_le_bytes();
        frame
            .extend_from_slice(&crc)
            .map_err(|_| TransportError::Protocol("request + CRC exceeds frame buffer"))?;
        for attempt in 0..MAX_RETRIES {
            self.drain_rx();
            self.uart.write_all(&frame).map_err(TransportError::Uart)?;
            self.uart.flush().map_err(TransportError::Uart)?;

            let sent_at = Instant::now();
            let deadline = sent_at + embassy_time::Duration::from_millis(RESPONSE_TIMEOUT_MS);
            let expected_len = response.len();
            let mut filled = 0;
            match self
                .read_exact(response, &mut filled, expected_len, deadline)
                .await
            {
                Ok(()) => {
                    self.delay.delay_us(INTER_COMMAND_DELAY_US).await;
                    return Ok(expected_len);
                }
                Err(e @ (TransportError::Timeout | TransportError::Corrupt(_))) => {
                    warn_failure("raw", attempt, &e, &frame, &response[..filled], sent_at);
                }
                Err(e) => return Err(e),
            }
        }
        Err(TransportError::Timeout)
    }
}

impl<UART, DELAY> UartReconfigure for Rs485ModbusTransport<UART, DELAY>
where
    UART: UartReconfigure,
{
    type Error = UART::Error;

    async fn reconfigure_baud(&mut self, baud: u32) -> Result<(), Self::Error> {
        self.uart.reconfigure_baud(baud).await
    }
}
