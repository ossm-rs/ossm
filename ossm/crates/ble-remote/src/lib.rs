#![no_std]

use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

pub const CONNECTIONS_MAX: usize = 1;
pub const L2CAP_CHANNELS_MAX: usize = 2;
pub const MAX_COMMAND_LENGTH: usize = 64;
pub const MAX_STATE_LENGTH: usize = 128;
pub const MAX_PATTERN_LENGTH: usize = 1024;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker, Timer};
use esp_radio::ble::controller::BleConnector;
use heapless::String;
use log::{debug, error, info, warn};
use modes::{OssmMode, Rejected, RemoteHandle, RemoteId, Request, Status};
use pattern_engine::{EngineState, commands, commands::PatternCommand};
use static_cell::StaticCell;
use trouble_host::prelude::*;

const SERVICE_UUID: Uuid = uuid!("522b443a-4f53-534d-0001-420badbabe69");
const PRIMARY_COMMAND_UUID: Uuid = uuid!("522b443a-4f53-534d-1000-420badbabe69");
const SPEED_KNOB_UUID: Uuid = uuid!("522b443a-4f53-534d-1010-420badbabe69");
const CURRENT_STATE_UUID: Uuid = uuid!("522b443a-4f53-534d-2000-420badbabe69");
const PATTERN_LIST_UUID: Uuid = uuid!("522b443a-4f53-534d-3000-420badbabe69");
const PATTERN_DESCRIPTION_UUID: Uuid = uuid!("522b443a-4f53-534d-3010-420badbabe69");

static CONNECTED: AtomicBool = AtomicBool::new(false);
/// Pattern `go:strokeEngine` plays when nothing is playing: the last one
/// pattern mode accepted from `set:pattern`.
static LAST_PATTERN: AtomicUsize = AtomicUsize::new(0);

type Remote = RemoteHandle<OssmMode>;

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

#[gatt_server]
struct Server {
    ossm_service: OssmService,
}

#[gatt_service(uuid = SERVICE_UUID)]
struct OssmService {
    #[characteristic(uuid = PRIMARY_COMMAND_UUID, read, write)]
    primary_command: String<MAX_COMMAND_LENGTH>,

    #[characteristic(uuid = SPEED_KNOB_UUID, read, write)]
    speed_knob: String<16>,

    #[characteristic(uuid = CURRENT_STATE_UUID, read, notify)]
    current_state: String<MAX_STATE_LENGTH>,

    #[characteristic(uuid = PATTERN_LIST_UUID, read)]
    pattern_list: String<MAX_PATTERN_LENGTH>,

    #[characteristic(uuid = PATTERN_DESCRIPTION_UUID, read, write)]
    pattern_description: String<MAX_PATTERN_LENGTH>,
}

