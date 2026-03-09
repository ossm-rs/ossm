use embedded_hal_async::delay::DelayNs;
use ossm::{Motor, Rs485, SelfHoming};

use crate::{modbus_constants, Modbus, ModbusTransport, Motor57AIM, RoRegister, RwRegister};

impl<T: ModbusTransport, D> Motor57AIM<Modbus<T>, D> {
    async fn write_register(&mut self, reg: RwRegister, value: u16) -> Result<(), T::Error> {
        self.interface
            .transport
            .write_holding(self.interface.device_addr, reg as u16, value)
            .await
    }

    async fn read_register(&mut self, reg: u16) -> Result<u16, T::Error> {
        let regs = self
            .interface
            .transport
            .read_holding(self.interface.device_addr, reg, 1)
            .await?;
        Ok(regs[0])
    }

    async fn read_register_pair(&mut self, reg_low: u16) -> Result<i32, T::Error> {
        let regs = self
            .interface
            .transport
            .read_holding(self.interface.device_addr, reg_low, 2)
            .await?;
        let raw = ((regs[1] as u32) << 16) | regs[0] as u32;
        Ok(raw as i32)
    }

    /// Enable Modbus control and driver output.
    pub async fn enable_driver(&mut self) -> Result<(), T::Error> {
        self.write_register(RwRegister::ModbusEnable, 1).await?;
        self.write_register(RwRegister::DriverOutputEnable, 1).await
    }

    /// Disable driver output and Modbus control.
    pub async fn disable_driver(&mut self) -> Result<(), T::Error> {
        self.write_register(RwRegister::DriverOutputEnable, 0)
            .await?;
        self.write_register(RwRegister::ModbusEnable, 0).await
    }

    /// Atomic 4-byte absolute position command (vendor function 0x7B).
    pub async fn set_absolute_position(&mut self, steps: i32) -> Result<(), T::Error> {
        let mut request = [0u8; 6];
        request[0] = self.interface.device_addr;
        request[1] = modbus_constants::SET_ABSOLUTE_POSITION_FUNC;
        request[2..6].copy_from_slice(&steps.to_be_bytes());
        let mut response = [0u8; 8];
        self.interface
            .transport
            .raw_transaction(&request, &mut response)
            .await?;
        Ok(())
    }

    pub async fn set_speed(&mut self, rpm: u16) -> Result<(), T::Error> {
        self.write_register(RwRegister::MotorTargetSpeed, rpm).await
    }

    pub async fn set_acceleration(&mut self, value: u16) -> Result<(), T::Error> {
        self.write_register(RwRegister::MotorAcceleration, value)
            .await
    }

    pub async fn set_max_output(&mut self, output: u16) -> Result<(), T::Error> {
        self.write_register(RwRegister::StandstillMaxOutput, output)
            .await
    }

    /// Configure the motor for maximum tracking performance.
    ///
    /// Sets speed, acceleration, and output to maximum so the motor acts
    /// as a pure position servo, following commands from ruckig with
    /// minimal lag.
    pub async fn configure_max_tracking(&mut self) -> Result<(), T::Error> {
        self.set_speed(modbus_constants::OPERATING_SPEED_RPM)
            .await?;
        self.set_acceleration(modbus_constants::OPERATING_ACCELERATION)
            .await?;
        self.set_max_output(modbus_constants::OPERATING_MAX_OUTPUT)
            .await
    }

    /// Trigger the motor's built-in homing sequence.
    pub async fn trigger_homing(&mut self) -> Result<(), T::Error> {
        self.write_register(
            RwRegister::MotorTargetSpeed,
            modbus_constants::HOME_SPEED_RPM,
        )
        .await?;
        self.write_register(
            RwRegister::StandstillMaxOutput,
            modbus_constants::HOME_MAX_OUTPUT,
        )
        .await?;
        self.write_register(RwRegister::SpecificFunction, 1).await
    }

    pub async fn read_absolute_position(&mut self) -> Result<i32, T::Error> {
        self.read_register_pair(RwRegister::AbsolutePositionLowU16 as u16)
            .await
    }

    pub async fn read_remaining_steps(&mut self) -> Result<i32, T::Error> {
        self.read_register_pair(RoRegister::TargetPositionLowU16 as u16)
            .await
    }

    pub async fn read_current_amps(&mut self) -> Result<f32, T::Error> {
        let raw = self.read_register(RoRegister::SystemCurrent as u16).await?;
        Ok(raw as f32 / 2000.0)
    }

    pub async fn read_voltage_volts(&mut self) -> Result<f32, T::Error> {
        let raw = self.read_register(RoRegister::SystemVoltage as u16).await?;
        Ok(raw as f32 / 327.0)
    }

    pub async fn read_temperature(&mut self) -> Result<u16, T::Error> {
        self.read_register(RoRegister::SystemTemperature as u16)
            .await
    }

    pub async fn read_alarm_code(&mut self) -> Result<u16, T::Error> {
        self.read_register(RoRegister::AlarmCode as u16).await
    }
}

impl<T: ModbusTransport, D: DelayNs> Motor for Motor57AIM<Modbus<T>, D> {
    type Error = T::Error;

    fn steps_per_rev(&self) -> u32 {
        self.config.steps_per_rev
    }

    fn max_output(&self) -> u16 {
        self.config.max_output
    }

    async fn enable(&mut self) -> Result<(), Self::Error> {
        self.enable_driver().await?;
        self.configure_max_tracking().await
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.disable_driver().await
    }

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        Motor57AIM::set_absolute_position(self, steps).await
    }

    async fn read_absolute_position(&mut self) -> Result<i32, Self::Error> {
        Motor57AIM::read_absolute_position(self).await
    }

    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error> {
        Motor57AIM::set_max_output(self, output).await
    }
}

impl<T: ModbusTransport, D: DelayNs> Rs485 for Motor57AIM<Modbus<T>, D> {}

impl<T: ModbusTransport, D: DelayNs> SelfHoming for Motor57AIM<Modbus<T>, D> {
    async fn home(&mut self) -> Result<(), Self::Error> {
        self.trigger_homing().await?;

        loop {
            self.delay
                .delay_ms(modbus_constants::HOME_POLL_INTERVAL_MS)
                .await;
            let remaining = self.read_remaining_steps().await?;
            if remaining.abs() < modbus_constants::HOME_STEP_THRESHOLD {
                break;
            }
        }

        self.delay
            .delay_ms(modbus_constants::POST_HOME_SETTLE_MS)
            .await;

        // Re-enable Modbus (homing resets it on the 57AIM)
        self.enable_driver().await?;
        self.delay
            .delay_ms(modbus_constants::POST_ENABLE_SETTLE_MS)
            .await;

        self.configure_max_tracking().await
    }
}
