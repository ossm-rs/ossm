/// Mechanical geometry of the linear actuator.
///
/// Used by boards to convert between physical units (mm) and motor units
/// (steps, revolutions). This is a board-level concern — the motion
/// controller and everything upstream only see mm.
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
    /// mm of linear travel per motor revolution.
    pub fn mm_per_rev(&self) -> f32 {
        self.pulley_teeth as f32 * self.belt_pitch_mm
    }

    /// Steps per mm of linear travel, given the motor's steps per revolution.
    pub fn steps_per_mm(&self, steps_per_rev: u32) -> f32 {
        steps_per_rev as f32 / self.mm_per_rev()
    }

    /// Convert mm to steps.
    pub fn mm_to_steps(&self, mm: f64, steps_per_rev: u32) -> i32 {
        (mm * self.steps_per_mm(steps_per_rev) as f64) as i32
    }

    /// Convert steps to mm.
    pub fn steps_to_mm(&self, steps: i32, steps_per_rev: u32) -> f64 {
        steps as f64 / self.steps_per_mm(steps_per_rev) as f64
    }
}
