use embedded_hal::digital::OutputPin;
use embedded_io::{ErrorType, Read, Write};

/// Half-duplex RS485 wrapper that toggles a DE (Driver Enable) pin
/// around UART writes. DE goes HIGH before transmitting and LOW
/// after flush, switching the transceiver between TX and RX mode.
///
/// Implements `Read + Write` so it can be used as a drop-in replacement
/// for a plain UART in any driver that expects `embedded_io` traits.
pub struct Rs485<UART, DE> {
    uart: UART,
    de: DE,
}

impl<UART, DE> Rs485<UART, DE>
where
    DE: OutputPin,
{
    pub fn new(uart: UART, mut de: DE) -> Self {
        let _ = de.set_low(); // Start in receive mode
        Self { uart, de }
    }
}

impl<UART, DE> ErrorType for Rs485<UART, DE>
where
    UART: ErrorType,
{
    type Error = UART::Error;
}

impl<UART, DE> Read for Rs485<UART, DE>
where
    UART: Read,
    DE: OutputPin,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.uart.read(buf)
    }
}

impl<UART, DE> Write for Rs485<UART, DE>
where
    UART: Write,
    DE: OutputPin,
{
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let _ = self.de.set_high(); // Enable driver (transmit)
        self.uart.write(buf)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.uart.flush()?;
        let _ = self.de.set_low(); // Back to receive mode
        Ok(())
    }
}
