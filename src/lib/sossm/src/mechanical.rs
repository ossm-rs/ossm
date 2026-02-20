#[derive(Debug, Clone)]
pub struct MechanicalConfig {
    pub pulley_teeth: u32,
    pub belt_pitch_mm: f32,
}

impl Default for MechanicalConfig {
    fn default() -> Self {
        Self {
            pulley_teeth: 20,
            belt_pitch_mm: 2.0,
        }
    }
}

impl MechanicalConfig {
    pub fn steps_per_mm(&self, steps_per_rev: u32) -> f32 {
        steps_per_rev as f32 / (self.pulley_teeth as f32 * self.belt_pitch_mm)
    }
}