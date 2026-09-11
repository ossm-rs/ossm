use core::sync::atomic::{AtomicBool, Ordering};
use portable_atomic::AtomicU64;

use embassy_time::{Duration, Instant, Timer};
use esp_hal::{Blocking, usb_serial_jtag::UsbSerialJtag};
use log::{info, warn};
use control_interface::{ControlSender, policy};

pub const OWNER_HEARTBEAT_INTERVAL_MS: u64 = 250;
pub const OWNER_TIMEOUT_MS: u64 = 750;
const MAX_COMMAND_LEN: usize = 160;
const PREFIX: &str = "@OWNER:";

static LAST_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);
static WIFI_LOCAL_LIMITS_LATCHED: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_wifi_local_limits_latched(active: bool) {
    WIFI_LOCAL_LIMITS_LATCHED.store(active, Ordering::Release);
}

pub(crate) fn wifi_local_limits_latched() -> bool {
    WIFI_LOCAL_LIMITS_LATCHED.load(Ordering::Acquire)
}

fn heartbeat() {
    LAST_HEARTBEAT_MS.store(Instant::now().as_millis(), Ordering::Release);
}

pub(crate) fn heartbeat_fresh() -> bool {
    if wifi_local_limits_latched() {
        return true;
    }
    let last = LAST_HEARTBEAT_MS.load(Ordering::Acquire);
    last != 0 && Instant::now().as_millis().saturating_sub(last) <= OWNER_TIMEOUT_MS
}

fn parse_f64(value: Option<&str>) -> Option<f64> {
    value?.parse::<f64>().ok()
}

pub(crate) fn process_line(line: &str, control: &'static ControlSender) {
    let line = line.trim();
    if !line.starts_with(PREFIX) {
        return;
    }

    let mut parts = line[PREFIX.len()..].split(':');
    let command = parts.next().unwrap_or("");

    match command {
        "HB" => heartbeat(),
        "ESTOP" => match parts.next().unwrap_or("") {
            "RESET" => {
                policy::set_estop_active(false);
                control.reapply_policy();
                info!("Software E-stop reset; motion remains stopped until a new remote speed command");
            }
            "" => {
                policy::set_estop_active(true);
                control.stop();
                control.reapply_policy();
                warn!("SOFTWARE E-STOP LATCHED");
            }
            _ => warn!("Unknown OWNER ESTOP command"),
        },
        "MASTER" => match parts.next().unwrap_or("") {
            "ENABLE" => {
                policy::deactivate_owner_session();
                policy::set_master_enabled(true);
                control.reapply_policy();
                info!("Owner master enabled - persisted owner limits active");
            }
            "DISABLE" => {
                policy::set_master_enabled(false);
                control.reapply_policy();
                info!("Owner master disabled - stock motion envelope active");
            }
            _ => warn!("Unknown OWNER MASTER command"),
        },
        "ENABLE" => {
            // ENABLE is itself proof of a live local owner connection.
            // Future heartbeats only track the page session; persisted limits remain active after timeout.
            heartbeat();
            if policy::master_enabled() {
                if policy::activate_owner_session() {
                    control.reapply_policy();
                    info!("Owner limits active");
                }
            } else {
                warn!("Owner enable rejected: master off");
            }
        }
        "DISABLE" => {
            policy::deactivate_owner_session();
            control.reapply_policy();
            info!("Owner page session disabled - persisted owner limits remain active if master enabled");
        }
        "LIMITS" => {
            let values = (
                parse_f64(parts.next()),
                parse_f64(parts.next()),
                parse_f64(parts.next()),
                parse_f64(parts.next()),
                parse_f64(parts.next()),
            );
            if let (Some(max_speed), Some(min_stroke), Some(max_stroke), Some(min_depth), Some(max_depth)) = values {
                heartbeat();
                policy::set_owner_limits(max_speed, min_stroke, max_stroke, min_depth, max_depth);
                control.reapply_policy();
                info!(
                    "Owner limits updated speed={:.1} stroke={:.1}..{:.1} depth={:.1}..{:.1}",
                    max_speed, min_stroke, max_stroke, min_depth, max_depth
                );
            } else {
                warn!("Bad OWNER LIMITS command");
            }
        }
        _ => warn!("Unknown OWNER command"),
    }
}

/// Local owner control accepts the same command protocol from USB and Wi-Fi.
/// USB RX remains available even when the Wi-Fi owner page is enabled.
/// There are deliberately no protocol
/// responses or periodic USB writes; this keeps esp-println as the only USB TX
/// producer and avoids the contention seen in the older firmware branch.
#[embassy_executor::task]
pub async fn owner_usb_task(
    mut usb: UsbSerialJtag<'static, Blocking>,
    control: &'static ControlSender,
) {
    info!("Owner USB RX task started");
    let mut command = [0u8; MAX_COMMAND_LEN];
    let mut len = 0usize;

    loop {
        match usb.read_byte() {
            Ok(byte) => match byte {
                b'\r' => {}
                b'\n' => {
                    if len > 0 {
                        if let Ok(line) = core::str::from_utf8(&command[..len]) {
                            process_line(line, control);
                        }
                        len = 0;
                    }
                }
                _ => {
                    if len < command.len() {
                        command[len] = byte;
                        len += 1;
                    } else {
                        len = 0;
                        warn!("Owner USB command too long");
                    }
                }
            },
            Err(nb::Error::WouldBlock) => Timer::after(Duration::from_millis(2)).await,
            Err(nb::Error::Other(_)) => Timer::after(Duration::from_millis(10)).await,
        }
    }
}

#[embassy_executor::task]
pub async fn owner_supervisor_task(control: &'static ControlSender) {
    loop {
        if policy::owner_session_active() && !heartbeat_fresh() {
            policy::deactivate_owner_session();
            control.reapply_policy();
            warn!("Owner heartbeat timeout - page session closed; persisted owner limits remain active");
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}