fn get_all_patterns_json() -> String<MAX_PATTERN_LENGTH> {
    let mut output: String<MAX_PATTERN_LENGTH> = String::new();
    output.write_char('[').ok();
    for (i, meta) in commands::pattern_list().iter().enumerate() {
        let rollback = output.len();
        let sep = if i > 0 { "," } else { "" };
        if write!(output, r#"{sep}{{"name":"{}","idx":{i}}}"#, meta.name).is_err()
            || output.len() + 1 > output.capacity()
        {
            output.truncate(rollback);
            error!("Pattern list truncated at index {i}");
            break;
        }
    }
    output.write_char(']').ok();
    output
}

fn get_pattern_description(index: usize) -> String<MAX_PATTERN_LENGTH> {
    let mut output = String::new();

    let description = commands::pattern_description(index).unwrap_or("Invalid pattern index");

    if output.push_str(description).is_err() {
        output
            .push_str("Pattern Description Too Long")
            .expect("Always fits");
    }

    output
}

pub fn start(
    spawner: &Spawner,
    connector: BleConnector<'static>,
    remote: &'static Remote,
) {
    let bt_controller: ExternalController<_, 20> = ExternalController::new(connector);

    let resources = mk_static!(HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX>, HostResources::new());
    let stack = mk_static!(
        trouble_host::Stack<
            'static,
            ExternalController<BleConnector<'static>, 20>,
            DefaultPacketPool,
        >,
        trouble_host::new(bt_controller, resources)
    );

    let Host {
        peripheral, runner, ..
    } = stack.build();

    spawner.must_spawn(ble_runner_task(runner));
    spawner.must_spawn(ble_events_task(stack, peripheral, remote));

    info!("BLE remote tasks started, waiting for connection...");
}

#[embassy_executor::task]
pub async fn ble_events_task(
    stack: &'static Stack<
        'static,
        ExternalController<BleConnector<'static>, 20>,
        DefaultPacketPool,
    >,
    mut peripheral: Peripheral<
        'static,
        ExternalController<BleConnector<'static>, 20>,
        DefaultPacketPool,
    >,
    remote: &'static Remote,
) {
    info!("Starting advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "OSSM",
        appearance: &appearance::motorized_device::GENERIC_MOTORIZED_DEVICE,
    }))
    .unwrap();

    loop {
        match advertise("OSSM-rs", &mut peripheral).await {
            Ok(connection) => {
                CONNECTED.store(true, Ordering::Release);
                info!("BLE Connected");

                Timer::after_millis(100).await;

                if let Err(err) = connection.set_phy(stack, PhyKind::Le2M).await {
                    warn!("Failed to set 2M PHY, continuing with default: {:?}", err);
                }

                let connect_params = ConnectParams {
                    min_connection_interval: Duration::from_micros(7500),
                    max_connection_interval: Duration::from_micros(7500),
                    supervision_timeout: Duration::from_millis(1000),
                    ..Default::default()
                };
                match connection
                    .update_connection_params(stack, &connect_params)
                    .await
                {
                    Ok(()) => info!("Connection interval set to 7.5ms"),
                    Err(err) => warn!(
                        "Failed to update connection params, continuing with defaults: {:?}",
                        err
                    ),
                }

                Timer::after_millis(100).await;

                match connection.read_phy(stack).await {
                    Ok(phy) => info!("PHY {:?} MTU {:?}", phy, connection.att_mtu()),
                    Err(err) => warn!(
                        "Could not read PHY: {:?}, MTU {:?}",
                        err,
                        connection.att_mtu()
                    ),
                };

                let gatt_connection = connection
                    .with_attribute_server(&server)
                    .expect("Could not transform connection into GATT connection");

                let events = gatt_events_task(&server, &gatt_connection, remote);
                let notify = state_notifications(&server, &gatt_connection, remote);

                match select(events, notify).await {
                    Either::First(res) => {
                        if let Err(err) = res {
                            error!("[gatt] error in events task: {:?}", err);
                        }
                    }
                    Either::Second(res) => match res {
                        Ok(()) => info!("[gatt] notify task ended cleanly"),
                        Err(err) => error!("[gatt] error in notify task: {:?}", err),
                    },
                }

                // Only pattern mode moves the motor; other modes reject this.
                let _ = remote
                    .send(RemoteId::Ble, Request::Pattern(PatternCommand::Stop))
                    .await;
                info!("BLE session ended, stopping engine");
            }
            Err(err) => {
                error!("[adv] error: {:?}", err);
            }
        }
    }
}

#[embassy_executor::task]
pub async fn ble_runner_task(
    mut runner: Runner<'static, ExternalController<BleConnector<'static>, 20>, DefaultPacketPool>,
) {
    loop {
        if let Err(err) = runner.run().await {
            error!("[ble_task] error: {:?}", err);
        }
    }
}

