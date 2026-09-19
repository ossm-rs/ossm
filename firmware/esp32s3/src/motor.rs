// `Config` always matches the real motor's shape (rs485 for this crate today).
// Sim mode uses the same fields and drops them inside `build`.
#[cfg(feature = "motor-rs485")]
pub use ossm_esp::motor::rs485::Config;

#[cfg(feature = "motor-sim")]
pub use ossm_esp::motor::sim::Motor;

#[cfg(feature = "motor-sim")]
pub async fn build(config: Config) -> Motor {
    ossm_esp::motor::sim::build(config)
}

#[cfg(all(feature = "motor-rs485", not(feature = "motor-sim")))]
pub use ossm_esp::motor::rs485::build;
