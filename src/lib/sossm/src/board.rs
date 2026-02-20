use core::fmt::Debug;
use crate::Motor;

#[allow(async_fn_in_trait)]
pub trait Board {
    type Error: Debug + From<<Self::M as Motor>::Error>;
    type M: Motor;

    fn motor(&mut self) -> &mut Self::M;
    fn steps_per_mm(&self) -> f32;

    async fn enable(&mut self, enable: bool) -> Result<(), Self::Error> {
        self.motor().enable(enable).await?;
        Ok(())
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        self.motor().home().await?;
        Ok(())
    }

    async fn move_to(&mut self, mm: f32) -> Result<(), Self::Error> {
        let steps = (mm * self.steps_per_mm()) as i32;
        self.motor().set_absolute_position(steps).await?;
        Ok(())
    }

    async fn wait_until_stopped(&mut self, threshold_mm: f32) {
        let threshold = (threshold_mm * self.steps_per_mm()) as i32;
        self.motor().wait_for_target_reached(threshold).await;
    }
}
