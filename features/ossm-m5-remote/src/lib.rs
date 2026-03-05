#![no_std]

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Ticker};
use esp_radio::esp_now::{
    BROADCAST_ADDRESS, EspNowManager, EspNowReceiver, EspNowSender, PeerInfo,
};
use log::{error, info, trace};
use pattern_engine::SharedPatternInput;
use portable_atomic::{AtomicU32, AtomicU64};
use zerocopy::{Immutable, IntoBytes, KnownLayout, TryFromBytes};

const OSSM_ID: i32 = 1;
const M5_ID: i32 = 99;
const MAX_NO_REMOTE_HEARTBEAT_MS: u64 = 10_000;

static LAST_HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Current pattern index in **engine** space.
static CURRENT_PATTERN_IDX: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Pattern index mapping (remote ↔ engine)
//
// The M5 remote has a hardcoded pattern list that differs in order (and
// contents) from the pattern engine's `AnyPattern::all_builtin()`.
// ---------------------------------------------------------------------------

/// The pattern list as it appears on the M5 remote's UI roller.
///
/// Variants without an engine equivalent return `None` from
/// [`Self::to_engine_index`].
#[derive(Debug, Clone, Copy)]
enum RemotePattern {
    SimpleStroke,
    TeasingPounding,
    RoboStroke,
    HalfnHalf,
    Deeper,
    StopNGo,
    Insist,
    Knot,
}

impl RemotePattern {
    /// Parse the wire index sent by the M5 remote.
    fn from_remote_index(idx: u32) -> Option<Self> {
        match idx {
            0 => Some(Self::SimpleStroke),
            1 => Some(Self::TeasingPounding),
            2 => Some(Self::RoboStroke),
            3 => Some(Self::HalfnHalf),
            4 => Some(Self::Deeper),
            5 => Some(Self::StopNGo),
            6 => Some(Self::Insist),
            7 => Some(Self::Knot),
            _ => None,
        }
    }

    /// Convert an engine pattern index to a remote pattern, if one exists.
    fn from_engine_index(idx: u32) -> Option<Self> {
        match idx {
            0 => Some(Self::SimpleStroke),
            1 => Some(Self::Deeper),
            2 => Some(Self::HalfnHalf),
            3 => Some(Self::StopNGo),
            4 => Some(Self::TeasingPounding),
            _ => None, // Torque (5), None (6) have no remote equivalent
        }
    }

    /// Convert to an engine pattern index.
    ///
    /// Patterns without an engine implementation map to the None pattern
    /// (index 6), which holds position.
    fn to_engine_index(self) -> u32 {
        match self {
            Self::SimpleStroke => 0,
            Self::Deeper => 1,
            Self::HalfnHalf => 2,
            Self::StopNGo => 3,
            Self::TeasingPounding => 4,
            Self::RoboStroke | Self::Insist | Self::Knot => 6, // None pattern
        }
    }

