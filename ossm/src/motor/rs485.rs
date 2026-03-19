use super::Motor;

/// Marker trait for motors wired over RS-485.
///
/// No additional methods — this exists so boards can constrain their
/// motor generic to the correct physical interface at compile time.
#[allow(async_fn_in_trait)]
pub trait Rs485Motor: Motor {}
