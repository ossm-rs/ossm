#[derive(Debug, Clone)]
pub struct MechanicalConfig {
    pub pulley_teeth: u32,
    pub belt_pitch_mm: f32,
    /// Closest point to the homing end-stop the machine should move to (mm).
    pub min_position_mm: f64,
    /// Furthest point from the homing end-stop the machine should move to (mm).
    pub max_position_mm: f64,
}

impl Default for MechanicalConfig {
    fn default() -> Self {
        Self {
            pulley_teeth: 20,
            belt_pitch_mm: 2.0,
            min_position_mm: 10.0,
            max_position_mm: 190.0,
        }
    }
}

impl MechanicalConfig {
    pub fn steps_per_mm(&self, steps_per_rev: u32) -> f32 {
        steps_per_rev as f32 / (self.pulley_teeth as f32 * self.belt_pitch_mm)
    }
}