    /// The wire index expected by the M5 remote.
    fn to_remote_index(self) -> u32 {
        match self {
            Self::SimpleStroke => 0,
            Self::TeasingPounding => 1,
            Self::RoboStroke => 2,
            Self::HalfnHalf => 3,
            Self::Deeper => 4,
            Self::StopNGo => 5,
            Self::Insist => 6,
            Self::Knot => 7,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemoteConfig {
    pub max_velocity_mm_s: f64,
    pub max_travel_mm: f64,
}

#[derive(Debug, Clone)]
pub enum RemoteEvent {
    Enable,
    Disable,
    SwitchPattern(u32),
}

pub type RemoteEventChannel = Channel<CriticalSectionRawMutex, RemoteEvent, 4>;

#[derive(Default, Debug, TryFromBytes, IntoBytes, Immutable)]
#[repr(i32)]
#[allow(dead_code)]
enum M5Command {
    Conn = 0,
    Speed = 1,
    Depth = 2,
    Stroke = 3,
    Sensation = 4,
    Pattern = 5,
    TorqueF = 6,
    TorqueR = 7,
    Off = 10,
    On = 11,
    SetupDI = 12,
    SetupDIF = 13,
    Reboot = 14,

    CumSpeed = 20,
    CumTime = 21,
    CumSize = 22,
    CumAccel = 23,

    Connect = 88,

    #[default]
    Heartbeat = 99,
}

#[derive(Default, Debug, TryFromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C)]
struct M5Packet {
    speed: f32,
    depth: f32,
    stroke: f32,
    sensation: f32,
    pattern: f32,
    rstate: bool,
    connected: bool,
    heartbeat: bool,
    _padding: bool,
    command: M5Command,
    value: f32,
    target: i32,
}

impl M5Packet {
    fn heartbeat_packet(config: RemoteConfig) -> Self {
        let engine_idx = CURRENT_PATTERN_IDX.load(Ordering::Acquire);
        let remote_idx = RemotePattern::from_engine_index(engine_idx)
            .map(|p| p.to_remote_index())
            .unwrap_or(0);
        Self {
            connected: true,
            target: M5_ID,
            speed: config.max_velocity_mm_s as f32,
            depth: config.max_travel_mm as f32,
            pattern: remote_idx as f32,
            ..Default::default()
        }
    }
}

async fn send_heartbeat_packet(
    sender: &'static Mutex<NoopRawMutex, EspNowSender<'static>>,
    peer: &PeerInfo,
    config: RemoteConfig,
) {
    let mut sender = sender.lock().await;
    if let Err(err) = sender
        .send_async(
            &peer.peer_address,
            M5Packet::heartbeat_packet(config).as_bytes(),
        )
        .await
    {
        error!("Could not send heartbeat packet: {}", err);
    }
}

/// Receives packets from the M5 remote, updates pattern input, and emits
/// remote events for on/off/pattern changes.
#[embassy_executor::task]
pub async fn receiver_task(
    manager: &'static EspNowManager<'static>,
    sender: &'static Mutex<NoopRawMutex, EspNowSender<'static>>,
    mut receiver: EspNowReceiver<'static>,
    pattern_input: &'static SharedPatternInput,
    remote_events: &'static RemoteEventChannel,
    config: RemoteConfig,
) {
    info!("ESP-NOW receiver task started");

    loop {
        let r = receiver.receive_async().await;
        let data = r.data();

        let packet = match M5Packet::try_ref_from_bytes(data) {
            Ok(packet) => packet,
            Err(err) => {
                error!("Failed to parse M5 packet: {:?}", err);
                continue;
            }
        };

        match packet.command {
            M5Command::Speed
            | M5Command::Depth
            | M5Command::Stroke
            | M5Command::Sensation
            | M5Command::Heartbeat => {
                trace!("M5 packet: {:?}", packet);
            }
            _ => {
                info!("M5 packet: {:?}", packet);
            }
        }

        match packet.command {
            M5Command::On => {
                let ack = M5Packet {
                    target: M5_ID,
                    command: M5Command::On,
                    ..Default::default()
                };
                if let Ok(peer) = manager.fetch_peer(true) {
                    let mut sender = sender.lock().await;
                    if let Err(err) = sender.send_async(&peer.peer_address, ack.as_bytes()).await {
                        error!("Could not send ON ack: {}", err);
                    }
                }
                remote_events.send(RemoteEvent::Enable).await;
            }
            M5Command::Off => {
                let ack = M5Packet {
                    target: M5_ID,
                    command: M5Command::Off,
                    ..Default::default()
                };
                if let Ok(peer) = manager.fetch_peer(true) {
                    let mut sender = sender.lock().await;
                    if let Err(err) = sender.send_async(&peer.peer_address, ack.as_bytes()).await {
                        error!("Could not send OFF ack: {}", err);
                    }
                }
                remote_events.send(RemoteEvent::Disable).await;
            }
            M5Command::Speed => {
                let velocity = (packet.value as f64) / config.max_velocity_mm_s;
                pattern_input.lock(|cell| {
                    let mut input = cell.get();
                    input.velocity = velocity.clamp(0.0, 1.0);
                    cell.set(input);
                });
            }
            M5Command::Depth => {
                let depth = (packet.value as f64) / config.max_travel_mm;
                pattern_input.lock(|cell| {
                    let mut input = cell.get();
                    input.depth = depth.clamp(0.0, 1.0);
                    cell.set(input);
                });
            }
            M5Command::Stroke => {
                let stroke = (packet.value as f64) / config.max_travel_mm;
                pattern_input.lock(|cell| {
                    let mut input = cell.get();
                    input.stroke = stroke.clamp(0.0, 1.0);
                    cell.set(input);
                });
            }
            M5Command::Sensation => {
                // Remote sends -100..100; pattern engine expects -1.0..1.0
                let sensation = ((packet.value as f64) / 100.0).clamp(-1.0, 1.0);
                pattern_input.lock(|cell| {
                    let mut input = cell.get();
                    input.sensation = sensation;
                    cell.set(input);
                });
            }
            M5Command::Pattern => {
                let remote_idx = packet.value as u32;
                if let Some(pattern) = RemotePattern::from_remote_index(remote_idx) {
                    let engine_idx = pattern.to_engine_index();
                    CURRENT_PATTERN_IDX.store(engine_idx, Ordering::Release);
                    remote_events
                        .send(RemoteEvent::SwitchPattern(engine_idx))
                        .await;
                }
            }
            M5Command::Heartbeat => {
                let now = Instant::now().as_millis();
                LAST_HEARTBEAT.store(now, Ordering::Release);
            }
            _ => {}
        }

        // Auto-pairing: if this packet is targeted at us via broadcast from
        // an unknown peer, register them and send a heartbeat to confirm.
        if packet.target == OSSM_ID
            && r.info.dst_address == BROADCAST_ADDRESS
            && !manager.peer_exists(&r.info.src_address)
        {
            let peer = PeerInfo {
                interface: esp_radio::esp_now::EspNowWifiInterface::Sta,
                peer_address: r.info.src_address,
                lmk: None,
                channel: None,
                encrypt: false,
            };
            manager.add_peer(peer).unwrap();
            info!("Added new peer {:?}", r.info.src_address);

            send_heartbeat_packet(sender, &peer, config).await;
        }
    }
}

/// Periodically sends heartbeat packets to the paired remote so it knows
/// the OSSM is still alive. Includes max velocity and travel for the
/// remote's UI.
#[embassy_executor::task]
pub async fn heartbeat_send_task(
    manager: &'static EspNowManager<'static>,
    sender: &'static Mutex<NoopRawMutex, EspNowSender<'static>>,
    config: RemoteConfig,
) {
    info!("ESP-NOW heartbeat send task started");

    let mut ticker = Ticker::every(Duration::from_millis(5000));

    loop {
        ticker.next().await;

        let peer = match manager.fetch_peer(true) {
            Ok(peer) => peer,
            Err(_) => continue,
        };

        send_heartbeat_packet(sender, &peer, config).await;
    }
}

/// Monitors incoming heartbeats from the remote. If none arrive within the
/// timeout window, emits a [`RemoteEvent::Disable`] to shut down motion.
#[embassy_executor::task]
pub async fn heartbeat_check_task(remote_events: &'static RemoteEventChannel) {
    info!("ESP-NOW heartbeat check task started");

    let mut ticker = Ticker::every(Duration::from_millis(1000));

    loop {
        ticker.next().await;

        let last_heartbeat = Instant::from_millis(LAST_HEARTBEAT.load(Ordering::Acquire));
        let elapsed = last_heartbeat.elapsed().as_millis();

        let was_connected = CONNECTED.load(Ordering::Acquire);
        let is_connected = elapsed <= MAX_NO_REMOTE_HEARTBEAT_MS;

        CONNECTED.store(is_connected, Ordering::Release);

        if was_connected && !is_connected {
            info!("Remote heartbeat lost, disabling");
            remote_events.send(RemoteEvent::Disable).await;
        }
    }
}

/// Returns whether the M5 remote is currently connected (heartbeats arriving).
pub fn is_connected() -> bool {
    CONNECTED.load(Ordering::Acquire)
}

/// Sets the current pattern index (used by heartbeat to sync with remote).
pub fn set_current_pattern(idx: u32) {
    CURRENT_PATTERN_IDX.store(idx, Ordering::Release);
}

/// Returns the current pattern index.
pub fn current_pattern() -> u32 {
    CURRENT_PATTERN_IDX.load(Ordering::Acquire)
}
