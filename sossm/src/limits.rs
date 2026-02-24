/// Operational motion limits passed to the trajectory planner.
///
/// These cap what patterns and commands can request. They should be set to
/// values the machine can safely sustain - informed by mechanical construction
/// and the motor's capabilities, but ultimately operator-configurable.
#[derive(Debug, Clone)]
pub struct MotionLimits {
    /// Maximum linear velocity (mm/s).
    pub max_velocity_mm_s: f64,
    /// Maximum linear acceleration (mm/s²).
    pub max_acceleration_mm_s2: f64,
    /// Maximum linear jerk (mm/s³).
    pub max_jerk_mm_s3: f64,
}

impl Default for MotionLimits {
    fn default() -> Self {
        Self {
            max_velocity_mm_s: 600.0,
            max_acceleration_mm_s2: 30_000.0,
            max_jerk_mm_s3: 100_000.0,
        }
    }
}