async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, P>,
    remote: &'static Remote,
) -> Result<(), Error> {
    let reason = loop {
        match connection.next().await {
            GattConnectionEvent::Disconnected { reason } => break reason,
            GattConnectionEvent::Gatt { event } => {
                let mut write = false;
                let mut event_handle = 0;
                match &event {
                    GattEvent::Read(event) => {
                        if event.handle() == server.ossm_service.current_state.handle {
                            let state_json = state_to_json(&remote.status());
                            debug!("Read State: {}", state_json);
                            server.set(&server.ossm_service.current_state, &state_json)?;
                        }
                        if event.handle() == server.ossm_service.pattern_list.handle {
                            let patterns = get_all_patterns_json();
                            server.set(&server.ossm_service.pattern_list, &patterns)?;
                        }
                    }
                    GattEvent::Write(event) => {
                        write = true;
                        event_handle = event.handle();
                    }
                    GattEvent::Other(_) => {}
                };
                // This step is also performed at drop(), but writing it explicitly is necessary
                // in order to ensure reply is sent.
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => {
                        error!("[gatt] error sending response: {:?}", e);
                    }
                };

                // This is here because the event needs to be accepted before the data can be accessed
                if write {
                    if event_handle == server.ossm_service.primary_command.handle {
                        let command: String<MAX_COMMAND_LENGTH> =
                            server.get(&server.ossm_service.primary_command)?;

                        process_command(&command, server, remote).await;
                    }
                    if event_handle == server.ossm_service.pattern_description.handle {
                        let command: String<MAX_PATTERN_LENGTH> =
                            server.get(&server.ossm_service.pattern_description)?;

                        let description = if let Ok(index) = command.parse::<usize>() {
                            get_pattern_description(index)
                        } else {
                            let mut description: String<MAX_PATTERN_LENGTH> = String::new();
                            description
                                .push_str("Could not parse pattern index")
                                .expect("Always fits");
                            description
                        };

                        server.set(&server.ossm_service.pattern_description, &description)?;
                    }
                }
            }
            GattConnectionEvent::PhyUpdated { .. }
            | GattConnectionEvent::ConnectionParamsUpdated { .. }
            | GattConnectionEvent::RequestConnectionParams { .. }
            | GattConnectionEvent::DataLengthUpdated { .. } => {}
        }
    };
    CONNECTED.store(false, Ordering::Release);
    info!("[gatt] disconnected: {:?}", reason);
    Ok(())
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'values, 'server, C: Controller>(
    name: &'values str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
) -> Result<Connection<'values, DefaultPacketPool>, BleHostError<C::Error>> {
    let uuid: [u8; 16] = SERVICE_UUID
        .as_raw()
        .try_into()
        .expect("Service UUID incorrect");

    let mut advertiser_data = [0; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[uuid]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..len],
                scan_data: &[],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?;
    info!("[adv] connection established");
    Ok(conn)
}

async fn state_notifications<P: PacketPool>(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, P>,
    remote: &'static Remote,
) -> Result<(), Error> {
    let mut sub = remote
        .subscribe_engine()
        .expect("No state subscriber slots available");
    let mut heartbeat = Ticker::every(Duration::from_secs(1));
    let mut old_status: Option<Status<OssmMode>> = None;
    loop {
        // Engine transitions wake us early; mode changes that don't touch
        // the engine show up on the heartbeat.
        select(sub.next_message_pure(), heartbeat.next()).await;

        let status = remote.status();
        if old_status != Some(status) {
            let state_json = state_to_json(&status);
            debug!("Notify State: {}", state_json);
            server
                .ossm_service
                .current_state
                .notify(connection, &state_json)
                .await?;
            old_status = Some(status);
        }
    }
}

fn state_to_json(status: &Status<OssmMode>) -> String<MAX_STATE_LENGTH> {
    let state = status.engine;
    let input = &status.input;
    let pattern_name = match state {
        EngineState::Playing(idx) | EngineState::Paused(idx) => commands::pattern_list()
            .get(idx)
            .map(|m| m.name)
            .unwrap_or(""),
        _ => "",
    };
    let mut out: String<MAX_STATE_LENGTH> = String::new();
    let state_str = match state {
        _ if status.mode == OssmMode::Fault => "fault",
        EngineState::Idle => "idle",
        EngineState::Homing => "homing",
        EngineState::Ready => "ready",
        EngineState::Playing(_) => "playing",
        EngineState::Paused(_) => "paused",
    };
    let idx = match state {
        EngineState::Playing(i) | EngineState::Paused(i) => i,
        _ => 0,
    };
    let speed = (input.velocity * 100.0) as u32;
    let stroke = (input.stroke * 100.0) as u32;
    let depth = (input.depth * 100.0) as u32;
    // Map internal -1.0..1.0 back to BLE protocol 0–100.
    let sensation = ((input.sensation + 1.0) * 50.0) as u32;
    let _ = write!(
        out,
        r#"{{"state":"{state_str}","speed":{speed},"stroke":{stroke},"sensation":{sensation},"depth":{depth},"pattern":{idx},"patternName":"{pattern_name}"}}"#,
    );
    out
}

/// How the RADR app's commands map onto modes. The app never reads the
/// `ok:`/`fail:` reply, so replies are kept honest for remotes that do:
/// `ok:` when the command was applied or is meaningless in the current
/// mode, `fail:` when it would need motion the current mode refuses.
async fn process_command(
    command: &String<MAX_COMMAND_LENGTH>,
    server: &Server<'_>,
    remote: &'static Remote,
) {
    info!("BLE Command {}", command);

    let ok = match parse_command(command) {
        Some(parsed) => apply_command(parsed, remote).await,
        None => false,
    };

    let mut response_str: String<MAX_COMMAND_LENGTH> = String::new();
    let prefix = if ok { "ok:" } else { "fail:" };
    response_str.write_str(prefix).expect("Should always fit");
    if response_str.write_str(command.as_str()).is_err() {
        response_str
            .write_str("overflow")
            .expect("Should always fit");
    }
    if let Err(err) = server.set(&server.ossm_service.primary_command, &response_str) {
        error!("Failed to write the response to a set command {:?}", err);
    }
}

#[derive(Debug, Clone, Copy)]
enum BleCommand {
    /// `go:strokeEngine` or `go:simplePenetration`.
    StartPatterns,
    /// `go:menu`.
    Menu,
    SetPattern(usize),
    /// `set:speed|stroke|depth|sensation`.
    Input(PatternCommand),
}

fn parse_command(command: &str) -> Option<BleCommand> {
    let mut parts = command.split(':');
    let (cmd, action) = (parts.next()?, parts.next());
    let Some(action) = action else {
        error!("No action in command");
        return None;
    };
    match cmd {
        "set" => {
            let Some(value) = parts.next() else {
                error!("No value after set");
                return None;
            };
            let Ok(value) = value.parse::<u32>() else {
                error!("Could not parse set value");
                return None;
            };
            let normalized = value as f64 / 100.0;
            Some(match action {
                "speed" => BleCommand::Input(PatternCommand::SetSpeed(normalized)),
                "stroke" => BleCommand::Input(PatternCommand::SetStroke(normalized)),
                "depth" => BleCommand::Input(PatternCommand::SetDepth(normalized)),
                // BLE sends 0–100; internal range is -1.0..1.0.
                "sensation" => BleCommand::Input(PatternCommand::SetSensation(normalized * 2.0 - 1.0)),
                "pattern" => BleCommand::SetPattern(value as usize),
                _ => {
                    error!("Invalid set command {}", action);
                    return None;
                }
            })
        }
        "go" => match action {
            "simplePenetration" | "strokeEngine" => Some(BleCommand::StartPatterns),
            "menu" => Some(BleCommand::Menu),
            _ => {
                error!("Unknown go action: {}", action);
                None
            }
        },
        _ => {
            error!("Unknown command: {}", cmd);
            None
        }
    }
}

/// Returns whether to reply `ok:`.
async fn apply_command(command: BleCommand, remote: &'static Remote) -> bool {
    let mode = remote.status().mode;
    match command {
        BleCommand::StartPatterns => start_patterns(remote).await,
        BleCommand::Menu => match mode {
            OssmMode::Patterns => remote.switch(OssmMode::Idle).await.is_ok(),
            // Already stopped.
            _ => true,
        },
        BleCommand::SetPattern(idx) => match send(remote, PatternCommand::Play(idx)).await {
            Ok(()) => {
                LAST_PATTERN.store(idx, Ordering::Relaxed);
                true
            }
            // The app only picks patterns while playing; outside pattern
            // mode there is nothing to change.
            Err(Rejected::NotAccepted) => true,
            Err(_) => false,
        },
        // Values sent outside pattern mode are dropped: pattern mode
        // starts at speed 0, so a dropped value can't make it faster.
        BleCommand::Input(input) => {
            let _ = send(remote, input).await;
            true
        }
    }
}

/// Enter pattern mode and make sure a pattern is playing, without
/// restarting one that already is. The app sends this twice on every
/// connect and every resume.
async fn start_patterns(remote: &'static Remote) -> bool {
    if remote.switch(OssmMode::Patterns).await.is_err() {
        return false;
    }
    match remote.status().engine {
        EngineState::Playing(_) | EngineState::Paused(_) | EngineState::Homing => true,
        EngineState::Idle | EngineState::Ready => {
            let idx = LAST_PATTERN.load(Ordering::Relaxed);
            send(remote, PatternCommand::Play(idx)).await.is_ok()
        }
    }
}

async fn send(remote: &'static Remote, command: PatternCommand) -> Result<(), Rejected> {
    remote.send(RemoteId::Ble, Request::Pattern(command)).await
}

pub fn is_ble_connected() -> bool {
    CONNECTED.load(Ordering::Acquire)
}
